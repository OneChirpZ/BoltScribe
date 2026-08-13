import Field from "../../components/Field";
import HelpTip from "../../components/HelpTip";
import ShortcutPicker from "../../components/ShortcutPicker";
import { soundSourceShortcutKeyOptions } from "../../domain/hotkeys";
import type { TextBundle } from "../../domain/i18n";
import { supportsOutputVolumeDucking, supportsSoundSourceHotkeyFallback } from "../../domain/platform";
import type { AppConfig, AudioOutputDevice, OutputVolumeDuckingConfig } from "../../types";

export default function OutputDuckingSection({
  config,
  audioOutputDevices,
  onChange,
  text,
}: {
  config: AppConfig;
  audioOutputDevices: AudioOutputDevice[];
  onChange: (config: AppConfig) => void;
  text: TextBundle;
}) {
  const canDuckOutputVolume = supportsOutputVolumeDucking();
  const canUseSoundSourceFallback = supportsSoundSourceHotkeyFallback();
  const outputDucking = normalizedOutputDucking(config);
  const defaultOutputDevice = audioOutputDevices.find((device) => device.is_default) ?? null;
  const defaultOutputDeviceUsesMuteFallback =
    canDuckOutputVolume
    && defaultOutputDevice !== null
    && !defaultOutputDevice.supports_volume_control
    && defaultOutputDevice.supports_mute_control;
  const defaultOutputDeviceUnsupported =
    canDuckOutputVolume
    && defaultOutputDevice !== null
    && !defaultOutputDevice.supports_volume_control
    && !defaultOutputDevice.supports_mute_control;
  const defaultOutputDeviceUsesSoundSourceFallback =
    outputDucking.enabled
    && canUseSoundSourceFallback
    && defaultOutputDeviceUnsupported
    && outputDucking.sound_source_hotkey_fallback_enabled;
  const outputDeviceNames = uniqueStrings(audioOutputDevices.map((device) => device.name));
  const missingNames = outputDucking.device_name_whitelist.filter((name) => !outputDeviceNames.includes(name));

  function update(patch: Partial<OutputVolumeDuckingConfig>) {
    onChange({
      ...config,
      audio: {
        ...config.audio,
        output_volume_ducking: { ...outputDucking, ...patch },
      },
    });
  }

  function toggleDeviceName(name: string, checked: boolean) {
    const current = outputDucking.device_name_whitelist.filter((item) => item !== name);
    update({ device_name_whitelist: checked ? uniqueStrings([...current, name]) : current });
  }

  return (
    <div className="settings-subsection output-ducking-block">
      <div className="output-ducking-heading">
        <h3 id="output-ducking-heading">{text.settings.audioOutputVolumeDucking}</h3>
        <HelpTip content={canDuckOutputVolume ? text.settings.outputVolumeDuckingHelp : text.settings.outputVolumeDuckingUnsupported} />
      </div>
      <fieldset className="output-ducking-controls" aria-labelledby="output-ducking-heading" disabled={!canDuckOutputVolume}>
        <label className="toggle-row">
          <input
            type="checkbox"
            checked={outputDucking.enabled}
            onChange={(event) => update({ enabled: event.target.checked })}
          />
          <span>{text.settings.outputVolumeDuckingEnabled}</span>
        </label>
        {outputDucking.enabled && canDuckOutputVolume ? (
          <div className="output-ducking-options">
            <div className="output-ducking-layout">
              <div className="output-ducking-primary">
                <div className="output-ducking-status">
                  <span className="status-chip">
                    {text.settings.outputVolumeDuckingCurrent(
                      defaultOutputDevice?.name ?? text.settings.outputVolumeDuckingNoOutputDevice,
                    )}
                  </span>
                  {defaultOutputDeviceUnsupported && !defaultOutputDeviceUsesSoundSourceFallback ? (
                    <span className="status-chip warning">
                      {text.settings.outputVolumeDuckingUnsupportedShort}
                      <HelpTip content={text.settings.outputVolumeDuckingDeviceUnsupported} />
                    </span>
                  ) : null}
                  {defaultOutputDeviceUsesSoundSourceFallback ? (
                    <span className="status-chip warning">
                      {text.settings.outputVolumeDuckingSoundSourceFallbackShort}
                      <HelpTip content={text.settings.outputVolumeDuckingSoundSourceHelp} />
                    </span>
                  ) : null}
                  {defaultOutputDeviceUsesMuteFallback ? (
                    <span className="status-chip warning">
                      {text.settings.outputVolumeDuckingMuteFallbackShort}
                      <HelpTip content={text.settings.outputVolumeDuckingDeviceMuteFallback} />
                    </span>
                  ) : null}
                </div>
                <div className="toggle-with-help">
                  <label className="toggle-row">
                    <input
                      type="checkbox"
                      checked={outputDucking.mute_instead_of_reduce}
                      onChange={(event) => update({ mute_instead_of_reduce: event.target.checked })}
                    />
                    <span>{text.settings.outputVolumeDuckingMuteInstead}</span>
                  </label>
                  <HelpTip content={text.settings.outputVolumeDuckingMuteInsteadHelp} />
                </div>
                {!outputDucking.mute_instead_of_reduce ? (
                  <Field label={text.settings.outputVolumeDuckingReduction} group>
                    <div className="range-with-value">
                      <input
                        className="range-input"
                        type="range"
                        aria-label={text.settings.outputVolumeDuckingReduction}
                        min="0"
                        max="100"
                        step="1"
                        value={outputDucking.reduction_percent}
                        onChange={(event) => update({ reduction_percent: clampPercent(Number(event.target.value)) })}
                      />
                      <input
                        type="number"
                        aria-label={text.settings.outputVolumeDuckingReduction}
                        min="0"
                        max="100"
                        step="1"
                        value={outputDucking.reduction_percent}
                        onChange={(event) => update({ reduction_percent: clampPercent(Number(event.target.value)) })}
                      />
                      <span>%</span>
                    </div>
                  </Field>
                ) : null}
                {canUseSoundSourceFallback ? (
                  <div className="sound-source-fallback">
                    <div className="toggle-with-help">
                      <label className="toggle-row">
                        <input
                          type="checkbox"
                          checked={outputDucking.sound_source_hotkey_fallback_enabled}
                          onChange={(event) => update({ sound_source_hotkey_fallback_enabled: event.target.checked })}
                        />
                        <span>{text.settings.outputVolumeDuckingSoundSourceFallback}</span>
                      </label>
                      <HelpTip content={text.settings.outputVolumeDuckingSoundSourceHelp} />
                    </div>
                    <ShortcutPicker
                      label={text.settings.outputVolumeDuckingSoundSourceHotkey}
                      enabled={outputDucking.sound_source_hotkey_fallback_enabled}
                      value={outputDucking.sound_source_toggle_mute_hotkey}
                      onChange={(value) => update({ sound_source_toggle_mute_hotkey: value })}
                      text={text}
                      platform="macos"
                      keyOptions={soundSourceShortcutKeyOptions}
                      showEnabledToggle={false}
                    />
                  </div>
                ) : null}
              </div>
              <Field label={text.settings.outputVolumeDuckingWhitelist} className="output-device-card" group>
                <div className="device-checklist output-device-checklist">
                  {audioOutputDevices.map((device) => (
                    <label key={device.id} className="device-checklist-row">
                      <input
                        type="checkbox"
                        checked={outputDucking.device_name_whitelist.includes(device.name)}
                        onChange={(event) => toggleDeviceName(device.name, event.target.checked)}
                      />
                      <span>
                        {device.name}{device.is_default ? text.settings.audioDefaultBadge : ""}
                        {outputDuckingDeviceBadge(
                          device,
                          text,
                          canUseSoundSourceFallback && outputDucking.sound_source_hotkey_fallback_enabled,
                        )}
                      </span>
                    </label>
                  ))}
                  {missingNames.map((name) => (
                    <label key={name} className="device-checklist-row">
                      <input type="checkbox" checked onChange={(event) => toggleDeviceName(name, event.target.checked)} />
                      <span>{text.settings.outputVolumeDuckingMissingDevice(name)}</span>
                    </label>
                  ))}
                  {audioOutputDevices.length === 0 ? <span>{text.settings.audioNoOutputDevices}</span> : null}
                  {outputDucking.device_name_whitelist.length === 0
                    ? <span>{text.settings.outputVolumeDuckingWhitelistAll}</span>
                    : null}
                </div>
              </Field>
            </div>
          </div>
        ) : (
          <p className="settings-help-text">
            {canDuckOutputVolume ? text.settings.outputVolumeDuckingHelp : text.settings.outputVolumeDuckingUnsupported}
          </p>
        )}
      </fieldset>
    </div>
  );
}

