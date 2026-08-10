use crate::asr::{
    AsrOutput, AsrProvider, LiveAsrDiagnostics, VolcengineFileAsr, VolcengineLiveAsrSession,
};
use crate::config::{AppConfig, ConfigStore, CorrectionConfig, LlmConfig, RaceModelTarget};
use crate::corrector::{LlmCallLog, LlmProvider, OpenAiCompatibleCorrector};
use crate::history::{self, HistoryRecord, HistoryStore};
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

#[derive(Default)]
pub struct AppState {
    runtime: Mutex<WorkflowRuntime>,
    recorder: RecorderController,
    vad_test: Mutex<Option<crate::vad_test::VadTestSession>>,
}

#[derive(Default)]
struct WorkflowRuntime {
    starting: bool,
    recording: bool,
    processing: bool,
    history_retry_id: Option<String>,
    status: WorkflowStatus,
    config: Option<AppConfig>,
    live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
    vad_gate: Option<VadGate>,
    volume_ducking: Option<OutputVolumeDuckingSession>,
    audio_level_meter: Option<AudioLevelMeter>,
    active_task_id: Option<String>,
    cancel_token: Option<Arc<AtomicBool>>,
}

#[derive(Debug)]
struct CorrectionOutcome {
    text: String,
    logs: Vec<LlmCallLog>,
}

#[derive(Debug)]
struct CorrectionFailure {
    message: String,
    logs: Vec<LlmCallLog>,
}

#[derive(Clone)]
struct WorkflowTask {
    id: String,
    cancel_token: Arc<AtomicBool>,
}

struct StopContext {
    task: WorkflowTask,
    config: Option<AppConfig>,
    live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
    vad_gate: Option<VadGate>,
    volume_ducking: Option<OutputVolumeDuckingSession>,
    pending_stop: Option<PendingRecordingStop>,
}

enum ToggleAction {
    Busy,
    Start(WorkflowTask),
    Stop(Box<StopContext>),
    CancelArmed(WorkflowTask),
}

struct TogglePlan {
    status: WorkflowStatus,
    action: ToggleAction,
}

struct HistoryRetryGuard<'a> {
    state: &'a AppState,
    record_id: String,
}

impl WorkflowTask {
    fn cancelled(&self) -> bool {
        self.cancel_token.load(Ordering::SeqCst)
    }
}

impl Drop for HistoryRetryGuard<'_> {
    fn drop(&mut self) {
        let Ok(mut runtime) = self.state.runtime.lock() else {
            return;
        };
        if runtime.history_retry_id.as_deref() == Some(self.record_id.as_str()) {
            runtime.history_retry_id = None;
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct WorkflowStatus {
    pub mode: WorkflowMode,
    pub stage: WorkflowStage,
    pub message: String,
    pub current_audio_path: Option<String>,
    pub last_record_id: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize)]
struct AudioLevelSample {
    level: f32,
    recording_revision: u64,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowMode {
    #[default]
    Idle,
    Starting,
    Recording,
    Processing,
    Error,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStage {
    #[default]
    Idle,
    Starting,
    WaitingForSpeech,
    Recording,
    Recognizing,
    FileAsrFallback,
    Correcting,
    Pasting,
    Complete,
    Error,
}

impl WorkflowRuntime {
    fn update_status(&mut self, mut status: WorkflowStatus) -> WorkflowStatus {
        status.revision = self.status.revision.saturating_add(1);
        self.status = status.clone();
        status
    }

    fn update_task_status(
        &mut self,
        task_id: &str,
        status: WorkflowStatus,
    ) -> Option<WorkflowStatus> {
        if self.active_task_id.as_deref() != Some(task_id) {
            return None;
        }

        self.starting = status.mode == WorkflowMode::Starting;
        self.recording = status.mode == WorkflowMode::Recording;
        self.processing = status.mode == WorkflowMode::Processing;
        if matches!(&status.mode, WorkflowMode::Idle | WorkflowMode::Error) {
            stop_audio_level_meter(self);
            self.active_task_id = None;
            self.cancel_token = None;
        }
        Some(self.update_status(status))
    }
}

impl AppState {
    pub(crate) fn recorder_ref(&self) -> &RecorderController {
        &self.recorder
    }

    pub(crate) fn vad_test_slot(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<crate::vad_test::VadTestSession>>> {
        self.vad_test
            .lock()
            .map_err(|_| anyhow!("Failed to lock VAD test state"))
    }

    pub(crate) fn runtime_for_vad_test(&self) -> Result<bool> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        Ok(runtime.starting
            || runtime.recording
            || runtime.processing
            || runtime.history_retry_id.is_some())
    }

    pub fn status(&self) -> WorkflowStatus {
        self.runtime
            .lock()
            .map(|runtime| runtime.status.clone())
            .unwrap_or_else(|_| WorkflowStatus {
                mode: WorkflowMode::Error,
                stage: WorkflowStage::Error,
                message: "状态锁读取失败".to_string(),
                current_audio_path: None,
                last_record_id: None,
                revision: 0,
            })
    }

    pub(crate) fn run_while_inactive<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        if runtime.starting
            || runtime.recording
            || runtime.processing
            || runtime.history_retry_id.is_some()
            || self
                .vad_test
                .lock()
                .map(|test| test.is_some())
                .unwrap_or(true)
        {
            return Err(anyhow!(
                "Cannot modify local data while recording or processing"
            ));
        }
        let result = action();
        drop(runtime);
        result
    }

    fn begin_history_retry(&self, record_id: &str) -> Result<HistoryRetryGuard<'_>> {
        let record_id = record_id.trim();
        if record_id.is_empty() {
            return Err(anyhow!("History record id cannot be empty"));
        }

        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        if runtime.starting || runtime.recording || runtime.processing {
            return Err(anyhow!(
                "Cannot retry history while recording or processing"
            ));
        }
        if runtime.history_retry_id.is_some() {
            return Err(anyhow!("Another history record is already retrying"));
        }
        runtime.history_retry_id = Some(record_id.to_string());
        drop(runtime);

        Ok(HistoryRetryGuard {
            state: self,
            record_id: record_id.to_string(),
        })
    }
}

pub fn toggle_recording_from_app(app: AppHandle) -> Result<WorkflowStatus> {
    let state = app.state::<AppState>();
    toggle_recording(app.clone(), state.inner())
}

pub fn toggle_recording(app: AppHandle, state: &AppState) -> Result<WorkflowStatus> {
    let vad_test_active = {
        let _runtime = state
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        state
            .vad_test
            .lock()
            .map_err(|_| anyhow!("Failed to lock VAD test state"))?
            .is_some()
    };
    if vad_test_active {
        return Err(anyhow!("VAD microphone test is active"));
    }
    let plan = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        let plan = prepare_toggle(&mut runtime)?;
        dispatch_pending_stop(&mut runtime, &state.recorder, plan)
    };
    publish_status(&app, &plan.status);

    match plan.action {
        ToggleAction::Busy => {}
        ToggleAction::CancelArmed(_task) => {
            if let Some(state) = app.try_state::<AppState>() {
                let _ = cancel_current_workflow(app.clone(), state.inner());
            }
        }
        ToggleAction::Start(task) => {
            let recorder = state.recorder.clone();
            std::thread::spawn(move || start_recording_attempt(app, task, recorder));
        }
        ToggleAction::Stop(context) => {
            let mut context = *context;
            let pending_stop = context
                .pending_stop
                .take()
                .ok_or_else(|| anyhow!("Recorder stop was not dispatched"))?;
            std::thread::spawn(move || {
                let task_for_error = context.task.clone();
                if let Err(err) = stop_and_process_recording(
                    app.clone(),
                    context.task,
                    pending_stop,
                    context.config,
                    context.live_asr,
                    context.live_asr_start_error,
                    context.vad_gate,
                    context.volume_ducking,
                ) {
                    if task_for_error.cancelled() {
                        return;
                    }
                    let message = format!("处理失败：{err}");
                    set_status_for_task(
                        &app,
                        &task_for_error.id,
                        WorkflowStatus {
                            mode: WorkflowMode::Processing,
                            stage: WorkflowStage::Error,
                            message: message.clone(),
                            current_audio_path: None,
                            last_record_id: None,
                            revision: 0,
                        },
                    );
                    std::thread::sleep(Duration::from_millis(700));
                    set_status_for_task(
                        &app,
                        &task_for_error.id,
                        WorkflowStatus {
                            mode: WorkflowMode::Error,
                            stage: WorkflowStage::Error,
                            message,
                            current_audio_path: None,
                            last_record_id: None,
                            revision: 0,
                        },
                    );
                }
            });
        }
    }

    Ok(plan.status)
}

