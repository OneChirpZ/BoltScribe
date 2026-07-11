use crate::asr::{AsrOutput, AsrProvider, VolcengineFileAsr, VolcengineLiveAsrSession};
use crate::config::{AppConfig, ConfigStore, CorrectionConfig, LlmConfig, RaceModelTarget};
use crate::corrector::{LlmCallLog, LlmProvider, OpenAiCompatibleCorrector};
use crate::history::{self, HistoryRecord, HistoryStore};
use crate::injector;
use crate::output_volume::{self, OutputVolumeDuckingSession};
use crate::paths;
use crate::recorder::{RecordedAudio, RecorderController};
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
}

#[derive(Default)]
struct WorkflowRuntime {
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
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowMode {
    #[default]
    Idle,
    Recording,
    Processing,
    Error,
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
            })
    }

    pub(crate) fn run_while_inactive<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        if runtime.recording || runtime.processing {
            return Err(anyhow!(
                "Cannot change the data folder while recording or processing"
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
    let should_stop = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        if runtime.processing {
            return Ok(runtime.status.clone());
        }

        if runtime.recording {
            let task_id = runtime
                .active_task_id
                .clone()
                .ok_or_else(|| anyhow!("Workflow task is missing"))?;
            let cancel_token = runtime
                .cancel_token
                .clone()
                .ok_or_else(|| anyhow!("Workflow cancel token is missing"))?;
            let config = runtime.config.take();
            let live_asr = runtime.live_asr.take();
            let live_asr_start_error = runtime.live_asr_start_error.take();
            let volume_ducking = runtime.volume_ducking.take();
            runtime.recording = false;
            runtime.processing = true;
            runtime.status = WorkflowStatus {
                mode: WorkflowMode::Processing,
                message: "正在停止录音并处理转写".to_string(),
                current_audio_path: None,
                last_record_id: runtime.status.last_record_id.clone(),
            };
            Some((
                task_id,
                cancel_token,
                config,
                live_asr,
                live_asr_start_error,
                volume_ducking,
            ))
        } else {
            let total_started_at = Instant::now();
            let config_started_at = Instant::now();
            let config = ConfigStore::load()?;
            log_timing("config load", config_started_at, total_started_at);
            let task_id = Uuid::new_v4().to_string();
            let cancel_token = Arc::new(AtomicBool::new(false));
            let live_asr_started_at = Instant::now();
            let (live_asr, live_asr_start_error) =
                match VolcengineLiveAsrSession::start(&config.asr) {
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
            if let Err(err) = state
                .recorder
                .start_with_config(audio_sink, config.audio.clone())
            {
                restore_output_volume(volume_ducking.take());
                return Err(err);
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
            runtime.recording = true;
            runtime.config = Some(config);
            runtime.live_asr = live_asr;
            runtime.live_asr_start_error = live_asr_start_error;
            runtime.volume_ducking = volume_ducking;
            runtime.active_task_id = Some(task_id);
            runtime.cancel_token = Some(cancel_token);
            runtime.status = WorkflowStatus {
                mode: WorkflowMode::Recording,
                message: "正在录音，再次按快捷键停止".to_string(),
                current_audio_path: None,
                last_record_id: runtime.status.last_record_id.clone(),
            };
            let status = runtime.status.clone();
            drop(runtime);
            emit_status(&app, &status);
            return Ok(status);
        }
    };

    let status = state.status();
    emit_status(&app, &status);

    if let Some((task_id, cancel_token, config, live_asr, live_asr_start_error, volume_ducking)) =
        should_stop
    {
        let recorder = state.recorder.clone();
        std::thread::spawn(move || {
            let task = WorkflowTask {
                id: task_id,
                cancel_token,
            };
            let task_for_error = task.clone();
            if let Err(err) = stop_and_process_recording(
                app.clone(),
                task,
                recorder,
                config,
                live_asr,
                live_asr_start_error,
                volume_ducking,
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
                    },
                    false,
                );
            }
        });
    }

    Ok(status)
}

pub fn cancel_current_workflow(app: AppHandle, state: &AppState) -> Result<WorkflowStatus> {
    let (status, should_cancel_recorder, volume_ducking) = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        if !runtime.recording && !runtime.processing {
            return Ok(runtime.status.clone());
        }

        if let Some(token) = &runtime.cancel_token {
            token.store(true, Ordering::SeqCst);
        }

        let was_recording = runtime.recording;
        runtime.recording = false;
        runtime.processing = false;
        runtime.config = None;
        runtime.live_asr = None;
        runtime.live_asr_start_error = None;
        let volume_ducking = runtime.volume_ducking.take();
        runtime.active_task_id = None;
        runtime.cancel_token = None;
        runtime.status = WorkflowStatus {
            mode: WorkflowMode::Idle,
            message: "已取消本次转写".to_string(),
            current_audio_path: runtime.status.current_audio_path.clone(),
            last_record_id: runtime.status.last_record_id.clone(),
        };
        (runtime.status.clone(), was_recording, volume_ducking)
    };
    emit_status(&app, &status);

    let cancel_result = if should_cancel_recorder {
        state.recorder.cancel()
    } else {
        Ok(())
    };
    restore_output_volume(volume_ducking);
    cancel_result?;

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
    recorder: RecorderController,
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
    let recordings_dir = match paths::recordings_dir() {
        Ok(path) => path,
        Err(err) => {
            restore_output_volume(volume_ducking);
            return Err(err);
        }
    };
    let recorder_stop_started_at = Instant::now();
    let recorded = recorder.stop(recordings_dir);
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
        audio_path,
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
            current_audio_path: Some(record.audio_path.display().to_string()),
            last_record_id: Some(record.id),
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
        audio_path: recorded.path,
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

    runtime.processing = keep_processing;
    if status.mode == WorkflowMode::Idle || status.mode == WorkflowMode::Error {
        runtime.recording = false;
        runtime.processing = false;
        runtime.active_task_id = None;
        runtime.cancel_token = None;
    }
    runtime.status = status.clone();
    drop(runtime);
    emit_status(app, &status);
    true
}

fn emit_status(app: &AppHandle, status: &WorkflowStatus) {
    crate::windows::sync_overlay_window(app, status);
    if let Err(err) = crate::tray::sync_voice_input_label(app, status) {
        eprintln!("failed to sync tray voice input item: {err}");
    }
    let _ = app.emit("workflow://status", status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_workflow_allows_guarded_action() {
        let state = AppState::default();

        assert_eq!(state.run_while_inactive(|| Ok(42)).unwrap(), 42);
    }

    #[test]
    fn active_workflow_rejects_guarded_action() {
        for (recording, processing) in [(true, false), (false, true)] {
            let state = AppState::default();
            {
                let mut runtime = state.runtime.lock().unwrap();
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
