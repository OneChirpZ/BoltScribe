use crate::asr::{AsrOutput, AsrProvider, VolcengineFileAsr, VolcengineLiveAsrSession};
use crate::config::{AppConfig, ConfigStore, CorrectionConfig, LlmConfig, RaceModelTarget};
use crate::corrector::{LlmCallLog, LlmProvider, OpenAiCompatibleCorrector};
use crate::history::{self, HistoryRecord, HistoryStore};
use crate::injector;
use crate::output_volume::{self, OutputVolumeDuckingSession};
use crate::paths;
use crate::recorder::{AudioLevelMeter, PendingRecordingStop, RecordedAudio, RecorderController};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::Serialize;
use std::collections::VecDeque;
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
}

#[derive(Default)]
struct WorkflowRuntime {
    starting: bool,
    recording: bool,
    processing: bool,
    status: WorkflowStatus,
    config: Option<AppConfig>,
    live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
    volume_ducking: Option<OutputVolumeDuckingSession>,
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
    volume_ducking: Option<OutputVolumeDuckingSession>,
    pending_stop: Option<PendingRecordingStop>,
}

enum ToggleAction {
    Busy,
    Start(WorkflowTask),
    Stop(Box<StopContext>),
}

struct TogglePlan {
    status: WorkflowStatus,
    action: ToggleAction,
}

impl WorkflowTask {
    fn cancelled(&self) -> bool {
        self.cancel_token.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct WorkflowStatus {
    pub mode: WorkflowMode,
    pub message: String,
    pub current_audio_path: Option<String>,
    pub last_record_id: Option<String>,
    pub revision: u64,
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

impl WorkflowRuntime {
    fn update_status(&mut self, mut status: WorkflowStatus) -> WorkflowStatus {
        status.revision = self.status.revision.saturating_add(1);
        self.status = status.clone();
        status
    }
}

impl AppState {
    pub fn status(&self) -> WorkflowStatus {
        self.runtime
            .lock()
            .map(|runtime| runtime.status.clone())
            .unwrap_or_else(|_| WorkflowStatus {
                mode: WorkflowMode::Error,
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
        if runtime.starting || runtime.recording || runtime.processing {
            return Err(anyhow!(
                "Cannot modify local data while recording or processing"
            ));
        }
        let result = action();
        drop(runtime);
        result
    }
}

pub fn toggle_recording_from_app(app: AppHandle) -> Result<WorkflowStatus> {
    let state = app.state::<AppState>();
    toggle_recording(app.clone(), state.inner())
}

pub fn toggle_recording(app: AppHandle, state: &AppState) -> Result<WorkflowStatus> {
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
                            message: message.clone(),
                            current_audio_path: None,
                            last_record_id: None,
                            revision: 0,
                        },
                        true,
                    );
                    std::thread::sleep(Duration::from_millis(700));
                    set_status_for_task(
                        &app,
                        &task_for_error.id,
                        WorkflowStatus {
                            mode: WorkflowMode::Error,
                            message,
                            current_audio_path: None,
                            last_record_id: None,
                            revision: 0,
                        },
                        false,
                    );
                }
            });
        }
    }

    Ok(plan.status)
}

