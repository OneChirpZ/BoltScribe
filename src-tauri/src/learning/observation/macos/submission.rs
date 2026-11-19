use super::submit_tap::SubmitInput;
use super::submit_target::SubmitTarget;
use super::{
    copy_attribute, read_text_before_detailed, AxFailure, AxRef, Control, Owned, Scope,
    TextSnapshot, READ_BUDGET,
};
use core_foundation_sys::base::CFEqual;
use objc2_app_kit::NSWorkspace;
use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const CAPTURE_BUDGET: Duration = Duration::from_millis(85);
const PRE_DISPATCH_CLEAR_BUDGET: Duration = Duration::from_millis(40);
const CLICK_LIMIT: Duration = Duration::from_millis(750);
const CLEAR_CHECKS_MS: [u64; 5] = [0, 35, 100, 220, 450];
const CLEAR_LIMIT: Duration = Duration::from_millis(550);

pub(super) struct SubmitCapture<'a> {
    target: SubmitTarget,
    pid: i32,
    app: AxRef,
    editor: AxRef,
    control: &'a Control,
    scope: &'a RefCell<Scope>,
    suspended: Cell<bool>,
    pending: RefCell<Option<Pending>>,
    input_version: Cell<u64>,
    failure: Cell<Option<(Instant, &'static str, AxFailure)>>,
}

enum Pending {
    Press { button: Owned, expires: Instant },
    Clear(ClearCheck),
    Confirmed(String),
}

struct ClearCheck {
    text: String,
    started: Instant,
    next: usize,
}

#[derive(PartialEq, Eq)]
enum SubmitTrigger {
    Click,
    Return,
}

pub(super) enum Submission {
    None,
    Waiting(Instant),
    Confirmed(String),
}

impl<'a> SubmitCapture<'a> {
    pub(super) fn new(
        bundle_id: &str,
        pid: i32,
        app: AxRef,
        editor: AxRef,
        control: &'a Control,
        scope: &'a RefCell<Scope>,
    ) -> Option<Self> {
        Some(Self {
            target: SubmitTarget::identify(editor, bundle_id)?,
            pid,
            app,
            editor,
            control,
            scope,
            suspended: Cell::new(false),
            pending: RefCell::new(None),
            input_version: Cell::new(0),
            failure: Cell::new(None),
        })
    }

    pub(super) fn suspend(&self) {
        self.suspended.set(true);
        self.pending.take();
        self.failure.set(None);
    }

    pub(super) fn resume(&self) {
        self.suspended.set(false);
    }

    pub(super) fn take_failure(&self) -> Option<(&'static str, AxFailure)> {
        let (at, phase, failure) = self.failure.take()?;
        (at.elapsed() <= CLEAR_LIMIT).then_some((phase, failure))
    }

    fn record_failure(&self, phase: &'static str, failure: AxFailure) {
        self.failure.set(Some((Instant::now(), phase, failure)));
    }

    // This runs before dispatch on the observer thread. Only a matching
    // Return or click release starts a capture, and all events proceed.
    pub(super) fn on_input(&self, input: SubmitInput) {
        self.input_version
            .set(self.input_version.get().wrapping_add(1));
        if self.suspended.get() {
            return;
        }
        let previous = self.pending.take();
        if matches!(previous, Some(Pending::Confirmed(_))) {
            self.pending.replace(previous);
            return;
        }
        self.failure.set(None);
        if input == SubmitInput::Disabled {
            self.record_failure("submit", AxFailure::at("tap_disabled"));
            return;
        }
        if (input == SubmitInput::Other && !matches!(previous, Some(Pending::Clear(_))))
            || !self.is_active()
        {
            return;
        }
        let now = Instant::now();
        let deadline = now + CAPTURE_BUDGET;
        let previous = match previous {
            Some(Pending::Clear(check)) => {
                // Confirm a preceding Send before dispatching another input.
                // Afterwards that input could be what cleared the editor.
                let deadline = (now + PRE_DISPATCH_CLEAR_BUDGET).min(check.started + CLEAR_LIMIT);
                let snapshot = read_text_before_detailed(self.editor, deadline, false);
                let failure = snapshot.as_ref().err().copied();
                let result = check.snapshot(&self.scope.borrow(), snapshot);
                if self.has_send_evidence(result, deadline) {
                    self.pending.replace(Some(Pending::Confirmed(check.text)));
                    return;
                }
                if let Some(failure) = failure {
                    self.record_failure("confirm_send", failure);
                }
                None
            }
            previous => previous,
        };
        let pending = match input {
            SubmitInput::MouseDown { x, y } if self.editor_focused() => self
                .target
                .send_button_at(self.app, x, y, deadline)
                .map(|button| Pending::Press {
                    button,
                    expires: Instant::now() + CLICK_LIMIT,
                }),
            SubmitInput::MouseUp { x, y } => {
                let Some(Pending::Press { button, expires }) = previous else {
                    return;
                };
                if Instant::now() >= expires {
                    return;
                }
                let Some(released) = self.target.send_button_at(self.app, x, y, deadline) else {
                    return;
                };
                if unsafe { CFEqual(button.0, released.0) } == 0 {
                    return;
                }
                self.capture(deadline, SubmitTrigger::Click)
            }
            SubmitInput::Return
                if self.editor_focused() && self.target.is_current_page(deadline) =>
            {
                self.capture(deadline, SubmitTrigger::Return)
            }
            _ => None,
        };
        self.pending.replace(pending);
    }