function normalizedOutputDucking(config: AppConfig): OutputVolumeDuckingConfig {
  const ducking = config.audio.output_volume_ducking;
  return {
    enabled: ducking?.enabled ?? false,
    mute_instead_of_reduce: ducking?.mute_instead_of_reduce ?? false,
    reduction_percent: ducking?.reduction_percent ?? 70,
    device_name_whitelist: ducking?.device_name_whitelist ?? [],
    sound_source_hotkey_fallback_enabled: ducking?.sound_source_hotkey_fallback_enabled ?? false,
    sound_source_toggle_mute_hotkey: ducking?.sound_source_toggle_mute_hotkey ?? "Cmd+Opt+Ctrl+A",
  };
}

function outputDuckingDeviceBadge(device: AudioOutputDevice, text: TextBundle, soundSourceFallbackEnabled: boolean): string {
  if (device.supports_volume_control) {
    return "";
  }
  if (device.supports_mute_control) {
    return ` (${text.settings.outputVolumeDuckingDeviceMuteFallbackBadge})`;
  }
  if (soundSourceFallbackEnabled) {
    return ` (${text.settings.outputVolumeDuckingDeviceSoundSourceFallbackBadge})`;
  }
  return ` (${text.settings.outputVolumeDuckingDeviceUnsupportedBadge})`;
}

function uniqueStrings(items: string[]): string[] {
  const values: string[] = [];
  for (const item of items) {
    const value = item.trim();
    if (value && !values.includes(value)) {
      values.push(value);
    }
  }
  return values;
}

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) {
    return 0;
  }
  return Math.min(100, Math.max(0, Math.round(value)));
}
