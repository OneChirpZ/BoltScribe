import Field from "../../components/Field";
import { formatByteCount } from "../../domain/historyMaintenance";
import type { SettingsPageProps } from "./types";
import { useRecordingCleanup } from "./useRecordingCleanup";

const maxHistoryRecords = 500;
const bytesPerGb = 1024 * 1024 * 1024;
const maxStorageGb = 2;

type RetentionDataSectionProps = Pick<
  SettingsPageProps,
  | "config"
  | "dataDir"
  | "onChange"
  | "onCleanupRecordingFiles"
  | "onPreviewRecordingCleanup"
  | "canCleanupRecordings"
  | "text"
>;

export default function RetentionDataSection(props: RetentionDataSectionProps) {
  const { config, dataDir, onChange, text } = props;
  const cleanup = useRecordingCleanup({
    dataDirPath: dataDir?.path,
    onCleanup: props.onCleanupRecordingFiles,
    onPreview: props.onPreviewRecordingCleanup,
  });
  const storageGb = Number((config.retention.max_storage_bytes / bytesPerGb).toFixed(2));

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

  async function cleanupOldRecordingFiles() {
    if (!cleanup.amountValid) {
      return;
    }
    const unitLabel = cleanup.unit === "day"
      ? text.settings.cleanupUnitDay
      : cleanup.unit === "week"
        ? text.settings.cleanupUnitWeek
        : text.settings.cleanupUnitMonth;
    if (window.confirm(text.settings.cleanupRecordingsConfirm(cleanup.amountNumber, unitLabel))) {
      await cleanup.runCleanup();
    }
  }

  return (
    <div className="settings-section">
      <div className="section-title">
        <h2>{text.settings.retention}</h2>
      </div>
      <div className="form-grid retention-settings-grid">
        <Field label={text.settings.maxRecords} className="field-compact">
          <input
            type="number"
            min="1"
            max={maxHistoryRecords}
            value={config.retention.max_history_records}
            onChange={(event) => updateMaxRecords(event.target.value)}
          />
        </Field>
        <Field label={text.settings.maxStorage} className="field-compact">
          <input
            type="number"
            min="0.01"
            max={maxStorageGb}
            step="0.01"
            value={storageGb}
            onChange={(event) => updateMaxStorageGb(event.target.value)}
          />
        </Field>
        <div className="field-wide recording-cleanup-panel">
          <div className="recording-cleanup-heading">
            <strong>{text.settings.cleanupRecordings}</strong>
            <span>{text.settings.cleanupRecordingsHelp}</span>
          </div>
          <div className="recording-cleanup-storage" role="status" aria-live="polite" aria-atomic="true">
            <div className="recording-cleanup-storage-item">
              <span>{text.settings.cleanupStorageCurrent}</span>
              <strong>
                {cleanup.previewError && !cleanup.preview
                  ? text.settings.cleanupStorageUnavailable
                  : cleanup.preview
                    ? formatByteCount(cleanup.preview.recording_bytes)
                    : text.common.checking}
              </strong>
              <small>{cleanup.preview ? text.settings.cleanupStorageFiles(cleanup.preview.recording_files) : ""}</small>
            </div>
            <div className="recording-cleanup-storage-item reclaimable">
              <span>{text.settings.cleanupStorageReclaimable}</span>
              <strong>
                {!cleanup.amountValid
                  ? "—"
                  : cleanup.previewError
                    ? text.settings.cleanupStorageUnavailable
                    : cleanup.preview
                      ? formatByteCount(cleanup.preview.eligible_bytes)
                      : text.common.checking}
              </strong>
              <small>
                {cleanup.amountValid && cleanup.preview
                  ? text.settings.cleanupStorageFiles(cleanup.preview.eligible_files)
                  : ""}
              </small>
            </div>
          </div>
          <div className="recording-cleanup-controls">
            <label>
              <span>{text.settings.cleanupAmount}</span>
              <input
                type="number"
                min="1"
                step="1"
                inputMode="numeric"
                value={cleanup.amount}
                onChange={(event) => cleanup.setAmount(event.target.value)}
              />
            </label>
            <label>
              <span>{text.settings.cleanupUnit}</span>
              <select value={cleanup.unit} onChange={(event) => cleanup.setUnit(event.target.value as typeof cleanup.unit)}>
                <option value="day">{text.settings.cleanupUnitDay}</option>
                <option value="week">{text.settings.cleanupUnitWeek}</option>
                <option value="month">{text.settings.cleanupUnitMonth}</option>
              </select>
            </label>
            <button
              className="secondary small recording-cleanup-button"
              type="button"
              disabled={!props.canCleanupRecordings || !cleanup.amountValid || cleanup.cleaning}
              onClick={() => { void cleanupOldRecordingFiles(); }}
            >
              {cleanup.cleaning ? text.settings.cleaningRecordings : text.settings.cleanupRecordingsAction}
            </button>
          </div>
          {cleanup.previewLoading && cleanup.preview
            ? <p className="settings-help-text">{text.settings.cleanupStorageRefreshing}</p>
            : null}
          {!props.canCleanupRecordings
            ? <p className="settings-help-text warning">{text.settings.cleanupRecordingsBusy}</p>
            : null}
        </div>
      </div>
    </div>
  );
}

function clampInt(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) {
    return min;
  }
  return Math.min(max, Math.max(min, Math.round(value)));
}

function clampNumber(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) {
    return min;
  }
  return Math.min(max, Math.max(min, value));
}