pub fn retry_history_record(state: &AppState, record_id: &str) -> Result<HistoryRecord> {
    let _retry_guard = state.begin_history_retry(record_id)?;
    let total_started_at = Instant::now();
    let record = HistoryStore::load_retryable(record_id)?;
    let audio_path = record
        .audio_path
        .as_deref()
        .ok_or_else(|| anyhow!("The recording for this history record is unavailable"))?;
    let config = ConfigStore::load()?;

    let asr_started_at = Instant::now();
    let asr_output = VolcengineFileAsr.transcribe(audio_path, &config.asr)?;
    let asr_duration_ms = asr_started_at.elapsed().as_millis() as u64;
    let mut updated = build_retried_record(record, asr_output, &config, asr_duration_ms)?;
    updated.total_duration_ms = total_started_at.elapsed().as_millis();
    HistoryStore::replace(&updated)?;
    Ok(updated)
}

fn prepare_toggle(runtime: &mut WorkflowRuntime) -> Result<TogglePlan> {
    if runtime.history_retry_id.is_some() {
        return Ok(TogglePlan {
            status: runtime.status.clone(),
            action: ToggleAction::Busy,
        });
    }

    if runtime.starting {
        let status = runtime.update_status(WorkflowStatus {
            mode: WorkflowMode::Starting,
            stage: WorkflowStage::Starting,
            message: "正在启动麦克风，请稍候".to_string(),
            current_audio_path: None,
            last_record_id: runtime.status.last_record_id.clone(),
            revision: 0,
        });
        return Ok(TogglePlan {
            status,
            action: ToggleAction::Busy,
        });
    }

    if runtime.processing {
        return Ok(TogglePlan {
            status: runtime.status.clone(),
            action: ToggleAction::Busy,
        });
    }

    if runtime.recording {
        if runtime
            .vad_gate
            .as_ref()
            .is_some_and(|gate| gate.snapshot().phase != VadPhase::Activated)
        {
            let task = WorkflowTask {
                id: runtime
                    .active_task_id
                    .clone()
                    .ok_or_else(|| anyhow!("Workflow task is missing"))?,
                cancel_token: runtime
                    .cancel_token
                    .clone()
                    .ok_or_else(|| anyhow!("Workflow cancel token is missing"))?,
            };
            return Ok(TogglePlan {
                status: runtime.status.clone(),
                action: ToggleAction::CancelArmed(task),
            });
        }
        stop_audio_level_meter(runtime);
        let task = WorkflowTask {
            id: runtime
                .active_task_id
                .clone()
                .ok_or_else(|| anyhow!("Workflow task is missing"))?,
            cancel_token: runtime
                .cancel_token
                .clone()
                .ok_or_else(|| anyhow!("Workflow cancel token is missing"))?,
        };
        let context = StopContext {
            task,
            config: runtime.config.take(),
            live_asr: runtime.live_asr.take(),
            live_asr_start_error: runtime.live_asr_start_error.take(),
            vad_gate: runtime.vad_gate.take(),
            volume_ducking: runtime.volume_ducking.take(),
            pending_stop: None,
        };
        runtime.recording = false;
        runtime.processing = true;
        let status = runtime.update_status(WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::Recognizing,
            message: "正在停止录音并处理转写".to_string(),
            current_audio_path: None,
            last_record_id: runtime.status.last_record_id.clone(),
            revision: 0,
        });
        return Ok(TogglePlan {
            status,
            action: ToggleAction::Stop(Box::new(context)),
        });
    }

    let task = WorkflowTask {
        id: Uuid::new_v4().to_string(),
        cancel_token: Arc::new(AtomicBool::new(false)),
    };
    runtime.starting = true;
    runtime.active_task_id = Some(task.id.clone());
    runtime.cancel_token = Some(task.cancel_token.clone());
    let status = runtime.update_status(WorkflowStatus {
        mode: WorkflowMode::Starting,
        stage: WorkflowStage::Starting,
        message: "正在启动麦克风，请稍候".to_string(),
        current_audio_path: None,
        last_record_id: runtime.status.last_record_id.clone(),
        revision: 0,
    });
    Ok(TogglePlan {
        status,
        action: ToggleAction::Start(task),
    })
}

fn dispatch_pending_stop(
    runtime: &mut WorkflowRuntime,
    recorder: &RecorderController,
    mut plan: TogglePlan,
) -> TogglePlan {
    let ToggleAction::Stop(context) = &mut plan.action else {
        return plan;
    };

    let trim = context.vad_gate.as_ref().and_then(VadGate::trim);
    let dispatch_result = paths::recordings_dir().and_then(|path| match trim {
        Some(trim) => recorder.begin_stop_with_trim(path, Some(trim)),
        None => recorder.begin_stop(path),
    });
    match dispatch_result {
        Ok(pending_stop) => {
            context.pending_stop = Some(pending_stop);
            plan
        }
        Err(err) => {
            context.task.cancel_token.store(true, Ordering::SeqCst);
            let cleanup_error = recorder.cancel().err();
            restore_output_volume(context.volume_ducking.take());

            runtime.starting = false;
            runtime.recording = false;
            runtime.processing = false;
            runtime.config = None;
            runtime.live_asr = None;
            runtime.live_asr_start_error = None;
            runtime.vad_gate = None;
            runtime.volume_ducking = None;
            runtime.active_task_id = None;
            runtime.cancel_token = None;
            let last_record_id = runtime.status.last_record_id.clone();
            let cleanup_suffix = cleanup_error
                .map(|cleanup_err| format!("；清理录音器失败：{cleanup_err:#}"))
                .unwrap_or_default();
            let status = runtime.update_status(WorkflowStatus {
                mode: WorkflowMode::Error,
                stage: WorkflowStage::Error,
                message: format!("停止录音失败：{err:#}{cleanup_suffix}"),
                current_audio_path: None,
                last_record_id,
                revision: 0,
            });
            TogglePlan {
                status,
                action: ToggleAction::Busy,
            }
        }
    }
}

