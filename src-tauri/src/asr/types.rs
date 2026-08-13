use crate::config::AsrConfig;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LiveAsrDiagnostics {
    pub connection_attempts: u32,
    pub first_connected_after_ms: Option<u64>,
    pub peak_buffered_bytes: u64,
    pub last_error_category: Option<String>,
    pub fallback_reason: Option<String>,
}

#[derive(Debug)]
pub struct LiveAsrResult {
    pub output: Result<TranscriptionOutcome>,
    pub diagnostics: LiveAsrDiagnostics,
}

pub trait AsrProvider {
    fn transcribe(&self, audio_path: &Path, config: &AsrConfig) -> Result<TranscriptionOutcome>;
}

#[derive(Debug, Clone)]
pub struct AsrOutput {
    pub text: String,
    pub provider: String,
    pub task_id: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum TranscriptionOutcome {
    Text(AsrOutput),
    NoSpeech,
}

#[derive(Clone)]
pub struct LiveAsrActivity {
    started_at: Instant,
    last_progress_ms: Arc<AtomicU64>,
}

impl LiveAsrActivity {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            last_progress_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn last_progress_ms(&self) -> Option<u64> {
        match self.last_progress_ms.load(Ordering::Acquire) {
            0 => None,
            value => Some(value),
        }
    }

    pub(super) fn note_progress(&self) {
        let elapsed = self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.last_progress_ms
            .store(elapsed.max(1), Ordering::Release);
    }
}
