use crate::asr::LiveAsrDiagnostics;
use crate::config::RetentionConfig;
use crate::corrector::LlmCallLog;
use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

const HISTORY_READ_CHUNK_SIZE: u64 = 16 * 1024;
const STATS_SCHEMA_VERSION: u8 = 1;
const MAX_RECORDING_CLEANUP_DAYS: u32 = 36_500;
const MAX_RECORDING_CLEANUP_WEEKS: u32 = 5_200;
const MAX_RECORDING_CLEANUP_MONTHS: u32 = 1_200;
const ORPHAN_RECORDING_GRACE_HOURS: i64 = 24;
static HISTORY_MUTATION_LOCK: Mutex<()> = Mutex::new(());

struct HistoryPage {
    records: Vec<HistoryRecord>,
    visible_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub audio_path: Option<PathBuf>,
    pub asr_provider: String,
    pub asr_task_id: Option<String>,
    pub audio_started_at: DateTime<Utc>,
    pub audio_finished_at: DateTime<Utc>,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
    pub audio_sample_count: usize,
    pub raw_text: String,
    pub corrected_text: String,
    pub pasted_text: String,
    pub correction_enabled: bool,
    pub correction_error: Option<String>,
    #[serde(default)]
    pub correction_logs: Vec<LlmCallLog>,
    pub injection_error: Option<String>,
    pub workflow_error: Option<String>,
    pub asr_duration_ms: Option<u64>,
    pub service_audio_duration_ms: Option<u64>,
    #[serde(default)]
    pub live_asr_diagnostics: Option<LiveAsrDiagnostics>,
    pub total_duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeleteHistoryResult {
    pub deleted_records: usize,
    pub deleted_audio_files: usize,
    pub freed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecordingCleanupResult {
    pub deleted_files: usize,
    pub cleared_history_records: usize,
    pub freed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecordingCleanupPreview {
    pub recording_files: usize,
    pub recording_bytes: u64,
    pub eligible_files: usize,
    pub eligible_bytes: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordingCleanupUnit {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct InputStats {
    pub total_character_count: u64,
    pub total_audio_duration_ms: u64,
    pub average_chars_per_minute: f64,
    pub daily: Vec<DailyInputStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DailyInputStats {
    pub date: String,
    pub record_count: u64,
    pub character_count: u64,
    pub audio_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct InputStatsEvent {
    schema_version: u8,
    record_id: String,
    date: String,
    character_count: u64,
    audio_duration_ms: u64,
}

pub struct HistoryStore;

impl HistoryStore {
    pub fn append(record: &HistoryRecord, retention: &RetentionConfig) -> Result<()> {
        let _guard = history_mutation_guard()?;
        let path = paths::history_path()?;
        let stats_path = paths::input_stats_path()?;
        append_history_record(&path, record)?;

        if let Err(err) = append_stats_for_record(&stats_path, record) {
            eprintln!(
                "Warning: history record {} was saved, but input statistics could not be updated: {err:#}",
                record.id
            );
        }

        match (history_read_paths(), history_recordings_dirs()) {
            (Ok(history_paths), Ok(recordings_dirs)) => {
                if let Err(err) = prune_paths(&history_paths, retention, &recordings_dirs) {
                    eprintln!(
                        "Warning: history record {} was saved, but retention cleanup failed: {err:#}",
                        record.id
                    );
                }
            }
            (Err(err), _) | (_, Err(err)) => {
                eprintln!(
                    "Warning: history record {} was saved, but retention paths could not be resolved: {err:#}",
                    record.id
                );
            }
        }
        Ok(())
    }

    pub fn load(limit: usize, offset: usize) -> Result<Vec<HistoryRecord>> {
        load_from_paths(&history_read_paths()?, limit, offset)
    }

    pub fn load_retryable(record_id: &str) -> Result<HistoryRecord> {
        let _guard = history_mutation_guard()?;
        load_retryable_from_paths(
            &history_read_paths()?,
            &history_recordings_dirs()?,
            record_id,
        )
    }

    pub fn replace(record: &HistoryRecord) -> Result<()> {
        let _guard = history_mutation_guard()?;
        replace_in_paths(&history_read_paths()?, &paths::input_stats_path()?, record)
    }

    pub fn stats() -> Result<InputStats> {
        let _guard = history_mutation_guard()?;
        let stats_path = paths::input_stats_path()?;
        let history_paths = history_read_paths()?;
        backfill_stats_from_history(&stats_path, &history_paths)?;
        load_stats_from_sources(&stats_path, &history_paths)
    }

    pub fn prune(retention: &RetentionConfig) -> Result<()> {
        let _guard = history_mutation_guard()?;
        prune_paths(
            &history_read_paths()?,
            retention,
            &history_recordings_dirs()?,
        )
    }

    pub fn delete(record_id: &str) -> Result<DeleteHistoryResult> {
        let _guard = history_mutation_guard()?;
        let history_paths = history_read_paths()?;
        backfill_stats_from_history(&paths::input_stats_path()?, &history_paths)?;
        delete_from_paths(&history_paths, &history_recordings_dirs()?, record_id)
    }

    pub fn cleanup_recordings_older_than(
        amount: u32,
        unit: RecordingCleanupUnit,
    ) -> Result<RecordingCleanupResult> {
        let cutoff = recording_cleanup_cutoff(Utc::now(), amount, unit)?;
        let _guard = history_mutation_guard()?;
        cleanup_recordings_before(&history_read_paths()?, &history_recordings_dirs()?, cutoff)
    }

    pub fn preview_recording_cleanup(
        amount: u32,
        unit: RecordingCleanupUnit,
    ) -> Result<RecordingCleanupPreview> {
        let cutoff = recording_cleanup_cutoff(Utc::now(), amount, unit)?;
        let _guard = history_mutation_guard()?;
        preview_recording_cleanup_before(
            &history_read_paths()?,
            &history_recordings_dirs()?,
            cutoff,
        )
    }
}

fn recording_cleanup_cutoff(
    now: DateTime<Utc>,
    amount: u32,
    unit: RecordingCleanupUnit,
) -> Result<DateTime<Utc>> {
    if amount == 0 {
        bail!("Recording cleanup age must be greater than zero");
    }

    match unit {
        RecordingCleanupUnit::Day => {
            if amount > MAX_RECORDING_CLEANUP_DAYS {
                bail!("Recording cleanup days cannot exceed {MAX_RECORDING_CLEANUP_DAYS}");
            }
            Ok(now - ChronoDuration::days(i64::from(amount)))
        }
        RecordingCleanupUnit::Week => {
            if amount > MAX_RECORDING_CLEANUP_WEEKS {
                bail!("Recording cleanup weeks cannot exceed {MAX_RECORDING_CLEANUP_WEEKS}");
            }
            Ok(now - ChronoDuration::weeks(i64::from(amount)))
        }
        RecordingCleanupUnit::Month => {
            if amount > MAX_RECORDING_CLEANUP_MONTHS {
                bail!("Recording cleanup months cannot exceed {MAX_RECORDING_CLEANUP_MONTHS}");
            }
            Ok(now - ChronoDuration::days(i64::from(amount) * 30))
        }
    }
}

fn history_mutation_guard() -> Result<MutexGuard<'static, ()>> {
    HISTORY_MUTATION_LOCK
        .lock()
        .map_err(|_| anyhow!("History storage lock is poisoned"))
}

#[cfg(test)]
fn append_to_paths(
    record: &HistoryRecord,
    retention: &RetentionConfig,
    history_path: &Path,
    stats_path: &Path,
    recordings_dir: &Path,
) -> Result<()> {
    append_history_record(history_path, record)?;
    if let Err(err) = append_stats_for_record(stats_path, record) {
        eprintln!(
            "Warning: history record {} was saved, but input statistics could not be updated: {err:#}",
            record.id
        );
    }
    prune_paths(
        &[history_path.to_path_buf()],
        retention,
        &[recordings_dir.to_path_buf()],
    )
}

fn append_history_record(path: &Path, record: &HistoryRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(record)?)
        .with_context(|| format!("Failed to append {}", path.display()))?;
    Ok(())
}

fn load_retryable_from_paths(
    history_paths: &[PathBuf],
    recordings_dirs: &[PathBuf],
    record_id: &str,
) -> Result<HistoryRecord> {
    let record_id = record_id.trim();
    if record_id.is_empty() {
        bail!("History record id cannot be empty");
    }

    for history_path in history_paths {
        if !history_path.exists() {
            continue;
        }
        let file = fs::File::open(history_path)
            .with_context(|| format!("Failed to open {}", history_path.display()))?;
        let lines = BufReader::new(file)
            .lines()
            .collect::<std::io::Result<Vec<_>>>()?;
        for line in lines.iter().rev() {
            let Ok(mut record) = serde_json::from_str::<HistoryRecord>(line) else {
                continue;
            };
            if record.id != record_id {
                continue;
            }
            if !record
                .workflow_error
                .as_deref()
                .is_some_and(|error| !error.trim().is_empty())
            {
                bail!("History record {record_id} is not a failed workflow");
            }
            if !record.raw_text.trim().is_empty()
                || !record.corrected_text.trim().is_empty()
                || !record.pasted_text.trim().is_empty()
            {
                bail!("History record {record_id} already contains transcription text");
            }
            let audio_path = record.audio_path.as_deref().ok_or_else(|| {
                anyhow!("The recording for history record {record_id} is unavailable")
            })?;
            record.audio_path = Some(normalized_retry_recording_path(
                audio_path,
                recordings_dirs,
            )?);
            return Ok(record);
        }
    }

    bail!("History record {record_id} was not found")
}

fn normalized_retry_recording_path(path: &Path, recordings_dirs: &[PathBuf]) -> Result<PathBuf> {
    if path.extension().and_then(|value| value.to_str()) != Some("wav") {
        bail!("Retry recording must be a .wav file");
    }
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("Retry recording path is outside the recordings directory");
    }

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Retry recording {} is unavailable", path.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) {
        bail!("Retry recording cannot be a symbolic link");
    }
    if !metadata.file_type().is_file() {
        bail!("Retry recording must be a regular file");
    }

    let normalized_path = path
        .canonicalize()
        .with_context(|| format!("Failed to resolve retry recording {}", path.display()))?;
    let normalized_parent = normalized_path
        .parent()
        .ok_or_else(|| anyhow!("Retry recording path has no parent directory"))?;
    let allowed = recordings_dirs.iter().any(|recordings_dir| {
        normalized_recordings_directory(recordings_dir).is_ok_and(|directory| {
            directory
                .as_deref()
                .is_some_and(|directory| directory == normalized_parent)
        })
    });
    if !allowed {
        bail!("Retry recording path is outside the recordings directory");
    }

    Ok(normalized_path)
}

fn replace_in_paths(
    history_paths: &[PathBuf],
    stats_path: &Path,
    record: &HistoryRecord,
) -> Result<()> {
    if record.id.trim().is_empty() {
        bail!("History record id cannot be empty");
    }
    let record_id = record.id.as_str();

    let replacement = serde_json::to_value(record)?;
    let mut history_files = load_mutable_history_files(history_paths)?;
    let mut replaced_records = 0usize;
    for history_file in &mut history_files {
        for line in &mut history_file.lines {
            let matches = line
                .value
                .as_ref()
                .and_then(|value| value.get("id"))
                .and_then(serde_json::Value::as_str)
                == Some(record_id);
            if !matches {
                continue;
            }
            let Some(value) = line.value.as_mut() else {
                continue;
            };
            merge_json_fields(value, &replacement)?;
            line.record = Some(record.clone());
            line.changed = true;
            history_file.changed = true;
            replaced_records += 1;
        }
    }

    if replaced_records == 0 {
        bail!("History record {record_id} was not found");
    }

    let mut changed_files = Vec::new();
    if let Some(stats_file) = prepare_stats_file_for_record(stats_path, record)? {
        changed_files.push(stats_file);
    }
    changed_files.extend(
        history_files
            .into_iter()
            .filter(|history_file| history_file.changed),
    );
    write_mutable_history_files_atomically(&changed_files)
}

fn merge_json_fields(
    target: &mut serde_json::Value,
    replacement: &serde_json::Value,
) -> Result<()> {
    let target = target
        .as_object_mut()
        .ok_or_else(|| anyhow!("Stored JSON value is not an object"))?;
    let replacement = replacement
        .as_object()
        .ok_or_else(|| anyhow!("Replacement JSON value is not an object"))?;
    for (key, value) in replacement {
        target.insert(key.clone(), value.clone());
    }
    Ok(())
}

#[derive(Clone)]
struct HistoryRecordLocation {
    file_index: usize,
    line_index: usize,
    created_at: DateTime<Utc>,
    audio_path: Option<PathBuf>,
}

fn prune_paths(
    history_paths: &[PathBuf],
    retention: &RetentionConfig,
    recordings_dirs: &[PathBuf],
) -> Result<()> {
    prune_paths_at(history_paths, retention, recordings_dirs, Utc::now())
}

fn prune_paths_at(
    history_paths: &[PathBuf],
    retention: &RetentionConfig,
    recordings_dirs: &[PathBuf],
    now: DateTime<Utc>,
) -> Result<()> {
    let mut history_files = load_mutable_history_files(history_paths)?;
    let mut records = Vec::new();
    for (file_index, history_file) in history_files.iter().enumerate() {
        for (line_index, line) in history_file.lines.iter().enumerate() {
            let Some(record) = line.record.as_ref() else {
                continue;
            };
            let audio_path = line
                .value
                .as_ref()
                .and_then(audio_path_from_value)
                .map(|path| normalized_safe_recording_path(&path, recordings_dirs))
                .transpose()?
                .flatten();
            records.push(HistoryRecordLocation {
                file_index,
                line_index,
                created_at: record.created_at,
                audio_path,
            });
        }
    }
    records.sort_by_key(|record| record.created_at);

    let count_removals = records.len().saturating_sub(retention.max_history_records);
    for location in records.iter().take(count_removals) {
        mark_history_line_removed(&mut history_files, location);
    }

    let mut retained_references = retained_audio_reference_counts(&history_files, recordings_dirs)?;
    let removed_audio_paths = removed_audio_paths(&history_files, recordings_dirs)?;
    let inventory = recording_file_inventory(recordings_dirs)?;
    let managed_files = inventory
        .into_iter()
        .filter(|file| file.app_managed_name)
        .map(|file| (file.path.clone(), file))
        .collect::<HashMap<_, _>>();
    let mut delete_paths = HashSet::new();

    for audio_path in removed_audio_paths {
        if !retained_references.contains_key(&audio_path) && managed_files.contains_key(&audio_path)
        {
            delete_paths.insert(audio_path);
        }
    }

    let has_malformed_history = history_files.iter().any(|history_file| {
        history_file
            .lines
            .iter()
            .any(|line| !line.raw.trim().is_empty() && line.value.is_none())
    });
    if !has_malformed_history {
        let orphan_cutoff = now - ChronoDuration::hours(ORPHAN_RECORDING_GRACE_HOURS);
        for file in managed_files.values() {
            if !retained_references.contains_key(&file.path)
                && file
                    .modified_at
                    .is_some_and(|modified| modified < orphan_cutoff)
            {
                delete_paths.insert(file.path.clone());
            }
        }
    }

    let mut projected_storage_bytes = managed_files
        .values()
        .filter(|file| retained_references.contains_key(&file.path))
        .fold(0u64, |total, file| total.saturating_add(file.bytes));

    let mut next_record = count_removals;
    while projected_storage_bytes > retention.max_storage_bytes && next_record < records.len() {
        let location = &records[next_record];
        next_record += 1;
        if history_files[location.file_index].lines[location.line_index].removed {
            continue;
        }

        mark_history_line_removed(&mut history_files, location);
        let Some(audio_path) = location.audio_path.as_ref() else {
            continue;
        };
        let remove_audio = match retained_references.get_mut(audio_path) {
            Some(reference_count) => {
                *reference_count = reference_count.saturating_sub(1);
                *reference_count == 0
            }
            None => false,
        };
        if !remove_audio {
            continue;
        }
        retained_references.remove(audio_path);
        if let Some(file) = managed_files.get(audio_path) {
            projected_storage_bytes = projected_storage_bytes.saturating_sub(file.bytes);
            delete_paths.insert(audio_path.clone());
        }
    }

    write_mutable_history_files_atomically(&history_files)?;
    delete_recording_paths(&delete_paths, recordings_dirs)?;
    Ok(())
}

fn mark_history_line_removed(
    history_files: &mut [MutableHistoryFile],
    location: &HistoryRecordLocation,
) {
    let history_file = &mut history_files[location.file_index];
    history_file.lines[location.line_index].removed = true;
    history_file.changed = true;
}

fn retained_audio_reference_counts(
    history_files: &[MutableHistoryFile],
    recordings_dirs: &[PathBuf],
) -> Result<HashMap<PathBuf, usize>> {
    let mut references = HashMap::new();
    for line in history_files
        .iter()
        .flat_map(|history_file| history_file.lines.iter())
        .filter(|line| !line.removed)
    {
        let Some(raw_audio_path) = line.value.as_ref().and_then(audio_path_from_value) else {
            continue;
        };
        let Some(audio_path) = normalized_safe_recording_path(&raw_audio_path, recordings_dirs)?
        else {
            continue;
        };
        *references.entry(audio_path).or_insert(0) += 1;
    }
    Ok(references)
}

fn removed_audio_paths(
    history_files: &[MutableHistoryFile],
    recordings_dirs: &[PathBuf],
) -> Result<HashSet<PathBuf>> {
    let mut paths = HashSet::new();
    for line in history_files
        .iter()
        .flat_map(|history_file| history_file.lines.iter())
        .filter(|line| line.removed)
    {
        let Some(raw_audio_path) = line.value.as_ref().and_then(audio_path_from_value) else {
            continue;
        };
        if let Some(audio_path) = normalized_safe_recording_path(&raw_audio_path, recordings_dirs)?
        {
            paths.insert(audio_path);
        }
    }
    Ok(paths)
}

struct MutableHistoryLine {
    raw: String,
    value: Option<serde_json::Value>,
    record: Option<HistoryRecord>,
    removed: bool,
    changed: bool,
}

struct MutableHistoryFile {
    path: PathBuf,
    lines: Vec<MutableHistoryLine>,
    changed: bool,
}

fn load_mutable_history_files(paths: &[PathBuf]) -> Result<Vec<MutableHistoryFile>> {
    paths
        .iter()
        .filter(|path| path.exists())
        .map(|path| {
            let file = fs::File::open(path)
                .with_context(|| format!("Failed to open {}", path.display()))?;
            let mut lines = Vec::new();
            for raw in BufReader::new(file).lines() {
                let raw = raw?;
                let value = serde_json::from_str::<serde_json::Value>(&raw).ok();
                let record = value
                    .as_ref()
                    .and_then(|value| serde_json::from_value(value.clone()).ok());
                lines.push(MutableHistoryLine {
                    raw,
                    value,
                    record,
                    removed: false,
                    changed: false,
                });
            }
            Ok(MutableHistoryFile {
                path: path.clone(),
                lines,
                changed: false,
            })
        })
        .collect()
}

fn audio_path_from_value(value: &serde_json::Value) -> Option<PathBuf> {
    value
        .get("audio_path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn delete_from_paths(
    history_paths: &[PathBuf],
    recordings_dirs: &[PathBuf],
    record_id: &str,
) -> Result<DeleteHistoryResult> {
    let record_id = record_id.trim();
    if record_id.is_empty() {
        bail!("History record id cannot be empty");
    }

    let mut history_files = load_mutable_history_files(history_paths)?;
    let mut removed_audio_paths = HashSet::new();
    let mut deleted_records = 0usize;

    for history_file in &mut history_files {
        for line in &mut history_file.lines {
            let matches = line
                .value
                .as_ref()
                .and_then(|value| value.get("id"))
                .and_then(serde_json::Value::as_str)
                == Some(record_id);
            if !matches {
                continue;
            }
            if let Some(audio_path) = line.value.as_ref().and_then(audio_path_from_value) {
                removed_audio_paths.insert(audio_path);
            }
            line.removed = true;
            history_file.changed = true;
            deleted_records += 1;
        }
    }

    if deleted_records == 0 {
        return Ok(DeleteHistoryResult {
            deleted_records: 0,
            deleted_audio_files: 0,
            freed_bytes: 0,
        });
    }

    let mut remaining_audio_paths = HashSet::new();
    for audio_path in history_files
        .iter()
        .flat_map(|history_file| history_file.lines.iter())
        .filter(|line| !line.removed)
        .filter_map(|line| line.value.as_ref().and_then(audio_path_from_value))
    {
        if let Some(audio_path) = normalized_safe_recording_path(&audio_path, recordings_dirs)? {
            remaining_audio_paths.insert(audio_path);
        }
    }

    write_mutable_history_files_atomically(&history_files)?;

    let mut deletable_audio_paths = HashSet::new();
    for audio_path in removed_audio_paths {
        let Some(audio_path) = normalized_safe_recording_path(&audio_path, recordings_dirs)? else {
            continue;
        };
        if !remaining_audio_paths.contains(&audio_path) {
            deletable_audio_paths.insert(audio_path);
        }
    }
    let (deleted_audio_files, freed_bytes) =
        delete_recording_paths(&deletable_audio_paths, recordings_dirs)?;

    Ok(DeleteHistoryResult {
        deleted_records,
        deleted_audio_files,
        freed_bytes,
    })
}

fn cleanup_recordings_before(
    history_paths: &[PathBuf],
    recordings_dirs: &[PathBuf],
    cutoff: DateTime<Utc>,
) -> Result<RecordingCleanupResult> {
    let mut history_files = load_mutable_history_files(history_paths)?;
    let candidate_audio_paths =
        cleanup_candidate_audio_paths(&history_files, recordings_dirs, cutoff)?;

    let mut cleared_history_records = 0usize;
    for history_file in &mut history_files {
        for line in &mut history_file.lines {
            let Some(record) = line.record.as_ref() else {
                continue;
            };
            if record.audio_finished_at >= cutoff {
                continue;
            }
            let Some(raw_audio_path) = line.value.as_ref().and_then(audio_path_from_value) else {
                continue;
            };
            let Some(audio_path) =
                normalized_safe_recording_path(&raw_audio_path, recordings_dirs)?
            else {
                continue;
            };
            if !candidate_audio_paths.contains(&audio_path) {
                continue;
            }
            if let Some(value) = line.value.as_mut() {
                value["audio_path"] = serde_json::Value::Null;
            }
            if let Some(record) = line.record.as_mut() {
                record.audio_path = None;
            }
            line.changed = true;
            history_file.changed = true;
            cleared_history_records += 1;
        }
    }
    write_mutable_history_files_atomically(&history_files)?;
    let (deleted_files, freed_bytes) =
        delete_recording_paths(&candidate_audio_paths, recordings_dirs)?;

    Ok(RecordingCleanupResult {
        deleted_files,
        cleared_history_records,
        freed_bytes,
    })
}

fn cleanup_candidate_audio_paths(
    history_files: &[MutableHistoryFile],
    recordings_dirs: &[PathBuf],
    cutoff: DateTime<Utc>,
) -> Result<HashSet<PathBuf>> {
    let mut protected_audio_paths = HashSet::new();
    let mut candidate_audio_paths = HashSet::new();
    let mut referenced_audio_paths = HashSet::new();
    for line in history_files
        .iter()
        .flat_map(|history_file| history_file.lines.iter())
    {
        let Some(raw_audio_path) = line.value.as_ref().and_then(audio_path_from_value) else {
            continue;
        };
        let Some(audio_path) = normalized_safe_recording_path(&raw_audio_path, recordings_dirs)?
        else {
            continue;
        };
        referenced_audio_paths.insert(audio_path.clone());
        match &line.record {
            Some(record) if record.audio_finished_at < cutoff => {
                candidate_audio_paths.insert(audio_path);
            }
            _ => {
                protected_audio_paths.insert(audio_path);
            }
        }
    }
    candidate_audio_paths.retain(|path| !protected_audio_paths.contains(path));

    let has_malformed_history = history_files.iter().any(|history_file| {
        history_file
            .lines
            .iter()
            .any(|line| !line.raw.trim().is_empty() && line.value.is_none())
    });
    if !has_malformed_history {
        for file in recording_file_inventory(recordings_dirs)? {
            if file.app_managed_name
                && !referenced_audio_paths.contains(&file.path)
                && file.modified_at.is_some_and(|modified| modified < cutoff)
            {
                candidate_audio_paths.insert(file.path);
            }
        }
    }
    Ok(candidate_audio_paths)
}

fn preview_recording_cleanup_before(
    history_paths: &[PathBuf],
    recordings_dirs: &[PathBuf],
    cutoff: DateTime<Utc>,
) -> Result<RecordingCleanupPreview> {
    let history_files = load_mutable_history_files(history_paths)?;
    let candidate_audio_paths =
        cleanup_candidate_audio_paths(&history_files, recordings_dirs, cutoff)?;
    let (recording_files, recording_bytes) = recording_storage_usage(recordings_dirs)?;
    let mut eligible_files = 0usize;
    let mut eligible_bytes = 0u64;
    for audio_path in candidate_audio_paths {
        if let Some(metadata) = regular_recording_metadata(&audio_path, recordings_dirs)? {
            eligible_bytes = eligible_bytes.saturating_add(metadata.len());
            eligible_files += 1;
        }
    }
    Ok(RecordingCleanupPreview {
        recording_files,
        recording_bytes,
        eligible_files,
        eligible_bytes,
    })
}

fn recording_storage_usage(recordings_dirs: &[PathBuf]) -> Result<(usize, u64)> {
    let inventory = recording_file_inventory(recordings_dirs)?;
    Ok((
        inventory.len(),
        inventory
            .iter()
            .fold(0u64, |total, file| total.saturating_add(file.bytes)),
    ))
}

fn write_mutable_history_files_atomically(history_files: &[MutableHistoryFile]) -> Result<()> {
    let replacements = history_files
        .iter()
        .filter(|history_file| history_file.changed)
        .map(|history_file| {
            Ok((
                history_file.path.clone(),
                mutable_history_file_contents(history_file)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    write_file_replacements_with_rollback(&replacements, write_file_bytes_atomically)
}

fn mutable_history_file_contents(history_file: &MutableHistoryFile) -> Result<Vec<u8>> {
    let mut contents = Vec::new();
    for line in &history_file.lines {
        if line.removed {
            continue;
        }
        if line.changed {
            if let Some(value) = &line.value {
                writeln!(contents, "{}", serde_json::to_string(value)?)?;
            }
        } else {
            writeln!(contents, "{}", line.raw)?;
        }
    }
    Ok(contents)
}

fn write_file_replacements_with_rollback<F>(
    replacements: &[(PathBuf, Vec<u8>)],
    mut write_replacement: F,
) -> Result<()>
where
    F: FnMut(&Path, &[u8]) -> Result<()>,
{
    let originals = replacements
        .iter()
        .map(|(path, _)| {
            if path.exists() {
                fs::read(path).map(Some).with_context(|| {
                    format!("Failed to read {} before replacement", path.display())
                })
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>>>()?;

    for (index, (path, contents)) in replacements.iter().enumerate() {
        if let Err(error) = write_replacement(path, contents) {
            let mut rollback_errors = Vec::new();
            for rollback_index in (0..=index).rev() {
                let rollback_path = &replacements[rollback_index].0;
                let rollback_result = match &originals[rollback_index] {
                    Some(original) => write_replacement(rollback_path, original),
                    None if rollback_path.exists() => fs::remove_file(rollback_path)
                        .with_context(|| format!("Failed to remove {}", rollback_path.display())),
                    None => Ok(()),
                };
                if let Err(rollback_error) = rollback_result {
                    rollback_errors
                        .push(format!("{}: {rollback_error:#}", rollback_path.display()));
                }
            }

            if rollback_errors.is_empty() {
                return Err(error);
            }
            bail!(
                "{error:#}; failed to roll back file replacements: {}",
                rollback_errors.join("; ")
            );
        }
    }
    Ok(())
}

fn write_file_bytes_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let temp_path = path.with_extension("jsonl.tmp");
    {
        let mut file = fs::File::create(&temp_path)
            .with_context(|| format!("Failed to create {}", temp_path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("Failed to write {}", temp_path.display()))?;
        file.sync_all()?;
    }
    replace_history_file(&temp_path, path)
}

#[cfg(test)]
fn load_stats_from_paths(paths: &[PathBuf]) -> Result<InputStats> {
    let mut accumulator = InputStatsAccumulator::default();
    let mut seen_ids = HashSet::new();
    add_history_paths_to_stats(paths, &mut seen_ids, &mut accumulator)?;
    Ok(accumulator.finish())
}

fn load_stats_from_sources(stats_path: &Path, history_paths: &[PathBuf]) -> Result<InputStats> {
    let mut accumulator = InputStatsAccumulator::default();
    let mut seen_ids = HashSet::new();

    for event in read_stats_events_in_file_order(stats_path)? {
        if event.schema_version == STATS_SCHEMA_VERSION && seen_ids.insert(event.record_id.clone())
        {
            accumulator.add_event(&event);
        }
    }

    add_history_paths_to_stats(history_paths, &mut seen_ids, &mut accumulator)?;
    Ok(accumulator.finish())
}

fn add_history_paths_to_stats(
    paths: &[PathBuf],
    seen_ids: &mut HashSet<String>,
    accumulator: &mut InputStatsAccumulator,
) -> Result<()> {
    for path in paths {
        add_history_path_to_stats(path, seen_ids, accumulator)?;
    }
    Ok(())
}

fn add_history_path_to_stats(
    path: &Path,
    seen_ids: &mut HashSet<String>,
    accumulator: &mut InputStatsAccumulator,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    for record in read_records_in_file_order(path)? {
        if should_hide_history_record(&record) || !seen_ids.insert(record.id.clone()) {
            continue;
        }
        if let Some(event) = stats_event_from_record(&record) {
            accumulator.add_event(&event);
        }
    }
    Ok(())
}

#[derive(Default)]
struct InputStatsAccumulator {
    total_character_count: u64,
    total_audio_duration_ms: u64,
    daily: BTreeMap<String, DailyInputStats>,
}

impl InputStatsAccumulator {
    fn add_event(&mut self, event: &InputStatsEvent) {
        self.total_character_count += event.character_count;
        self.total_audio_duration_ms += event.audio_duration_ms;
        let entry = self
            .daily
            .entry(event.date.clone())
            .or_insert(DailyInputStats {
                date: event.date.clone(),
                record_count: 0,
                character_count: 0,
                audio_duration_ms: 0,
            });
        entry.record_count += 1;
        entry.character_count += event.character_count;
        entry.audio_duration_ms += event.audio_duration_ms;
    }

    fn finish(self) -> InputStats {
        let average_chars_per_minute = if self.total_audio_duration_ms == 0 {
            0.0
        } else {
            self.total_character_count as f64 / (self.total_audio_duration_ms as f64 / 60_000.0)
        };

        InputStats {
            total_character_count: self.total_character_count,
            total_audio_duration_ms: self.total_audio_duration_ms,
            average_chars_per_minute,
            daily: self.daily.into_values().collect(),
        }
    }
}

fn append_stats_for_record(path: &Path, record: &HistoryRecord) -> Result<()> {
    let Some(event) = stats_event_from_record(record) else {
        return Ok(());
    };
    append_stats_events(path, std::slice::from_ref(&event))
}

fn prepare_stats_file_for_record(
    path: &Path,
    record: &HistoryRecord,
) -> Result<Option<MutableHistoryFile>> {
    let replacement = stats_event_from_record(record)
        .map(serde_json::to_value)
        .transpose()?;
    let mut stats_file = if path.exists() {
        load_mutable_history_files(&[path.to_path_buf()])?
            .pop()
            .ok_or_else(|| anyhow!("Failed to load {}", path.display()))?
    } else {
        MutableHistoryFile {
            path: path.to_path_buf(),
            lines: Vec::new(),
            changed: false,
        }
    };

    let mut found = false;
    for line in &mut stats_file.lines {
        let matches = line
            .value
            .as_ref()
            .and_then(|value| value.get("record_id"))
            .and_then(serde_json::Value::as_str)
            == Some(record.id.as_str());
        if !matches {
            continue;
        }

        if found || replacement.is_none() {
            line.removed = true;
        } else if let (Some(value), Some(replacement)) = (line.value.as_mut(), &replacement) {
            merge_json_fields(value, replacement)?;
            line.changed = true;
        }
        found = true;
        stats_file.changed = true;
    }

    if !found {
        if let Some(replacement) = replacement {
            stats_file.lines.push(MutableHistoryLine {
                raw: String::new(),
                value: Some(replacement),
                record: None,
                removed: false,
                changed: true,
            });
            stats_file.changed = true;
        }
    }

    Ok(stats_file.changed.then_some(stats_file))
}

fn backfill_stats_from_history(stats_path: &Path, history_paths: &[PathBuf]) -> Result<()> {
    let mut seen_ids = stats_record_ids(stats_path)?;
    let mut missing_events = Vec::new();

    for path in history_paths {
        if !path.exists() {
            continue;
        }
        for record in read_records_in_file_order(path)? {
            if should_hide_history_record(&record) || !seen_ids.insert(record.id.clone()) {
                continue;
            }
            if let Some(event) = stats_event_from_record(&record) {
                missing_events.push(event);
            }
        }
    }

    append_stats_events(stats_path, &missing_events)
}

fn stats_record_ids(path: &Path) -> Result<HashSet<String>> {
    Ok(read_stats_events_in_file_order(path)?
        .into_iter()
        .filter(|event| event.schema_version == STATS_SCHEMA_VERSION)
        .map(|event| event.record_id)
        .collect())
}

fn append_stats_events(path: &Path, events: &[InputStatsEvent]) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open {}", path.display()))?;
    for event in events {
        writeln!(file, "{}", serde_json::to_string(event)?)
            .with_context(|| format!("Failed to append {}", path.display()))?;
    }
    Ok(())
}

fn read_stats_events_in_file_order(path: &Path) -> Result<Vec<InputStatsEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file =
        fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<InputStatsEvent>(&line) {
            events.push(event);
        }
    }
    Ok(events)
}

fn stats_event_from_record(record: &HistoryRecord) -> Option<InputStatsEvent> {
    if should_hide_history_record(record) {
        return None;
    }

    Some(InputStatsEvent {
        schema_version: STATS_SCHEMA_VERSION,
        record_id: record.id.clone(),
        date: record
            .created_at
            .with_timezone(&Local)
            .date_naive()
            .to_string(),
        character_count: count_input_characters(record) as u64,
        audio_duration_ms: audio_duration_ms(record),
    })
}

fn count_input_characters(record: &HistoryRecord) -> usize {
    let text = if !record.pasted_text.trim().is_empty() {
        &record.pasted_text
    } else if !record.corrected_text.trim().is_empty() {
        &record.corrected_text
    } else {
        &record.raw_text
    };
    count_chinese_characters_and_english_words(text)
}

fn count_chinese_characters_and_english_words(text: &str) -> usize {
    let mut count = 0usize;
    let mut in_english_word = false;

    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            if !in_english_word {
                count += 1;
                in_english_word = true;
            }
            continue;
        }

        in_english_word = false;
        if is_chinese_character(ch) {
            count += 1;
        }
    }

    count
}

fn is_chinese_character(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
            | '\u{2CEB0}'..='\u{2EBEF}'
            | '\u{2F800}'..='\u{2FA1F}'
            | '\u{30000}'..='\u{3134F}'
    )
}

fn audio_duration_ms(record: &HistoryRecord) -> u64 {
    if record.audio_sample_rate > 0 && record.audio_channels > 0 && record.audio_sample_count > 0 {
        let samples_per_second = record.audio_sample_rate as f64 * record.audio_channels as f64;
        return ((record.audio_sample_count as f64 / samples_per_second) * 1000.0).round() as u64;
    }

    record
        .audio_finished_at
        .signed_duration_since(record.audio_started_at)
        .num_milliseconds()
        .max(0) as u64
}

fn load_from_paths(paths: &[PathBuf], limit: usize, offset: usize) -> Result<Vec<HistoryRecord>> {
    let mut records = Vec::new();
    let mut remaining_offset = offset;

    for path in paths {
        if records.len() >= limit {
            break;
        }

        let page = load_page_from_path(path, limit - records.len(), remaining_offset)?;
        remaining_offset = remaining_offset.saturating_sub(page.visible_count);
        records.extend(page.records);
    }

    Ok(records)
}

#[cfg(test)]
fn load_from_path(path: &PathBuf, limit: usize, offset: usize) -> Result<Vec<HistoryRecord>> {
    Ok(load_page_from_path(path, limit, offset)?.records)
}

fn load_page_from_path(path: &PathBuf, limit: usize, offset: usize) -> Result<HistoryPage> {
    if limit == 0 {
        return Ok(HistoryPage {
            records: Vec::new(),
            visible_count: 0,
        });
    }
    if !path.exists() {
        return Ok(HistoryPage {
            records: Vec::new(),
            visible_count: 0,
        });
    }

    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut position = file.metadata()?.len();
    let mut carry = Vec::new();
    let mut skipped = 0usize;
    let mut records = Vec::new();
    let mut visible_count = 0usize;

    while position > 0 && records.len() < limit {
        let read_size = HISTORY_READ_CHUNK_SIZE.min(position);
        position -= read_size;
        file.seek(SeekFrom::Start(position))?;

        let mut buffer = vec![0; read_size as usize];
        file.read_exact(&mut buffer)?;
        buffer.extend_from_slice(&carry);

        let mut end = buffer.len();
        while records.len() < limit {
            let Some(newline_index) = buffer[..end].iter().rposition(|byte| *byte == b'\n') else {
                break;
            };
            collect_history_line(
                &buffer[newline_index + 1..end],
                offset,
                &mut skipped,
                &mut visible_count,
                &mut records,
            );
            end = newline_index;
        }

        carry = buffer[..end].to_vec();
    }

    if records.len() < limit && !carry.is_empty() {
        collect_history_line(
            &carry,
            offset,
            &mut skipped,
            &mut visible_count,
            &mut records,
        );
    }

    Ok(HistoryPage {
        records,
        visible_count,
    })
}

fn collect_history_line(
    line: &[u8],
    offset: usize,
    skipped: &mut usize,
    visible_count: &mut usize,
    records: &mut Vec<HistoryRecord>,
) {
    if line.iter().all(|byte| byte.is_ascii_whitespace()) {
        return;
    }

    let Ok(record) = serde_json::from_slice::<HistoryRecord>(line) else {
        return;
    };
    if should_hide_history_record(&record) {
        return;
    }
    *visible_count += 1;
    if *skipped < offset {
        *skipped += 1;
        return;
    }

    records.push(record);
}

fn should_hide_history_record(record: &HistoryRecord) -> bool {
    record
        .workflow_error
        .as_deref()
        .is_some_and(is_empty_asr_text_error)
}

pub fn is_empty_asr_text_error(message: &str) -> bool {
    message.contains("Volcengine ASR response did not contain text")
        || message.contains("Volcengine live ASR response did not contain text")
        || message.contains("ASR returned empty text")
}

fn read_records_in_file_order(path: &Path) -> Result<Vec<HistoryRecord>> {
    let file =
        fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<HistoryRecord>(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

#[cfg(not(target_os = "windows"))]
fn replace_history_file(temp_path: &Path, path: &Path) -> Result<()> {
    fs::rename(temp_path, path).with_context(|| format!("Failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn replace_history_file(temp_path: &Path, path: &Path) -> Result<()> {
    let backup_path = path.with_extension("jsonl.replacing");
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .with_context(|| format!("Failed to remove {}", backup_path.display()))?;
    }
    if path.exists() {
        fs::rename(path, &backup_path)
            .with_context(|| format!("Failed to back up {}", path.display()))?;
    }
    if let Err(err) = fs::rename(temp_path, path) {
        if backup_path.exists() {
            let _ = fs::rename(&backup_path, path);
        }
        return Err(err).with_context(|| format!("Failed to replace {}", path.display()));
    }
    if backup_path.exists() {
        if let Err(error) = fs::remove_file(&backup_path) {
            eprintln!(
                "Warning: replaced {} but failed to remove backup {}: {error}",
                path.display(),
                backup_path.display()
            );
        }
    }
    Ok(())
}

#[derive(Clone)]
struct RecordingFileInfo {
    path: PathBuf,
    bytes: u64,
    modified_at: Option<DateTime<Utc>>,
    app_managed_name: bool,
}

fn recording_file_inventory(recordings_dirs: &[PathBuf]) -> Result<Vec<RecordingFileInfo>> {
    let mut seen = HashSet::new();
    let mut inventory = Vec::new();
    for recordings_dir in recordings_dirs {
        let Some(recordings_dir) = normalized_recordings_directory(recordings_dir)? else {
            continue;
        };
        if !recordings_dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&recordings_dir)
            .with_context(|| format!("Failed to read {}", recordings_dir.display()))?
        {
            let path = entry?.path();
            let Some(path) = normalized_safe_recording_path(&path, recordings_dirs)? else {
                continue;
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            let Some(metadata) = regular_recording_metadata(&path, recordings_dirs)? else {
                continue;
            };
            inventory.push(RecordingFileInfo {
                app_managed_name: is_app_managed_recording_path(&path),
                modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
                bytes: metadata.len(),
                path,
            });
        }
    }
    Ok(inventory)
}

fn delete_recording_paths(
    paths: &HashSet<PathBuf>,
    recordings_dirs: &[PathBuf],
) -> Result<(usize, u64)> {
    let mut paths = paths.iter().collect::<Vec<_>>();
    paths.sort();
    let mut deleted_files = 0usize;
    let mut freed_bytes = 0u64;
    let mut errors = Vec::new();

    for path in paths {
        let Some(path) = normalized_safe_recording_path(path, recordings_dirs)? else {
            continue;
        };
        let Some(metadata) = regular_recording_metadata(&path, recordings_dirs)? else {
            continue;
        };
        match fs::remove_file(&path) {
            Ok(()) => {
                deleted_files += 1;
                freed_bytes = freed_bytes.saturating_add(metadata.len());
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => errors.push(format!("{}: {err}", path.display())),
        }
    }

    if !errors.is_empty() {
        bail!("Failed to remove recording files: {}", errors.join("; "));
    }
    Ok((deleted_files, freed_bytes))
}

fn regular_recording_metadata(
    path: &Path,
    recordings_dirs: &[PathBuf],
) -> Result<Option<fs::Metadata>> {
    let Some(path) = normalized_safe_recording_path(path, recordings_dirs)? else {
        return Ok(None);
    };
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() => {
            Ok(None)
        }
        Ok(metadata) => Ok(Some(metadata)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

fn is_app_managed_recording_path(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some((timestamp, id)) = stem.split_once('_') else {
        return false;
    };
    timestamp.parse::<i64>().is_ok() && uuid::Uuid::parse_str(id).is_ok()
}

fn metadata_is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn normalized_recordings_directory(path: &Path) -> Result<Option<PathBuf>> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Ok(None);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() => {
            Ok(None)
        }
        Ok(_) => path
            .canonicalize()
            .map(Some)
            .with_context(|| format!("Failed to resolve {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Some(path.to_path_buf())),
        Err(err) => Err(err).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

fn is_safe_recording_path(path: &Path, recordings_dir: &Path) -> Result<bool> {
    if path.extension().and_then(|value| value.to_str()) != Some("wav") {
        return Ok(false);
    }
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Ok(false);
    }

    let Some(recordings_dir) = normalized_recordings_directory(recordings_dir)? else {
        return Ok(false);
    };
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let parent = match fs::symlink_metadata(parent) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() => {
            return Ok(false);
        }
        Ok(_) => parent
            .canonicalize()
            .with_context(|| format!("Failed to resolve {}", parent.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => parent.to_path_buf(),
        Err(err) => {
            return Err(err).with_context(|| format!("Failed to inspect {}", parent.display()));
        }
    };
    Ok(parent == recordings_dir)
}

fn normalized_safe_recording_path(
    path: &Path,
    recordings_dirs: &[PathBuf],
) -> Result<Option<PathBuf>> {
    for recordings_dir in recordings_dirs {
        if is_safe_recording_path(path, recordings_dir)? {
            let Some(file_name) = path.file_name() else {
                return Ok(None);
            };
            let parent = path.parent().unwrap_or_else(|| Path::new(""));
            let parent = parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf());
            return Ok(Some(parent.join(file_name)));
        }
    }
    Ok(None)
}

fn history_recordings_dirs() -> Result<Vec<PathBuf>> {
    let current_dir = paths::recordings_dir()?;
    let legacy_dir = paths::legacy_history_path()?
        .parent()
        .map(|parent| parent.join("recordings"));
    let mut dirs = vec![current_dir];
    if let Some(legacy_dir) = legacy_dir {
        if !dirs.contains(&legacy_dir) {
            dirs.push(legacy_dir);
        }
    }
    Ok(dirs)
}

fn history_read_paths() -> Result<Vec<PathBuf>> {
    let path = paths::history_path()?;
    let legacy_path = paths::legacy_history_path()?;
    let mut paths = Vec::new();

    if path.exists() {
        paths.push(path.clone());
    }
    if legacy_path.exists() && legacy_path != path {
        paths.push(legacy_path);
    }
    if paths.is_empty() {
        paths.push(path);
    }

    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(id: &str, workflow_error: Option<&str>) -> HistoryRecord {
        HistoryRecord {
            id: id.to_string(),
            created_at: Utc::now(),
            audio_path: Some(PathBuf::from("/tmp/a.wav")),
            asr_provider: "mock".to_string(),
            asr_task_id: None,
            audio_started_at: Utc::now(),
            audio_finished_at: Utc::now(),
            audio_sample_rate: 16000,
            audio_channels: 1,
            audio_sample_count: 16000,
            raw_text: "raw".to_string(),
            corrected_text: "corrected".to_string(),
            pasted_text: "corrected".to_string(),
            correction_enabled: true,
            correction_error: None,
            correction_logs: Vec::new(),
            injection_error: None,
            workflow_error: workflow_error.map(str::to_string),
            asr_duration_ms: None,
            service_audio_duration_ms: None,
            live_asr_diagnostics: None,
            total_duration_ms: 1,
        }
    }

    fn sample_record_with(
        id: &str,
        created_at: &str,
        pasted_text: &str,
        sample_count: usize,
        workflow_error: Option<&str>,
    ) -> HistoryRecord {
        let mut record = sample_record(id, workflow_error);
        record.created_at = DateTime::parse_from_rfc3339(created_at)
            .unwrap()
            .with_timezone(&Utc);
        record.pasted_text = pasted_text.to_string();
        record.corrected_text = pasted_text.to_string();
        record.raw_text = pasted_text.to_string();
        record.audio_sample_count = sample_count;
        record
    }

    fn temp_path(name: &str, extension: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "boltscribe-{name}-{}-{}.{}",
            std::process::id(),
            Utc::now().timestamp_millis(),
            extension
        ))
    }

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "boltscribe-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_millis()
        ))
    }

    fn managed_recording_path(recordings_dir: &Path, sequence: u64) -> PathBuf {
        recordings_dir.join(format!(
            "1780000000_00000000-0000-4000-8000-{sequence:012x}.wav"
        ))
    }

    fn utc(timestamp: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(timestamp)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn write_history_lines(path: &Path, lines: &[String]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    #[test]
    fn old_history_without_live_asr_diagnostics_remains_readable() {
        let record = sample_record("legacy", None);
        let mut value = serde_json::to_value(record).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("live_asr_diagnostics");

        let loaded: HistoryRecord = serde_json::from_value(value).unwrap();

        assert!(loaded.live_asr_diagnostics.is_none());
    }

    #[test]
    fn history_record_serializes() {
        let record = sample_record("1", None);
        assert!(serde_json::to_string(&record)
            .unwrap()
            .contains("corrected"));
    }

    #[test]
    fn load_retryable_requires_failed_record_with_safe_regular_wav() {
        let base = temp_dir("load-retryable");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        let outside_dir = base.join("outside");
        fs::create_dir_all(&recordings_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();

        let safe_path = recordings_dir.join("safe.wav");
        let missing_path = recordings_dir.join("missing.wav");
        let directory_path = recordings_dir.join("directory.wav");
        let wrong_extension_path = recordings_dir.join("recording.mp3");
        let outside_path = outside_dir.join("outside.wav");
        fs::write(&safe_path, b"safe wav").unwrap();
        fs::create_dir_all(&directory_path).unwrap();
        fs::write(&wrong_extension_path, b"not wav").unwrap();
        fs::write(&outside_path, b"outside").unwrap();

        let failed_record = |id: &str, audio_path: Option<PathBuf>| {
            let mut record = sample_record(id, Some("Failed to connect ASR websocket"));
            record.audio_path = audio_path;
            record.raw_text.clear();
            record.corrected_text.clear();
            record.pasted_text.clear();
            record
        };
        let mut record_with_text = failed_record("has-text", Some(safe_path.clone()));
        record_with_text.raw_text = "already transcribed".to_string();
        let mut records = vec![
            failed_record("safe", Some(recordings_dir.join(".").join("safe.wav"))),
            sample_record("successful", None),
            sample_record("blank-error", Some("   ")),
            failed_record("no-audio", None),
            failed_record("missing", Some(missing_path)),
            failed_record("directory", Some(directory_path)),
            failed_record("wrong-extension", Some(wrong_extension_path)),
            failed_record("outside", Some(outside_path.clone())),
            failed_record(
                "parent-traversal",
                Some(
                    recordings_dir
                        .join("..")
                        .join("outside")
                        .join("outside.wav"),
                ),
            ),
            failed_record("relative", Some(PathBuf::from("recordings/safe.wav"))),
            record_with_text,
        ];

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let symlink_path = recordings_dir.join("symlink.wav");
            symlink(&safe_path, &symlink_path).unwrap();
            records.push(failed_record("symlink", Some(symlink_path)));
        }

        write_history_lines(
            &history_path,
            &records
                .iter()
                .map(|record| serde_json::to_string(record).unwrap())
                .collect::<Vec<_>>(),
        );

        let loaded = load_retryable_from_paths(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            " safe ",
        )
        .unwrap();
        assert_eq!(loaded.audio_path, Some(safe_path.canonicalize().unwrap()));

        let mut rejected_ids = vec![
            "successful",
            "blank-error",
            "no-audio",
            "missing",
            "directory",
            "wrong-extension",
            "outside",
            "parent-traversal",
            "relative",
            "has-text",
            "unknown",
            "",
        ];
        #[cfg(unix)]
        rejected_ids.push("symlink");
        for record_id in rejected_ids {
            assert!(
                load_retryable_from_paths(
                    std::slice::from_ref(&history_path),
                    std::slice::from_ref(&recordings_dir),
                    record_id,
                )
                .is_err(),
                "expected {record_id:?} to be rejected"
            );
        }

        assert!(outside_path.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn replace_updates_all_matching_records_and_replaces_stats_without_duplicates() {
        let base = temp_dir("replace-history-record");
        let first_history_path = base.join("history.jsonl");
        let second_history_path = base.join("legacy-history.jsonl");
        let stats_path = base.join("input_stats.jsonl");

        let first_record = sample_record_with(
            "retry-me",
            "2026-07-20T08:00:00Z",
            "",
            16_000,
            Some("Failed to connect ASR websocket"),
        );
        let second_record = first_record.clone();
        let mut first_value = serde_json::to_value(first_record.clone()).unwrap();
        first_value["future_history_field"] = serde_json::json!({ "source": "current" });
        let mut second_value = serde_json::to_value(second_record).unwrap();
        second_value["future_history_field"] = serde_json::json!({ "source": "legacy" });
        let unrelated_raw = "  {\"future_only\":true}  ".to_string();
        let malformed_history_raw = "{not valid history json".to_string();
        write_history_lines(
            &first_history_path,
            &[
                serde_json::to_string(&first_value).unwrap(),
                unrelated_raw.clone(),
                malformed_history_raw.clone(),
            ],
        );
        write_history_lines(
            &second_history_path,
            &[serde_json::to_string(&second_value).unwrap()],
        );

        let old_event = InputStatsEvent {
            schema_version: STATS_SCHEMA_VERSION,
            record_id: "retry-me".to_string(),
            date: "2026-07-20".to_string(),
            character_count: 0,
            audio_duration_ms: 1_000,
        };
        let mut old_event_value = serde_json::to_value(old_event.clone()).unwrap();
        old_event_value["future_stats_field"] = serde_json::json!("preserved");
        let duplicate_event = InputStatsEvent {
            character_count: 999,
            ..old_event
        };
        let malformed_stats_raw = "{not valid stats json".to_string();
        write_history_lines(
            &stats_path,
            &[
                serde_json::to_string(&old_event_value).unwrap(),
                serde_json::to_string(&duplicate_event).unwrap(),
                malformed_stats_raw.clone(),
            ],
        );

        let mut replacement = first_record;
        replacement.raw_text = "你好 world".to_string();
        replacement.corrected_text = "你好 world".to_string();
        replacement.pasted_text = "你好 world".to_string();
        replacement.audio_sample_count = 32_000;
        replacement.workflow_error = None;
        replace_in_paths(
            &[first_history_path.clone(), second_history_path.clone()],
            &stats_path,
            &replacement,
        )
        .unwrap();

        for (path, expected_source) in [
            (&first_history_path, "current"),
            (&second_history_path, "legacy"),
        ] {
            let matching = fs::read_to_string(path)
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .find(|value| value["id"] == "retry-me")
                .unwrap();
            assert_eq!(matching["raw_text"], "你好 world");
            assert!(matching["workflow_error"].is_null());
            assert_eq!(
                matching["future_history_field"],
                serde_json::json!({ "source": expected_source })
            );
        }
        let first_history_contents = fs::read_to_string(&first_history_path).unwrap();
        assert!(first_history_contents
            .lines()
            .any(|line| line == unrelated_raw));
        assert!(first_history_contents
            .lines()
            .any(|line| line == malformed_history_raw));

        let stats_contents = fs::read_to_string(&stats_path).unwrap();
        assert!(stats_contents
            .lines()
            .any(|line| line == malformed_stats_raw));
        let matching_events = stats_contents
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|value| value["record_id"] == "retry-me")
            .collect::<Vec<_>>();
        assert_eq!(matching_events.len(), 1);
        assert_eq!(matching_events[0]["character_count"], 3);
        assert_eq!(matching_events[0]["audio_duration_ms"], 2_000);
        assert_eq!(matching_events[0]["future_stats_field"], "preserved");

        let stats = load_stats_from_sources(
            &stats_path,
            &[first_history_path.clone(), second_history_path.clone()],
        )
        .unwrap();
        assert_eq!(stats.total_character_count, 3);
        assert_eq!(stats.total_audio_duration_ms, 2_000);
        assert_eq!(
            stats.daily.iter().map(|day| day.record_count).sum::<u64>(),
            1
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn replace_appends_missing_stats_event_and_preserves_malformed_line() {
        let base = temp_dir("replace-appends-stats");
        let history_path = base.join("history.jsonl");
        let stats_path = base.join("input_stats.jsonl");
        let malformed_stats_raw = "{malformed stats".to_string();
        let failed_record = sample_record("retry-me", Some("network failure"));
        write_history_lines(
            &history_path,
            &[serde_json::to_string(&failed_record).unwrap()],
        );
        write_history_lines(&stats_path, std::slice::from_ref(&malformed_stats_raw));

        let mut replacement = failed_record;
        replacement.pasted_text = "retry works".to_string();
        replacement.workflow_error = None;
        replace_in_paths(
            std::slice::from_ref(&history_path),
            &stats_path,
            &replacement,
        )
        .unwrap();

        let contents = fs::read_to_string(&stats_path).unwrap();
        assert_eq!(contents.lines().next(), Some(malformed_stats_raw.as_str()));
        assert_eq!(
            read_stats_events_in_file_order(&stats_path)
                .unwrap()
                .iter()
                .filter(|event| event.record_id == "retry-me")
                .count(),
            1
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn multi_file_replacement_rolls_back_already_replaced_files_on_failure() {
        let base = temp_dir("replacement-rollback");
        let first_path = base.join("input_stats.jsonl");
        let second_path = base.join("history.jsonl");
        fs::create_dir_all(&base).unwrap();
        fs::write(&first_path, b"old stats\n").unwrap();
        fs::write(&second_path, b"old history\n").unwrap();
        let replacements = vec![
            (first_path.clone(), b"new stats\n".to_vec()),
            (second_path.clone(), b"new history\n".to_vec()),
        ];
        let mut injected_failure = false;

        let result =
            write_file_replacements_with_rollback(&replacements, |path: &Path, contents: &[u8]| {
                if path == second_path && !injected_failure {
                    injected_failure = true;
                    bail!("injected replacement failure");
                }
                write_file_bytes_atomically(path, contents)
            });

        assert!(result.is_err());
        assert_eq!(fs::read(&first_path).unwrap(), b"old stats\n");
        assert_eq!(fs::read(&second_path).unwrap(), b"old history\n");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn empty_asr_text_errors_are_ignorable() {
        assert!(is_empty_asr_text_error(
            "Volcengine ASR response did not contain text, log_id=None"
        ));
        assert!(is_empty_asr_text_error(
            "Volcengine live ASR response did not contain text, log_id=None"
        ));
        assert!(is_empty_asr_text_error("ASR returned empty text"));
        assert!(!is_empty_asr_text_error(
            "Volcengine ASR websocket timed out"
        ));
    }

    #[test]
    fn counts_only_chinese_characters_and_english_words() {
        assert_eq!(
            count_chinese_characters_and_english_words("你好，Agent! LDFC works. 123"),
            5
        );
        assert_eq!(
            count_chinese_characters_and_english_words("Hello, world! 这是 GPT-5.4-mini。"),
            6
        );
        assert_eq!(
            count_chinese_characters_and_english_words("，。！？ 123"),
            0
        );
    }

    #[test]
    fn load_from_path_pages_newest_visible_records() {
        let path = std::env::temp_dir().join(format!(
            "boltscribe-history-test-{}-{}.jsonl",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));
        let mut file = fs::File::create(&path).unwrap();
        let records = [
            sample_record("1", None),
            sample_record("2", None),
            sample_record(
                "3",
                Some("Volcengine ASR response did not contain text, log_id=None"),
            ),
            sample_record("4", None),
            sample_record("5", None),
        ];
        for record in records {
            writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        }

        let first_page = load_from_path(&path, 2, 0).unwrap();
        let second_page = load_from_path(&path, 2, 2).unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(
            first_page
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "4"]
        );
        assert_eq!(
            second_page
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["2", "1"]
        );
    }

    #[test]
    fn load_from_path_returns_empty_when_history_file_is_missing() {
        let path = std::env::temp_dir().join(format!(
            "boltscribe-missing-history-test-{}-{}.jsonl",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));

        let records = load_from_path(&path, 20, 0).unwrap();

        assert!(records.is_empty());
    }

    #[test]
    fn load_from_paths_continues_into_legacy_records() {
        let new_path = std::env::temp_dir().join(format!(
            "boltscribe-history-new-test-{}-{}.jsonl",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));
        let legacy_path = std::env::temp_dir().join(format!(
            "boltscribe-history-legacy-test-{}-{}.jsonl",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));

        let mut new_file = fs::File::create(&new_path).unwrap();
        for record in [sample_record("new-1", None), sample_record("new-2", None)] {
            writeln!(new_file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        }

        let mut legacy_file = fs::File::create(&legacy_path).unwrap();
        for record in [
            sample_record("legacy-1", None),
            sample_record("legacy-2", None),
            sample_record("legacy-3", None),
        ] {
            writeln!(legacy_file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        }

        let first_page = load_from_paths(&[new_path.clone(), legacy_path.clone()], 3, 0).unwrap();
        let second_page = load_from_paths(&[new_path.clone(), legacy_path.clone()], 3, 3).unwrap();
        let _ = fs::remove_file(new_path);
        let _ = fs::remove_file(legacy_path);

        assert_eq!(
            first_page
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["new-2", "new-1", "legacy-3"]
        );
        assert_eq!(
            second_page
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["legacy-2", "legacy-1"]
        );
    }

    #[test]
    fn load_stats_from_paths_summarizes_visible_input() {
        let path = std::env::temp_dir().join(format!(
            "boltscribe-history-stats-test-{}-{}.jsonl",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));
        let mut file = fs::File::create(&path).unwrap();
        let records = [
            HistoryRecord {
                pasted_text: "你好，world!".to_string(),
                audio_sample_count: 16_000,
                ..sample_record("1", None)
            },
            HistoryRecord {
                pasted_text: "Agent works.".to_string(),
                audio_sample_count: 32_000,
                ..sample_record("2", None)
            },
            HistoryRecord {
                pasted_text: "ignored".to_string(),
                audio_sample_count: 64_000,
                ..sample_record(
                    "3",
                    Some("Volcengine ASR response did not contain text, log_id=None"),
                )
            },
        ];
        for record in records {
            writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        }

        let stats = load_stats_from_paths(std::slice::from_ref(&path)).unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(stats.total_character_count, 5);
        assert_eq!(stats.total_audio_duration_ms, 3_000);
        assert!((stats.average_chars_per_minute - 100.0).abs() < 0.01);
        assert_eq!(stats.daily.len(), 1);
        assert_eq!(stats.daily[0].record_count, 2);
        assert_eq!(stats.daily[0].character_count, 5);
    }

    #[test]
    fn stats_event_from_history_record_uses_minimal_stats_fields() {
        let record = sample_record_with(
            "ledger-1",
            "2026-05-16T12:00:00Z",
            "你好，world!",
            24_000,
            None,
        );
        let event = stats_event_from_record(&record).unwrap();

        assert_eq!(event.schema_version, STATS_SCHEMA_VERSION);
        assert_eq!(event.record_id, "ledger-1");
        assert_eq!(
            event.date,
            record
                .created_at
                .with_timezone(&Local)
                .date_naive()
                .to_string()
        );
        assert_eq!(event.character_count, 3);
        assert_eq!(event.audio_duration_ms, 1_500);
        assert!(
            stats_event_from_record(&sample_record("hidden", Some("ASR returned empty text")))
                .is_none()
        );
    }

    #[test]
    fn stats_ledger_summarizes_input_events() {
        let path = temp_path("stats-ledger-summary", "jsonl");
        let events = [
            InputStatsEvent {
                schema_version: STATS_SCHEMA_VERSION,
                record_id: "1".to_string(),
                date: "2026-05-16".to_string(),
                character_count: 5,
                audio_duration_ms: 3_000,
            },
            InputStatsEvent {
                schema_version: STATS_SCHEMA_VERSION,
                record_id: "2".to_string(),
                date: "2026-05-16".to_string(),
                character_count: 10,
                audio_duration_ms: 2_000,
            },
        ];
        append_stats_events(&path, &events).unwrap();

        let stats = load_stats_from_sources(&path, &[]).unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(stats.total_character_count, 15);
        assert_eq!(stats.total_audio_duration_ms, 5_000);
        assert!((stats.average_chars_per_minute - 180.0).abs() < 0.01);
        assert_eq!(stats.daily.len(), 1);
        assert_eq!(stats.daily[0].record_count, 2);
        assert_eq!(stats.daily[0].character_count, 15);
    }

    #[test]
    fn stats_sources_deduplicate_ledger_and_history_records() {
        let stats_path = temp_path("stats-dedupe-ledger", "jsonl");
        let history_path = temp_path("stats-dedupe-history", "jsonl");
        append_stats_events(
            &stats_path,
            &[InputStatsEvent {
                schema_version: STATS_SCHEMA_VERSION,
                record_id: "same".to_string(),
                date: "2026-05-16".to_string(),
                character_count: 5,
                audio_duration_ms: 1_000,
            }],
        )
        .unwrap();

        let mut file = fs::File::create(&history_path).unwrap();
        for record in [
            sample_record_with("same", "2026-05-16T12:00:00Z", "重复不计", 16_000, None),
            sample_record_with("new", "2026-05-17T12:00:00Z", "新增", 32_000, None),
        ] {
            writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        }

        let stats =
            load_stats_from_sources(&stats_path, std::slice::from_ref(&history_path)).unwrap();
        let _ = fs::remove_file(stats_path);
        let _ = fs::remove_file(history_path);

        assert_eq!(stats.total_audio_duration_ms, 3_000);
        assert_eq!(stats.total_character_count, 7);
        assert_eq!(stats.daily.len(), 2);
    }

    #[test]
    fn backfill_stats_from_history_writes_missing_visible_records() {
        let stats_path = temp_path("stats-backfill", "jsonl");
        let history_path = temp_path("history-backfill", "jsonl");
        append_stats_events(
            &stats_path,
            &[InputStatsEvent {
                schema_version: STATS_SCHEMA_VERSION,
                record_id: "existing".to_string(),
                date: "2026-05-16".to_string(),
                character_count: 1,
                audio_duration_ms: 1_000,
            }],
        )
        .unwrap();

        let mut file = fs::File::create(&history_path).unwrap();
        for record in [
            sample_record_with("existing", "2026-05-16T12:00:00Z", "已有", 16_000, None),
            sample_record_with("missing", "2026-05-17T12:00:00Z", "补回", 32_000, None),
            sample_record_with(
                "hidden",
                "2026-05-18T12:00:00Z",
                "忽略",
                48_000,
                Some("ASR returned empty text"),
            ),
        ] {
            writeln!(file, "{}", serde_json::to_string(&record).unwrap()).unwrap();
        }

        backfill_stats_from_history(&stats_path, std::slice::from_ref(&history_path)).unwrap();
        let events = read_stats_events_in_file_order(&stats_path).unwrap();
        let _ = fs::remove_file(stats_path);
        let _ = fs::remove_file(history_path);

        assert_eq!(
            events
                .iter()
                .map(|event| event.record_id.as_str())
                .collect::<Vec<_>>(),
            vec!["existing", "missing"]
        );
    }

    #[test]
    fn stats_ledger_retains_counts_after_history_prune() {
        let base = temp_dir("stats-prune-retains");
        let history_path = base.join("history.jsonl");
        let stats_path = base.join("input_stats.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let old_record = sample_record_with("old", "2026-05-16T12:00:00Z", "旧记录", 16_000, None);
        let new_record = sample_record_with("new", "2026-06-03T12:00:00Z", "新记录", 32_000, None);
        append_history_record(&history_path, &old_record).unwrap();
        backfill_stats_from_history(&stats_path, std::slice::from_ref(&history_path)).unwrap();

        append_to_paths(
            &new_record,
            &RetentionConfig {
                max_history_records: 1,
                max_storage_bytes: u64::MAX,
            },
            &history_path,
            &stats_path,
            &recordings_dir,
        )
        .unwrap();

        let records = read_records_in_file_order(&history_path).unwrap();
        let stats =
            load_stats_from_sources(&stats_path, std::slice::from_ref(&history_path)).unwrap();
        let _ = fs::remove_dir_all(base);

        assert_eq!(
            records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["new"]
        );
        assert_eq!(stats.total_audio_duration_ms, 3_000);
        assert_eq!(stats.daily.len(), 2);
    }

    #[test]
    fn append_stats_failure_does_not_block_history_prune() {
        let base = temp_dir("stats-failure-still-prunes");
        let history_path = base.join("history.jsonl");
        let stats_path = base.join("input_stats.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&stats_path).unwrap();
        fs::create_dir_all(&recordings_dir).unwrap();

        append_history_record(
            &history_path,
            &sample_record_with("old", "2026-05-16T12:00:00Z", "旧记录", 16_000, None),
        )
        .unwrap();
        let result = append_to_paths(
            &sample_record_with("new", "2026-06-03T12:00:00Z", "新记录", 32_000, None),
            &RetentionConfig {
                max_history_records: 1,
                max_storage_bytes: u64::MAX,
            },
            &history_path,
            &stats_path,
            &recordings_dir,
        );

        let records = read_records_in_file_order(&history_path).unwrap();
        let _ = fs::remove_dir_all(base);

        assert!(result.is_ok());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "new");
    }

    #[test]
    fn prune_preserves_raw_lines_and_audio_referenced_by_a_kept_record() {
        let base = temp_dir("prune-preserves-raw-and-shared-audio");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let shared_audio_path = managed_recording_path(&recordings_dir, 1);
        fs::write(&shared_audio_path, b"shared").unwrap();
        let old_record = HistoryRecord {
            audio_path: Some(shared_audio_path.clone()),
            ..sample_record_with("old", "2026-06-01T00:00:00Z", "old", 16_000, None)
        };
        let new_record = HistoryRecord {
            audio_path: Some(shared_audio_path.clone()),
            ..sample_record_with("new", "2026-06-02T00:00:00Z", "new", 16_000, None)
        };
        let mut new_value = serde_json::to_value(new_record).unwrap();
        new_value["future_field"] = serde_json::json!({ "preserved": true });
        let new_raw_line = format!("  {}  ", serde_json::to_string(&new_value).unwrap());
        let malformed_line = "{future malformed history".to_string();
        write_history_lines(
            &history_path,
            &[
                serde_json::to_string(&old_record).unwrap(),
                new_raw_line.clone(),
                malformed_line.clone(),
            ],
        );

        prune_paths_at(
            std::slice::from_ref(&history_path),
            &RetentionConfig {
                max_history_records: 1,
                max_storage_bytes: u64::MAX,
            },
            std::slice::from_ref(&recordings_dir),
            Utc::now(),
        )
        .unwrap();

        assert!(shared_audio_path.exists());
        assert_eq!(
            fs::read_to_string(&history_path).unwrap(),
            format!("{new_raw_line}\n{malformed_line}\n")
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn prune_enforces_recording_storage_across_current_and_legacy_history() {
        let base = temp_dir("prune-current-and-legacy-storage");
        let current_history = base.join("current-history.jsonl");
        let legacy_history = base.join("legacy-history.jsonl");
        let current_recordings = base.join("current-recordings");
        let legacy_recordings = base.join("legacy-recordings");
        fs::create_dir_all(&current_recordings).unwrap();
        fs::create_dir_all(&legacy_recordings).unwrap();

        let current_audio = managed_recording_path(&current_recordings, 2);
        let legacy_audio = managed_recording_path(&legacy_recordings, 3);
        fs::write(&current_audio, b"newest").unwrap();
        fs::write(&legacy_audio, b"oldest").unwrap();
        let current_record = HistoryRecord {
            audio_path: Some(current_audio.clone()),
            ..sample_record_with("current", "2026-06-02T00:00:00Z", "new", 16_000, None)
        };
        let legacy_record = HistoryRecord {
            audio_path: Some(legacy_audio.clone()),
            ..sample_record_with("legacy", "2026-06-01T00:00:00Z", "old", 16_000, None)
        };
        write_history_lines(
            &current_history,
            &[serde_json::to_string(&current_record).unwrap()],
        );
        write_history_lines(
            &legacy_history,
            &[serde_json::to_string(&legacy_record).unwrap()],
        );

        prune_paths_at(
            &[current_history.clone(), legacy_history.clone()],
            &RetentionConfig {
                max_history_records: 10,
                max_storage_bytes: 6,
            },
            &[current_recordings, legacy_recordings],
            Utc::now(),
        )
        .unwrap();

        assert!(current_audio.exists());
        assert!(!legacy_audio.exists());
        assert_eq!(
            read_records_in_file_order(&current_history).unwrap().len(),
            1
        );
        assert!(read_records_in_file_order(&legacy_history)
            .unwrap()
            .is_empty());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn prune_reclaims_managed_orphans_after_the_grace_period() {
        let base = temp_dir("prune-old-managed-orphan");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();
        let orphan_audio = managed_recording_path(&recordings_dir, 4);
        fs::write(&orphan_audio, b"orphan").unwrap();
        let modified =
            DateTime::<Utc>::from(fs::metadata(&orphan_audio).unwrap().modified().unwrap());
        let retention = RetentionConfig {
            max_history_records: 10,
            max_storage_bytes: u64::MAX,
        };

        prune_paths_at(
            &[],
            &retention,
            std::slice::from_ref(&recordings_dir),
            modified + ChronoDuration::hours(ORPHAN_RECORDING_GRACE_HOURS - 1),
        )
        .unwrap();
        assert!(orphan_audio.exists());

        prune_paths_at(
            &[],
            &retention,
            std::slice::from_ref(&recordings_dir),
            modified + ChronoDuration::hours(ORPHAN_RECORDING_GRACE_HOURS + 1),
        )
        .unwrap();
        assert!(!orphan_audio.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn recording_directory_symlink_cannot_escape_cleanup_boundary() {
        use std::os::unix::fs::symlink;

        let base = temp_dir("cleanup-rejects-recordings-symlink");
        let outside_dir = base.join("outside");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&outside_dir).unwrap();
        symlink(&outside_dir, &recordings_dir).unwrap();
        let outside_audio = managed_recording_path(&outside_dir, 5);
        fs::write(&outside_audio, b"outside").unwrap();
        let linked_audio = recordings_dir.join(outside_audio.file_name().unwrap());

        let result = delete_recording_paths(
            &HashSet::from([linked_audio]),
            std::slice::from_ref(&recordings_dir),
        )
        .unwrap();

        assert_eq!(result, (0, 0));
        assert!(outside_audio.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn delete_recording_paths_only_removes_safe_regular_files() {
        let base = std::env::temp_dir().join(format!(
            "boltscribe-history-delete-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));
        let recordings_dir = base.join("recordings");
        let outside_dir = base.join("outside");
        fs::create_dir_all(&recordings_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();

        let safe_path = recordings_dir.join("safe.wav");
        let unsafe_path = outside_dir.join("unsafe.wav");
        fs::write(&safe_path, b"safe").unwrap();
        fs::write(&unsafe_path, b"unsafe").unwrap();

        let paths = HashSet::from([safe_path.clone(), unsafe_path.clone()]);
        let result = delete_recording_paths(&paths, std::slice::from_ref(&recordings_dir)).unwrap();
        assert_eq!(result, (1, 4));
        assert!(!safe_path.exists());
        assert!(unsafe_path.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn delete_history_record_preserves_unrelated_raw_lines_and_unknown_fields() {
        let base = temp_dir("delete-preserves-raw-lines");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let deleted_audio_path = recordings_dir.join("deleted.wav");
        let kept_audio_path = recordings_dir.join("kept.wav");
        fs::write(&deleted_audio_path, b"delete me").unwrap();
        fs::write(&kept_audio_path, b"keep me").unwrap();

        let deleted_record = HistoryRecord {
            audio_path: Some(deleted_audio_path.clone()),
            ..sample_record("delete-me", None)
        };
        let kept_record = HistoryRecord {
            audio_path: Some(kept_audio_path.clone()),
            ..sample_record("keep-me", None)
        };
        let mut kept_value = serde_json::to_value(kept_record).unwrap();
        kept_value["future_field"] = serde_json::json!({ "enabled": true });
        let kept_raw_line = format!("  {}  ", serde_json::to_string(&kept_value).unwrap());
        let malformed_line = "{this is not valid json".to_string();
        write_history_lines(
            &history_path,
            &[
                serde_json::to_string(&deleted_record).unwrap(),
                kept_raw_line.clone(),
                malformed_line.clone(),
            ],
        );

        let result = delete_from_paths(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            "delete-me",
        )
        .unwrap();

        assert_eq!(
            result,
            DeleteHistoryResult {
                deleted_records: 1,
                deleted_audio_files: 1,
                freed_bytes: 9,
            }
        );
        assert!(!deleted_audio_path.exists());
        assert!(kept_audio_path.exists());
        assert_eq!(
            fs::read_to_string(&history_path).unwrap(),
            format!("{kept_raw_line}\n{malformed_line}\n")
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn delete_history_record_keeps_audio_referenced_by_another_record() {
        let base = temp_dir("delete-keeps-shared-audio");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let shared_audio_path = recordings_dir.join("shared.wav");
        fs::write(&shared_audio_path, b"shared").unwrap();
        let records = [
            HistoryRecord {
                audio_path: Some(shared_audio_path.clone()),
                ..sample_record("delete-me", None)
            },
            HistoryRecord {
                audio_path: Some(shared_audio_path.clone()),
                ..sample_record("keep-me", None)
            },
        ];
        write_history_lines(
            &history_path,
            &records
                .iter()
                .map(|record| serde_json::to_string(record).unwrap())
                .collect::<Vec<_>>(),
        );

        let result = delete_from_paths(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            "delete-me",
        )
        .unwrap();

        assert_eq!(result.deleted_records, 1);
        assert_eq!(result.deleted_audio_files, 0);
        assert_eq!(result.freed_bytes, 0);
        assert!(shared_audio_path.exists());
        let remaining = read_records_in_file_order(&history_path).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "keep-me");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn delete_history_record_normalizes_shared_audio_paths_before_deleting() {
        let base = temp_dir("delete-normalizes-shared-audio");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let shared_audio_path = recordings_dir.join("shared.wav");
        let equivalent_audio_path = recordings_dir.join(".").join("shared.wav");
        fs::write(&shared_audio_path, b"shared").unwrap();
        let records = [
            HistoryRecord {
                audio_path: Some(shared_audio_path.clone()),
                ..sample_record("delete-me", None)
            },
            HistoryRecord {
                audio_path: Some(equivalent_audio_path),
                ..sample_record("keep-me", None)
            },
        ];
        write_history_lines(
            &history_path,
            &records
                .iter()
                .map(|record| serde_json::to_string(record).unwrap())
                .collect::<Vec<_>>(),
        );

        let result = delete_from_paths(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            "delete-me",
        )
        .unwrap();

        assert_eq!(result.deleted_audio_files, 0);
        assert!(shared_audio_path.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn delete_history_record_never_removes_audio_outside_recordings_dirs() {
        let base = temp_dir("delete-rejects-unsafe-audio");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        let outside_dir = base.join("outside");
        fs::create_dir_all(&recordings_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();

        let outside_audio_path = outside_dir.join("outside.wav");
        fs::write(&outside_audio_path, b"must remain").unwrap();
        let record = HistoryRecord {
            audio_path: Some(outside_audio_path.clone()),
            ..sample_record("delete-me", None)
        };
        write_history_lines(&history_path, &[serde_json::to_string(&record).unwrap()]);

        let result = delete_from_paths(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            "delete-me",
        )
        .unwrap();

        assert_eq!(result.deleted_records, 1);
        assert_eq!(result.deleted_audio_files, 0);
        assert_eq!(result.freed_bytes, 0);
        assert!(outside_audio_path.exists());
        assert!(read_records_in_file_order(&history_path)
            .unwrap()
            .is_empty());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn cleanup_uses_audio_finished_at_and_keeps_history_with_null_audio_path() {
        let base = temp_dir("cleanup-uses-audio-finished-at");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let old_audio_path = recordings_dir.join("old.wav");
        let recent_audio_path = recordings_dir.join("recent.wav");
        let boundary_audio_path = recordings_dir.join("boundary.wav");
        fs::write(&old_audio_path, b"old").unwrap();
        fs::write(&recent_audio_path, b"recent").unwrap();
        fs::write(&boundary_audio_path, b"boundary").unwrap();

        let cutoff = utc("2026-07-09T12:00:00Z");
        let mut old_record = sample_record("old-audio", None);
        old_record.created_at = utc("2026-07-15T12:00:00Z");
        old_record.audio_finished_at = utc("2026-07-08T12:00:00Z");
        old_record.audio_path = Some(old_audio_path.clone());

        let mut recent_record = sample_record("recent-audio", None);
        recent_record.created_at = utc("2025-01-01T12:00:00Z");
        recent_record.audio_finished_at = utc("2026-07-10T12:00:00Z");
        recent_record.audio_path = Some(recent_audio_path.clone());

        let mut boundary_record = sample_record("boundary-audio", None);
        boundary_record.audio_finished_at = cutoff;
        boundary_record.audio_path = Some(boundary_audio_path.clone());

        let mut old_value = serde_json::to_value(old_record).unwrap();
        old_value["future_field"] = serde_json::json!("preserved");
        write_history_lines(
            &history_path,
            &[
                serde_json::to_string(&old_value).unwrap(),
                serde_json::to_string(&recent_record).unwrap(),
                serde_json::to_string(&boundary_record).unwrap(),
            ],
        );

        let result = cleanup_recordings_before(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            cutoff,
        )
        .unwrap();

        assert_eq!(
            result,
            RecordingCleanupResult {
                deleted_files: 1,
                cleared_history_records: 1,
                freed_bytes: 3,
            }
        );
        assert!(!old_audio_path.exists());
        assert!(recent_audio_path.exists());
        assert!(boundary_audio_path.exists());

        let contents = fs::read_to_string(&history_path).unwrap();
        let values = contents
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 3);
        let old_value = values
            .iter()
            .find(|value| value["id"] == "old-audio")
            .unwrap();
        assert!(old_value["audio_path"].is_null());
        assert_eq!(old_value["future_field"], "preserved");
        assert_eq!(
            values
                .iter()
                .find(|value| value["id"] == "recent-audio")
                .unwrap()["audio_path"],
            recent_audio_path.to_string_lossy().as_ref()
        );
        assert_eq!(
            values
                .iter()
                .find(|value| value["id"] == "boundary-audio")
                .unwrap()["audio_path"],
            boundary_audio_path.to_string_lossy().as_ref()
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn cleanup_never_removes_audio_outside_recordings_dirs() {
        let base = temp_dir("cleanup-rejects-unsafe-audio");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        let outside_dir = base.join("outside");
        fs::create_dir_all(&recordings_dir).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();

        let outside_audio_path = outside_dir.join("outside.wav");
        fs::write(&outside_audio_path, b"must remain").unwrap();
        let mut old_record = sample_record("outside-audio", None);
        old_record.audio_finished_at = utc("2026-07-01T12:00:00Z");
        old_record.audio_path = Some(outside_audio_path.clone());
        write_history_lines(
            &history_path,
            &[serde_json::to_string(&old_record).unwrap()],
        );

        let result = cleanup_recordings_before(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            utc("2026-07-09T12:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            result,
            RecordingCleanupResult {
                deleted_files: 0,
                cleared_history_records: 0,
                freed_bytes: 0,
            }
        );
        assert!(outside_audio_path.exists());
        let records = read_records_in_file_order(&history_path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].audio_path.as_ref(), Some(&outside_audio_path));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn cleanup_keeps_old_audio_when_a_new_record_shares_its_path() {
        let base = temp_dir("cleanup-protects-shared-audio");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let shared_audio_path = recordings_dir.join("shared.wav");
        fs::write(&shared_audio_path, b"shared").unwrap();
        let mut old_record = sample_record("old-reference", None);
        old_record.audio_finished_at = utc("2026-07-01T12:00:00Z");
        old_record.audio_path = Some(shared_audio_path.clone());
        let mut new_record = sample_record("new-reference", None);
        new_record.audio_finished_at = utc("2026-07-15T12:00:00Z");
        new_record.audio_path = Some(shared_audio_path.clone());
        write_history_lines(
            &history_path,
            &[
                serde_json::to_string(&old_record).unwrap(),
                serde_json::to_string(&new_record).unwrap(),
            ],
        );

        let result = cleanup_recordings_before(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            utc("2026-07-09T12:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            result,
            RecordingCleanupResult {
                deleted_files: 0,
                cleared_history_records: 0,
                freed_bytes: 0,
            }
        );
        assert!(shared_audio_path.exists());
        let records = read_records_in_file_order(&history_path).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| record.audio_path.is_some()));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn cleanup_missing_audio_is_idempotent_and_clears_history_once() {
        let base = temp_dir("cleanup-missing-audio-idempotent");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let missing_audio_path = recordings_dir.join("missing.wav");
        let mut old_record = sample_record("missing-audio", None);
        old_record.audio_finished_at = utc("2026-07-01T12:00:00Z");
        old_record.audio_path = Some(missing_audio_path);
        write_history_lines(
            &history_path,
            &[serde_json::to_string(&old_record).unwrap()],
        );

        let first = cleanup_recordings_before(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            utc("2026-07-09T12:00:00Z"),
        )
        .unwrap();
        let second = cleanup_recordings_before(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            utc("2026-07-09T12:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            first,
            RecordingCleanupResult {
                deleted_files: 0,
                cleared_history_records: 1,
                freed_bytes: 0,
            }
        );
        assert_eq!(
            second,
            RecordingCleanupResult {
                deleted_files: 0,
                cleared_history_records: 0,
                freed_bytes: 0,
            }
        );
        let records = read_records_in_file_order(&history_path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "missing-audio");
        assert!(records[0].audio_path.is_none());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn cleanup_preview_matches_cleanup_and_counts_recording_directory_usage() {
        let base = temp_dir("cleanup-preview");
        let history_path = base.join("history.jsonl");
        let recordings_dir = base.join("recordings");
        fs::create_dir_all(&recordings_dir).unwrap();

        let old_audio_path = recordings_dir.join("old.wav");
        let recent_audio_path = recordings_dir.join("recent.wav");
        let orphan_audio_path =
            recordings_dir.join("1780000000_00000000-0000-4000-8000-000000000003.wav");
        fs::write(&old_audio_path, b"old").unwrap();
        fs::write(&recent_audio_path, b"recent").unwrap();
        fs::write(&orphan_audio_path, b"orphan!").unwrap();
        fs::write(recordings_dir.join("ignored.txt"), b"ignored").unwrap();

        let cutoff = Utc::now() + ChronoDuration::seconds(1);
        let mut old_record = sample_record("old", None);
        old_record.audio_finished_at = cutoff - ChronoDuration::days(2);
        old_record.audio_path = Some(old_audio_path.clone());
        let mut recent_record = sample_record("recent", None);
        recent_record.audio_finished_at = cutoff + ChronoDuration::days(2);
        recent_record.audio_path = Some(recent_audio_path.clone());
        write_history_lines(
            &history_path,
            &[
                serde_json::to_string(&old_record).unwrap(),
                serde_json::to_string(&recent_record).unwrap(),
            ],
        );

        let preview = preview_recording_cleanup_before(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            cutoff,
        )
        .unwrap();
        assert_eq!(
            preview,
            RecordingCleanupPreview {
                recording_files: 3,
                recording_bytes: 16,
                eligible_files: 2,
                eligible_bytes: 10,
            }
        );

        let cleanup = cleanup_recordings_before(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            cutoff,
        )
        .unwrap();
        assert_eq!(cleanup.freed_bytes, preview.eligible_bytes);
        assert_eq!(cleanup.deleted_files, preview.eligible_files);

        let after = preview_recording_cleanup_before(
            std::slice::from_ref(&history_path),
            std::slice::from_ref(&recordings_dir),
            cutoff,
        )
        .unwrap();
        assert_eq!(after.recording_files, 1);
        assert_eq!(after.recording_bytes, 6);
        assert_eq!(after.eligible_files, 0);
        assert_eq!(after.eligible_bytes, 0);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn recording_cleanup_cutoff_supports_days_weeks_and_thirty_day_months() {
        let now = utc("2026-03-31T12:34:56Z");

        assert_eq!(
            recording_cleanup_cutoff(now, 3, RecordingCleanupUnit::Day).unwrap(),
            utc("2026-03-28T12:34:56Z")
        );
        assert_eq!(
            recording_cleanup_cutoff(now, 2, RecordingCleanupUnit::Week).unwrap(),
            utc("2026-03-17T12:34:56Z")
        );
        assert_eq!(
            recording_cleanup_cutoff(now, 1, RecordingCleanupUnit::Month).unwrap(),
            utc("2026-03-01T12:34:56Z")
        );

        for unit in [
            RecordingCleanupUnit::Day,
            RecordingCleanupUnit::Week,
            RecordingCleanupUnit::Month,
        ] {
            assert!(recording_cleanup_cutoff(now, 0, unit).is_err());
        }
    }
}
