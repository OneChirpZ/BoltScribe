use crate::asr::{LiveAsrActivity, VolcengineLiveAsrSession};
use crate::config::{
    AsrConfig, VoiceActivityDetectionConfig, VAD_CONFIRMATION_STEP_MS, VAD_CONTINUOUS_SPEECH_MS,
    VAD_MAX_CONFIRMATION_MS, VAD_MAX_NOISE_MARGIN_DB, VAD_MAX_NOISE_WINDOW_MS,
    VAD_MIN_CONFIRMATION_MS, VAD_MIN_NOISE_MARGIN_DB, VAD_MIN_NOISE_WINDOW_MS,
    VAD_NOISE_WINDOW_STEP_MS,
};
use crate::recorder::{AudioChunk, AudioSink, AudioTrim};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use webrtc_vad::{SampleRate, Vad, VadMode};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const FRAME_MS: u64 = 20;
const FRAME_SAMPLES: usize = TARGET_SAMPLE_RATE as usize / 1000 * FRAME_MS as usize;
const WEBRTC_VAD_MODE: u8 = 3;
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
struct GateProfile {
    noise_margin_db: u32,
    confirmation_ms: u32,
    noise_window_ms: u32,
    noise_window_frames: usize,
    window_frames: usize,
    required_voice_frames: usize,
    required_consecutive_frames: usize,
}

#[derive(Debug, Clone, Copy)]
struct SpeechFrameDecision {
    qualified_voice: bool,
    activated: bool,
    noise_calibrated: bool,
    noise_floor_dbfs: f32,
    trigger_threshold_dbfs: f32,
    trigger_progress: f32,
    first_qualified_ms: Option<u64>,
}

struct PreActivationAudio {
    chunks: Option<VecDeque<AudioChunk>>,
}

impl PreActivationAudio {
    fn new() -> Self {
        Self {
            chunks: Some(VecDeque::new()),
        }
    }

    fn push(&mut self, chunk: &AudioChunk) {
        if let Some(chunks) = self.chunks.as_mut() {
            chunks.push_back(chunk.clone());
        }
    }

    fn take_for_activation(&mut self) -> VecDeque<AudioChunk> {
        self.chunks.take().unwrap_or_default()
    }
}

#[derive(Debug)]
struct AdaptiveNoiseFloor {
    samples: VecDeque<f32>,
    bootstrap: VecDeque<(f32, bool, u64)>,
    window_frames: usize,
    floor_dbfs: f32,
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

