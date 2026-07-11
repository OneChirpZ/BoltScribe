import { useEffect, useState } from "react";
import type { AppConfig, LlmProviderSettings, RaceModelTarget } from "../types";
import Field from "../components/Field";
import HelpTip from "../components/HelpTip";
import PanelHeader from "../components/PanelHeader";
import type { TextBundle } from "../domain/i18n";
import { optionalNumber } from "../domain/numbers";
import { builtInModelsForProvider, deleteModelPreset, modelsForProvider, providerLabel, providerPresets, saveModelPreset, thinkingEfforts } from "../domain/providers";

export default function ModelsPage({
  config,
  onChange,
  onSaveConfig,
  onNotice,
  canSave,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  onSaveConfig: (config: AppConfig, successMessage?: string) => void;
  onNotice: (message: string) => void;
  canSave: boolean;
  text: TextBundle;
}) {
  const providerKey = config.llm.provider in providerPresets ? config.llm.provider : "custom";
  const presetModels = modelsForProvider(providerKey, config.llm.model_presets);
  const [selectedPresetModel, setSelectedPresetModel] = useState(
    presetModels.some((model) => model === config.llm.model) ? config.llm.model : "__custom__",
  );
  const [showAsrAccessKey, setShowAsrAccessKey] = useState(false);
  const [showLlmApiKey, setShowLlmApiKey] = useState(false);
  const builtInModels = builtInModelsForProvider(providerKey);
  const canDeletePreset = selectedPresetModel !== "__custom__" && !builtInModels.includes(selectedPresetModel);
  const raceModels = config.llm.race_models ?? [];
  const raceTargets = selectedRaceTargets(config);
  const raceTargetKeys = new Set(raceTargets.map(raceTargetKey));
  const raceTargetOptions = allRaceTargetOptions(config);
  const asrAuthMode = config.asr.auth_mode === "legacy" ? "legacy" : "api_key";
  const asrAccessKeyLabel = asrAuthMode === "legacy" ? text.models.accessKeyLegacy : text.models.accessKeyApiKey;

  useEffect(() => {
    setSelectedPresetModel(presetModels.some((model) => model === config.llm.model) ? config.llm.model : "__custom__");
  }, [providerKey]);

  useEffect(() => {
    const hideSecrets = () => {
      setShowAsrAccessKey(false);
      setShowLlmApiKey(false);
    };
    const hideSecretsWhenDocumentIsHidden = () => {
      if (document.hidden) {
        hideSecrets();
      }
    };
    window.addEventListener("blur", hideSecrets);
    document.addEventListener("visibilitychange", hideSecretsWhenDocumentIsHidden);
    return () => {
      window.removeEventListener("blur", hideSecrets);
      document.removeEventListener("visibilitychange", hideSecretsWhenDocumentIsHidden);
    };
  }, []);

  function updateProvider(provider: string) {
    setShowLlmApiKey(false);
    const preset = providerPresets[provider as keyof typeof providerPresets];
    const provider_settings = upsertProviderSettings(config.llm.provider_settings ?? [], {
      provider: providerKey,
      endpoint: config.llm.endpoint,
      api_format: config.llm.api_format,
      api_key: config.llm.api_key,
    });
    const savedProviderSettings = provider_settings.find((setting) => setting.provider === provider);
    const nextPresetModels = modelsForProvider(provider, config.llm.model_presets);
    const firstPresetModel = nextPresetModels[0] ?? config.llm.model;
    setSelectedPresetModel(firstPresetModel || "__custom__");
    const nextLlm = {
      ...config.llm,
      provider,
      endpoint: savedProviderSettings?.endpoint ?? (provider === "custom" ? config.llm.endpoint : preset.endpoint),
      api_format: savedProviderSettings?.api_format ?? (provider === "custom" ? config.llm.api_format : preset.api_format),
      api_key: savedProviderSettings?.api_key ?? (provider === "custom" ? config.llm.api_key : ""),
      model: provider === "custom" ? config.llm.model : firstPresetModel,
      provider_settings,
    };
    onChange({ ...config, llm: nextLlm });
  }

  function updateModel(model: string) {
    onChange({ ...config, llm: { ...config.llm, model } });
  }

  function updateProviderField(field: "endpoint" | "api_format" | "api_key", value: string) {
    const provider_settings = upsertProviderSettings(config.llm.provider_settings ?? [], {
      provider: providerKey,
      endpoint: field === "endpoint" ? value : config.llm.endpoint,
      api_format: field === "api_format" ? value : config.llm.api_format,
      api_key: field === "api_key" ? value : config.llm.api_key,
    });
    onChange({ ...config, llm: { ...config.llm, [field]: value, provider_settings } });
  }

  function updateRaceEnabled(enabled: boolean) {
    const currentTarget = { provider: providerKey, model: config.llm.model };
    const nextRaceTargets = enabled && raceTargets.length === 0 && config.llm.model.trim()
      ? [currentTarget]
      : raceTargets;
    onChange({ ...config, llm: { ...config.llm, race_enabled: enabled, race_targets: nextRaceTargets, race_models: legacyRaceModels(providerKey, nextRaceTargets) } });
  }

  function updateAsrAuthMode(auth_mode: string) {
    setShowAsrAccessKey(false);
    onChange({ ...config, asr: { ...config.asr, auth_mode } });
  }

  function toggleRaceTarget(target: RaceModelTarget, checked: boolean) {
    const key = raceTargetKey(target);
    const nextRaceTargets = checked
      ? [...raceTargets.filter((item) => raceTargetKey(item) !== key), target]
      : raceTargets.filter((item) => raceTargetKey(item) !== key);
    onChange({ ...config, llm: { ...config.llm, race_targets: nextRaceTargets, race_models: legacyRaceModels(providerKey, nextRaceTargets) } });
  }

  function saveCurrentModelPreset() {
    const model = config.llm.model.trim();
    if (!model) {
      onNotice(text.models.emptyModelNotice);
      return;
    }

    const previousModel = selectedPresetModel === "__custom__" ? "" : selectedPresetModel;
    const model_presets = saveModelPreset(config.llm.model_presets ?? [], providerKey, previousModel, model);
    const nextRaceTargets = previousModel
      ? raceTargets.map((item) => (item.provider === providerKey && item.model === previousModel ? { provider: providerKey, model } : item))
      : raceTargets;
    setSelectedPresetModel(model);
    const nextConfig = { ...config, llm: { ...config.llm, model, model_presets, race_targets: uniqueRaceTargets(nextRaceTargets), race_models: legacyRaceModels(providerKey, nextRaceTargets) } };
    onChange(nextConfig);
    onSaveConfig(nextConfig, previousModel ? text.models.presetUpdated : text.models.presetSaved);
  }

  function deleteCurrentModelPreset() {
    if (!canDeletePreset) {
      return;
    }
    if (!window.confirm(text.models.deleteConfirm(selectedPresetModel))) {
      return;
    }

    const model_presets = deleteModelPreset(config.llm.model_presets ?? [], providerKey, selectedPresetModel);
    setSelectedPresetModel("__custom__");
    const nextConfig = {
      ...config,
      llm: {
        ...config.llm,
        model_presets,
        race_targets: raceTargets.filter((target) => !(target.provider === providerKey && target.model === selectedPresetModel)),
        race_models: raceModels.filter((model) => model !== selectedPresetModel),
      },
    };
    onChange(nextConfig);
    onSaveConfig(nextConfig, text.models.presetDeleted);
  }

  return (
    <section className="panel page-stack config-page models-page">
      <PanelHeader title={text.models.title} />

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

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.models.providerSettings}</h2>
        </div>
        <div className="form-grid">
          <SegmentedField
            label={text.models.provider}
            value={providerKey}
            options={Object.entries(providerPresets).map(([value, preset]) => ({ value, label: preset.label }))}
            onChange={updateProvider}
          />
          <SegmentedField
            label={text.models.apiFormat}
            value={config.llm.api_format}
            options={[
              { value: "responses", label: "Responses" },
              { value: "chat_completions", label: "Chat Completions" },
            ]}
            onChange={(value) => updateProviderField("api_format", value)}
          />
          <Field label="Endpoint" className="field-wide field-long">
            <input value={config.llm.endpoint} onChange={(event) => updateProviderField("endpoint", event.target.value)} />
          </Field>
          <Field label={text.models.apiKey} className="field-wide field-long" group>
            <div className="secret-field">
              <input
                type={showLlmApiKey ? "text" : "password"}
                aria-label={text.models.apiKey}
                value={config.llm.api_key}
                onChange={(event) => updateProviderField("api_key", event.target.value)}
              />
              <button
                className={`icon-button secret-toggle${showLlmApiKey ? " visible" : ""}`}
                type="button"
                onClick={() => setShowLlmApiKey((visible) => !visible)}
                aria-label={showLlmApiKey ? text.models.hideApiKey : text.models.showApiKey}
              >
                <SecretEyeIcon visible={showLlmApiKey} />
              </button>
            </div>
          </Field>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.models.modelPresetSettings}</h2>
          <div className="section-actions">
            <button className="secondary small" type="button" disabled={!canSave || !canDeletePreset} onClick={deleteCurrentModelPreset}>{text.models.deletePreset}</button>
            <button className="secondary small" type="button" disabled={!canSave} onClick={saveCurrentModelPreset}>{text.models.savePreset}</button>
          </div>
        </div>
        <div className="form-grid model-primary-grid">
          <Field label={text.models.modelPreset}>
            <select
              value={selectedPresetModel}
              onChange={(event) => {
                const model = event.target.value;
                setSelectedPresetModel(model);
                if (model !== "__custom__") {
                  updateModel(model);
                }
              }}
            >
              <option value="__custom__">{text.common.custom}</option>
              {presetModels.map((model) => (
                <option key={model} value={model}>{model}</option>
              ))}
            </select>
          </Field>
          <Field label={text.models.model}>
            <input value={config.llm.model} onChange={(event) => updateModel(event.target.value)} />
          </Field>
          <div className="field-wide compact-number-grid">
            <Field label="Temperature">
              <input type="number" min="0" max="2" step="0.1" value={config.llm.temperature} onChange={(event) => onChange({ ...config, llm: { ...config.llm, temperature: Number(event.target.value) } })} />
            </Field>
            <Field label={text.models.timeoutSecs}>
              <input type="number" min="1" value={config.llm.timeout_secs} onChange={(event) => onChange({ ...config, llm: { ...config.llm, timeout_secs: Number(event.target.value) } })} />
            </Field>
            <Field label="Max Output Tokens">
              <input
                type="number"
                min="1"
                value={config.llm.max_output_tokens ?? ""}
                onChange={(event) => onChange({ ...config, llm: { ...config.llm, max_output_tokens: optionalNumber(event.target.value) } })}
              />
            </Field>
          </div>
          <div className="field-wide compact-control-row">
            <label className="toggle-row inline-toggle">
              <input
                type="checkbox"
                checked={config.llm.thinking_enabled}
                onChange={(event) => onChange({ ...config, llm: { ...config.llm, thinking_enabled: event.target.checked } })}
              />
              <span>Thinking</span>
            </label>
            <Field label={text.models.thinkingEffort} className="field-compact">
              <select
                value={config.llm.thinking_effort}
                disabled={!config.llm.thinking_enabled}
                onChange={(event) => onChange({ ...config, llm: { ...config.llm, thinking_effort: event.target.value } })}
              >
                {thinkingEfforts.map((effort) => (
                  <option key={effort} value={effort}>{effort}</option>
                ))}
              </select>
            </Field>
          </div>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>
            {text.models.raceMode}
            <HelpTip content={text.models.raceHelp} />
          </h2>
        </div>
        <div className="race-settings-stack">
          <label className="toggle-row inline-toggle">
            <input
              type="checkbox"
              checked={config.llm.race_enabled ?? false}
              onChange={(event) => updateRaceEnabled(event.target.checked)}
            />
            <span>{text.models.enableRace}</span>
          </label>
          {config.llm.race_enabled ? (
            <Field label={text.models.raceModels} group>
              <div className="race-model-list">
                {raceTargetOptions.map((target) => (
                  <label key={raceTargetKey(target)} className="race-model-option">
                    <input
                      type="checkbox"
                      checked={raceTargetKeys.has(raceTargetKey(target))}
                      onChange={(event) => toggleRaceTarget(target, event.target.checked)}
                    />
                    <span>{providerLabel(target.provider)} / {target.model}</span>
                  </label>
                ))}
              </div>
            </Field>
          ) : <p className="settings-help-text">{text.models.raceHelp}</p>}
        </div>
      </div>
    </section>
  );
}

