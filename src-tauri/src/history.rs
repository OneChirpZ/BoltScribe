use crate::config::RetentionConfig;
use crate::corrector::LlmCallLog;
use crate::paths;
use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const HISTORY_READ_CHUNK_SIZE: u64 = 16 * 1024;
const STATS_SCHEMA_VERSION: u8 = 1;

struct HistoryPage {
    records: Vec<HistoryRecord>,
    visible_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub audio_path: PathBuf,
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
    pub total_duration_ms: u128,
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
        let path = paths::history_path()?;
        let stats_path = paths::input_stats_path()?;
        let recordings_dir = paths::recordings_dir()?;
        append_to_paths(record, retention, &path, &stats_path, &recordings_dir)
    }

    pub fn load(limit: usize, offset: usize) -> Result<Vec<HistoryRecord>> {
        load_from_paths(&history_read_paths()?, limit, offset)
    }

    pub fn stats() -> Result<InputStats> {
        let stats_path = paths::input_stats_path()?;
        let history_paths = history_read_paths()?;
        backfill_stats_from_history(&stats_path, &history_paths)?;
        load_stats_from_sources(&stats_path, &history_paths)
    }

    pub fn prune(retention: &RetentionConfig) -> Result<()> {
        let path = paths::history_path()?;
        let recordings_dir = paths::recordings_dir()?;
        prune_path(&path, retention, &recordings_dir)
    }
}

fn append_to_paths(
    record: &HistoryRecord,
    retention: &RetentionConfig,
    history_path: &Path,
    stats_path: &Path,
    recordings_dir: &Path,
) -> Result<()> {
    append_history_record(history_path, record)?;
    append_stats_for_record(stats_path, record)?;
    prune_path(history_path, retention, recordings_dir)
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

fn prune_path(path: &Path, retention: &RetentionConfig, recordings_dir: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let mut records = read_records_in_file_order(path)?;
    records.sort_by_key(|record| record.created_at);

    let mut keep_start = records.len().saturating_sub(retention.max_history_records);
    while keep_start < records.len()
        && storage_bytes(&records[keep_start..], recordings_dir)? > retention.max_storage_bytes
    {
        keep_start += 1;
    }

    if keep_start == 0 {
        return Ok(());
    }

    let removed = records[..keep_start].to_vec();
    let kept = records[keep_start..].to_vec();
    write_records_atomically(path, &kept)?;
    delete_recording_files(&removed, recordings_dir)?;
    Ok(())
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

fn write_records_atomically(path: &Path, records: &[HistoryRecord]) -> Result<()> {
    let temp_path = path.with_extension("jsonl.tmp");
    {
        let mut file = fs::File::create(&temp_path)
            .with_context(|| format!("Failed to create {}", temp_path.display()))?;
        for record in records {
            writeln!(file, "{}", serde_json::to_string(record)?)?;
        }
        file.sync_all()?;
    }
    fs::rename(&temp_path, path)
        .with_context(|| format!("Failed to replace {}", path.display()))?;
    Ok(())
}

fn storage_bytes(records: &[HistoryRecord], recordings_dir: &Path) -> Result<u64> {
    let mut total = 0u64;
    let mut seen = HashSet::new();
    for record in records {
        if !seen.insert(record.audio_path.clone()) {
            continue;
        }
        if is_safe_recording_path(&record.audio_path, recordings_dir)? && record.audio_path.exists()
        {
            total += record.audio_path.metadata()?.len();
        }
    }
    Ok(total)
}

fn delete_recording_files(records: &[HistoryRecord], recordings_dir: &Path) -> Result<()> {
    let mut seen = HashSet::new();
    for record in records {
        if !seen.insert(record.audio_path.clone()) {
            continue;
        }
        if !is_safe_recording_path(&record.audio_path, recordings_dir)?
            || !record.audio_path.exists()
        {
            continue;
        }
        fs::remove_file(&record.audio_path)
            .with_context(|| format!("Failed to remove {}", record.audio_path.display()))?;
    }
    Ok(())
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

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let recordings_dir = recordings_dir
        .canonicalize()
        .unwrap_or_else(|_| recordings_dir.to_path_buf());
    let parent = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    Ok(parent == recordings_dir)
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
            audio_path: PathBuf::from("/tmp/a.wav"),
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

    #[test]
    fn history_record_serializes() {
        let record = sample_record("1", None);
        assert!(serde_json::to_string(&record)
            .unwrap()
            .contains("corrected"));
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
    fn append_stats_failure_skips_history_prune() {
        let base = temp_dir("stats-failure-skips-prune");
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

        assert!(result.is_err());
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn delete_recording_files_only_removes_safe_recordings() {
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

        let records = [
            HistoryRecord {
                audio_path: safe_path.clone(),
                ..sample_record("safe", None)
            },
            HistoryRecord {
                audio_path: unsafe_path.clone(),
                ..sample_record("unsafe", None)
            },
        ];

        delete_recording_files(&records, &recordings_dir).unwrap();
        assert!(!safe_path.exists());
        assert!(unsafe_path.exists());
        let _ = fs::remove_dir_all(base);
    }
}
