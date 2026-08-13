use crate::config::{
    VAD_CONFIRMATION_STEP_MS, VAD_CONTINUOUS_SPEECH_MS, VAD_MAX_CONFIRMATION_MS,
    VAD_MAX_NOISE_MARGIN_DB, VAD_MAX_NOISE_WINDOW_MS, VAD_MIN_CONFIRMATION_MS,
    VAD_MIN_NOISE_MARGIN_DB, VAD_MIN_NOISE_WINDOW_MS, VAD_NOISE_WINDOW_STEP_MS,
};
use std::collections::VecDeque;
use webrtc_vad::VadMode;

const TARGET_SAMPLE_RATE: u32 = 16_000;
pub(super) const FRAME_MS: u64 = 20;
pub(super) const FRAME_SAMPLES: usize = TARGET_SAMPLE_RATE as usize / 1000 * FRAME_MS as usize;
pub(super) const WEBRTC_VAD_MODE: u8 = 3;
const NOISE_BOOTSTRAP_MIN_NON_VOICE_FRAMES: usize = 12;
const NOISE_BOOTSTRAP_NON_VOICE_NUMERATOR: usize = 2;
const NOISE_BOOTSTRAP_NON_VOICE_DENOMINATOR: usize = 5;
const NOISE_STATIONARY_RANGE_DB: f32 = 4.0;
const NOISE_STATIONARY_DELTA_DB: f32 = 0.75;
const SPEECH_BOOTSTRAP_RANGE_DB: f32 = 6.0;
const SPEECH_BOOTSTRAP_DELTA_DB: f32 = 1.0;
const SPEECH_BOOTSTRAP_MIN_MS: u64 = 1_200;
const SPEECH_BOOTSTRAP_MIN_FRAMES: usize = (SPEECH_BOOTSTRAP_MIN_MS / FRAME_MS) as usize;
const SPEECH_CONFIRMATION_RANGE_DB: f32 = 4.0;
const SPEECH_CONFIRMATION_DELTA_DB: f32 = 0.8;
const NOISE_UPDATE_HEADROOM_DB: f32 = 3.0;
const DIGITAL_SILENCE_DBFS: f32 = -95.0;
const NON_DIGITAL_SIGNAL_DBFS: f32 = -90.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct GateProfile {
    pub(super) noise_margin_db: u32,
    pub(super) confirmation_ms: u32,
    pub(super) noise_window_ms: u32,
    pub(super) noise_window_frames: usize,
    pub(super) window_frames: usize,
    pub(super) required_voice_frames: usize,
    pub(super) required_consecutive_frames: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SpeechFrameDecision {
    pub(super) qualified_voice: bool,
    pub(super) activated: bool,
    pub(super) noise_calibrated: bool,
    pub(super) noise_floor_dbfs: f32,
    pub(super) trigger_threshold_dbfs: f32,
    pub(super) trigger_progress: f32,
    pub(super) first_qualified_ms: Option<u64>,
}

#[derive(Debug)]
pub(super) struct AdaptiveNoiseFloor {
    pub(super) samples: VecDeque<f32>,
    bootstrap: VecDeque<(f32, bool, u64)>,
    pub(super) window_frames: usize,
    pub(super) floor_dbfs: f32,
    calibrated: bool,
}

impl AdaptiveNoiseFloor {
    fn new(window_frames: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(window_frames),
            bootstrap: VecDeque::with_capacity(window_frames),
            window_frames,
            floor_dbfs: -96.0,
            calibrated: false,
        }
    }

    fn reset_window(&mut self, window_frames: usize) {
        self.samples.clear();
        self.bootstrap.clear();
        self.window_frames = window_frames;
        self.floor_dbfs = -96.0;
        self.calibrated = false;
    }

    pub(super) fn calibrated(&self) -> bool {
        self.calibrated
    }

    fn observe(
        &mut self,
        level_dbfs: f32,
        raw_voice: bool,
        now_ms: u64,
        snr_margin_db: f32,
    ) -> bool {
        let was_calibrated = self.calibrated();
        let level_dbfs = level_dbfs.clamp(-96.0, 0.0);
        if self.calibrated()
            && self.floor_dbfs <= DIGITAL_SILENCE_DBFS
            && level_dbfs > NON_DIGITAL_SIGNAL_DBFS
        {
            self.samples.clear();
            self.bootstrap.clear();
            self.calibrated = false;
        }
        self.bootstrap.push_back((level_dbfs, raw_voice, now_ms));
        while self.bootstrap.len() > self.window_frames {
            self.bootstrap.pop_front();
        }

        if !self.calibrated() {
            let minimum_non_voice_frames = NOISE_BOOTSTRAP_MIN_NON_VOICE_FRAMES.max(
                self.window_frames
                    .saturating_mul(NOISE_BOOTSTRAP_NON_VOICE_NUMERATOR)
                    .saturating_add(NOISE_BOOTSTRAP_NON_VOICE_DENOMINATOR - 1)
                    / NOISE_BOOTSTRAP_NON_VOICE_DENOMINATOR,
            );
            if self.bootstrap.len() < minimum_non_voice_frames {
                return false;
            }
            if let Some(estimate) =
                initial_noise_estimate(&self.bootstrap, minimum_non_voice_frames)
            {
                self.seed(estimate);
            }
            return !was_calibrated && self.calibrated();
        }

        let threshold = self.trigger_threshold(snr_margin_db);
        if !raw_voice || level_dbfs <= threshold.min(self.floor_dbfs + NOISE_UPDATE_HEADROOM_DB) {
            self.push_sample(level_dbfs);
        }
        false
    }

    fn seed(&mut self, floor_dbfs: f32) {
        self.samples.clear();
        for _ in 0..NOISE_BOOTSTRAP_MIN_NON_VOICE_FRAMES {
            self.samples.push_back(floor_dbfs.clamp(-96.0, 0.0));
        }
        self.floor_dbfs = floor_dbfs.clamp(-96.0, 0.0);
        self.calibrated = true;
    }

    fn push_sample(&mut self, level_dbfs: f32) {
        self.samples.push_back(level_dbfs);
        while self.samples.len() > self.window_frames {
            self.samples.pop_front();
        }
        self.floor_dbfs = percentile(self.samples.iter().copied(), 0.2).unwrap_or(-96.0);
    }

    pub(super) fn trigger_threshold(&self, snr_margin_db: f32) -> f32 {
        (self.floor_dbfs + snr_margin_db).clamp(-96.0, -1.0)
    }

    fn provisional_non_voice_floor(&self) -> Option<f32> {
        let non_voice = self
            .bootstrap
            .iter()
            .filter_map(|(level, raw_voice, _)| (!raw_voice).then_some(*level))
            .collect::<Vec<_>>();
        (non_voice.len() >= NOISE_BOOTSTRAP_MIN_NON_VOICE_FRAMES)
            .then(|| percentile(non_voice, 0.2))
            .flatten()
    }
}

