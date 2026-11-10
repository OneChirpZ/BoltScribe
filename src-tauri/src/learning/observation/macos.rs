use super::*;
use core_foundation::array::CFArray;
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::array::{
    CFArrayContainsValue, CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef,
};
use core_foundation_sys::base::{CFEqual, CFGetTypeID, CFRange, CFRelease, CFTypeRef};
use core_foundation_sys::number::{
    kCFNumberSInt64Type, CFBooleanGetTypeID, CFBooleanGetValue, CFNumberGetTypeID, CFNumberGetValue,
};
use core_foundation_sys::runloop::*;
use core_foundation_sys::string::{
    CFStringGetCharacters, CFStringGetLength, CFStringGetTypeID, CFStringRef,
};
use objc2_app_kit::NSWorkspace;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::AtomicU8;
use std::sync::mpsc;

mod submission;
mod submit_tap;
mod submit_target;

use submission::{Submission, SubmitCapture};
use submit_tap::SubmitTap;

type AxRef = *const c_void;
const MESSAGE_TIMEOUT: f32 = 0.025;
const PREPARE_TIMEOUT: Duration = Duration::from_millis(150);
const READ_BUDGET: Duration = Duration::from_millis(125);
const VERIFY_DELAYS_MS: [u64; 3] = [80, 160, 320];
const INJECTION_LIMIT: Duration = Duration::from_secs(30);
const AX_RANGE_TYPE: u32 = 4;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> AxRef;
    fn AXUIElementGetPid(element: AxRef, pid: *mut i32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: AxRef,
        name: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementGetAttributeValueCount(
        element: AxRef,
        name: CFStringRef,
        count: *mut isize,
    ) -> i32;
    fn AXUIElementCopyAttributeValues(
        element: AxRef,
        name: CFStringRef,
        index: isize,
        count: isize,
        values: *mut CFArrayRef,
    ) -> i32;
    fn AXUIElementCopyMultipleAttributeValues(
        element: AxRef,
        names: CFArrayRef,
        options: u32,
        values: *mut CFArrayRef,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AxRef,
        name: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AxRef, timeout: f32) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, result: *mut c_void) -> bool;
    fn AXObserverCreate(
        pid: i32,
        callback: extern "C" fn(AxRef, AxRef, CFStringRef, *mut c_void),
        observer: *mut AxRef,
    ) -> i32;
    fn AXObserverAddNotification(
        observer: AxRef,
        element: AxRef,
        name: CFStringRef,
        context: *mut c_void,
    ) -> i32;
    fn AXObserverRemoveNotification(observer: AxRef, element: AxRef, name: CFStringRef) -> i32;
    fn AXObserverGetRunLoopSource(observer: AxRef) -> CFRunLoopSourceRef;
}

/// Addresses are sent only to signal the worker's runloop. The mutex in Control
/// keeps these refs alive while a sender uses them; they are cleared before drop.
pub(super) struct WakeHandle {
    runloop: usize,
    source: usize,
}

impl WakeHandle {
    pub(super) fn signal(&self) {
        unsafe {
            CFRunLoopSourceSignal(self.source as CFRunLoopSourceRef);
            CFRunLoopWakeUp(self.runloop as CFRunLoopRef);
        }
    }
}

struct Owned(CFTypeRef);

impl Owned {
    fn new(value: CFTypeRef) -> Option<Self> {
        (!value.is_null()).then(|| Self(value))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

struct WorkerGuard;

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        WORKER_RUNNING.store(false, Ordering::Release);
    }
}

