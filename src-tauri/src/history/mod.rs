mod model;
mod mutation;
mod read;
mod recordings;
mod retention;
mod stats;
mod storage;

pub use model::*;

use crate::config::RetentionConfig;
use crate::paths;
use anyhow::Result;
use chrono::Utc;
use mutation::{
    append_history_record, delete_from_paths, history_mutation_guard, load_retryable_from_paths,
    replace_in_paths,
};
use read::{history_read_paths, load_from_paths};
use recordings::history_recordings_dirs;
use retention::{
    cleanup_recordings_before, preview_recording_cleanup_before, prune_paths,
    recording_cleanup_cutoff,
};
use stats::{append_stats_for_record, backfill_stats_from_history, load_stats_from_sources};

pub fn is_empty_asr_text_error(message: &str) -> bool {
    read::is_empty_asr_text_error_impl(message)
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

    pub fn find(record_id: &str) -> Result<Option<HistoryRecord>> {
        read::find_from_paths(&history_read_paths()?, record_id)
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

#[cfg(test)]
mod tests;
