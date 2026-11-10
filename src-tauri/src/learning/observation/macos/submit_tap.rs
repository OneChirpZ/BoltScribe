use core_foundation_sys::base::CFRelease;
use core_foundation_sys::mach_port::{
    CFMachPortCreateRunLoopSource, CFMachPortInvalidate, CFMachPortRef,
};
use core_foundation_sys::runloop::{
    kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef,
    CFRunLoopRemoveSource, CFRunLoopSourceRef,
};
use core_graphics::geometry::CGPoint;
use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

type EventRef = *mut c_void;
type EventTapProxy = *mut c_void;

const LEFT_MOUSE_DOWN: u32 = 1;
const LEFT_MOUSE_UP: u32 = 2;
const KEY_DOWN: u32 = 10;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xffff_fffe;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xffff_ffff;
const KEY_AUTOREPEAT: u32 = 8;
const KEY_CODE: u32 = 9;
const RETURN_KEY: i64 = 36;
const KEYPAD_ENTER: i64 = 76;
const SHIFT: u64 = 1 << 17;
const CONTROL: u64 = 1 << 18;
const OPTION: u64 = 1 << 19;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreateForPid(
        pid: i32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: extern "C" fn(EventTapProxy, u32, EventRef, *mut c_void) -> EventRef,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enabled: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CGEventGetFlags(event: EventRef) -> u64;
    fn CGEventGetIntegerValueField(event: EventRef, field: u32) -> i64;
    fn CGEventGetLocation(event: EventRef) -> CGPoint;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SubmitInput {
    MouseDown { x: f64, y: f64 },
    MouseUp { x: f64, y: f64 },
    Return,
    Other,
    Disabled,
}

struct CallbackState<'a> {
    callback: RefCell<Box<dyn FnMut(SubmitInput) + 'a>>,
}

/// Lives on the observer worker's runloop. The active tap lets the caller take
/// a bounded snapshot before returning the original event for normal delivery.
pub(super) struct SubmitTap<'a> {
    tap: CFMachPortRef,
    source: CFRunLoopSourceRef,
    runloop: CFRunLoopRef,
    // The allocation must outlive the runloop source and its callback pointer.
    _callback: Box<CallbackState<'a>>,
}

impl<'a> SubmitTap<'a> {
    pub(super) fn new(pid: i32, callback: impl FnMut(SubmitInput) + 'a) -> Option<Self> {
        if pid <= 0 {
            return None;
        }
        let mut callback = Box::new(CallbackState {
            callback: RefCell::new(Box::new(callback)),
        });
        let context = (&mut *callback as *mut CallbackState<'a>).cast();
        let mask = (1u64 << LEFT_MOUSE_DOWN) | (1u64 << LEFT_MOUSE_UP) | (1u64 << KEY_DOWN);
        // Head insertion and default (active) options. Every callback returns
        // the supplied event unchanged, including disabled and error paths.
        let tap = unsafe { CGEventTapCreateForPid(pid, 0, 0, mask, event_callback, context) };
        if tap.is_null() {
            return None;
        }
        let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) };
        if source.is_null() {
            unsafe {
                CFMachPortInvalidate(tap);
                CFRelease(tap.cast());
            }
            return None;
        }
        let runloop = unsafe { CFRunLoopGetCurrent() };
        let listener = Self {
            tap,
            source,
            runloop,
            _callback: callback,
        };
        unsafe {
            CFRunLoopAddSource(runloop, source, kCFRunLoopDefaultMode);
            CGEventTapEnable(tap, true);
        }
        if !unsafe { CGEventTapIsEnabled(tap) } {
            return None;
        }
        Some(listener)
    }
}

impl Drop for SubmitTap<'_> {
    fn drop(&mut self) {
        unsafe {
            CGEventTapEnable(self.tap, false);
            CFRunLoopRemoveSource(self.runloop, self.source, kCFRunLoopDefaultMode);
            CFMachPortInvalidate(self.tap);
            CFRelease(self.source.cast());
            CFRelease(self.tap.cast());
        }
    }
}

fn key_input(key: i64, flags: u64, repeated: bool) -> SubmitInput {
    if matches!(key, RETURN_KEY | KEYPAD_ENTER)
        && flags & (SHIFT | CONTROL | OPTION) == 0
        && !repeated
    {
        SubmitInput::Return
    } else {
        SubmitInput::Other
    }
}