fn start_recording_attempt(app: AppHandle, task: WorkflowTask, recorder: RecorderController) {
    let total_started_at = Instant::now();
    let config_started_at = Instant::now();
    let config = match ConfigStore::load() {
        Ok(config) => config,
        Err(err) => {
            fail_recording_start(&app, &task.id, &err);
            return;
        }
    };
    log_timing("config load", config_started_at, total_started_at);

    let mut vad_gate = None;
    let (live_asr, live_asr_start_error, audio_sink) =
        if config.audio.voice_activity_detection.enabled {
            match VadGate::start(
                config.asr.clone(),
                config.audio.voice_activity_detection.clone(),
            ) {
                Ok(gate) => {
                    let sink = gate.audio_sender();
                    vad_gate = Some(gate);
                    (None, None, Some(sink))
                }
                Err(err) => {
                    eprintln!("VAD initialization failed; using legacy ASR flow: {err:#}");
                    let live_asr_started_at = Instant::now();
                    let result = VolcengineLiveAsrSession::start(&config.asr);
                    log_timing(
                        "live ASR session start",
                        live_asr_started_at,
                        total_started_at,
                    );
                    match result {
                        Ok(session) => {
                            let sink = session.audio_sender().ok();
                            (Some(session), None, sink)
                        }
                        Err(start_error) => (None, Some(start_error.to_string()), None),
                    }
                }
            }
        } else {
            let live_asr_started_at = Instant::now();
            let result = VolcengineLiveAsrSession::start(&config.asr);
            log_timing(
                "live ASR session start",
                live_asr_started_at,
                total_started_at,
            );
            match result {
                Ok(session) => {
                    let sink = session.audio_sender().ok();
                    (Some(session), None, sink)
                }
                Err(start_error) => (None, Some(start_error.to_string()), None),
            }
        };
    let mut volume_ducking =
        match output_volume::start_ducking_session(&config.audio.output_volume_ducking) {
            Ok(session) => session,
            Err(err) => {
                eprintln!("failed to duck output volume: {err:?}");
                None
            }
        };
    let recorder_started_at = Instant::now();
    let audio_level_meter = AudioLevelMeter::new();
    if let Err(err) = recorder.start_with_config(
        audio_sink,
        Some(audio_level_meter.clone()),
        config.audio.clone(),
    ) {
        audio_level_meter.stop();
        restore_output_volume(volume_ducking.take());
        fail_recording_start(&app, &task.id, &err);
        return;
    }
    log_timing(
        "recorder.start_with_config",
        recorder_started_at,
        total_started_at,
    );
    eprintln!(
        "[Timing] request_start_recording total sync: {}ms",
        total_started_at.elapsed().as_millis()
    );

    let mut config = Some(config);
    let mut live_asr = live_asr;
    let update = app.try_state::<AppState>().and_then(|state| {
        let mut runtime = state.runtime.lock().ok()?;
        if !runtime.starting || runtime.active_task_id.as_deref() != Some(task.id.as_str()) {
            return None;
        }
        runtime.starting = false;
        runtime.recording = true;
        runtime.config = config.take();
        runtime.live_asr = live_asr.take();
        runtime.live_asr_start_error = live_asr_start_error;
        runtime.vad_gate = vad_gate.take();
        runtime.volume_ducking = volume_ducking.take();
        runtime.audio_level_meter = Some(audio_level_meter.clone());
        let waiting_for_speech = runtime.vad_gate.is_some();
        let last_record_id = runtime.status.last_record_id.clone();
        Some(runtime.update_status(WorkflowStatus {
            mode: WorkflowMode::Recording,
            stage: if waiting_for_speech {
                WorkflowStage::WaitingForSpeech
            } else {
                WorkflowStage::Recording
            },
            message: if waiting_for_speech {
                "等待说话".to_string()
            } else {
                "正在录音，再次按快捷键停止".to_string()
            },
            current_audio_path: None,
            last_record_id,
            revision: 0,
        }))
    });

    if let Some(status) = update {
        publish_status(&app, &status);
        spawn_audio_level_emitter(app.clone(), audio_level_meter, task.id.clone());
        if let Some(gate) = app.try_state::<AppState>().and_then(|state| {
            state
                .runtime
                .lock()
                .ok()
                .and_then(|runtime| runtime.vad_gate.as_ref().map(VadGate::monitor_handle))
        }) {
            spawn_vad_monitor(app, task, gate);
        }
        return;
    }

    audio_level_meter.stop();
    if let Err(err) = recorder.cancel() {
        eprintln!("failed to cancel stale recording start: {err:?}");
    }
    restore_output_volume(volume_ducking.take());
}

fn fail_recording_start(app: &AppHandle, task_id: &str, error: &anyhow::Error) {
    eprintln!("recording start failed: {error:#}");
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Ok(mut runtime) = state.runtime.lock() else {
        return;
    };
    if !runtime.starting || runtime.active_task_id.as_deref() != Some(task_id) {
        return;
    }

    runtime.starting = false;
    runtime.recording = false;
    runtime.processing = false;
    runtime.config = None;
    runtime.live_asr = None;
    runtime.live_asr_start_error = None;
    runtime.vad_gate = None;
    runtime.volume_ducking = None;
    stop_audio_level_meter(&mut runtime);
    runtime.active_task_id = None;
    runtime.cancel_token = None;
    let last_record_id = runtime.status.last_record_id.clone();
    let status = runtime.update_status(WorkflowStatus {
        mode: WorkflowMode::Error,
        stage: WorkflowStage::Error,
        message: format!("录音启动失败：{error:#}"),
        current_audio_path: None,
        last_record_id,
        revision: 0,
    });
    drop(runtime);
    publish_status(app, &status);
}

fn cancel_task_if_current(app: &AppHandle, task: &WorkflowTask) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    let should_cancel = state.runtime.lock().ok().is_some_and(|runtime| {
        runtime.recording
            && runtime.active_task_id.as_deref() == Some(task.id.as_str())
            && runtime
                .vad_gate
                .as_ref()
                .is_some_and(|gate| gate.snapshot().phase == VadPhase::TimedOut)
    });
    if !should_cancel {
        return false;
    }
    if let Err(err) = cancel_current_workflow(app.clone(), state.inner()) {
        eprintln!("failed to cancel timed-out VAD recording: {err:#}");
        return false;
    }
    true
}

