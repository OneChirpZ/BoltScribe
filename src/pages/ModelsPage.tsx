import { useEffect, useState } from "react";
import type { AppConfig, LlmProviderSettings, RaceModelTarget } from "../types";
import Field from "../components/Field";
import PanelHeader from "../components/PanelHeader";
import ShortcutPicker from "../components/ShortcutPicker";
import type { TextBundle } from "../domain/i18n";
import { defaultRecordingOverlayScale, maxRecordingOverlayScale, minRecordingOverlayScale } from "../domain/overlay";
import { hotkeyEnabledSlots, hotkeySlots, updateHotkey, updateHotkeyEnabled } from "../domain/hotkeys";
import { optionalNumber } from "../domain/numbers";
import { builtInModelsForProvider, deleteModelPreset, modelsForProvider, providerLabel, providerPresets, saveModelPreset, thinkingEfforts } from "../domain/providers";

export default function ModelsPage({
  config,
  onChange,
  onSave,
  onSaveConfig,
  onNotice,
  canSave,
  text,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  onSave: () => void;
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
  const shortcutSlots = hotkeySlots(config);
  const shortcutEnabledSlots = hotkeyEnabledSlots(config);
  const builtInModels = builtInModelsForProvider(providerKey);
  const canDeletePreset = selectedPresetModel !== "__custom__" && !builtInModels.includes(selectedPresetModel);
  const raceModels = config.llm.race_models ?? [];
  const raceTargets = selectedRaceTargets(config);
  const raceTargetKeys = new Set(raceTargets.map(raceTargetKey));
  const raceTargetOptions = allRaceTargetOptions(config);

  useEffect(() => {
    setSelectedPresetModel(presetModels.some((model) => model === config.llm.model) ? config.llm.model : "__custom__");
  }, [providerKey]);

  function updateProvider(provider: string) {
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
    <section className="panel page-stack">
      <PanelHeader title={text.models.title} action={<button className="primary small" disabled={!canSave} onClick={onSave}>{text.common.save}</button>} />

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.models.shortcuts}</h2>
        </div>
        <div className="shortcut-grid">
          <ShortcutPicker
            label={text.models.shortcut1}
            enabled={shortcutEnabledSlots[0]}
            value={shortcutSlots[0]}
            onEnabledChange={(enabled) => onChange(updateHotkeyEnabled(config, 0, enabled))}
            onChange={(value) => onChange(updateHotkey(config, 0, value))}
            text={text}
          />
          <ShortcutPicker
            label={text.models.shortcut2}
            enabled={shortcutEnabledSlots[1]}
            value={shortcutSlots[1]}
            onEnabledChange={(enabled) => onChange(updateHotkeyEnabled(config, 1, enabled))}
            onChange={(value) => onChange(updateHotkey(config, 1, value))}
            text={text}
          />
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.models.asr}</h2>
        </div>
        <div className="form-grid">
          <Field label={text.models.asrLanguage}>
            <input value={config.asr.language} onChange={(event) => onChange({ ...config, asr: { ...config.asr, language: event.target.value } })} />
          </Field>
          <Field label={text.models.appKey}>
            <input value={config.asr.app_key} onChange={(event) => onChange({ ...config, asr: { ...config.asr, app_key: event.target.value } })} />
          </Field>
          <Field label={text.models.accessKey}>
            <div className="secret-field">
              <input
                type={showAsrAccessKey ? "text" : "password"}
                value={config.asr.access_key}
                onChange={(event) => onChange({ ...config, asr: { ...config.asr, access_key: event.target.value } })}
              />
              <button
                className={`icon-button secret-toggle${showAsrAccessKey ? " visible" : ""}`}
                type="button"
                onClick={() => setShowAsrAccessKey((visible) => !visible)}
                aria-label={showAsrAccessKey ? text.models.hideAccessKey : text.models.showAccessKey}
              >
                <SecretEyeIcon visible={showAsrAccessKey} />
              </button>
            </div>
          </Field>
          <Field label="Resource ID">
            <input value={config.asr.resource_id} onChange={(event) => onChange({ ...config, asr: { ...config.asr, resource_id: event.target.value } })} />
          </Field>
          <Field label="WebSocket URL">
            <input value={config.asr.stream_url} onChange={(event) => onChange({ ...config, asr: { ...config.asr, stream_url: event.target.value } })} />
          </Field>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.models.interface}</h2>
        </div>
        <div className="form-grid">
          <Field label={text.models.overlayScale(Math.round((config.ui.recording_overlay_scale ?? defaultRecordingOverlayScale) * 200))} className="field-wide">
            <input
              className="range-input"
              type="range"
              min={String(minRecordingOverlayScale)}
              max={String(maxRecordingOverlayScale)}
              step="0.05"
              value={config.ui.recording_overlay_scale ?? defaultRecordingOverlayScale}
              onChange={(event) => onChange({
                ...config,
                ui: {
                  ...config.ui,
                  recording_overlay_scale: Number(event.target.value),
                },
              })}
            />
          </Field>
        </div>
      </div>

      <div className="settings-section">
        <div className="section-title">
          <h2>{text.models.providerSettings}</h2>
        </div>
        <div className="form-grid">
          <Field label={text.models.provider}>
            <select value={providerKey} onChange={(event) => updateProvider(event.target.value)}>
              {Object.entries(providerPresets).map(([key, preset]) => (
                <option key={key} value={key}>{preset.label}</option>
              ))}
            </select>
          </Field>
          <Field label={text.models.apiFormat}>
            <select value={config.llm.api_format} onChange={(event) => updateProviderField("api_format", event.target.value)}>
              <option value="responses">Responses</option>
              <option value="chat_completions">Chat Completions</option>
            </select>
          </Field>
          <Field label="Endpoint">
            <input value={config.llm.endpoint} onChange={(event) => updateProviderField("endpoint", event.target.value)} />
          </Field>
          <Field label={text.models.apiKey}>
            <div className="secret-field">
              <input
                type={showLlmApiKey ? "text" : "password"}
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
        <div className="form-grid">
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
          <label className="toggle-row inline-toggle">
            <input
              type="checkbox"
              checked={config.llm.thinking_enabled}
              onChange={(event) => onChange({ ...config, llm: { ...config.llm, thinking_enabled: event.target.checked } })}
            />
            Thinking
          </label>
          <Field label={text.models.thinkingEffort}>
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

      <div className="settings-section">
        <div className="section-title">
          <h2>
            {text.models.raceMode}
            <span
              className="section-help"
              title={text.models.raceHelp}
            >
              ?
            </span>
          </h2>
        </div>
        <div className="form-grid">
          <label className="toggle-row inline-toggle">
            <input
              type="checkbox"
              checked={config.llm.race_enabled ?? false}
              onChange={(event) => updateRaceEnabled(event.target.checked)}
            />
            {text.models.enableRace}
          </label>
          <Field label={text.models.raceModels} className="field-wide">
            <div className="race-model-list">
              {raceTargetOptions.map((target) => (
                <label key={raceTargetKey(target)} className="race-model-option">
                  <input
                    type="checkbox"
                    checked={raceTargetKeys.has(raceTargetKey(target))}
                    disabled={!config.llm.race_enabled}
                    onChange={(event) => toggleRaceTarget(target, event.target.checked)}
                  />
                  <span>{providerLabel(target.provider)} / {target.model}</span>
                </label>
              ))}
            </div>
          </Field>
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
  return uniqueRaceTargets(Object.keys(providerPresets).flatMap((provider) => (
    modelsForProvider(provider, config.llm.model_presets).map((model) => ({ provider, model }))
  )));
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
