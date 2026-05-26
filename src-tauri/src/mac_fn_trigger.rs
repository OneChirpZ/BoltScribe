use crate::workflow;
use anyhow::{anyhow, Result};
use std::os::raw::c_void;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, OnceLock,
};
use std::time::Duration;
use tauri::AppHandle;

const LONG_PRESS_DURATION: Duration = Duration::from_millis(300);

#[cfg(target_os = "macos")]
static STATE: OnceLock<Arc<FnTriggerState>> = OnceLock::new();

#[cfg(target_os = "macos")]
struct FnTriggerState {
    app: AppHandle,
    enabled: AtomicBool,
    pressed: AtomicBool,
    sequence: AtomicU64,
    triggered_in_press: AtomicBool,
}

pub(crate) fn apply(app: &AppHandle, enabled: bool) -> Result<()> {
    platform::apply(app, enabled)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease};
    use core_foundation_sys::mach_port::{CFMachPortCreateRunLoopSource, CFMachPortRef};
    use core_foundation_sys::runloop::{
        kCFRunLoopCommonModes, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRun,
    };

    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
    const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
    const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
    const K_CG_EVENT_FLAG_MASK_SECONDARY_FN: u64 = 0x0080_0000;
    const K_IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
    const K_IOHID_ACCESS_TYPE_GRANTED: i32 = 0;

    type CGEventRef = *mut c_void;
    type CGEventTapProxy = *mut c_void;
    type CGEventTapCallBack =
        unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: CGEventTapCallBack,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventGetFlags(event: CGEventRef) -> u64;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFMachPortInvalidate(port: CFMachPortRef);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOHIDCheckAccess(request_type: u32) -> i32;
        fn IOHIDRequestAccess(request_type: u32) -> bool;
    }

    pub(super) fn apply(app: &AppHandle, enabled: bool) -> Result<()> {
        if let Some(state) = STATE.get() {
            state.enabled.store(enabled, Ordering::SeqCst);
            return Ok(());
        }
        if !enabled {
            return Ok(());
        }

        request_input_monitoring_access()?;
        let state = Arc::new(FnTriggerState {
            app: app.clone(),
            enabled: AtomicBool::new(true),
            pressed: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
            triggered_in_press: AtomicBool::new(false),
        });
        start_event_thread(state.clone())?;
        let _ = STATE.set(state);
        Ok(())
    }

    fn request_input_monitoring_access() -> Result<()> {
        unsafe {
            if IOHIDCheckAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT)
                == K_IOHID_ACCESS_TYPE_GRANTED
            {
                return Ok(());
            }
            if IOHIDRequestAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT) {
                return Ok(());
            }
        }
        Err(anyhow!(
            "Input Monitoring permission is required for Fn long-press trigger"
        ))
    }

    fn start_event_thread(state: Arc<FnTriggerState>) -> Result<()> {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("boltscribe-fn-trigger".to_string())
            .spawn(move || run_event_tap(state, sender))
            .map_err(|err| anyhow!("Failed to start Fn trigger listener: {err}"))?;

        receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| anyhow!("Timed out starting Fn trigger listener"))?
    }

    fn run_event_tap(state: Arc<FnTriggerState>, startup: mpsc::Sender<Result<()>>) {
        unsafe {
            let state_ptr = Arc::into_raw(state) as *mut c_void;
            let event_mask = 1u64 << K_CG_EVENT_FLAGS_CHANGED;
            let tap = CGEventTapCreate(
                K_CG_HID_EVENT_TAP,
                K_CG_HEAD_INSERT_EVENT_TAP,
                K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
                event_mask,
                fn_event_callback,
                state_ptr,
            );
            if tap.is_null() {
                drop(Arc::from_raw(state_ptr as *const FnTriggerState));
                let _ = startup.send(Err(anyhow!(
                    "Failed to create Fn trigger event tap; check Accessibility permission"
                )));
                return;
            }

            let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
            if source.is_null() {
                CFMachPortInvalidate(tap);
                CFRelease(tap as *const c_void);
                drop(Arc::from_raw(state_ptr as *const FnTriggerState));
                let _ = startup.send(Err(anyhow!("Failed to create Fn trigger run loop source")));
                return;
            }

            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
            let _ = startup.send(Ok(()));
            CFRunLoopRun();

            CFRelease(source as *const c_void);
            CFMachPortInvalidate(tap);
            CFRelease(tap as *const c_void);
            drop(Arc::from_raw(state_ptr as *const FnTriggerState));
        }
    }

    unsafe extern "C" fn fn_event_callback(
        _proxy: CGEventTapProxy,
        event_type: u32,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef {
        if event.is_null() || user_info.is_null() {
            return event;
        }
        if event_type == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
            || event_type == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
        {
            return event;
        }
        if event_type != K_CG_EVENT_FLAGS_CHANGED {
            return event;
        }

        let state = &*(user_info as *const FnTriggerState);
        if !state.enabled.load(Ordering::SeqCst) {
            state.pressed.store(false, Ordering::SeqCst);
            state.sequence.fetch_add(1, Ordering::SeqCst);
            return event;
        }

        let fn_down = CGEventGetFlags(event) & K_CG_EVENT_FLAG_MASK_SECONDARY_FN != 0;
        if fn_down {
            if !state.pressed.swap(true, Ordering::SeqCst) {
                state.triggered_in_press.store(false, Ordering::SeqCst);
                let sequence = state.sequence.fetch_add(1, Ordering::SeqCst) + 1;
                let timer_state = clone_state(user_info);
                std::thread::spawn(move || trigger_after_long_press(timer_state, sequence));
            }
        } else if state.pressed.swap(false, Ordering::SeqCst) {
            state.sequence.fetch_add(1, Ordering::SeqCst);
            state.triggered_in_press.store(false, Ordering::SeqCst);
        }
        event
    }

    unsafe fn clone_state(user_info: *mut c_void) -> Arc<FnTriggerState> {
        let ptr = user_info as *const FnTriggerState;
        Arc::increment_strong_count(ptr);
        Arc::from_raw(ptr)
    }

    fn trigger_after_long_press(state: Arc<FnTriggerState>, sequence: u64) {
        std::thread::sleep(LONG_PRESS_DURATION);
        if !state.enabled.load(Ordering::SeqCst)
            || !state.pressed.load(Ordering::SeqCst)
            || state.sequence.load(Ordering::SeqCst) != sequence
            || state.triggered_in_press.swap(true, Ordering::SeqCst)
        {
            return;
        }

        if let Err(err) = workflow::toggle_recording_from_app(state.app.clone()) {
            eprintln!("Fn long-press trigger failed: {err:?}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub(super) fn apply(_app: &AppHandle, _enabled: bool) -> Result<()> {
        Ok(())
    }
}
