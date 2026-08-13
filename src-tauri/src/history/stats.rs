use super::model::{DailyInputStats, HistoryRecord, InputStats};
use super::read::{read_records_in_file_order, should_hide_history_record};
use super::storage::{
    load_mutable_history_files, merge_json_fields, MutableHistoryFile, MutableHistoryLine,
};
use anyhow::{anyhow, Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub(super) const STATS_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct InputStatsEvent {
    pub(super) schema_version: u8,
    pub(super) record_id: String,
    pub(super) date: String,
    pub(super) character_count: u64,
    pub(super) audio_duration_ms: u64,
}

#[cfg(test)]
pub(super) fn load_stats_from_paths(paths: &[PathBuf]) -> Result<InputStats> {
    let mut accumulator = InputStatsAccumulator::default();
    let mut seen_ids = HashSet::new();
    add_history_paths_to_stats(paths, &mut seen_ids, &mut accumulator)?;
    Ok(accumulator.finish())
}

pub(super) fn load_stats_from_sources(
    stats_path: &Path,
    history_paths: &[PathBuf],
) -> Result<InputStats> {
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

pub(super) fn append_stats_for_record(path: &Path, record: &HistoryRecord) -> Result<()> {
    let Some(event) = stats_event_from_record(record) else {
        return Ok(());
    };
    append_stats_events(path, std::slice::from_ref(&event))
}

pub(super) fn prepare_stats_file_for_record(
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

pub(super) fn backfill_stats_from_history(
    stats_path: &Path,
    history_paths: &[PathBuf],
) -> Result<()> {
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

pub(super) fn append_stats_events(path: &Path, events: &[InputStatsEvent]) -> Result<()> {
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

pub(super) fn read_stats_events_in_file_order(path: &Path) -> Result<Vec<InputStatsEvent>> {
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

pub(super) fn stats_event_from_record(record: &HistoryRecord) -> Option<InputStatsEvent> {
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

pub(super) fn count_chinese_characters_and_english_words(text: &str) -> usize {
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

pub(super) fn audio_duration_ms(record: &HistoryRecord) -> u64 {
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