fn prepare_toggle(runtime: &mut WorkflowRuntime) -> Result<TogglePlan> {
    if runtime.starting {
        let status = runtime.update_status(WorkflowStatus {
            mode: WorkflowMode::Starting,
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
        let status = runtime.update_status(WorkflowStatus {
            mode: WorkflowMode::Processing,
            message: "上一段内容仍在处理中，请稍候".to_string(),
            current_audio_path: runtime.status.current_audio_path.clone(),
            last_record_id: runtime.status.last_record_id.clone(),
            revision: 0,
        });
        return Ok(TogglePlan {
            status,
            action: ToggleAction::Busy,
        });
    }

    if runtime.recording {
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
            volume_ducking: runtime.volume_ducking.take(),
            pending_stop: None,
        };
        runtime.recording = false;
        runtime.processing = true;
        let status = runtime.update_status(WorkflowStatus {
            mode: WorkflowMode::Processing,
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

    let dispatch_result = paths::recordings_dir().and_then(|path| recorder.begin_stop(path));
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
            runtime.volume_ducking = None;
            runtime.active_task_id = None;
            runtime.cancel_token = None;
            let last_record_id = runtime.status.last_record_id.clone();
            let cleanup_suffix = cleanup_error
                .map(|cleanup_err| format!("；清理录音器失败：{cleanup_err:#}"))
                .unwrap_or_default();
            let status = runtime.update_status(WorkflowStatus {
                mode: WorkflowMode::Error,
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

    let live_asr_started_at = Instant::now();
    let (live_asr, live_asr_start_error) = match VolcengineLiveAsrSession::start(&config.asr) {
        Ok(session) => (Some(session), None),
        Err(err) => (None, Some(err.to_string())),
    };
    log_timing(
        "live ASR session start",
        live_asr_started_at,
        total_started_at,
    );
    let audio_sink = live_asr
        .as_ref()
        .and_then(|session| session.audio_sender().ok());
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
        runtime.volume_ducking = volume_ducking.take();
        let last_record_id = runtime.status.last_record_id.clone();
        Some(runtime.update_status(WorkflowStatus {
            mode: WorkflowMode::Recording,
            message: "正在录音，再次按快捷键停止".to_string(),
            current_audio_path: None,
            last_record_id,
            revision: 0,
        }))
    });

    if let Some(status) = update {
        spawn_audio_level_emitter(app.clone(), audio_level_meter);
        publish_status(&app, &status);
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
    runtime.volume_ducking = None;
    runtime.active_task_id = None;
    runtime.cancel_token = None;
    let last_record_id = runtime.status.last_record_id.clone();
    let status = runtime.update_status(WorkflowStatus {
        mode: WorkflowMode::Error,
        message: format!("录音启动失败：{error:#}"),
        current_audio_path: None,
        last_record_id,
        revision: 0,
    });
    drop(runtime);
    publish_status(app, &status);
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
                message: "就绪".to_string(),
                current_audio_path,
                last_record_id,
                revision: 0,
            })
        } else {
            if let Some(token) = &runtime.cancel_token {
                token.store(true, Ordering::SeqCst);
            }

            let cancel_error = state.recorder.cancel().err();
            restore_output_volume(runtime.volume_ducking.take());
            runtime.starting = false;
            runtime.recording = false;
            runtime.processing = false;
            runtime.config = None;
            runtime.live_asr = None;
            runtime.live_asr_start_error = None;
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
                message: cancel_error
                    .map(|err| format!("取消录音失败：{err:#}"))
                    .unwrap_or_else(|| "已取消本次转写".to_string()),
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

fn stop_and_process_recording(
    app: AppHandle,
    task: WorkflowTask,
    pending_stop: PendingRecordingStop,
    config: Option<AppConfig>,
    live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
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
    log_timing("recorder.stop", recorder_stop_started_at, total_started_at);
    if task.cancelled() {
        return Ok(());
    }
    let process_result = process_recording(
        app.clone(),
        &task,
        recorded.clone(),
        total_started_at,
        config,
        live_asr,
        live_asr_start_error,
    );
    if task.cancelled() {
        return Ok(());
    }

    match process_result {
        Ok(()) => Ok(()),
        Err(err) if history::is_empty_asr_text_error(&err.to_string()) => {
            set_status_for_task(
                &app,
                &task.id,
                WorkflowStatus {
                    mode: WorkflowMode::Idle,
                    message: "未检测到语音，已忽略本次转写".to_string(),
                    current_audio_path: Some(recorded.path.display().to_string()),
                    last_record_id: None,
                    revision: 0,
                },
                false,
            );
            Ok(())
        }
        Err(err) => {
            if append_failed_history(recorded, err.to_string(), total_started_at, &retention)
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

fn process_recording(
    app: AppHandle,
    task: &WorkflowTask,
    recorded: RecordedAudio,
    total_started_at: Instant,
    config: Option<AppConfig>,
    live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
) -> Result<()> {
    let audio_path = recorded.path.clone();
    if task.cancelled() {
        return Ok(());
    }
    if !set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            message: "录音已保存，正在调用语音识别".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
        true,
    ) {
        return Ok(());
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
    )?;
    log_timing("ASR", asr_started_at, total_started_at);
    eprintln!(
        "[Timing] ASR provider={}, service_audio_duration_ms={:?}",
        asr_output.provider, asr_output.duration_ms
    );
    if task.cancelled() {
        return Ok(());
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
            message: "语音识别完成，正在纠错".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
        true,
    ) {
        return Ok(());
    }

    let correction_started_at = Instant::now();
    let (corrected_text, correction_error, correction_logs) =
        correct_recording_text(&raw_text, &config);
    log_timing("AI correction", correction_started_at, total_started_at);
    let pasted_text = corrected_text.clone();
    if task.cancelled() {
        return Ok(());
    }

    if !set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            message: "正在粘贴文本".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
        true,
    ) {
        return Ok(());
    }

    let injection_started_at = Instant::now();
    let injection_error = injector::paste_text(&pasted_text)
        .err()
        .map(|err| err.to_string());
    log_timing("text injection", injection_started_at, total_started_at);
    if task.cancelled() {
        return Ok(());
    }
    if injection_error.is_none() {
        if !set_status_for_task(
            &app,
            &task.id,
            WorkflowStatus {
                mode: WorkflowMode::Processing,
                message: "粘贴完成".to_string(),
                current_audio_path: Some(audio_path.display().to_string()),
                last_record_id: None,
                revision: 0,
            },
            true,
        ) {
            return Ok(());
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
        total_duration_ms: total_started_at.elapsed().as_millis(),
    };
    HistoryStore::append(&record, &config.retention)?;

    set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Idle,
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
        false,
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
) -> Result<AsrOutput> {
    if let Some(session) = live_asr {
        match session.finish() {
            Ok(output) => return Ok(output),
            Err(err) => {
                set_status_for_task(
                    app,
                    task_id,
                    WorkflowStatus {
                        mode: WorkflowMode::Processing,
                        message: "实时识别失败，正在使用录音文件重试".to_string(),
                        current_audio_path: Some(recorded.path.display().to_string()),
                        last_record_id: None,
                        revision: 0,
                    },
                    true,
                );
                eprintln!("live ASR failed, falling back to file ASR: {err}");
            }
        }
    } else if let Some(err) = live_asr_start_error {
        eprintln!("live ASR did not start, falling back to file ASR: {err}");
    }

    let mut output = VolcengineFileAsr.transcribe(&recorded.path, &config.asr)?;
    output.provider = "volcengine_ws_file_fallback".to_string();
    Ok(output)
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
        total_duration_ms: total_started_at.elapsed().as_millis(),
    };
    HistoryStore::append(&record, retention)
}

fn set_status_for_task(
    app: &AppHandle,
    task_id: &str,
    status: WorkflowStatus,
    keep_processing: bool,
) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    let Ok(mut runtime) = state.runtime.lock() else {
        return false;
    };
    if runtime.active_task_id.as_deref() != Some(task_id) {
        return false;
    }

    runtime.starting = false;
    runtime.processing = keep_processing;
    if status.mode == WorkflowMode::Idle || status.mode == WorkflowMode::Error {
        runtime.recording = false;
        runtime.processing = false;
        runtime.active_task_id = None;
        runtime.cancel_token = None;
    }
    let status = runtime.update_status(status);
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

const AUDIO_LEVEL_BASELINE_SAMPLES: usize = 10;
const AUDIO_LEVEL_HISTORY_SAMPLES: usize = 40;

#[derive(Default)]
struct AdaptiveAudioLevel {
    dbfs_history: VecDeque<f32>,
}

impl AdaptiveAudioLevel {
    fn normalize(&mut self, dbfs: Option<f32>) -> f32 {
        let Some(dbfs) = dbfs.filter(|level| *level >= -90.0) else {
            return 0.0;
        };

        self.dbfs_history.push_back(dbfs);
        if self.dbfs_history.len() > AUDIO_LEVEL_HISTORY_SAMPLES {
            self.dbfs_history.pop_front();
        }
        if self.dbfs_history.len() < AUDIO_LEVEL_BASELINE_SAMPLES {
            return 0.0;
        }

        let mut sorted = self.dbfs_history.iter().copied().collect::<Vec<_>>();
        sorted.sort_by(f32::total_cmp);
        let noise_floor_index = ((sorted.len() - 1) as f32 * 0.2).round() as usize;
        let noise_floor = sorted[noise_floor_index];
        ((dbfs - noise_floor - 6.0) / 24.0)
            .clamp(0.0, 1.0)
            .powf(0.8)
    }
}

fn spawn_audio_level_emitter(app: AppHandle, meter: AudioLevelMeter) {
    std::thread::spawn(move || {
        let mut normalizer = AdaptiveAudioLevel::default();
        let mut smoothed = 0.0_f32;
        while meter.is_active() {
            let level = normalizer.normalize(meter.take_level());
            let response = if level >= smoothed { 0.72 } else { 0.24 };
            smoothed += (level - smoothed) * response;
            // The frontend listener uses Tauri's global `Any` target. A targeted
            // `emit_to("overlay", ...)` does not reach that listener in Tauri 2.
            let _ = app.emit("audio://level", smoothed.clamp(0.0, 1.0));
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = app.emit("audio://level", 0.0_f32);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_audio_level_suppresses_device_noise_and_preserves_speech() {
        let mut built_in = AdaptiveAudioLevel::default();
        for _ in 0..AUDIO_LEVEL_BASELINE_SAMPLES {
            built_in.normalize(Some(-42.0));
        }
        assert_eq!(built_in.normalize(Some(-38.0)), 0.0);
        assert!(built_in.normalize(Some(-20.0)) > 0.7);

        let mut external = AdaptiveAudioLevel::default();
        for _ in 0..AUDIO_LEVEL_BASELINE_SAMPLES {
            external.normalize(Some(-64.0));
        }
        assert_eq!(external.normalize(Some(-60.0)), 0.0);
        assert!(external.normalize(Some(-38.0)) > 0.8);
    }

    #[test]
    fn adaptive_audio_level_ignores_missing_and_digital_silence_samples() {
        let mut level = AdaptiveAudioLevel::default();

        assert_eq!(level.normalize(None), 0.0);
        assert_eq!(level.normalize(Some(-96.0)), 0.0);
        assert!(level.dbfs_history.is_empty());
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
    fn toggle_while_processing_returns_visible_busy_feedback() {
        let mut runtime = WorkflowRuntime {
            processing: true,
            ..Default::default()
        };

        let plan = prepare_toggle(&mut runtime).unwrap();

        assert!(matches!(plan.action, ToggleAction::Busy));
        assert_eq!(plan.status.mode, WorkflowMode::Processing);
        assert_eq!(plan.status.message, "上一段内容仍在处理中，请稍候");
        assert_eq!(plan.status.revision, 1);
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