pub(super) fn prepare(
    record_id: String,
    text: String,
    complete: Completion,
    activity: Option<super::super::activity::ActivityHandle>,
) -> Option<Session> {
    let control = Arc::new(Control {
        cancelled: AtomicBool::new(false),
        finishing: AtomicBool::new(false),
        injected: AtomicU8::new(0),
        wake: Mutex::new(None),
    });
    if !GATE.install(&control, &WORKER_RUNNING) {
        SKIPPED.fetch_add(1, Ordering::Relaxed);
        if let Some(activity) = activity {
            activity.update(
                super::ActivityState::Skipped,
                Some("observer_busy"),
                vec![],
                None,
            );
        }
        return None;
    }
    let worker_activity = activity.clone();
    let worker_control = control.clone();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("input-correction-observer".into())
        .spawn(move || {
            let _guard = WorkerGuard;
            let mut reason = "target_unavailable";
            let sample = run(
                &worker_control,
                record_id,
                text,
                ready_tx,
                worker_activity.clone(),
                &mut reason,
            );
            if let Some(sample) =
                sample.filter(|_| !worker_control.cancelled.load(Ordering::Acquire))
            {
                COMPLETED.fetch_add(1, Ordering::Relaxed);
                complete(sample);
            } else {
                SKIPPED.fetch_add(1, Ordering::Relaxed);
                if let Some(activity) = worker_activity {
                    let cancelled = worker_control.cancelled.load(Ordering::Acquire);
                    let reason = if cancelled { "cancelled" } else { reason };
                    let state = if reason == "no_change" {
                        super::ActivityState::NoChange
                    } else {
                        super::ActivityState::Skipped
                    };
                    activity.update(state, Some(reason), vec![], None);
                }
            }
        });
    if spawned.is_err() {
        if let Some(activity) = activity {
            activity.update(
                super::ActivityState::Failed,
                Some("worker_unavailable"),
                vec![],
                None,
            );
        }
        WORKER_RUNNING.store(false, Ordering::Release);
        SKIPPED.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    if ready_rx.recv_timeout(PREPARE_TIMEOUT) != Ok(true) {
        if let Some(activity) = activity {
            activity.update(
                super::ActivityState::Skipped,
                Some("target_unavailable"),
                vec![],
                None,
            );
        }
        control.cancelled.store(true, Ordering::Release);
        control.wake();
        return None;
    }
    Some(Session {
        control,
        committed: false,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum End {
    Focus,
    Deactivated,
    Destroyed,
}

struct CallbackState {
    target: AxRef,
    value_changed: CFString,
    destroyed: CFString,
    deactivated: CFString,
    injecting: bool,
    end: Option<End>,
    debounce: Debounce,
}

extern "C" fn notification(
    _observer: AxRef,
    element: AxRef,
    name: CFStringRef,
    context: *mut c_void,
) {
    if context.is_null() {
        return;
    }
    // This callback is delivered exclusively on the worker's runloop. It never
    // queries AX, logs content, allocates snapshots, or calls the model.
    let state = unsafe { &mut *context.cast::<CallbackState>() };
    NOTIFICATIONS.fetch_add(1, Ordering::Relaxed);
    if state.end == Some(End::Deactivated) {
        return;
    }
    if unsafe { CFEqual(name.cast(), state.destroyed.as_concrete_TypeRef().cast()) } != 0 {
        state.end = Some(End::Destroyed);
        return;
    }
    if unsafe {
        CFEqual(
            name.cast(),
            state.value_changed.as_concrete_TypeRef().cast(),
        )
    } != 0
    {
        if state.end.is_none() && !state.injecting && unsafe { CFEqual(element, state.target) } != 0
        {
            state.debounce.changed(Instant::now());
        }
    } else if unsafe { CFEqual(name.cast(), state.deactivated.as_concrete_TypeRef().cast()) } != 0 {
        state.end = Some(End::Deactivated);
    } else if state.end.is_none() {
        state.end = Some(End::Focus);
    }
}

struct NativeObserver<'a> {
    control: &'a Control,
    observer: Owned,
    runloop: CFRunLoopRef,
    observer_source: CFRunLoopSourceRef,
    control_source: Owned,
    registrations: Vec<(AxRef, CFString)>,
    // Dropped after notification delivery has been disconnected.
    state: Box<CallbackState>,
}

impl<'a> NativeObserver<'a> {
    fn new(control: &'a Control, target: AxRef, app: AxRef, pid: i32) -> Option<Self> {
        let mut observer = ptr::null();
        if unsafe { AXObserverCreate(pid, notification, &mut observer) } != 0 {
            return None;
        }
        let observer = Owned::new(observer)?;
        let runloop = unsafe { CFRunLoopGetCurrent() };
        let observer_source = unsafe { AXObserverGetRunLoopSource(observer.0) };
        extern "C" fn wake(_info: *const c_void) {}
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: ptr::null_mut(),
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: wake,
        };
        let control_source =
            Owned::new(unsafe { CFRunLoopSourceCreate(ptr::null(), 0, &mut context) }.cast())?;
        if observer_source.is_null() {
            return None;
        }
        let state = Box::new(CallbackState {
            target,
            value_changed: CFString::new("AXValueChanged"),
            destroyed: CFString::new("AXUIElementDestroyed"),
            deactivated: CFString::new("AXApplicationDeactivated"),
            injecting: true,
            end: None,
            debounce: Debounce::default(),
        });
        let mut native = Self {
            control,
            observer,
            runloop,
            observer_source,
            control_source,
            registrations: Vec::new(),
            state,
        };
        // Fail closed when scope notifications are unsupported, rather than
        // substituting a global keyboard hook or periodic focus polling.
        for (element, name) in [
            (target, "AXValueChanged"),
            (target, "AXUIElementDestroyed"),
            (app, "AXFocusedUIElementChanged"),
            (app, "AXApplicationDeactivated"),
        ] {
            let name = CFString::new(name);
            let context = (&mut *native.state as *mut CallbackState).cast();
            if unsafe {
                AXObserverAddNotification(
                    native.observer.0,
                    element,
                    name.as_concrete_TypeRef(),
                    context,
                )
            } != 0
            {
                return None;
            }
            native.registrations.push((element, name));
        }
        unsafe {
            CFRunLoopAddSource(runloop, observer_source, kCFRunLoopDefaultMode);
            CFRunLoopAddSource(
                runloop,
                native.control_source.0 as CFRunLoopSourceRef,
                kCFRunLoopDefaultMode,
            );
        }
        if let Ok(mut wake) = control.wake.lock() {
            *wake = Some(WakeHandle {
                runloop: runloop as usize,
                source: native.control_source.0 as usize,
            });
        }
        Some(native)
    }

    fn wait_until(&mut self, deadline: Instant) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, remaining.as_secs_f64(), 1);
        }
    }
}

