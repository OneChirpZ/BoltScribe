import { useEffect, useMemo, useRef, useState } from "react";
import type { AppConfig, PromptVariable } from "../types";
import Field from "../components/Field";
import PanelHeader from "../components/PanelHeader";
import TokenButton from "../components/TokenButton";
import type { TextBundle } from "../domain/i18n";
import { buildPromptPreview, correctionRulesFromLines, correctionRulesToLines, dictionaryFromLines, dictionaryToLines, normalizeVariableName, variableToken } from "../domain/promptTemplate";

export default function CorrectionPage({
  config,
  onChange,
  onSave,
  canSave,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  onSave: () => void;
  canSave: boolean;
  text: TextBundle;
}) {
  const promptPreview = useMemo(
    () => buildPromptPreview(
      config.correction.user_requirements,
      config.correction.dictionary,
      config.correction.correction_rules ?? [],
      config.correction.variables,
      config.correction.prompt_template,
    ),
    [config.correction.user_requirements, config.correction.dictionary, config.correction.correction_rules, config.correction.variables, config.correction.prompt_template],
  );
  const templateRef = useRef<HTMLTextAreaElement>(null);
  const [dictionaryLines, setDictionaryLines] = useState(() => dictionaryToLines(config.correction.dictionary));
  const [correctionRuleLines, setCorrectionRuleLines] = useState(() => correctionRulesToLines(config.correction.correction_rules ?? []));

  const variables = config.correction.variables ?? [];

  useEffect(() => {
    if (!dictionaryLinesRepresentEntries(dictionaryLines, config.correction.dictionary)) {
      setDictionaryLines(dictionaryToLines(config.correction.dictionary));
    }
  }, [config.correction.dictionary]);

  useEffect(() => {
    if (!correctionRuleLinesRepresentRules(correctionRuleLines, config.correction.correction_rules ?? [])) {
      setCorrectionRuleLines(correctionRulesToLines(config.correction.correction_rules ?? []));
    }
  }, [config.correction.correction_rules]);

  function updateDictionaryLines(value: string) {
    setDictionaryLines(value);
    onChange({
      ...config,
      correction: {
        ...config.correction,
        dictionary: dictionaryFromLines(value),
      },
    });
  }

  function updateCorrectionRuleLines(value: string) {
    setCorrectionRuleLines(value);
    onChange({
      ...config,
      correction: {
        ...config.correction,
        correction_rules: correctionRulesFromLines(value),
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
    if (textarea) {
      textarea.focus();
      textarea.setSelectionRange(start, end);
      if (document.execCommand("insertText", false, token)) {
        updatePromptTemplate(textarea.value);
        textarea.dispatchEvent(new Event("input", { bubbles: true }));
        return;
      }
    }

    const nextTemplate = `${template.slice(0, start)}${token}${template.slice(end)}`;
    updatePromptTemplate(nextTemplate);
    window.requestAnimationFrame(() => {
      templateRef.current?.focus();
      const cursor = start + token.length;
      templateRef.current?.setSelectionRange(cursor, cursor);
    });
  }

  return (
    <section className="panel page-stack">
      <PanelHeader title={text.correction.title} action={<button className="primary small" disabled={!canSave} onClick={onSave}>{text.common.save}</button>} />
      <label className="toggle-row">
        <input
          type="checkbox"
          checked={config.correction.enabled}
          onChange={(event) => onChange({ ...config, correction: { ...config.correction, enabled: event.target.checked } })}
        />
        {text.correction.enabled}
      </label>
      <div className="settings-section">
        <div className="section-title">
          <h2>{text.correction.templateSection}</h2>
          <div className="template-tokens">
            <TokenButton token="{{user_requirements}}" onClick={insertTemplateToken} />
            <TokenButton token="{{dictionary}}" onClick={insertTemplateToken} />
            <TokenButton token="{{correction_rules}}" onClick={insertTemplateToken} />
            <TokenButton token="{{raw_text}}" onClick={insertTemplateToken} />
            {variables.filter((variable) => variable.name.trim()).map((variable, index) => (
              <TokenButton key={`${variable.name}-${index}`} token={variableToken(variable.name)} onClick={insertTemplateToken} />
            ))}
          </div>
        </div>
        <Field label={text.correction.systemPrompt}>
          <textarea
            className="system-prompt"
            value={config.llm.system_prompt}
            onChange={(event) => onChange({ ...config, llm: { ...config.llm, system_prompt: event.target.value } })}
          />
        </Field>
        <div className="template-grid">
          <Field label={text.correction.template}>
            <textarea
              ref={templateRef}
              className="prompt-template"
              value={config.correction.prompt_template}
              onChange={(event) => updatePromptTemplate(event.target.value)}
            />
          </Field>
          <div className="prompt-preview">
            <div className="preview-title">{text.correction.preview}</div>
            <pre>{promptPreview}</pre>
          </div>
        </div>
      </div>
      <div className="settings-section">
        <div className="section-title">
          <h2>{text.correction.userRequirements}</h2>
        </div>
        <Field label={text.correction.requirementText}>
          <textarea
            className="requirements"
            value={config.correction.user_requirements}
            onChange={(event) => onChange({ ...config, correction: { ...config.correction, user_requirements: event.target.value } })}
          />
        </Field>
      </div>
      <div className="settings-section">
        <div className="section-title">
          <h2>{text.correction.dictionary}</h2>
        </div>
        <Field label={text.correction.dictionaryHelp}>
          <textarea
            className="dictionary-lines"
            value={dictionaryLines}
            onChange={(event) => updateDictionaryLines(event.target.value)}
          />
        </Field>
      </div>
      <div className="settings-section">
        <div className="section-title">
          <h2>{text.correction.correctionRules}</h2>
        </div>
        <Field label={text.correction.correctionRulesHelp}>
          <textarea
            className="dictionary-lines"
            value={correctionRuleLines}
            onChange={(event) => updateCorrectionRuleLines(event.target.value)}
          />
        </Field>
      </div>
      <div className="settings-section">
        <div className="section-title">
          <h2>{text.correction.variables}</h2>
          <button className="secondary small" onClick={addVariable}>{text.correction.addVariable}</button>
        </div>
        <div className="variable-list">
          {variables.length === 0 ? <p className="empty">{text.correction.emptyVariables}</p> : null}
          {variables.map((variable, index) => (
            <div className="variable-entry" key={index}>
              <input
                placeholder={text.correction.variableNamePlaceholder}
                value={variable.name}
                onChange={(event) => updateVariable(index, { ...variable, name: normalizeVariableName(event.target.value) })}
              />
              <textarea
                placeholder={text.correction.variableValuePlaceholder}
                value={variable.value}
                onChange={(event) => updateVariable(index, { ...variable, value: event.target.value })}
              />
              <button className="icon-button" onClick={() => removeVariable(index)} aria-label={text.correction.deleteVariable}>×</button>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function dictionaryLinesRepresentEntries(value: string, dictionary: AppConfig["correction"]["dictionary"]) {
  return dictionaryToLines(dictionaryFromLines(value)) === dictionaryToLines(dictionary);
}

function correctionRuleLinesRepresentRules(value: string, rules: AppConfig["correction"]["correction_rules"]) {
  return correctionRulesToLines(correctionRulesFromLines(value)) === correctionRulesToLines(rules);
}
