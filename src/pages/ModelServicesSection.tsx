import { useEffect, useState } from "react";
import Field from "../components/Field";
import { modelManagerText, removeModelService } from "../domain/modelServices";
import { providerPresets } from "../domain/providers";
import type { AppConfig, ModelService } from "../types";
import { automaticCatalog, type useModelCatalogs } from "./useModelCatalogs";

export default function ModelServicesSection({ config, onChange, catalogs }: {
  catalogs: ReturnType<typeof useModelCatalogs>;
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}) {
  const text = modelManagerText(config.ui.app_language);
  const services = config.model_services;
  const [selectedId, setSelectedId] = useState(config.llm.service_id);
  const service = services.find((item) => item.id === selectedId) ?? services[0];
  const [editingId, setEditingId] = useState<string | null>(null);
  const [showKey, setShowKey] = useState(false);

  useEffect(() => {
    const hide = () => setShowKey(false);
    window.addEventListener("blur", hide);
    document.addEventListener("visibilitychange", hide);
    return () => {
      window.removeEventListener("blur", hide);
      document.removeEventListener("visibilitychange", hide);
    };
  }, []);

  function edit(id: string) {
    setSelectedId(id);
    setEditingId(id);
    setShowKey(false);
  }

  function update(patch: Partial<ModelService>) {
    if (!service) return;
    onChange({ ...config, model_services: services.map((item) => item.id === service.id ? { ...item, ...patch } : item) });
  }

  function add(provider: "openai" | "custom") {
    const preset = providerPresets[provider];
    const next: ModelService = {
      id: crypto.randomUUID(), name: provider === "openai" ? "OpenAI" : text.newService, provider,
      api_format: preset.api_format, endpoint: preset.endpoint, api_key: "", models: [],
    };
    onChange({ ...config, model_services: [...services, next] });
    edit(next.id);
  }

  function remove() {
    if (!service || !window.confirm(text.removeConfirm)) return;
    const next = removeModelService(config, service.id);
    onChange(next);
    setSelectedId(next.model_services[0]?.id ?? "");
    setEditingId(null);
    setShowKey(false);
  }

  return (
    <section className="settings-section model-services-section" aria-label={text.services}>
      <div className="section-title">
        <h2>{text.services}</h2>
        <div className="section-actions">
          {!services.some((item) => item.provider === "openai") ? (
            <button className="secondary small" type="button" onClick={() => add("openai")}>{text.addOpenAI}</button>
          ) : null}
          <button className="secondary small" type="button" onClick={() => add("custom")}>{text.add}</button>
        </div>
      </div>
      {services.length ? <div className="model-service-list">
        {services.map((item) => {
          const selected = item.id === service.id;
          const catalog = catalogs.catalog(item);
          let count = text.modelCount(new Set(item.models.map((model) => model.trim()).filter(Boolean)).size);
          if (automaticCatalog(item)) {
            if (catalog?.loading) count = text.fetching;
            else if (catalog?.models) count = text.modelCount(catalog.models.length);
            else if (catalog?.error) count = text.fetchError;
            else count = item.api_key.trim() && item.endpoint.trim() ? text.saveToFetch : text.unconfigured;
          }
          return (
            <div key={item.id} className={`model-service-row${selected ? " selected" : ""}`} role="group" aria-label={item.name || item.id}>
              <button type="button" className="model-service-select" aria-pressed={selected} aria-expanded={editingId === item.id} onClick={() => edit(item.id)}>
                <span>{item.name || item.id}</span>
              </button>
              <span className="model-service-count">{count}</span>
              <div className="model-service-actions">
                {automaticCatalog(item) ? <button type="button" className="secondary small" disabled={!catalogs.canFetch(item) || catalog?.loading}
                  onClick={() => catalogs.refresh(item)}>{text.refreshModels}</button> : null}
                <button type="button" className="secondary small" aria-expanded={editingId === item.id} onClick={() => {
                  if (editingId !== item.id) edit(item.id);
                  else {
                    setEditingId(null);
                    setShowKey(false);
                  }
                }}>{editingId === item.id ? text.closeSettings : text.editSettings}</button>
              </div>
              {catalog?.error ? <span className="model-service-error" role="status">{text.fetchError} · {catalog.error}</span> : null}
            </div>
          );
        })}
      </div> : <p className="settings-help-text">{text.empty}</p>}
      {service && editingId === service.id ? <div className="form-grid model-service-grid" role="group" aria-label={`${service.name} · ${text.connectionSettings}`}>
        <Field label={text.serviceName}>
          <input value={service.name} onChange={(event) => update({ name: event.target.value })} />
        </Field>
        <Field label={text.provider}>
          <select value={service.provider} onChange={(event) => {
            const provider = event.target.value as keyof typeof providerPresets;
            const preset = providerPresets[provider];
            setShowKey(false);
            update({ provider, endpoint: preset.endpoint, api_format: preset.api_format, api_key: "" });
          }}>
            {Object.entries(providerPresets).map(([key, preset]) => <option key={key} value={key}>{preset.label}</option>)}
          </select>
        </Field>
        <Field label={text.format}>
          <select value={service.api_format} onChange={(event) => update({ api_format: event.target.value })}>
            <option value="responses">Responses</option><option value="chat_completions">Chat Completions</option>
          </select>
        </Field>
        <Field label={text.endpoint} className="model-service-endpoint">
          <input value={service.endpoint} spellCheck={false} autoComplete="off" onChange={(event) => update({ endpoint: event.target.value })} />
        </Field>
        <Field label={text.apiKey} group>
          <div className="secret-field">
            <input aria-label={text.apiKey} type={showKey ? "text" : "password"} value={service.api_key} autoComplete="new-password" onChange={(event) => update({ api_key: event.target.value })} />
            <button className="secondary small" type="button" onClick={() => setShowKey((shown) => !shown)}>
              {config.ui.app_language === "en-US" ? (showKey ? "Hide" : "Show") : (showKey ? "隐藏" : "显示")}
            </button>
          </div>
        </Field>
        {!automaticCatalog(service) ? <Field label={text.catalog} className="field-wide">
          <textarea rows={3} value={service.models.join("\n")} spellCheck={false} onChange={(event) => update({ models: event.target.value.split("\n") })} />
        </Field> : null}
        <div className="field-wide model-service-destructive">
          <button type="button" className="secondary small" onClick={remove}>{text.remove}</button>
        </div>
      </div> : null}
    </section>
  );
}