impl Drop for NativeObserver<'_> {
    fn drop(&mut self) {
        if let Ok(mut wake) = self.control.wake.lock() {
            *wake = None;
        }
        unsafe {
            CFRunLoopRemoveSource(self.runloop, self.observer_source, kCFRunLoopDefaultMode);
            CFRunLoopRemoveSource(
                self.runloop,
                self.control_source.0 as CFRunLoopSourceRef,
                kCFRunLoopDefaultMode,
            );
            for (element, name) in &self.registrations {
                AXObserverRemoveNotification(self.observer.0, *element, name.as_concrete_TypeRef());
            }
        }
    }
}

fn run(
    control: &Control,
    record_id: String,
    text: String,
    ready: mpsc::SyncSender<bool>,
    activity: Option<super::super::activity::ActivityHandle>,
    reason: &mut &'static str,
) -> Option<LearningSample> {
    if !unsafe { AXIsProcessTrusted() } {
        return None;
    }
    let workspace = NSWorkspace::sharedWorkspace();
    let frontmost = workspace.frontmostApplication()?;
    let pid = frontmost.processIdentifier();
    if pid <= 0 || pid as u32 == std::process::id() {
        return None;
    }
    let app = Owned::new(unsafe { AXUIElementCreateApplication(pid) })?;
    unsafe {
        AXUIElementSetMessagingTimeout(app.0, MESSAGE_TIMEOUT);
    }
    let target = copy_attribute(app.0, "AXFocusedUIElement")?;
    unsafe {
        AXUIElementSetMessagingTimeout(target.0, MESSAGE_TIMEOUT);
    }
    let mut target_pid = 0;
    if unsafe { AXUIElementGetPid(target.0, &mut target_pid) } != 0 || target_pid != pid {
        return None;
    }
    let started = Instant::now();
    let attrs = copy_attributes(
        target.0,
        &[
            "AXRole",
            "AXSubrole",
            "AXProtectedContent",
            "AXIsPassword",
            "AXNumberOfCharacters",
            "AXSelectedTextRange",
            "AXMarkedTextRange",
            "AXDOMClassList",
        ],
    )?;
    let role = string_value(array_at(&attrs, 0)?)?;
    let subrole = array_at(&attrs, 1)
        .and_then(string_value)
        .unwrap_or_default();
    if !matches!(role.as_str(), "AXTextField" | "AXTextArea")
        || subrole == "AXSecureTextField"
        || array_at(&attrs, 2).is_some_and(boolean_value)
        || array_at(&attrs, 3).is_some_and(boolean_value)
        || array_at(&attrs, 6)
            .and_then(range_value)
            .is_some_and(|range| range.length > 0)
    {
        return None;
    }
    let selection = array_at(&attrs, 5).and_then(range_value)?;
    let length = array_at(&attrs, 4).and_then(number_value);
    let classes = array_at(&attrs, 7);
    let deadline = started + READ_BUDGET;
    let baseline = length.and_then(|length| read_editor_text(target.0, classes, length, deadline));
    let empty_placeholder = empty_web_placeholder(target.0, classes, selection, length, deadline);
    let insertion = baseline
        .as_deref()
        .and_then(|baseline| {
            Insertion::new(
                baseline,
                usize::try_from(selection.location).ok()?,
                usize::try_from(selection.length).ok()?,
                text.clone(),
            )
        })
        .or_else(|| {
            // Hidden suggestions may not support a consistent text range.
            // This candidate still requires exact dictation readback below.
            empty_placeholder
                .then(|| Insertion::new("", 0, 0, text))
                .flatten()
        })?;
    let web_draft = baseline.as_deref().filter(|baseline| {
        selection.length == 0 && baseline.ends_with('\n') && has_class(classes, "ProseMirror")
    });
    if started.elapsed() >= READ_BUDGET {
        return None;
    }
    let mut native = NativeObserver::new(control, target.0, app.0, pid)?;
    if workspace.frontmostApplication()?.processIdentifier() != pid
        || control.cancelled.load(Ordering::Acquire)
        || ready.send(true).is_err()
    {
        return None;
    }

    let injection_deadline = Instant::now() + INJECTION_LIMIT;
    while control.injected.load(Ordering::Acquire) == 0 {
        if control.cancelled.load(Ordering::Acquire)
            || native.state.end.is_some()
            || Instant::now() >= injection_deadline
        {
            return None;
        }
        native.wait_until(injection_deadline);
    }
    if control.injected.load(Ordering::Acquire) != 1 || control.cancelled.load(Ordering::Acquire) {
        return None;
    }
    let injected_at = Instant::now();
    let mut confirmed = None;
    for delay_ms in VERIFY_DELAYS_MS {
        let verify_at = injected_at + Duration::from_millis(delay_ms);
        while Instant::now() < verify_at {
            *reason = "focus_changed";
            if native.state.end.is_some() || control.cancelled.load(Ordering::Acquire) {
                return None;
            }
            native.wait_until(verify_at);
        }
        *reason = "focus_changed";
        if workspace.frontmostApplication()?.processIdentifier() != pid
            || native.state.end.is_some()
            || control.cancelled.load(Ordering::Acquire)
        {
            return None;
        }
        *reason = "target_unavailable";
        let Some(focused) = copy_attribute(app.0, "AXFocusedUIElement") else {
            continue;
        };
        if unsafe { CFEqual(focused.0, target.0) } == 0 {
            *reason = "focus_changed";
            return None;
        }
        if let Some(TextSnapshot::Committed(verified)) = read_text(target.0) {
            *reason = "injection_mismatch";
            confirmed = insertion
                .confirm(&verified, empty_placeholder)
                .or_else(|| confirm_web_append(web_draft?, &insertion.original, &verified));
            if confirmed.is_some() {
                break;
            }
        }
    }
    let insertion = confirmed?;
    native.state.injecting = false;
    native.state.debounce = Debounce::default();
    let submit_capture = frontmost.bundleIdentifier().and_then(|bundle_id| {
        SubmitCapture::new(
            &bundle_id.to_string(),
            pid,
            app.0,
            target.0,
            control,
            &insertion,
        )
    });
    let submit_tap = submit_capture
        .as_ref()
        .and_then(|capture| SubmitTap::new(pid, |input| capture.on_input(input)));
    let mut current = insertion.original.clone();
    let mut submitted = false;
    *reason = "observation_interrupted";
    let observation_started = Instant::now();
    loop {
        let now = Instant::now();
        let deadline = native.state.debounce.idle_deadline(observation_started);
        if control.cancelled.load(Ordering::Acquire) {
            return None;
        }
        let destroyed = native.state.end == Some(End::Destroyed);
        if let Some(capture) = submit_capture
            .as_ref()
            .filter(|_| submit_tap.is_some() && native.state.end != Some(End::Deactivated))
        {
            match capture.check(destroyed) {
                Submission::Confirmed(text) => {
                    current = text;
                    submitted = true;
                    break;
                }
                Submission::Waiting(next) => {
                    native.wait_until(next);
                    continue;
                }
                Submission::None => {}
            }
        }
        if destroyed {
            return None;
        }
        let finishing = control.finishing.load(Ordering::Acquire)
            || native.state.end.is_some()
            || now >= deadline;
        if finishing {
            // A pending change might be an IME draft, submission/clear, or an
            // unobserved edit. Never learn the stale last stable snapshot.
            if native.state.debounce.dirty_at.is_some() {
                *reason = "unstable_edit";
                return None;
            }
            break;
        }
        if native.state.debounce.ready(now) {
            match read_text(target.0)? {
                TextSnapshot::Committed(snapshot) => {
                    *reason = "out_of_scope";
                    current = insertion.extract(&snapshot)?;
                    *reason = "observation_interrupted";
                }
                TextSnapshot::Placeholder => {
                    *reason = "input_cleared";
                    return None;
                }
                TextSnapshot::Composing => {}
            }
            // An IME composition can pause while its candidate list is open.
            // Wait for its next change or the idle timeout without polling it.
            native.state.debounce.dirty_at = None;
        }
        let wake_at = native.state.debounce.next_deadline(deadline);
        native.wait_until(wake_at);
    }
    drop(submit_tap);
    drop(submit_capture);
    if current == insertion.original || current.trim().is_empty() {
        *reason = "no_change";
        return None;
    }
    if !submitted {
        // A terminal event may overtake a value notification. Confirm the last
        // stable sample against the original element, never the newly focused one.
        let TextSnapshot::Committed(final_text) = read_text(target.0)? else {
            return None;
        };
        if insertion.extract(&final_text).as_deref() != Some(current.as_str()) {
            return None;
        }
    }
    // NativeObserver drops before the completion callback or any model work.
    Some(LearningSample {
        activity,
        original: insertion.original,
        current,
        record_id,
    })
}

