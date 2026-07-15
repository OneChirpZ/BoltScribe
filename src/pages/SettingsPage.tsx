import { useRef, type ChangeEvent } from "react";
import type { AppConfig, AudioInputDevice, AudioInputDeviceRef, AudioOutputDevice, ConfigImportReport, DataDirInfo, OutputVolumeDuckingConfig } from "../types";
import Field from "../components/Field";
import HelpTip from "../components/HelpTip";
import PanelHeader from "../components/PanelHeader";
import ShortcutPicker from "../components/ShortcutPicker";
import { applyLanguageDefaultCorrectionTemplate } from "../domain/defaultCorrectionTemplates";
import { hotkeyEnabledSlots, hotkeySlots, soundSourceShortcutKeyOptions, updateHotkey, updateHotkeyEnabled } from "../domain/hotkeys";
import type { AppLanguage, TextBundle } from "../domain/i18n";
import { defaultRecordingOverlayScale, maxRecordingOverlayScale, minRecordingOverlayScale } from "../domain/overlay";
import type { PermissionRequestState } from "../domain/permissions";
import { supportsDockVisibilityControl, supportsFnLongPressTrigger, supportsOutputVolumeDucking, supportsSoundSourceHotkeyFallback, supportsTraySingleClickRecording } from "../domain/platform";

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
  dataDir,
  audioDevicesRefreshing,
  onChange,
  onExportConfig,
  onImportConfig,
  onOpenDataDir,
  onChooseDataDir,
  onResetDataDir,
  onRefreshAudioDevices,
  inputMonitoringGranted,
  inputMonitoringPermission,
  onRefreshInputMonitoring,
  onRequestInputMonitoring,
  onApplyFnTrigger,
  importReport,
  canSave,
  canChangeDataDir,
  text,
}: {
  config: AppConfig;
  audioDevices: AudioInputDevice[];
  audioOutputDevices: AudioOutputDevice[];
  dataDir: DataDirInfo | null;
  audioDevicesRefreshing: boolean;
  onChange: (config: AppConfig) => void;
  onExportConfig: () => void;
  onImportConfig: (file: File) => void;
  onOpenDataDir: () => void;
  onChooseDataDir: () => void;
  onResetDataDir: () => void;
  onRefreshAudioDevices: () => void;
  inputMonitoringGranted: boolean | null;
  inputMonitoringPermission: PermissionRequestState;
  onRefreshInputMonitoring: () => void;
  onRequestInputMonitoring: () => void;
  onApplyFnTrigger: (enabled: boolean) => void;
  importReport: ConfigImportReport | null;
  canSave: boolean;
  canChangeDataDir: boolean;
  text: TextBundle;
}) {
  const importInputRef = useRef<HTMLInputElement | null>(null);
  const storageGb = Number((config.retention.max_storage_bytes / bytesPerGb).toFixed(2));
  const canControlDockVisibility = supportsDockVisibilityControl();
  const canUseFnLongPressTrigger = supportsFnLongPressTrigger();
  const canUseTraySingleClickRecording = supportsTraySingleClickRecording();
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
  const inputDevicePriority = config.audio.input_device_priority ?? [];
  const inputDeviceBlacklist = config.audio.input_device_blacklist ?? [];
  const addablePriorityDevices = audioDevices.filter(
    (device) => !containsInputDeviceRef(inputDevicePriority, device) && !containsInputDeviceRef(inputDeviceBlacklist, device),
  );
  const missingBlacklistedDevices = inputDeviceBlacklist.filter((blocked) => !findInputDevice(blocked, audioDevices));

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

  function resetOverlayAppearance() {
    onChange({
      ...config,
      ui: {
        ...config.ui,
        recording_overlay_scale: defaultRecordingOverlayScale,
        recording_overlay_offset_x: 0,
        recording_overlay_offset_y: 0,
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
    } else {
      onApplyFnTrigger(false);
    }
  }

  function updateInputDevicePolicy(priority: AudioInputDeviceRef[], blacklist: AudioInputDeviceRef[]) {
    const first = priority[0] ?? null;
    onChange({
      ...config,
      audio: {
        ...config.audio,
        input_device_mode: first ? "manual" : "system_default",
        input_device_id: first?.id || null,
        input_device_name: first?.name || null,
        input_device_priority: priority,
        input_device_blacklist: blacklist,
      },
    });
  }

  function addInputDevicePriority(id: string) {
    const device = audioDevices.find((item) => item.id === id);
    if (!device || containsInputDeviceRef(inputDevicePriority, device) || containsInputDeviceRef(inputDeviceBlacklist, device)) {
      return;
    }
    updateInputDevicePolicy([...inputDevicePriority, inputDeviceRef(device)], inputDeviceBlacklist);
  }

  function moveInputDevicePriority(index: number, direction: -1 | 1) {
    const target = index + direction;
    if (target < 0 || target >= inputDevicePriority.length) {
      return;
    }
    const next = [...inputDevicePriority];
    [next[index], next[target]] = [next[target], next[index]];
    updateInputDevicePolicy(next, inputDeviceBlacklist);
  }

  function removeInputDevicePriority(index: number) {
    updateInputDevicePolicy(inputDevicePriority.filter((_, itemIndex) => itemIndex !== index), inputDeviceBlacklist);
  }

  function toggleInputDeviceBlacklist(device: AudioInputDeviceRef, checked: boolean) {
    const nextBlacklist = checked
      ? uniqueInputDeviceRefs([...inputDeviceBlacklist, device])
      : inputDeviceBlacklist.filter((blocked) => !inputDeviceRefsMatch(blocked, device));
    const nextPriority = checked
      ? inputDevicePriority.filter((preferred) => !inputDeviceRefsMatch(preferred, device))
      : inputDevicePriority;
    updateInputDevicePolicy(nextPriority, nextBlacklist);
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
    <section className="panel page-stack settings-page">
      <PanelHeader title={text.settings.title} />

      <div className="settings-overview-grid">
        <div className="settings-section">
          <div className="section-title">
            <h2>{text.settings.languageSection}</h2>
          </div>
          <Field label={text.settings.language} className="field-medium">
            <select
              value={config.ui.app_language ?? "zh-CN"}
              onChange={(event) => updateLanguage(event.target.value)}
            >
              <option value="zh-CN">{text.settings.chinese}</option>
              <option value="en-US">{text.settings.english}</option>
            </select>
          </Field>
        </div>

        <div className="settings-section">
          <div className="section-title">
            <h2>{text.settings.system}</h2>
          </div>
          <div className="settings-toggle-stack">
            <label className="toggle-row">
              <input
                type="checkbox"
                checked={config.system.launch_at_login}
                onChange={(event) => onChange({ ...config, system: { ...config.system, launch_at_login: event.target.checked } })}
              />
              <span>{text.settings.launchAtLogin}</span>
            </label>
            {canControlDockVisibility ? (
              <label className="toggle-row">
                <input
                  type="checkbox"
                  checked={config.system.hide_dock_icon ?? false}
                  onChange={(event) => onChange({ ...config, system: { ...config.system, hide_dock_icon: event.target.checked } })}
                />
                <span>{text.settings.hideDockIcon}</span>
              </label>
            ) : null}
          </div>
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
        {canUseTraySingleClickRecording ? (
          <div className="trigger-options">
            <div className="toggle-with-help">
              <label className="toggle-row">
                <input
                  type="checkbox"
                  checked={config.system.tray_left_click_recording_enabled ?? true}
                  onChange={(event) =>
                    onChange({
                      ...config,
                      system: {
                        ...config.system,
                        tray_left_click_recording_enabled: event.target.checked,
                      },
                    })
                  }
                />
                <span>{text.settings.traySingleClickRecording}</span>
              </label>
              <HelpTip content={text.settings.traySingleClickRecordingHelp} />
            </div>
          </div>
        ) : null}
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
            <div className="fn-trigger-details">
              <Field label={text.settings.fnLongPressDuration} className="field-compact">
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
          </div>
        ) : null}
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.audio}</h2>
          <div className="section-actions">
            <button className="secondary small" type="button" disabled={audioDevicesRefreshing} onClick={onRefreshAudioDevices}>
              {audioDevicesRefreshing ? text.settings.audioRefreshingDevices : text.settings.audioRefreshDevices}
            </button>
          </div>
        </div>
        <div className="audio-settings-stack">
          <div className="settings-subsection audio-input-subsection">
            <div className="settings-subsection-heading">
              <h3>{text.settings.audioInput}</h3>
            </div>
            <div className="audio-input-policy">
              <Field label={text.settings.audioInputPriority} group>
                <div className="input-device-priority-list">
                  {inputDevicePriority.map((preferred, index) => {
                    const available = findInputDevice(preferred, audioDevices);
                    return (
                      <div className="input-device-priority-row" key={`${preferred.id}:${preferred.name}`}>
                        <span className="input-device-rank">{index + 1}</span>
                        <span className="input-device-name">
                          {available?.name ?? (preferred.name || preferred.id)}
                          {available?.is_default ? text.settings.audioDefaultBadge : ""}
                          {!available ? text.settings.audioPolicyMissingBadge : ""}
                        </span>
                        <button
                          className="secondary small input-device-order-button"
                          type="button"
                          disabled={index === 0}
                          title={text.settings.audioPriorityMoveUp}
                          aria-label={text.settings.audioPriorityMoveUp}
                          onClick={() => moveInputDevicePriority(index, -1)}
                        >
                          ↑
                        </button>
                        <button
                          className="secondary small input-device-order-button"
                          type="button"
                          disabled={index === inputDevicePriority.length - 1}
                          title={text.settings.audioPriorityMoveDown}
                          aria-label={text.settings.audioPriorityMoveDown}
                          onClick={() => moveInputDevicePriority(index, 1)}
                        >
                          ↓
                        </button>
                        <button className="secondary small" type="button" onClick={() => removeInputDevicePriority(index)}>
                          {text.settings.audioPriorityRemove}
                        </button>
                      </div>
                    );
                  })}
                  {inputDevicePriority.length === 0 ? <span>{text.settings.audioPriorityEmpty}</span> : null}
                  <select
                    aria-label={text.settings.audioPriorityAdd}
                    value=""
                    disabled={addablePriorityDevices.length === 0}
                    onChange={(event) => addInputDevicePriority(event.target.value)}
                  >
                    <option value="">{text.settings.audioPriorityAdd}</option>
                    {addablePriorityDevices.map((device) => (
                      <option key={device.id} value={device.id}>
                        {device.name}{device.is_default ? text.settings.audioDefaultBadge : ""}
                      </option>
                    ))}
                  </select>
                </div>
              </Field>

              <Field label={text.settings.audioInputBlacklist} group>
                <div className="device-checklist">
                  {audioDevices.map((device) => (
                    <label key={device.id} className="device-checklist-row">
                      <input
                        type="checkbox"
                        checked={containsInputDeviceRef(inputDeviceBlacklist, device)}
                        onChange={(event) => toggleInputDeviceBlacklist(inputDeviceRef(device), event.target.checked)}
                      />
                      <span>{device.name}{device.is_default ? text.settings.audioDefaultBadge : ""}</span>
                    </label>
                  ))}
                  {missingBlacklistedDevices.map((blocked) => (
                    <label key={`${blocked.id}:${blocked.name}`} className="device-checklist-row">
                      <input
                        type="checkbox"
                        checked
                        onChange={(event) => toggleInputDeviceBlacklist(blocked, event.target.checked)}
                      />
                      <span>{blocked.name || blocked.id}{text.settings.audioPolicyMissingBadge}</span>
                    </label>
                  ))}
                  {audioDevices.length === 0 && missingBlacklistedDevices.length === 0 ? <span>{text.settings.audioNoInputDevices}</span> : null}
                </div>
              </Field>
              <p className="settings-help-text">{text.settings.audioInputPolicyHelp}</p>
            </div>
          </div>

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
                  onChange={(event) => updateOutputVolumeDucking({ enabled: event.target.checked })}
                />
                <span>{text.settings.outputVolumeDuckingEnabled}</span>
              </label>
              {outputDucking.enabled && canDuckOutputVolume ? (
                <div className="output-ducking-options">
                  <div className="output-ducking-status">
                    <span className="status-chip">
                      {text.settings.outputVolumeDuckingCurrent(defaultOutputDevice?.name ?? text.settings.outputVolumeDuckingNoOutputDevice)}
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
                        onChange={(event) =>
                          updateOutputVolumeDucking({ mute_instead_of_reduce: event.target.checked })
                        }
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
                          onChange={(event) => updateOutputVolumeReduction(event.target.value)}
                        />
                        <input
                          type="number"
                          aria-label={text.settings.outputVolumeDuckingReduction}
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
                      <div className="toggle-with-help">
                        <label className="toggle-row">
                          <input
                            type="checkbox"
                            checked={outputDucking.sound_source_hotkey_fallback_enabled}
                            onChange={(event) => updateOutputVolumeDucking({ sound_source_hotkey_fallback_enabled: event.target.checked })}
                          />
                          <span>{text.settings.outputVolumeDuckingSoundSourceFallback}</span>
                        </label>
                        <HelpTip content={text.settings.outputVolumeDuckingSoundSourceHelp} />
                      </div>
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
                  <Field label={text.settings.outputVolumeDuckingWhitelist} group>
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
                      {audioOutputDevices.length === 0 ? <span>{text.settings.audioNoOutputDevices}</span> : null}
                      {outputDucking.device_name_whitelist.length === 0 ? <span>{text.settings.outputVolumeDuckingWhitelistAll}</span> : null}
                    </div>
                  </Field>
                </div>
              ) : (
                <p className="settings-help-text">
                  {canDuckOutputVolume ? text.settings.outputVolumeDuckingHelp : text.settings.outputVolumeDuckingUnsupported}
                </p>
              )}
            </fieldset>
          </div>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.settings.retention}</h2>
        </div>
        <div className="form-grid">
          <Field label={text.settings.maxRecords} className="field-compact">
            <input type="number" min="1" max={maxHistoryRecords} value={config.retention.max_history_records} onChange={(event) => updateMaxRecords(event.target.value)} />
          </Field>
          <Field label={text.settings.maxStorage} className="field-compact">
            <input type="number" min="0.01" max={maxStorageGb} step="0.01" value={storageGb} onChange={(event) => updateMaxStorageGb(event.target.value)} />
          </Field>
          <div className="field-wide data-dir-panel">
            <div className="data-dir-heading">
              <div>
                <strong>{text.settings.dataDirectory}</strong>
                <span>{text.settings.dataDirHelp}</span>
              </div>
              <span className="status-chip">
                {dataDir ? (dataDir.is_default ? text.settings.defaultDataDir : text.settings.customDataDir) : text.common.checking}
              </span>
            </div>
            <div className="data-dir-path" title={dataDir?.path ?? ""}>
              {dataDir?.path ?? text.common.checking}
            </div>
            <div className="section-actions data-dir-actions">
              <button className="secondary small" type="button" disabled={!canSave} onClick={onOpenDataDir}>
                {text.nav.openDataDir}
              </button>
              <button className="secondary small" type="button" disabled={!canChangeDataDir} onClick={onChooseDataDir}>
                {text.settings.changeDataDir}
              </button>
              <button className="secondary small" type="button" disabled={!canChangeDataDir || !dataDir || dataDir.is_default} onClick={onResetDataDir}>
                {text.settings.resetDataDir}
              </button>
            </div>
            {canSave && !canChangeDataDir ? <p className="settings-help-text warning">{text.settings.dataDirBusy}</p> : null}
          </div>
        </div>
      </div>

      <div className="settings-overview-grid settings-overview-grid-final">
        <div className="settings-section">
          <div className="section-title">
            <h2>{text.settings.overlayAppearance}</h2>
            <button className="secondary small" type="button" onClick={resetOverlayAppearance}>{text.settings.resetOverlay}</button>
          </div>
          <div className="overlay-control-row">
            <Field
              label={text.settings.overlayScale(Math.round((config.ui.recording_overlay_scale ?? defaultRecordingOverlayScale) * 200))}
              className="compact-range-field overlay-scale-control"
            >
              <div className="range-compact">
                <input
                  className="range-input"
                  type="range"
                  min={String(minRecordingOverlayScale)}
                  max={String(maxRecordingOverlayScale)}
                  step="0.05"
                  value={config.ui.recording_overlay_scale ?? defaultRecordingOverlayScale}
                  onChange={(event) => onChange({
                    ...config,
                    ui: {
                      ...config.ui,
                      recording_overlay_scale: Number(event.target.value),
                    },
                  })}
                />
              </div>
            </Field>
            <Field label={text.settings.offsetX} className="field-compact">
              <input
                type="number"
                min={-maxOverlayOffset}
                max={maxOverlayOffset}
                step="1"
                value={config.ui.recording_overlay_offset_x ?? 0}
                onChange={(event) => updateOverlayOffset("x", event.target.value)}
              />
            </Field>
            <Field label={text.settings.offsetY} className="field-compact">
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
          </div>
          <p className="settings-help-text">{text.settings.configPortabilityHelp}</p>
          <div className="section-actions settings-card-actions">
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
          {importReport ? <ConfigImportReportView report={importReport} text={text} /> : null}
        </div>
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