fn spawn_vad_monitor(app: AppHandle, task: WorkflowTask, monitor: VadMonitorHandle) {
    std::thread::Builder::new()
        .name("boltscribe-vad-monitor".to_string())
        .spawn(move || {
            let mut announced_activation = false;
            loop {
                if task.cancelled() {
                    return;
                }
                if let Some(state) = app.try_state::<AppState>() {
                    let active = state.runtime.lock().ok().is_some_and(|runtime| {
                        runtime.recording
                            && runtime.active_task_id.as_deref() == Some(task.id.as_str())
                    });
                    if !active {
                        return;
                    }
                }
                let snapshot = monitor.snapshot();
                match snapshot.phase {
                    VadPhase::TimedOut => {
                        let _ = cancel_task_if_current(&app, &task);
                        return;
                    }
                    VadPhase::Activated => {
                        if !announced_activation {
                            announced_activation = true;
                            set_status_for_task(
                                &app,
                                &task.id,
                                WorkflowStatus {
                                    mode: WorkflowMode::Recording,
                                    stage: WorkflowStage::Recording,
                                    message: "正在录音，再次按快捷键停止".to_string(),
                                    current_audio_path: None,
                                    last_record_id: None,
                                    revision: 0,
                                },
                            );
                        }
                        let last_activity = snapshot
                            .last_vad_activity_ms
                            .into_iter()
                            .chain(snapshot.last_asr_activity_ms)
                            .max();
                        if let Some(last_activity) = last_activity {
                            if snapshot.elapsed_ms >= last_activity.saturating_add(30_000) {
                                if let Some(state) = app.try_state::<AppState>() {
                                    let active = state.runtime.lock().ok().is_some_and(|runtime| {
                                        runtime.recording
                                            && runtime.active_task_id.as_deref()
                                                == Some(task.id.as_str())
                                    });
                                    if active {
                                        let _ = toggle_recording(app.clone(), state.inner());
                                    }
                                }
                                return;
                            }
                        }
                    }
                    VadPhase::Cancelled | VadPhase::Error => return,
                    VadPhase::Armed => {}
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .ok();
}

pub fn cancel_current_workflow(app: AppHandle, state: &AppState) -> Result<WorkflowStatus> {
    let status = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        if runtime.starting {
            return Ok(runtime.status.clone());
        }
        if !runtime.recording && !runtime.processing {
            if runtime.status.mode != WorkflowMode::Error {
                return Ok(runtime.status.clone());
            }
            let current_audio_path = runtime.status.current_audio_path.clone();
            let last_record_id = runtime.status.last_record_id.clone();
            runtime.update_status(WorkflowStatus {
                mode: WorkflowMode::Idle,
                stage: WorkflowStage::Idle,
                message: "就绪".to_string(),
                current_audio_path,
                last_record_id,
                revision: 0,
            })
        } else {
            if let Some(token) = &runtime.cancel_token {
                token.store(true, Ordering::SeqCst);
            }

            stop_audio_level_meter(&mut runtime);
            let vad_timed_out = runtime
                .vad_gate
                .as_ref()
                .is_some_and(|gate| gate.snapshot().phase == VadPhase::TimedOut);
            let vad_gate = runtime.vad_gate.take();
            let cancel_error = state.recorder.cancel().err();
            if let Some(gate) = vad_gate {
                let _ = gate.finish(true);
            }
            restore_output_volume(runtime.volume_ducking.take());
            runtime.starting = false;
            runtime.recording = false;
            runtime.processing = false;
            runtime.config = None;
            runtime.live_asr = None;
            runtime.live_asr_start_error = None;
            runtime.vad_gate = None;
            runtime.active_task_id = None;
            runtime.cancel_token = None;
            let current_audio_path = runtime.status.current_audio_path.clone();
            let last_record_id = runtime.status.last_record_id.clone();
            runtime.update_status(WorkflowStatus {
                mode: if cancel_error.is_some() {
                    WorkflowMode::Error
                } else {
                    WorkflowMode::Idle
                },
                stage: if cancel_error.is_some() {
                    WorkflowStage::Error
                } else {
                    WorkflowStage::Idle
                },
                message: cancel_error
                    .map(|err| format!("取消录音失败：{err:#}"))
                    .unwrap_or_else(|| {
                        if vad_timed_out {
                            "未检测到语音，已自动取消".to_string()
                        } else {
                            "已取消本次转写".to_string()
                        }
                    }),
                current_audio_path,
                last_record_id,
                revision: 0,
            })
        }
    };
    publish_status(&app, &status);
    Ok(status)
}

fn restore_output_volume(session: Option<OutputVolumeDuckingSession>) {
    if let Some(session) = session {
        if let Err(err) = session.restore() {
            eprintln!("failed to restore output volume: {err:?}");
        }
    }
}

fn stop_audio_level_meter(runtime: &mut WorkflowRuntime) {
    if let Some(meter) = runtime.audio_level_meter.take() {
        meter.stop();
    }
}

#[allow(clippy::too_many_arguments)]
fn stop_and_process_recording(
    app: AppHandle,
    task: WorkflowTask,
    pending_stop: PendingRecordingStop,
    config: Option<AppConfig>,
    live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
    vad_gate: Option<VadGate>,
    volume_ducking: Option<OutputVolumeDuckingSession>,
) -> Result<()> {
    let total_started_at = Instant::now();
    let retention = config
        .as_ref()
        .map(|config| config.retention.clone())
        .unwrap_or_default();
    let recorder_stop_started_at = Instant::now();
    let recorded = pending_stop.wait();
    restore_output_volume(volume_ducking);
    let recorded = recorded?;
    let gated = vad_gate.map(|gate| gate.finish(false));
    let (live_asr, live_asr_start_error) = match gated {
        Some(result) if !result.activated => {
            recorded.discard()?;
            set_status_for_task(
                &app,
                &task.id,
                WorkflowStatus {
                    mode: WorkflowMode::Idle,
                    stage: WorkflowStage::Idle,
                    message: "未检测到语音，已忽略本次转写".to_string(),
                    current_audio_path: None,
                    last_record_id: None,
                    revision: 0,
                },
            );
            return Ok(());
        }
        Some(result) => (result.live_asr, result.live_asr_start_error),
        None => (live_asr, live_asr_start_error),
    };
    log_timing("recorder.stop", recorder_stop_started_at, total_started_at);
    if task.cancelled() {
        recorded.discard()?;
        return Ok(());
    }
    let mut live_asr_diagnostics = None;
    let process_result = process_recording(
        app.clone(),
        &task,
        recorded.clone(),
        total_started_at,
        config,
        live_asr,
        live_asr_start_error,
        &mut live_asr_diagnostics,
    );
    if task.cancelled() {
        return recorded.discard();
    }

    match process_result {
        Ok(()) => Ok(()),
        Err(err) if history::is_empty_asr_text_error(&err.to_string()) => {
            recorded.discard()?;
            set_status_for_task(
                &app,
                &task.id,
                WorkflowStatus {
                    mode: WorkflowMode::Idle,
                    stage: WorkflowStage::Idle,
                    message: "未检测到语音，已忽略本次转写".to_string(),
                    current_audio_path: None,
                    last_record_id: None,
                    revision: 0,
                },
            );
            Ok(())
        }
        Err(err) => {
            if append_failed_history(
                recorded,
                err.to_string(),
                total_started_at,
                &retention,
                live_asr_diagnostics,
            )
            .is_ok()
            {
                let _ = app.emit("history://updated", ());
            }
            Err(err)
        }
    }
}

fn log_timing(stage: &str, started_at: Instant, total_started_at: Instant) {
    eprintln!(
        "[Timing] {stage}: {}ms (total: {}ms)",
        started_at.elapsed().as_millis(),
        total_started_at.elapsed().as_millis()
    );
}

#[allow(clippy::too_many_arguments)]
fn process_recording(
    app: AppHandle,
    task: &WorkflowTask,
    recorded: RecordedAudio,
    total_started_at: Instant,
    config: Option<AppConfig>,
    live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
    live_asr_diagnostics: &mut Option<LiveAsrDiagnostics>,
) -> Result<()> {
    let audio_path = recorded.path.clone();
    if task.cancelled() {
        return recorded.discard();
    }
    if !set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::Recognizing,
            message: "录音已保存，正在调用语音识别".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
    ) {
        return recorded.discard();
    }

    let config = match config {
        Some(config) => config,
        None => ConfigStore::load()?,
    };
    let asr_started_at = Instant::now();
    let asr_output = transcribe_with_live_fallback(
        &app,
        &task.id,
        &recorded,
        &config,
        live_asr,
        live_asr_start_error,
        live_asr_diagnostics,
    )?;
    log_timing("ASR", asr_started_at, total_started_at);
    eprintln!(
        "[Timing] ASR provider={}, service_audio_duration_ms={:?}",
        asr_output.provider, asr_output.duration_ms
    );
    if task.cancelled() {
        return recorded.discard();
    }
    let asr_elapsed_ms = Some(asr_started_at.elapsed().as_millis() as u64);
    let raw_text = asr_output.text.trim().to_string();
    if raw_text.is_empty() {
        return Err(anyhow!("ASR returned empty text"));
    }

    if !set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::Correcting,
            message: "语音识别完成，正在纠错".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
    ) {
        return recorded.discard();
    }

    let correction_started_at = Instant::now();
    let (corrected_text, correction_error, correction_logs) =
        correct_recording_text(&raw_text, &config);
    log_timing("AI correction", correction_started_at, total_started_at);
    let pasted_text = corrected_text.clone();
    if task.cancelled() {
        return recorded.discard();
    }

    if !set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::Pasting,
            message: "正在粘贴文本".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
    ) {
        return recorded.discard();
    }

    let injection_started_at = Instant::now();
    let injection_error = injector::paste_text(&pasted_text)
        .err()
        .map(|err| err.to_string());
    log_timing("text injection", injection_started_at, total_started_at);
    if task.cancelled() {
        return recorded.discard();
    }
    if injection_error.is_none() {
        if !set_status_for_task(
            &app,
            &task.id,
            WorkflowStatus {
                mode: WorkflowMode::Processing,
                stage: WorkflowStage::Complete,
                message: "粘贴完成".to_string(),
                current_audio_path: Some(audio_path.display().to_string()),
                last_record_id: None,
                revision: 0,
            },
        ) {
            return recorded.discard();
        }
        std::thread::sleep(std::time::Duration::from_millis(700));
    }

    let record = HistoryRecord {
        id: recorded.id.clone(),
        created_at: Utc::now(),
        audio_path: Some(audio_path.clone()),
        asr_provider: asr_output.provider,
        asr_task_id: asr_output.task_id,
        audio_started_at: recorded.started_at,
        audio_finished_at: recorded.finished_at,
        audio_sample_rate: recorded.sample_rate,
        audio_channels: recorded.channels,
        audio_sample_count: recorded.sample_count,
        raw_text,
        corrected_text,
        pasted_text,
        correction_enabled: config.correction.enabled,
        correction_error,
        correction_logs,
        injection_error,
        workflow_error: None,
        asr_duration_ms: asr_elapsed_ms,
        service_audio_duration_ms: asr_output.duration_ms,
        live_asr_diagnostics: live_asr_diagnostics.clone(),
        total_duration_ms: total_started_at.elapsed().as_millis(),
    };
    HistoryStore::append(&record, &config.retention)?;

    set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Idle,
            stage: WorkflowStage::Idle,
            message: if record.injection_error.is_some() {
                paste_failure_message()
            } else if record.correction_error.is_some() {
                "处理完成，纠错失败，已粘贴原始转写".to_string()
            } else {
                "处理完成，已粘贴纠错文本".to_string()
            },
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: Some(record.id),
            revision: 0,
        },
    );
    let _ = app.emit("history://updated", ());
    eprintln!(
        "[Timing] workflow total: {}ms",
        total_started_at.elapsed().as_millis()
    );
    Ok(())
}

