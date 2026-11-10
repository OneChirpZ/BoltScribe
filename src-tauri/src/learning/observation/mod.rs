//! A bounded, event-driven observation of one confirmed dictation insertion.
//! Native handles and notification delivery stay on the platform worker.

#[cfg(any(target_os = "macos", test))]
use super::MAX_TEXT_UNITS;
use super::{
    activity::{self, ActivityState},
    LearningSample,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
#[cfg(any(target_os = "macos", test))]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
mod macos;

// The editor snapshot includes local anchors around the learning sample.
#[cfg(any(target_os = "macos", test))]
const MAX_EDITOR_TEXT_UNITS: usize = 32_000;
#[cfg(any(target_os = "macos", test))]
pub(super) const QUIET_PERIOD: Duration = Duration::from_secs(1);
#[cfg(any(target_os = "macos", test))]
pub(super) const OBSERVATION_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

static GATE: RuntimeGate = RuntimeGate {
    state: Mutex::new(GateState {
        enabled: false,
        active: None,
    }),
};
static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
static NOTIFICATIONS: AtomicU64 = AtomicU64::new(0);
static TEXT_READS: AtomicU64 = AtomicU64::new(0);
static COMPLETED: AtomicU64 = AtomicU64::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "macos")]
type Completion = Box<dyn FnOnce(LearningSample) + Send>;

pub struct Session {
    control: Arc<Control>,
    committed: bool,
}

impl Session {
    /// Call immediately after the injector returns. Success still requires native
    /// readback; it is never treated as evidence that text reached the field.
    pub fn injected(mut self, success: bool) {
        self.committed = success;
        #[cfg(target_os = "macos")]
        self.control
            .injected
            .store(if success { 1 } else { 2 }, Ordering::Release);
        if !success {
            self.control.cancelled.store(true, Ordering::Release);
        }
        self.control.wake();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.committed {
            self.control.cancelled.store(true, Ordering::Release);
            self.control.wake();
        }
    }
}

struct Control {
    cancelled: AtomicBool,
    finishing: AtomicBool,
    #[cfg(target_os = "macos")]
    injected: std::sync::atomic::AtomicU8,
    #[cfg(target_os = "macos")]
    wake: Mutex<Option<macos::WakeHandle>>,
}

#[derive(Default)]
struct GateState {
    enabled: bool,
    active: Option<Weak<Control>>,
}

#[derive(Default)]
struct RuntimeGate {
    state: Mutex<GateState>,
}

impl RuntimeGate {
    fn enabled(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.enabled)
    }

    #[cfg(any(target_os = "macos", test))]
    fn install(&self, control: &Arc<Control>, running: &AtomicBool) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if !state.enabled
            || running
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return false;
        }
        state.active = Some(Arc::downgrade(control));
        true
    }

    fn set_enabled(&self, enabled: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.enabled = enabled;
            if !enabled {
                if let Some(control) = state.active.as_ref().and_then(Weak::upgrade) {
                    control.cancelled.store(true, Ordering::Release);
                    control.wake();
                }
            }
        }
    }
}

impl Control {
    fn wake(&self) {
        #[cfg(target_os = "macos")]
        if let Ok(wake) = self.wake.lock() {
            if let Some(wake) = wake.as_ref() {
                wake.signal();
            }
        }
    }
}

/// Preparing is optional and bounded. A busy/unsupported/unreadable target must
/// never prevent the original dictation from being injected.
pub fn prepare(
    record_id: String,
    text: String,
    on_complete: impl FnOnce(LearningSample) + Send + 'static,
) -> Option<Session> {
    if !GATE.enabled() {
        return None;
    }
    let config = crate::config::ConfigStore::load()
        .ok()
        .map(|config| config.learning_model());
    let activity = activity::begin(&record_id, config.as_ref());
    #[cfg(target_os = "macos")]
    {
        if text.trim().is_empty() || text.encode_utf16().count() > MAX_TEXT_UNITS {
            SKIPPED.fetch_add(1, Ordering::Relaxed);
            if let Some(activity) = activity {
                activity.update(ActivityState::Skipped, Some("invalid_sample"), vec![], None);
            }
            return None;
        }
        macos::prepare(record_id, text, Box::new(on_complete), activity)
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(activity) = activity {
            activity.update(ActivityState::Skipped, Some("unsupported"), vec![], None);
        }
        let _ = (record_id, text, on_complete);
        None
    }
}

/// Installation and disabling share the gate lock. A recording holding an old
/// configuration can therefore never install a fresh observer after disabling.
pub fn set_enabled(enabled: bool) {
    GATE.set_enabled(enabled);
}

pub fn cancel() {
    if let Ok(state) = GATE.state.lock() {
        if let Some(control) = state.active.as_ref().and_then(Weak::upgrade) {
            control.cancelled.store(true, Ordering::Release);
            control.wake();
        }
    }
}

