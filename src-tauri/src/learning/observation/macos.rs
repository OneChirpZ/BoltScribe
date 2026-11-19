use super::scope::Scope;
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
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::AtomicU8;
use std::sync::mpsc;

mod submission;
mod submit_tap;
mod submit_target;
mod web_offsets;

use submission::{Submission, SubmitCapture};
use submit_tap::SubmitTap;

type AxRef = *const c_void;
const MESSAGE_TIMEOUT: f32 = 0.025;
const PREPARE_TIMEOUT: Duration = Duration::from_millis(150);
const READ_BUDGET: Duration = Duration::from_millis(125);
const VERIFY_DELAYS_MS: [u64; 7] = [80, 160, 320, 500, 800, 1_200, 2_000];
const STANDARD_VERIFY_ATTEMPTS: usize = 3;
const INJECTION_LIMIT: Duration = Duration::from_secs(30);
const AX_RANGE_TYPE: u32 = 4;
const AX_ERROR_TYPE: u32 = 5;
const AX_INVALID_UI_ELEMENT: i32 = -25202;

#[derive(Clone, Copy)]
struct AxFailure {
    step: &'static str,
    code: Option<i32>,
}

impl AxFailure {
    fn at(step: &'static str) -> Self {
        Self { step, code: None }
    }

    fn native(step: &'static str, code: i32) -> Self {
        Self {
            step,
            code: Some(code),
        }
    }

    fn editor_invalid(self) -> bool {
        self.code == Some(AX_INVALID_UI_ELEMENT)
    }
}

#[derive(Clone, Copy)]
struct CaptureFailure {
    reason: &'static str,
    phase: &'static str,
    ax: Option<AxFailure>,
    mismatch: Option<VerificationMismatch>,
}

#[derive(Clone, Copy)]
struct VerificationMismatch {
    baseline: Option<(TextReadSource, usize)>,
    selection: (isize, isize),
    inserted_units: usize,
    readback: (Option<TextReadSource>, usize),
    prefix_matches: bool,
    suffix_matches: bool,
    contains_insertion: bool,
    partial_insertion: bool,
}

impl CaptureFailure {
    fn new(reason: &'static str, phase: &'static str, step: &'static str) -> Self {
        Self::read(reason, phase, AxFailure::at(step))
    }

    fn read(reason: &'static str, phase: &'static str, ax: AxFailure) -> Self {
        Self {
            reason,
            phase,
            ax: Some(ax),
            mismatch: None,
        }
    }

    fn no_change() -> Self {
        Self {
            reason: "no_change",
            phase: "observe",
            ax: None,
            mismatch: None,
        }
    }

    fn diagnostic(self) -> Option<String> {
        // These fields only accept static identifiers and native error codes.
        let ax = self.ax?;
        let mut diagnostic = match ax.code {
            Some(code) => format!("AX {}.{} (error {code})", self.phase, ax.step),
            None => format!("AX {}.{}", self.phase, ax.step),
        };
        if let Some(detail) = self.mismatch {
            use std::fmt::Write;
            let baseline = detail.baseline.map_or_else(
                || "unavailable".into(),
                |(source, units)| format!("{}:{units}", source.attribute()),
            );
            let (source, units) = detail.readback;
            let readback = source.map_or_else(
                || units.to_string(),
                |source| format!("{}:{units}", source.attribute()),
            );
            let _ = write!(
                diagnostic,
                " (baseline={baseline}, selection={}:{}, inserted={}, readback={readback}, prefix={}, suffix={}, contains_insertion={}, partial={})",
                detail.selection.0, detail.selection.1, detail.inserted_units,
                detail.prefix_matches, detail.suffix_matches, detail.contains_insertion,
                detail.partial_insertion,
            );
        }
        Some(diagnostic)
    }
}

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

#[derive(Default)]
pub(super) struct Injection(AtomicU8);

const PENDING: u8 = 0;
const INJECTED: u8 = 1;
const FAILED: u8 = 2;
const EXPIRED: u8 = 3;

impl Injection {
    pub(super) fn finish(&self, success: bool) -> bool {
        self.set_status(if success { INJECTED } else { FAILED })
    }

    fn set_status(&self, status: u8) -> bool {
        self.0
            .compare_exchange(PENDING, status, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn status(&self) -> u8 {
        self.0.load(Ordering::Acquire)
    }
}

struct Batch {
    record_id: String,
    activity: Option<activity::ActivityHandle>,
    complete: Completion,
    correction: Option<Vec<CorrectionEvidence>>,
}

impl Batch {
    fn finish(self, result: Result<(String, String), CaptureFailure>, cancelled: bool) {
        match result {
            Ok((original, current)) if !cancelled => {
                COMPLETED.fetch_add(1, Ordering::Relaxed);
                (self.complete)(LearningSample {
                    activity: self.activity,
                    original,
                    current,
                    record_id: self.record_id,
                    correction: self.correction,
                });
            }
            result => {
                let failure = result.err();
                let reason = if cancelled {
                    "cancelled"
                } else {
                    failure.map_or("observation_interrupted", |failure| failure.reason)
                };
                skip(
                    self.activity.as_ref(),
                    reason,
                    failure.filter(|_| !cancelled),
                );
            }
        }
    }

    fn merge(&mut self, next: &Self) -> bool {
        let merged = match (&self.activity, &next.activity) {
            (Some(current), Some(next)) => current.merge_source(next),
            (None, None) => true,
            _ => false,
        };
        if merged {
            const MAX_CORRECTION_RECORDS: usize = 4;
            self.correction = self
                .correction
                .take()
                .zip(next.correction.as_ref())
                .and_then(|(mut current, added)| {
                    if current.len() + added.len() > MAX_CORRECTION_RECORDS {
                        return None;
                    }
                    current.extend_from_slice(added);
                    Some(current)
                });
        }
        merged
    }
}

fn skip(
    activity: Option<&activity::ActivityHandle>,
    reason: &'static str,
    failure: Option<CaptureFailure>,
) {
    SKIPPED.fetch_add(1, Ordering::Relaxed);
    if let Some(activity) = activity {
        let state = if reason == "no_change" {
            ActivityState::NoChange
        } else {
            ActivityState::Skipped
        };
        activity.update(
            state,
            Some(reason),
            vec![],
            failure.and_then(CaptureFailure::diagnostic),
        );
    }
}

pub(super) struct Request {
    batch: Batch,
    text: String,
    ready: mpsc::SyncSender<()>,
    injection: Arc<Injection>,
}

struct WorkerGuard(Arc<Control>);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let mut state = match GATE.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        if state
            .active
            .as_ref()
            .is_some_and(|active| Weak::ptr_eq(active, &Arc::downgrade(&self.0)))
        {
            state.active = None;
            WORKER_RUNNING.store(false, Ordering::Release);
            let request = take_request(&self.0);
            drop(state);
            if let Some(request) = request {
                request.batch.finish(
                    Err(CaptureFailure::new(
                        "observation_interrupted",
                        "prepare",
                        "worker_stopped",
                    )),
                    self.0.cancelled.load(Ordering::Acquire),
                );
            }
        }
    }
}

