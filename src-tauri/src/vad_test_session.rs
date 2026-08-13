use crate::config::AudioConfig;
use crate::recorder::RecorderController;
use crate::vad::{VadGate, VadPhase, VadSnapshot};
use anyhow::Result;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const TEST_LIMIT: Duration = Duration::from_secs(60);
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
pub struct VadTestStatus {
    pub mode: String,
    pub raw_voice_active: bool,
    pub voice_active: bool,
    pub level: f32,
    pub noise_calibrated: bool,
    pub noise_floor: f32,
    pub trigger_threshold: f32,
    pub trigger_progress: f32,
    pub elapsed_ms: u64,
    pub remaining_ms: u64,
    pub noise_margin_db: u32,
    pub confirmation_ms: u32,
    pub noise_window_ms: u32,
    pub revision: u64,
    pub error: Option<String>,
}

impl Default for VadTestStatus {
    fn default() -> Self {
        Self {
            mode: "idle".to_string(),
            raw_voice_active: false,
            voice_active: false,
            level: -96.0,
            noise_calibrated: false,
            noise_floor: -96.0,
            trigger_threshold: -96.0,
            trigger_progress: 0.0,
            elapsed_ms: 0,
            remaining_ms: TEST_LIMIT.as_millis() as u64,
            noise_margin_db: 12,
            confirmation_ms: 480,
            noise_window_ms: 2_000,
            revision: 0,
            error: None,
        }
    }
}

pub(crate) struct VadTestSession {
    gate: VadGate,
    started_at: Instant,
    revision_base: u64,
}

impl VadTestSession {
    pub(crate) fn start(recorder: &RecorderController, audio_config: AudioConfig) -> Result<Self> {
        let gate = VadGate::start_test(audio_config.voice_activity_detection.clone())?;
        if let Err(err) = recorder.start_with_config(Some(gate.audio_sender()), None, audio_config)
        {
            let _ = gate.finish(true);
            return Err(err);
        }
        Ok(Self {
            gate,
            started_at: Instant::now(),
            revision_base: NEXT_REVISION.fetch_add(1_000_000, Ordering::Relaxed),
        })
    }

    pub(crate) fn status(&self) -> VadTestStatus {
        let snapshot = self.gate.snapshot();
        let mut status = status_from_snapshot(snapshot, self.started_at.elapsed());
        status.revision = self.revision_base.saturating_add(status.revision);
        status
    }

    pub(crate) fn update_settings(
        &self,
        noise_margin_db: u32,
        confirmation_ms: u32,
        noise_window_ms: u32,
    ) -> Result<()> {
        self.gate
            .update_settings(noise_margin_db, confirmation_ms, noise_window_ms)
    }

    pub(crate) fn should_expire(&self) -> bool {
        self.started_at.elapsed() >= TEST_LIMIT
    }

    pub(crate) fn stop(self, recorder: &RecorderController) -> VadTestStatus {
        let _ = recorder.cancel();
        let status = self.status();
        let _ = self.gate.finish(true);
        status
    }
}

pub(crate) fn idle_status() -> VadTestStatus {
    VadTestStatus {
        revision: NEXT_REVISION.fetch_add(1_000_000, Ordering::Relaxed),
        ..VadTestStatus::default()
    }
}

fn status_from_snapshot(snapshot: VadSnapshot, elapsed: Duration) -> VadTestStatus {
    let mode = match snapshot.phase {
        VadPhase::Armed => "listening",
        VadPhase::Activated => "voice",
        VadPhase::TimedOut => "timed_out",
        VadPhase::Error => "error",
        VadPhase::Cancelled => "idle",
    };
    VadTestStatus {
        mode: mode.to_string(),
        raw_voice_active: snapshot.raw_voice_active,
        voice_active: snapshot.voice_active,
        level: snapshot.level,
        noise_calibrated: snapshot.noise_calibrated,
        noise_floor: snapshot.noise_floor,
        trigger_threshold: snapshot.trigger_threshold,
        trigger_progress: snapshot.trigger_progress,
        elapsed_ms: elapsed.as_millis().min(u64::MAX as u128) as u64,
        remaining_ms: TEST_LIMIT
            .as_millis()
            .saturating_sub(elapsed.as_millis())
            .min(u64::MAX as u128) as u64,
        noise_margin_db: snapshot.noise_margin_db,
        confirmation_ms: snapshot.confirmation_ms,
        noise_window_ms: snapshot.noise_window_ms,
        revision: snapshot.revision,
        error: snapshot.error,
    }
}
