// Compatibility facade for callers that have not moved to the domain-grouped AppApi yet.
import { appApi } from "./api";

export { appApi } from "./api";
export type { AppApi } from "./api";

export const {
  loadConfig,
  saveConfig,
  exportConfig,
  importConfig,
  listenConfigUpdated,
  listenConfigCloseRequested,
} = appApi.config;

export const {
  getStatus,
  toggleRecording,
  cancelCurrentWorkflow,
  listenWorkflowStatus,
} = appApi.workflow;

export const {
  loadHistory,
  deleteHistoryRecord,
  retryHistoryRecord,
  cleanupRecordingFiles,
  previewRecordingCleanup,
  loadStats,
  listenHistoryUpdated,
} = appApi.history;

export const {
  loadAudioInputDevices,
  loadAudioOutputDevices,
  restartSystemAudioService,
  startVadTest,
  updateVadTestSettings,
  stopVadTest,
  getVadTestStatus,
  listenVadTestStatus,
  listenAudioLevel,
} = appApi.audio;

export const {
  accessibilityPermissionGranted,
  requestAccessibilityPermission,
  openAccessibilitySettings,
  inputMonitoringPermissionGranted,
  requestInputMonitoringPermission,
  openInputMonitoringSettings,
  requestMicrophonePermission,
} = appApi.permissions;

export const {
  getAppVersion,
  openAppDir,
  openGitHubRepository,
  getDataDir,
  chooseDataDir,
  setDataDir,
  resetDataDir,
  hideMainWindow,
  applyFnTrigger,
  copyTextToClipboard,
} = appApi.system;
