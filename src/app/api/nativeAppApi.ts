import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
import type { AppApi } from "./contracts";

export const nativeAppApi = {
  config: {
    loadConfig: () => invoke<AppConfig>("load_config"),
    saveConfig: (
      config: AppConfig,
      expectedOverlayPosition?: RecordingOverlayPositionUpdate | null,
    ) => invoke<SaveConfigResult>("save_config", {
      config,
      expectedOverlayPosition: expectedOverlayPosition ?? null,
    }),
    exportConfig: (config: AppConfig) => invoke<string>("export_config", { config }),
    importConfig: (raw: string) => invoke<ConfigImportResult>("import_config", { raw }),
    listenConfigUpdated: (handler: (config: AppConfig) => void) =>
      listen<AppConfig>("config://updated", (event) => handler(event.payload)),
    listenConfigCloseRequested: (handler: () => void) => listen("config://close-requested", handler),
  },
  overlay: {
    startRecordingOverlayDrag: () => invoke<void>("start_recording_overlay_drag"),
    listenRecordingOverlayPositionUpdated: (handler: (update: RecordingOverlayPositionUpdate) => void) =>
      listen<RecordingOverlayPositionUpdate>("overlay://position-updated", (event) => handler(event.payload)),
  },
  workflow: {
    getStatus: () => invoke<WorkflowStatus>("get_status"),
    toggleRecording: () => invoke<WorkflowStatus>("toggle_recording"),
    cancelCurrentWorkflow: () => invoke<WorkflowStatus>("cancel_current_workflow"),
    listenWorkflowStatus: (handler: (status: WorkflowStatus) => void) =>
      listen<WorkflowStatus>("workflow://status", (event) => handler(event.payload)),
  },
  history: {
    loadHistory: (limit: number, offset = 0) => invoke<HistoryRecord[]>("load_history", { limit, offset }),
    deleteHistoryRecord: (id: string) => invoke<DeleteHistoryResult>("delete_history_record", { id }),
    retryHistoryRecord: (id: string) => invoke<HistoryRecord>("retry_history_record", { id }),
    cleanupRecordingFiles: (amount: number, unit: RecordingCleanupUnit) =>
      invoke<RecordingCleanupResult>("cleanup_recording_files", { amount, unit }),
    previewRecordingCleanup: (amount: number, unit: RecordingCleanupUnit) =>
      invoke<RecordingCleanupPreview>("preview_recording_cleanup", { amount, unit }),
    loadStats: () => invoke<InputStats>("load_stats"),
    listenHistoryUpdated: (handler: () => void) => listen("history://updated", handler),
  },
  audio: {
    loadAudioInputDevices: () => invoke<AudioInputDevice[]>("load_audio_input_devices"),
    loadAudioOutputDevices: () => invoke<AudioOutputDevice[]>("load_audio_output_devices"),
    restartSystemAudioService: () => invoke<boolean>("restart_system_audio_service"),
    startVadTest: (audioConfig: AppConfig["audio"]) => invoke<VadTestStatus>("start_vad_test", { audioConfig }),
    updateVadTestSettings: (noiseMarginDb: number, confirmationMs: number, noiseWindowMs: number) =>
      invoke<VadTestStatus>("update_vad_test_settings", { noiseMarginDb, confirmationMs, noiseWindowMs }),
    stopVadTest: () => invoke<VadTestStatus>("stop_vad_test"),
    getVadTestStatus: () => invoke<VadTestStatus>("get_vad_test_status"),
    listenVadTestStatus: (handler: (status: VadTestStatus) => void) =>
      listen<VadTestStatus>("vad-test://status", (event) => handler(event.payload)),
    listenAudioLevel: (handler: (sample: AudioLevelSample) => void) =>
      listen<AudioLevelSample>("audio://level", (event) => handler(event.payload)),
  },
  permissions: {
    accessibilityPermissionGranted: () => invoke<boolean>("accessibility_permission_granted"),
    requestAccessibilityPermission: () => invoke<boolean>("request_accessibility_permission"),
    openAccessibilitySettings: () => invoke<void>("open_accessibility_settings"),
    inputMonitoringPermissionGranted: () => invoke<boolean>("input_monitoring_permission_granted"),
    requestInputMonitoringPermission: () => invoke<boolean>("request_input_monitoring_permission"),
    openInputMonitoringSettings: () => invoke<void>("open_input_monitoring_settings"),
    requestMicrophonePermission: () => invoke<boolean>("request_microphone_permission"),
  },
  system: {
    getAppVersion: () => getVersion(),
    openAppDir: () => invoke<void>("open_app_dir"),
    openGitHubRepository: () => invoke<void>("open_github_repository"),
    getDataDir: () => invoke<DataDirInfo>("get_data_dir"),
    chooseDataDir: () => invoke<string | null>("choose_data_dir"),
    setDataDir: (path: string) => invoke<DataDirInfo>("set_data_dir", { path }),
    resetDataDir: () => invoke<DataDirInfo>("reset_data_dir"),
    closeMainWindow: () => invoke<void>("close_main_window"),
    applyFnTrigger: (enabled: boolean, longPressDurationMs: number) =>
      invoke<void>("apply_fn_trigger", { enabled, longPressDurationMs }),
    copyTextToClipboard: (text: string) => invoke<void>("copy_text_to_clipboard", { text }),
  },
} satisfies AppApi;
