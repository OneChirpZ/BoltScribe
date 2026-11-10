import { useState } from "react";
import Field from "../components/Field";
import HelpTip from "../components/HelpTip";
import type { TextBundle } from "../domain/i18n";
import { modelManagerText, modelService } from "../domain/modelServices";
import { optionalNumber } from "../domain/numbers";
import { thinkingEfforts } from "../domain/providers";
import type { AppConfig, LlmConfig } from "../types";

export default function ModelTaskSettings({ config, onChange, purpose, text, choices }: {
  choices: (id: string) => string[];
  config: AppConfig; onChange: (config: AppConfig) => void;
  purpose: "realtime" | "learning"; text: TextBundle;
}) {
  const copy = modelManagerText(config.ui.app_language);
  const learning = purpose === "learning";
  const profile = learning ? config.learning.llm : config.llm;
  const service = modelService(config, profile.service_id);
  const title = learning ? copy.learning : copy.realtime;
  const [customFor, setCustomFor] = useState<string | null>(null);
  const manual = customFor === profile.service_id;

  function update(patch: Partial<LlmConfig>) {
    const llm = { ...profile, ...patch };
    onChange(learning ? { ...config, learning: { ...config.learning, llm } } : { ...config, llm });
  }

  return (
    <section className="settings-section model-task-card" aria-label={title}>
      <div className="section-title"><h2>{title}</h2></div>
      <div className="form-grid model-primary-grid">
        <Field label={copy.service}>
          <select aria-label={`${title} · ${copy.service}`} value={service?.id ?? ""} onChange={(event) => {
            const service_id = event.target.value;
            const models = choices(service_id);
            setCustomFor(null);
            update({ service_id, model: models.includes(profile.model) ? profile.model : "" });
          }}>
            <option value="">{copy.choose}</option>
            {config.model_services.map((item) => <option key={item.id} value={item.id}>{item.name || item.id}</option>)}
          </select>
        </Field>
        <Field label={copy.model}>
          <select aria-label={`${title} · ${copy.model}`} disabled={!service}
            value={manual ? "__manual__" : profile.model} onChange={(event) => {
              if (event.target.value === "__manual__") setCustomFor(profile.service_id);
              else { setCustomFor(null); update({ model: event.target.value }); }
            }}>
            <option value="">{copy.chooseModel}</option>
            {choices(profile.service_id).map((model) => <option key={model} value={model}>{model}</option>)}
            <option value="__manual__">{copy.otherModel}</option>
          </select>
          {manual ? <input aria-label={`${title} · Model ID`} value={profile.model} spellCheck={false}
            onChange={(event) => update({ model: event.target.value })} /> : null}
        </Field>
      </div>
      <details className="model-task-parameters">
        <summary>{copy.advanced}</summary>
        <div className="form-grid">
          <div className="field-wide compact-number-grid">
            <Field label="Temperature"><input aria-label={`${title} · Temperature`} type="number" min="0" max="2" step="0.1" value={profile.temperature} onChange={(event) => update({ temperature: Number(event.target.value) })} /></Field>
            <Field label={copy.timeout}><input aria-label={`${title} · ${copy.timeout}`} type="number" min="1" max={learning ? 180 : undefined} value={profile.timeout_secs} onChange={(event) => update({ timeout_secs: Number(event.target.value) })} /></Field>
            <Field label={copy.maxTokens}><input aria-label={`${title} · ${copy.maxTokens}`} type="number" min="1" max={learning ? 16384 : undefined} value={profile.max_output_tokens ?? ""} onChange={(event) => update({ max_output_tokens: optionalNumber(event.target.value) })} /></Field>
          </div>
          <div className="field-wide compact-control-row">
            {!learning && (service?.provider === "openai" || service?.provider === "volc_ark") ? <>
              <label className="toggle-row inline-toggle"><input type="checkbox" checked={profile.fast_mode_enabled ?? true} onChange={(event) => update({ fast_mode_enabled: event.target.checked })} /><span>{text.models.fastMode}</span></label>
              <HelpTip content={text.models.fastModeHelp} />
            </> : null}
            <label className="toggle-row inline-toggle"><input type="checkbox" checked={profile.thinking_enabled} onChange={(event) => update({ thinking_enabled: event.target.checked })} aria-label={`${title} · ${copy.thinking}`} /><span>{copy.thinking}</span></label>
            <Field label={copy.effort} className="field-compact">
              <select aria-label={`${title} · ${copy.effort}`} value={profile.thinking_effort} disabled={!profile.thinking_enabled} onChange={(event) => update({ thinking_effort: event.target.value })}>
                {thinkingEfforts.map((effort) => <option key={effort} value={effort}>{effort}</option>)}
              </select>
            </Field>
          </div>
        </div>
      </details>
    </section>
  );
}