fn copy_attribute(element: AxRef, name: &str) -> Option<Owned> {
    let name = CFString::new(name);
    let mut value = ptr::null();
    (unsafe { AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef(), &mut value) } == 0)
        .then(|| Owned::new(value))
        .flatten()
}

fn copy_attributes(element: AxRef, names: &[&str]) -> Option<Owned> {
    let names: Vec<CFString> = names.iter().map(|name| CFString::new(name)).collect();
    let names = CFArray::from_CFTypes(&names);
    let mut values = ptr::null();
    (unsafe {
        AXUIElementCopyMultipleAttributeValues(element, names.as_concrete_TypeRef(), 0, &mut values)
    } == 0)
        .then(|| Owned::new(values.cast()))
        .flatten()
}

fn array_at(array: &Owned, index: isize) -> Option<CFTypeRef> {
    let array = array.0 as CFArrayRef;
    (index >= 0 && index < unsafe { CFArrayGetCount(array) })
        .then(|| unsafe { CFArrayGetValueAtIndex(array, index) })
}

fn string_value(value: CFTypeRef) -> Option<String> {
    bounded_string_value(value, MAX_TEXT_UNITS)
}

fn bounded_string_value(value: CFTypeRef, max_units: usize) -> Option<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    let value = value as CFStringRef;
    let length = unsafe { CFStringGetLength(value) };
    let length = usize::try_from(length).ok()?;
    if length > max_units {
        return None;
    }
    let mut units = vec![0; length];
    unsafe {
        CFStringGetCharacters(
            value,
            CFRange {
                location: 0,
                length: length as isize,
            },
            units.as_mut_ptr(),
        );
    }
    String::from_utf16(&units).ok()
}

