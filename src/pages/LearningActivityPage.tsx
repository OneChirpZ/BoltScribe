import { useCallback, useEffect, useRef, useState } from "react";
import { appApi, type LearningApi } from "../app/api";
import { defaultLearningConfig } from "../domain/learning";
import { activityReasonLabel, activityStateLabel, learningActivityText, learningChangeText } from "../domain/learningActivity";
import { modelService } from "../domain/modelServices";
import type { AppConfig, LearningActivity, LearningActivityPage as ActivityPage, LearningChange, LearningChangeAction, LearningStatus } from "../types";
import { LearningChangeValues } from "../components/LearningChangeValues";

const pageSize = 30;
const changePageSize = 8;
type OpenRecord = (recordId: string, activityId?: string) => void;

export default function LearningActivityPage({ config, savedConfig = config, onChange, onOpenModels, onOpenRecord, recordId, onClearRecordFilter, api = appApi.learning }: {
  config: AppConfig;
  savedConfig?: AppConfig;
  onChange: (config: AppConfig) => void;
  onOpenModels: () => void;
  onOpenRecord: OpenRecord;
  recordId?: string;
  onClearRecordFilter?: () => void;
  api?: LearningApi;
}) {
  const language = config.ui.app_language;
  const text = learningActivityText(language);
  const changeText = learningChangeText(language);
  const learning = config.learning ?? defaultLearningConfig();
  const savedLearning = savedConfig.learning ?? defaultLearningConfig();
  const service = modelService(savedConfig, savedLearning.llm.service_id);
  const configured = Boolean(savedLearning.llm.model.trim() && service?.endpoint.trim() && service?.api_key.trim());
  const [activity, setActivity] = useState<ActivityPage>({ active: [], history: [], has_more: false });
  const [status, setStatus] = useState<LearningStatus | null>(null);
  const [pageIndex, setPageIndex] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [changes, setChanges] = useState<LearningChange[]>([]);
  const [changeLimit, setChangeLimit] = useState(changePageSize);
  const [changeError, setChangeError] = useState<string | null>(null);
  const [resolving, setResolving] = useState(false);
  const [now, setNow] = useState(Date.now);
  const helpDialog = useRef<HTMLDialogElement>(null);
  const helpButton = useRef<HTMLButtonElement>(null);
  const live = useRef(false);
  const generation = useRef(0);
  const changeRevision = useRef(0);
  const offset = pageIndex * pageSize;

  const refresh = useCallback(async () => {
    const request = ++generation.current;
    const revision = changeRevision.current;
    try {
      const results = await Promise.allSettled([
        api.loadActivity(pageSize, offset, recordId), api.getStatus(), api.loadChanges(),
      ]);
      if (!live.current || request !== generation.current) return;
      const [nextActivity, nextStatus, nextChanges] = results;
      if (nextActivity.status === "fulfilled") setActivity(nextActivity.value);
      if (nextStatus.status === "fulfilled") setStatus(nextStatus.value);
      if (nextChanges.status === "fulfilled" && revision === changeRevision.current) setChanges(nextChanges.value);
      setNow(Date.now());
      setError(results.flatMap((result) => result.status === "rejected" ? [String(result.reason)] : []).join("\n") || null);
    } catch (failure) {
      if (live.current && request === generation.current) setError(String(failure));
    } finally {
      if (live.current && request === generation.current) setLoading(false);
    }
  }, [api, offset, recordId]);

  useEffect(() => {
    setPageIndex(0);
    setChangeLimit(changePageSize);
    setChangeError(null);
    setActivity({ active: [], history: [], has_more: false });
  }, [recordId]);

  useEffect(() => {
    live.current = true;
    setLoading(true);
    let stopped = false;
    let unlisten: (() => void) | undefined;
    void api.listenUpdated(() => { void refresh(); }).then((stop) => {
      if (stopped) stop();
      else unlisten = stop;
    }).catch((failure) => {
      if (!stopped) setError(String(failure));
    }).finally(() => { if (!stopped) void refresh(); });
    return () => {
      stopped = true;
      live.current = false;
      generation.current += 1;
      unlisten?.();
    };
  }, [api, refresh]);

  useEffect(() => {
    if (activity.active.length === 0) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [activity.active.length]);

  const availability = !savedLearning.enabled ? text.disabled
    : status?.observation.supported === false ? text.unsupported
      : !configured ? text.unconfigured : activity.active.length ? text.running : text.waiting;

  function changePage(next: number) {
    setLoading(true);
    setPageIndex(next);
  }

  async function resolveChange(id: string, action: LearningChangeAction) {
    setResolving(true);
    setChangeError(null);
    try {
      const result = await api.resolveChange(id, action);
      changeRevision.current += 1;
      if (live.current) setChanges(result);
    } catch (failure) {
      if (live.current) setChangeError(String(failure));
    } finally {
      if (live.current) setResolving(false);
    }
  }

  const visibleActivity = new Set([...activity.active, ...activity.history].map((item) => item.id));
  const visibleChanges = changes.filter((change) => (
    !recordId || change.record_id === recordId || visibleActivity.has(change.activity_id ?? "")
  ));
  const orderedChanges = [
    ...visibleChanges.filter((change) => change.status === "pending"),
    ...visibleChanges.filter((change) => change.status !== "pending"),
  ];

  return (
    <section className="panel learning-activity-page">
      <header className="learning-activity-header">
        <div className="learning-activity-title">
          <h1>{text.title}</h1>
          <button ref={helpButton} type="button" className="secondary small" aria-haspopup="dialog"
            aria-controls="learning-help-dialog" onClick={() => helpDialog.current?.showModal()}>{text.help}</button>
        </div>
        <label className="checkbox-row">
          <input type="checkbox" checked={learning.enabled} disabled={status?.observation.supported === false && !learning.enabled}
            onChange={(event) => onChange({ ...config, learning: { ...learning, enabled: event.target.checked } })} />
          <span>{text.enabled}</span>
        </label>
      </header>
      <dialog ref={helpDialog} id="learning-help-dialog" className="learning-help-dialog"
        aria-labelledby="learning-help-title" onClose={() => helpButton.current?.focus()}>
        <header className="panel-header">
          <h2 id="learning-help-title">{text.helpTitle}</h2>
          <button type="button" className="secondary small" onClick={() => helpDialog.current?.close()}>{text.closeHelp}</button>
        </header>
        <p>{text.helpObserve}</p>
        <p>{text.helpSend}</p>
        <p>{text.helpStore}</p>
      </dialog>
      <div className="learning-activity-settings">
        <div>
          <strong>{availability}</strong>
          <span>{service && savedLearning.llm.model.trim() ? `${service.name} / ${savedLearning.llm.model}` : text.unconfigured}</span>
        </div>
        <button type="button" className="secondary small" onClick={onOpenModels}>{text.configure}</button>
      </div>
      {recordId ? (
        <div className="learning-record-filter">
          <span>{text.filtered}</span>
          {onClearRecordFilter ? <button type="button" className="secondary small" onClick={onClearRecordFilter}>{text.allRecords}</button> : null}
        </div>
      ) : null}
      {error ? <p className="learning-error" role="alert">{error}</p> : null}
      <section className="learning-activity-current" aria-labelledby="learning-current-title">
        <header className="learning-activity-current-heading">
          <h2 id="learning-current-title">{text.current}</h2>
          {activity.active.length === 0 ? <p className="learning-activity-idle" role="status">{loading ? text.loading : text.idle}</p> : null}
        </header>
        {activity.active.length ? <ul className="learning-activity-list">
          {activity.active.map((item) => <ActivityRow key={item.id} item={item} language={language} now={now} sourceRecordId={recordId} onOpenRecord={onOpenRecord} />)}
        </ul> : null}
      </section>
      {orderedChanges.length ? (
        <section aria-labelledby="learning-changes-title">
          <h2 id="learning-changes-title">{changeText.title}</h2>
          {changeError ? <p className="learning-error" role="alert">{changeError}</p> : null}
          <ul className="learning-changes-list">
            {orderedChanges.slice(0, changeLimit).map((change) => <ChangeRow key={change.id}
              change={change} language={language} sourceRecordId={recordId} resolving={resolving}
              onResolve={resolveChange} onOpenRecord={onOpenRecord} />)}
          </ul>
          {orderedChanges.length > changeLimit ? (
            <button className="secondary small" type="button" onClick={() => setChangeLimit(changeLimit + changePageSize)}>{changeText.more}</button>
          ) : null}
        </section>
      ) : null}
      <section className="learning-activity-history" aria-labelledby="learning-history-title">
        <header className="learning-activity-heading">
          <h2 id="learning-history-title">{text.history}</h2>
          <button className="secondary small" type="button" disabled={loading} onClick={() => { setLoading(true); void refresh(); }}>{text.refresh}</button>
        </header>
        {activity.history.length ? <ul className="learning-activity-list">
          {activity.history.map((item) => <ActivityRow key={item.id} item={item} language={language} now={now} sourceRecordId={recordId} onOpenRecord={onOpenRecord} />)}
        </ul> : <p className="learning-empty">{loading ? text.loading : text.empty}</p>}
        {pageIndex > 0 || activity.has_more ? <div className="history-pagination">
          <button type="button" className="secondary small" disabled={loading || pageIndex === 0} onClick={() => changePage(pageIndex - 1)}>{text.previous}</button>
          <span>{text.page(pageIndex + 1)}</span>
          <button type="button" className="secondary small" disabled={loading || !activity.has_more} onClick={() => changePage(pageIndex + 1)}>{text.next}</button>
        </div> : null}
      </section>
    </section>
  );
}

function ChangeRow({ change, language, sourceRecordId, resolving, onResolve, onOpenRecord }: {
  change: LearningChange;
  language: string;
  sourceRecordId?: string;
  resolving: boolean;
  onResolve: (id: string, action: LearningChangeAction) => Promise<void>;
  onOpenRecord: OpenRecord;
}) {
  const text = learningChangeText(language);
  const inactive = change.status === "applied" && change.effective === false;
  return (
    <li className="learning-change-row">
      <div className="learning-change-heading">
        <strong>{text.operations[change.operation]} · {text.kinds[change.kind]}</strong>
        <span aria-live="polite">{text[change.origin]} · {inactive ? text.inactive : text.states[change.status]}</span>
      </div>
      <LearningChangeValues change={change} language={language} />
      {change.reason ? <p className="learning-activity-reason">{change.reason}</p> : null}
      <div className="learning-change-actions">
        {change.status === "pending" ? <>
          <button className="secondary small" type="button" disabled={resolving} onClick={() => { void onResolve(change.id, "approve"); }}>{text.approve}</button>
          <button className="secondary small" type="button" disabled={resolving} onClick={() => { void onResolve(change.id, "dismiss"); }}>{text.dismiss}</button>
        </> : null}
        {change.status === "applied" && !inactive ? (
          <button className="secondary small" type="button" disabled={resolving} onClick={() => { void onResolve(change.id, "undo"); }}>{text.undo}</button>
        ) : null}
        <button className="secondary small" type="button" onClick={() => {
          onOpenRecord(sourceRecordId ?? change.record_id, change.activity_id ?? undefined);
        }}>{text.source}</button>
        <time dateTime={change.created_at}>{new Date(change.created_at).toLocaleString(language)}</time>
      </div>
      {inactive ? <p className="learning-activity-reason">{text.inactiveReason}</p> : null}
    </li>
  );
}

function ActivityRow({ item, language, now, sourceRecordId, onOpenRecord }: { item: LearningActivity; language: string; now: number; sourceRecordId?: string; onOpenRecord: OpenRecord }) {
  const text = learningActivityText(language);
  const active = item.finished_at === null;
  const startedAt = new Date(item.started_at).toLocaleString(language);
  const summary = item.state === "learned"
    ? (item.entries.length ? text.learned(item.entries.length) : text.updated)
    : item.reason === "already_covered" ? text.covered : activityStateLabel(item.state, language);
  const progress = item.state === "observing" ? text.observing : item.state === "queued" ? text.queued : text.analyzing;
  const seconds = Math.max(0, Math.floor((now - new Date(item.updated_at).getTime()) / 1000));
  const reason = activityReasonLabel(item.reason, language);
  const detailRecordId = sourceRecordId && item.additional_record_ids?.includes(sourceRecordId) ? sourceRecordId : item.record_id;
  return (
    <li className={`learning-activity-row learning-activity-${item.state}`}>
      <div className="learning-activity-result">
        <strong className="learning-activity-state">{summary}</strong>
        {active ? <p className="learning-activity-progress"><span className="learning-live-dot" aria-hidden="true" />{progress}<span className="learning-activity-elapsed">{text.elapsed(seconds)}</span></p> : null}
        {item.entries.length ? <ul className="learning-activity-terms">
          {item.entries.map((entry) => <li key={entry.id}>{entry.from ? <><span>{entry.from}</span><span aria-hidden="true">→</span></> : null}<strong>{entry.term}</strong></li>)}
        </ul> : null}
        {!active && reason && item.reason !== "no_change" ? <p className="learning-activity-reason">{reason}</p> : null}
        {item.error ? <p className="learning-error">{item.error}</p> : null}
      </div>
      <dl className="learning-activity-meta">
        <div className="learning-activity-started"><dt>{text.started}</dt><dd><time dateTime={item.started_at}>{startedAt}</time></dd></div>
        {item.finished_at ? <div className="learning-activity-finished"><dt>{text.finished}</dt><dd><time dateTime={item.finished_at}>{new Date(item.finished_at).toLocaleString(language)}</time></dd></div> : null}
        {item.model ? <div className="learning-activity-model"><dt>{text.model}</dt><dd>{[item.service_id, item.model].filter(Boolean).join(" / ")}</dd></div> : null}
      </dl>
      <button type="button" className="secondary small learning-activity-action" onClick={() => onOpenRecord(detailRecordId, item.id)} aria-label={`${text.details} · ${startedAt}`}>{text.details}</button>
    </li>
  );
}
