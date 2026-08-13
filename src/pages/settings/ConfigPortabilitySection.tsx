import { useRef, type ChangeEvent } from "react";
import type { TextBundle } from "../../domain/i18n";
import type { ConfigImportReport } from "../../types";

export default function ConfigPortabilitySection({
  canSave,
  importReport,
  onExportConfig,
  onImportConfig,
  text,
}: {
  canSave: boolean;
  importReport: ConfigImportReport | null;
  onExportConfig: () => void;
  onImportConfig: (file: File) => void;
  text: TextBundle;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);

  function importSelectedFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (file) {
      onImportConfig(file);
    }
  }

  return (
    <div className="settings-section">
      <div className="section-title">
        <h2>{text.settings.configPortability}</h2>
      </div>
      <p className="settings-help-text">{text.settings.configPortabilityHelp}</p>
      <div className="section-actions settings-card-actions">
        <button className="secondary small" type="button" disabled={!canSave} onClick={onExportConfig}>
          {text.settings.exportConfig}
        </button>
        <button className="secondary small" type="button" disabled={!canSave} onClick={() => inputRef.current?.click()}>
          {text.settings.importConfig}
        </button>
        <input
          ref={inputRef}
          className="hidden-file-input"
          type="file"
          accept="application/json,.json"
          onChange={importSelectedFile}
        />
      </div>
      {importReport ? <ConfigImportReportView report={importReport} text={text} /> : null}
    </div>
  );
}

function ConfigImportReportView({ report, text }: { report: ConfigImportReport; text: TextBundle }) {
  const hasDetails =
    report.missing_fields.length > 0
    || report.unknown_fields.length > 0
    || report.invalid_fields.length > 0
    || report.notes.length > 0;

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
        {items.map((item) => <li key={item}>{item}</li>)}
      </ul>
    </div>
  );
}