    pub(super) fn check(&self, editor_destroyed: bool) -> Submission {
        let Some(pending) = self.pending.take() else {
            return Submission::None;
        };
        if !self.is_active() {
            return Submission::None;
        }
        let now = Instant::now();
        match pending {
            Pending::Confirmed(text) => Submission::Confirmed(text),
            Pending::Press { expires, .. } if !editor_destroyed && now < expires => {
                self.pending.replace(Some(pending));
                Submission::Waiting(expires)
            }
            Pending::Clear(check) if now < check.started + CLEAR_LIMIT => {
                if !editor_destroyed {
                    return self.check_clear(check, now);
                }
                if self.has_send_evidence(ClearResult::Removed, check.started + CLEAR_LIMIT) {
                    Submission::Confirmed(check.text)
                } else {
                    self.unconfirmed_clear(None)
                }
            }
            Pending::Clear(_) => self.unconfirmed_clear(None),
            _ => Submission::None,
        }
    }

    fn check_clear(&self, mut check: ClearCheck, now: Instant) -> Submission {
        let Some(delay) = CLEAR_CHECKS_MS.get(check.next) else {
            return Submission::None;
        };
        let next_at = check.started + Duration::from_millis(*delay);
        if now < next_at {
            self.pending.replace(Some(Pending::Clear(check)));
            return Submission::Waiting(next_at);
        }
        let version = self.input_version.get();
        let clear_deadline = check.started + CLEAR_LIMIT;
        let snapshot = read_text_before_detailed(
            self.editor,
            (Instant::now() + READ_BUDGET).min(clear_deadline),
            false,
        );
        if self.input_version.get() != version {
            return Submission::Waiting(Instant::now());
        }
        if !self.is_active() {
            return Submission::None;
        }
        let failure = snapshot.as_ref().err().copied();
        if Instant::now() >= clear_deadline {
            return self.unconfirmed_clear(failure);
        }
        let result = check.snapshot(&self.scope.borrow(), snapshot);
        match result {
            result @ (ClearResult::Cleared | ClearResult::Removed) => {
                let confirmed = self.has_send_evidence(result, clear_deadline);
                if self.input_version.get() != version {
                    return Submission::Waiting(Instant::now());
                }
                if confirmed {
                    Submission::Confirmed(check.text)
                } else {
                    self.unconfirmed_clear(failure)
                }
            }
            ClearResult::Edited => Submission::None,
            ClearResult::Waiting => {
                check.next += 1;
                if let Some(delay) = CLEAR_CHECKS_MS.get(check.next) {
                    let next_at = check.started + Duration::from_millis(*delay);
                    self.pending.replace(Some(Pending::Clear(check)));
                    Submission::Waiting(next_at)
                } else {
                    self.unconfirmed_clear(failure)
                }
            }
        }
    }

    fn has_send_evidence(&self, result: ClearResult, deadline: Instant) -> bool {
        let confirmed = match result {
            ClearResult::Cleared => true,
            ClearResult::Removed => self.target.is_current_page(deadline),
            ClearResult::Waiting | ClearResult::Edited => false,
        };
        confirmed && Instant::now() < deadline && self.is_active()
    }

    fn unconfirmed_clear(&self, failure: Option<AxFailure>) -> Submission {
        self.record_failure(
            "confirm_send",
            failure.unwrap_or_else(|| AxFailure::at("clear_unconfirmed")),
        );
        Submission::None
    }

    fn is_active(&self) -> bool {
        !self.suspended.get()
            && !self.control.cancelled.load(Ordering::Acquire)
            && NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .is_some_and(|app| app.processIdentifier() == self.pid)
    }

    fn editor_focused(&self) -> bool {
        copy_attribute(self.app, "AXFocusedUIElement")
            .is_some_and(|focused| unsafe { CFEqual(focused.0, self.editor) != 0 })
    }

    fn capture(&self, deadline: Instant, trigger: SubmitTrigger) -> Option<Pending> {
        let snapshot =
            read_text_before_detailed(self.editor, deadline, trigger == SubmitTrigger::Return)
                .and_then(|snapshot| scoped_submit_text(&self.scope.borrow(), snapshot, deadline));
        let text = match snapshot {
            Ok(text) => text,
            Err(failure) => {
                self.record_failure("before_send", failure);
                return None;
            }
        };
        Some(Pending::Clear(ClearCheck {
            text,
            started: Instant::now(),
            next: 0,
        }))
    }
}

