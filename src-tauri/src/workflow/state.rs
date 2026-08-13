use super::*;

#[derive(Default)]
pub struct AppState {
    pub(super) runtime: Mutex<WorkflowRuntime>,
    pub(super) recorder: RecorderController,
    pub(super) vad_test: Mutex<Option<crate::vad_test_session::VadTestSession>>,
}

#[derive(Default)]
pub(super) struct WorkflowRuntime {
    pub(super) activity: WorkflowActivity,
    pub(super) status: WorkflowStatus,
}

#[derive(Default)]
pub(super) enum WorkflowActivity {
    #[default]
    Inactive,
    Starting(WorkflowTask),
    Recording(Box<RecordingSession>),
    Processing(WorkflowTask),
    RetryingHistory {
        record_id: String,
    },
}

pub(super) struct RecordingSession {
    pub(super) task: WorkflowTask,
    pub(super) config: AppConfig,
    pub(super) live_asr: Option<VolcengineLiveAsrSession>,
    pub(super) live_asr_start_error: Option<String>,
    pub(super) vad_gate: Option<VadGate>,
    pub(super) volume_ducking: Option<OutputVolumeDuckingSession>,
    pub(super) audio_level_meter: AudioLevelMeter,
}

#[derive(Debug)]
pub(super) struct CorrectionOutcome {
    pub(super) text: String,
    pub(super) logs: Vec<LlmCallLog>,
}

#[derive(Debug)]
pub(super) struct CorrectionFailure {
    pub(super) message: String,
    pub(super) logs: Vec<LlmCallLog>,
}

pub(super) enum ProcessingOutcome {
    Complete,
    NoSpeech,
    Cancelled,
}

#[derive(Clone)]
pub(super) struct WorkflowTask {
    pub(super) id: String,
    pub(super) cancel_token: Arc<AtomicBool>,
}

pub(super) struct StopContext {
    pub(super) task: WorkflowTask,
    pub(super) config: Option<AppConfig>,
    pub(super) live_asr: Option<VolcengineLiveAsrSession>,
    pub(super) live_asr_start_error: Option<String>,
    pub(super) vad_gate: Option<VadGate>,
    pub(super) volume_ducking: Option<OutputVolumeDuckingSession>,
    pub(super) pending_stop: Option<PendingRecordingStop>,
}

pub(super) struct ProcessRecordingContext {
    pub(super) task: WorkflowTask,
    pub(super) recorded: RecordedAudio,
    pub(super) total_started_at: Instant,
    pub(super) config: Option<AppConfig>,
    pub(super) live_asr: Option<VolcengineLiveAsrSession>,
    pub(super) live_asr_start_error: Option<String>,
}

#[derive(Debug)]
pub(super) enum ProcessingCommit {
    Cancelled,
    Committed(WorkflowStatus),
}

pub(super) enum ToggleAction {
    Busy,
    Start(WorkflowTask),
    Stop(Box<StopContext>),
    CancelArmed(WorkflowTask),
}

pub(super) struct TogglePlan {
    pub(super) status: WorkflowStatus,
    pub(super) action: ToggleAction,
}

pub(super) struct HistoryRetryGuard<'a> {
    pub(super) state: &'a AppState,
    pub(super) record_id: String,
}

impl WorkflowTask {
    pub(super) fn cancelled(&self) -> bool {
        self.cancel_token.load(Ordering::SeqCst)
    }
}

pub(super) fn restore_output_volume(session: Option<OutputVolumeDuckingSession>) {
    if let Some(session) = session {
        if let Err(err) = session.restore() {
            eprintln!("failed to restore output volume: {err:?}");
        }
    }
}

impl WorkflowActivity {
    pub(super) fn task(&self) -> Option<&WorkflowTask> {
        match self {
            Self::Starting(task) | Self::Processing(task) => Some(task),
            Self::Recording(session) => Some(&session.task),
            Self::Inactive | Self::RetryingHistory { .. } => None,
        }
    }

