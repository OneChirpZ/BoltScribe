import type {
  AppConfig,
  AudioInputDevice,
  AudioOutputDevice,
  ConfigImportReport,
  DataDirInfo,
  RecordingCleanupPreview,
  RecordingCleanupUnit,
  VadTestStatus,
} from "../../types";
import type { TextBundle } from "../../domain/i18n";
import type { PermissionRequestState } from "../../domain/permissions";

export interface SettingsPageProps {
  config: AppConfig;
  audioDevices: AudioInputDevice[];
  audioOutputDevices: AudioOutputDevice[];
  dataDir: DataDirInfo | null;
  audioDevicesRefreshing: boolean;
  audioServiceRestarting: boolean;
  onChange: (config: AppConfig) => void;
  onExportConfig: () => void;
  onImportConfig: (file: File) => void;
  onOpenDataDir: () => void;
  onChooseDataDir: () => void;
  onResetDataDir: () => void;
  onCleanupRecordingFiles: (amount: number, unit: RecordingCleanupUnit) => Promise<void>;
  onPreviewRecordingCleanup: (amount: number, unit: RecordingCleanupUnit) => Promise<RecordingCleanupPreview>;
  onRefreshAudioDevices: () => void;
  onRestartAudioService: () => void;
  vadTestStatus: VadTestStatus;
  onStartVadTest: () => void;
  onUpdateVadTestSettings: (noiseMarginDb: number, confirmationMs: number, noiseWindowMs: number) => void;
  onStopVadTest: () => void;
  inputMonitoringGranted: boolean | null;
  inputMonitoringPermission: PermissionRequestState;
  onRefreshInputMonitoring: () => void;
  onRequestInputMonitoring: () => void;
  onApplyFnTrigger: (enabled: boolean) => void;
  importReport: ConfigImportReport | null;
  canSave: boolean;
  canChangeDataDir: boolean;
  canCleanupRecordings: boolean;
  canRestartAudioService: boolean;
  text: TextBundle;
}
