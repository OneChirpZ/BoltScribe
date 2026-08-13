import { supportsAudioServiceRestart } from "../../domain/platform";
import InputVadSection from "./InputVadSection";
import OutputDuckingSection from "./OutputDuckingSection";
import type { SettingsPageProps } from "./types";

type AudioSectionProps = Pick<
  SettingsPageProps,
  | "config"
  | "audioDevices"
  | "audioOutputDevices"
  | "audioDevicesRefreshing"
  | "audioServiceRestarting"
  | "onChange"
  | "onRefreshAudioDevices"
  | "onRestartAudioService"
  | "vadTestStatus"
  | "onStartVadTest"
  | "onUpdateVadTestSettings"
  | "onStopVadTest"
  | "canRestartAudioService"
  | "text"
>;

export default function AudioSection(props: AudioSectionProps) {
  const {
    audioDevicesRefreshing,
    audioServiceRestarting,
    canRestartAudioService,
    onRefreshAudioDevices,
    onRestartAudioService,
    text,
    vadTestStatus,
  } = props;

  function confirmAudioServiceRestart() {
    if (window.confirm(text.settings.audioRestartServiceConfirm)) {
      onRestartAudioService();
    }
  }

  return (
    <div className="settings-section">
      <div className="section-title">
        <h2>{text.settings.audio}</h2>
        <div className="section-actions">
          {supportsAudioServiceRestart() ? (
            <button
              className="secondary small"
              type="button"
              disabled={
                !canRestartAudioService
                || audioServiceRestarting
                || audioDevicesRefreshing
                || vadTestStatus.mode !== "idle"
              }
              title={text.settings.audioRestartServiceHelp}
              onClick={confirmAudioServiceRestart}
            >
              {audioServiceRestarting ? text.settings.audioRestartingService : text.settings.audioRestartService}
            </button>
          ) : null}
          <button
            className="secondary small"
            type="button"
            disabled={audioDevicesRefreshing || audioServiceRestarting || vadTestStatus.mode !== "idle"}
            onClick={onRefreshAudioDevices}
          >
            {audioDevicesRefreshing ? text.settings.audioRefreshingDevices : text.settings.audioRefreshDevices}
          </button>
        </div>
      </div>
      <div className="audio-settings-stack">
        <InputVadSection
          config={props.config}
          audioDevices={props.audioDevices}
          onChange={props.onChange}
          vadTestStatus={vadTestStatus}
          onStartVadTest={props.onStartVadTest}
          onUpdateVadTestSettings={props.onUpdateVadTestSettings}
          onStopVadTest={props.onStopVadTest}
          text={text}
        />
        <OutputDuckingSection
          config={props.config}
          audioOutputDevices={props.audioOutputDevices}
          onChange={props.onChange}
          text={text}
        />
      </div>
    </div>
  );
}
