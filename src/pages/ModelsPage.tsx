import { useEffect, useState } from "react";
import type { AppConfig, ModelService, RaceModelTarget } from "../types";
import Field from "../components/Field";
import HelpTip from "../components/HelpTip";
import PanelHeader from "../components/PanelHeader";
import type { TextBundle } from "../domain/i18n";
import { serviceModels } from "../domain/modelServices";
import ModelServicesSection from "./ModelServicesSection";
import { appApi } from "../app/api";
import { useModelCatalogs } from "./useModelCatalogs";
import ModelTaskSettings from "./ModelTaskSettings";

const emptyServices: ModelService[] = [];

export default function ModelsPage({ config, onChange, text, savedServices = emptyServices, fetchModels = appApi.config.listServiceModels }: {
  savedServices?: ModelService[];
  fetchModels?: (id: string) => Promise<string[]>;
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  text: TextBundle;
}) {
  const catalogs = useModelCatalogs(savedServices, fetchModels);
  function choices(id: string) {
    const service = config.model_services.find((item) => item.id === id);
    return serviceModels(config, id, service && catalogs.catalog(service)?.models);
  }
  const [showAsrAccessKey, setShowAsrAccessKey] = useState(false);
  const asrAuthMode = config.asr.auth_mode === "legacy" ? "legacy" : "api_key";
  const asrAccessKeyLabel = asrAuthMode === "legacy" ? text.models.accessKeyLegacy : text.models.accessKeyApiKey;
  useEffect(() => {
    const hide = () => setShowAsrAccessKey(false);
    window.addEventListener("blur", hide);
    document.addEventListener("visibilitychange", hide);
    return () => { window.removeEventListener("blur", hide); document.removeEventListener("visibilitychange", hide); };
  }, []);
  function updateAsrAuthMode(auth_mode: string) {
    setShowAsrAccessKey(false);
    onChange({ ...config, asr: { ...config.asr, auth_mode } });
  }
  function toggleRaceTarget(target: RaceModelTarget, checked: boolean) {
    const rest = config.llm.race_targets.filter((item) => !(item.service_id === target.service_id && item.model === target.model));
    onChange({ ...config, llm: { ...config.llm, race_targets: checked ? [...rest, target] : rest, race_models: [] } });
  }
  return (
    <section className="panel page-stack config-page models-page">
      <PanelHeader title={text.models.title} />
      <ModelServicesSection config={config} onChange={onChange} catalogs={catalogs} />
      <div className="model-task-grid">
        <ModelTaskSettings choices={choices} config={config} onChange={onChange} purpose="realtime" text={text} />
        <ModelTaskSettings choices={choices} config={config} onChange={onChange} purpose="learning" text={text} />
      </div>
      <section className="settings-section">
        <div className="section-title"><h2>{text.models.raceMode}<HelpTip content={text.models.raceHelp} /></h2></div>
        <div className="race-settings-stack">
          <label className="toggle-row inline-toggle"><input type="checkbox" checked={config.llm.race_enabled} onChange={(event) => {
            const race_targets = event.target.checked && !config.llm.race_targets.length && config.llm.service_id && config.llm.model.trim()
              ? [{ service_id: config.llm.service_id, model: config.llm.model }] : config.llm.race_targets;
            onChange({ ...config, llm: { ...config.llm, race_enabled: event.target.checked, race_targets, race_models: [] } });
          }} /><span>{text.models.enableRace}</span></label>
          {config.llm.race_enabled ? <Field label={text.models.raceModels} group><div className="race-model-list">
            {config.model_services.flatMap((service) => choices(service.id).map((model) => {
              const target = { service_id: service.id, model };
              return <label key={`${service.id}:${model}`} className="race-model-option"><input type="checkbox" checked={config.llm.race_targets.some((item) => item.service_id === service.id && item.model === model)} onChange={(event) => toggleRaceTarget(target, event.target.checked)} /><span>{service.name} / {model}</span></label>;
            }))}
          </div></Field> : <p className="settings-help-text">{text.models.raceHelp}</p>}
        </div>
      </section>
      <div className="settings-section">
        <div className="section-title">
          <h2>{text.models.asr}</h2>
        </div>
        <div className="form-grid">
          <SegmentedField
            label={text.models.asrAuthMode}
            value={asrAuthMode}
            options={[
              { value: "api_key", label: text.models.asrAuthModeNew },
              { value: "legacy", label: text.models.asrAuthModeLegacy },
            ]}
            onChange={updateAsrAuthMode}
          />
          <Field label={text.models.asrLanguage} className="field-compact">
            <input value={config.asr.language} onChange={(event) => onChange({ ...config, asr: { ...config.asr, language: event.target.value } })} />
          </Field>
          {asrAuthMode === "legacy" ? (
            <Field label={text.models.appKey} className="field-medium">
              <input value={config.asr.app_key} onChange={(event) => onChange({ ...config, asr: { ...config.asr, app_key: event.target.value } })} />
            </Field>
          ) : null}
          <Field label={asrAccessKeyLabel} className="field-medium" group>
            <div className="secret-field">
              <input
                type={showAsrAccessKey ? "text" : "password"}
                aria-label={asrAccessKeyLabel}
                value={config.asr.access_key}
                onChange={(event) => onChange({ ...config, asr: { ...config.asr, access_key: event.target.value } })}
              />
              <button
                className={`icon-button secret-toggle${showAsrAccessKey ? " visible" : ""}`}
                type="button"
                onClick={() => setShowAsrAccessKey((visible) => !visible)}
                aria-label={showAsrAccessKey ? text.models.hideAccessKey(asrAccessKeyLabel) : text.models.showAccessKey(asrAccessKeyLabel)}
              >
                <SecretEyeIcon visible={showAsrAccessKey} />
              </button>
            </div>
          </Field>
          <Field label="Resource ID" className="field-medium">
            <input value={config.asr.resource_id} onChange={(event) => onChange({ ...config, asr: { ...config.asr, resource_id: event.target.value } })} />
          </Field>
          <Field label="WebSocket URL" className="field-wide field-long">
            <input value={config.asr.stream_url} onChange={(event) => onChange({ ...config, asr: { ...config.asr, stream_url: event.target.value } })} />
          </Field>
        </div>
      </div>

    </section>
  );
}

function SegmentedField({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
}) {
  return (
    <div className="field">
      <span className="field-label">{label}</span>
      <div className="segmented-control" role="group" aria-label={label}>
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            className={option.value === value ? "active" : ""}
            aria-pressed={option.value === value}
            onClick={() => {
              if (option.value !== value) {
                onChange(option.value);
              }
            }}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}

function SecretEyeIcon({ visible }: { visible: boolean }) {
  return (
    <svg className="secret-eye" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M2.5 12s3.4-6 9.5-6 9.5 6 9.5 6-3.4 6-9.5 6-9.5-6-9.5-6Z" />
      <circle className="secret-eye-pupil" cx="12" cy="12" r="3.1" />
      {!visible ? <path className="secret-eye-slash" d="M4.5 19.5 19.5 4.5" /> : null}
    </svg>
  );
}