    pub(super) fn task_id(&self) -> Option<&str> {
        self.task().map(|task| task.id.as_str())
    }

    pub(super) fn is_active(&self) -> bool {
        !matches!(self, Self::Inactive)
    }

    pub(super) fn is_recording_task(&self, task_id: &str) -> bool {
        matches!(self, Self::Recording(session) if session.task.id == task_id)
    }
}

impl Drop for HistoryRetryGuard<'_> {
    fn drop(&mut self) {
        let Ok(mut runtime) = self.state.runtime.lock() else {
            return;
        };
        if matches!(
            &runtime.activity,
            WorkflowActivity::RetryingHistory { record_id } if record_id == &self.record_id
        ) {
            runtime.activity = WorkflowActivity::Inactive;
        }
    }
}

impl WorkflowRuntime {
    pub(super) fn update_status(&mut self, mut status: WorkflowStatus) -> WorkflowStatus {
        status.revision = self.status.revision.saturating_add(1);
        self.status = status.clone();
        status
    }

    pub(super) fn update_task_status(
        &mut self,
        task_id: &str,
        status: WorkflowStatus,
    ) -> Option<WorkflowStatus> {
        if self.activity.task_id() != Some(task_id) {
            return None;
        }

        if matches!(&status.mode, WorkflowMode::Idle | WorkflowMode::Error) {
            let previous = std::mem::take(&mut self.activity);
            if let WorkflowActivity::Recording(session) = previous {
                session.audio_level_meter.stop();
            }
        }
        Some(self.update_status(status))
    }

    pub(super) fn commit_processing(
        &mut self,
        task: &WorkflowTask,
        commit: impl FnOnce() -> Result<WorkflowStatus>,
    ) -> Result<ProcessingCommit> {
        if task.cancelled() || self.activity.task_id() != Some(&task.id) {
            return Ok(ProcessingCommit::Cancelled);
        }

        let status = commit()?;
        let previous = std::mem::take(&mut self.activity);
        debug_assert!(matches!(previous, WorkflowActivity::Processing(_)));
        Ok(ProcessingCommit::Committed(self.update_status(status)))
    }
}

impl AppState {
    pub(crate) fn recorder_ref(&self) -> &RecorderController {
        &self.recorder
    }

    pub(crate) fn vad_test_slot(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<crate::vad_test_session::VadTestSession>>> {
        self.vad_test
            .lock()
            .map_err(|_| anyhow!("Failed to lock VAD test state"))
    }

    pub(crate) fn runtime_for_vad_test(&self) -> Result<bool> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        Ok(runtime.activity.is_active())
    }

    pub fn status(&self) -> WorkflowStatus {
        self.runtime
            .lock()
            .map(|runtime| runtime.status.clone())
            .unwrap_or_else(|_| WorkflowStatus {
                mode: WorkflowMode::Error,
                stage: WorkflowStage::Error,
                code: WorkflowStatusCode::Failed,
                detail: Some("状态锁读取失败".to_string()),
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
        if runtime.activity.is_active()
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

    pub(super) fn begin_history_retry(&self, record_id: &str) -> Result<HistoryRetryGuard<'_>> {
        let record_id = record_id.trim();
        if record_id.is_empty() {
            return Err(anyhow!("History record id cannot be empty"));
        }

        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("Failed to lock workflow state"))?;
        match &runtime.activity {
            WorkflowActivity::Inactive => {}
            WorkflowActivity::RetryingHistory { .. } => {
                return Err(anyhow!("Another history record is already retrying"));
            }
            _ => {
                return Err(anyhow!(
                    "Cannot retry history while recording or processing"
                ));
            }
        }
        runtime.activity = WorkflowActivity::RetryingHistory {
            record_id: record_id.to_string(),
        };
        drop(runtime);

        Ok(HistoryRetryGuard {
            state: self,
            record_id: record_id.to_string(),
        })
    }
}
