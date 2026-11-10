//! Local learning history with scoped edits and bounded model output.
use super::{
    validation::sensitive, LearnedEntry, LearningConfig, MAX_RESPONSE_BYTES, MAX_TEXT_UNITS,
};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const MAX_HISTORY: usize = 1_000;
const MAX_BYTES: usize = 32 * 1024 * 1024;
static JOURNALS: OnceLock<Mutex<HashMap<PathBuf, Journal>>> = OnceLock::new();
static APP: OnceLock<AppHandle> = OnceLock::new();
static WRITER: OnceLock<Arc<(Mutex<PendingWrites>, Condvar)>> = OnceLock::new();

#[derive(Default)]
struct PendingWrites {
    journals: HashMap<PathBuf, Journal>,
    errors: HashMap<PathBuf, String>,
    unavailable: bool,
    writing: bool,
}

fn writer() -> &'static Arc<(Mutex<PendingWrites>, Condvar)> {
    WRITER.get_or_init(|| {
        let pending = Arc::new((Mutex::new(PendingWrites::default()), Condvar::new()));
        let worker_pending = pending.clone();
        let spawned = std::thread::Builder::new()
            .name("learning-activity-writer".into())
            .spawn(move || loop {
                let (path, journal) = {
                    let (lock, wake) = &*worker_pending;
                    let Ok(mut pending) = lock.lock() else {
                        return;
                    };
                    while pending.journals.is_empty() {
                        let Ok(next) = wake.wait(pending) else {
                            return;
                        };
                        pending = next;
                    }
                    let path = pending
                        .journals
                        .keys()
                        .next()
                        .expect("pending journal")
                        .clone();
                    let journal = pending.journals.remove(&path).expect("pending journal");
                    pending.writing = true;
                    (path, journal)
                };
                let result = save(&path, &journal);
                if let Ok(mut pending) = worker_pending.0.lock() {
                    pending.writing = false;
                    worker_pending.1.notify_all();
                    match result {
                        Ok(()) => {
                            pending.errors.remove(&path);
                        }
                        Err(_) => {
                            pending
                                .errors
                                .insert(path, "Cannot save learning activity history".into());
                        }
                    }
                }
                if let Some(app) = APP.get() {
                    let _ = app.emit("learning://updated", ());
                }
            });
        if spawned.is_err() {
            if let Ok(mut writes) = pending.0.lock() {
                writes.unavailable = true;
            }
        }
        pending
    })
}

// Called while the journal lock is held, preserving mutation order for each path.
fn schedule_save(path: &Path, journal: &Journal) -> Result<()> {
    let (lock, wake) = &**writer();
    let mut pending = lock
        .lock()
        .map_err(|_| anyhow::anyhow!("Learning activity writer is unavailable"))?;
    if pending.unavailable {
        bail!("Learning activity writer is unavailable");
    }
    pending.journals.insert(path.to_owned(), journal.clone());
    wake.notify_one();
    Ok(())
}

/// Drain scheduled metadata before normal application exit. Crashed runs recover as interrupted.
pub fn flush() -> Result<()> {
    let (lock, wake) = &**writer();
    let pending = lock
        .lock()
        .map_err(|_| anyhow::anyhow!("Learning activity writer is unavailable"))?;
    let (pending, timeout) = wake
        .wait_timeout_while(pending, std::time::Duration::from_secs(2), |pending| {
            !pending.journals.is_empty() || pending.writing
        })
        .map_err(|_| anyhow::anyhow!("Learning activity writer is unavailable"))?;
    if timeout.timed_out() || pending.unavailable || !pending.errors.is_empty() {
        bail!("Learning activity history could not be fully saved");
    }
    Ok(())
}

fn check_writer(path: &Path) -> Result<()> {
    let pending = writer()
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("Learning activity writer is unavailable"))?;
    if pending.unavailable {
        bail!("Learning activity writer is unavailable");
    }
    if let Some(error) = pending.errors.get(path) {
        bail!("{error}");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Observing,
    Queued,
    Analyzing,
    Learned,
    NoChange,
    NoTerms,
    Skipped,
    Failed,
    Interrupted,
}