#[derive(Debug)]
pub(super) struct SpeechGate {
    pub(super) profile: GateProfile,
    pub(super) noise: AdaptiveNoiseFloor,
    startup_voice: VecDeque<(bool, f32, u64)>,
    startup_first_voice_ms: Option<u64>,
    qualified_window: VecDeque<(bool, u64, f32)>,
    consecutive_voice_frames: usize,
}

impl SpeechGate {
    pub(super) fn new(noise_margin_db: u32, confirmation_ms: u32, noise_window_ms: u32) -> Self {
        let profile = gate_profile(noise_margin_db, confirmation_ms, noise_window_ms);
        Self {
            noise: AdaptiveNoiseFloor::new(profile.noise_window_frames),
            profile,
            startup_voice: VecDeque::with_capacity(SPEECH_BOOTSTRAP_MIN_FRAMES),
            startup_first_voice_ms: None,
            qualified_window: VecDeque::new(),
            consecutive_voice_frames: 0,
        }
    }

    pub(super) fn update_settings(
        &mut self,
        noise_margin_db: u32,
        confirmation_ms: u32,
        noise_window_ms: u32,
    ) {
        let profile = gate_profile(noise_margin_db, confirmation_ms, noise_window_ms);
        if profile.noise_window_frames != self.profile.noise_window_frames {
            self.noise.reset_window(profile.noise_window_frames);
        }
        self.profile = profile;
        self.startup_voice.clear();
        self.startup_first_voice_ms = None;
        self.qualified_window.clear();
        self.consecutive_voice_frames = 0;
    }