    fn calibrated(&self) -> bool {
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

    fn trigger_threshold(&self, snr_margin_db: f32) -> f32 {
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
struct SpeechGate {
    profile: GateProfile,
    noise: AdaptiveNoiseFloor,
    startup_voice: VecDeque<(bool, f32, u64)>,
    startup_first_voice_ms: Option<u64>,
    qualified_window: VecDeque<(bool, u64, f32)>,
    consecutive_voice_frames: usize,
}

impl SpeechGate {
    fn new(noise_margin_db: u32, confirmation_ms: u32, noise_window_ms: u32) -> Self {
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

    fn update_settings(
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

    fn process(&mut self, raw_voice: bool, level_dbfs: f32, now_ms: u64) -> SpeechFrameDecision {
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
    fn new(
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

enum GateCommand {
    Stop,
    Cancel,
    UpdateSettings {
        noise_margin_db: u32,
        confirmation_ms: u32,
        noise_window_ms: u32,
        applied: mpsc::SyncSender<()>,
    },
}

#[derive(Clone)]
pub struct VadMonitorHandle {
    snapshot: Arc<Mutex<VadSnapshot>>,
}

impl VadMonitorHandle {
    pub fn snapshot(&self) -> VadSnapshot {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_else(|_| VadSnapshot {
                phase: VadPhase::Error,
                mode: 3,
                noise_margin_db: 12,
                confirmation_ms: 480,
                noise_window_ms: 2_000,
                raw_voice_active: false,
                voice_active: false,
                level: -96.0,
                noise_calibrated: false,
                noise_floor: -96.0,
                trigger_threshold: -96.0,
                trigger_progress: 0.0,
                elapsed_ms: 0,
                remaining_ms: 0,
                first_voice_ms: None,
                last_vad_activity_ms: None,
                last_asr_activity_ms: None,
                revision: 0,
                error: Some("VAD 状态锁读取失败".to_string()),
            })
    }
}

pub struct VadGate {
    sender: Option<AudioSink>,
    command_sender: mpsc::Sender<GateCommand>,
    snapshot: Arc<Mutex<VadSnapshot>>,
    asr_session: Arc<Mutex<Option<VolcengineLiveAsrSession>>>,
    asr_start_error: Arc<Mutex<Option<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl VadGate {
    pub fn start(asr_config: AsrConfig, config: VoiceActivityDetectionConfig) -> Result<Self> {
        Self::start_internal(Some(asr_config), config)
    }

    pub fn start_test(config: VoiceActivityDetectionConfig) -> Result<Self> {
        Self::start_internal(None, config)
    }

    fn start_internal(
        asr_config: Option<AsrConfig>,
        config: VoiceActivityDetectionConfig,
    ) -> Result<Self> {
        let timeout = Duration::from_secs(config.initial_silence_timeout_secs as u64);
        let snapshot = Arc::new(Mutex::new(VadSnapshot::new(
            config.noise_margin_db,
            config.confirmation_ms,
            config.noise_window_ms,
            timeout,
        )));
        let asr_session = Arc::new(Mutex::new(None));
        let asr_start_error = Arc::new(Mutex::new(None));
        let (audio_sender, audio_receiver) = mpsc::channel::<AudioChunk>();
        let (command_sender, command_receiver) = mpsc::channel::<GateCommand>();
        let worker_snapshot = snapshot.clone();
        let worker_asr = asr_session.clone();
        let worker_error = asr_start_error.clone();
        let handle = std::thread::Builder::new()
            .name("boltscribe-vad".to_string())
            .spawn(move || {
                vad_worker(
                    asr_config,
                    config,
                    audio_receiver,
                    command_receiver,
                    worker_snapshot,
                    worker_asr,
                    worker_error,
                )
            })
            .map_err(|err| anyhow!("failed to start VAD worker: {err}"))?;

        Ok(Self {
            sender: Some(audio_sender),
            command_sender,
            snapshot,
            asr_session,
            asr_start_error,
            handle: Some(handle),
        })
    }

    pub fn audio_sender(&self) -> AudioSink {
        self.sender
            .as_ref()
            .expect("VAD gate sender missing")
            .clone()
    }

    pub fn monitor_handle(&self) -> VadMonitorHandle {
        VadMonitorHandle {
            snapshot: self.snapshot.clone(),
        }
    }

    pub fn snapshot(&self) -> VadSnapshot {
        self.monitor_handle().snapshot()
    }

    pub fn update_settings(
        &self,
        noise_margin_db: u32,
        confirmation_ms: u32,
        noise_window_ms: u32,
    ) -> Result<()> {
        let (applied_sender, applied_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(GateCommand::UpdateSettings {
                noise_margin_db,
                confirmation_ms,
                noise_window_ms,
                applied: applied_sender,
            })
            .map_err(|_| anyhow!("VAD worker is not running"))?;
        applied_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| anyhow!("VAD worker did not apply settings in time"))
    }

    pub fn trim(&self) -> Option<AudioTrim> {
        let snapshot = self.snapshot();
        snapshot.first_voice_ms?;
        let last_activity_ms = snapshot
            .last_vad_activity_ms
            .into_iter()
            .chain(snapshot.last_asr_activity_ms)
            .max()?;
        Some(AudioTrim { last_activity_ms })
    }

    pub fn finish(mut self, cancelled: bool) -> GatedAsrResult {
        self.sender.take();
        let _ = self.command_sender.send(if cancelled {
            GateCommand::Cancel
        } else {
            GateCommand::Stop
        });
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let snapshot = self.snapshot();
        let live_asr = self
            .asr_session
            .lock()
            .ok()
            .and_then(|mut session| session.take());
        let live_asr_start_error = self
            .asr_start_error
            .lock()
            .ok()
            .and_then(|mut error| error.take());
        GatedAsrResult {
            activated: snapshot.phase == VadPhase::Activated,
            live_asr,
            live_asr_start_error,
        }
    }
}

impl Drop for VadGate {
    fn drop(&mut self) {
        self.sender.take();
        let _ = self.command_sender.send(GateCommand::Cancel);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn vad_worker(
    asr_config: Option<AsrConfig>,
    config: VoiceActivityDetectionConfig,
    audio_receiver: mpsc::Receiver<AudioChunk>,
    command_receiver: mpsc::Receiver<GateCommand>,
    snapshot: Arc<Mutex<VadSnapshot>>,
    asr_session: Arc<Mutex<Option<VolcengineLiveAsrSession>>>,
    asr_start_error: Arc<Mutex<Option<String>>>,
) {
    let started_at = Instant::now();
    let timeout = Duration::from_secs(config.initial_silence_timeout_secs as u64);
    let mut converter = PcmConverter::default();
    let mut vad = Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, fixed_vad_mode());
    let mut speech_gate = SpeechGate::new(
        config.noise_margin_db,
        config.confirmation_ms,
        config.noise_window_ms,
    );
    let mut frames = Vec::new();
    let mut pre_activation_audio = PreActivationAudio::new();
    let mut activated = false;
    let mut asr_activity_offset_ms = 0u64;
    let mut stopped = false;

    while !stopped {
        while let Ok(command) = command_receiver.try_recv() {
            match command {
                GateCommand::Stop => stopped = true,
                GateCommand::Cancel => {
                    set_phase(&snapshot, VadPhase::Cancelled, None);
                    stopped = true;
                }
                GateCommand::UpdateSettings {
                    noise_margin_db,
                    confirmation_ms,
                    noise_window_ms,
                    applied,
                } => {
                    speech_gate.update_settings(noise_margin_db, confirmation_ms, noise_window_ms);
                    vad.set_mode(fixed_vad_mode());
                    if let Ok(mut state) = snapshot.lock() {
                        state.mode = WEBRTC_VAD_MODE;
                        state.noise_margin_db = speech_gate.profile.noise_margin_db;
                        state.confirmation_ms = speech_gate.profile.confirmation_ms;
                        state.noise_window_ms = speech_gate.profile.noise_window_ms;
                        state.noise_calibrated = speech_gate.noise.calibrated();
                        state.noise_floor = speech_gate.noise.floor_dbfs;
                        state.trigger_threshold = speech_gate
                            .noise
                            .trigger_threshold(speech_gate.profile.noise_margin_db as f32);
                        state.trigger_progress = 0.0;
                        state.revision = state.revision.saturating_add(1);
                    }
                    let _ = applied.send(());
                }
            }
        }
        if stopped {
            break;
        }
        if initial_silence_timed_out(activated, started_at.elapsed(), timeout) {
            set_phase(&snapshot, VadPhase::TimedOut, None);
            break;
        }

        match audio_receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => {
                let was_activated = activated;
                pre_activation_audio.push(&chunk);

                if let Ok(mut state) = snapshot.lock() {
                    state.elapsed_ms = started_at.elapsed().as_millis() as u64;
                    state.remaining_ms = timeout
                        .as_millis()
                        .saturating_sub(state.elapsed_ms as u128)
                        .min(u64::MAX as u128) as u64;
                    state.level = chunk_level_dbfs(&chunk.samples);
                }

                match converter.push_chunk(&chunk) {
                    Ok(samples) => frames.extend(samples),
                    Err(err) => {
                        let diagnostic = err.to_string();
                        if asr_config.is_some() {
                            // A real recording fails open: preserve the
                            // existing live-ASR behavior if local conversion
                            // ever fails, while retaining a sanitized note.
                            if !activated {
                                activated = true;
                                asr_activity_offset_ms = started_at.elapsed().as_millis() as u64;
                                if let Ok(mut state) = snapshot.lock() {
                                    state.phase = VadPhase::Activated;
                                    state.error = Some(diagnostic);
                                    state.revision = state.revision.saturating_add(1);
                                }
                                start_live_asr(
                                    asr_config.as_ref(),
                                    &mut pre_activation_audio,
                                    &asr_session,
                                    &asr_start_error,
                                );
                            }
                        } else {
                            set_phase(&snapshot, VadPhase::Error, Some(diagnostic));
                        }
                        continue;
                    }
                }

                while frames.len() >= FRAME_SAMPLES {
                    let frame: Vec<i16> = frames.drain(..FRAME_SAMPLES).collect();
                    let raw_voice = vad.is_voice_segment(&frame).unwrap_or(false);
                    let now_ms = started_at.elapsed().as_millis() as u64;
                    let level_dbfs = chunk_level_dbfs(&frame);
                    let decision = speech_gate.process(raw_voice, level_dbfs, now_ms);
                    if let Ok(mut state) = snapshot.lock() {
                        state.raw_voice_active = raw_voice;
                        state.voice_active = decision.qualified_voice;
                        state.level = level_dbfs;
                        state.noise_calibrated = decision.noise_calibrated;
                        state.noise_floor = decision.noise_floor_dbfs;
                        state.trigger_threshold = decision.trigger_threshold_dbfs;
                        state.trigger_progress = decision.trigger_progress;
                        if activated && decision.qualified_voice {
                            state.last_vad_activity_ms = Some(now_ms);
                        }
                        state.last_asr_activity_ms = current_asr_activity(&asr_session)
                            .map(|value| value.saturating_add(asr_activity_offset_ms));
                        state.revision = state.revision.saturating_add(1);
                    }

                    if !activated && decision.activated {
                        activated = true;
                        asr_activity_offset_ms = started_at.elapsed().as_millis() as u64;
                        if let Ok(mut state) = snapshot.lock() {
                            state.phase = VadPhase::Activated;
                            state.first_voice_ms = decision.first_qualified_ms;
                            state.last_vad_activity_ms = Some(now_ms);
                            state.trigger_progress = 1.0;
                            state.revision = state.revision.saturating_add(1);
                        }
                        start_live_asr(
                            asr_config.as_ref(),
                            &mut pre_activation_audio,
                            &asr_session,
                            &asr_start_error,
                        );
                    }
                }
                if activated && was_activated {
                    if let Ok(slot) = asr_session.lock() {
                        if let Some(session) = slot.as_ref() {
                            if let Ok(sender) = session.audio_sender() {
                                let _ = sender.send(chunk);
                            }
                        }
                    }
                }
                if let Ok(mut state) = snapshot.lock() {
                    state.last_asr_activity_ms = current_asr_activity(&asr_session)
                        .map(|value| value.saturating_add(asr_activity_offset_ms));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let elapsed_ms = started_at.elapsed().as_millis() as u64;
                if let Ok(mut state) = snapshot.lock() {
                    state.elapsed_ms = elapsed_ms;
                    state.remaining_ms = timeout
                        .as_millis()
                        .saturating_sub(elapsed_ms as u128)
                        .min(u64::MAX as u128) as u64;
                }
                if initial_silence_timed_out(activated, started_at.elapsed(), timeout) {
                    set_phase(&snapshot, VadPhase::TimedOut, None);
                    stopped = true;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => stopped = true,
        }
    }

    if !matches!(
        snapshot.lock().ok().map(|state| state.phase),
        Some(VadPhase::Cancelled | VadPhase::TimedOut)
    ) {
        set_phase(
            &snapshot,
            if activated {
                VadPhase::Activated
            } else {
                VadPhase::Cancelled
            },
            None,
        );
    }
}

fn start_live_asr(
    asr_config: Option<&AsrConfig>,
    pre_activation_audio: &mut PreActivationAudio,
    asr_session: &Arc<Mutex<Option<VolcengineLiveAsrSession>>>,
    asr_start_error: &Arc<Mutex<Option<String>>>,
) {
    let buffered = pre_activation_audio.take_for_activation();
    let Some(asr_config) = asr_config else {
        return;
    };
    match VolcengineLiveAsrSession::start_with_activity(asr_config, Some(LiveAsrActivity::new())) {
        Ok(session) => {
            if let Ok(sender) = session.audio_sender() {
                for chunk in buffered {
                    if sender.send(chunk).is_err() {
                        break;
                    }
                }
            }
            if let Ok(mut slot) = asr_session.lock() {
                *slot = Some(session);
            }
        }
        Err(err) => {
            if let Ok(mut error) = asr_start_error.lock() {
                *error = Some(err.to_string());
            }
        }
    }
}

fn initial_silence_timed_out(activated: bool, elapsed: Duration, timeout: Duration) -> bool {
    !activated && elapsed >= timeout
}

fn current_asr_activity(session: &Arc<Mutex<Option<VolcengineLiveAsrSession>>>) -> Option<u64> {
    session
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().and_then(VolcengineLiveAsrSession::activity))
        .and_then(|activity| activity.last_progress_ms())
}

fn set_phase(snapshot: &Arc<Mutex<VadSnapshot>>, phase: VadPhase, error: Option<String>) {
    if let Ok(mut state) = snapshot.lock() {
        state.phase = phase;
        state.error = error;
        state.revision = state.revision.saturating_add(1);
    }
}

fn gate_profile(noise_margin_db: u32, confirmation_ms: u32, noise_window_ms: u32) -> GateProfile {
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

fn fixed_vad_mode() -> VadMode {
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

fn chunk_level_dbfs(samples: &[i16]) -> f32 {
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

#[derive(Default)]
struct PcmConverter {
    sample_rate: Option<u32>,
    channels: Option<u16>,
    pending_mono: Vec<i16>,
    next_src_pos: f64,
}

impl PcmConverter {
    fn push_chunk(&mut self, chunk: &AudioChunk) -> Result<Vec<i16>> {
        if chunk.samples.is_empty() {
            return Ok(Vec::new());
        }
        if chunk.sample_rate == 0 || chunk.channels == 0 {
            return Err(anyhow!("Invalid VAD audio format"));
        }
        match (self.sample_rate, self.channels) {
            (None, None) => {
                self.sample_rate = Some(chunk.sample_rate);
                self.channels = Some(chunk.channels);
            }
            (Some(rate), Some(channels))
                if rate == chunk.sample_rate && channels == chunk.channels => {}
            _ => return Err(anyhow!("VAD audio format changed during recording")),
        }
        self.pending_mono
            .extend(downmix(&chunk.samples, chunk.channels));
        Ok(self.drain(false))
    }

    fn drain(&mut self, final_chunk: bool) -> Vec<i16> {
        let Some(rate) = self.sample_rate else {
            return Vec::new();
        };
        if self.pending_mono.is_empty() {
            return Vec::new();
        }
        let step = rate as f64 / TARGET_SAMPLE_RATE as f64;
        let mut output = Vec::new();
        while self.next_src_pos + 1.0 < self.pending_mono.len() as f64 {
            output.push(interpolate(&self.pending_mono, self.next_src_pos));
            self.next_src_pos += step;
        }
        if final_chunk {
            while self.next_src_pos < self.pending_mono.len() as f64 {
                output.push(interpolate(&self.pending_mono, self.next_src_pos));
                self.next_src_pos += step;
            }
            self.pending_mono.clear();
            self.next_src_pos = 0.0;
        } else {
            let consumed =
                (self.next_src_pos.floor() as usize).min(self.pending_mono.len().saturating_sub(1));
            if consumed > 0 {
                self.pending_mono.drain(..consumed);
                self.next_src_pos -= consumed as f64;
            }
        }
        output
    }
}

fn downmix(samples: &[i16], channels: u16) -> Vec<i16> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks(channels as usize)
        .map(|frame| {
            let sum = frame.iter().map(|sample| *sample as i32).sum::<i32>();
            (sum / frame.len().max(1) as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

fn interpolate(samples: &[i16], position: f64) -> i16 {
    let index = position.floor() as usize;
    let fraction = position - index as f64;
    let first = samples[index] as f64;
    let second = samples.get(index + 1).copied().unwrap_or(samples[index]) as f64;
    (first + (second - first) * fraction)
        .round()
        .clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

#[cfg(test)]
mod tests {
    use super::{
        gate_profile, initial_silence_timed_out, PcmConverter, PreActivationAudio, SpeechGate,
        FRAME_MS, FRAME_SAMPLES,
    };
    use crate::recorder::AudioChunk;
    use std::time::Duration;

    const TEST_NOISE_WINDOW_MS: u32 = 600;

    #[test]
    fn custom_gate_defaults_and_frame_alignment_are_stable() {
        let default_profile = gate_profile(12, 480, 2_000);
        let custom_profile = gate_profile(27, 913, 1_249);

        assert_eq!(super::WEBRTC_VAD_MODE, 3);
        assert_eq!(default_profile.noise_margin_db, 12);
        assert_eq!(default_profile.confirmation_ms, 480);
        assert_eq!(default_profile.noise_window_ms, 2_000);
        assert_eq!(default_profile.noise_window_frames, 100);
        assert_eq!(default_profile.required_voice_frames, 24);
        assert_eq!(default_profile.required_consecutive_frames, 12);
        assert_eq!(default_profile.window_frames, 48);
        assert_eq!(custom_profile.noise_margin_db, 27);
        assert_eq!(custom_profile.confirmation_ms, 920);
        assert_eq!(custom_profile.noise_window_ms, 1_200);
        assert_eq!(custom_profile.noise_window_frames, 60);
        assert_eq!(custom_profile.required_voice_frames, 46);
        assert_eq!(custom_profile.required_consecutive_frames, 12);
        assert_eq!(custom_profile.window_frames, 92);
    }

    #[test]
    fn pre_activation_audio_preserves_long_input_in_order_and_stops_after_activation() {
        let mut pending = PreActivationAudio::new();
        for marker in 0..300i16 {
            pending.push(&AudioChunk {
                samples: vec![marker; 20],
                sample_rate: 1_000,
                channels: 1,
            });
        }

        let buffered = pending.take_for_activation();
        assert_eq!(buffered.len(), 300);
        assert_eq!(buffered.front().unwrap().samples[0], 0);
        assert_eq!(buffered.back().unwrap().samples[0], 299);

        pending.push(&AudioChunk {
            samples: vec![300],
            sample_rate: 1_000,
            channels: 1,
        });
        assert!(pending.take_for_activation().is_empty());
    }

    #[test]
    fn initial_silence_timeout_applies_even_while_audio_keeps_arriving() {
        let timeout = Duration::from_secs(15);
        assert!(!initial_silence_timed_out(
            false,
            Duration::from_millis(14_999),
            timeout
        ));
        assert!(initial_silence_timed_out(false, timeout, timeout));
        assert!(!initial_silence_timed_out(
            true,
            Duration::from_secs(60),
            timeout
        ));
    }

    #[test]
    fn steady_fan_marked_as_voice_remains_unconfirmed_instead_of_triggering() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        let mut activated = false;

        for frame in 0..80 {
            let decision = gate.process(true, -32.0, frame * FRAME_MS);
            activated |= decision.activated;
        }

        assert!(!gate.noise.calibrated());
        assert_eq!(gate.noise.floor_dbfs, -96.0);
        assert!(!activated);
    }

    #[test]
    fn steady_fan_starting_after_quiet_calibration_does_not_trigger() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        for frame in 0..30 {
            gate.process(false, -55.0, frame * FRAME_MS);
        }
        assert!(gate.noise.calibrated());

        let mut activated = false;
        for frame in 30..130 {
            activated |= gate.process(true, -32.0, frame * FRAME_MS).activated;
        }

        assert!(!activated);
        assert_eq!(gate.noise.floor_dbfs, -55.0);
    }

    #[test]
    fn speech_present_at_start_is_not_absorbed_as_the_noise_floor() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        let bootstrap = [-46.0, -32.0, -25.0, -37.0, -23.0, -29.0];
        let mut activation = None;
        for frame in 0..60 {
            let decision = gate.process(
                true,
                bootstrap[frame as usize % bootstrap.len()],
                frame * FRAME_MS,
            );
            if decision.activated {
                activation = Some(decision);
                break;
            }
        }

        assert!(gate.noise.calibrated());
        assert!(gate.noise.floor_dbfs < -37.0);
        assert_eq!(
            activation
                .expect("the provisional horizon should replay and activate startup speech")
                .first_qualified_ms,
            Some(0)
        );
    }

    #[test]
    fn partial_quiet_then_speech_uses_the_quiet_floor_and_preserves_the_boundary() {
        let mut gate = SpeechGate::new(12, 480, 2_000);
        for frame in 0..20 {
            gate.process(false, -55.0, frame * FRAME_MS);
        }

        let speech = [-40.0, -36.0, -32.0, -38.0, -29.0, -34.0];
        for frame in 20..80 {
            gate.process(
                true,
                speech[frame as usize % speech.len()],
                frame * FRAME_MS,
            );
        }
        let mut activation = None;
        for frame in 80..110 {
            let decision = gate.process(
                true,
                speech[frame as usize % speech.len()],
                frame * FRAME_MS,
            );
            if decision.activated {
                activation = Some(decision);
                break;
            }
        }

        assert!(
            (gate.noise.floor_dbfs - -55.0).abs() < 0.1,
            "unexpected floor: {}",
            gate.noise.floor_dbfs
        );
        assert_eq!(
            activation
                .expect("partial quiet evidence should keep weak speech above the threshold")
                .first_qualified_ms,
            Some(400)
        );
    }

    #[test]
    fn default_noise_window_is_a_maximum_and_calibrates_after_enough_quiet_evidence() {
        let mut gate = SpeechGate::new(12, 480, 2_000);
        for frame in 0..39 {
            assert!(
                !gate
                    .process(false, -52.0, frame * FRAME_MS)
                    .noise_calibrated
            );
        }

        let calibrated = gate.process(false, -52.0, 39 * FRAME_MS);

        assert!(calibrated.noise_calibrated);
        assert_eq!(gate.noise.window_frames, 100);
        assert!((gate.noise.floor_dbfs - -52.0).abs() < f32::EPSILON);
    }

    #[test]
    fn recorded_ambient_voice_runs_below_the_provisional_horizon_do_not_activate() {
        let mut gate = SpeechGate::new(12, 480, 2_000);
        let first_run = [-40.0, -34.0, -29.0, -23.0, -20.0, -31.0];
        let second_run = [-50.0, -37.0, -29.0, -24.0, -22.0, -36.0];
        let mut activated = false;

        for frame in 0..160 {
            let (raw_voice, level) = if frame < 44 {
                (true, first_run[frame as usize % first_run.len()])
            } else if (71..128).contains(&frame) {
                (true, second_run[frame as usize % second_run.len()])
            } else {
                (false, -52.0 + (frame % 4) as f32 * 0.4)
            };
            activated |= gate.process(raw_voice, level, frame * FRAME_MS).activated;
        }

        assert!(gate.noise.calibrated());
        assert!(!activated);
    }

    #[test]
    fn quiet_prefix_does_not_promote_short_recorded_ambient_runs() {
        let cases: [(usize, &[f32]); 2] = [
            (44, &[-40.0, -34.0, -29.0, -23.0, -20.0, -31.0]),
            (57, &[-50.0, -37.0, -29.0, -24.0, -22.0, -36.0]),
        ];

        for (run_frames, levels) in cases {
            let mut gate = SpeechGate::new(12, 480, 2_000);
            let mut activated = false;
            for frame in 0..12 {
                activated |= gate.process(false, -52.0, frame * FRAME_MS).activated;
            }
            for offset in 0..run_frames {
                let frame = 12 + offset;
                activated |= gate
                    .process(true, levels[offset % levels.len()], frame as u64 * FRAME_MS)
                    .activated;
            }
            for frame in (12 + run_frames)..160 {
                activated |= gate
                    .process(false, -52.0, frame as u64 * FRAME_MS)
                    .activated;
            }

            assert!(!activated, "ambient run of {run_frames} frames activated");
        }
    }

    #[test]
    fn digital_silence_can_calibrate_without_absorbing_later_speech() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        for frame in 0..30 {
            gate.process(false, -96.0, frame * FRAME_MS);
        }

        let mut activated = false;
        let bootstrap = [-55.0, -42.0, -34.0, -48.0, -30.0, -39.0];
        for frame in 30..60 {
            gate.process(
                true,
                bootstrap[frame as usize % bootstrap.len()],
                frame * FRAME_MS,
            );
        }
        for frame in 60..120 {
            let level = [-40.0, -31.0, -36.0, -28.0][frame as usize % 4];
            activated |= gate.process(true, level, frame * FRAME_MS).activated;
        }

        assert!(gate.noise.floor_dbfs < -45.0);
        assert!(activated);
    }

    #[test]
    fn digital_silence_then_ambiguous_fan_restarts_calibration_without_triggering() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        for frame in 0..30 {
            gate.process(false, -96.0, frame * FRAME_MS);
        }

        let mut activated = false;
        for frame in 30..90 {
            activated |= gate.process(true, -32.0, frame * FRAME_MS).activated;
        }

        assert!(!gate.noise.calibrated());
        assert!(!activated);
    }

    #[test]
    fn speech_mixed_into_the_noise_window_uses_only_non_voice_frames_for_the_floor() {
        let mut gate = SpeechGate::new(12, 480, 2_000);
        for frame in 0..40 {
            gate.process(false, -55.0, frame * FRAME_MS);
        }
        for frame in 40..100 {
            let level = [-38.0, -27.0, -23.0, -31.0][frame as usize % 4];
            gate.process(true, level, frame * FRAME_MS);
        }

        assert!(gate.noise.calibrated());
        assert!((gate.noise.floor_dbfs - -55.0).abs() < f32::EPSILON);

        let mut activated = false;
        for frame in 100..130 {
            let level = [-30.0, -24.0, -27.0][frame as usize % 3];
            activated |= gate.process(true, level, frame * FRAME_MS).activated;
        }
        assert!(activated);
    }

    #[test]
    fn speech_covering_the_full_noise_window_is_not_absorbed_as_ambient() {
        let mut gate = SpeechGate::new(12, 480, 2_000);
        let speech = [-48.0, -36.0, -25.0, -41.0, -22.0, -31.0];
        for frame in 0..100 {
            gate.process(
                true,
                speech[frame as usize % speech.len()],
                frame * FRAME_MS,
            );
        }

        assert!(gate.noise.calibrated());
        assert!(gate.noise.floor_dbfs < -48.0);

        let mut activated = false;
        for frame in 100..130 {
            let level = [-32.0, -24.0, -29.0, -21.0][frame as usize % 4];
            activated |= gate.process(true, level, frame * FRAME_MS).activated;
        }
        assert!(activated);
    }

    #[test]
    fn dynamic_speech_over_a_steady_fan_eventually_calibrates_and_activates() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        for frame in 0..60 {
            gate.process(true, -34.0, frame * FRAME_MS);
        }
        assert!(!gate.noise.calibrated());

        let speech = [-35.0, -25.0, -20.0, -31.0, -23.0, -28.0];
        for frame in 60..90 {
            gate.process(
                true,
                speech[frame as usize % speech.len()],
                frame * FRAME_MS,
            );
        }
        assert!(gate.noise.calibrated());

        let mut activated = false;
        for frame in 90..120 {
            let level = [-27.0, -20.0, -25.0, -18.0][frame as usize % 4];
            activated |= gate.process(true, level, frame * FRAME_MS).activated;
        }
        assert!(activated);
    }

    #[test]
    fn a_short_impact_is_rejected_even_with_webrtc_hangover() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        for frame in 0..30 {
            gate.process(false, -55.0, frame * FRAME_MS);
        }

        let impact_levels = [-15.0, -26.0, -38.0, -46.0, -51.0, -54.0, -55.0, -55.0];
        let mut activated = false;
        for (index, level) in impact_levels.into_iter().enumerate() {
            let decision = gate.process(true, level, (30 + index as u64) * FRAME_MS);
            activated |= decision.activated;
        }

        assert!(!activated);
    }

    #[test]
    fn sustained_speech_above_the_noise_floor_activates() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        for frame in 0..30 {
            gate.process(false, -55.0, frame * FRAME_MS);
        }

        let mut activation = None;
        for frame in 30..60 {
            let level = [-32.0, -24.0, -29.0, -21.0][frame as usize % 4];
            let decision = gate.process(true, level, frame * FRAME_MS);
            if decision.activated {
                activation = Some(decision);
                break;
            }
        }

        let activation = activation.expect("sustained speech should activate");
        assert_eq!(activation.trigger_progress, 1.0);
        assert_eq!(activation.first_qualified_ms, Some(0));
    }

    #[test]
    fn activation_without_a_reliable_startup_boundary_keeps_the_noise_window() {
        let mut gate = SpeechGate::new(12, 480, 2_000);
        gate.process(true, -80.0, 0);
        for frame in 1..41 {
            gate.process(false, -55.0, frame * FRAME_MS);
        }
        assert!(gate.noise.calibrated());

        let speech = [-40.0, -36.0, -32.0, -38.0, -29.0, -34.0];
        let mut activation = None;
        for frame in 41..75 {
            let decision = gate.process(
                true,
                speech[frame as usize % speech.len()],
                frame * FRAME_MS,
            );
            if decision.activated {
                activation = Some(decision);
                break;
            }
        }

        assert_eq!(
            activation
                .expect("calibrated speech should activate")
                .first_qualified_ms,
            Some(0)
        );
    }

    #[test]
    fn custom_settings_update_resets_pending_voice_progress() {
        let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
        for frame in 0..30 {
            gate.process(false, -55.0, frame * FRAME_MS);
        }
        for frame in 30..35 {
            gate.process(true, -30.0, frame * FRAME_MS);
        }

        gate.update_settings(24, 800, TEST_NOISE_WINDOW_MS);
        let decision = gate.process(true, -30.0, 35 * FRAME_MS);

        assert_eq!(gate.profile.noise_margin_db, 24);
        assert_eq!(gate.profile.confirmation_ms, 800);
        assert_eq!(gate.profile.noise_window_ms, TEST_NOISE_WINDOW_MS);
        assert_eq!(gate.profile.required_voice_frames, 40);
        assert_eq!(gate.profile.required_consecutive_frames, 12);
        assert!(decision.trigger_progress < 1.0);
    }

    #[test]
    fn changing_the_noise_window_restarts_calibration_without_restarting_the_microphone() {
        let mut gate = SpeechGate::new(12, 480, 1_200);
        for frame in 0..23 {
            let decision = gate.process(false, -55.0, frame * FRAME_MS);
            assert!(!decision.noise_calibrated);
        }
        assert!(gate.process(false, -55.0, 23 * FRAME_MS).noise_calibrated);

        gate.update_settings(12, 480, 2_000);
        assert!(!gate.noise.calibrated());
        for frame in 24..63 {
            let decision = gate.process(false, -54.0, frame * FRAME_MS);
            assert!(!decision.noise_calibrated);
        }
        let recalibrated = gate.process(false, -54.0, 63 * FRAME_MS);

        assert!(recalibrated.noise_calibrated);
        assert_eq!(gate.profile.noise_window_ms, 2_000);
        assert!((gate.noise.floor_dbfs - -54.0).abs() < f32::EPSILON);
    }

    #[test]
    fn configured_noise_window_also_limits_the_adaptive_history() {
        let mut gate = SpeechGate::new(12, 480, 400);
        for frame in 0..20 {
            gate.process(false, -55.0, frame * FRAME_MS);
        }
        for frame in 20..80 {
            let level = -58.0 + (frame % 6) as f32;
            gate.process(false, level, frame * FRAME_MS);
        }

        assert!(gate.noise.calibrated());
        assert_eq!(gate.noise.window_frames, 20);
        assert_eq!(gate.noise.samples.len(), 20);
    }

    #[test]
    fn converter_produces_16khz_mono_samples() {
        let mut converter = PcmConverter::default();
        let output = converter
            .push_chunk(&AudioChunk {
                samples: vec![1; 480 * 2],
                sample_rate: 48_000,
                channels: 2,
            })
            .unwrap();
        assert!(!output.is_empty());
        assert!(output.len() <= FRAME_SAMPLES);
    }
}
