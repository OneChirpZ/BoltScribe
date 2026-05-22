import type { AppConfig, HistoryRecord, WorkflowStatus } from "../types";
import HistoryPage from "../components/HistoryPage";
import { activeHotkeys, displayShortcut } from "../domain/hotkeys";
import type { AppLanguage, TextBundle } from "../domain/i18n";
import { translateWorkflowMessage, workflowModeLabel } from "../domain/i18n";
import { providerLabel } from "../domain/providers";

export default function HomePage({
  config,
  status,
  busy,
  history,
  onToggle,
  onOpenPermissionGuide,
  onRefreshHistory,
  onOpenHistoryPage,
  onCopyHistory,
  language,
  text,
}: {
  config: AppConfig;
  status: WorkflowStatus;
  busy: boolean;
  history: HistoryRecord[];
  onToggle: () => void;
  onOpenPermissionGuide: () => void;
  onRefreshHistory: () => void;
  onOpenHistoryPage: () => void;
  onCopyHistory: (text: string, label: string) => void;
  language: AppLanguage;
  text: TextBundle;
}) {
  return (
    <div className="home-stack">
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
          <button className={status.mode === "recording" ? "primary danger" : "primary"} disabled={busy || status.mode === "processing"} onClick={onToggle}>
            {status.mode === "recording" ? text.home.stopAndProcess : text.home.startRecording}
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
          <Summary label={text.home.asr} value="火山引擎 WebSocket" />
          <Summary label={text.home.llm} value={`${providerLabel(config.llm.provider)} / ${config.llm.model || text.home.unconfigured}`} />
          <Summary label={text.home.dictionaryItems} value={text.home.itemCount(countNonEmptyLines(config.correction.dictionary_text))} />
          <Summary label="Thinking" value={config.llm.thinking_enabled ? config.llm.thinking_effort : text.common.closed} />
        </div>
      </section>
      <HistoryPage title={text.home.latestHistory} history={history} onRefresh={onRefreshHistory} onOpenFullHistory={onOpenHistoryPage} onCopy={onCopyHistory} text={text} />
    </div>
  );
}

function countNonEmptyLines(value: string) {
  return value.split(/\r?\n/).filter((line) => line.trim()).length;
}

function Summary({ label, value }: { label: string; value: string }) {
  return (
    <div className="summary-item">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
