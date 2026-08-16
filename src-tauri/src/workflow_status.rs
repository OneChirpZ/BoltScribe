use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
pub struct WorkflowStatus {
    pub mode: WorkflowMode,
    pub stage: WorkflowStage,
    pub code: WorkflowStatusCode,
    pub detail: Option<String>,
    pub message: String,
    pub current_audio_path: Option<String>,
    pub last_record_id: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatusCode {
    #[default]
    Ready,
    StartingMicrophone,
    Stopping,
    WaitingForSpeech,
    Recording,
    Cancelled,
    NoSpeech,
    Recognizing,
    #[allow(dead_code)] // Retained for status compatibility with older frontends.
    FileAsrFallback,
    Correcting,
    Pasting,
    PasteComplete,
    CompleteCorrected,
    CompleteCorrectionFallback,
    PasteFailedAccessibility,
    PasteFailedClipboard,
    RecordingStartFailed,
    RecordingStopFailed,
    RecordingCancelFailed,
    Failed,
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
    #[allow(dead_code)] // Retained for status compatibility with older frontends.
    FileAsrFallback,
    Correcting,
    Pasting,
    Complete,
    Error,
}