function inputDeviceRef(device: AudioInputDevice): AudioInputDeviceRef {
  return { id: device.id, name: device.name };
}

function inputDeviceRefsMatch(left: AudioInputDeviceRef, right: AudioInputDeviceRef) {
  return (Boolean(left.id) && left.id === right.id)
    || (Boolean(left.name) && Boolean(right.name) && left.name.toLocaleLowerCase() === right.name.toLocaleLowerCase());
}

function containsInputDeviceRef(items: AudioInputDeviceRef[], device: AudioInputDevice | AudioInputDeviceRef) {
  return items.some((item) => inputDeviceRefsMatch(item, device));
}

function findInputDevice(reference: AudioInputDeviceRef, devices: AudioInputDevice[]) {
  if (reference.id) {
    const exact = devices.find((device) => device.id === reference.id);
    if (exact) {
      return exact;
    }
  }
  return reference.name
    ? devices.find((device) => device.name.toLocaleLowerCase() === reference.name.toLocaleLowerCase()) ?? null
    : null;
}

function uniqueInputDeviceRefs(items: AudioInputDeviceRef[]) {
  const unique: AudioInputDeviceRef[] = [];
  for (const item of items) {
    if (!containsInputDeviceRef(unique, item)) {
      unique.push(item);
    }
  }
  return unique;
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