fn boolean_value(value: CFTypeRef) -> bool {
    !value.is_null()
        && unsafe { CFGetTypeID(value) } == unsafe { CFBooleanGetTypeID() }
        && unsafe { CFBooleanGetValue(value.cast()) }
}

fn number_value(value: CFTypeRef) -> Option<usize> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFNumberGetTypeID() } {
        return None;
    }
    let mut number: i64 = 0;
    if !unsafe {
        CFNumberGetValue(
            value.cast(),
            kCFNumberSInt64Type,
            (&mut number as *mut i64).cast(),
        )
    } {
        return None;
    }
    usize::try_from(number).ok()
}

fn range_value(value: CFTypeRef) -> Option<CFRange> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { AXValueGetTypeID() } {
        return None;
    }
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    unsafe { AXValueGetValue(value, AX_RANGE_TYPE, (&mut range as *mut CFRange).cast()) }
        .then_some(range)
}

enum TextSnapshot {
    Committed(String),
    Composing,
    Placeholder,
}

fn has_class(value: Option<CFTypeRef>, name: &str) -> bool {
    let Some(value) = value
        .filter(|value| !value.is_null() && unsafe { CFGetTypeID(*value) == CFArrayGetTypeID() })
    else {
        return false;
    };
    let array = value as CFArrayRef;
    let name = CFString::new(name);
    unsafe {
        CFArrayContainsValue(
            array,
            CFRange {
                location: 0,
                length: CFArrayGetCount(array),
            },
            name.as_concrete_TypeRef().cast(),
        ) != 0
    }
}