fn paste_failure_message() -> String {
    if cfg!(target_os = "windows") {
        "处理完成，但粘贴失败，请检查剪贴板或当前输入位置".to_string()
    } else {
        "处理完成，但粘贴失败，请检查辅助功能权限".to_string()
    }
}

fn transcribe_with_live_fallback(
    app: &AppHandle,
    task_id: &str,
    recorded: &RecordedAudio,
    config: &AppConfig,
    live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
    diagnostics: &mut Option<LiveAsrDiagnostics>,
) -> Result<AsrOutput> {
    if let Some(session) = live_asr {
        let result = session.finish();
        *diagnostics = Some(result.diagnostics);
        match result.output {
            Ok(output) => return Ok(output),
            Err(err) => {
                publish_file_asr_fallback_status(app, task_id, recorded);
                eprintln!("live ASR failed, falling back to file ASR: {err:#}");
            }
        }
    } else if let Some(err) = live_asr_start_error {
        *diagnostics = Some(LiveAsrDiagnostics {
            last_error_category: Some("configuration".to_string()),
            fallback_reason: Some("session_start_failed".to_string()),
            ..Default::default()
        });
        publish_file_asr_fallback_status(app, task_id, recorded);
        eprintln!("live ASR did not start, falling back to file ASR: {err}");
    } else {
        publish_file_asr_fallback_status(app, task_id, recorded);
    }

    let mut output = VolcengineFileAsr.transcribe(&recorded.path, &config.asr)?;
    output.provider = "volcengine_ws_file_fallback".to_string();
    Ok(output)
}

