import { useRef, type ChangeEvent } from "react";
import type { AppConfig, AudioInputDevice, AudioOutputDevice, ConfigImportReport, OutputVolumeDuckingConfig } from "../types";
import Field from "../components/Field";
import HelpTip from "../components/HelpTip";
import PanelHeader from "../components/PanelHeader";
import ShortcutPicker from "../components/ShortcutPicker";
import { applyLanguageDefaultCorrectionTemplate } from "../domain/defaultCorrectionTemplates";
import { hotkeyEnabledSlots, hotkeySlots, soundSourceShortcutKeyOptions, updateHotkey, updateHotkeyEnabled } from "../domain/hotkeys";
import type { AppLanguage, TextBundle } from "../domain/i18n";
import type { PermissionRequestState } from "../domain/permissions";
import { supportsDockVisibilityControl, supportsFnLongPressTrigger, supportsOutputVolumeDucking, supportsSoundSourceHotkeyFallback } from "../domain/platform";

const maxHistoryRecords = 500;
const bytesPerGb = 1024 * 1024 * 1024;
const maxStorageGb = 2;
const maxOverlayOffset = 4000;
const minFnLongPressDurationMs = 50;
const maxFnLongPressDurationMs = 5000;

export default function SettingsPage({
  config,
  audioDevices,
  audioOutputDevices,
  onChange,
  onSave,
  onExportConfig,
  onImportConfig,
  onRefreshAudioDevices,
  inputMonitoringGranted,
  inputMonitoringPermission,
  onRefreshInputMonitoring,
  onRequestInputMonitoring,
  importReport,
  canSave,
  text,
}: {
  config: AppConfig;
  audioDevices: AudioInputDevice[];
  audioOutputDevices: AudioOutputDevice[];
  onChange: (config: AppConfig) => void;
  onSave: () => void;
  onExportConfig: () => void;
  onImportConfig: (file: File) => void;
  onRefreshAudioDevices: () => void;
  inputMonitoringGranted: boolean | null;
  inputMonitoringPermission: PermissionRequestState;
  onRefreshInputMonitoring: () => void;
  onRequestInputMonitoring: () => void;
  importReport: ConfigImportReport | null;
  canSave: boolean;
  text: TextBundle;
}) {
  const importInputRef = useRef<HTMLInputElement | null>(null);
  const storageGb = Number((config.retention.max_storage_bytes / bytesPerGb).toFixed(2));
  const canControlDockVisibility = supportsDockVisibilityControl();
  const canUseFnLongPressTrigger = supportsFnLongPressTrigger();
  const canDuckOutputVolume = supportsOutputVolumeDucking();
  const canUseSoundSourceFallback = supportsSoundSourceHotkeyFallback();
  const shortcutSlots = hotkeySlots(config);
  const shortcutEnabledSlots = hotkeyEnabledSlots(config);
  const outputDucking = outputVolumeDuckingConfig(config);
  const defaultOutputDevice = audioOutputDevices.find((device) => device.is_default) ?? null;
  const defaultOutputDeviceUsesMuteFallback =
    canDuckOutputVolume && defaultOutputDevice !== null && !defaultOutputDevice.supports_volume_control && defaultOutputDevice.supports_mute_control;
  const defaultOutputDeviceUnsupported =
    canDuckOutputVolume && defaultOutputDevice !== null && !defaultOutputDevice.supports_volume_control && !defaultOutputDevice.supports_mute_control;
  const defaultOutputDeviceUsesSoundSourceFallback =
    outputDucking.enabled && canUseSoundSourceFallback && defaultOutputDeviceUnsupported && outputDucking.sound_source_hotkey_fallback_enabled;
  const outputDeviceNames = uniqueStrings(audioOutputDevices.map((device) => device.name));
  const missingOutputDuckingNames = outputDucking.device_name_whitelist.filter((name) => !outputDeviceNames.includes(name));

  function updateMaxRecords(value: string) {
    const max_history_records = clampInt(Number(value), 1, maxHistoryRecords);
    onChange({ ...config, retention: { ...config.retention, max_history_records } });
  }

  function updateMaxStorageGb(value: string) {
    const gb = clampNumber(Number(value), 0.01, maxStorageGb);
    onChange({
      ...config,
      retention: {
        ...config.retention,
        max_storage_bytes: Math.round(gb * bytesPerGb),
      },
    });
  }

  function updateOverlayOffset(axis: "x" | "y", value: string) {
    const nextValue = clampInt(Number(value), -maxOverlayOffset, maxOverlayOffset);
    onChange({
      ...config,
      ui: {
        ...config.ui,
        [axis === "x" ? "recording_overlay_offset_x" : "recording_overlay_offset_y"]: nextValue,
      },
    });
  }

  function updateLanguage(value: string) {
    const language: AppLanguage = value === "en-US" ? "en-US" : "zh-CN";
    onChange(applyLanguageDefaultCorrectionTemplate(config, language));
  }

  function updateFnLongPressDuration(value: string) {
    const fn_long_press_duration_ms = clampInt(Number(value), minFnLongPressDurationMs, maxFnLongPressDurationMs);
    onChange({ ...config, system: { ...config.system, fn_long_press_duration_ms } });
  }

  function updateFnLongPressEnabled(enabled: boolean) {
    onChange({ ...config, system: { ...config.system, fn_long_press_enabled: enabled } });
    if (enabled) {
      onRequestInputMonitoring();
    }
  }

  function updateAudioInputDevice(value: string) {
    if (value === "system_default") {
      onChange({
        ...config,
        audio: {
          ...config.audio,
          input_device_mode: "system_default",
          input_device_id: null,
          input_device_name: null,
        },
      });
      return;
    }

    const device = audioDevices.find((item) => item.id === value);
    onChange({
      ...config,
      audio: {
        ...config.audio,
        input_device_mode: "manual",
        input_device_id: value,
        input_device_name: device?.name ?? value,
      },
    });
  }

  function updateOutputVolumeDucking(patch: Partial<OutputVolumeDuckingConfig>) {
    onChange({
      ...config,
      audio: {
        ...config.audio,
        output_volume_ducking: {
          ...outputDucking,
          ...patch,
        },
      },
    });
  }

  function updateOutputVolumeReduction(value: string) {
    updateOutputVolumeDucking({ reduction_percent: clampInt(Number(value), 0, 100) });
  }

  function toggleOutputDuckingDeviceName(name: string, checked: boolean) {
    const current = outputDucking.device_name_whitelist.filter((item) => item !== name);
    updateOutputVolumeDucking({
      device_name_whitelist: checked ? uniqueStrings([...current, name]) : current,
    });
  }

  function importSelectedFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (file) {
      onImportConfig(file);
    }
  }

  return (
    <section className="panel page-stack">
      <PanelHeader title={text.settings.title} action={<button className="primary small" disabled={!canSave} onClick={onSave}>{text.common.save}</button>} />

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.languageSection}</h2>
        </div>
        <div className="form-grid">
          <Field label={text.settings.language}>
            <select
              value={config.ui.app_language ?? "zh-CN"}
              onChange={(event) => updateLanguage(event.target.value)}
            >
              <option value="zh-CN">{text.settings.chinese}</option>
              <option value="en-US">{text.settings.english}</option>
            </select>
          </Field>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.inputTriggers}</h2>
        </div>
        <div className="shortcut-grid">
          <ShortcutPicker
            label={text.settings.shortcut1}
            enabled={shortcutEnabledSlots[0]}
            value={shortcutSlots[0]}
            onEnabledChange={(enabled) => onChange(updateHotkeyEnabled(config, 0, enabled))}
            onChange={(value) => onChange(updateHotkey(config, 0, value))}
            text={text}
          />
          <ShortcutPicker
            label={text.settings.shortcut2}
            enabled={shortcutEnabledSlots[1]}
            value={shortcutSlots[1]}
            onEnabledChange={(enabled) => onChange(updateHotkeyEnabled(config, 1, enabled))}
            onChange={(value) => onChange(updateHotkey(config, 1, value))}
            text={text}
          />
        </div>
        {canUseFnLongPressTrigger ? (
          <div className="trigger-options">
            <label className="toggle-row">
              <input
                type="checkbox"
                checked={config.system.fn_long_press_enabled ?? false}
                onChange={(event) => updateFnLongPressEnabled(event.target.checked)}
              />
              {text.settings.fnLongPressTrigger}
            </label>
            <Field label={text.settings.fnLongPressDuration}>
              <div className="number-with-unit">
                <input
                  type="number"
                  min={minFnLongPressDurationMs}
                  max={maxFnLongPressDurationMs}
                  step="50"
                  value={config.system.fn_long_press_duration_ms ?? 200}
                  disabled={!config.system.fn_long_press_enabled}
                  onChange={(event) => updateFnLongPressDuration(event.target.value)}
                />
                <span>{text.common.milliseconds}</span>
              </div>
            </Field>
            {config.system.fn_long_press_enabled ? (
              <div className="trigger-permission-row">
                <span>
                  {text.permission.inputMonitoring}：
                  <strong className={inputMonitoringGranted ? "permission-ok" : "permission-missing"}>
                    {inputMonitoringPermissionLabel(inputMonitoringPermission, inputMonitoringGranted, text)}
                  </strong>
                </span>
                <div>
                  <button className="secondary small" type="button" onClick={onRefreshInputMonitoring}>{text.permission.recheck}</button>
                  <button className="secondary small" type="button" onClick={onRequestInputMonitoring} disabled={inputMonitoringPermission === "checking"}>
                    {inputMonitoringPermission === "checking" ? text.common.checking : text.permission.requestInputMonitoring}
                  </button>
                </div>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.audioInput}</h2>
          <div className="section-actions">
            <button className="secondary small" type="button" onClick={onRefreshAudioDevices}>
              {text.settings.audioRefreshDevices}
            </button>
          </div>
        </div>
        <div className="form-grid">
          <Field label={text.settings.audioInputDevice} className="field-wide">
            <select
              value={audioInputDeviceValue(config, audioDevices)}
              onChange={(event) => updateAudioInputDevice(event.target.value)}
            >
              <option value="system_default">{text.settings.audioSystemDefault}</option>
              {selectedAudioDeviceMissing(config, audioDevices) ? (
                <option value={selectedAudioDeviceValue(config)}>
                  {text.settings.audioDeviceMissing(config.audio.input_device_name ?? config.audio.input_device_id ?? "")}
                </option>
              ) : null}
              {audioDevices.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name}{device.is_default ? text.settings.audioDefaultBadge : ""}
                </option>
              ))}
            </select>
            {audioDevices.length === 0 ? <span>{text.settings.audioNoInputDevices}</span> : null}
          </Field>
          <div className="field-wide output-ducking-block">
            <div className="output-ducking-heading">
              <h3>{text.settings.audioOutputVolumeDucking}</h3>
              <HelpTip content={canDuckOutputVolume ? text.settings.outputVolumeDuckingHelp : text.settings.outputVolumeDuckingUnsupported} />
            </div>
            <fieldset className="output-ducking-controls" disabled={!canDuckOutputVolume}>
              <label className="toggle-row">
                <input
                  type="checkbox"
                  checked={outputDucking.enabled}
                  onChange={(event) => updateOutputVolumeDucking({ enabled: event.target.checked })}
                />
                <span>{text.settings.outputVolumeDuckingEnabled}</span>
              </label>
              <div className="output-ducking-status">
                <span className="status-chip">
                  {canDuckOutputVolume
                    ? text.settings.outputVolumeDuckingCurrent(defaultOutputDevice?.name ?? text.settings.outputVolumeDuckingNoOutputDevice)
                    : text.settings.outputVolumeDuckingUnsupported}
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
              <label className="toggle-row">
                <input
                  type="checkbox"
                  checked={outputDucking.mute_instead_of_reduce}
                  onChange={(event) =>
                    updateOutputVolumeDucking({ mute_instead_of_reduce: event.target.checked })
                  }
                />
                <span>{text.settings.outputVolumeDuckingMuteInstead}</span>
                <HelpTip content={text.settings.outputVolumeDuckingMuteInsteadHelp} />
              </label>
              {!outputDucking.mute_instead_of_reduce ? (
                <Field label={text.settings.outputVolumeDuckingReduction} className="field-wide">
                  <div className="range-with-value">
                    <input
                      className="range-input"
                      type="range"
                      min="0"
                      max="100"
                      step="1"
                      value={outputDucking.reduction_percent}
                      onChange={(event) => updateOutputVolumeReduction(event.target.value)}
                    />
                    <input
                      type="number"
                      min="0"
                      max="100"
                      step="1"
                      value={outputDucking.reduction_percent}
                      onChange={(event) => updateOutputVolumeReduction(event.target.value)}
                    />
                    <span>%</span>
                  </div>
                </Field>
              ) : null}
              {canUseSoundSourceFallback ? (
                <div className="sound-source-fallback">
                  <label className="toggle-row">
                    <input
                      type="checkbox"
                      checked={outputDucking.sound_source_hotkey_fallback_enabled}
                      onChange={(event) => updateOutputVolumeDucking({ sound_source_hotkey_fallback_enabled: event.target.checked })}
                    />
                    <span>{text.settings.outputVolumeDuckingSoundSourceFallback}</span>
                    <HelpTip content={text.settings.outputVolumeDuckingSoundSourceHelp} />
                  </label>
                  <ShortcutPicker
                    label={text.settings.outputVolumeDuckingSoundSourceHotkey}
                    enabled={outputDucking.sound_source_hotkey_fallback_enabled}
                    value={outputDucking.sound_source_toggle_mute_hotkey}
                    onChange={(value) => updateOutputVolumeDucking({ sound_source_toggle_mute_hotkey: value })}
                    text={text}
                    platform="macos"
                    keyOptions={soundSourceShortcutKeyOptions}
                    showEnabledToggle={false}
                  />
                </div>
              ) : null}
              <Field label={text.settings.outputVolumeDuckingWhitelist} className="field-wide">
                <div className="device-checklist">
                  {audioOutputDevices.map((device) => (
                    <label key={device.id} className="device-checklist-row">
                      <input
                        type="checkbox"
                        checked={outputDucking.device_name_whitelist.includes(device.name)}
                        onChange={(event) => toggleOutputDuckingDeviceName(device.name, event.target.checked)}
                      />
                      <span>
                        {device.name}{device.is_default ? text.settings.audioDefaultBadge : ""}
                        {outputDuckingDeviceBadge(device, text, canUseSoundSourceFallback && outputDucking.sound_source_hotkey_fallback_enabled)}
                      </span>
                    </label>
                  ))}
                  {missingOutputDuckingNames.map((name) => (
                    <label key={name} className="device-checklist-row">
                      <input
                        type="checkbox"
                        checked
                        onChange={(event) => toggleOutputDuckingDeviceName(name, event.target.checked)}
                      />
                      <span>{text.settings.outputVolumeDuckingMissingDevice(name)}</span>
                    </label>
                  ))}
                  {canDuckOutputVolume && audioOutputDevices.length === 0 ? <span>{text.settings.audioNoOutputDevices}</span> : null}
                  {canDuckOutputVolume && outputDucking.device_name_whitelist.length === 0 ? <span>{text.settings.outputVolumeDuckingWhitelistAll}</span> : null}
                </div>
              </Field>
            </fieldset>
          </div>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.retention}</h2>
        </div>
        <div className="form-grid">
          <Field label={text.settings.maxRecords}>
            <input type="number" min="1" max={maxHistoryRecords} value={config.retention.max_history_records} onChange={(event) => updateMaxRecords(event.target.value)} />
          </Field>
          <Field label={text.settings.maxStorage}>
            <input type="number" min="0.01" max={maxStorageGb} step="0.01" value={storageGb} onChange={(event) => updateMaxStorageGb(event.target.value)} />
          </Field>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.overlayPosition}</h2>
        </div>
        <div className="form-grid">
          <Field label={text.settings.offsetX}>
            <input
              type="number"
              min={-maxOverlayOffset}
              max={maxOverlayOffset}
              step="1"
              value={config.ui.recording_overlay_offset_x ?? 0}
              onChange={(event) => updateOverlayOffset("x", event.target.value)}
            />
          </Field>
          <Field label={text.settings.offsetY}>
            <input
              type="number"
              min={-maxOverlayOffset}
              max={maxOverlayOffset}
              step="1"
              value={config.ui.recording_overlay_offset_y ?? 0}
              onChange={(event) => updateOverlayOffset("y", event.target.value)}
            />
          </Field>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.configPortability}</h2>
          <div className="section-actions">
            <button className="secondary small" type="button" disabled={!canSave} onClick={onExportConfig}>
              {text.settings.exportConfig}
            </button>
            <button className="secondary small" type="button" disabled={!canSave} onClick={() => importInputRef.current?.click()}>
              {text.settings.importConfig}
            </button>
            <input
              ref={importInputRef}
              className="hidden-file-input"
              type="file"
              accept="application/json,.json"
              onChange={importSelectedFile}
            />
          </div>
        </div>
        {importReport ? <ConfigImportReportView report={importReport} text={text} /> : null}
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.system}</h2>
        </div>
        <label className="toggle-row">
          <input
            type="checkbox"
            checked={config.system.launch_at_login}
            onChange={(event) => onChange({ ...config, system: { ...config.system, launch_at_login: event.target.checked } })}
          />
          {text.settings.launchAtLogin}
        </label>
        {canControlDockVisibility ? (
          <label className="toggle-row">
            <input
              type="checkbox"
              checked={config.system.hide_dock_icon ?? false}
              onChange={(event) => onChange({ ...config, system: { ...config.system, hide_dock_icon: event.target.checked } })}
            />
            {text.settings.hideDockIcon}
          </label>
        ) : null}
      </div>
    </section>
  );
}