fn caret_at_text_edge(selection: CFRange, length: Option<usize>) -> bool {
    selection.length == 0
        && (selection.location == 0
            || usize::try_from(selection.location).is_ok_and(|offset| Some(offset) == length))
}

// Chromium can misreport the caret in an empty final paragraph and expose
// an extra trailing break. Require the entire draft before the exact insertion.
fn confirm_web_append(baseline: &str, original: &str, verified: &str) -> Option<Insertion> {
    let prefix = verified.strip_suffix(original)?;
    let without_break = baseline.strip_suffix('\n')?;
    (prefix == baseline
        || (prefix == without_break && (prefix.is_empty() || prefix.ends_with('\n'))))
    .then(|| Insertion {
        prefix: prefix.to_owned(),
        suffix: String::new(),
        original: original.to_owned(),
    })
}

// ProseMirror's empty paragraph decoration also hosts Codex's unaccepted
// prompt suggestion. Its generated text can appear in AXStringForRange even
// though the document is empty. Inspect only this field's single paragraph.
fn empty_web_placeholder(
    element: AxRef,
    classes: Option<CFTypeRef>,
    selection: CFRange,
    length: Option<usize>,
    deadline: Instant,
) -> bool {
    if !caret_at_text_edge(selection, length)
        || !has_class(classes, "ProseMirror")
        || Instant::now() >= deadline
    {
        return false;
    }
    let name = CFString::new("AXChildren");
    let mut count = 0;
    if unsafe { AXUIElementGetAttributeValueCount(element, name.as_concrete_TypeRef(), &mut count) }
        != 0
        || count != 1
        || Instant::now() >= deadline
    {
        return false;
    }
    let mut children = ptr::null();
    if unsafe {
        AXUIElementCopyAttributeValues(element, name.as_concrete_TypeRef(), 0, 1, &mut children)
    } != 0
    {
        return false;
    }
    let Some(children) = Owned::new(children.cast()) else {
        return false;
    };
    let Some(paragraph) = array_at(&children, 0) else {
        return false;
    };
    if Instant::now() >= deadline {
        return false;
    }
    unsafe {
        AXUIElementSetMessagingTimeout(paragraph, MESSAGE_TIMEOUT);
    }
    let classes = copy_attribute(paragraph, "AXDOMClassList");
    has_class(classes.as_ref().map(|value| value.0), "placeholder")
}