pub fn finish() {
    if let Ok(state) = GATE.state.lock() {
        if let Some(control) = state.active.as_ref().and_then(Weak::upgrade) {
            control.finishing.store(true, Ordering::Release);
            control.wake();
        }
    }
}

pub fn supported() -> bool {
    cfg!(target_os = "macos")
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ObservationStats {
    pub supported: bool,
    pub active: bool,
    pub notifications: u64,
    pub text_reads: u64,
    pub completed: u64,
    pub skipped: u64,
}

pub fn stats() -> ObservationStats {
    ObservationStats {
        supported: supported(),
        active: WORKER_RUNNING.load(Ordering::Acquire),
        notifications: NOTIFICATIONS.load(Ordering::Relaxed),
        text_reads: TEXT_READS.load(Ordering::Relaxed),
        completed: COMPLETED.load(Ordering::Relaxed),
        skipped: SKIPPED.load(Ordering::Relaxed),
    }
}

/// UTF-16 offsets come from native text APIs. Once confirmed, the prefix and
/// suffix must remain unchanged to keep surrounding content out of the sample.
#[derive(Clone, Debug)]
#[cfg(any(target_os = "macos", test))]
struct Insertion {
    prefix: String,
    suffix: String,
    original: String,
}

#[cfg(any(target_os = "macos", test))]
impl Insertion {
    fn new(baseline: &str, start: usize, length: usize, original: String) -> Option<Self> {
        let units: Vec<u16> = baseline.encode_utf16().collect();
        let end = start.checked_add(length)?;
        if end > units.len()
            || units.len() > MAX_EDITOR_TEXT_UNITS
            || units.len() - length + original.encode_utf16().count() > MAX_EDITOR_TEXT_UNITS
        {
            return None;
        }
        let prefix = String::from_utf16(&units[..start]).ok()?;
        let suffix = String::from_utf16(&units[end..]).ok()?;
        Some(Self {
            prefix,
            suffix,
            original,
        })
    }

    fn matches_injection(&self, text: &str) -> bool {
        self.extract(text)
            .is_some_and(|value| value == self.original)
    }

    fn confirm(&self, text: &str, empty_placeholder: bool) -> Option<Self> {
        if self.matches_injection(text) {
            return Some(self.clone());
        }
        // Chromium can turn a trailing non-breaking space into a regular
        // space once the insertion follows it. Keep the verified space for later reads.
        if let Some(prefix) = self.prefix.strip_suffix('\u{a0}') {
            let insertion = Self {
                prefix: format!("{prefix} "),
                ..self.clone()
            };
            if insertion.matches_injection(text) {
                return Some(insertion);
            }
        }
        // Some web editors expose generated placeholder/suggestion text as
        // their value. Accept its disappearance only after exact readback of
        // our own insertion, with no surrounding text.
        (empty_placeholder && text == self.original).then(|| Self {
            prefix: String::new(),
            suffix: String::new(),
            original: self.original.clone(),
        })
    }

    fn extract(&self, text: &str) -> Option<String> {
        if text.encode_utf16().count() > MAX_EDITOR_TEXT_UNITS {
            return None;
        }
        let middle = text
            .strip_prefix(&self.prefix)?
            .strip_suffix(&self.suffix)?;
        (!middle.trim().is_empty()).then(|| middle.to_string())
    }
}

#[derive(Default)]
#[cfg(any(target_os = "macos", test))]
struct Debounce {
    dirty_at: Option<Instant>,
    last_changed_at: Option<Instant>,
}

#[cfg(any(target_os = "macos", test))]
impl Debounce {
    fn changed(&mut self, now: Instant) {
        self.dirty_at = Some(now);
        self.last_changed_at = Some(now);
    }

    fn idle_deadline(&self, observation_started: Instant) -> Instant {
        self.last_changed_at.unwrap_or(observation_started) + OBSERVATION_IDLE_TIMEOUT
    }

    fn ready(&self, now: Instant) -> bool {
        self.dirty_at
            .is_some_and(|at| now.saturating_duration_since(at) >= QUIET_PERIOD)
    }

    fn next_deadline(&self, deadline: Instant) -> Instant {
        self.dirty_at
            .map_or(deadline, |at| (at + QUIET_PERIOD).min(deadline))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::validation::sample_rejection;

    #[test]
    fn extracts_only_the_inserted_segment() {
        let insertion = Insertion::new("前文 后文", 3, 0, "科德克斯".into()).unwrap();
        assert!(insertion.matches_injection("前文 科德克斯后文"));
        assert_eq!(
            insertion.extract("前文 Codex后文").as_deref(),
            Some("Codex")
        );
        assert!(insertion.extract("前言 Codex后文").is_none());
        assert!(insertion.extract("前文 Codex后记").is_none());
        assert!(insertion.extract("").is_none());
    }

    #[test]
    fn confirms_dictation_after_placeholder_or_unaccepted_prompt_disappears() {
        for decoration in ["和 Codex 一起工作\n", "建议的下一条 prompt", ""] {
            let pending = Insertion::new(decoration, 0, 0, "用科德克斯写文档".into()).unwrap();
            let insertion = pending.confirm("用科德克斯写文档", true).unwrap();
            assert_eq!(
                insertion.extract("用Codex写文档").as_deref(),
                Some("用Codex写文档")
            );
            assert!(!insertion.matches_injection("建议的下一条 prompt"));
        }
    }

    #[test]
    fn confirms_after_the_editor_normalizes_the_space_before_the_insertion() {
        let draft = "前一段已写好。\n具体来说，\u{a0}";
        let pending =
            Insertion::new(draft, draft.encode_utf16().count(), 0, "用波特思考".into()).unwrap();
        let readback = "前一段已写好。\n具体来说， 用波特思考";
        assert!(!pending.matches_injection(readback));
        let confirmed = pending.confirm(readback, false).unwrap();
        assert_eq!(
            confirmed
                .extract("前一段已写好。\n具体来说， 用BoltScribe")
                .as_deref(),
            Some("用BoltScribe")
        );
        for invalid in [
            "改写了前一段。\n具体来说， 用波特思考",
            "前一段已写好。\n具体来说，用波特思考",
            "前一段已写好。\n具体来说， 用波特",
        ] {
            assert!(pending.confirm(invalid, false).is_none());
        }
    }

    #[test]
    fn requires_both_placeholder_evidence_and_exact_injection_readback() {
        let pending = Insertion::new("提示文字\n", 0, 0, "本轮语音".into()).unwrap();
        assert!(pending.confirm("本轮语音", false).is_none());
        assert!(pending.confirm("本轮语音\n", true).is_none());
        assert!(pending.confirm("前文本轮语音", true).is_none());
        assert!(pending.confirm("本轮语音其他草稿", true).is_none());
        assert!(pending.confirm("其他文字", true).is_none());
    }

    #[test]
    fn preserves_real_drafts_and_accepted_suggestions_including_placeholder_like_text() {
        let pending = Insertion::new("提示文字\n", 0, 0, "本轮语音".into()).unwrap();
        // Even if decoration metadata is stale, the original scope wins.
        let insertion = pending.confirm("本轮语音提示文字\n", true).unwrap();
        assert_eq!(
            insertion.extract("修改后语音提示文字\n").as_deref(),
            Some("修改后语音")
        );
        assert!(insertion.extract("修改后语音").is_none());
        let draft = Insertion::new("😀已有\n草稿", 4, 1, "本轮语音".into()).unwrap();
        let insertion = draft.confirm("😀已有本轮语音草稿", false).unwrap();
        assert_eq!(
            insertion.extract("😀已有纠错后语音草稿").as_deref(),
            Some("纠错后语音")
        );
        assert!(insertion.extract("😀改写纠错后语音草稿").is_none());
    }

    #[test]
    fn selection_replacement_and_emoji_use_utf16_offsets() {
        let insertion = Insertion::new("😀旧词尾", 2, 2, "波特".into()).unwrap();
        assert!(insertion.matches_injection("😀波特尾"));
        assert_eq!(insertion.extract("😀Bolt尾").as_deref(), Some("Bolt"));
        assert!(Insertion::new("😀旧词尾", 1, 0, "x".into()).is_none());
        assert!(Insertion::new("text", 2, usize::MAX, "x".into()).is_none());
    }

    #[test]
    fn appending_to_a_multiline_draft_keeps_learning_scoped_to_the_new_dictation() {
        let paragraph = "前一段已写好。\n\n";
        for draft in [
            paragraph.to_owned() + "具体来说，",
            paragraph.repeat(1_000) + "具体来说，",
        ] {
            let original = "用波特思考写文档";
            let insertion =
                Insertion::new(&draft, draft.encode_utf16().count(), 0, original.into())
                    .unwrap()
                    .confirm(&format!("{draft}{original}"), false)
                    .unwrap();
            for (edited, rejection) in [
                ("用BoltScribe写文档".to_owned(), None),
                ("明天上午开会见面".to_owned(), Some("unrelated_edit")),
                ("长".repeat(MAX_TEXT_UNITS + 1), Some("invalid_sample")),
            ] {
                let current = insertion.extract(&format!("{draft}{edited}")).unwrap();
                assert_eq!(current, edited);
                assert_eq!(
                    sample_rejection(&LearningSample {
                        activity: None,
                        original: insertion.original.clone(),
                        current,
                        record_id: "test".into(),
                    }),
                    rejection
                );
            }
        }
    }

    #[test]
    fn editor_snapshot_bounds_apply_before_and_after_insertion() {
        let draft = "😀".repeat(MAX_EDITOR_TEXT_UNITS / 2);
        assert!(Insertion::new(&draft, 0, 0, "x".into()).is_none());
        assert!(Insertion::new(&format!("{draft}x"), 0, 2, "x".into()).is_none());
        let insertion = Insertion::new(&draft, 0, 2, "文".into()).unwrap();
        let tail = &draft["😀".len()..];
        assert!(insertion.matches_injection(&format!("文{tail}")));
        assert!(insertion.extract(&format!("文稿好{tail}")).is_none());
    }

    #[test]
    fn no_events_means_no_text_read_deadline() {
        let now = Instant::now();
        let debounce = Debounce::default();
        let end = debounce.idle_deadline(now);
        assert_eq!(end, now + Duration::from_secs(45));
        assert!(!debounce.ready(end));
        assert_eq!(debounce.next_deadline(end), end);
    }

    #[test]
    fn burst_and_ime_intermediate_notifications_coalesce_before_read() {
        let now = Instant::now();
        let mut debounce = Debounce::default();
        for index in 0..100 {
            let at = now + Duration::from_millis(index * 10);
            debounce.changed(at);
            assert!(!debounce.ready(at));
        }
        assert!(!debounce.ready(now + Duration::from_millis(1989)));
        assert!(debounce.ready(now + Duration::from_millis(1990)));
        debounce.dirty_at = None;
        assert!(!debounce.ready(now + Duration::from_secs(60)));
    }

    #[test]
    fn editing_extends_observation_until_45_seconds_after_the_last_change() {
        let started = Instant::now();
        let mut debounce = Debounce::default();
        // Continue editing well beyond the former fixed 60-second window.
        for seconds in [40, 80, 120] {
            let changed_at = started + Duration::from_secs(seconds);
            debounce.changed(changed_at);
            assert_eq!(
                debounce.idle_deadline(started),
                changed_at + Duration::from_secs(45)
            );
            assert_eq!(
                debounce.next_deadline(debounce.idle_deadline(started)),
                changed_at + QUIET_PERIOD
            );
            // Taking a debounced snapshot does not restart or forget the idle clock.
            debounce.dirty_at = None;
            assert_eq!(
                debounce.next_deadline(debounce.idle_deadline(started)),
                changed_at + Duration::from_secs(45)
            );
        }
        let deadline = debounce.idle_deadline(started);
        assert!(started + Duration::from_secs(164) < deadline);
        assert_eq!(deadline, started + Duration::from_secs(165));
    }

    fn new_control() -> Arc<Control> {
        Arc::new(Control {
            cancelled: AtomicBool::new(false),
            finishing: AtomicBool::new(false),
            #[cfg(target_os = "macos")]
            injected: std::sync::atomic::AtomicU8::new(0),
            #[cfg(target_os = "macos")]
            wake: Mutex::new(None),
        })
    }

    #[test]
    fn disabled_prepare_has_no_text_reads_or_callback() {
        // Gate mutation tests below use independent gates, so this read-only
        // default-state test cannot enable AX or interfere with another test.
        assert!(!GATE.enabled());
        let reads = TEXT_READS.load(Ordering::Relaxed);
        assert!(prepare("test".into(), "never observed".into(), |_| panic!(
            "disabled callback"
        ))
        .is_none());
        assert_eq!(TEXT_READS.load(Ordering::Relaxed), reads);
    }

    #[test]
    fn disabling_and_reenabling_does_not_revive_an_installed_session() {
        let gate = RuntimeGate::default();
        let running = AtomicBool::new(false);
        let control = new_control();
        assert!(!gate.install(&control, &running));
        assert!(!running.load(Ordering::Acquire));
        gate.set_enabled(true);
        assert!(gate.install(&control, &running));
        gate.set_enabled(false);
        assert!(control.cancelled.load(Ordering::Acquire));
        gate.set_enabled(true);
        assert!(control.cancelled.load(Ordering::Acquire));
        assert!(!gate.install(&new_control(), &running));
    }

    #[test]
    fn concurrent_disable_and_install_cannot_leave_a_live_session() {
        for _ in 0..32 {
            let gate = Arc::new(RuntimeGate::default());
            gate.set_enabled(true);
            let control = new_control();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let install_gate = gate.clone();
            let install_control = control.clone();
            let install_barrier = barrier.clone();
            let thread = std::thread::spawn(move || {
                install_barrier.wait();
                install_gate.install(&install_control, &AtomicBool::new(false))
            });
            barrier.wait();
            gate.set_enabled(false);
            let installed = thread.join().unwrap();
            assert!(!installed || control.cancelled.load(Ordering::Acquire));
            assert!(!gate.enabled());
        }
    }
}
