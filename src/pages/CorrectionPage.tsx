import { useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { AppConfig, PromptVariable } from "../types";
import CorrectionLineEditor from "../components/CorrectionLineEditor";
import TokenButton from "../components/TokenButton";
import type { TextBundle } from "../domain/i18n";
import type { CorrectionSection } from "../domain/navigation";
import { buildPromptPreview, isBuiltinVariable, normalizeVariableName, variableToken } from "../domain/promptTemplate";
import { enabledDictionaryText, reconcileDisabledDictionaryTerms } from "../domain/correctionText";

const promptEditors = ["system", "template", "variables"] as const;
type PromptEditor = (typeof promptEditors)[number];

export default function CorrectionPage({
  config,
  onChange,
  section,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  section: CorrectionSection;
  text: TextBundle;
}) {
  const dictionaryText = config.correction.dictionary_text ?? "";
  const disabledDictionaryTerms = config.correction.disabled_dictionary_terms ?? [];
  const correctionRulesText = config.correction.correction_rules_text ?? "";
  const promptDictionaryText = useMemo(
    () => enabledDictionaryText(dictionaryText, disabledDictionaryTerms),
    [dictionaryText, disabledDictionaryTerms],
  );
  const promptPreview = useMemo(
    () => buildPromptPreview(
      config.correction.user_requirements,
      promptDictionaryText,
      correctionRulesText,
      config.correction.variables,
      config.correction.prompt_template,
    ),
    [config.correction.user_requirements, promptDictionaryText, correctionRulesText, config.correction.variables, config.correction.prompt_template],
  );
  const templateRef = useRef<HTMLTextAreaElement>(null);
  const promptTabsId = useId();
  const [promptEditor, setPromptEditor] = useState<PromptEditor>("template");

  const variables = config.correction.variables ?? [];
  const sectionDetails = {
    requirements: { title: text.correction.requirementsNav, description: text.correction.requirementsDescription },
    dictionary: { title: text.correction.dictionaryNav, description: text.correction.dictionaryDescription },
    rules: { title: text.correction.rulesNav, description: text.correction.rulesDescription },
    prompt: { title: text.correction.promptNav, description: text.correction.promptDescription },
  }[section];

  function updateDictionaryLines(value: string) {
    onChange({
      ...config,
      correction: {
        ...config.correction,
        dictionary_text: value,
        disabled_dictionary_terms: reconcileDisabledDictionaryTerms(value, disabledDictionaryTerms),
      },
    });
  }

  function updateDisabledDictionaryTerms(disabled_dictionary_terms: string[]) {
    onChange({
      ...config,
      correction: {
        ...config.correction,
        disabled_dictionary_terms,
      },
    });
  }

  function updateCorrectionRuleLines(value: string) {
    onChange({
      ...config,
      correction: {
        ...config.correction,
        correction_rules_text: value,
      },
    });
  }

  function updateVariable(index: number, next: PromptVariable) {
    const nextVariables = variables.map((variable, i) => (i === index ? next : variable));
    onChange({ ...config, correction: { ...config.correction, variables: nextVariables } });
  }

  function removeVariable(index: number) {
    const nextVariables = variables.filter((_, i) => i !== index);
    onChange({ ...config, correction: { ...config.correction, variables: nextVariables } });
  }

  function addVariable() {
    onChange({
      ...config,
      correction: {
        ...config.correction,
        variables: [...variables, { name: "", value: "" }],
      },
    });
  }

  function updatePromptTemplate(prompt_template: string) {
    onChange({
      ...config,
      correction: {
        ...config.correction,
        prompt_template,
      },
    });
  }

  function insertTemplateToken(token: string) {
    const template = config.correction.prompt_template;
    const textarea = templateRef.current;
    const start = textarea?.selectionStart ?? template.length;
    const end = textarea?.selectionEnd ?? template.length;
    const nextTemplate = `${template.slice(0, start)}${token}${template.slice(end)}`;
    updatePromptTemplate(nextTemplate);
    window.requestAnimationFrame(() => {
      templateRef.current?.focus();
      const cursor = start + token.length;
      templateRef.current?.setSelectionRange(cursor, cursor);
    });
  }

  function handlePromptTabKeyDown(event: KeyboardEvent<HTMLButtonElement>, current: PromptEditor) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }
    event.preventDefault();
    const currentIndex = promptEditors.indexOf(current);
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? promptEditors.length - 1
        : event.key === "ArrowRight"
          ? (currentIndex + 1) % promptEditors.length
          : (currentIndex - 1 + promptEditors.length) % promptEditors.length;
    const next = promptEditors[nextIndex];
    setPromptEditor(next);
    const tabList = event.currentTarget.parentElement;
    window.requestAnimationFrame(() => {
      const tabs = tabList?.querySelectorAll<HTMLButtonElement>("button[role='tab']");
      tabs?.[nextIndex]?.focus();
    });
  }

  return (
    <section className="panel correction-page">
      <header className="correction-subpage-header">
        <div>
          <div className="eyebrow">{text.correction.title}</div>
          <h1>{sectionDetails.title}</h1>
        </div>
        <p>{sectionDetails.description}</p>
      </header>

      <div className="correction-subpage-body">
        {section === "requirements" ? (
          <label className="correction-fill-field correction-requirements-editor">
            <span className="field-label">{text.correction.requirementText}</span>
            <textarea
              className="correction-fill-editor"
              value={config.correction.user_requirements}
              onChange={(event) => onChange({ ...config, correction: { ...config.correction, user_requirements: event.target.value } })}
            />
          </label>
        ) : null}

        {section === "dictionary" ? (
          <CorrectionLineEditor
            kind="dictionary"
            value={dictionaryText}
            disabledDictionaryTerms={disabledDictionaryTerms}
            onChange={updateDictionaryLines}
            onDisabledDictionaryTermsChange={updateDisabledDictionaryTerms}
            text={text}
          />
        ) : null}

        {section === "rules" ? (
          <CorrectionLineEditor kind="rules" value={correctionRulesText} onChange={updateCorrectionRuleLines} text={text} />
        ) : null}

        {section === "prompt" ? (
          <div className="correction-prompt-workspace">
            <section className="correction-prompt-editor">
              <div className="prompt-editor-toolbar">
                <div className="segmented-control prompt-tabs" role="tablist" aria-label={text.correction.promptEditorLabel}>
                  {promptEditors.map((tab) => (
                    <button
                      key={tab}
                      className={promptEditor === tab ? "active" : ""}
                      type="button"
                      role="tab"
                      id={`${promptTabsId}-${tab}-tab`}
                      aria-controls={`${promptTabsId}-${tab}-panel`}
                      aria-selected={promptEditor === tab}
                      tabIndex={promptEditor === tab ? 0 : -1}
                      onClick={() => setPromptEditor(tab)}
                      onKeyDown={(event) => handlePromptTabKeyDown(event, tab)}
                    >
                      {tab === "system"
                        ? text.correction.systemPrompt
                        : tab === "template"
                          ? text.correction.template
                          : text.correction.variablesNav}
                    </button>
                  ))}
                </div>
                {promptEditor === "template" ? (
                  <div className="template-tokens">
                    {["{{user_requirements}}", "{{dictionary}}", "{{correction_rules}}", "{{raw_text}}"].map((token) => (
                      <TokenButton key={token} token={token} onClick={insertTemplateToken} label={text.correction.insertToken(token)} />
                    ))}
                    {variables.filter((variable) => variable.name.trim()).map((variable, index) => {
                      const token = variableToken(variable.name);
                      return <TokenButton key={`${variable.name}-${index}`} token={token} onClick={insertTemplateToken} label={text.correction.insertToken(token)} />;
                    })}
                  </div>
                ) : null}
              </div>
              <div
                className="correction-prompt-panel"
                id={`${promptTabsId}-${promptEditor}-panel`}
                role="tabpanel"
                aria-labelledby={`${promptTabsId}-${promptEditor}-tab`}
              >
                {promptEditor === "variables" ? (
                  <PromptVariablesEditor
                    variables={variables}
                    onAdd={addVariable}
                    onUpdate={updateVariable}
                    onRemove={removeVariable}
                    text={text}
                  />
                ) : (
                  <label className="correction-fill-field">
                    <span className="visually-hidden">
                      {promptEditor === "system" ? text.correction.systemPrompt : text.correction.template}
                    </span>
                    {promptEditor === "system" ? (
                      <textarea
                        className="correction-fill-editor prompt-workspace-textarea"
                        value={config.llm.system_prompt}
                        onChange={(event) => onChange({ ...config, llm: { ...config.llm, system_prompt: event.target.value } })}
                      />
                    ) : (
                      <textarea
                        ref={templateRef}
                        className="correction-fill-editor prompt-workspace-textarea"
                        value={config.correction.prompt_template}
                        spellCheck={false}
                        onChange={(event) => updatePromptTemplate(event.target.value)}
                      />
                    )}
                  </label>
                )}
              </div>
            </section>
            <section className="correction-message-preview" aria-label={text.correction.preview}>
              <div className="preview-title">{text.correction.preview}</div>
              <div className="preview-message preview-system-message">
                <h2>{text.correction.systemMessage}</h2>
                <pre>{config.llm.system_prompt}</pre>
              </div>
              <div className="preview-message preview-user-message">
                <h2>{text.correction.userMessage}</h2>
                <pre>{promptPreview}</pre>
              </div>
              <p className="preview-sample-note">{text.correction.sampleTranscriptNote}</p>
            </section>
          </div>
        ) : null}

      </div>
    </section>
  );
}