fn read_text(element: AxRef) -> Option<TextSnapshot> {
    read_text_before(element, Instant::now() + READ_BUDGET, false)
}

fn read_text_before(
    element: AxRef,
    deadline: Instant,
    require_caret: bool,
) -> Option<TextSnapshot> {
    if Instant::now() >= deadline {
        return None;
    }
    let attrs = copy_attributes(
        element,
        &[
            "AXNumberOfCharacters",
            "AXMarkedTextRange",
            "AXSelectedTextRange",
            "AXDOMClassList",
        ],
    )?;
    if array_at(&attrs, 1)
        .and_then(range_value)
        .is_some_and(|range| range.length > 0)
    {
        return Some(TextSnapshot::Composing);
    }
    let selection = array_at(&attrs, 2).and_then(range_value);
    if require_caret && !selection.is_some_and(|selection| selection.length == 0) {
        return None;
    }
    let classes = array_at(&attrs, 3);
    if selection
        .is_some_and(|selection| empty_web_placeholder(element, classes, selection, None, deadline))
    {
        return Some(TextSnapshot::Placeholder);
    }
    read_editor_text(
        element,
        classes,
        array_at(&attrs, 0).and_then(number_value)?,
        deadline,
    )
    .map(TextSnapshot::Committed)
}

fn read_editor_text(
    element: AxRef,
    classes: Option<CFTypeRef>,
    length: usize,
    deadline: Instant,
) -> Option<String> {
    read_range(element, length, deadline).or_else(|| {
        // Chromium can count paragraph separators that AXStringForRange
        // cannot address. AXValue exposes the full text including separators.
        if length > MAX_EDITOR_TEXT_UNITS
            || Instant::now() >= deadline
            || !has_class(classes, "ProseMirror")
        {
            return None;
        }
        TEXT_READS.fetch_add(1, Ordering::Relaxed);
        let value = copy_attribute(element, "AXValue")?;
        let text = bounded_string_value(value.0, MAX_EDITOR_TEXT_UNITS)?;
        (Instant::now() < deadline && text.encode_utf16().count() == length).then_some(text)
    })
}

