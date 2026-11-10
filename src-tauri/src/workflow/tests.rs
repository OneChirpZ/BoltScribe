use super::correction::*;
use super::processing::*;
use super::publish::*;
use super::recording::*;
use super::state::*;
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

fn test_task(id: &str) -> WorkflowTask {
    WorkflowTask {
        id: id.to_string(),
        cancel_token: Arc::new(AtomicBool::new(false)),
    }
}

fn test_recording_session(id: &str, meter: AudioLevelMeter) -> RecordingSession {
    RecordingSession {
        task: test_task(id),
        config: AppConfig::default(),
        live_asr: None,
        live_asr_start_error: None,
        vad_gate: None,
        volume_ducking: None,
        audio_level_meter: meter,
    }
}

#[test]
fn escape_cancellation_reuses_capsule_cancellation_for_waiting_and_recording() {
    for stage in [WorkflowStage::WaitingForSpeech, WorkflowStage::Recording] {
        for recording_task_id in [Some("recording"), None] {
            let state = AppState::default();
            let meter = AudioLevelMeter::new();
            let session = test_recording_session("recording", meter.clone());
            let task = session.task.clone();
            {
                let mut runtime = state.runtime.lock().unwrap();
                runtime.activity = WorkflowActivity::Recording(Box::new(session));
                runtime.status.mode = WorkflowMode::Recording;
                runtime.status.stage = stage.clone();
            }

            assert_eq!(state.recording_task_id().as_deref(), Some("recording"));
            let status = cancel_workflow_state(&state, recording_task_id)
                .unwrap()
                .expect("active recording should be cancelled");

            assert_eq!(status.mode, WorkflowMode::Idle);
            assert_eq!(status.code, WorkflowStatusCode::Cancelled);
            assert!(task.cancelled());
            assert!(!meter.is_active());
            assert_eq!(state.recording_task_id(), None);
            assert!(!state.runtime.lock().unwrap().activity.is_active());
        }
    }
}

#[test]
fn delayed_escape_does_not_cancel_a_new_recording_or_processing_task() {
    let state = AppState::default();
    let meter = AudioLevelMeter::new();
    let session = test_recording_session("new-recording", meter.clone());
    let task = session.task.clone();
    state.runtime.lock().unwrap().activity = WorkflowActivity::Recording(Box::new(session));

    assert!(cancel_workflow_state(&state, Some("old-recording"))
        .unwrap()
        .is_none());
    assert_eq!(state.recording_task_id().as_deref(), Some("new-recording"));
    assert!(!task.cancelled());
    assert!(meter.is_active());

    state.runtime.lock().unwrap().activity = WorkflowActivity::Processing(task.clone());
    assert!(cancel_workflow_state(&state, Some("new-recording"))
        .unwrap()
        .is_none());
    assert!(!task.cancelled());
}

#[test]
fn escape_is_inactive_outside_recording() {
    for activity in [
        WorkflowActivity::Inactive,
        WorkflowActivity::Starting(test_task("starting")),
        WorkflowActivity::Processing(test_task("processing")),
        WorkflowActivity::RetryingHistory {
            record_id: "history".to_string(),
        },
    ] {
        let state = AppState::default();
        state.runtime.lock().unwrap().activity = activity;
        assert_eq!(state.recording_task_id(), None);
    }

    let state = AppState::default();
    state.runtime.lock().unwrap().status.mode = WorkflowMode::Error;
    assert!(cancel_workflow_state(&state, Some("old-recording"))
        .unwrap()
        .is_none());
    assert_eq!(state.status().mode, WorkflowMode::Error);
}