function upsertProviderSettings(settings: LlmProviderSettings[], nextSetting: LlmProviderSettings) {
  const provider = nextSetting.provider.trim();
  if (!provider) {
    return settings;
  }
  const next = settings.filter((setting) => setting.provider !== provider);
  return [...next, {
    provider,
    endpoint: nextSetting.endpoint.trim(),
    api_format: nextSetting.api_format.trim(),
    api_key: nextSetting.api_key.trim(),
  }];
}

function selectedRaceTargets(config: AppConfig): RaceModelTarget[] {
  const targets = config.llm.race_targets ?? [];
  if (targets.length > 0) {
    return uniqueRaceTargets(targets);
  }
  return uniqueRaceTargets((config.llm.race_models ?? []).map((model) => ({
    provider: config.llm.provider,
    model,
  })));
}

function allRaceTargetOptions(config: AppConfig): RaceModelTarget[] {
  const configuredTargets = Object.keys(providerPresets).flatMap((provider) => (
    modelsForProvider(provider, config.llm.model_presets).map((model) => ({ provider, model }))
  ));
  const currentTarget = config.llm.model.trim()
    ? [{ provider: config.llm.provider, model: config.llm.model }]
    : [];
  return uniqueRaceTargets([...configuredTargets, ...selectedRaceTargets(config), ...currentTarget]);
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

function uniqueRaceTargets(targets: RaceModelTarget[]) {
  const seen = new Set<string>();
  const unique: RaceModelTarget[] = [];
  for (const target of targets) {
    const provider = target.provider.trim();
    const model = target.model.trim();
    const key = raceTargetKey({ provider, model });
    if (!provider || !model || seen.has(key)) {
      continue;
    }
    seen.add(key);
    unique.push({ provider, model });
  }
  return unique;
}

function legacyRaceModels(provider: string, targets: RaceModelTarget[]) {
  return uniqueRaceTargets(targets)
    .filter((target) => target.provider === provider)
    .map((target) => target.model);
}

function raceTargetKey(target: RaceModelTarget) {
  return `${target.provider}::${target.model}`;
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