fn read_range(element: AxRef, length: usize, deadline: Instant) -> Option<String> {
    if length > MAX_EDITOR_TEXT_UNITS || Instant::now() >= deadline {
        return None;
    }
    if length == 0 {
        return Some(String::new());
    }
    let range = CFRange {
        location: 0,
        length: length as isize,
    };
    let parameter =
        Owned::new(unsafe { AXValueCreate(AX_RANGE_TYPE, (&range as *const CFRange).cast()) })?;
    let attribute = CFString::new("AXStringForRange");
    let mut value = ptr::null();
    TEXT_READS.fetch_add(1, Ordering::Relaxed);
    if unsafe {
        AXUIElementCopyParameterizedAttributeValue(
            element,
            attribute.as_concrete_TypeRef(),
            parameter.0,
            &mut value,
        )
    } != 0
    {
        return None;
    }
    let value = Owned::new(value)?;
    let text = bounded_string_value(value.0, MAX_EDITOR_TEXT_UNITS)?;
    if Instant::now() >= deadline || text.encode_utf16().count() != length {
        return None;
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirms_web_appends_with_a_misreported_caret_or_trailing_break() {
        let original = "今天下午 3 点开会";
        let edited = "今天下午 4 点开会";
        for (baseline, caret, prefix) in [
            ("\n", 0, ""),
            ("\n", 1, ""),
            ("哈哈哈\n", 3, "哈哈哈\n"),
            ("哈哈哈\n\n", 0, "哈哈哈\n"),
        ] {
            let pending = Insertion::new(baseline, caret, 0, original.into()).unwrap();
            let verified = format!("{prefix}{original}");
            assert!(pending.confirm(&verified, false).is_none());
            let confirmed = confirm_web_append(baseline, original, &verified).unwrap();
            assert_eq!(
                confirmed.extract(&format!("{prefix}{edited}")).as_deref(),
                Some(edited)
            );
        }
    }

    #[test]
    fn web_append_preserves_the_draft_and_requires_exact_dictation() {
        for invalid in ["前文语音", "改了\n语音", "前文\n改过的语音", "前文\n语音\n"]
        {
            assert!(confirm_web_append("前文\n\n", "语音", invalid).is_none());
        }
        let confirmed = confirm_web_append("前文\n\n", "语音", "前文\n\n语音").unwrap();
        assert_eq!(
            confirmed.extract("前文\n\n新语音").as_deref(),
            Some("新语音")
        );
        assert!(confirmed.extract("改了\n\n新语音").is_none());
    }

    #[test]
    fn decodes_editor_snapshots_larger_than_the_learning_sample_limit() {
        let draft = "😀".repeat(MAX_TEXT_UNITS / 2 + 1);
        let value = CFString::new(&draft);
        assert!(string_value(value.as_CFTypeRef()).is_none());
        assert_eq!(
            bounded_string_value(value.as_CFTypeRef(), MAX_EDITOR_TEXT_UNITS).as_deref(),
            Some(draft.as_str())
        );
    }

    fn state(target: &CFString) -> CallbackState {
        CallbackState {
            target: target.as_CFTypeRef(),
            value_changed: CFString::new("AXValueChanged"),
            destroyed: CFString::new("AXUIElementDestroyed"),
            deactivated: CFString::new("AXApplicationDeactivated"),
            injecting: true,
            end: None,
            debounce: Debounce::default(),
        }
    }

    fn notify(state: &mut CallbackState, element: &CFString, event: &str) {
        notification(
            ptr::null(),
            element.as_CFTypeRef(),
            CFString::new(event).as_concrete_TypeRef(),
            (state as *mut CallbackState).cast(),
        );
    }

    #[test]
    fn thousands_of_self_injection_events_do_not_read_text_or_mark_edits() {
        let target = CFString::new("target");
        let mut state = state(&target);
        let reads = TEXT_READS.load(Ordering::Relaxed);
        for _ in 0..5_000 {
            notify(&mut state, &target, "AXValueChanged");
        }
        assert!(state.debounce.dirty_at.is_none());
        assert!(state.end.is_none());
        assert_eq!(TEXT_READS.load(Ordering::Relaxed), reads);
        state.injecting = false;
        notify(&mut state, &target, "AXValueChanged");
        assert!(state.debounce.dirty_at.is_some());
        assert_eq!(TEXT_READS.load(Ordering::Relaxed), reads);
    }

    #[test]
    fn unrelated_elements_and_late_notifications_cannot_reopen_scope() {
        let target = CFString::new("target");
        let other = CFString::new("other");
        let mut state = state(&target);
        state.injecting = false;
        notify(&mut state, &other, "AXValueChanged");
        assert!(state.debounce.dirty_at.is_none());
        notify(&mut state, &target, "AXUIElementDestroyed");
        assert!(state.end == Some(End::Destroyed));
        notify(&mut state, &other, "AXFocusedUIElementChanged");
        notify(&mut state, &target, "AXValueChanged");
        assert!(state.end == Some(End::Destroyed));
        assert!(state.debounce.dirty_at.is_none());
    }

    #[test]
    fn losing_focus_during_injection_invalidates_the_session() {
        let target = CFString::new("target");
        let mut state = state(&target);
        notify(&mut state, &target, "AXApplicationDeactivated");
        assert!(state.end == Some(End::Deactivated));
        assert!(state.debounce.dirty_at.is_none());
    }

    #[test]
    fn leaving_the_app_ends_submission_even_if_the_composer_then_disappears() {
        let target = CFString::new("target");
        let mut state = state(&target);
        notify(&mut state, &target, "AXFocusedUIElementChanged");
        notify(&mut state, &target, "AXApplicationDeactivated");
        notify(&mut state, &target, "AXUIElementDestroyed");
        assert!(state.end == Some(End::Deactivated));
    }
}
