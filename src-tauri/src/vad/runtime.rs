use super::detector::{
    chunk_level_dbfs, fixed_vad_mode, SpeechGate, FRAME_SAMPLES, WEBRTC_VAD_MODE,
};
use super::pcm::PcmConverter;
use super::{GatedAsrResult, VadPhase, VadSnapshot};
use crate::asr::{LiveAsrActivity, VolcengineLiveAsrSession};
use crate::config::{AsrConfig, VoiceActivityDetectionConfig};
use crate::recorder::{AudioChunk, AudioSink, AudioTrim};
use anyhow::{anyhow, Result};
use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use webrtc_vad::{SampleRate, Vad};

pub(super) struct PreActivationAudio {
    chunks: Option<VecDeque<AudioChunk>>,
}

impl PreActivationAudio {
    pub(super) fn new() -> Self {
        Self {
            chunks: Some(VecDeque::new()),
        }
    }

    pub(super) fn push(&mut self, chunk: &AudioChunk) {
        if let Some(chunks) = self.chunks.as_mut() {
            chunks.push_back(chunk.clone());
        }
    }

    pub(super) fn take_for_activation(&mut self) -> VecDeque<AudioChunk> {
        self.chunks.take().unwrap_or_default()
    }
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

pub(super) fn initial_silence_timed_out(
    activated: bool,
    elapsed: Duration,
    timeout: Duration,
) -> bool {
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
