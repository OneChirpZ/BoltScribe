import { useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { HistoryRecord } from "../types";
import type { TextBundle } from "../domain/i18n";
import PanelHeader from "./PanelHeader";

export default function HistoryPage({
  title = "历史记录",
  history,
  onRefresh,
  onCopy,
  onRetry,
  onDelete,
  canRetry,
  canDelete,
  onOpenFullHistory,
  footer,
  text,
}: {
  title?: string;
  history: HistoryRecord[];
  onRefresh: () => void;
  onCopy: (text: string, label: string) => void;
  onRetry: (record: HistoryRecord) => Promise<void>;
  onDelete: (record: HistoryRecord) => Promise<void>;
  canRetry: boolean;
  canDelete: boolean;
  onOpenFullHistory?: () => void;
  footer?: ReactNode;
  text: TextBundle;
}) {
  const [logRecord, setLogRecord] = useState<HistoryRecord | null>(null);
  const [retryingId, setRetryingId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  async function requestRetry(record: HistoryRecord) {
    setRetryingId(record.id);
    try {
      await onRetry(record);
    } finally {
      setRetryingId(null);
    }
  }

  async function requestDelete(record: HistoryRecord) {
    const createdAt = new Date(record.created_at).toLocaleString();
    if (!window.confirm(text.history.deleteConfirm(createdAt))) {
      return;
    }
    setDeletingId(record.id);
    try {
      await onDelete(record);
    } finally {
      setDeletingId(null);
    }
  }

  const action = (
    <div className="history-header-actions">
      <button className="secondary small" onClick={onRefresh}>{text.common.refresh}</button>
      {onOpenFullHistory ? <button className="secondary small" onClick={onOpenFullHistory}>{text.common.allHistory}</button> : null}
    </div>
  );

  return (
    <section className="panel history-panel">
      <PanelHeader title={title} action={action} />
      <div className="history-list">
        {history.length === 0 ? <p className="empty">{text.common.emptyHistory}</p> : null}
        {history.map((record) => {
          const correctedText = record.corrected_text || record.pasted_text;
          const displayText = correctedText || record.raw_text || record.workflow_error || text.history.noText;
          const retryable = Boolean(
            record.workflow_error
            && record.audio_path
            && record.raw_text.trim() === ""
            && record.corrected_text.trim() === ""
            && record.pasted_text.trim() === "",
          );
          const retrying = retryingId === record.id;
          const deleting = deletingId === record.id;
          const mutating = retryingId !== null || deletingId !== null;
          return (
            <article className="history-item" key={record.id}>
              <div className="history-item-header">
                <div className="history-meta">
                  <span>{new Date(record.created_at).toLocaleString()}</span>
                  <span>{record.total_duration_ms} ms</span>
                  {record.workflow_error ? <span className="warning">{text.history.workflowFailed}</span> : null}
                  {record.correction_error ? <span className="warning">{text.history.correctionFallback}</span> : null}
                  {record.injection_error ? <span className="warning">{text.history.pasteFailed}</span> : null}
                  {!record.audio_path ? <span className="history-audio-cleared">{text.history.audioCleared}</span> : null}
                </div>
                <div className="history-actions">
                  <button className="secondary small history-action-log" type="button" onClick={() => setLogRecord(record)}>{text.history.viewLog}</button>
                  {retryable ? (
                    <button className="secondary small history-action-retry" type="button" disabled={!canRetry || mutating} onClick={() => { void requestRetry(record); }}>
                      {retrying ? text.history.retrying : text.history.retry}
                    </button>
                  ) : null}
                  <button className="secondary small history-action-corrected" type="button" disabled={!correctedText.trim()} onClick={() => onCopy(correctedText, text.history.correctedLabel)}>{text.history.copyCorrected}</button>
                  <button className="secondary small history-action-raw" type="button" disabled={!record.raw_text.trim()} onClick={() => onCopy(record.raw_text, text.history.rawLabel)}>{text.history.copyRaw}</button>
                  <button className="secondary small history-action-delete" type="button" disabled={!canDelete || mutating} onClick={() => { void requestDelete(record); }}>
                    {deleting ? text.history.deleting : text.history.delete}
                  </button>
                </div>
              </div>
              <p>{displayText}</p>
              <details>
                <summary>{text.history.rawTranscript}</summary>
                <pre>{record.raw_text}</pre>
              </details>
            </article>
          );
        })}
      </div>
      {footer}
      {logRecord ? createPortal(
        <div className="modal-backdrop" role="presentation" onClick={() => setLogRecord(null)}>
          <section className="history-log-dialog" role="dialog" aria-modal="true" aria-labelledby="history-log-title" onClick={(event) => event.stopPropagation()}>
            <div className="panel-header">
              <div>
                <p className="eyebrow">{text.history.logTitle}</p>
                <h1 id="history-log-title">{new Date(logRecord.created_at).toLocaleString()}</h1>
              </div>
              <button className="notice-close" type="button" onClick={() => setLogRecord(null)} aria-label={text.history.closeLog}>×</button>
            </div>
            <pre>{formatHistoryLog(logRecord)}</pre>
          </section>
        </div>,
        document.body,
      ) : null}
    </section>
  );
}

function formatHistoryLog(record: HistoryRecord) {
  return JSON.stringify({
    id: record.id,
    created_at: record.created_at,
    audio_path: record.audio_path,
    asr: {
      provider: record.asr_provider,
      task_id: record.asr_task_id,
      duration_ms: record.asr_duration_ms,
      service_audio_duration_ms: record.service_audio_duration_ms,
      live_diagnostics: record.live_asr_diagnostics ?? null,
    },
    audio: {
      started_at: record.audio_started_at,
      finished_at: record.audio_finished_at,
      sample_rate: record.audio_sample_rate,
      channels: record.audio_channels,
      sample_count: record.audio_sample_count,
    },
    correction: {
      enabled: record.correction_enabled,
      error: record.correction_error,
      calls: record.correction_logs ?? [],
    },
    injection: {
      error: record.injection_error,
    },
    workflow_error: record.workflow_error,
    total_duration_ms: record.total_duration_ms,
    raw_text: record.raw_text,
    corrected_text: record.corrected_text,
    pasted_text: record.pasted_text,
  }, null, 2);
}
