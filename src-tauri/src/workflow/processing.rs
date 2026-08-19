use super::correction::*;
use super::publish::*;
use super::state::*;
use super::*;

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
    let asr_output = match VolcengineFileAsr.transcribe(audio_path, &config.asr)? {
        TranscriptionOutcome::Text(output) => output,
        TranscriptionOutcome::NoSpeech => return Err(anyhow!("ASR returned empty text")),
    };
    let asr_duration_ms = asr_started_at.elapsed().as_millis() as u64;
    let mut updated = build_retried_record(record, asr_output, &config, asr_duration_ms)?;
    updated.total_duration_ms = total_started_at.elapsed().as_millis();
    HistoryStore::replace(&updated)?;
    Ok(updated)
}

pub(super) fn stop_and_process_recording(app: AppHandle, mut context: StopContext) -> Result<()> {
    let total_started_at = Instant::now();
    let retention = context
        .config
        .as_ref()
        .map(|config| config.retention.clone())
        .unwrap_or_default();
    let recorder_stop_started_at = Instant::now();
    let pending_stop = context
        .pending_stop
        .take()
        .ok_or_else(|| anyhow!("Recorder stop was not dispatched"))?;
    let recorded = pending_stop.wait();
    restore_output_volume(context.volume_ducking.take());
    let recorded = recorded?;
    let gated = context.vad_gate.take().map(|gate| gate.finish(false));
    let (live_asr, live_asr_start_error) = match gated {
        Some(result) if !result.activated => {
            recorded.discard()?;
            set_status_for_task(
                &app,
                &context.task.id,
                WorkflowStatus {
                    mode: WorkflowMode::Idle,
                    stage: WorkflowStage::Idle,
                    code: WorkflowStatusCode::NoSpeech,
                    detail: None,
                    message: "未检测到语音，已忽略本次转写".to_string(),
                    current_audio_path: None,
                    last_record_id: None,
                    revision: 0,
                },
            );
            return Ok(());
        }
        Some(result) => (result.live_asr, result.live_asr_start_error),
        None => (context.live_asr.take(), context.live_asr_start_error.take()),
    };
    log_timing("recorder.stop", recorder_stop_started_at, total_started_at);
    if context.task.cancelled() {
        recorded.discard()?;
        return Ok(());
    }
    let mut live_asr_diagnostics = None;
    let process_result = process_recording(
        app.clone(),
        ProcessRecordingContext {
            task: context.task.clone(),
            recorded: recorded.clone(),
            total_started_at,
            config: context.config.take(),
            live_asr,
            live_asr_start_error,
        },
        &mut live_asr_diagnostics,
    );
    if context.task.cancelled() {
        return recorded.discard();
    }

    match process_result {
        Ok(ProcessingOutcome::Complete) => Ok(()),
        Ok(ProcessingOutcome::Cancelled) => Ok(()),
        Ok(ProcessingOutcome::NoSpeech) => {
            recorded.discard()?;
            set_status_for_task(
                &app,
                &context.task.id,
                WorkflowStatus {
                    mode: WorkflowMode::Idle,
                    stage: WorkflowStage::Idle,
                    code: WorkflowStatusCode::NoSpeech,
                    detail: None,
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

pub(super) fn log_timing(stage: &str, started_at: Instant, total_started_at: Instant) {
    eprintln!(
        "[Timing] {stage}: {}ms (total: {}ms)",
        started_at.elapsed().as_millis(),
        total_started_at.elapsed().as_millis()
    );
}

fn process_recording(
    app: AppHandle,
    context: ProcessRecordingContext,
    live_asr_diagnostics: &mut Option<LiveAsrDiagnostics>,
) -> Result<ProcessingOutcome> {
    let ProcessRecordingContext {
        task,
        recorded,
        total_started_at,
        config,
        live_asr,
        live_asr_start_error,
    } = context;
    let audio_path = recorded.path.clone();
    if task.cancelled() {
        recorded.discard()?;
        return Ok(ProcessingOutcome::Cancelled);
    }
    if !set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::Recognizing,
            code: WorkflowStatusCode::Recognizing,
            detail: None,
            message: "录音已保存，正在进行语音识别".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
    ) {
        recorded.discard()?;
        return Ok(ProcessingOutcome::Cancelled);
    }

    let config = match config {
        Some(config) => config,
        None => ConfigStore::load()?,
    };
    let asr_started_at = Instant::now();
    let asr_outcome = transcribe_live_with_delayed_file_race(
        &recorded,
        &config,
        live_asr,
        live_asr_start_error,
        live_asr_diagnostics,
        &task,
    )?;
    let asr_output = match asr_outcome {
        TranscriptionOutcome::Text(output) => output,
        TranscriptionOutcome::NoSpeech => return Ok(ProcessingOutcome::NoSpeech),
    };
    log_timing("ASR", asr_started_at, total_started_at);
    eprintln!(
        "[Timing] ASR provider={}, service_audio_duration_ms={:?}",
        asr_output.provider, asr_output.duration_ms
    );
    if task.cancelled() {
        recorded.discard()?;
        return Ok(ProcessingOutcome::Cancelled);
    }
    let asr_elapsed_ms = Some(asr_started_at.elapsed().as_millis() as u64);
    let raw_text = asr_output.text.trim().to_string();
    debug_assert!(!raw_text.is_empty());

    if !set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::Correcting,
            code: WorkflowStatusCode::Correcting,
            detail: None,
            message: "语音识别完成，正在纠错".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
    ) {
        recorded.discard()?;
        return Ok(ProcessingOutcome::Cancelled);
    }

    let correction_started_at = Instant::now();
    let (corrected_text, correction_error, correction_logs) =
        correct_recording_text(&raw_text, &config);
    log_timing("AI correction", correction_started_at, total_started_at);
    let pasted_text = corrected_text.clone();
    if task.cancelled() {
        recorded.discard()?;
        return Ok(ProcessingOutcome::Cancelled);
    }

    if !set_status_for_task(
        &app,
        &task.id,
        WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::Pasting,
            code: WorkflowStatusCode::Pasting,
            detail: None,
            message: "正在粘贴文本".to_string(),
            current_audio_path: Some(audio_path.display().to_string()),
            last_record_id: None,
            revision: 0,
        },
    ) {
        recorded.discard()?;
        return Ok(ProcessingOutcome::Cancelled);
    }

    let injection_started_at = Instant::now();
    let injection_error = injector::paste_text(&pasted_text)
        .err()
        .map(|err| err.to_string());
    log_timing("text injection", injection_started_at, total_started_at);
    if task.cancelled() {
        recorded.discard()?;
        return Ok(ProcessingOutcome::Cancelled);
    }
    if injection_error.is_none() {
        if !set_status_for_task(
            &app,
            &task.id,
            WorkflowStatus {
                mode: WorkflowMode::Processing,
                stage: WorkflowStage::Complete,
                code: WorkflowStatusCode::PasteComplete,
                detail: None,
                message: "粘贴完成".to_string(),
                current_audio_path: Some(audio_path.display().to_string()),
                last_record_id: None,
                revision: 0,
            },
        ) {
            recorded.discard()?;
            return Ok(ProcessingOutcome::Cancelled);
        }
        std::thread::sleep(std::time::Duration::from_millis(700));
        if task.cancelled() {
            recorded.discard()?;
            return Ok(ProcessingOutcome::Cancelled);
        }
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
    let (completion_code, completion_detail, completion_message) =
        if record.injection_error.is_some() {
            let (code, message) = paste_failure_status();
            (code, record.injection_error.clone(), message)
        } else if record.correction_error.is_some() {
            (
                WorkflowStatusCode::CompleteCorrectionFallback,
                record.correction_error.clone(),
                "处理完成，纠错失败，已粘贴原始转写".to_string(),
            )
        } else {
            (
                WorkflowStatusCode::CompleteCorrected,
                None,
                "处理完成，已粘贴纠错文本".to_string(),
            )
        };

    let completion_status = WorkflowStatus {
        mode: WorkflowMode::Idle,
        stage: WorkflowStage::Idle,
        code: completion_code,
        detail: completion_detail,
        message: completion_message,
        current_audio_path: Some(audio_path.display().to_string()),
        last_record_id: Some(record.id.clone()),
        revision: 0,
    };
    let commit = {
        let Some(state) = app.try_state::<AppState>() else {
            return Ok(ProcessingOutcome::Cancelled);
        };
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        runtime.commit_processing(&task, || {
            HistoryStore::append(&record, &config.retention)?;
            Ok(completion_status)
        })?
    };
    let ProcessingCommit::Committed(status) = commit else {
        recorded.discard()?;
        return Ok(ProcessingOutcome::Cancelled);
    };
    publish_status(&app, &status);
    let _ = app.emit("history://updated", ());
    eprintln!(
        "[Timing] workflow total: {}ms",
        total_started_at.elapsed().as_millis()
    );
    Ok(ProcessingOutcome::Complete)
}

fn paste_failure_status() -> (WorkflowStatusCode, String) {
    if cfg!(target_os = "windows") {
        (
            WorkflowStatusCode::PasteFailedClipboard,
            "处理完成，但粘贴失败，请检查剪贴板或当前输入位置".to_string(),
        )
    } else {
        (
            WorkflowStatusCode::PasteFailedAccessibility,
            "处理完成，但粘贴失败，请检查辅助功能权限".to_string(),
        )
    }
}

enum AsrRaceEvent {
    Live(LiveAsrResult),
    File(Result<TranscriptionOutcome>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LivePriorityEvent {
    PostStopReconnectFailed,
    EndedWithoutText,
    ProducedText,
}

pub(super) fn should_launch_file_asr(file_started: bool, event: LivePriorityEvent) -> bool {
    !file_started && event != LivePriorityEvent::ProducedText
}

fn transcribe_live_with_delayed_file_race(
    recorded: &RecordedAudio,
    config: &AppConfig,
    mut live_asr: Option<VolcengineLiveAsrSession>,
    live_asr_start_error: Option<String>,
    diagnostics: &mut Option<LiveAsrDiagnostics>,
    task: &WorkflowTask,
) -> Result<TranscriptionOutcome> {
    let live_cancellation = live_asr
        .as_ref()
        .map(VolcengineLiveAsrSession::cancellation)
        .unwrap_or_default();
    let mut file_race_trigger = live_asr
        .as_mut()
        .and_then(VolcengineLiveAsrSession::take_file_race_trigger);
    let file_cancellation = AsrCancellation::new();
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut sender = Some(sender);

    let live_handle = live_asr.map(|session| {
        let sender = sender.as_ref().expect("race sender is available").clone();
        std::thread::spawn(move || {
            let _ = sender.send(AsrRaceEvent::Live(session.finish()));
        })
    });
    let mut live_handle = live_handle;
    let mut file_handle = None;
    let mut file_started = false;
    let mut live_reported = false;
    let mut file_reported = false;
    let mut live_panicked = false;
    let mut file_panicked = false;

    let mut race = AsrRaceState::default();
    let mut terminal = None;
    let mut winner = None;
    if live_handle.is_none() {
        live_reported = true;
        let message =
            live_asr_start_error.unwrap_or_else(|| "Live ASR session was unavailable".to_string());
        *diagnostics = Some(LiveAsrDiagnostics {
            last_error_category: Some("configuration".to_string()),
            fallback_reason: Some("session_start_failed".to_string()),
            ..Default::default()
        });
        terminal = race.accept(AsrRaceBranch::Live, Err(anyhow!(message)));
        if should_launch_file_asr(file_started, LivePriorityEvent::EndedWithoutText) {
            file_started = start_file_asr_branch(
                &mut file_handle,
                &mut sender,
                recorded,
                config,
                &file_cancellation,
            );
        }
    }

    let mut cancelled = false;
    while terminal.is_none() {
        if task.cancelled() {
            cancelled = true;
            live_cancellation.cancel();
            file_cancellation.cancel();
            break;
        }

        if file_race_trigger
            .as_ref()
            .is_some_and(|trigger| trigger.try_recv().is_ok())
            && should_launch_file_asr(file_started, LivePriorityEvent::PostStopReconnectFailed)
        {
            eprintln!("live ASR post-stop retry failed; starting file ASR race");
            file_started = start_file_asr_branch(
                &mut file_handle,
                &mut sender,
                recorded,
                config,
                &file_cancellation,
            );
            file_race_trigger.take();
        }

        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(AsrRaceEvent::Live(result)) => {
                live_reported = true;
                *diagnostics = Some(result.diagnostics);
                if !cancelled {
                    let has_text = matches!(&result.output, Ok(TranscriptionOutcome::Text(_)));
                    let priority_event = if has_text {
                        LivePriorityEvent::ProducedText
                    } else {
                        LivePriorityEvent::EndedWithoutText
                    };
                    terminal = race.accept(AsrRaceBranch::Live, result.output);
                    if has_text {
                        winner = Some(AsrRaceBranch::Live);
                    }
                    if terminal.is_none() && should_launch_file_asr(file_started, priority_event) {
                        file_started = start_file_asr_branch(
                            &mut file_handle,
                            &mut sender,
                            recorded,
                            config,
                            &file_cancellation,
                        );
                        file_race_trigger.take();
                    }
                }
            }
            Ok(AsrRaceEvent::File(result)) => {
                file_reported = true;
                if !cancelled {
                    let has_text = matches!(&result, Ok(TranscriptionOutcome::Text(_)));
                    terminal = race.accept(AsrRaceBranch::File, result);
                    if has_text {
                        winner = Some(AsrRaceBranch::File);
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if terminal
            .as_ref()
            .is_some_and(|result| matches!(result, Ok(TranscriptionOutcome::Text(_))))
        {
            live_cancellation.cancel();
            file_cancellation.cancel();
            break;
        }

        if !live_reported
            && live_handle
                .as_ref()
                .is_some_and(std::thread::JoinHandle::is_finished)
        {
            let handle = live_handle.take().expect("live handle was checked");
            if handle.join().is_err() {
                live_panicked = true;
                live_reported = true;
                *diagnostics = Some(LiveAsrDiagnostics {
                    last_error_category: Some("worker_panicked".to_string()),
                    fallback_reason: Some("worker_panicked".to_string()),
                    ..Default::default()
                });
                terminal = race.accept(
                    AsrRaceBranch::Live,
                    Err(anyhow!("Live ASR worker panicked")),
                );
                if terminal.is_none()
                    && should_launch_file_asr(file_started, LivePriorityEvent::EndedWithoutText)
                {
                    file_started = start_file_asr_branch(
                        &mut file_handle,
                        &mut sender,
                        recorded,
                        config,
                        &file_cancellation,
                    );
                    file_race_trigger.take();
                }
            }
        }
        if file_started
            && !file_reported
            && file_handle
                .as_ref()
                .is_some_and(std::thread::JoinHandle::is_finished)
        {
            let handle = file_handle.take().expect("file handle was checked");
            if handle.join().is_err() {
                file_panicked = true;
                file_reported = true;
                terminal = race.accept(
                    AsrRaceBranch::File,
                    Err(anyhow!("File ASR worker panicked")),
                );
            }
        }
    }

    sender.take();
    live_cancellation.cancel();
    file_cancellation.cancel();
    live_panicked |= live_handle.is_some_and(|handle| handle.join().is_err());
    file_panicked |= file_handle.is_some_and(|handle| handle.join().is_err());
    for event in receiver.try_iter() {
        if let AsrRaceEvent::Live(result) = event {
            *diagnostics = Some(result.diagnostics);
        }
    }
    if winner == Some(AsrRaceBranch::File) {
        if let Some(diagnostics) = diagnostics.as_mut() {
            if diagnostics.fallback_reason.as_deref() == Some("cancelled") {
                diagnostics.fallback_reason = Some("parallel_file_won".to_string());
            }
        }
    }

    if task.cancelled() || cancelled {
        return Err(anyhow!("ASR operation cancelled"));
    }
    if terminal.is_none() && !live_reported && live_panicked {
        *diagnostics = Some(LiveAsrDiagnostics {
            last_error_category: Some("worker_panicked".to_string()),
            fallback_reason: Some("worker_panicked".to_string()),
            ..Default::default()
        });
        terminal = race.accept(
            AsrRaceBranch::Live,
            Err(anyhow!("Live ASR worker panicked")),
        );
    }
    if terminal.is_none() && file_started && !file_reported && file_panicked {
        terminal = race.accept(
            AsrRaceBranch::File,
            Err(anyhow!("File ASR worker panicked")),
        );
    }

    terminal.unwrap_or_else(|| {
        Err(anyhow!(
            "ASR race ended before either branch produced a result"
        ))
    })
}

fn start_file_asr_branch(
    file_handle: &mut Option<std::thread::JoinHandle<()>>,
    sender: &mut Option<std::sync::mpsc::Sender<AsrRaceEvent>>,
    recorded: &RecordedAudio,
    config: &AppConfig,
    cancellation: &AsrCancellation,
) -> bool {
    let Some(file_sender) = sender.take() else {
        return false;
    };
    let file_path = recorded.path.clone();
    let file_config = config.asr.clone();
    let file_cancellation = cancellation.clone();
    *file_handle = Some(std::thread::spawn(move || {
        let result = VolcengineFileAsr
            .transcribe_cancellable(&file_path, &file_config, &file_cancellation)
            .map(|outcome| match outcome {
                TranscriptionOutcome::Text(mut output) => {
                    output.provider = "volcengine_ws_file_fallback".to_string();
                    TranscriptionOutcome::Text(output)
                }
                TranscriptionOutcome::NoSpeech => TranscriptionOutcome::NoSpeech,
            });
        let _ = file_sender.send(AsrRaceEvent::File(result));
    }));
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AsrRaceBranch {
    Live,
    File,
}

#[derive(Default)]
pub(super) struct AsrRaceState {
    live: Option<Result<TranscriptionOutcome>>,
    file: Option<Result<TranscriptionOutcome>>,
}

impl AsrRaceState {
    pub(super) fn accept(
        &mut self,
        branch: AsrRaceBranch,
        result: Result<TranscriptionOutcome>,
    ) -> Option<Result<TranscriptionOutcome>> {
        if matches!(&result, Ok(TranscriptionOutcome::Text(_))) {
            return Some(result);
        }
        match branch {
            AsrRaceBranch::Live => self.live = Some(result),
            AsrRaceBranch::File => self.file = Some(result),
        }
        if self.live.is_none() || self.file.is_none() {
            return None;
        }
        let live = self.live.take().expect("live result was checked");
        let file = self.file.take().expect("file result was checked");
        Some(match (live, file) {
            (Ok(TranscriptionOutcome::NoSpeech), _) | (_, Ok(TranscriptionOutcome::NoSpeech)) => {
                Ok(TranscriptionOutcome::NoSpeech)
            }
            (Err(live_error), Err(file_error)) => Err(anyhow!(
                "Both live and file ASR failed; live: {live_error:#}; file: {file_error:#}"
            )),
            _ => unreachable!("text outcomes are returned immediately"),
        })
    }
}