function PromptVariablesEditor({
  variables,
  onAdd,
  onUpdate,
  onRemove,
  text,
}: {
  variables: PromptVariable[];
  onAdd: () => void;
  onUpdate: (index: number, variable: PromptVariable) => void;
  onRemove: (index: number) => void;
  text: TextBundle;
}) {
  const normalizedVariableNames = variables.map((variable) => normalizeVariableName(variable.name));

  return (
    <div className="correction-variables-workspace">
      <div className="correction-variables-toolbar">
        <span>{text.correction.itemCount(variables.length)}</span>
        <button className="primary small" type="button" onClick={onAdd}>{text.correction.addVariable}</button>
      </div>
      <div className="variable-list correction-variable-list">
        {variables.length === 0 ? <div className="structured-empty">{text.correction.emptyVariables}</div> : null}
        {variables.map((variable, index) => {
          const normalizedName = normalizedVariableNames[index];
          const duplicate = Boolean(normalizedName) && normalizedVariableNames.filter((name) => name === normalizedName).length > 1;
          const builtinConflict = isBuiltinVariable(normalizedName);
          return (
            <div className="variable-entry correction-variable-entry" key={index}>
              <label>
                <span className="field-label">{text.correction.variableName}</span>
                <input
                  placeholder={text.correction.variableNamePlaceholder}
                  value={variable.name}
                  onChange={(event) => onUpdate(index, { ...variable, name: normalizeVariableName(event.target.value) })}
                />
              </label>
              <label>
                <span className="field-label">{text.correction.variableValue}</span>
                <textarea
                  placeholder={text.correction.variableValuePlaceholder}
                  value={variable.value}
                  onChange={(event) => onUpdate(index, { ...variable, value: event.target.value })}
                />
              </label>
              <button className="icon-button" type="button" onClick={() => onRemove(index)} aria-label={text.correction.deleteVariable}>×</button>
              {!normalizedName ? <span className="structured-row-error">{text.correction.variableNameRequired}</span> : null}
              {builtinConflict ? <span className="structured-row-error">{text.correction.variableBuiltinConflict}</span> : null}
              {duplicate ? <span className="structured-row-warning">{text.correction.variableDuplicate}</span> : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
