import type { AppConfig } from "../types";
import Field from "../components/Field";
import PanelHeader from "../components/PanelHeader";
import { applyLanguageDefaultCorrectionTemplate } from "../domain/defaultCorrectionTemplates";
import type { AppLanguage, TextBundle } from "../domain/i18n";
import { supportsDockVisibilityControl } from "../domain/platform";

const maxHistoryRecords = 500;
const bytesPerGb = 1024 * 1024 * 1024;
const maxStorageGb = 2;
const maxOverlayOffset = 4000;

export default function SettingsPage({
  config,
  onChange,
  onSave,
  canSave,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  onSave: () => void;
  canSave: boolean;
  text: TextBundle;
}) {
  const storageGb = Number((config.retention.max_storage_bytes / bytesPerGb).toFixed(2));
  const canControlDockVisibility = supportsDockVisibilityControl();

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
