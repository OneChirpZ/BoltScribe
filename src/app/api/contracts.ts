import type {
  AppConfig,
  AudioInputDevice,
  AudioLevelSample,
  AudioOutputDevice,
  ConfigImportResult,
  DataDirInfo,
  DeleteHistoryResult,
  HistoryRecord,
  InputStats,
  RecordingCleanupPreview,
  RecordingCleanupResult,
  RecordingCleanupUnit,
  RecordingOverlayPositionUpdate,
  SaveConfigResult,
  VadTestStatus,
  WorkflowStatus,
} from "../../types";

export type Unlisten = () => void;

export interface ConfigApi {
  loadConfig(): Promise<AppConfig>;
  saveConfig(
    config: AppConfig,
    expectedOverlayPosition?: RecordingOverlayPositionUpdate | null,
  ): Promise<SaveConfigResult>;
  exportConfig(config: AppConfig): Promise<string>;
  importConfig(raw: string): Promise<ConfigImportResult>;
  listenConfigUpdated(handler: (config: AppConfig) => void): Promise<Unlisten>;
  listenConfigCloseRequested(handler: () => void): Promise<Unlisten>;
}

export interface WorkflowApi {
  getStatus(): Promise<WorkflowStatus>;
  toggleRecording(): Promise<WorkflowStatus>;
  cancelCurrentWorkflow(): Promise<WorkflowStatus>;
  listenWorkflowStatus(handler: (status: WorkflowStatus) => void): Promise<Unlisten>;
}

export interface OverlayApi {
  startRecordingOverlayDrag(): Promise<void>;
  listenRecordingOverlayPositionUpdated(
    handler: (update: RecordingOverlayPositionUpdate) => void,
  ): Promise<Unlisten>;
}

export interface HistoryApi {
  loadHistory(limit: number, offset?: number): Promise<HistoryRecord[]>;
  deleteHistoryRecord(id: string): Promise<DeleteHistoryResult>;
  retryHistoryRecord(id: string): Promise<HistoryRecord>;
  cleanupRecordingFiles(amount: number, unit: RecordingCleanupUnit): Promise<RecordingCleanupResult>;
  previewRecordingCleanup(amount: number, unit: RecordingCleanupUnit): Promise<RecordingCleanupPreview>;
  loadStats(): Promise<InputStats>;
  listenHistoryUpdated(handler: () => void): Promise<Unlisten>;
}

export interface AudioApi {
  loadAudioInputDevices(): Promise<AudioInputDevice[]>;
  loadAudioOutputDevices(): Promise<AudioOutputDevice[]>;
  restartSystemAudioService(): Promise<boolean>;
  startVadTest(audioConfig: AppConfig["audio"]): Promise<VadTestStatus>;
  updateVadTestSettings(noiseMarginDb: number, confirmationMs: number, noiseWindowMs: number): Promise<VadTestStatus>;
  stopVadTest(): Promise<VadTestStatus>;
  getVadTestStatus(): Promise<VadTestStatus>;
  listenVadTestStatus(handler: (status: VadTestStatus) => void): Promise<Unlisten>;
  listenAudioLevel(handler: (sample: AudioLevelSample) => void): Promise<Unlisten>;
}

export interface PermissionsApi {
  accessibilityPermissionGranted(): Promise<boolean>;
  requestAccessibilityPermission(): Promise<boolean>;
  openAccessibilitySettings(): Promise<void>;
  inputMonitoringPermissionGranted(): Promise<boolean>;
  requestInputMonitoringPermission(): Promise<boolean>;
  openInputMonitoringSettings(): Promise<void>;
  requestMicrophonePermission(): Promise<boolean>;
}

export interface SystemApi {
  getAppVersion(): Promise<string>;
  openAppDir(): Promise<void>;
  openGitHubRepository(): Promise<void>;
  getDataDir(): Promise<DataDirInfo>;
  chooseDataDir(): Promise<string | null>;
  setDataDir(path: string): Promise<DataDirInfo>;
  resetDataDir(): Promise<DataDirInfo>;
  closeMainWindow(): Promise<void>;
  applyFnTrigger(enabled: boolean, longPressDurationMs: number): Promise<void>;
  copyTextToClipboard(text: string): Promise<void>;
}

export interface AppApi {
  config: ConfigApi;
  overlay: OverlayApi;
  workflow: WorkflowApi;
  history: HistoryApi;
  audio: AudioApi;
  permissions: PermissionsApi;
  system: SystemApi;
}