fn publish_file_asr_fallback_status(app: &AppHandle, task_id: &str, recorded: &RecordedAudio) {
    set_status_for_task(
        app,
        task_id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::FileAsrFallback,
            message: "正在回退到录音文件识别".to_string(),
            current_audio_path: Some(recorded.path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
    );
}

fn correct_recording_text(
    raw_text: &str,
    config: &AppConfig,
) -> (String, Option<String>, Vec<LlmCallLog>) {
    if !config.correction.enabled {
        return (raw_text.to_string(), None, Vec::new());
    }

    match correct_with_config(raw_text, &config.llm, &config.correction) {
        Ok(outcome) => (outcome.text, None, outcome.logs),
        Err(err) => (raw_text.to_string(), Some(err.message), err.logs),
    }
}

fn build_retried_record(
    mut record: HistoryRecord,
    asr_output: AsrOutput,
    config: &AppConfig,
    asr_duration_ms: u64,
) -> Result<HistoryRecord> {
    let raw_text = asr_output.text.trim().to_string();
    if raw_text.is_empty() {
        return Err(anyhow!("ASR returned empty text"));
    }
    let (corrected_text, correction_error, correction_logs) =
        correct_recording_text(&raw_text, config);

    record.asr_provider = asr_output.provider;
    record.asr_task_id = asr_output.task_id;
    record.raw_text = raw_text;
    record.corrected_text = corrected_text;
    // A manual retry updates history only. It must never paste into whichever
    // application happens to be focused while the history window is open.
    record.pasted_text.clear();
    record.correction_enabled = config.correction.enabled;
    record.correction_error = correction_error;
    record.correction_logs = correction_logs;
    record.injection_error = None;
    record.workflow_error = None;
    record.asr_duration_ms = Some(asr_duration_ms);
    record.service_audio_duration_ms = asr_output.duration_ms;
    Ok(record)
}

fn correct_with_config(
    raw_text: &str,
    llm: &LlmConfig,
    correction: &CorrectionConfig,
) -> std::result::Result<CorrectionOutcome, CorrectionFailure> {
    let race_targets = active_race_targets(llm);
    if race_targets.len() < 2 {
        return OpenAiCompatibleCorrector
            .correct(raw_text, llm, correction)
            .map(|output| CorrectionOutcome {
                text: output.text,
                logs: vec![output.log],
            })
            .map_err(|err| CorrectionFailure {
                message: err.message,
                logs: err.log.into_iter().map(|log| *log).collect(),
            });
    }

    correct_with_race(raw_text, llm, correction, race_targets)
}

fn active_race_targets(llm: &LlmConfig) -> Vec<RaceModelTarget> {
    if !llm.race_enabled {
        return Vec::new();
    }

    let source_targets = if llm.race_targets.is_empty() {
        llm.race_models
            .iter()
            .map(|model| RaceModelTarget {
                provider: llm.provider.clone(),
                model: model.clone(),
            })
            .collect::<Vec<_>>()
    } else {
        llm.race_targets.clone()
    };

    let mut targets = Vec::new();
    for target in source_targets {
        let provider = target.provider.trim();
        let model = target.model.trim();
        if provider.is_empty()
            || model.is_empty()
            || targets.iter().any(|existing: &RaceModelTarget| {
                existing.provider == provider && existing.model == model
            })
        {
            continue;
        }
        targets.push(RaceModelTarget {
            provider: provider.to_string(),
            model: model.to_string(),
        });
    }
    targets
}

fn correct_with_race(
    raw_text: &str,
    llm: &LlmConfig,
    correction: &CorrectionConfig,
    targets: Vec<RaceModelTarget>,
) -> std::result::Result<CorrectionOutcome, CorrectionFailure> {
    let worker_count = targets.len();
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut handles = Vec::with_capacity(worker_count);

    for target in targets {
        let sender = sender.clone();
        let raw_text = raw_text.to_string();
        let llm = llm_for_race_target(llm, &target);
        let correction = correction.clone();
        handles.push(std::thread::spawn(move || {
            let result = OpenAiCompatibleCorrector.correct(&raw_text, &llm, &correction);
            let _ = sender.send((target, result));
        }));
    }
    drop(sender);

    let mut errors = Vec::new();
    let mut logs = Vec::new();
    for _ in 0..worker_count {
        let (target, result) = receiver.recv().map_err(|_| CorrectionFailure {
            message: "LLM race correction workers exited before returning".to_string(),
            logs: logs.clone(),
        })?;
        match result {
            Ok(output) => {
                logs.push(output.log.clone());
                return Ok(CorrectionOutcome {
                    text: output.text,
                    logs,
                });
            }
            Err(err) => {
                if let Some(log) = err.log {
                    logs.push(*log);
                }
                errors.push(format!(
                    "{} / {}: {}",
                    target.provider, target.model, err.message
                ));
            }
        }
    }
    drop(handles);

    Err(CorrectionFailure {
        message: format!("LLM race correction failed: {}", errors.join(" | ")),
        logs,
    })
}

fn llm_for_race_target(base: &LlmConfig, target: &RaceModelTarget) -> LlmConfig {
    let mut llm = base.clone();
    llm.provider = target.provider.clone();
    llm.model = target.model.clone();
    if target.provider == base.provider {
        return llm;
    }

    if let Some(settings) = base
        .provider_settings
        .iter()
        .find(|settings| settings.provider == target.provider)
    {
        llm.endpoint = settings.endpoint.clone();
        llm.api_format = settings.api_format.clone();
        llm.api_key = settings.api_key.clone();
        return llm;
    }

    if let Some((endpoint, api_format)) = provider_defaults(&target.provider) {
        llm.endpoint = endpoint.to_string();
        llm.api_format = api_format.to_string();
    }
    llm.api_key.clear();
    llm
}

fn provider_defaults(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        "openai" => Some(("https://api.openai.com/v1", "responses")),
        "volc_ark" => Some(("https://ark.cn-beijing.volces.com/api/v3", "responses")),
        "custom" => Some(("", "chat_completions")),
        _ => None,
    }
}

fn append_failed_history(
    recorded: RecordedAudio,
    workflow_error: String,
    total_started_at: Instant,
    retention: &crate::config::RetentionConfig,
    live_asr_diagnostics: Option<LiveAsrDiagnostics>,
) -> Result<()> {
    let record = HistoryRecord {
        id: recorded.id,
        created_at: Utc::now(),
        audio_path: Some(recorded.path),
        asr_provider: "volcengine".to_string(),
        asr_task_id: None,
        audio_started_at: recorded.started_at,
        audio_finished_at: recorded.finished_at,
        audio_sample_rate: recorded.sample_rate,
        audio_channels: recorded.channels,
        audio_sample_count: recorded.sample_count,
        raw_text: String::new(),
        corrected_text: String::new(),
        pasted_text: String::new(),
        correction_enabled: false,
        correction_error: None,
        correction_logs: Vec::new(),
        injection_error: None,
        workflow_error: Some(workflow_error),
        asr_duration_ms: None,
        service_audio_duration_ms: None,
        live_asr_diagnostics,
        total_duration_ms: total_started_at.elapsed().as_millis(),
    };
    HistoryStore::append(&record, retention)
}

fn set_status_for_task(app: &AppHandle, task_id: &str, status: WorkflowStatus) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    let Ok(mut runtime) = state.runtime.lock() else {
        return false;
    };
    let Some(status) = runtime.update_task_status(task_id, status) else {
        return false;
    };
    drop(runtime);
    publish_status(app, &status);
    true
}

fn publish_status(app: &AppHandle, status: &WorkflowStatus) {
    if !status_is_current(app, status.revision) {
        return;
    }
    crate::windows::sync_overlay_window(app, status);
    if !status_is_current(app, status.revision) {
        return;
    }
    if let Err(err) = crate::tray::sync_voice_input_label(app, status) {
        eprintln!("failed to sync tray voice input item: {err}");
    }
    if !status_is_current(app, status.revision) {
        return;
    }
    if let Err(err) = app.emit("workflow://status", status) {
        eprintln!("failed to emit workflow status: {err}");
    }
}

fn status_is_current(app: &AppHandle, revision: u64) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return true;
    };
    state
        .runtime
        .lock()
        .map(|runtime| runtime.status.revision == revision)
        .unwrap_or(false)
}

// Keep the measured DJI Mic Mini idle floor hidden while allowing low-output
// receivers such as Wireless Mic Rx to show quiet speech. The response curve
// is purely instantaneous: it does not retain prior samples or smooth changes
// over time.
const AUDIO_LEVEL_FLOOR_DBFS: f32 = -42.5;
const AUDIO_LEVEL_CEILING_DBFS: f32 = 0.0;
const AUDIO_LEVEL_RESPONSE_EXPONENT: f32 = 0.7;

