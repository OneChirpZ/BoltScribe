use super::detector::{gate_profile, WEBRTC_VAD_MODE};
use crate::asr::VolcengineLiveAsrSession;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VadPhase {
    Armed,
    Activated,
    TimedOut,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct VadSnapshot {
    pub phase: VadPhase,
    pub mode: u8,
    pub noise_margin_db: u32,
    pub confirmation_ms: u32,
    pub noise_window_ms: u32,
    pub raw_voice_active: bool,
    pub voice_active: bool,
    pub level: f32,
    pub noise_calibrated: bool,
    pub noise_floor: f32,
    pub trigger_threshold: f32,
    pub trigger_progress: f32,
    pub elapsed_ms: u64,
    pub remaining_ms: u64,
    pub first_voice_ms: Option<u64>,
    pub last_vad_activity_ms: Option<u64>,
    pub last_asr_activity_ms: Option<u64>,
    pub revision: u64,
    pub error: Option<String>,
}

impl VadSnapshot {
    pub(super) fn new(
        noise_margin_db: u32,
        confirmation_ms: u32,
        noise_window_ms: u32,
        timeout: Duration,
    ) -> Self {
        let profile = gate_profile(noise_margin_db, confirmation_ms, noise_window_ms);
        Self {
            phase: VadPhase::Armed,
            mode: WEBRTC_VAD_MODE,
            noise_margin_db: profile.noise_margin_db,
            confirmation_ms: profile.confirmation_ms,
            noise_window_ms: profile.noise_window_ms,
            raw_voice_active: false,
            voice_active: false,
            level: -96.0,
            noise_calibrated: false,
            noise_floor: -96.0,
            trigger_threshold: -96.0,
            trigger_progress: 0.0,
            elapsed_ms: 0,
            remaining_ms: timeout.as_millis().min(u64::MAX as u128) as u64,
            first_voice_ms: None,
            last_vad_activity_ms: None,
            last_asr_activity_ms: None,
            revision: 1,
            error: None,
        }
    }
}

pub struct GatedAsrResult {
    pub activated: bool,
    pub live_asr: Option<VolcengineLiveAsrSession>,
    pub live_asr_start_error: Option<String>,
}
