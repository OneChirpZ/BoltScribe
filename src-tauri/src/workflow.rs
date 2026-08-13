use crate::asr::{
    AsrOutput, AsrProvider, LiveAsrDiagnostics, TranscriptionOutcome, VolcengineFileAsr,
    VolcengineLiveAsrSession,
};
use crate::config::{AppConfig, ConfigStore, CorrectionConfig, LlmConfig, RaceModelTarget};
use crate::corrector::{LlmCallLog, LlmProvider, OpenAiCompatibleCorrector};
use crate::history::{HistoryRecord, HistoryStore};
use crate::injector;
use crate::output_volume::{self, OutputVolumeDuckingSession};
use crate::paths;
use crate::recorder::{AudioLevelMeter, PendingRecordingStop, RecordedAudio, RecorderController};
use crate::vad::{VadGate, VadMonitorHandle, VadPhase};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

mod correction;
mod processing;
mod publish;
mod recording;
mod state;
mod status;

#[cfg(test)]
mod tests;

pub use crate::workflow_status::{WorkflowMode, WorkflowStage, WorkflowStatus, WorkflowStatusCode};
pub use processing::retry_history_record;
pub use recording::{cancel_current_workflow, toggle_recording, toggle_recording_from_app};
pub use state::AppState;
