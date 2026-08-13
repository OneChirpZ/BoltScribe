use super::model::{RecordingCleanupPreview, RecordingCleanupResult, RecordingCleanupUnit};
use super::recordings::{
    audio_path_from_value, delete_recording_paths, is_app_managed_recording_path,
    normalized_recordings_directory, normalized_safe_recording_path, regular_recording_metadata,
};
use super::storage::{
    load_mutable_history_files, write_mutable_history_files_atomically, MutableHistoryFile,
};
use crate::config::RetentionConfig;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

const MAX_RECORDING_CLEANUP_DAYS: u32 = 36_500;
const MAX_RECORDING_CLEANUP_WEEKS: u32 = 5_200;
const MAX_RECORDING_CLEANUP_MONTHS: u32 = 1_200;
pub(super) const ORPHAN_RECORDING_GRACE_HOURS: i64 = 24;

pub(super) fn recording_cleanup_cutoff(
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

#[derive(Clone)]
struct HistoryRecordLocation {
    file_index: usize,
    line_index: usize,
    created_at: DateTime<Utc>,
    audio_path: Option<PathBuf>,
}

pub(super) fn prune_paths(
    history_paths: &[PathBuf],
    retention: &RetentionConfig,
    recordings_dirs: &[PathBuf],
) -> Result<()> {
    prune_paths_at(history_paths, retention, recordings_dirs, Utc::now())
}

pub(super) fn prune_paths_at(
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

pub(super) fn cleanup_recordings_before(
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

pub(super) fn preview_recording_cleanup_before(
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