fn scoped_submit_text(
    scope: &Scope,
    snapshot: TextSnapshot,
    deadline: Instant,
) -> Result<String, AxFailure> {
    let snapshot = match snapshot {
        TextSnapshot::Committed(text) => text,
        TextSnapshot::Composing => return Err(AxFailure::at("submit_snapshot_composing")),
        TextSnapshot::Placeholder => return Err(AxFailure::at("submit_snapshot_placeholder")),
    };
    let text = scope
        .extract(&snapshot)
        .ok_or_else(|| AxFailure::at("submit_snapshot_out_of_scope"))?;
    if Instant::now() >= deadline {
        return Err(AxFailure::at("submit_snapshot_timeout"));
    }
    Ok(text)
}

#[derive(Debug, PartialEq, Eq)]
enum ClearResult {
    Cleared,
    Removed,
    Waiting,
    Edited,
}

impl ClearCheck {
    fn snapshot(&self, scope: &Scope, snapshot: Result<TextSnapshot, AxFailure>) -> ClearResult {
        match snapshot {
            Ok(TextSnapshot::Placeholder) => ClearResult::Cleared,
            Ok(TextSnapshot::Committed(value)) if value.trim().is_empty() => ClearResult::Cleared,
            Ok(TextSnapshot::Committed(value))
                if scope.extract(&value).as_deref() == Some(self.text.as_str()) =>
            {
                ClearResult::Waiting
            }
            // Chrome can invalidate the old composer before its AX destroyed
            // notification is delivered. Only this specific error proves removal.
            Err(failure) if failure.editor_invalid() => ClearResult::Removed,
            Err(_) => ClearResult::Waiting,
            _ => ClearResult::Edited,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::observation::Insertion;

    #[test]
    fn pre_dispatch_capture_extracts_the_current_edit_without_waiting_for_debounce() {
        let scope = Scope::new(Insertion::new("原有段落\n", 5, 0, "用科德克斯".into()).unwrap());
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            scoped_submit_text(
                &scope,
                TextSnapshot::Committed("原有段落\n用 Codex".into()),
                deadline,
            )
            .ok()
            .as_deref(),
            Some("用 Codex")
        );
        for (snapshot, step) in [
            (
                TextSnapshot::Committed("其他段落\n用 Codex".into()),
                "submit_snapshot_out_of_scope",
            ),
            (TextSnapshot::Composing, "submit_snapshot_composing"),
            (TextSnapshot::Placeholder, "submit_snapshot_placeholder"),
        ] {
            assert_eq!(
                scoped_submit_text(&scope, snapshot, deadline)
                    .err()
                    .unwrap()
                    .step,
                step
            );
        }
    }

    #[test]
    fn submission_keeps_both_dictations_and_excludes_intervening_draft_text() {
        let scope = Scope::new(Insertion::new("前文后文", 2, 0, "初始语音".into()).unwrap());
        let baseline = "前文修改后的语音后文";
        let next = Insertion::new(
            baseline,
            baseline.encode_utf16().count(),
            0,
            "另一段语音".into(),
        )
        .unwrap();
        let scope = scope.merge(baseline, &next).unwrap();
        let snapshot = "前文修改后的语音后文另一段修改";
        let deadline = Instant::now() + Duration::from_secs(1);
        let text = scoped_submit_text(&scope, TextSnapshot::Committed(snapshot.into()), deadline)
            .ok()
            .unwrap();
        let check = ClearCheck {
            text,
            started: Instant::now(),
            next: 0,
        };
        assert_eq!(
            check.snapshot(&scope, Ok(TextSnapshot::Committed(snapshot.into()))),
            ClearResult::Waiting
        );
        assert_eq!(
            check.snapshot(&scope, Ok(TextSnapshot::Placeholder)),
            ClearResult::Cleared
        );
        assert_eq!(check.text, "修改后的语音\n另一段修改");
    }

    #[test]
    fn return_that_inserts_newline_or_confirms_ime_does_not_submit() {
        let scope = Scope::new(Insertion::new("", 0, 0, "初始语音".into()).unwrap());
        let check = ClearCheck {
            text: "手动改过".into(),
            started: Instant::now(),
            next: 0,
        };
        assert_eq!(
            check.snapshot(&scope, Ok(TextSnapshot::Committed("手动改过\n".into()))),
            ClearResult::Edited
        );
        assert_eq!(
            check.snapshot(&scope, Ok(TextSnapshot::Composing)),
            ClearResult::Edited
        );
        assert_eq!(
            check.snapshot(&scope, Err(AxFailure::at("read_budget"))),
            ClearResult::Waiting
        );
        assert_eq!(
            check.snapshot(&scope, Ok(TextSnapshot::Committed(String::new()))),
            ClearResult::Cleared
        );
    }

    #[test]
    fn recognizes_editor_removal_without_waiting_for_the_ax_notification() {
        let scope = Scope::new(Insertion::new("", 0, 0, "初始语音".into()).unwrap());
        let check = ClearCheck {
            text: "修改后的语音".into(),
            started: Instant::now(),
            next: 0,
        };
        assert_eq!(
            check.snapshot(
                &scope,
                Err(AxFailure::native("AXCopyMultipleAttributeValues", -25202))
            ),
            ClearResult::Removed
        );
        assert_eq!(
            check.snapshot(
                &scope,
                Err(AxFailure::native("AXCopyMultipleAttributeValues", -25204))
            ),
            ClearResult::Waiting
        );
    }
}
