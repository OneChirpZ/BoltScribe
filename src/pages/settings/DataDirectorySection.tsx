import type { DataDirInfo } from "../../types";
import type { TextBundle } from "../../domain/i18n";

export default function DataDirectorySection({
  dataDir,
  canSave,
  canChangeDataDir,
  onOpenDataDir,
  onChooseDataDir,
  onResetDataDir,
  text,
}: {
  dataDir: DataDirInfo | null;
  canSave: boolean;
  canChangeDataDir: boolean;
  onOpenDataDir: () => void;
  onChooseDataDir: () => void;
  onResetDataDir: () => void;
  text: TextBundle;
}) {
  return (
    <div className="data-dir-panel">
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
        <button
          className="secondary small"
          type="button"
          disabled={!canChangeDataDir || !dataDir || dataDir.is_default}
          onClick={onResetDataDir}
        >
          {text.settings.resetDataDir}
        </button>
      </div>
      {canSave && !canChangeDataDir
        ? <p className="settings-help-text warning">{text.settings.dataDirBusy}</p>
        : null}
    </div>
  );
}