fn take_request(control: &Control) -> Option<Request> {
    control.request.lock().ok()?.take()
}

fn next_or_retire(control: &Control) -> Option<Request> {
    // Serialize retirement with prepare so a queued paste cannot be stranded.
    let mut state = GATE.state.lock().ok()?;
    let request = take_request(control);
    if request.is_none() {
        state.active = None;
        WORKER_RUNNING.store(false, Ordering::Release);
    }
    request
}

fn run_worker(control: Arc<Control>, mut request: Request) {
    let _guard = WorkerGuard(control.clone());
    loop {
        let Request {
            mut batch,
            text,
            ready,
            injection,
        } = request;
        let mut next = None;
        let result = run(&control, &mut batch, text, ready, &injection, &mut next);
        let next = next.or_else(|| next_or_retire(&control));
        let cancelled = control.cancelled.load(Ordering::Acquire);
        match next {
            Some(pending) => {
                // A new field has its own capture deadline. Do not make its
                // paste wait for the earlier sample's validation or queueing.
                std::thread::spawn(move || batch.finish(result, cancelled));
                request = pending;
            }
            None => {
                batch.finish(result, cancelled);
                break;
            }
        }
    }
}

pub(super) fn prepare(
    record_id: String,
    text: String,
    correction: CorrectionEvidence,
    complete: Completion,
    activity: Option<activity::ActivityHandle>,
) -> Option<Session> {
    let injection = Arc::new(Injection::default());
    let (ready, ready_rx) = mpsc::sync_channel(1);
    let request = Request {
        batch: Batch {
            record_id,
            activity: activity.clone(),
            complete,
            correction: Some(vec![correction]),
        },
        text,
        ready,
        injection: injection.clone(),
    };
    let control = {
        let mut state = GATE.state.lock().ok()?;
        if !state.enabled {
            skip(activity.as_ref(), "cancelled", None);
            return None;
        }
        if let Some(control) = state.active.as_ref().and_then(Weak::upgrade) {
            let mut pending = control.request.lock().ok()?;
            if control.cancelled.load(Ordering::Acquire) || pending.is_some() {
                skip(activity.as_ref(), "observer_busy", None);
                return None;
            }
            *pending = Some(request);
            drop(pending);
            control.wake();
            control
        } else {
            let control = Arc::new(Control {
                cancelled: AtomicBool::new(false),
                recordings: AtomicU64::new(0),
                request: Mutex::new(None),
                wake: Mutex::new(None),
            });
            if !state.install(&control, &WORKER_RUNNING) {
                skip(activity.as_ref(), "observer_busy", None);
                return None;
            }
            let worker_control = control.clone();
            let spawned = std::thread::Builder::new()
                .name("input-correction-observer".into())
                .spawn(move || run_worker(worker_control, request));
            if spawned.is_err() {
                state.active = None;
                WORKER_RUNNING.store(false, Ordering::Release);
                SKIPPED.fetch_add(1, Ordering::Relaxed);
                if let Some(activity) = activity {
                    activity.update(
                        ActivityState::Failed,
                        Some("worker_unavailable"),
                        vec![],
                        None,
                    );
                }
                return None;
            }
            control
        }
    };
    match ready_rx.recv_timeout(PREPARE_TIMEOUT) {
        Ok(()) => Some(Session { control, injection }),
        // The worker owns the failure detail when preparation ends early.
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            injection.set_status(EXPIRED);
            control.wake();
            if let Some(activity) = activity {
                activity.update(
                    ActivityState::Skipped,
                    Some("target_unavailable"),
                    vec![],
                    CaptureFailure::new("target_unavailable", "prepare", "timeout").diagnostic(),
                );
            }
            None
        }
    }
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
    fn new(control: &'a Control, target: AxRef, app: AxRef, pid: i32) -> Result<Self, AxFailure> {
        let mut observer = ptr::null();
        let status = unsafe { AXObserverCreate(pid, notification, &mut observer) };
        if status != 0 {
            return Err(AxFailure::native("create_observer", status));
        }
        let observer = Owned::new(observer).ok_or(AxFailure::at("create_observer"))?;
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
            Owned::new(unsafe { CFRunLoopSourceCreate(ptr::null(), 0, &mut context) }.cast())
                .ok_or(AxFailure::at("control_run_loop_source"))?;
        if observer_source.is_null() {
            return Err(AxFailure::at("observer_run_loop_source"));
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
        for (element, notification_name) in [
            (target, "AXValueChanged"),
            (target, "AXUIElementDestroyed"),
            (app, "AXFocusedUIElementChanged"),
            (app, "AXApplicationDeactivated"),
        ] {
            let name = CFString::new(notification_name);
            let context = (&mut *native.state as *mut CallbackState).cast();
            let status = unsafe {
                AXObserverAddNotification(
                    native.observer.0,
                    element,
                    name.as_concrete_TypeRef(),
                    context,
                )
            };
            if status != 0 {
                return Err(AxFailure::native(notification_name, status));
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
        Ok(native)
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

struct Editor {
    pid: i32,
    app: Owned,
    target: Owned,
    bundle_id: Option<String>,
}

impl Editor {
    fn focused() -> Result<Self, CaptureFailure> {
        let prepare_failure = |step| CaptureFailure::new("target_unavailable", "prepare", step);
        let prepare_read = |failure| CaptureFailure::read("target_unavailable", "prepare", failure);
        if !unsafe { AXIsProcessTrusted() } {
            return Err(prepare_failure("accessibility_permission"));
        }
        let frontmost = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .ok_or_else(|| prepare_failure("frontmost_application"))?;
        let pid = frontmost.processIdentifier();
        if pid <= 0 || pid as u32 == std::process::id() {
            return Err(prepare_failure("target_process"));
        }
        let app = Owned::new(unsafe { AXUIElementCreateApplication(pid) })
            .ok_or_else(|| prepare_failure("create_application"))?;
        unsafe {
            AXUIElementSetMessagingTimeout(app.0, MESSAGE_TIMEOUT);
        }
        let target = copy_attribute_detailed(app.0, "AXFocusedUIElement").map_err(prepare_read)?;
        unsafe {
            AXUIElementSetMessagingTimeout(target.0, MESSAGE_TIMEOUT);
        }
        let mut target_pid = 0;
        let status = unsafe { AXUIElementGetPid(target.0, &mut target_pid) };
        if status != 0 {
            return Err(prepare_read(AxFailure::native(
                "focused_element_process",
                status,
            )));
        }
        if target_pid != pid {
            return Err(prepare_failure("focused_element_process_mismatch"));
        }

        Ok(Self {
            pid,
            app,
            target,
            bundle_id: frontmost.bundleIdentifier().map(|id| id.to_string()),
        })
    }

    fn frontmost(&self) -> bool {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .is_some_and(|app| app.processIdentifier() == self.pid)
    }

    fn is_focused(&self) -> bool {
        self.frontmost()
            && copy_attribute(self.app.0, "AXFocusedUIElement")
                .is_some_and(|focused| unsafe { CFEqual(focused.0, self.target.0) } != 0)
    }
}

struct PendingInsertion {
    baseline: Option<EditorText>,
    insertion: Option<Insertion>,
    text: String,
    selection: CFRange,
    prosemirror: bool,
    empty_placeholder: bool,
}

impl PendingInsertion {
    fn capture(target: AxRef, text: String) -> Result<Self, CaptureFailure> {
        let prepare_failure = |step| CaptureFailure::new("target_unavailable", "prepare", step);
        let prepare_read = |failure| CaptureFailure::read("target_unavailable", "prepare", failure);
        let started = Instant::now();
        let attrs = copy_attributes_detailed(
            target,
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
        )
        .map_err(prepare_read)?;
        let role = array_at(&attrs, 0)
            .and_then(string_value)
            .ok_or_else(|| prepare_read(attribute_failure("AXRole", array_at(&attrs, 0))))?;
        let subrole = array_at(&attrs, 1)
            .and_then(string_value)
            .unwrap_or_default();
        if !matches!(role.as_str(), "AXTextField" | "AXTextArea") {
            return Err(prepare_failure("unsupported_role"));
        }
        if subrole == "AXSecureTextField"
            || array_at(&attrs, 2).is_some_and(boolean_value)
            || array_at(&attrs, 3).is_some_and(boolean_value)
        {
            return Err(prepare_failure("protected_field"));
        }
        if array_at(&attrs, 6)
            .and_then(range_value)
            .is_some_and(|range| range.length > 0)
        {
            return Err(prepare_failure("marked_text"));
        }
        let selection = array_at(&attrs, 5).and_then(range_value).ok_or_else(|| {
            prepare_read(attribute_failure(
                "AXSelectedTextRange",
                array_at(&attrs, 5),
            ))
        })?;
        let length = array_at(&attrs, 4).and_then(number_value);
        let classes = array_at(&attrs, 7);
        let deadline = started + READ_BUDGET;
        let baseline = length
            .ok_or_else(|| attribute_failure("AXNumberOfCharacters", array_at(&attrs, 4)))
            .and_then(|length| read_editor_text(target, classes, length, deadline));
        let baseline_text = baseline.as_ref().ok();
        let empty_placeholder = empty_web_placeholder(target, classes, selection, length, deadline);
        let insertion = baseline_text
            .and_then(|baseline| {
                Insertion::new(
                    &baseline.text,
                    usize::try_from(selection.location).ok()?,
                    usize::try_from(selection.length).ok()?,
                    text.clone(),
                )
            })
            .or_else(|| {
                // Hidden suggestions may not support a consistent text range.
                // This candidate still requires exact dictation readback below.
                empty_placeholder
                    .then(|| Insertion::new("", 0, 0, text.clone()))
                    .flatten()
            });
        let prosemirror = has_class(classes, "ProseMirror");
        let web_draft = baseline_text.filter(|_| selection.length == 0 && prosemirror);
        let offset_draft = web_draft.filter(|draft| {
            draft.source == TextReadSource::Value
                && selection.location >= 0
                && draft.text.contains('\n')
        });
        if insertion.is_none() && offset_draft.is_none() {
            return Err(prepare_read(
                baseline
                    .as_ref()
                    .err()
                    .copied()
                    .unwrap_or(AxFailure::at("insertion_range")),
            ));
        }
        if started.elapsed() >= READ_BUDGET {
            return Err(prepare_failure("read_budget"));
        }
        Ok(Self {
            baseline: baseline.ok(),
            insertion,
            text,
            selection,
            prosemirror,
            empty_placeholder,
        })
    }

    fn verify(
        &self,
        editor: &Editor,
        native: &mut NativeObserver,
        ready: mpsc::SyncSender<()>,
        injection: &Injection,
    ) -> Result<Insertion, CaptureFailure> {
        let control = native.control;
        if !editor.frontmost()
            || control.cancelled.load(Ordering::Acquire)
            || injection.status() != PENDING
            || native.state.end.is_some()
        {
            return Err(CaptureFailure::new(
                "target_unavailable",
                "prepare",
                "target_lost",
            ));
        }
        ready
            .send(())
            .map_err(|_| CaptureFailure::new("target_unavailable", "prepare", "ready_channel"))?;

        let injection_deadline = Instant::now() + INJECTION_LIMIT;
        while injection.status() == PENDING {
            if control.cancelled.load(Ordering::Acquire)
                || native.state.end.is_some()
                || Instant::now() >= injection_deadline
            {
                return Err(CaptureFailure::new(
                    "target_unavailable",
                    "injection",
                    "waiting_for_injector",
                ));
            }
            native.wait_until(injection_deadline);
        }
        if injection.status() != INJECTED || control.cancelled.load(Ordering::Acquire) {
            return Err(CaptureFailure::new(
                "target_unavailable",
                "injection",
                "injector_failed",
            ));
        }
        let injected_at = Instant::now();
        let Self {
            baseline,
            insertion,
            text,
            selection,
            prosemirror,
            empty_placeholder,
        } = self;
        let baseline_text = baseline.as_ref();
        let web_draft = baseline_text.filter(|_| selection.length == 0 && *prosemirror);
        let offset_draft = web_draft.filter(|draft| {
            draft.source == TextReadSource::Value
                && selection.location >= 0
                && draft.text.contains('\n')
        });
        let mut confirmed = None;
        let confirm = |verified: &str| {
            insertion
                .as_ref()
                .and_then(|insertion| insertion.confirm(verified, *empty_placeholder))
                .or_else(|| confirm_web_append(&web_draft?.text, text, verified))
                .or_else(|| {
                    web_offsets::confirm(
                        &offset_draft?.text,
                        usize::try_from(selection.location).ok()?,
                        text,
                        verified,
                    )
                })
                .or_else(|| web_offsets::confirm_layout(&web_draft?.text, text, verified))
        };
        let is_partial = |verified: &str| {
            baseline_text.is_some_and(|baseline| baseline.text == verified)
                || insertion.as_ref().is_some_and(|anchors| {
                    partial_at_anchors(&anchors.prefix, &anchors.suffix, text, verified)
                })
                || web_draft.is_some_and(|draft| partial_web_append(&draft.text, text, verified))
                || offset_draft.is_some_and(|draft| {
                    usize::try_from(selection.location).is_ok_and(|caret| {
                        web_offsets::is_partial(&draft.text, caret, text, verified)
                    })
                })
        };
        let mut verification_failure =
            CaptureFailure::new("target_unavailable", "verify", "no_snapshot");
        let verify_focus = || CaptureFailure::new("focus_changed", "verify", "target_lost");
        let mut partial_seen = false;
        for (attempt, delay_ms) in VERIFY_DELAYS_MS.into_iter().enumerate() {
            // An exact fragment may be visible to AX before the editor finishes
            // applying the posted key events. Never confirm a fragment or retype it.
            if attempt >= STANDARD_VERIFY_ATTEMPTS && !partial_seen {
                break;
            }
            let verify_at = injected_at + Duration::from_millis(delay_ms);
            while Instant::now() < verify_at {
                if native.state.end.is_some() || control.cancelled.load(Ordering::Acquire) {
                    return Err(verify_focus());
                }
                native.wait_until(verify_at);
            }
            if !editor.frontmost()
                || native.state.end.is_some()
                || control.cancelled.load(Ordering::Acquire)
            {
                return Err(verify_focus());
            }
            let focused = match copy_attribute_detailed(editor.app.0, "AXFocusedUIElement") {
                Ok(focused) => focused,
                Err(failure) => {
                    verification_failure =
                        CaptureFailure::read("target_unavailable", "verify", failure);
                    continue;
                }
            };
            if unsafe { CFEqual(focused.0, editor.target.0) } == 0 {
                return Err(verify_focus());
            }
            let read_deadline = Instant::now() + READ_BUDGET;
            let snapshot = match read_text_with_source(editor.target.0, read_deadline, false) {
                Err(AxFailure {
                    step: "text_length_mismatch",
                    ..
                }) if *prosemirror => {
                    // The character count and whole value may update at different
                    // times. Exact insertion proof below still applies.
                    read_verification_value(editor.target.0, read_deadline)
                        .map(|text| (TextSnapshot::Committed(text), Some(TextReadSource::Value)))
                }
                snapshot => snapshot,
            };
            verification_failure = match snapshot {
                Ok((TextSnapshot::Committed(mut verified), mut source)) => {
                    confirmed = confirm(&verified);
                    if confirmed.is_some() {
                        break;
                    }
                    let mut partial = is_partial(&verified);
                    if *prosemirror && source != Some(TextReadSource::Value) {
                        if let Ok(value) = read_verification_value(editor.target.0, read_deadline) {
                            confirmed = confirm(&value);
                            if confirmed.is_some() {
                                break;
                            }
                            if is_partial(&value) {
                                partial = true;
                                verified = value;
                                source = Some(TextReadSource::Value);
                            }
                        }
                    }
                    partial_seen |= partial;
                    let mut failure =
                        CaptureFailure::new("injection_mismatch", "verify", "insertion_mismatch");
                    failure.mismatch = Some(VerificationMismatch {
                        baseline: baseline_text
                            .map(|draft| (draft.source, draft.text.encode_utf16().count())),
                        selection: (selection.location, selection.length),
                        inserted_units: text.encode_utf16().count(),
                        readback: (source, verified.encode_utf16().count()),
                        prefix_matches: insertion
                            .as_ref()
                            .is_some_and(|anchors| verified.starts_with(&anchors.prefix)),
                        suffix_matches: insertion
                            .as_ref()
                            .is_some_and(|anchors| verified.ends_with(&anchors.suffix)),
                        contains_insertion: verified.contains(text),
                        partial_insertion: partial,
                    });
                    failure
                }
                Ok((TextSnapshot::Composing, _)) => {
                    CaptureFailure::new("target_unavailable", "verify", "marked_text")
                }
                Ok((TextSnapshot::Placeholder, _)) => {
                    CaptureFailure::new("target_unavailable", "verify", "placeholder")
                }
                Err(failure) => CaptureFailure::read("target_unavailable", "verify", failure),
            };
        }
        confirmed.ok_or(verification_failure)
    }
}

fn run(
    control: &Control,
    batch: &mut Batch,
    text: String,
    ready: mpsc::SyncSender<()>,
    injection: &Injection,
    next: &mut Option<Request>,
) -> Result<(String, String), CaptureFailure> {
    if injection.status() != PENDING || control.cancelled.load(Ordering::Acquire) {
        return Err(CaptureFailure::new("cancelled", "prepare", "cancelled"));
    }
    let editor = Editor::focused()?;
    let pending = PendingInsertion::capture(editor.target.0, text)?;
    let mut native = NativeObserver::new(control, editor.target.0, editor.app.0, editor.pid)
        .map_err(|failure| CaptureFailure::read("target_unavailable", "register", failure))?;
    let insertion = pending.verify(&editor, &mut native, ready, injection)?;
    let scope = RefCell::new(Scope::new(insertion));
    native.state.injecting = false;
    native.state.debounce = Debounce::default();
    let submit_capture = editor.bundle_id.as_deref().and_then(|bundle_id| {
        SubmitCapture::new(
            bundle_id,
            editor.pid,
            editor.app.0,
            editor.target.0,
            control,
            &scope,
        )
    });
    let submit_tap = submit_capture
        .as_ref()
        .and_then(|capture| SubmitTap::new(editor.pid, |input| capture.on_input(input)));
    let submit_failure = |reason| {
        submit_capture
            .as_ref()
            .and_then(SubmitCapture::take_failure)
            .map(|(phase, failure)| CaptureFailure::read(reason, phase, failure))
    };
    let mut last_committed = None;
    let mut observation_started = Instant::now();
    let current = loop {
        let now = Instant::now();
        let idle_deadline = native.state.debounce.idle_deadline(observation_started);
        let recording = control.recordings.load(Ordering::Acquire) != 0;
        if control.cancelled.load(Ordering::Acquire) {
            return Err(CaptureFailure::new("cancelled", "observe", "cancelled"));
        }
        let destroyed = native.state.end == Some(End::Destroyed);
        if let Some(capture) = submit_capture
            .as_ref()
            .filter(|_| submit_tap.is_some() && native.state.end != Some(End::Deactivated))
        {
            match capture.check(destroyed) {
                Submission::Confirmed(text) => break text,
                Submission::Waiting(next) => {
                    native.wait_until(next);
                    continue;
                }
                Submission::None => {}
            }
        }
        *next = take_request(control);
        // The injector proceeds after its preparation budget even when the
        // observer is still busy. Do not read that unverified paste as an edit.
        let settled = native.state.debounce.dirty_at.is_none();
        if prepare_expired(next.as_ref()) {
            break settled_before_paste(last_committed, settled)?;
        }
        if native.state.end.is_some()
            || next.as_ref().is_some_and(|_| !editor.is_focused())
            || (next.is_none() && !recording && now >= idle_deadline)
        {
            let snapshot = if destroyed {
                Err(AxFailure::native("editor_destroyed", AX_INVALID_UI_ELEMENT))
            } else {
                // A final committed read can include an edit whose AX notification
                // has not arrived yet. Always read the original editor.
                read_text_detailed(editor.target.0)
            };
            if prepare_expired(next.as_ref()) {
                break settled_before_paste(last_committed, settled)?;
            }
            break finish_observation(&scope.borrow(), last_committed, snapshot, settled)
                .map_err(|failure| submit_failure(failure.reason).unwrap_or(failure))?;
        }
        if let Some(Request {
            batch: next_batch,
            text,
            ready,
            injection,
        }) = next.take()
        {
            if let Some(capture) = &submit_capture {
                capture.suspend();
            }
            native.state.injecting = true;
            let pending = match PendingInsertion::capture(editor.target.0, text) {
                Ok(pending) => pending,
                Err(failure) => {
                    let snapshot = read_text_detailed(editor.target.0);
                    drop(ready); // Allow the workflow to paste only after the final read.
                    next_batch.finish(Err(failure), control.cancelled.load(Ordering::Acquire));
                    if injection.status() == EXPIRED {
                        break settled_before_paste(last_committed, settled)?;
                    }
                    break finish_observation(&scope.borrow(), last_committed, snapshot, false)?;
                }
            };
            let before = pending.baseline.as_ref().map(|draft| draft.text.as_str());
            let previous = before.and_then(|text| scope.borrow().extract(text));
            let confirmed = match pending.verify(&editor, &mut native, ready, &injection) {
                Ok(confirmed) => confirmed,
                Err(failure) => {
                    // Only the pre-paste snapshot is safe after an unverified paste.
                    next_batch.finish(Err(failure), control.cancelled.load(Ordering::Acquire));
                    if injection.status() == EXPIRED {
                        break settled_before_paste(last_committed, settled)?;
                    }
                    break previous.ok_or_else(|| {
                        CaptureFailure::new("out_of_scope", "observe", "insertion_range")
                    })?;
                }
            };

            let merged = before.and_then(|draft| scope.borrow().merge(draft, &confirmed));
            if let Some(merged) = merged.filter(|_| batch.merge(&next_batch)) {
                scope.replace(merged);
            } else {
                // Ambiguous or overlapping placements form a new scope.
                let earlier = previous
                    .ok_or_else(|| {
                        CaptureFailure::new("out_of_scope", "observe", "insertion_range")
                    })
                    .and_then(|current| scoped_pair(&scope.borrow(), current));
                let earlier_batch = std::mem::replace(batch, next_batch);
                let cancelled = control.cancelled.load(Ordering::Acquire);
                std::thread::spawn(move || earlier_batch.finish(earlier, cancelled));
                scope.replace(Scope::new(confirmed));
            }
            native.state.injecting = false;
            native.state.debounce = Debounce::default();
            last_committed = None;
            observation_started = Instant::now();
            if let Some(capture) = &submit_capture {
                capture.resume();
            }
            continue;
        }
        if native.state.debounce.ready(now) {
            match read_text_detailed(editor.target.0).map_err(|failure| {
                CaptureFailure::read("observation_interrupted", "observe", failure)
            })? {
                TextSnapshot::Committed(snapshot) => {
                    if snapshot.trim().is_empty() {
                        return Err(CaptureFailure::new("input_cleared", "observe", "empty"));
                    }
                    last_committed = scope.borrow().extract(&snapshot);
                }
                TextSnapshot::Placeholder => {
                    return Err(CaptureFailure::new(
                        "input_cleared",
                        "observe",
                        "placeholder",
                    ));
                }
                TextSnapshot::Composing => last_committed = None,
            }
            // An IME composition can pause while its candidate list is open.
            // Wait for its next change or the idle timeout without polling it.
            native.state.debounce.dirty_at = None;
        }
        let deadline = if recording && now >= idle_deadline {
            now + OBSERVATION_IDLE_TIMEOUT
        } else {
            idle_deadline
        };
        native.wait_until(native.state.debounce.next_deadline(deadline));
    };
    drop(submit_tap);
    let pair = scoped_pair(&scope.borrow(), current);
    pair.map_err(|failure| submit_failure(failure.reason).unwrap_or(failure))
}

fn prepare_expired(request: Option<&Request>) -> bool {
    request.is_some_and(|request| request.injection.status() == EXPIRED)
}

fn settled_before_paste(
    last_committed: Option<String>,
    settled: bool,
) -> Result<String, CaptureFailure> {
    last_committed.filter(|_| settled).ok_or_else(|| {
        CaptureFailure::new("observation_interrupted", "observe", "next_prepare_timeout")
    })
}

fn scoped_pair(scope: &Scope, current: String) -> Result<(String, String), CaptureFailure> {
    let original = scope.original();
    if current == original {
        return Err(CaptureFailure::no_change());
    }
    Ok((original, current))
}

fn finish_observation(
    scope: &Scope,
    last_committed: Option<String>,
    snapshot: Result<TextSnapshot, AxFailure>,
    settled: bool,
) -> Result<String, CaptureFailure> {
    match snapshot {
        Ok(TextSnapshot::Committed(text)) if text.trim().is_empty() => {
            Err(CaptureFailure::new("input_cleared", "final_read", "empty"))
        }
        Ok(TextSnapshot::Committed(text)) => scope
            .extract(&text)
            .ok_or_else(|| CaptureFailure::new("out_of_scope", "final_read", "insertion_range")),
        Ok(TextSnapshot::Placeholder) => Err(CaptureFailure::new(
            "input_cleared",
            "final_read",
            "placeholder",
        )),
        Ok(TextSnapshot::Composing) => Err(CaptureFailure::new(
            "unstable_edit",
            "final_read",
            "marked_text",
        )),
        Err(failure) => {
            if settled && failure.editor_invalid() {
                // The original editor is gone. Keep only the last committed,
                // scoped read, provided no later change is awaiting inspection.
                if let Some(text) = last_committed {
                    return Ok(text);
                }
            }
            Err(CaptureFailure::read(
                if settled {
                    "observation_interrupted"
                } else {
                    "unstable_edit"
                },
                "final_read",
                failure,
            ))
        }
    }
}

fn copy_attribute(element: AxRef, name: &'static str) -> Option<Owned> {
    copy_attribute_detailed(element, name).ok()
}

fn copy_attribute_detailed(element: AxRef, name: &'static str) -> Result<Owned, AxFailure> {
    let attribute = CFString::new(name);
    let mut value = ptr::null();
    let status = unsafe {
        AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value)
    };
    if status != 0 {
        return Err(AxFailure::native(name, status));
    }
    Owned::new(value).ok_or(AxFailure::at(name))
}

fn copy_attributes(element: AxRef, names: &[&str]) -> Option<Owned> {
    copy_attributes_detailed(element, names).ok()
}

fn copy_attributes_detailed(element: AxRef, names: &[&str]) -> Result<Owned, AxFailure> {
    let names: Vec<CFString> = names.iter().map(|name| CFString::new(name)).collect();
    let names = CFArray::from_CFTypes(&names);
    let mut values = ptr::null();
    let status = unsafe {
        AXUIElementCopyMultipleAttributeValues(element, names.as_concrete_TypeRef(), 0, &mut values)
    };
    if status != 0 {
        return Err(AxFailure::native("AXCopyMultipleAttributeValues", status));
    }
    Owned::new(values.cast()).ok_or(AxFailure::at("AXCopyMultipleAttributeValues"))
}

fn array_at(array: &Owned, index: isize) -> Option<CFTypeRef> {
    let array = array.0 as CFArrayRef;
    (index >= 0 && index < unsafe { CFArrayGetCount(array) })
        .then(|| unsafe { CFArrayGetValueAtIndex(array, index) })
}

fn attribute_failure(step: &'static str, value: Option<CFTypeRef>) -> AxFailure {
    let Some(value) = value
        .filter(|value| !value.is_null() && unsafe { CFGetTypeID(*value) == AXValueGetTypeID() })
    else {
        return AxFailure::at(step);
    };
    let mut code = 0i32;
    if unsafe { AXValueGetValue(value, AX_ERROR_TYPE, (&mut code as *mut i32).cast()) } {
        AxFailure::native(step, code)
    } else {
        AxFailure::at(step)
    }
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextReadSource {
    Range,
    Value,
}

impl TextReadSource {
    fn attribute(self) -> &'static str {
        match self {
            Self::Range => "AXStringForRange",
            Self::Value => "AXValue",
        }
    }
}

struct EditorText {
    text: String,
    source: TextReadSource,
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

fn partial_at_anchors(prefix: &str, suffix: &str, original: &str, verified: &str) -> bool {
    verified
        .strip_prefix(prefix)
        .and_then(|text| text.strip_suffix(suffix))
        .is_some_and(|fragment| fragment.len() < original.len() && original.starts_with(fragment))
}

fn partial_web_append(baseline: &str, original: &str, verified: &str) -> bool {
    let Some(without_break) = baseline.strip_suffix('\n') else {
        return false;
    };
    partial_at_anchors(baseline, "", original, verified)
        || ((without_break.is_empty() || without_break.ends_with('\n'))
            && partial_at_anchors(without_break, "", original, verified))
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

fn read_text_detailed(element: AxRef) -> Result<TextSnapshot, AxFailure> {
    read_text_before_detailed(element, Instant::now() + READ_BUDGET, false)
}

fn read_text_before_detailed(
    element: AxRef,
    deadline: Instant,
    require_caret: bool,
) -> Result<TextSnapshot, AxFailure> {
    read_text_with_source(element, deadline, require_caret).map(|(snapshot, _)| snapshot)
}

fn read_text_with_source(
    element: AxRef,
    deadline: Instant,
    require_caret: bool,
) -> Result<(TextSnapshot, Option<TextReadSource>), AxFailure> {
    if Instant::now() >= deadline {
        return Err(AxFailure::at("read_budget"));
    }
    let attrs = copy_attributes_detailed(
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
        return Ok((TextSnapshot::Composing, None));
    }
    let selection = array_at(&attrs, 2).and_then(range_value);
    if require_caret {
        let selection = selection
            .ok_or_else(|| attribute_failure("AXSelectedTextRange", array_at(&attrs, 2)))?;
        if selection.length != 0 {
            return Err(AxFailure::at("non_caret_selection"));
        }
    }
    let classes = array_at(&attrs, 3);
    if selection
        .is_some_and(|selection| empty_web_placeholder(element, classes, selection, None, deadline))
    {
        return Ok((TextSnapshot::Placeholder, None));
    }
    read_editor_text(
        element,
        classes,
        array_at(&attrs, 0)
            .and_then(number_value)
            .ok_or_else(|| attribute_failure("AXNumberOfCharacters", array_at(&attrs, 0)))?,
        deadline,
    )
    .map(|snapshot| {
        (
            TextSnapshot::Committed(snapshot.text),
            Some(snapshot.source),
        )
    })
}

fn read_verification_value(element: AxRef, deadline: Instant) -> Result<String, AxFailure> {
    if Instant::now() >= deadline {
        return Err(AxFailure::at("read_budget"));
    }
    TEXT_READS.fetch_add(1, Ordering::Relaxed);
    let value = copy_attribute_detailed(element, "AXValue")?;
    let text = bounded_string_value(value.0, MAX_EDITOR_TEXT_UNITS)
        .ok_or_else(|| attribute_failure("AXValue", Some(value.0)))?;
    if Instant::now() >= deadline {
        return Err(AxFailure::at("read_budget"));
    }
    Ok(text)
}

fn read_editor_text(
    element: AxRef,
    classes: Option<CFTypeRef>,
    length: usize,
    deadline: Instant,
) -> Result<EditorText, AxFailure> {
    read_range(element, length, deadline)
        .map(|text| EditorText {
            text,
            source: TextReadSource::Range,
        })
        .or_else(|range_failure| {
            // Chromium can count paragraph separators that AXStringForRange
            // cannot address. AXValue exposes the full text including separators.
            if length > MAX_EDITOR_TEXT_UNITS
                || Instant::now() >= deadline
                || !has_class(classes, "ProseMirror")
            {
                return Err(range_failure);
            }
            TEXT_READS.fetch_add(1, Ordering::Relaxed);
            let value = copy_attribute_detailed(element, "AXValue")?;
            editor_string(value, length, deadline, "AXValue").map(|text| EditorText {
                text,
                source: TextReadSource::Value,
            })
        })
}

fn read_range(element: AxRef, length: usize, deadline: Instant) -> Result<String, AxFailure> {
    if length > MAX_EDITOR_TEXT_UNITS {
        return Err(AxFailure::at("text_length_limit"));
    }
    if Instant::now() >= deadline {
        return Err(AxFailure::at("read_budget"));
    }
    if length == 0 {
        return Ok(String::new());
    }
    let range = CFRange {
        location: 0,
        length: length as isize,
    };
    let parameter =
        Owned::new(unsafe { AXValueCreate(AX_RANGE_TYPE, (&range as *const CFRange).cast()) })
            .ok_or(AxFailure::at("create_text_range"))?;
    let attribute = CFString::new("AXStringForRange");
    let mut value = ptr::null();
    TEXT_READS.fetch_add(1, Ordering::Relaxed);
    let status = unsafe {
        AXUIElementCopyParameterizedAttributeValue(
            element,
            attribute.as_concrete_TypeRef(),
            parameter.0,
            &mut value,
        )
    };
    if status != 0 {
        return Err(AxFailure::native("AXStringForRange", status));
    }
    let value = Owned::new(value).ok_or(AxFailure::at("AXStringForRange"))?;
    editor_string(value, length, deadline, "AXStringForRange")
}

fn editor_string(
    value: Owned,
    length: usize,
    deadline: Instant,
    attribute: &'static str,
) -> Result<String, AxFailure> {
    let text = bounded_string_value(value.0, MAX_EDITOR_TEXT_UNITS)
        .ok_or_else(|| attribute_failure(attribute, Some(value.0)))?;
    if Instant::now() >= deadline {
        return Err(AxFailure::at("read_budget"));
    }
    if text.encode_utf16().count() != length {
        return Err(AxFailure::at("text_length_mismatch"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_timeout_cannot_confirm_the_late_paste_or_change_the_prior_result() {
        let prior = Injection::default();
        assert!(prior.finish(true));
        let next = Injection::default();
        assert!(next.set_status(EXPIRED));
        assert!(!next.finish(true));
        assert_eq!(next.status(), EXPIRED);
        assert_eq!(prior.status(), INJECTED);
        assert!(settled_before_paste(Some("可能已撤销的修改".into()), false).is_err());
        assert_eq!(
            settled_before_paste(Some("已核对的修改".into()), true)
                .ok()
                .as_deref(),
            Some("已核对的修改")
        );
    }

    #[test]
    fn finishing_reads_the_latest_committed_edit_without_waiting_for_debounce() {
        let scope = Scope::new(Insertion::new("前文后文", 2, 0, "口述".into()).unwrap());
        let edited = finish_observation(
            &scope,
            Some("第一次修改".into()),
            Ok(TextSnapshot::Committed("前文最后一次修改后文".into())),
            false,
        );
        assert_eq!(edited.ok().as_deref(), Some("最后一次修改"));
    }

    #[test]
    fn closing_the_editor_keeps_only_a_settled_committed_sample() {
        let scope = Scope::new(Insertion::new("前文后文", 2, 0, "口述".into()).unwrap());
        let invalid = AxFailure::native("AXCopyMultipleAttributeValues", AX_INVALID_UI_ELEMENT);
        let edited = finish_observation(&scope, Some("修改".into()), Err(invalid), true);
        assert_eq!(edited.ok().as_deref(), Some("修改"));
        assert!(finish_observation(&scope, Some("修改".into()), Err(invalid), false).is_err());
        assert!(finish_observation(&scope, None, Err(invalid), true).is_err());
        assert!(finish_observation(
            &scope,
            Some("修改".into()),
            Ok(TextSnapshot::Composing),
            true,
        )
        .is_err());
        assert!(finish_observation(
            &scope,
            Some("修改".into()),
            Ok(TextSnapshot::Placeholder),
            true,
        )
        .is_err());
    }

    #[test]
    fn partial_verification_preserves_the_whole_draft() {
        let draft = "第一段\n\n";
        let spoken = "语音还在上屏";
        assert!(partial_web_append(draft, spoken, "第一段\n语音"));
        assert!(partial_web_append(draft, spoken, "第一段\n\n语音"));
        assert!(!partial_web_append(draft, spoken, "第一段\n语音还在上屏"));
        assert!(!partial_web_append(draft, spoken, "改了前文\n语音"));
        assert!(!partial_web_append(draft, spoken, "第一段语音"));
        assert!(partial_at_anchors("前文", "后文", spoken, "前文语音后文"));
        assert!(!partial_at_anchors(
            "前文",
            "后文",
            spoken,
            "前文语音改后文"
        ));
    }

    #[test]
    fn reports_the_stage_and_error_of_a_missing_batched_attribute() {
        let code = -25205i32;
        let value =
            Owned::new(unsafe { AXValueCreate(AX_ERROR_TYPE, (&code as *const i32).cast()) })
                .unwrap();
        let failure = attribute_failure("AXSelectedTextRange", Some(value.0));
        assert_eq!(
            CaptureFailure::read("target_unavailable", "prepare", failure)
                .diagnostic()
                .as_deref(),
            Some("AX prepare.AXSelectedTextRange (error -25205)")
        );
    }

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
