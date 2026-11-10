use super::submit_tap::SubmitInput;
use super::submit_target::SubmitTarget;
use super::{
    copy_attribute, read_text, read_text_before, AxRef, Control, Insertion, Owned, TextSnapshot,
    MESSAGE_TIMEOUT,
};
use core_foundation_sys::base::CFEqual;
use objc2_app_kit::NSWorkspace;
use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const CAPTURE_BUDGET: Duration = Duration::from_millis(85);
const CLICK_LIMIT: Duration = Duration::from_millis(750);
const CLEAR_CHECKS_MS: [u64; 4] = [35, 100, 220, 450];
const CLEAR_LIMIT: Duration = Duration::from_millis(550);

pub(super) struct SubmitCapture<'a> {
    target: SubmitTarget,
    pid: i32,
    app: AxRef,
    editor: AxRef,
    control: &'a Control,
    insertion: &'a Insertion,
    pending: RefCell<Option<Pending>>,
    input_version: Cell<u64>,
}

enum Pending {
    Press { button: Owned, expires: Instant },
    Clear(ClearCheck),
}

struct ClearCheck {
    text: String,
    started: Instant,
    next: usize,
    can_confirm_removed: bool,
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
        insertion: &'a Insertion,
    ) -> Option<Self> {
        Some(Self {
            target: SubmitTarget::identify(editor, bundle_id)?,
            pid,
            app,
            editor,
            control,
            insertion,
            pending: RefCell::new(None),
            input_version: Cell::new(0),
        })
    }

    // This runs before dispatch on the observer thread. Only a matching
    // Return or click release may read text, and the event always proceeds.
    pub(super) fn on_input(&self, input: SubmitInput) {
        self.input_version
            .set(self.input_version.get().wrapping_add(1));
        let previous = self.pending.take();
        if matches!(input, SubmitInput::Other | SubmitInput::Disabled) || !self.is_active() {
            return;
        }
        let deadline = Instant::now() + CAPTURE_BUDGET;
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
            SubmitInput::Return if self.editor_focused() => {
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
            Pending::Press { expires, .. } if !editor_destroyed && now < expires => {
                self.pending.replace(Some(pending));
                Submission::Waiting(expires)
            }
            Pending::Clear(check) if now < check.started + CLEAR_LIMIT => {
                if !editor_destroyed {
                    return self.check_clear(check, now);
                }
                // Opening a new chat can replace the composer. Require
                // evidence that its send action was available before removal.
                check.removed()
            }
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
        let snapshot = read_text(self.editor);
        if self.input_version.get() != version {
            return Submission::Waiting(Instant::now());
        }
        if Instant::now() >= check.started + CLEAR_LIMIT || !self.is_active() {
            return Submission::None;
        }
        match check.snapshot(self.insertion, snapshot) {
            ClearResult::Cleared => Submission::Confirmed(check.text),
            ClearResult::Edited => Submission::None,
            ClearResult::Waiting => {
                check.next += 1;
                if let Some(delay) = CLEAR_CHECKS_MS.get(check.next) {
                    let next_at = check.started + Duration::from_millis(*delay);
                    self.pending.replace(Some(Pending::Clear(check)));
                    Submission::Waiting(next_at)
                } else {
                    Submission::None
                }
            }
        }
    }

    fn is_active(&self) -> bool {
        !self.control.cancelled.load(Ordering::Acquire)
            && !self.control.finishing.load(Ordering::Acquire)
            && NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .is_some_and(|app| app.processIdentifier() == self.pid)
    }

    fn editor_focused(&self) -> bool {
        copy_attribute(self.app, "AXFocusedUIElement")
            .is_some_and(|focused| unsafe { CFEqual(focused.0, self.editor) != 0 })
    }

    fn capture(&self, deadline: Instant, trigger: SubmitTrigger) -> Option<Pending> {
        let TextSnapshot::Committed(snapshot) =
            read_text_before(self.editor, deadline, trigger == SubmitTrigger::Return)?
        else {
            return None;
        };
        let text = self.insertion.extract(&snapshot)?;
        let started = Instant::now();
        if started >= deadline {
            return None;
        }
        let can_confirm_removed = trigger == SubmitTrigger::Click
            || (deadline.duration_since(started) > Duration::from_secs_f32(MESSAGE_TIMEOUT)
                && self.target.has_enabled_send(deadline));
        Some(Pending::Clear(ClearCheck {
            text,
            started,
            next: 0,
            can_confirm_removed,
        }))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ClearResult {
    Cleared,
    Waiting,
    Edited,
}

impl ClearCheck {
    fn removed(self) -> Submission {
        if self.can_confirm_removed {
            Submission::Confirmed(self.text)
        } else {
            Submission::None
        }
    }

    fn snapshot(&self, insertion: &Insertion, snapshot: Option<TextSnapshot>) -> ClearResult {
        match snapshot {
            Some(TextSnapshot::Placeholder) => ClearResult::Cleared,
            Some(TextSnapshot::Committed(value)) if value.trim().is_empty() => ClearResult::Cleared,
            Some(TextSnapshot::Committed(value))
                if insertion.extract(&value).as_deref() == Some(self.text.as_str()) =>
            {
                ClearResult::Waiting
            }
            None => ClearResult::Waiting,
            _ => ClearResult::Edited,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_keeps_pre_clear_dictation_with_draft_context() {
        let insertion = Insertion::new("前文后文", 2, 0, "初始语音".into()).unwrap();
        let check = ClearCheck {
            text: "修改后的语音".into(),
            started: Instant::now(),
            next: 0,
            can_confirm_removed: true,
        };
        assert_eq!(
            check.snapshot(
                &insertion,
                Some(TextSnapshot::Committed("前文修改后的语音后文".into()))
            ),
            ClearResult::Waiting
        );
        assert_eq!(
            check.snapshot(&insertion, Some(TextSnapshot::Placeholder)),
            ClearResult::Cleared
        );
        assert_eq!(check.text, "修改后的语音");
    }

    #[test]
    fn return_that_inserts_newline_or_confirms_ime_does_not_submit() {
        let insertion = Insertion::new("", 0, 0, "初始语音".into()).unwrap();
        let check = ClearCheck {
            text: "手动改过".into(),
            started: Instant::now(),
            next: 0,
            can_confirm_removed: false,
        };
        assert_eq!(
            check.snapshot(
                &insertion,
                Some(TextSnapshot::Committed("手动改过\n".into()))
            ),
            ClearResult::Edited
        );
        assert_eq!(
            check.snapshot(&insertion, Some(TextSnapshot::Composing)),
            ClearResult::Edited
        );
        assert_eq!(check.snapshot(&insertion, None), ClearResult::Waiting);
        assert_eq!(
            check.snapshot(&insertion, Some(TextSnapshot::Committed(String::new()))),
            ClearResult::Cleared
        );
    }

    #[test]
    fn replacement_editor_requires_evidence_of_an_enabled_send_action() {
        for can_confirm_removed in [false, true] {
            let check = ClearCheck {
                text: "修改后的语音".into(),
                started: Instant::now(),
                next: 0,
                can_confirm_removed,
            };
            match check.removed() {
                Submission::Confirmed(text) if can_confirm_removed => {
                    assert_eq!(text, "修改后的语音");
                }
                Submission::None if !can_confirm_removed => {}
                _ => panic!("unexpected outcome for a replaced composer"),
            }
        }
    }
}
