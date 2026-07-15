import { useEffect, useId, useMemo, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import type { TextBundle } from "../domain/i18n";
import {
  appendTextLine,
  parseCorrectionRulesText,
  parseDictionaryText,
  removeTextLine,
  replaceTextLine,
  serializeCorrectionRule,
  serializeDictionaryItem,
  type CorrectionRuleLine,
  type DictionaryLine,
  type TextLineRange,
} from "../domain/correctionText";

type EditorMode = "items" | "text";

export default function CorrectionLineEditor({
  kind,
  value,
  onChange,
  text,
}: {
  kind: "dictionary" | "rules";
  value: string;
  onChange: (value: string) => void;
  text: TextBundle;
}) {
  const [mode, setMode] = useState<EditorMode>("items");
  const [pendingSelection, setPendingSelection] = useState<TextLineRange | null>(null);
  const bulkEditorRef = useRef<HTMLTextAreaElement>(null);
  const modeTabsId = useId();
  const dictionaryLines = useMemo(() => kind === "dictionary" ? parseDictionaryText(value) : [], [kind, value]);
  const ruleLines = useMemo(() => kind === "rules" ? parseCorrectionRulesText(value) : [], [kind, value]);
  const dictionaryEntries = dictionaryLines.filter((line): line is Extract<DictionaryLine, { kind: "entry" }> => line.kind === "entry");
  const rules = ruleLines.filter((line): line is Extract<CorrectionRuleLine, { kind: "rule" }> => line.kind === "rule");
  const unparsedRules = ruleLines.filter((line): line is Extract<CorrectionRuleLine, { kind: "unparsed" }> => line.kind === "unparsed");
  const duplicateTerms = duplicateKeys(dictionaryEntries.map((line) => line.term));
  const duplicateRuleSources = duplicateKeys(rules.map((line) => line.source));

  useEffect(() => {
    if (mode !== "text" || !pendingSelection) {
      return;
    }
    const frame = window.requestAnimationFrame(() => {
      const editor = bulkEditorRef.current;
      editor?.focus();
      editor?.setSelectionRange(pendingSelection.start, pendingSelection.contentEnd);
      setPendingSelection(null);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [mode, pendingSelection]);

  function openTextLine(range: TextLineRange) {
    setPendingSelection(range);
    setMode("text");
  }

  return (
    <div className="correction-line-editor">
      <div className="correction-editor-toolbar">
        <ModeSwitch idPrefix={modeTabsId} mode={mode} onChange={setMode} text={text} />
        <span className="correction-item-count">
          {text.correction.itemCount(kind === "dictionary" ? dictionaryEntries.length : rules.length)}
        </span>
      </div>

      {mode === "text" ? (
        <div
          className="correction-bulk-panel"
          id={`${modeTabsId}-text-panel`}
          role="tabpanel"
          aria-labelledby={`${modeTabsId}-text-tab`}
        >
          <label className="correction-fill-field">
            <span className="field-label">
              {kind === "dictionary" ? text.correction.dictionaryTextLabel : text.correction.rulesTextLabel}
            </span>
            <textarea
              ref={bulkEditorRef}
              className="correction-bulk-editor"
              value={value}
              spellCheck={false}
              onChange={(event) => onChange(event.target.value)}
            />
          </label>
          <p className="correction-format-help">
            {kind === "dictionary" ? text.correction.dictionaryHelp : text.correction.correctionRulesHelp}
          </p>
        </div>
      ) : (
        <div
          className="correction-items-panel"
          id={`${modeTabsId}-items-panel`}
          role="tabpanel"
          aria-labelledby={`${modeTabsId}-items-tab`}
        >
          {kind === "dictionary" ? (
            <DictionaryItems
              value={value}
              entries={dictionaryEntries}
              duplicateTerms={duplicateTerms}
              onChange={onChange}
              text={text}
            />
          ) : (
            <RuleItems
              value={value}
              rules={rules}
              unparsedRules={unparsedRules}
              duplicateSources={duplicateRuleSources}
              onChange={onChange}
              onOpenTextLine={openTextLine}
              text={text}
            />
          )}
        </div>
      )}
    </div>
  );
}

function ModeSwitch({
  idPrefix,
  mode,
  onChange,
  text,
}: {
  idPrefix: string;
  mode: EditorMode;
  onChange: (mode: EditorMode) => void;
  text: TextBundle;
}) {
  const modes: EditorMode[] = ["items", "text"];

  function handleKeyDown(event: KeyboardEvent<HTMLButtonElement>, currentMode: EditorMode) {
    const currentIndex = modes.indexOf(currentMode);
    let nextIndex = currentIndex;
    if (event.key === "ArrowRight") {
      nextIndex = (currentIndex + 1) % modes.length;
    } else if (event.key === "ArrowLeft") {
      nextIndex = (currentIndex - 1 + modes.length) % modes.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = modes.length - 1;
    } else {
      return;
    }
    event.preventDefault();
    onChange(modes[nextIndex]);
    const tabList = event.currentTarget.parentElement;
    window.requestAnimationFrame(() => {
      const tabs = tabList?.querySelectorAll<HTMLButtonElement>("button[role='tab']");
      tabs?.[nextIndex]?.focus();
    });
  }

  return (
    <div className="segmented-control correction-mode-switch" role="tablist" aria-label={text.correction.modeSwitchLabel}>
      {modes.map((item) => (
        <button
          key={item}
          className={mode === item ? "active" : ""}
          type="button"
          role="tab"
          id={`${idPrefix}-${item}-tab`}
          aria-controls={`${idPrefix}-${item}-panel`}
          aria-selected={mode === item}
          tabIndex={mode === item ? 0 : -1}
          onClick={() => onChange(item)}
          onKeyDown={(event) => handleKeyDown(event, item)}
        >
          {item === "items" ? text.correction.itemMode : text.correction.textMode}
        </button>
      ))}
    </div>
  );
}

function DictionaryItems({
  value,
  entries,
  duplicateTerms,
  onChange,
  text,
}: {
  value: string;
  entries: Array<Extract<DictionaryLine, { kind: "entry" }>>;
  duplicateTerms: Set<string>;
  onChange: (value: string) => void;
  text: TextBundle;
}) {
  const [newTerm, setNewTerm] = useState("");
  const [addError, setAddError] = useState("");
  const addInputRef = useRef<HTMLInputElement>(null);

  function addTerm(event: FormEvent) {
    event.preventDefault();
    const term = serializeDictionaryItem(newTerm);
    if (!term) {
      setAddError(text.correction.termRequired);
      return;
    }
    onChange(appendTextLine(value, term));
    setNewTerm("");
    setAddError("");
    addInputRef.current?.focus();
  }

  return (
    <>
      <form className="dictionary-add-form" onSubmit={addTerm}>
        <label>
          <span className="field-label">{text.correction.addTerm}</span>
          <input
            ref={addInputRef}
            value={newTerm}
            placeholder={text.correction.termPlaceholder}
            onChange={(event) => {
              setNewTerm(event.target.value);
              if (addError) setAddError("");
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                setNewTerm("");
                setAddError("");
              }
            }}
          />
        </label>
        <button className="primary small" type="submit">{text.correction.addTerm}</button>
        {addError ? <span className="field-error">{addError}</span> : null}
      </form>
      <p className="correction-format-help">{text.correction.dictionaryItemHelp}</p>
      <div className="dictionary-chip-list" role="list">
        {entries.length === 0 ? <div className="structured-empty">{text.correction.emptyDictionary}</div> : null}
        {entries.map((line) => (
          <DictionaryRow
            key={line.range.index}
            line={line}
            duplicate={duplicateTerms.has(normalizedKey(line.term))}
            onDelete={() => onChange(removeTextLine(value, line.range))}
            text={text}
          />
        ))}
      </div>
    </>
  );
}

function DictionaryRow({
  line,
  duplicate,
  onDelete,
  text,
}: {
  line: Extract<DictionaryLine, { kind: "entry" }>;
  duplicate: boolean;
  onDelete: () => void;
  text: TextBundle;
}) {
  const duplicateDescriptionId = `${line.range.index}-duplicate-term`;

  return (
    <span className={duplicate ? "dictionary-chip duplicate" : "dictionary-chip"} role="listitem">
      <span className="dictionary-chip-text" title={line.term}>{line.term}</span>
      {duplicate ? <span className="visually-hidden" id={duplicateDescriptionId}>{text.correction.duplicateTerm}</span> : null}
      <button
        className="dictionary-chip-remove"
        type="button"
        onClick={onDelete}
        aria-label={text.correction.deleteTerm(line.term)}
        aria-describedby={duplicate ? duplicateDescriptionId : undefined}
      >
        ×
      </button>
    </span>
  );
}

function RuleItems({
  value,
  rules,
  unparsedRules,
  duplicateSources,
  onChange,
  onOpenTextLine,
  text,
}: {
  value: string;
  rules: Array<Extract<CorrectionRuleLine, { kind: "rule" }>>;
  unparsedRules: Array<Extract<CorrectionRuleLine, { kind: "unparsed" }>>;
  duplicateSources: Set<string>;
  onChange: (value: string) => void;
  onOpenTextLine: (range: TextLineRange) => void;
  text: TextBundle;
}) {
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [note, setNote] = useState("");
  const [addError, setAddError] = useState("");
  const sourceRef = useRef<HTMLInputElement>(null);

  function addRule(event: FormEvent) {
    event.preventDefault();
    if (!source.trim()) {
      setAddError(text.correction.sourceRequired);
      return;
    }
    if (!target.trim()) {
      setAddError(text.correction.targetRequired);
      return;
    }
    onChange(appendTextLine(value, serializeCorrectionRule({ source, target, note })));
    setSource("");
    setTarget("");
    setNote("");
    setAddError("");
    sourceRef.current?.focus();
  }

  return (
    <>
      <form className="rule-add-form" onSubmit={addRule}>
        <label>
          <span className="field-label">{text.correction.sourceLabel}</span>
          <input ref={sourceRef} value={source} placeholder={text.correction.sourcePlaceholder} onChange={(event) => { setSource(event.target.value); setAddError(""); }} />
        </label>
        <label>
          <span className="field-label">{text.correction.targetLabel}</span>
          <input value={target} placeholder={text.correction.targetPlaceholder} onChange={(event) => { setTarget(event.target.value); setAddError(""); }} />
        </label>
        <label>
          <span className="field-label">{text.correction.noteLabel}</span>
          <input value={note} placeholder={text.correction.notePlaceholder} onChange={(event) => setNote(event.target.value)} />
        </label>
        <button className="primary small" type="submit">{text.correction.addRule}</button>
        {addError ? <span className="field-error">{addError}</span> : null}
      </form>
      <p className="correction-format-help">{text.correction.rulesItemHelp}</p>
      {unparsedRules.length > 0 ? (
        <div className="unparsed-lines-warning" role="status">
          <div>
            <strong>{text.correction.unparsedTitle(unparsedRules.length)}</strong>
            <span>{text.correction.unparsedHelp}</span>
          </div>
          <div className="unparsed-line-actions">
            {unparsedRules.map((line) => (
              <button key={line.range.index} className="secondary small" type="button" onClick={() => onOpenTextLine(line.range)}>
                {text.correction.editTextLine(line.range.index + 1)}
              </button>
            ))}
          </div>
        </div>
      ) : null}
      <div className="structured-item-list" role="list">
        {rules.length === 0 ? <div className="structured-empty">{text.correction.emptyRules}</div> : null}
        {rules.map((line) => (
          <RuleRow
            key={line.range.index}
            line={line}
            duplicate={duplicateSources.has(normalizedKey(line.source))}
            onReplace={(nextLine) => onChange(replaceTextLine(value, line.range, nextLine))}
            onDelete={() => onChange(removeTextLine(value, line.range))}
            text={text}
          />
        ))}
      </div>
    </>
  );
}

function RuleRow({
  line,
  duplicate,
  onReplace,
  onDelete,
  text,
}: {
  line: Extract<CorrectionRuleLine, { kind: "rule" }>;
  duplicate: boolean;
  onReplace: (nextLine: string) => void;
  onDelete: () => void;
  text: TextBundle;
}) {
  const [source, setSource] = useState(line.source);
  const [target, setTarget] = useState(line.target);
  const [note, setNote] = useState(line.note);

  useEffect(() => {
    setSource(line.source);
    setTarget(line.target);
    setNote(line.note);
  }, [line.range.raw]);

  function updateRule(nextSource: string, nextTarget: string, nextNote: string) {
    onReplace(serializeCorrectionRule({ source: nextSource, target: nextTarget, note: nextNote }));
  }

  return (
    <div className="structured-item-row rule-item-row" role="listitem">
      <span className="structured-line-number">{text.correction.lineNumber(line.range.index + 1)}</span>
      <label>
        <span className="field-label">{text.correction.sourceLabel}</span>
        <input
          value={source}
          aria-invalid={!source.trim()}
          onChange={(event) => {
            const nextSource = event.target.value;
            setSource(nextSource);
            updateRule(nextSource, target, note);
          }}
        />
      </label>
      <span className="rule-arrow" aria-hidden="true">→</span>
      <label>
        <span className="field-label">{text.correction.targetLabel}</span>
        <input
          value={target}
          aria-invalid={!target.trim()}
          onChange={(event) => {
            const nextTarget = event.target.value;
            setTarget(nextTarget);
            updateRule(source, nextTarget, note);
          }}
        />
      </label>
      <label>
        <span className="field-label">{text.correction.noteLabel}</span>
        <input
          value={note}
          onChange={(event) => {
            const nextNote = event.target.value;
            setNote(nextNote);
            updateRule(source, target, nextNote);
          }}
        />
      </label>
      <button className="icon-button" type="button" onClick={onDelete} aria-label={text.correction.deleteRule(line.source, line.target)}>×</button>
      {!source.trim() ? <span className="structured-row-error">{text.correction.sourceRequired}</span> : null}
      {!target.trim() ? <span className="structured-row-error">{text.correction.targetRequired}</span> : null}
      {duplicate ? <span className="structured-row-warning">{text.correction.duplicateRule}</span> : null}
    </div>
  );
}

function normalizedKey(value: string) {
  return value.trim().toLocaleLowerCase();
}

function duplicateKeys(values: string[]) {
  const counts = new Map<string, number>();
  for (const value of values) {
    const key = normalizedKey(value);
    if (key) counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return new Set([...counts.entries()].filter(([, count]) => count > 1).map(([key]) => key));
}
