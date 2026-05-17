import { useRef, type ChangeEvent } from "react";
import type { AppConfig, AudioInputDevice, ConfigImportReport } from "../types";
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
  audioDevices,
  onChange,
  onSave,
  onExportConfig,
  onImportConfig,
  onRefreshAudioDevices,
  importReport,
  canSave,
  text,
}: {
  config: AppConfig;
  audioDevices: AudioInputDevice[];
  onChange: (config: AppConfig) => void;
  onSave: () => void;
  onExportConfig: () => void;
  onImportConfig: (file: File) => void;
  onRefreshAudioDevices: () => void;
  importReport: ConfigImportReport | null;
  canSave: boolean;
  text: TextBundle;
}) {
  const importInputRef = useRef<HTMLInputElement | null>(null);
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