#[test]
fn active_workflow_rejects_guarded_action() {
    for activity in [
        WorkflowActivity::Starting(test_task("starting")),
        WorkflowActivity::Recording(Box::new(test_recording_session(
            "recording",
            AudioLevelMeter::new(),
        ))),
        WorkflowActivity::Processing(test_task("processing")),
    ] {
        let state = AppState::default();
        {
            let mut runtime = state.runtime.lock().unwrap();
            runtime.activity = activity;
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
    assert!(matches!(runtime.activity, WorkflowActivity::Starting(_)));
    assert!(matches!(plan.action, ToggleAction::Start(_)));
}

#[test]
fn toggle_is_ignored_without_changing_status_during_history_retry() {
    let mut runtime = WorkflowRuntime {
        activity: WorkflowActivity::RetryingHistory {
            record_id: "record-1".to_string(),
        },
        ..Default::default()
    };

    let plan = prepare_toggle(&mut runtime).unwrap();

    assert!(matches!(plan.action, ToggleAction::Busy));
    assert_eq!(plan.status.mode, WorkflowMode::Idle);
    assert_eq!(plan.status.revision, 0);
    assert!(matches!(
        runtime.activity,
        WorkflowActivity::RetryingHistory { .. }
    ));
}

#[test]
fn repeated_toggle_while_starting_cannot_stop_the_pending_recording() {
    let mut runtime = WorkflowRuntime::default();
    let first = prepare_toggle(&mut runtime).unwrap();
    let task_id = runtime.activity.task_id().map(str::to_string);

    let second = prepare_toggle(&mut runtime).unwrap();

    assert!(matches!(first.action, ToggleAction::Start(_)));
    assert!(matches!(second.action, ToggleAction::Busy));
    assert_eq!(second.status.mode, WorkflowMode::Starting);
    assert_eq!(second.status.revision, 2);
    assert_eq!(runtime.activity.task_id(), task_id.as_deref());
    assert!(matches!(runtime.activity, WorkflowActivity::Starting(_)));
}

#[test]
fn toggle_while_processing_preserves_the_current_progress_stage() {
    let mut runtime = WorkflowRuntime {
        activity: WorkflowActivity::Processing(test_task("task-1")),
        status: WorkflowStatus {
            mode: WorkflowMode::Processing,
            stage: WorkflowStage::FileAsrFallback,
            code: WorkflowStatusCode::FileAsrFallback,
            detail: None,
            message: "正在回退到录音文件识别".to_string(),
            current_audio_path: Some("recording.wav".to_string()),
            last_record_id: None,
            revision: 7,
        },
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
    let mut runtime = WorkflowRuntime {
        activity: WorkflowActivity::Recording(Box::new(test_recording_session(
            "task-1",
            AudioLevelMeter::new(),
        ))),
        ..Default::default()
    };

    let plan = prepare_toggle(&mut runtime).unwrap();

    assert!(matches!(plan.action, ToggleAction::Stop(_)));
    assert_eq!(plan.status.mode, WorkflowMode::Processing);
    assert!(matches!(runtime.activity, WorkflowActivity::Processing(_)));
}

#[test]
fn vad_activation_keeps_the_recording_stoppable() {
    let meter = AudioLevelMeter::new();
    let mut runtime = WorkflowRuntime {
        activity: WorkflowActivity::Recording(Box::new(test_recording_session(
            "task-1",
            meter.clone(),
        ))),
        ..Default::default()
    };

    let waiting = runtime
        .update_task_status(
            "task-1",
            WorkflowStatus {
                mode: WorkflowMode::Recording,
                stage: WorkflowStage::WaitingForSpeech,
                code: WorkflowStatusCode::WaitingForSpeech,
                detail: None,
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
                code: WorkflowStatusCode::Recording,
                detail: None,
                message: "正在录音，再次按快捷键停止".to_string(),
                current_audio_path: None,
                last_record_id: None,
                revision: 0,
            },
        )
        .unwrap();

    assert!(matches!(runtime.activity, WorkflowActivity::Recording(_)));
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
fn terminal_status_releases_the_processing_activity() {
    let mut runtime = WorkflowRuntime {
        activity: WorkflowActivity::Processing(test_task("task-1")),
        ..Default::default()
    };

    let status = runtime
        .update_task_status(
            "task-1",
            WorkflowStatus {
                mode: WorkflowMode::Idle,
                stage: WorkflowStage::Idle,
                code: WorkflowStatusCode::NoSpeech,
                detail: None,
                message: "未检测到语音".to_string(),
                current_audio_path: None,
                last_record_id: None,
                revision: 0,
            },
        )
        .unwrap();

    assert_eq!(status.mode, WorkflowMode::Idle);
    assert!(matches!(runtime.activity, WorkflowActivity::Inactive));
    assert!(matches!(
        prepare_toggle(&mut runtime).unwrap().action,
        ToggleAction::Start(_)
    ));
}

#[test]
fn processing_commit_rejects_cancelled_and_stale_tasks() {
    let cancelled = test_task("cancelled");
    cancelled.cancel_token.store(true, Ordering::SeqCst);
    let mut runtime = WorkflowRuntime {
        activity: WorkflowActivity::Processing(cancelled.clone()),
        ..Default::default()
    };
    let mut appended = false;
    let result = runtime
        .commit_processing(&cancelled, || {
            appended = true;
            Ok(WorkflowStatus::default())
        })
        .unwrap();
    assert!(matches!(result, ProcessingCommit::Cancelled));
    assert!(!appended);

    let current = test_task("current");
    let stale = test_task("stale");
    runtime.activity = WorkflowActivity::Processing(current);
    let result = runtime
        .commit_processing(&stale, || {
            appended = true;
            Ok(WorkflowStatus::default())
        })
        .unwrap();
    assert!(matches!(result, ProcessingCommit::Cancelled));
    assert!(!appended);
}

#[test]
fn processing_commit_persists_once_and_releases_activity() {
    let task = test_task("processing");
    let mut runtime = WorkflowRuntime {
        activity: WorkflowActivity::Processing(task.clone()),
        ..Default::default()
    };
    let result = runtime
        .commit_processing(&task, || {
            Ok(WorkflowStatus {
                code: WorkflowStatusCode::CompleteCorrected,
                last_record_id: Some("record-1".to_string()),
                ..WorkflowStatus::default()
            })
        })
        .unwrap();

    let ProcessingCommit::Committed(status) = result else {
        panic!("processing commit was unexpectedly cancelled");
    };
    assert_eq!(status.code, WorkflowStatusCode::CompleteCorrected);
    assert_eq!(status.last_record_id.as_deref(), Some("record-1"));
    assert!(matches!(runtime.activity, WorkflowActivity::Inactive));
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
                service_id: "openai".to_string(),
                model: "gpt-5.4-mini".to_string(),
            },
            RaceModelTarget {
                service_id: "openai".to_string(),
                model: "gpt-5.4".to_string(),
            },
        ]
    );
}

#[test]
fn race_targets_resolve_distinct_services_of_the_same_provider_without_reusing_keys() {
    let mut config = AppConfig::default();
    config.model_services[0].api_key = "primary-placeholder".into();
    let mut second = config.model_services[0].clone();
    second.id = "openai-secondary".into();
    second.endpoint = "https://secondary.example.test/v1".into();
    second.api_key = "secondary-placeholder".into();
    config.model_services.push(second);
    let base = config.resolve_llm(&config.llm);
    let target = RaceModelTarget {
        service_id: "openai-secondary".into(),
        model: "second-model".into(),
    };
    let resolved = llm_for_race_target(&base, &target);
    assert_eq!(resolved.provider, "openai");
    assert_eq!(resolved.api_key, "secondary-placeholder");
    assert_eq!(resolved.endpoint, "https://secondary.example.test/v1");
    assert_eq!(resolved.fast_mode_enabled, base.fast_mode_enabled);
    let missing = llm_for_race_target(
        &base,
        &RaceModelTarget {
            service_id: "removed-service".into(),
            model: "second-model".into(),
        },
    );
    assert!(missing.api_key.is_empty());
    assert!(missing.endpoint.is_empty());
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

fn race_text(provider: &str) -> TranscriptionOutcome {
    TranscriptionOutcome::Text(AsrOutput {
        text: "race winner".to_string(),
        provider: provider.to_string(),
        task_id: Some("race-task".to_string()),
        duration_ms: Some(100),
    })
}

#[test]
fn asr_race_no_speech_waits_for_the_other_branch_text() {
    let mut race = AsrRaceState::default();

    assert!(race
        .accept(AsrRaceBranch::Live, Ok(TranscriptionOutcome::NoSpeech))
        .is_none());
    let result = race
        .accept(
            AsrRaceBranch::File,
            Ok(race_text("volcengine_ws_file_fallback")),
        )
        .unwrap()
        .unwrap();

    let TranscriptionOutcome::Text(output) = result else {
        panic!("file text should win after live no-speech");
    };
    assert_eq!(output.provider, "volcengine_ws_file_fallback");
}

#[test]
fn asr_race_single_failure_waits_for_the_other_branch_text() {
    let mut race = AsrRaceState::default();

    assert!(race
        .accept(AsrRaceBranch::File, Err(anyhow!("file failed")))
        .is_none());
    let result = race
        .accept(AsrRaceBranch::Live, Ok(race_text("volcengine_ws_live")))
        .unwrap()
        .unwrap();

    let TranscriptionOutcome::Text(output) = result else {
        panic!("live text should win after file failure");
    };
    assert_eq!(output.provider, "volcengine_ws_live");
}

#[test]
fn asr_race_reports_both_branch_failures_together() {
    let mut race = AsrRaceState::default();

    assert!(race
        .accept(AsrRaceBranch::Live, Err(anyhow!("live failed")))
        .is_none());
    let error = race
        .accept(AsrRaceBranch::File, Err(anyhow!("file failed")))
        .unwrap()
        .unwrap_err()
        .to_string();

    assert!(error.contains("live failed"));
    assert!(error.contains("file failed"));
}

#[test]
fn asr_race_no_speech_beats_a_single_branch_failure() {
    let mut race = AsrRaceState::default();

    assert!(race
        .accept(AsrRaceBranch::Live, Err(anyhow!("live failed")))
        .is_none());
    let result = race
        .accept(AsrRaceBranch::File, Ok(TranscriptionOutcome::NoSpeech))
        .unwrap()
        .unwrap();

    assert!(matches!(result, TranscriptionOutcome::NoSpeech));
}

#[test]
fn file_asr_waits_for_the_live_priority_gate() {
    assert!(!should_launch_file_asr(
        false,
        LivePriorityEvent::ProducedText,
    ));
    assert!(should_launch_file_asr(
        false,
        LivePriorityEvent::PostStopReconnectFailed,
    ));
    assert!(should_launch_file_asr(
        false,
        LivePriorityEvent::EndedWithoutText,
    ));
    assert!(!should_launch_file_asr(
        true,
        LivePriorityEvent::PostStopReconnectFailed,
    ));
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
