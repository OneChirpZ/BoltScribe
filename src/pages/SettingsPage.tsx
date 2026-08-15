import PanelHeader from "../components/PanelHeader";
import AdvancedSettingsSection from "./settings/AdvancedSettingsSection";
import AppearanceSystemSection from "./settings/AppearanceSystemSection";
import AudioSection from "./settings/AudioSection";
import ConfigPortabilitySection from "./settings/ConfigPortabilitySection";
import OverlaySection from "./settings/OverlaySection";
import RetentionDataSection from "./settings/RetentionDataSection";
import TriggerSection from "./settings/TriggerSection";
import type { SettingsPageProps } from "./settings/types";

export default function SettingsPage(props: SettingsPageProps) {
  return (
    <section className="panel page-stack settings-page">
      <PanelHeader title={props.text.settings.title} />

      <AppearanceSystemSection config={props.config} onChange={props.onChange} text={props.text} />

      <TriggerSection
        config={props.config}
        onChange={props.onChange}
        inputMonitoringGranted={props.inputMonitoringGranted}
        inputMonitoringPermission={props.inputMonitoringPermission}
        onRefreshInputMonitoring={props.onRefreshInputMonitoring}
        onRequestInputMonitoring={props.onRequestInputMonitoring}
        onApplyFnTrigger={props.onApplyFnTrigger}
        text={props.text}
      />

      <AudioSection
        config={props.config}
        audioDevices={props.audioDevices}
        audioOutputDevices={props.audioOutputDevices}
        audioDevicesRefreshing={props.audioDevicesRefreshing}
        audioServiceRestarting={props.audioServiceRestarting}
        onChange={props.onChange}
        onRefreshAudioDevices={props.onRefreshAudioDevices}
        onRestartAudioService={props.onRestartAudioService}
        vadTestStatus={props.vadTestStatus}
        onStartVadTest={props.onStartVadTest}
        onUpdateVadTestSettings={props.onUpdateVadTestSettings}
        onStopVadTest={props.onStopVadTest}
        canRestartAudioService={props.canRestartAudioService}
        text={props.text}
      />

      <RetentionDataSection
        config={props.config}
        dataDir={props.dataDir}
        onChange={props.onChange}
        onOpenDataDir={props.onOpenDataDir}
        onChooseDataDir={props.onChooseDataDir}
        onResetDataDir={props.onResetDataDir}
        onCleanupRecordingFiles={props.onCleanupRecordingFiles}
        onPreviewRecordingCleanup={props.onPreviewRecordingCleanup}
        canSave={props.canSave}
        canChangeDataDir={props.canChangeDataDir}
        canCleanupRecordings={props.canCleanupRecordings}
        text={props.text}
      />

      <div className="settings-overview-grid settings-overview-grid-final">
        <OverlaySection config={props.config} onChange={props.onChange} text={props.text} />
        <ConfigPortabilitySection
          canSave={props.canSave}
          importReport={props.importReport}
          onExportConfig={props.onExportConfig}
          onImportConfig={props.onImportConfig}
          text={props.text}
        />
      </div>

      <AdvancedSettingsSection config={props.config} onChange={props.onChange} text={props.text} />
    </section>
  );
}
