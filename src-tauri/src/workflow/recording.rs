use super::processing::{log_timing, stop_and_process_recording};
use super::publish::*;
use super::state::*;
use super::*;

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
            crate::learning::observation::finish();
            let recorder = state.recorder.clone();
            std::thread::spawn(move || start_recording_attempt(app, task, recorder));
        }
        ToggleAction::Stop(context) => {
            let context = *context;
            std::thread::spawn(move || {
                let task_for_error = context.task.clone();
                if let Err(err) = stop_and_process_recording(app.clone(), context) {
                    if task_for_error.cancelled() {
                        return;
                    }
                    let detail = err.to_string();
                    let message = format!("处理失败：{detail}");
                    set_status_for_task(
                        &app,
                        &task_for_error.id,
                        WorkflowStatus {
                            mode: WorkflowMode::Processing,
                            stage: WorkflowStage::Error,
                            code: WorkflowStatusCode::Failed,
                            detail: Some(detail.clone()),
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
                            code: WorkflowStatusCode::Failed,
                            detail: Some(detail),
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

pub(super) fn prepare_toggle(runtime: &mut WorkflowRuntime) -> Result<TogglePlan> {
    match &runtime.activity {
        WorkflowActivity::RetryingHistory { .. } | WorkflowActivity::Processing(_) => {
            return Ok(TogglePlan {
                status: runtime.status.clone(),
                action: ToggleAction::Busy,
            });
        }
        WorkflowActivity::Starting(_) => {
            let status = runtime.update_status(WorkflowStatus {
                mode: WorkflowMode::Starting,
                stage: WorkflowStage::Starting,
                code: WorkflowStatusCode::StartingMicrophone,
                detail: None,
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
        WorkflowActivity::Recording(session)
            if session
                .vad_gate
                .as_ref()
                .is_some_and(|gate| gate.snapshot().phase != VadPhase::Activated) =>
        {
            return Ok(TogglePlan {
                status: runtime.status.clone(),
                action: ToggleAction::CancelArmed(session.task.clone()),
            });
        }
        WorkflowActivity::Recording(_) => {
            let WorkflowActivity::Recording(session) = std::mem::take(&mut runtime.activity) else {
                unreachable!("recording activity changed while locked")
            };
            session.audio_level_meter.stop();
            let task = session.task.clone();
            let context = StopContext {
                task: session.task,
                config: Some(session.config),
                live_asr: session.live_asr,
                live_asr_start_error: session.live_asr_start_error,
                vad_gate: session.vad_gate,
                volume_ducking: session.volume_ducking,
                pending_stop: None,
            };
            runtime.activity = WorkflowActivity::Processing(task);
            let status = runtime.update_status(WorkflowStatus {
                mode: WorkflowMode::Processing,
                stage: WorkflowStage::Recognizing,
                code: WorkflowStatusCode::Stopping,
                detail: None,
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
        WorkflowActivity::Inactive => {}
    }

    let task = WorkflowTask {
        id: Uuid::new_v4().to_string(),
        cancel_token: Arc::new(AtomicBool::new(false)),
    };
    runtime.activity = WorkflowActivity::Starting(task.clone());
    let status = runtime.update_status(WorkflowStatus {
        mode: WorkflowMode::Starting,
        stage: WorkflowStage::Starting,
        code: WorkflowStatusCode::StartingMicrophone,
        detail: None,
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

            runtime.activity = WorkflowActivity::Inactive;
            let last_record_id = runtime.status.last_record_id.clone();
            let cleanup_suffix = cleanup_error
                .map(|cleanup_err| format!("；清理录音器失败：{cleanup_err:#}"))
                .unwrap_or_default();
            let status = runtime.update_status(WorkflowStatus {
                mode: WorkflowMode::Error,
                stage: WorkflowStage::Error,
                code: WorkflowStatusCode::RecordingStopFailed,
                detail: Some(format!("{err:#}{cleanup_suffix}")),
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
        if !matches!(
            &runtime.activity,
            WorkflowActivity::Starting(active) if active.id == task.id
        ) {
            return None;
        }
        let session = RecordingSession {
            task: task.clone(),
            config: config.take()?,
            live_asr: live_asr.take(),
            live_asr_start_error,
            vad_gate: vad_gate.take(),
            volume_ducking: volume_ducking.take(),
            audio_level_meter: audio_level_meter.clone(),
        };
        let waiting_for_speech = session.vad_gate.is_some();
        runtime.activity = WorkflowActivity::Recording(Box::new(session));
        let last_record_id = runtime.status.last_record_id.clone();
        Some(runtime.update_status(WorkflowStatus {
            mode: WorkflowMode::Recording,
            stage: if waiting_for_speech {
                WorkflowStage::WaitingForSpeech
            } else {
                WorkflowStage::Recording
            },
            code: if waiting_for_speech {
                WorkflowStatusCode::WaitingForSpeech
            } else {
                WorkflowStatusCode::Recording
            },
            detail: None,
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
                .and_then(|runtime| match &runtime.activity {
                    WorkflowActivity::Recording(session) => {
                        session.vad_gate.as_ref().map(VadGate::monitor_handle)
                    }
                    _ => None,
                })
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
    if !matches!(
        &runtime.activity,
        WorkflowActivity::Starting(task) if task.id == task_id
    ) {
        return;
    }

    runtime.activity = WorkflowActivity::Inactive;
    let last_record_id = runtime.status.last_record_id.clone();
    let status = runtime.update_status(WorkflowStatus {
        mode: WorkflowMode::Error,
        stage: WorkflowStage::Error,
        code: WorkflowStatusCode::RecordingStartFailed,
        detail: Some(format!("{error:#}")),
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
        matches!(&runtime.activity, WorkflowActivity::Recording(session)
            if session.task.id == task.id
            && session.vad_gate.as_ref()
                .is_some_and(|gate| gate.snapshot().phase == VadPhase::TimedOut)
        )
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
                    let active = state
                        .runtime
                        .lock()
                        .ok()
                        .is_some_and(|runtime| runtime.activity.is_recording_task(&task.id));
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
                                    code: WorkflowStatusCode::Recording,
                                    detail: None,
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
                                        runtime.activity.is_recording_task(&task.id)
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
    cancel_workflow(app, state, None)
}

pub(crate) fn cancel_recording_if_current(app: AppHandle, task_id: &str) -> Result<WorkflowStatus> {
    let state = app.state::<AppState>();
    cancel_workflow(app.clone(), state.inner(), Some(task_id))
}

fn cancel_workflow(
    app: AppHandle,
    state: &AppState,
    recording_task_id: Option<&str>,
) -> Result<WorkflowStatus> {
    let Some(status) = cancel_workflow_state(state, recording_task_id)? else {
        return Ok(state.status());
    };
    publish_status(&app, &status);
    Ok(status)
}

pub(super) fn cancel_workflow_state(
    state: &AppState,
    recording_task_id: Option<&str>,
) -> Result<Option<WorkflowStatus>> {
    let status = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        if recording_task_id.is_some_and(|task_id| !runtime.activity.is_recording_task(task_id)) {
            return Ok(None);
        }
        if matches!(&runtime.activity, WorkflowActivity::Starting(_)) {
            return Ok(None);
        }
        if !matches!(
            &runtime.activity,
            WorkflowActivity::Recording(_) | WorkflowActivity::Processing(_)
        ) {
            if runtime.status.mode != WorkflowMode::Error {
                return Ok(None);
            }
            let current_audio_path = runtime.status.current_audio_path.clone();
            let last_record_id = runtime.status.last_record_id.clone();
            runtime.update_status(WorkflowStatus {
                mode: WorkflowMode::Idle,
                stage: WorkflowStage::Idle,
                code: WorkflowStatusCode::Ready,
                detail: None,
                message: "就绪".to_string(),
                current_audio_path,
                last_record_id,
                revision: 0,
            })
        } else {
            let activity = std::mem::take(&mut runtime.activity);
            let (task, vad_timed_out, vad_gate, volume_ducking) = match activity {
                WorkflowActivity::Recording(mut session) => {
                    session.audio_level_meter.stop();
                    let vad_timed_out = session
                        .vad_gate
                        .as_ref()
                        .is_some_and(|gate| gate.snapshot().phase == VadPhase::TimedOut);
                    (
                        session.task,
                        vad_timed_out,
                        session.vad_gate.take(),
                        session.volume_ducking.take(),
                    )
                }
                WorkflowActivity::Processing(task) => (task, false, None, None),
                _ => unreachable!("active workflow changed while locked"),
            };
            task.cancel_token.store(true, Ordering::SeqCst);
            let cancel_error = state.recorder.cancel().err();
            if let Some(gate) = vad_gate {
                let _ = gate.finish(true);
            }
            restore_output_volume(volume_ducking);
            let current_audio_path = runtime.status.current_audio_path.clone();
            let last_record_id = runtime.status.last_record_id.clone();
            let (mode, stage, code, detail, message) = match cancel_error {
                Some(err) => {
                    let detail = format!("{err:#}");
                    (
                        WorkflowMode::Error,
                        WorkflowStage::Error,
                        WorkflowStatusCode::RecordingCancelFailed,
                        Some(detail.clone()),
                        format!("取消录音失败：{detail}"),
                    )
                }
                None if vad_timed_out => (
                    WorkflowMode::Idle,
                    WorkflowStage::Idle,
                    WorkflowStatusCode::NoSpeech,
                    None,
                    "未检测到语音，已自动取消".to_string(),
                ),
                None => (
                    WorkflowMode::Idle,
                    WorkflowStage::Idle,
                    WorkflowStatusCode::Cancelled,
                    None,
                    "已取消本次转写".to_string(),
                ),
            };
            runtime.update_status(WorkflowStatus {
                mode,
                stage,
                code,
                detail,
                message,
                current_audio_path,
                last_record_id,
                revision: 0,
            })
        }
    };
    Ok(Some(status))
}