function ConfigImportReportView({
  report,
  text,
}: {
  report: ConfigImportReport;
  text: TextBundle;
}) {
  const hasDetails =
    report.missing_fields.length > 0 ||
    report.unknown_fields.length > 0 ||
    report.invalid_fields.length > 0 ||
    report.notes.length > 0;

  return (
    <div className="config-import-report">
      <div>
        <strong>{text.settings.importReport}</strong>
        <p>{text.settings.importFormat(report.format, report.version)}</p>
      </div>
      {!hasDetails ? <p>{text.settings.importNoIssues}</p> : null}
      <ReportList title={text.settings.importMissingFields} items={report.missing_fields} />
      <ReportList title={text.settings.importUnknownFields} items={report.unknown_fields} />
      <ReportList title={text.settings.importInvalidFields} items={report.invalid_fields} />
      <ReportList title={text.settings.importNotes} items={report.notes} />
    </div>
  );
}

function inputMonitoringPermissionLabel(status: PermissionRequestState, granted: boolean | null, text: TextBundle) {
  if (granted === true) {
    return text.permission.statusEnabled;
  }
  if (status === "checking") {
    return text.common.checking;
  }
  if (status === "denied") {
    return text.permission.statusDisabled;
  }
  return text.permission.statusNotRequested;
}

