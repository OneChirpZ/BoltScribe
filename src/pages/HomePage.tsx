import type { AppConfig, HistoryRecord, WorkflowStatus } from "../types";
import HistoryPage from "../components/HistoryPage";
import { activeHotkeys, displayShortcut } from "../domain/hotkeys";
import type { AppLanguage, TextBundle } from "../domain/i18n";
import { translateWorkflowMessage, workflowModeLabel } from "../domain/i18n";
import { providerLabel } from "../domain/providers";
import type { CorrectionSection } from "../domain/navigation";
import { parseCorrectionRulesText, parseDictionaryText } from "../domain/correctionText";

export default function HomePage({
  config,
  status,
  busy,
  history,
  inputDevicesChecked,
  hasInputDevice,
  audioDevicesRefreshing,
  onToggle,
  onCancel,
  onOpenPermissionGuide,
  onRefreshAudioDevices,
  onRefreshHistory,
  onOpenHistoryPage,
  onOpenModels,
  onOpenCorrectionSection,
  onCopyHistory,
  onRetryHistory,
  onDeleteHistory,
  canRetryHistory,
  canDeleteHistory,
  language,
  text,
}: {
  config: AppConfig;
  status: WorkflowStatus;
  busy: boolean;
  history: HistoryRecord[];
  inputDevicesChecked: boolean;
  hasInputDevice: boolean;
  audioDevicesRefreshing: boolean;
  onToggle: () => void;
  onCancel: () => void;
  onOpenPermissionGuide: () => void;
  onRefreshAudioDevices: () => void;
  onRefreshHistory: () => void;
  onOpenHistoryPage: () => void;
  onOpenModels: () => void;
  onOpenCorrectionSection: (section: CorrectionSection) => void;
  onCopyHistory: (text: string, label: string) => void;
  onRetryHistory: (record: HistoryRecord) => Promise<void>;
  onDeleteHistory: (record: HistoryRecord) => Promise<void>;
  canRetryHistory: boolean;
  canDeleteHistory: boolean;
  language: AppLanguage;
  text: TextBundle;
}) {
  return (
    <div className="home-stack">
      {inputDevicesChecked && !hasInputDevice ? (
        <div className="device-warning-banner">
          <div>
            <strong>{text.home.noInputDeviceTitle}</strong>
            <span>{text.home.noInputDeviceText}</span>
          </div>
          <button className="secondary small" type="button" disabled={audioDevicesRefreshing} onClick={onRefreshAudioDevices}>
            {audioDevicesRefreshing ? text.common.checking : text.home.refreshInputDevices}
          </button>
        </div>
      ) : null}
      <section className={`panel hero-panel status-${status.mode}`}>
        <div className="hero-status-row">
          <div>
            <p className="eyebrow">{text.home.currentStatus}</p>
            <h1>{workflowModeLabel(status.mode, language)}</h1>
            <p className="status-message">{translateWorkflowMessage(status.message || text.common.ready, language)}</p>
          </div>
          <div className="status-visual" aria-hidden="true">
            <span />
            <span />
            <span />
            <span />
            <span />
          </div>
        </div>
        <div className="action-row">
          <button
            className={status.mode === "recording" || status.mode === "processing" ? "primary danger" : "primary"}
            disabled={busy || status.mode === "starting"}
            onClick={status.mode === "processing" ? onCancel : onToggle}
          >
            {status.mode === "processing" ? text.overlay.cancel : status.mode === "recording" ? text.home.stopAndProcess : text.home.startRecording}
          </button>
          <button className="secondary" type="button" onClick={onOpenPermissionGuide}>{text.home.permissionGuide}</button>
          <div className="hotkey-list">
            {activeHotkeys(config).length === 0 ? (
              <div className="hotkey-pill muted">{text.home.hotkeyDisabled}</div>
            ) : (
              activeHotkeys(config).map((hotkey) => (
                <div className="hotkey-pill" key={hotkey}>{displayShortcut(hotkey)}</div>
              ))
            )}
          </div>
        </div>
        <div className="summary-grid">
          <Summary label={text.home.asr} value={language === "zh-CN" ? "火山引擎 WebSocket" : "Volcengine WebSocket"} onClick={onOpenModels} />
          <Summary
            label={text.home.llm}
            value={`${providerLabel(config.llm.provider)} / ${config.llm.model || text.home.unconfigured}`}
            detail={text.home.raceModeStatus(config.llm.race_enabled ?? false)}
            onClick={onOpenModels}
          />
          <Summary
            label={text.home.dictionaryItems}
            value={text.home.itemCount(parseDictionaryText(config.correction.dictionary_text ?? "").filter((line) => line.kind === "entry").length)}
            onClick={() => onOpenCorrectionSection("dictionary")}
          />
          <Summary
            label={text.home.correctionItems}
            value={text.home.itemCount(parseCorrectionRulesText(config.correction.correction_rules_text ?? "").filter((line) => line.kind === "rule").length)}
            onClick={() => onOpenCorrectionSection("rules")}
          />
        </div>
      </section>
      <HistoryPage title={text.home.latestHistory} history={history} onRefresh={onRefreshHistory} onOpenFullHistory={onOpenHistoryPage} onCopy={onCopyHistory} onRetry={onRetryHistory} onDelete={onDeleteHistory} canRetry={canRetryHistory} canDelete={canDeleteHistory} text={text} />
    </div>
  );
}

function Summary({ label, value, detail, onClick }: { label: string; value: string; detail?: string; onClick: () => void }) {
  return (
    <button className="summary-item" type="button" onClick={onClick} aria-label={`${label}：${value}${detail ? `，${detail}` : ""}`}>
      <span>{label}</span>
      <strong>{value}</strong>
      {detail ? <small>{detail}</small> : null}
    </button>
  );
}
