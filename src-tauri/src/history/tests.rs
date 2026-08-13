use super::mutation::*;
use super::read::*;
use super::recordings::*;
use super::retention::*;
use super::stats::*;
use super::storage::*;
use super::*;
use anyhow::bail;
use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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

fn append_to_paths(
    record: &HistoryRecord,
    retention: &RetentionConfig,
    history_path: &Path,
    stats_path: &Path,
    recordings_dir: &Path,
) -> anyhow::Result<()> {
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

    let stats = load_stats_from_sources(&stats_path, std::slice::from_ref(&history_path)).unwrap();
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
    let stats = load_stats_from_sources(&stats_path, std::slice::from_ref(&history_path)).unwrap();
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
    let modified = DateTime::<Utc>::from(fs::metadata(&orphan_audio).unwrap().modified().unwrap());
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