fn dbfs_to_display_level(dbfs: Option<f32>) -> f32 {
    let Some(dbfs) = dbfs.filter(|level| level.is_finite()) else {
        return 0.0;
    };
    let normalized = ((dbfs - AUDIO_LEVEL_FLOOR_DBFS)
        / (AUDIO_LEVEL_CEILING_DBFS - AUDIO_LEVEL_FLOOR_DBFS))
        .clamp(0.0, 1.0);
    normalized.powf(AUDIO_LEVEL_RESPONSE_EXPONENT)
}

fn audio_level_session_revision(runtime: &WorkflowRuntime, task_id: &str) -> Option<u64> {
    (runtime.recording
        && runtime.status.mode == WorkflowMode::Recording
        && runtime.active_task_id.as_deref() == Some(task_id)
        && runtime.audio_level_meter.is_some())
    .then_some(runtime.status.revision)
}

fn current_audio_level_session_revision(app: &AppHandle, task_id: &str) -> Option<u64> {
    let state = app.try_state::<AppState>()?;
    state
        .runtime
        .lock()
        .ok()
        .and_then(|runtime| audio_level_session_revision(&runtime, task_id))
}

fn spawn_audio_level_emitter(app: AppHandle, meter: AudioLevelMeter, task_id: String) {
    std::thread::spawn(move || {
        while meter.is_active() {
            if current_audio_level_session_revision(&app, &task_id).is_none() {
                break;
            }
            // Each event represents the most recent 50 ms measurement window.
            // Do not add attack, release, hold, or other temporal smoothing.
            let level = dbfs_to_display_level(meter.take_level());
            // The frontend listener uses Tauri's global `Any` target. A targeted
            // `emit_to("overlay", ...)` does not reach that listener in Tauri 2.
            let Some(recording_revision) = current_audio_level_session_revision(&app, &task_id)
            else {
                break;
            };
            let _ = app.emit(
                "audio://level",
                AudioLevelSample {
                    level,
                    recording_revision,
                },
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Some(recording_revision) = current_audio_level_session_revision(&app, &task_id) {
            let _ = app.emit(
                "audio://level",
                AudioLevelSample {
                    level: 0.0,
                    recording_revision,
                },
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_dbfs_meter_makes_quiet_wireless_mic_speech_visible() {
        for (dbfs, expected) in [
            (-60.0, 0.0),
            (-42.5, 0.0),
            (-40.0, 0.137),
            (-36.0, 0.269),
            (-31.5, 0.388),
            (-24.0, 0.559),
            (-18.0, 0.680),
            (-12.0, 0.793),
            (-6.0, 0.899),
            (0.0, 1.0),
        ] {
            let actual = dbfs_to_display_level(Some(dbfs));
            assert!(
                (actual - expected).abs() < 0.003,
                "{dbfs} dBFS mapped to {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn fixed_dbfs_meter_is_finite_monotonic_and_only_full_at_zero() {
        assert_eq!(dbfs_to_display_level(None), 0.0);
        assert_eq!(dbfs_to_display_level(Some(f32::NAN)), 0.0);
        assert_eq!(dbfs_to_display_level(Some(-96.0)), 0.0);
        assert!(dbfs_to_display_level(Some(-1.0)) < 1.0);
        assert_eq!(dbfs_to_display_level(Some(0.0)), 1.0);

        let mut previous = 0.0;
        for dbfs in -60..=0 {
            let level = dbfs_to_display_level(Some(dbfs as f32));
            assert!(level.is_finite());
            assert!(level >= previous);
            previous = level;
        }
    }

    #[test]
    fn fixed_downward_expander_keeps_the_measured_microphone_floor_at_zero() {
        // The measured DJI Mic Mini idle floor is about -46 dBFS. It must not
        // slowly appear after the input stream's initial all-zero callbacks.
        for dbfs in [None, Some(-96.0), Some(-46.0), Some(-42.5)]
            .into_iter()
            .cycle()
            .take(40)
        {
            assert_eq!(dbfs_to_display_level(dbfs), 0.0);
        }
    }

    #[test]
    fn meter_uses_each_measurement_directly_without_attack_or_release() {
        let levels = [Some(-18.0), Some(-40.0), Some(-12.0), None].map(dbfs_to_display_level);

        assert!((levels[0] - 0.680).abs() < 0.003);
        assert!((levels[1] - 0.137).abs() < 0.003);
        assert!((levels[2] - 0.793).abs() < 0.003);
        assert_eq!(levels[3], 0.0);
    }

    #[test]
    fn inactive_workflow_allows_guarded_action() {
        let state = AppState::default();

        assert_eq!(state.run_while_inactive(|| Ok(42)).unwrap(), 42);
    }

    #[test]
    fn active_workflow_rejects_guarded_action() {
        for (starting, recording, processing) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let state = AppState::default();
            {
                let mut runtime = state.runtime.lock().unwrap();
                runtime.starting = starting;
                runtime.recording = recording;
                runtime.processing = processing;
            }
            let called = std::cell::Cell::new(false);

            let result = state.run_while_inactive(|| {
                called.set(true);
                Ok(())
            });

            assert!(result.is_err());
            assert!(!called.get());
        }
    }

    #[test]
    fn history_retry_guard_blocks_mutations_and_parallel_retries_until_drop() {
        let state = AppState::default();
        let guard = state.begin_history_retry("record-1").unwrap();

        assert!(state.run_while_inactive(|| Ok(())).is_err());
        assert!(state.begin_history_retry("record-2").is_err());

        drop(guard);
        assert!(state.run_while_inactive(|| Ok(())).is_ok());
        assert!(state.begin_history_retry("record-2").is_ok());
    }

    #[test]
    fn idle_toggle_enters_starting_before_device_initialization() {
        let mut runtime = WorkflowRuntime::default();

        let plan = prepare_toggle(&mut runtime).unwrap();

        assert_eq!(plan.status.mode, WorkflowMode::Starting);
        assert_eq!(plan.status.revision, 1);
        assert!(runtime.starting);
        assert!(!runtime.recording);
        assert!(runtime.active_task_id.is_some());
        assert!(matches!(plan.action, ToggleAction::Start(_)));
    }

    #[test]
    fn toggle_is_ignored_without_changing_status_during_history_retry() {
        let mut runtime = WorkflowRuntime {
            history_retry_id: Some("record-1".to_string()),
            ..Default::default()
        };

        let plan = prepare_toggle(&mut runtime).unwrap();

        assert!(matches!(plan.action, ToggleAction::Busy));
        assert_eq!(plan.status.mode, WorkflowMode::Idle);
        assert_eq!(plan.status.revision, 0);
        assert!(!runtime.starting);
        assert!(runtime.active_task_id.is_none());
    }

    #[test]
    fn repeated_toggle_while_starting_cannot_stop_the_pending_recording() {
        let mut runtime = WorkflowRuntime::default();
        let first = prepare_toggle(&mut runtime).unwrap();
        let task_id = runtime.active_task_id.clone();

        let second = prepare_toggle(&mut runtime).unwrap();

        assert!(matches!(first.action, ToggleAction::Start(_)));
        assert!(matches!(second.action, ToggleAction::Busy));
        assert_eq!(second.status.mode, WorkflowMode::Starting);
        assert_eq!(second.status.revision, 2);
        assert_eq!(runtime.active_task_id, task_id);
        assert!(runtime.starting);
        assert!(!runtime.recording);
        assert!(!runtime.processing);
    }

    #[test]
    fn toggle_while_processing_preserves_the_current_progress_stage() {
        let mut runtime = WorkflowRuntime {
            processing: true,
            status: WorkflowStatus {
                mode: WorkflowMode::Processing,
                stage: WorkflowStage::FileAsrFallback,
                message: "正在回退到录音文件识别".to_string(),
                current_audio_path: Some("recording.wav".to_string()),
                last_record_id: None,
                revision: 7,
            },
            ..Default::default()
        };

        let plan = prepare_toggle(&mut runtime).unwrap();

        assert!(matches!(plan.action, ToggleAction::Busy));
        assert_eq!(plan.status.mode, WorkflowMode::Processing);
        assert_eq!(plan.status.stage, WorkflowStage::FileAsrFallback);
        assert_eq!(plan.status.message, "正在回退到录音文件识别");
        assert_eq!(plan.status.revision, 7);
    }

    #[test]
    fn only_recording_toggle_creates_a_stop_action() {
        let token = Arc::new(AtomicBool::new(false));
        let mut runtime = WorkflowRuntime {
            recording: true,
            active_task_id: Some("task-1".to_string()),
            cancel_token: Some(token),
            ..Default::default()
        };

        let plan = prepare_toggle(&mut runtime).unwrap();

        assert!(matches!(plan.action, ToggleAction::Stop(_)));
        assert_eq!(plan.status.mode, WorkflowMode::Processing);
        assert!(!runtime.recording);
        assert!(runtime.processing);
    }

    #[test]
    fn vad_activation_keeps_the_recording_stoppable() {
        let token = Arc::new(AtomicBool::new(false));
        let meter = AudioLevelMeter::new();
        let mut runtime = WorkflowRuntime {
            recording: true,
            active_task_id: Some("task-1".to_string()),
            cancel_token: Some(token),
            audio_level_meter: Some(meter.clone()),
            ..Default::default()
        };

        let waiting = runtime
            .update_task_status(
                "task-1",
                WorkflowStatus {
                    mode: WorkflowMode::Recording,
                    stage: WorkflowStage::WaitingForSpeech,
                    message: "等待说话".to_string(),
                    current_audio_path: None,
                    last_record_id: None,
                    revision: 0,
                },
            )
            .unwrap();
        assert_eq!(
            audio_level_session_revision(&runtime, "task-1"),
            Some(waiting.revision)
        );

        let activated = runtime
            .update_task_status(
                "task-1",
                WorkflowStatus {
                    mode: WorkflowMode::Recording,
                    stage: WorkflowStage::Recording,
                    message: "正在录音，再次按快捷键停止".to_string(),
                    current_audio_path: None,
                    last_record_id: None,
                    revision: 0,
                },
            )
            .unwrap();

        assert!(runtime.recording);
        assert!(!runtime.processing);
        assert!(meter.is_active());
        assert!(activated.revision > waiting.revision);
        assert_eq!(
            audio_level_session_revision(&runtime, "task-1"),
            Some(activated.revision)
        );
        let plan = prepare_toggle(&mut runtime).unwrap();
        assert!(matches!(plan.action, ToggleAction::Stop(_)));
    }

    #[test]
    fn active_race_targets_require_enabled_and_are_deduplicated() {
        let mut llm = AppConfig::default().llm;
        llm.race_models = vec![
            " gpt-5.4-mini ".to_string(),
            "gpt-5.4-mini".to_string(),
            String::new(),
            "gpt-5.4".to_string(),
        ];

        assert!(active_race_targets(&llm).is_empty());

        llm.race_enabled = true;

        assert_eq!(
            active_race_targets(&llm),
            vec![
                RaceModelTarget {
                    provider: "openai".to_string(),
                    model: "gpt-5.4-mini".to_string(),
                },
                RaceModelTarget {
                    provider: "openai".to_string(),
                    model: "gpt-5.4".to_string(),
                },
            ]
        );
    }

    #[test]
    fn disabled_correction_skips_race_and_returns_raw_text() {
        let mut config = AppConfig::default();
        config.correction.enabled = false;
        config.llm.race_enabled = true;
        config.llm.race_models = vec!["gpt-5.4-mini".to_string(), "gpt-5.4".to_string()];

        let (text, error, logs) = correct_recording_text("raw text", &config);

        assert_eq!(text, "raw text");
        assert!(error.is_none());
        assert!(logs.is_empty());
    }

    #[test]
    fn retried_record_replaces_failed_text_without_pasting() {
        let now = Utc::now();
        let original = HistoryRecord {
            id: "record-1".to_string(),
            created_at: now,
            audio_path: Some(std::path::PathBuf::from("/recordings/record-1.wav")),
            asr_provider: "volcengine".to_string(),
            asr_task_id: None,
            audio_started_at: now,
            audio_finished_at: now,
            audio_sample_rate: 16_000,
            audio_channels: 1,
            audio_sample_count: 16_000,
            raw_text: String::new(),
            corrected_text: String::new(),
            pasted_text: "stale paste".to_string(),
            correction_enabled: false,
            correction_error: Some("stale correction error".to_string()),
            correction_logs: Vec::new(),
            injection_error: Some("stale injection error".to_string()),
            workflow_error: Some("Failed to connect ASR".to_string()),
            asr_duration_ms: None,
            service_audio_duration_ms: None,
            live_asr_diagnostics: Some(LiveAsrDiagnostics {
                connection_attempts: 3,
                fallback_reason: Some("connection_attempts_exhausted".to_string()),
                ..Default::default()
            }),
            total_duration_ms: 30,
        };
        let mut config = AppConfig::default();
        config.correction.enabled = false;
        let output = AsrOutput {
            text: "  retry transcript  ".to_string(),
            provider: "volcengine_ws_file".to_string(),
            task_id: Some("task-2".to_string()),
            duration_ms: Some(1_000),
        };

        let updated = build_retried_record(original, output, &config, 250).unwrap();

        assert_eq!(updated.created_at, now);
        assert_eq!(updated.raw_text, "retry transcript");
        assert_eq!(updated.corrected_text, "retry transcript");
        assert!(updated.pasted_text.is_empty());
        assert!(updated.workflow_error.is_none());
        assert!(updated.correction_error.is_none());
        assert!(updated.injection_error.is_none());
        assert_eq!(updated.asr_provider, "volcengine_ws_file");
        assert_eq!(updated.asr_task_id.as_deref(), Some("task-2"));
        assert_eq!(updated.asr_duration_ms, Some(250));
        assert_eq!(updated.service_audio_duration_ms, Some(1_000));
        assert_eq!(
            updated
                .live_asr_diagnostics
                .as_ref()
                .map(|diagnostics| diagnostics.connection_attempts),
            Some(3)
        );
    }

    #[test]
    fn race_correction_reports_each_failed_model() {
        let mut config = AppConfig::default();
        config.llm.race_enabled = true;
        config.llm.race_models = vec!["model-a".to_string(), "model-b".to_string()];

        let error = correct_with_config("raw text", &config.llm, &config.correction)
            .unwrap_err()
            .message;

        assert!(error.contains("openai / model-a: LLM api_key is required"));
        assert!(error.contains("openai / model-b: LLM api_key is required"));
    }
}
