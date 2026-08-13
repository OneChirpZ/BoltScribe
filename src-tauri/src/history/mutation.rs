use super::model::{DeleteHistoryResult, HistoryRecord};
use super::recordings::{
    audio_path_from_value, delete_recording_paths, normalized_retry_recording_path,
    normalized_safe_recording_path,
};
use super::stats::prepare_stats_file_for_record;
use super::storage::{
    load_mutable_history_files, merge_json_fields, write_mutable_history_files_atomically,
};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static HISTORY_MUTATION_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn history_mutation_guard() -> Result<MutexGuard<'static, ()>> {
    HISTORY_MUTATION_LOCK
        .lock()
        .map_err(|_| anyhow!("History storage lock is poisoned"))
}

pub(super) fn append_history_record(path: &Path, record: &HistoryRecord) -> Result<()> {
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

pub(super) fn load_retryable_from_paths(
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

pub(super) fn replace_in_paths(
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

pub(super) fn delete_from_paths(
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