function ReportList({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0) {
    return null;
  }
  return (
    <div className="config-import-report-list">
      <span>{title}</span>
      <ul>
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </div>
  );
}

function audioInputDeviceValue(config: AppConfig, audioDevices: AudioInputDevice[]) {
  if (config.audio.input_device_mode !== "manual") {
    return "system_default";
  }
  return (selectedAudioDevice(config, audioDevices)?.id ?? selectedAudioDeviceValue(config)) || "system_default";
}

function selectedAudioDeviceValue(config: AppConfig) {
  return config.audio.input_device_id ?? config.audio.input_device_name ?? "";
}

function selectedAudioDevice(config: AppConfig, audioDevices: AudioInputDevice[]) {
  const id = config.audio.input_device_id;
  const name = config.audio.input_device_name;
  return audioDevices.find((device) => device.id === id) ?? audioDevices.find((device) => Boolean(name) && device.name === name) ?? null;
}

function selectedAudioDeviceMissing(config: AppConfig, audioDevices: AudioInputDevice[]) {
  return config.audio.input_device_mode === "manual" && Boolean(selectedAudioDeviceValue(config)) && !selectedAudioDevice(config, audioDevices);
}

function outputVolumeDuckingConfig(config: AppConfig): OutputVolumeDuckingConfig {
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

function outputDuckingDeviceBadge(device: AudioOutputDevice, text: TextBundle, soundSourceFallbackEnabled: boolean) {
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

function uniqueStrings(items: string[]) {
  const values: string[] = [];
  for (const item of items) {
    const value = item.trim();
    if (!value || values.includes(value)) {
      continue;
    }
    values.push(value);
  }
  return values;
}

function clampInt(value: number, min: number, max: number) {
  if (!Number.isFinite(value)) {
    return min;
  }
  return Math.min(max, Math.max(min, Math.round(value)));
}

function clampNumber(value: number, min: number, max: number) {
  if (!Number.isFinite(value)) {
    return min;
  }
  return Math.min(max, Math.max(min, value));
}