    pub(super) fn process(
        &mut self,
        raw_voice: bool,
        level_dbfs: f32,
        now_ms: u64,
    ) -> SpeechFrameDecision {
        self.startup_voice
            .push_back((raw_voice, level_dbfs, now_ms));
        while self.startup_voice.len() > SPEECH_BOOTSTRAP_MIN_FRAMES {
            self.startup_voice.pop_front();
        }
        let calibrated_from_noise = self.noise.observe(
            level_dbfs,
            raw_voice,
            now_ms,
            self.profile.noise_margin_db as f32,
        );
        let mut replayed_startup = false;
        if calibrated_from_noise {
            self.startup_first_voice_ms = None;
        } else if !self.noise.calibrated() {
            if let Some((floor_dbfs, first_voice_ms)) = provisional_speech_estimate(
                &self.startup_voice,
                self.profile.noise_margin_db as f32,
                self.noise.provisional_non_voice_floor(),
            ) {
                self.noise.seed(floor_dbfs);
                self.startup_first_voice_ms = Some(first_voice_ms);
                replayed_startup = true;
            }
        }
        let noise_calibrated = self.noise.calibrated();
        let trigger_threshold_dbfs = self
            .noise
            .trigger_threshold(self.profile.noise_margin_db as f32);
        let qualified_voice = noise_calibrated && raw_voice && level_dbfs >= trigger_threshold_dbfs;

        if replayed_startup {
            self.replay_startup_voice(trigger_threshold_dbfs);
        } else {
            self.push_qualified_frame(qualified_voice, now_ms, level_dbfs);
        }

        let qualified_frames = self
            .qualified_window
            .iter()
            .filter(|(qualified, _, _)| *qualified)
            .count();
        if qualified_frames == 0 {
            self.startup_first_voice_ms = None;
        }
        let qualified_levels = self
            .qualified_window
            .iter()
            .filter_map(|(qualified, _, level)| qualified.then_some(*level))
            .collect::<Vec<_>>();
        let level_span_db = percentile(qualified_levels.iter().copied(), 0.8)
            .zip(percentile(qualified_levels.iter().copied(), 0.2))
            .map(|(upper, lower)| upper - lower)
            .unwrap_or(0.0);
        let level_deltas = qualified_levels
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .collect::<Vec<_>>();
        let median_level_delta_db = percentile(level_deltas, 0.5).unwrap_or(0.0);
        let accumulated_progress =
            qualified_frames as f32 / self.profile.required_voice_frames.max(1) as f32;
        let consecutive_progress = self.consecutive_voice_frames as f32
            / self.profile.required_consecutive_frames.max(1) as f32;
        let dynamics_progress = level_span_db / SPEECH_CONFIRMATION_RANGE_DB;
        let modulation_progress = median_level_delta_db / SPEECH_CONFIRMATION_DELTA_DB;
        let trigger_progress = accumulated_progress
            .min(consecutive_progress)
            .min(dynamics_progress)
            .min(modulation_progress)
            .clamp(0.0, 1.0);
        let activated = qualified_frames >= self.profile.required_voice_frames
            && self.consecutive_voice_frames >= self.profile.required_consecutive_frames
            && level_span_db >= SPEECH_CONFIRMATION_RANGE_DB
            && median_level_delta_db >= SPEECH_CONFIRMATION_DELTA_DB;
        let first_qualified_ms = activated.then(|| {
            self.startup_first_voice_ms.unwrap_or_else(|| {
                self.qualified_window
                    .iter()
                    .find_map(|(qualified, at_ms, _)| qualified.then_some(*at_ms))
                    .map(|at_ms| at_ms.saturating_sub(self.profile.noise_window_ms as u64))
                    .unwrap_or(now_ms)
            })
        });

        SpeechFrameDecision {
            qualified_voice,
            activated,
            noise_calibrated,
            noise_floor_dbfs: self.noise.floor_dbfs,
            trigger_threshold_dbfs,
            trigger_progress,
            first_qualified_ms,
        }
    }

    fn replay_startup_voice(&mut self, trigger_threshold_dbfs: f32) {
        self.qualified_window.clear();
        self.consecutive_voice_frames = 0;
        let frames = self.startup_voice.iter().copied().collect::<Vec<_>>();
        for (raw_voice, level_dbfs, now_ms) in frames {
            self.push_qualified_frame(
                raw_voice && level_dbfs >= trigger_threshold_dbfs,
                now_ms,
                level_dbfs,
            );
        }
    }

    fn push_qualified_frame(&mut self, qualified_voice: bool, now_ms: u64, level_dbfs: f32) {
        self.qualified_window
            .push_back((qualified_voice, now_ms, level_dbfs));
        while self.qualified_window.len() > self.profile.window_frames {
            self.qualified_window.pop_front();
        }
        if qualified_voice {
            self.consecutive_voice_frames = self.consecutive_voice_frames.saturating_add(1);
        } else {
            self.consecutive_voice_frames = 0;
        }
    }
}

