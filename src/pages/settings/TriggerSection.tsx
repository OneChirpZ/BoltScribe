import Field from "../../components/Field";
import HelpTip from "../../components/HelpTip";
import ShortcutPicker from "../../components/ShortcutPicker";
import { hotkeyEnabledSlots, hotkeySlots, updateHotkey, updateHotkeyEnabled } from "../../domain/hotkeys";
import type { TextBundle } from "../../domain/i18n";
import type { PermissionRequestState } from "../../domain/permissions";
import { supportsFnLongPressTrigger, supportsTraySingleClickRecording } from "../../domain/platform";
import type { AppConfig } from "../../types";

const minFnLongPressDurationMs = 50;
const maxFnLongPressDurationMs = 5000;

export default function TriggerSection({
  config,
  onChange,
  inputMonitoringGranted,
  inputMonitoringPermission,
  onRefreshInputMonitoring,
  onRequestInputMonitoring,
  onApplyFnTrigger,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  inputMonitoringGranted: boolean | null;
  inputMonitoringPermission: PermissionRequestState;
  onRefreshInputMonitoring: () => void;
  onRequestInputMonitoring: () => void;
  onApplyFnTrigger: (enabled: boolean) => void;
  text: TextBundle;
}) {
  const shortcutSlots = hotkeySlots(config);
  const shortcutEnabledSlots = hotkeyEnabledSlots(config);

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

  return (
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
      {supportsTraySingleClickRecording() ? (
        <div className="trigger-options">
          <div className="toggle-with-help">
            <label className="toggle-row">
              <input
                type="checkbox"
                checked={config.system.tray_left_click_recording_enabled ?? true}
                onChange={(event) => onChange({
                  ...config,
                  system: {
                    ...config.system,
                    tray_left_click_recording_enabled: event.target.checked,
                  },
                })}
              />
              <span>{text.settings.traySingleClickRecording}</span>
            </label>
            <HelpTip content={text.settings.traySingleClickRecordingHelp} />
          </div>
        </div>
      ) : null}
      {supportsFnLongPressTrigger() ? (
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
                  <button className="secondary small" type="button" onClick={onRefreshInputMonitoring}>
                    {text.permission.recheck}
                  </button>
                  <button
                    className="secondary small"
                    type="button"
                    onClick={onRequestInputMonitoring}
                    disabled={inputMonitoringPermission === "checking"}
                  >
                    {inputMonitoringPermission === "checking" ? text.common.checking : text.permission.requestInputMonitoring}
                  </button>
                </div>
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function inputMonitoringPermissionLabel(
  status: PermissionRequestState,
  granted: boolean | null,
  text: TextBundle,
): string {
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

function clampInt(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) {
    return min;
  }
  return Math.min(max, Math.max(min, Math.round(value)));
}