impl ActivityState {
    fn active(&self) -> bool {
        matches!(self, Self::Observing | Self::Queued | Self::Analyzing)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningActivity {
    pub id: String,
    pub record_id: String,
    pub started_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub state: ActivityState,
    pub reason: Option<String>,
    pub model: Option<String>,
    pub service_id: Option<String>,
    pub entries: Vec<LearnedEntry>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_output: Option<String>,
}

impl LearningActivity {
    fn summary(&self) -> Self {
        Self {
            edited_text: None,
            model_output: None,
            ..self.clone()
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ActivityPage {
    pub active: Vec<LearningActivity>,
    pub history: Vec<LearningActivity>,
    pub has_more: bool,
}

#[derive(Debug, Clone)]
pub struct ActivityHandle {
    path: PathBuf,
    id: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Journal {
    entries: Vec<LearningActivity>,
}

pub fn initialize(app: AppHandle) {
    let _ = APP.set(app);
    if let Err(error) = list(1, 0, None) {
        eprintln!("[Learning] activity journal unavailable: {error}");
    }
}

fn path() -> Result<PathBuf> {
    Ok(crate::paths::app_dir()
        .map_err(anyhow::Error::msg)?
        .join("learning-activity.json"))
}

fn journals() -> &'static Mutex<HashMap<PathBuf, Journal>> {
    JOURNALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn loaded_journal<'a>(
    journals: &'a mut HashMap<PathBuf, Journal>,
    path: &Path,
) -> Result<&'a mut Journal> {
    if !journals.contains_key(path) {
        let mut journal = load(path)?;
        if journal.interrupt_unfinished() {
            schedule_save(path, &journal)?;
        }
        journals.insert(path.to_owned(), journal);
    }
    Ok(journals.get_mut(path).expect("journal loaded"))
}

fn with_journal<T>(path: &Path, operation: impl FnOnce(&mut Journal) -> Result<T>) -> Result<T> {
    let mut journals = journals()
        .lock()
        .map_err(|_| anyhow::anyhow!("Learning activity is unavailable"))?;
    operation(loaded_journal(&mut journals, path)?)
}

// Resolve the current directory while holding the same lock used by migration.
fn with_current_journal<T>(operation: impl FnOnce(&Path, &mut Journal) -> Result<T>) -> Result<T> {
    let mut journals = journals()
        .lock()
        .map_err(|_| anyhow::anyhow!("Learning activity is unavailable"))?;
    let current_path = path()?;
    let journal = loaded_journal(&mut journals, &current_path)?;
    operation(&current_path, journal)
}

/// Freeze metadata mutations and drain disk writes before data-directory copy/verification.
pub fn with_directory_change(
    operation: impl FnOnce() -> Result<crate::data_dir::DataDirInfo>,
) -> Result<crate::data_dir::DataDirInfo> {
    with_directory_change_impl(path, || {
        let info = operation()?;
        let target = PathBuf::from(&info.path).join("learning-activity.json");
        Ok((info, target))
    })
}

fn with_directory_change_impl<T>(
    source_path: impl FnOnce() -> Result<PathBuf>,
    operation: impl FnOnce() -> Result<(T, PathBuf)>,
) -> Result<T> {
    let mut journals = journals()
        .lock()
        .map_err(|_| anyhow::anyhow!("Learning activity is unavailable"))?;
    let source = source_path()?;
    let journal = loaded_journal(&mut journals, &source)?;
    if journal.finish_active(ActivityState::Skipped, "data_dir_changed") {
        schedule_save(&source, journal)?;
    }
    // The writer never acquires JOURNALS, so draining it under this lock is safe.
    flush()?;
    let (result, target) = operation()?;
    if target != source {
        // A target revisited later must reload the files just copied into it.
        journals.remove(&target);
    }
    // Keep the source's terminal records cached. A late observer/model handle
    // finds its completed attempt and cannot recreate the migrated source file.
    Ok(result)
}

impl Journal {
    fn interrupt_unfinished(&mut self) -> bool {
        self.finish_active(ActivityState::Interrupted, "app_restarted")
    }

    fn finish_active(&mut self, state: ActivityState, reason: &str) -> bool {
        let now = Utc::now().to_rfc3339();
        let mut changed = false;
        for entry in &mut self.entries {
            if entry.state.active() {
                entry.state = state.clone();
                entry.reason = Some(reason.into());
                entry.finished_at = Some(now.clone());
                entry.updated_at = now.clone();
                changed = true;
            }
        }
        changed
    }

    fn trim(&mut self) {
        let mut retained = 0;
        self.entries.reverse();
        self.entries.retain(|entry| {
            if entry.state.active() {
                return true;
            }
            retained += 1;
            retained <= MAX_HISTORY
        });
        self.entries.reverse();
    }

    fn page(&self, limit: usize, offset: usize, record_id: Option<&str>) -> ActivityPage {
        let matching =
            |entry: &&LearningActivity| record_id.map_or(true, |id| entry.record_id == id);
        let active = self
            .entries
            .iter()
            .rev()
            .filter(matching)
            .filter(|entry| entry.state.active())
            .map(LearningActivity::summary)
            .collect();
        let mut history: Vec<_> = self
            .entries
            .iter()
            .rev()
            .filter(matching)
            .filter(|entry| !entry.state.active())
            .skip(offset)
            .take(limit + 1)
            .map(LearningActivity::summary)
            .collect();
        let has_more = history.len() > limit;
        history.truncate(limit);
        ActivityPage {
            active,
            history,
            has_more,
        }
    }

    fn find_source(&self, record_id: &str, activity_id: Option<&str>) -> Option<&LearningActivity> {
        let mut matching = self
            .entries
            .iter()
            .rev()
            .filter(|entry| entry.record_id == record_id);
        if let Some(activity_id) = activity_id {
            return matching.find(|entry| entry.id == activity_id);
        }
        matching
            .clone()
            .find(|entry| entry.state == ActivityState::Learned)
            .or_else(|| matching.next())
    }

    fn clear_source_details(&mut self, record_id: &str) -> bool {
        let mut changed = false;
        let now = Utc::now().to_rfc3339();
        for entry in self
            .entries
            .iter_mut()
            .filter(|entry| entry.record_id == record_id)
        {
            changed |= entry.edited_text.take().is_some();
            changed |= entry.model_output.take().is_some();
            if entry.state.active() {
                entry.state = ActivityState::Skipped;
                entry.reason = Some("source_removed".into());
                entry.finished_at = Some(now.clone());
                entry.updated_at = now.clone();
                changed = true;
            }
        }
        changed
    }
}

pub fn list(limit: usize, offset: usize, record_id: Option<&str>) -> Result<ActivityPage> {
    with_current_journal(|path, journal| {
        check_writer(path)?;
        Ok(journal.page(limit.clamp(1, 100), offset, record_id))
    })
}

pub fn find_source(record_id: &str, activity_id: Option<&str>) -> Result<Option<LearningActivity>> {
    with_current_journal(|path, journal| {
        check_writer(path)?;
        Ok(journal.find_source(record_id, activity_id).cloned())
    })
}

pub fn clear_source_details(record_id: &str) -> Result<()> {
    let changed = with_current_journal(|path, journal| {
        let mut next = journal.clone();
        if !next.clear_source_details(record_id) {
            return Ok(false);
        }
        next.trim();
        schedule_save(path, &next)?;
        *journal = next;
        Ok(true)
    })?;
    if changed {
        notify();
    }
    Ok(())
}

pub fn begin(record_id: &str, config: Option<&LearningConfig>) -> Option<ActivityHandle> {
    let result = {
        let now = Utc::now().to_rfc3339();
        let id = Uuid::new_v4().to_string();
        let entry = LearningActivity {
            id: id.clone(),
            record_id: record_id.into(),
            started_at: now.clone(),
            updated_at: now,
            finished_at: None,
            state: ActivityState::Observing,
            reason: None,
            model: config.map(|config| config.llm.model.clone()),
            service_id: config.map(|config| config.llm.service_id.clone()),
            entries: Vec::new(),
            error: None,
            edited_text: None,
            model_output: None,
        };
        with_current_journal(|path, journal| {
            let mut next = journal.clone();
            next.entries.push(entry);
            next.trim();
            schedule_save(path, &next)?;
            *journal = next;
            Ok(ActivityHandle {
                path: path.to_owned(),
                id,
            })
        })
    };
    match result {
        Ok(handle) => {
            notify();
            Some(handle)
        }
        Err(error) => {
            eprintln!("[Learning] cannot record activity: {error}");
            None
        }
    }
}

impl ActivityHandle {
    pub fn is_current_active(&self) -> bool {
        // Worker cancellation takes the worker lock before migration freezes JOURNALS.
        // Enqueue calls this while holding that worker lock, keeping both checks atomic.
        with_current_journal(|current_path, journal| {
            Ok(self.path == current_path
                && journal
                    .entries
                    .iter()
                    .any(|entry| entry.id == self.id && entry.state.active()))
        })
        .unwrap_or(false)
    }

    pub fn update(
        &self,
        state: ActivityState,
        reason: Option<&str>,
        entries: Vec<LearnedEntry>,
        error: Option<String>,
    ) {
        self.mutate_active(|entry| {
            let now = Utc::now().to_rfc3339();
            entry.finished_at = (!state.active()).then(|| now.clone());
            entry.updated_at = now;
            entry.state = state;
            entry.reason = reason.map(str::to_owned);
            entry.entries = entries;
            entry.error = error;
        });
    }

    pub(super) fn set_details(&self, edited_text: Option<&str>, model_output: Option<&str>) {
        let edited_text = edited_text.filter(|text| {
            !text.trim().is_empty()
                && text.encode_utf16().count() <= MAX_TEXT_UNITS
                && !sensitive(text)
        });
        let model_output = model_output.filter(|text| {
            text.len() <= MAX_RESPONSE_BYTES
                && serde_json::from_str::<serde_json::Value>(text)
                    .is_ok_and(|value| !sensitive(&value.to_string()))
        });
        if edited_text.is_none() && model_output.is_none() {
            return;
        }
        self.mutate_active(|entry| {
            if let Some(text) = edited_text {
                entry.edited_text = Some(text.to_owned());
            }
            if let Some(text) = model_output {
                entry.model_output = Some(text.to_owned());
            }
        });
    }

    fn mutate_active(&self, mutate: impl FnOnce(&mut LearningActivity)) {
        let result = with_journal(&self.path, |journal| {
            let mut next = journal.clone();
            if let Some(entry) = next.entries.iter_mut().find(|entry| entry.id == self.id) {
                // A cancelled or completed attempt cannot be revived by a late model response.
                if !entry.state.active() {
                    return Ok(());
                }
                mutate(entry);
                next.trim();
                schedule_save(&self.path, &next)?;
                *journal = next;
            }
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("[Learning] cannot update activity: {error}");
        }
        notify();
    }
}

pub fn notify() {
    let Some(app) = APP.get() else {
        return;
    };
    let _ = app.emit("learning://updated", ());
    let update_app = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Ok(page) = list(1, 0, None) {
            let observing = page
                .active
                .iter()
                .filter(|entry| entry.state == ActivityState::Observing)
                .count();
            let queued = page
                .active
                .iter()
                .filter(|entry| entry.state == ActivityState::Queued)
                .count();
            let analyzing = page
                .active
                .iter()
                .filter(|entry| entry.state == ActivityState::Analyzing)
                .count();
            let _ = crate::tray::sync_learning_status(&update_app, observing, queued, analyzing);
        }
    });
}

fn load(path: &Path) -> Result<Journal> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Journal::default()),
        Err(_) => bail!("Cannot open learning activity history"),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("Cannot read learning activity history")?;
    if bytes.len() > MAX_BYTES {
        bail!("Learning activity history exceeded its size limit");
    }
    let journal: Journal =
        serde_json::from_slice(&bytes).context("Cannot parse learning activity history")?;
    if journal.entries.len() > MAX_HISTORY + 10 {
        bail!("Learning activity history has too many entries");
    }
    Ok(journal)
}

fn save(path: &Path, journal: &Journal) -> Result<()> {
    let bytes = journal_bytes(journal, MAX_BYTES)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pending = path.with_extension("json.pending");
    let mut file = File::create(&pending).context("Cannot save learning activity history")?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    super::store::replace(&pending, path)
}

fn journal_bytes(journal: &Journal, limit: usize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(journal)?;
    if bytes.len() <= limit {
        return Ok(bytes);
    }
    let mut size = bytes.len();
    drop(bytes);
    let mut count = journal.entries.len();
    let mut bounded = Journal::default();
    for entry in &journal.entries {
        if size > limit && !entry.state.active() {
            size -= serde_json::to_vec(entry)?.len() + usize::from(count > 1);
            count -= 1;
        } else {
            bounded.entries.push(entry.clone());
        }
    }
    if size > limit {
        bail!("Current learning activity exceeded its size limit");
    }
    Ok(serde_json::to_vec(&bounded)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The migration test deliberately pauses the global writer during setup.
    static WRITER_FIXTURES: Mutex<()> = Mutex::new(());

    fn entry(id: usize, state: ActivityState) -> LearningActivity {
        LearningActivity {
            id: id.to_string(),
            record_id: format!("record-{}", id % 2),
            started_at: String::new(),
            updated_at: String::new(),
            finished_at: None,
            state,
            reason: None,
            model: None,
            service_id: None,
            entries: vec![],
            error: None,
            edited_text: None,
            model_output: None,
        }
    }
    #[test]
    fn pagination_keeps_active_out_of_history_and_filters_by_source() {
        let journal = Journal {
            entries: vec![
                entry(0, ActivityState::Learned),
                entry(1, ActivityState::NoChange),
                entry(2, ActivityState::Queued),
                entry(3, ActivityState::Analyzing),
                entry(4, ActivityState::Failed),
            ],
        };
        let page = journal.page(1, 0, None);
        assert_eq!(page.active.len(), 2);
        assert_eq!(page.history[0].id, "4");
        assert!(page.has_more);
        assert_eq!(journal.page(1, 1, None).history[0].id, "1");
        let filtered = journal.page(10, 0, Some("record-0"));
        assert_eq!(filtered.active.len(), 1);
        assert_eq!(filtered.history.len(), 2);
    }

    #[test]
    fn source_lookup_keeps_details_for_the_selected_attempt_off_summary_pages() {
        let mut learned = entry(0, ActivityState::Learned);
        learned.edited_text = Some("修正后的语音".into());
        learned.model_output = Some(r#"{"terms":[]}"#.into());
        let journal = Journal {
            entries: vec![learned, entry(2, ActivityState::Skipped)],
        };
        let default = journal.find_source("record-0", None).unwrap();
        assert_eq!(default.id, "0");
        assert_eq!(default.edited_text.as_deref(), Some("修正后的语音"));
        assert_eq!(default.model_output.as_deref(), Some(r#"{"terms":[]}"#));
        assert_eq!(journal.find_source("record-0", Some("2")).unwrap().id, "2");
        assert!(journal.find_source("record-1", Some("0")).is_none());

        let page = journal.page(10, 0, None);
        let summary = serde_json::to_value(&page.history[1]).unwrap();
        assert!(summary.get("edited_text").is_none());
        assert!(summary.get("model_output").is_none());
        let old: LearningActivity = serde_json::from_value(summary).unwrap();
        assert!(old.edited_text.is_none() && old.model_output.is_none());
    }

    #[test]
    fn detail_writes_keep_the_original_format_and_reject_unsafe_or_late_updates() {
        let _fixture = WRITER_FIXTURES.lock().unwrap();
        let base =
            std::env::temp_dir().join(format!("boltscribe-activity-details-{}", Uuid::new_v4()));
        let path = base.join("learning-activity.json");
        let active = entry(0, ActivityState::Analyzing);
        let handle = ActivityHandle {
            path: path.clone(),
            id: active.id.clone(),
        };
        with_journal(&path, |journal| {
            journal.entries.push(active);
            Ok(())
        })
        .unwrap();

        let edited = "改为 BoltScribe。\n第二行照原样保留。";
        let output = " {\n  \"terms\": []\n}\n";
        handle.set_details(Some(edited), Some(output));
        handle.set_details(
            Some("包含 api_key 的修改"),
            Some(r#"{"terms":[{"term":"\u0070assword"}]}"#),
        );
        handle.set_details(
            Some(&"字".repeat(MAX_TEXT_UNITS + 1)),
            Some(&format!(
                "{}{{\"terms\":[]}}",
                " ".repeat(MAX_RESPONSE_BYTES)
            )),
        );
        handle.set_details(None, Some("不是 JSON 的模型响应"));
        handle.update(ActivityState::NoTerms, Some("no_terms"), vec![], None);
        handle.set_details(Some("过时的修改"), Some(r#"{"terms":[{}]}"#));
        flush().unwrap();

        let stored = load(&path).unwrap();
        assert_eq!(stored.entries[0].edited_text.as_deref(), Some(edited));
        assert_eq!(stored.entries[0].model_output.as_deref(), Some(output));
        assert_eq!(stored.entries[0].state, ActivityState::NoTerms);
        cleanup_fixture(&base);
    }

    #[test]
    fn deleting_a_source_keeps_learned_entries_and_ends_its_active_attempts() {
        let mut learned = entry(0, ActivityState::Learned);
        learned.edited_text = Some("修正后的语音".into());
        learned.model_output = Some(r#"{"terms":[]}"#.into());
        learned.entries.push(LearnedEntry {
            id: "term-1".into(),
            term: "BoltScribe".into(),
            from: Some("波特思考".into()),
            created_at: String::new(),
            record_id: "record-0".into(),
            enabled: true,
        });
        let mut journal = Journal {
            entries: vec![
                learned,
                entry(1, ActivityState::Analyzing),
                entry(2, ActivityState::Analyzing),
            ],
        };
        assert!(journal.clear_source_details("record-0"));
        assert!(!journal.clear_source_details("record-0"));
        let saved = &journal.entries[0];
        assert_eq!(saved.state, ActivityState::Learned);
        assert_eq!(saved.entries.len(), 1);
        assert!(saved.edited_text.is_none() && saved.model_output.is_none());
        assert_eq!(journal.entries[1].state, ActivityState::Analyzing);
        let cleared = &journal.entries[2];
        assert_eq!(cleared.state, ActivityState::Skipped);
        assert_eq!(cleared.reason.as_deref(), Some("source_removed"));
        assert!(cleared.finished_at.is_some());
    }

    #[test]
    fn size_retention_keeps_active_tasks_and_full_newest_details() {
        let oldest = entry(0, ActivityState::NoTerms);
        let active = entry(1, ActivityState::Analyzing);
        let mut newest = entry(2, ActivityState::NoTerms);
        newest.edited_text = Some("带换行和引号的详情\n\"样本\"".into());
        let expected = Journal {
            entries: vec![active.clone(), newest.clone()],
        };
        let expected_bytes = serde_json::to_vec(&expected).unwrap();
        let oversized = Journal {
            entries: vec![oldest, active, newest],
        };
        assert_eq!(
            journal_bytes(&oversized, expected_bytes.len()).unwrap(),
            expected_bytes
        );
        assert!(journal_bytes(&oversized, 1).is_err());
    }

    #[test]
    fn restart_recovery_only_changes_unfinished_attempts() {
        let mut journal = Journal {
            entries: vec![
                entry(0, ActivityState::Learned),
                entry(1, ActivityState::Observing),
                entry(2, ActivityState::Queued),
            ],
        };
        assert!(journal.interrupt_unfinished());
        assert_eq!(journal.entries[0].state, ActivityState::Learned);
        assert!(journal.entries[1..]
            .iter()
            .all(|entry| entry.state == ActivityState::Interrupted
                && entry.reason.as_deref() == Some("app_restarted")
                && entry.finished_at.is_some()));
        assert!(!journal.interrupt_unfinished());
    }
    #[test]
    fn migration_drains_pending_writes_and_late_handles_cannot_recreate_source() {
        let _fixture = WRITER_FIXTURES.lock().unwrap();
        let base =
            std::env::temp_dir().join(format!("boltscribe-activity-migration-{}", Uuid::new_v4()));
        let source = base.join("source/learning-activity.json");
        let target = base.join("target/learning-activity.json");
        let mut active = entry(123, ActivityState::Analyzing);
        active.model = Some("current-model".into());
        with_journal(&source, |journal| {
            journal.entries.push(active.clone());
            Ok(())
        })
        .unwrap();
        // Simulate an already visited destination with stale cached metadata.
        with_journal(&target, |journal| {
            journal.entries.push(entry(999, ActivityState::NoChange));
            Ok(())
        })
        .unwrap();
        let handle = ActivityHandle {
            path: source.clone(),
            id: active.id.clone(),
        };
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (writer_lock, _) = &**writer();
        let mut pending = writer_lock.lock().unwrap();
        pending.journals.insert(
            source.clone(),
            Journal {
                entries: vec![active],
            },
        );
        let migrate_source = source.clone();
        let migrate_target = target.clone();
        let migration = std::thread::spawn(move || {
            with_directory_change_impl(
                || {
                    // The journal mutex is now held and the earlier disk write is still pending.
                    ready_tx.send(()).unwrap();
                    Ok(migrate_source.clone())
                },
                || {
                    let stored = load(&migrate_source)?;
                    assert_eq!(stored.entries[0].state, ActivityState::Skipped);
                    assert_eq!(
                        stored.entries[0].reason.as_deref(),
                        Some("data_dir_changed")
                    );
                    fs::create_dir_all(migrate_target.parent().unwrap())?;
                    fs::rename(&migrate_source, &migrate_target)?;
                    Ok(((), migrate_target))
                },
            )
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        drop(pending);
        migration.join().unwrap().unwrap();
        handle.update(ActivityState::Learned, None, vec![], None);
        flush().unwrap();
        assert!(!source.exists());
        with_journal(&target, |journal| {
            assert_eq!(journal.entries.len(), 1);
            assert_eq!(journal.entries[0].id, "123");
            assert_eq!(journal.entries[0].state, ActivityState::Skipped);
            Ok(())
        })
        .unwrap();
        cleanup_fixture(&base);
    }

    fn cleanup_fixture(base: &Path) {
        // Fixture output remains recoverable in the macOS Trash.
        #[cfg(target_os = "macos")]
        {
            let trash = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".Trash");
            fs::rename(base, trash.join(base.file_name().unwrap())).unwrap();
        }
        #[cfg(not(target_os = "macos"))]
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn retention_keeps_current_tasks_and_newest_history() {
        let mut journal = Journal {
            entries: (0..MAX_HISTORY + 20)
                .map(|id| entry(id, ActivityState::NoChange))
                .collect(),
        };
        journal
            .entries
            .insert(0, entry(MAX_HISTORY + 99, ActivityState::Analyzing));
        journal.trim();
        assert_eq!(journal.entries.len(), MAX_HISTORY + 1);
        assert_eq!(journal.entries[0].state, ActivityState::Analyzing);
        assert_eq!(journal.entries[1].id, "20");
    }
}