pub(super) fn gate_profile(
    noise_margin_db: u32,
    confirmation_ms: u32,
    noise_window_ms: u32,
) -> GateProfile {
    let noise_margin_db = noise_margin_db.clamp(VAD_MIN_NOISE_MARGIN_DB, VAD_MAX_NOISE_MARGIN_DB);
    let confirmation_ms = confirmation_ms.clamp(VAD_MIN_CONFIRMATION_MS, VAD_MAX_CONFIRMATION_MS);
    let confirmation_ms = ((confirmation_ms + VAD_CONFIRMATION_STEP_MS / 2)
        / VAD_CONFIRMATION_STEP_MS)
        * VAD_CONFIRMATION_STEP_MS;
    let noise_window_ms = noise_window_ms.clamp(VAD_MIN_NOISE_WINDOW_MS, VAD_MAX_NOISE_WINDOW_MS);
    let noise_window_ms = ((noise_window_ms + VAD_NOISE_WINDOW_STEP_MS / 2)
        / VAD_NOISE_WINDOW_STEP_MS)
        * VAD_NOISE_WINDOW_STEP_MS;
    let required_voice_frames = (confirmation_ms / FRAME_MS as u32) as usize;
    GateProfile {
        noise_margin_db,
        confirmation_ms,
        noise_window_ms,
        noise_window_frames: (noise_window_ms / FRAME_MS as u32) as usize,
        window_frames: required_voice_frames.saturating_mul(2),
        required_voice_frames,
        required_consecutive_frames: (VAD_CONTINUOUS_SPEECH_MS / FRAME_MS as u32) as usize,
    }
}

pub(super) fn fixed_vad_mode() -> VadMode {
    VadMode::VeryAggressive
}

fn initial_noise_estimate(
    frames: &VecDeque<(f32, bool, u64)>,
    minimum_non_voice_frames: usize,
) -> Option<f32> {
    let non_voice: Vec<f32> = frames
        .iter()
        .filter_map(|(level, raw_voice, _)| (!raw_voice).then_some(*level))
        .collect();
    if non_voice.len() >= minimum_non_voice_frames {
        return percentile(non_voice, 0.2);
    }

    let levels: Vec<f32> = frames.iter().map(|(level, _, _)| *level).collect();
    let lower = percentile(levels.iter().copied(), 0.2)?;
    let upper = percentile(levels.iter().copied(), 0.8)?;
    let median = percentile(levels.iter().copied(), 0.5)?;
    let deltas: Vec<f32> = levels
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .collect();
    let median_delta = percentile(deltas, 0.5).unwrap_or(0.0);

    let voice_frames = frames.iter().filter(|(_, raw_voice, _)| *raw_voice).count();
    if voice_frames * 2 < frames.len()
        && upper - lower <= NOISE_STATIONARY_RANGE_DB
        && median_delta <= NOISE_STATIONARY_DELTA_DB
    {
        return Some(median);
    }

    None
}

fn provisional_speech_estimate(
    frames: &VecDeque<(bool, f32, u64)>,
    snr_margin_db: f32,
    non_voice_floor_ceiling: Option<f32>,
) -> Option<(f32, u64)> {
    if frames.len() < SPEECH_BOOTSTRAP_MIN_FRAMES
        || frames.iter().any(|(raw_voice, _, _)| !raw_voice)
    {
        return None;
    }

    let levels = frames
        .iter()
        .map(|(_, level, _)| *level)
        .collect::<Vec<_>>();
    let lower = percentile(levels.iter().copied(), 0.2)?;
    let upper = percentile(levels.iter().copied(), 0.8)?;
    let deltas = levels
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .collect::<Vec<_>>();
    let median_delta = percentile(deltas, 0.5).unwrap_or(0.0);
    if upper - lower < SPEECH_BOOTSTRAP_RANGE_DB || median_delta < SPEECH_BOOTSTRAP_DELTA_DB {
        return None;
    }

    let lower_envelope = percentile(levels, 0.1)?;
    let provisional_headroom = snr_margin_db.max(1.0);
    let speech_floor = (lower_envelope - provisional_headroom).clamp(-96.0, 0.0);
    let floor_dbfs = non_voice_floor_ceiling
        .map(|ceiling| speech_floor.min(ceiling))
        .unwrap_or(speech_floor);
    Some((floor_dbfs, frames.front()?.2))
}

fn percentile(values: impl IntoIterator<Item = f32>, percentile: f32) -> Option<f32> {
    let mut sorted: Vec<f32> = values
        .into_iter()
        .filter(|value| value.is_finite())
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f32::total_cmp);
    let index = ((sorted.len() - 1) as f32 * percentile.clamp(0.0, 1.0)).round() as usize;
    sorted.get(index).copied()
}

pub(super) fn chunk_level_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return -96.0;
    }
    let mean_square = samples
        .iter()
        .map(|sample| {
            let value = *sample as f64 / i16::MAX as f64;
            value * value
        })
        .sum::<f64>()
        / samples.len() as f64;
    if mean_square <= f64::EPSILON {
        -96.0
    } else {
        (20.0 * mean_square.sqrt().log10()).clamp(-96.0, 0.0) as f32
    }
}
