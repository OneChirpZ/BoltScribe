use crate::asr::LiveAsrDiagnostics;
use crate::corrector::LlmCallLog;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
