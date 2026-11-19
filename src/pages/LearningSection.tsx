import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { appApi, type LearningApi } from "../app/api";
import { LearningChangeValues } from "../components/LearningChangeValues";
import { defaultLearningConfig, learningText } from "../domain/learning";
import { learningChangeText } from "../domain/learningActivity";
import { modelManagerText, modelService } from "../domain/modelServices";
import type { AppConfig, LearnedEntry, LearningChange, LearningStatus } from "../types";

export default function LearningSection({ config, onChange, onOpenModels, onOpenLearning, onOpenRecord, kind = "dictionary", api = appApi.learning }: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  api?: LearningApi;
  onOpenModels?: () => void;
  onOpenLearning?: () => void;
  onOpenRecord?: (id: string) => void;
  kind?: "dictionary" | "rule";
}) {
  const language = config.ui.app_language;
  const text = learningText(language);
  const changeText = learningChangeText(language);
  const showSettings = kind === "dictionary";
  const heading = showSettings ? text.terms : text.rules;
  const searchLabel = showSettings ? text.search : text.searchRules;
  const emptyLabel = showSettings ? text.empty : text.emptyRules;
  const learning = config.learning ?? defaultLearningConfig();
  const model = learning.llm;
  const service = modelService(config, model.service_id);
  const modelText = modelManagerText(language);
  const [entries, setEntries] = useState<LearnedEntry[]>([]);
  const [changes, setChanges] = useState<LearningChange[]>([]);
  const [status, setStatus] = useState<LearningStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [mutating, setMutating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const live = useRef(false);
  const generation = useRef(0);

  const refresh = useCallback(async () => {
    const request = ++generation.current;
    try {
      const [nextEntries, nextStatus, nextChanges] = await Promise.all([
        api.loadEntries(), showSettings ? api.getStatus() : Promise.resolve(null), api.loadChanges(),
      ]);
      if (!live.current || request !== generation.current) return;
      setEntries(nextEntries);
      setStatus(nextStatus);
      setChanges(nextChanges);
      setError(null);
    } catch (failure) {
      if (live.current && request === generation.current) setError(String(failure));
    } finally {
      if (live.current && request === generation.current) setLoading(false);
    }
  }, [api, showSettings]);

  useEffect(() => {
    live.current = true;
    let stopped = false;
    let stopListening: (() => void) | undefined;
    void api.listenUpdated(() => { void refresh(); }).then((stop) => {
      if (stopped) stop();
      else stopListening = stop;
    }).catch((failure) => {
      if (!stopped) setError(String(failure));
    }).finally(() => {
      if (!stopped) void refresh();
    });
    return () => {
      stopped = true;
      live.current = false;
      generation.current += 1;
      stopListening?.();
    };
  }, [api, refresh]);

  async function mutate(entry: LearnedEntry, remove: boolean) {
    setMutating(true);
    setNotice(null);
    setError(null);
    generation.current += 1;
    try {
      const nextEntries = remove
        ? await api.deleteEntry(entry.id)
        : await api.setEntryEnabled(entry.id, !entry.enabled);
      if (!live.current) return;
      setEntries(nextEntries);
      if (remove) setNotice(text.deleted);
      await refresh();
    } catch (failure) {
      if (live.current) setError(String(failure));
    } finally {
      if (live.current) setMutating(false);
    }
  }

  const entriesForKind = useMemo(() => entries.filter((entry) => {
    if (!entry.kind || entry.kind === "legacy") return kind === "dictionary" || Boolean(entry.from);
    return entry.kind === kind;
  }), [entries, kind]);
  const filtered = useMemo(() => {
    const key = query.trim().toLocaleLowerCase();
    return entriesForKind.filter((entry) => `${entry.term}\n${entry.from ?? ""}`.toLocaleLowerCase().includes(key));
  }, [entriesForKind, query]);
  const appliedManual = changes.filter((change) => change.kind === kind && change.origin === "manual"
    && change.status === "applied" && change.effective === true);

  return (
    <section className="dictionary-source-card learning-section" aria-label={heading}>
      <header className="learning-entries-heading">
        <h2>{heading} <span className="learning-count">{text.count(entriesForKind.length)}</span></h2>
        {showSettings ? <label className="checkbox-row">
          <input type="checkbox" checked={learning.enabled} disabled={status?.observation.supported !== true && !learning.enabled} onChange={(event) => {
            onChange({ ...config, learning: { ...learning, enabled: event.target.checked } });
          }} />
          <span>{text.enabled}</span>
        </label> : null}
      </header>
      <div className="learning-search-row">
        <input type="search" className="learning-search" aria-label={searchLabel} placeholder={searchLabel} value={query} onChange={(event) => setQuery(event.target.value)} />
        <button type="button" className="secondary small" disabled={loading || mutating} onClick={() => { void refresh(); }}>{text.refresh}</button>
      </div>
      {onOpenLearning ? <button className="secondary small learning-activity-link" type="button" onClick={onOpenLearning}>{language === "en-US" ? "Learning activity" : "查看学习动态"}</button> : null}
      {status?.processing ? <span role="status">{text.processing}</span> : status?.pending ? <span role="status">{text.pending}</span> : null}
      {error ? <p className="learning-error" role="alert">{error}</p> : null}
      {status?.last_error ? <p className="learning-error" role="alert">{status.last_error}</p> : null}
      {notice ? <p className="learning-help" role="status">{notice}</p> : null}
      {loading ? <p className="learning-empty" role="status">{text.loading}</p> : filtered.length === 0 ? <p className="learning-empty">{entriesForKind.length ? text.noResults : emptyLabel}</p> : (
        <ul className="learning-entry-list">
          {filtered.map((entry) => (
            <li key={entry.id} className={entry.enabled ? "learning-entry" : "learning-entry disabled"}>
              <div className="learning-entry-content">
                <div className="learning-entry-spelling">
                  {entry.from ? <><span className="learning-entry-from" aria-label={text.source}>{entry.from}</span><span aria-hidden="true">→</span></> : null}
                  <strong aria-label={text.term}>{entry.term}</strong>
                </div>
                <div className="learning-entry-date">
                  <time dateTime={entry.created_at} title={text.created}>{new Date(entry.created_at).toLocaleString(language)}</time>
                  {!entry.kind || entry.kind === "legacy" ? <span title={entry.from ? text.legacyHelp : undefined}>{changeText.kinds.legacy}</span> : null}
                </div>
              </div>
              <div className="learning-entry-actions">
                {onOpenRecord && entry.record_id ? <button type="button" className="secondary small" onClick={() => onOpenRecord(entry.record_id)}>{changeText.source}</button> : null}
                <label className="checkbox-row">
                  <input type="checkbox" aria-label={`${text.enabledTerm} ${entry.term}`} checked={entry.enabled} disabled={mutating} onChange={() => { void mutate(entry, false); }} />
                  <span>{entry.enabled ? text.enabledTerm : text.disabledTerm}</span>
                </label>
                <button type="button" className="secondary small" aria-label={`${text.remove} ${entry.term}`} disabled={mutating} onClick={() => { void mutate(entry, true); }}>{text.remove}</button>
              </div>
            </li>
          ))}
        </ul>
      )}
      {appliedManual.length ? <aside className="learning-manual-changes" aria-label={changeText.appliedManual}>
        <h3>{changeText.appliedManual}</h3>
        <ul className="learning-changes-list">
          {appliedManual.map((change) => <li key={change.id}><LearningChangeValues change={change} language={language} /></li>)}
        </ul>
      </aside> : null}
      {showSettings ? <footer className="learning-settings">
        <div className="learning-model-reference">
          <div><strong>{text.modelTitle}</strong><span>{service && model.model.trim() ? `${service.name} / ${model.model}` : modelText.unconfigured}</span></div>
          {onOpenModels ? <button className="secondary small" type="button" onClick={onOpenModels}>{modelText.configure}</button> : null}
        </div>

        <p className="learning-help">{text.enabledHelp}</p>
        {learning.enabled && (!model.model.trim() || !service?.api_key.trim() || !service?.endpoint.trim())
          ? <p className="learning-help">{text.modelRequired}</p> : null}
        {status?.observation.supported === false ? <p className="learning-help">{text.unsupported}</p> : null}
      </footer> : null}
    </section>
  );
}