fn dispatch(state: &CallbackState<'_>, input: SubmitInput) {
    if let Ok(mut callback) = state.callback.try_borrow_mut() {
        // Neither reentry nor a Rust unwind may prevent the original event
        // from being returned through the CoreGraphics callback boundary.
        let _ = catch_unwind(AssertUnwindSafe(|| callback(input)));
    }
}

extern "C" fn event_callback(
    _proxy: EventTapProxy,
    event_type: u32,
    event: EventRef,
    context: *mut c_void,
) -> EventRef {
    if context.is_null() {
        return event;
    }
    let state = unsafe { &*context.cast::<CallbackState<'_>>() };
    if matches!(
        event_type,
        TAP_DISABLED_BY_TIMEOUT | TAP_DISABLED_BY_USER_INPUT
    ) {
        // Disabled notifications may carry a null event. Do not silently
        // resume the tap after the system or user has disabled it.
        dispatch(state, SubmitInput::Disabled);
        return event;
    }
    if event.is_null() {
        return event;
    }
    let input = match event_type {
        KEY_DOWN => key_input(
            unsafe { CGEventGetIntegerValueField(event, KEY_CODE) },
            unsafe { CGEventGetFlags(event) },
            unsafe { CGEventGetIntegerValueField(event, KEY_AUTOREPEAT) } != 0,
        ),
        LEFT_MOUSE_DOWN | LEFT_MOUSE_UP => {
            let point = unsafe { CGEventGetLocation(event) };
            if event_type == LEFT_MOUSE_DOWN {
                SubmitInput::MouseDown {
                    x: point.x,
                    y: point.y,
                }
            } else {
                SubmitInput::MouseUp {
                    x: point.x,
                    y: point.y,
                }
            }
        }
        _ => return event,
    };
    dispatch(state, input);
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn return_accepts_optional_command_and_keypad_flags() {
        const COMMAND: u64 = 1 << 20;
        const NUMERIC_PAD: u64 = 1 << 21;
        for key in [RETURN_KEY, KEYPAD_ENTER] {
            for flags in [0, COMMAND, NUMERIC_PAD, COMMAND | NUMERIC_PAD] {
                assert_eq!(key_input(key, flags, false), SubmitInput::Return);
            }
        }
    }

    #[test]
    fn modifiers_repeats_and_other_keys_do_not_match_return() {
        for flags in [SHIFT, CONTROL, OPTION, SHIFT | (1 << 20)] {
            assert_eq!(key_input(RETURN_KEY, flags, false), SubmitInput::Other);
        }
        assert_eq!(key_input(RETURN_KEY, 0, true), SubmitInput::Other);
        assert_eq!(key_input(KEYPAD_ENTER, 0, true), SubmitInput::Other);
        assert_eq!(key_input(0, 0, false), SubmitInput::Other);
    }

    #[test]
    fn callback_preserves_event_identity_and_reports_null_disabled_events() {
        let calls = Cell::new(0);
        let mut state = CallbackState {
            callback: RefCell::new(Box::new(|input| {
                assert_eq!(input, SubmitInput::Disabled);
                calls.set(calls.get() + 1);
            })),
        };
        let context = (&mut state as *mut CallbackState<'_>).cast();
        assert!(event_callback(
            ptr::null_mut(),
            TAP_DISABLED_BY_TIMEOUT,
            ptr::null_mut(),
            context
        )
        .is_null());
        assert_eq!(calls.get(), 1);
        let mut opaque = 0u8;
        let original = (&mut opaque as *mut u8).cast();
        assert_eq!(
            event_callback(ptr::null_mut(), 0, original, context),
            original
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn reentrant_callback_returns_without_panicking_or_double_borrowing() {
        let calls = Cell::new(0);
        let state = CallbackState {
            callback: RefCell::new(Box::new(|_| calls.set(calls.get() + 1))),
        };
        let borrowing_callback = state.callback.borrow_mut();
        dispatch(&state, SubmitInput::Return);
        assert_eq!(calls.get(), 0);
        drop(borrowing_callback);
        dispatch(&state, SubmitInput::Return);
        assert_eq!(calls.get(), 1);
    }
}
