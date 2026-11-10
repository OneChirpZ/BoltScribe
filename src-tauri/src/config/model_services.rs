use super::{AppConfig, LlmConfig, LlmProviderSettings, ModelService};
use crate::learning::LearningConfig;
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) fn default_services() -> Vec<ModelService> {
    [
        ("openai", "OpenAI", "https://api.openai.com/v1", "responses"),
        (
            "volc_ark",
            "火山方舟",
            "https://ark.cn-beijing.volces.com/api/v3",
            "responses",
        ),
        ("custom", "自定义", "", "chat_completions"),
    ]
    .into_iter()
    .map(|(provider, name, endpoint, api_format)| ModelService {
        id: provider.into(),
        name: name.into(),
        provider: provider.into(),
        endpoint: endpoint.into(),
        api_format: api_format.into(),
        api_key: String::new(),
        models: Vec::new(),
    })
    .collect()
}

impl AppConfig {
    /// Resolve transport at use time; task profiles never own a second API key.
    pub fn resolve_llm(&self, profile: &LlmConfig) -> LlmConfig {
        let mut resolved = profile.clone();
        resolved.endpoint.clear();
        resolved.api_key.clear();
        resolved.provider_settings.clear();
        resolved.model_presets.clear();
        resolved.resolved_services = self.model_services.clone();
        if let Some(service) = self
            .model_services
            .iter()
            .find(|s| s.id == profile.service_id)
        {
            resolved.provider = service.provider.clone();
            resolved.api_format = service.api_format.clone();
            resolved.endpoint = service.endpoint.clone();
            resolved.api_key = service.api_key.clone();
        }
        resolved
    }

    pub fn learning_model(&self) -> LearningConfig {
        let mut learning = self.learning.clone();
        learning.llm = self.resolve_llm(&learning.llm);
        // Learning never races; changes to unrelated services need not cancel it.
        learning.llm.resolved_services.clear();
        learning
    }

    /// Called only for documents without the shared registry. Distinct legacy
    /// connections are kept as distinct services, including different keys.
    pub(super) fn migrate_legacy_model_services(&mut self) {
        let mut services = Vec::new();
        let live_id = add_legacy(&mut services, &connection(&self.llm));
        for setting in &self.llm.provider_settings {
            if setting.provider != self.llm.provider {
                add_legacy(&mut services, setting);
            }
        }
        let learning_connection = connection(&self.learning.llm);
        let learning_id = if !learning_connection.api_key.trim().is_empty() {
            add_legacy(&mut services, &learning_connection)
        } else if self.learning.llm.model.trim().is_empty() {
            live_id.clone()
        } else if let Some(service) = services
            .iter()
            .find(|s| s.provider == learning_connection.provider)
        {
            service.id.clone()
        } else {
            add_legacy(&mut services, &learning_connection)
        };
        for setting in &self.learning.llm.provider_settings {
            if setting.provider != self.learning.llm.provider {
                add_legacy(&mut services, setting);
            }
        }
        for profile in [&self.llm, &self.learning.llm] {
            for preset in &profile.model_presets {
                for service in services
                    .iter_mut()
                    .filter(|s| s.provider == preset.provider)
                {
                    add_model(service, &preset.model);
                }
            }
        }
        self.llm.service_id = live_id;
        self.learning.llm.service_id = learning_id;
        for profile in [&mut self.llm, &mut self.learning.llm] {
            if let Some(service) = services.iter_mut().find(|s| s.id == profile.service_id) {
                add_model(service, &profile.model);
            }
            for target in &mut profile.race_targets {
                if target.service_id == profile.provider {
                    target.service_id = profile.service_id.clone();
                } else if let Some(service) =
                    services.iter().find(|s| s.provider == target.service_id)
                {
                    target.service_id = service.id.clone();
                }
                if let Some(service) = services.iter_mut().find(|s| s.id == target.service_id) {
                    add_model(service, &target.model);
                }
            }
            profile.provider_settings.clear();
            profile.model_presets.clear();
            profile.api_key.clear();
            profile.endpoint.clear();
        }
        self.model_services = services;
    }
}

fn connection(profile: &LlmConfig) -> LlmProviderSettings {
    LlmProviderSettings {
        provider: profile.provider.clone(),
        endpoint: profile.endpoint.clone(),
        api_format: profile.api_format.clone(),
        api_key: profile.api_key.clone(),
    }
}

fn add_model(service: &mut ModelService, model: &str) {
    let model = model.trim();
    if !model.is_empty() && !service.models.iter().any(|m| m == model) {
        service.models.push(model.into());
    }
}

fn add_legacy(services: &mut Vec<ModelService>, settings: &LlmProviderSettings) -> String {
    let provider = settings.provider.trim();
    let endpoint = settings.endpoint.trim().trim_end_matches('/');
    let api_format = settings.api_format.trim();
    let api_key = settings.api_key.trim();
    if let Some(service) = services.iter().find(|s| {
        s.provider == provider
            && s.endpoint == endpoint
            && s.api_format == api_format
            && s.api_key == api_key
    }) {
        return service.id.clone();
    }
    let base = if provider.is_empty() {
        "custom"
    } else {
        provider
    };
    let mut id = base.to_string();
    let mut index = 2;
    while services.iter().any(|s| s.id == id) {
        id = format!("{base}-{index}");
        index += 1;
    }
    let label = match base {
        "openai" => "OpenAI",
        "volc_ark" => "火山方舟",
        "custom" => "自定义",
        _ => base,
    };
    let name = if id == base {
        label.to_string()
    } else {
        format!("{label} {}", index - 1)
    };
    services.push(ModelService {
        id: id.clone(),
        name,
        provider: base.into(),
        endpoint: endpoint.into(),
        api_format: api_format.into(),
        api_key: api_key.into(),
        models: Vec::new(),
    });
    id
}

pub(super) fn normalize_services(services: &mut [ModelService]) {
    for service in services {
        service.id = service.id.trim().into();
        service.name = service.name.trim().into();
        service.provider = service.provider.trim().into();
        service.endpoint = service.endpoint.trim().trim_end_matches('/').into();
        service.api_key = service.api_key.trim().into();
        let mut seen = HashSet::new();
        service.models = service
            .models
            .iter()
            .map(|m| m.trim())
            .filter(|m| !m.is_empty() && seen.insert((*m).to_string()))
            .map(str::to_string)
            .collect();
    }
}

/// Legacy fields are accepted only at the import boundary. They disappear from
/// every subsequent save/export after being moved into model_services.
pub(super) fn legacy_import_defaults(
    defaults: &mut Value,
    schema: &mut Value,
    imported: &mut Value,
) {
    defaults["model_services"] = json!([]);
    for prefix in ["/llm", "/learning/llm"] {
        let transport = json!({
            "provider":"openai", "endpoint":"https://api.openai.com/v1",
            "api_format":"responses", "api_key":"", "provider_settings":[], "model_presets":[]
        });
        for target in [&mut *defaults, &mut *schema] {
            if let Some(object) = target.pointer_mut(prefix).and_then(Value::as_object_mut) {
                for (key, value) in transport.as_object().unwrap() {
                    object.insert(key.clone(), value.clone());
                }
            }
        }
        if let Some(targets) = imported
            .pointer_mut(&format!("{prefix}/race_targets"))
            .and_then(Value::as_array_mut)
        {
            for target in targets {
                if let Some(object) = target.as_object_mut() {
                    if let Some(provider) = object.remove("provider") {
                        object.entry("service_id").or_insert(provider);
                    }
                }
            }
        }
        let profile = schema.pointer_mut(prefix).unwrap();
        profile["provider_settings"] =
            json!([{"provider":"", "endpoint":"", "api_format":"", "api_key":""}]);
        profile["model_presets"] = json!([{"provider":"", "model":""}]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_tasks_resolve_one_service_without_copying_credentials_into_profiles() {
        let mut config = AppConfig::default();
        config.model_services[0].api_key = "shared-key".into();
        config.learning.enabled = true;
        config.learning.llm.model = "quality-model".into();
        assert_eq!(config.resolve_llm(&config.llm).api_key, "shared-key");
        assert_eq!(config.learning_model().llm.api_key, "shared-key");
        assert!(config.learning_model().can_observe());
        assert!(config.llm.api_key.is_empty());
        assert!(config.learning.llm.api_key.is_empty());
        config.model_services[0].api_key = "rotated-key".into();
        assert_eq!(config.learning_model().llm.api_key, "rotated-key");
        let raw = serde_json::to_value(&config).unwrap();
        assert!(raw["llm"].get("api_key").is_none());
        assert!(raw["learning"]["llm"].get("api_key").is_none());
        assert_eq!(raw["model_services"][0]["api_key"], "rotated-key");
    }

    #[test]
    fn a_removed_service_cannot_fall_back_to_stale_task_credentials() {
        let mut config = AppConfig::default();
        config.learning.enabled = true;
        config.learning.llm.api_key = "stale-key".into();
        config.learning.llm.model = "quality-model".into();
        config.model_services.clear();
        assert!(!config.learning_model().can_observe());
        assert!(config.learning_model().llm.api_key.is_empty());
    }

    #[test]
    fn migration_deduplicates_identical_connections_but_preserves_distinct_keys() {
        let mut config = AppConfig::default();
        config.llm.api_key = "live-key".into();
        config.llm.endpoint = "https://example.test/v1".into();
        config.learning.llm = config.llm.clone();
        config.learning.llm.model = "quality-model".into();
        config.migrate_legacy_model_services();
        assert_eq!(config.model_services.len(), 1);
        assert_eq!(config.llm.service_id, config.learning.llm.service_id);
        config.learning.llm.api_key = "different-key".into();
        config.learning.llm.endpoint = "https://example.test/v1".into();
        config.llm.api_key = "live-key".into();
        config.llm.endpoint = "https://example.test/v1".into();
        config.migrate_legacy_model_services();
        assert_eq!(config.model_services.len(), 2);
        assert_ne!(config.llm.service_id, config.learning.llm.service_id);
        assert_eq!(config.learning_model().llm.api_key, "different-key");
    }

    #[test]
    fn legacy_import_and_new_export_preserve_tasks_catalog_and_race_service_references() {
        let mut raw = serde_json::to_value(AppConfig::default()).unwrap();
        raw.as_object_mut().unwrap().remove("model_services");
        for path in ["/llm", "/learning/llm"] {
            let profile = raw.pointer_mut(path).unwrap();
            profile.as_object_mut().unwrap().remove("service_id");
            profile["provider"] = json!("openai");
            profile["endpoint"] = json!("https://example.test/v1");
            profile["api_format"] = json!("responses");
            profile["api_key"] = json!("shared-placeholder");
        }
        raw["learning"]["llm"]["model"] = json!("quality-model");
        raw["llm"]["provider_settings"] = json!([{
            "provider":"custom", "endpoint":"https://second.example.test/v1",
            "api_format":"chat_completions", "api_key":"second-placeholder"
        }]);
        raw["llm"]["model_presets"] = json!([{"provider":"custom", "model":"backup-model"}]);
        raw["llm"]["race_targets"] = json!([
            {"provider":"openai", "model":"fast-model"},
            {"provider":"custom", "model":"backup-model"}
        ]);
        let imported = crate::config::ConfigStore::import_json(&raw.to_string()).unwrap();
        assert!(imported.report.invalid_fields.is_empty());
        assert!(imported.report.unknown_fields.is_empty());
        let config = imported.config;
        assert_eq!(config.model_services.len(), 2);
        assert_eq!(config.llm.service_id, config.learning.llm.service_id);
        assert_eq!(config.learning_model().llm.api_key, "shared-placeholder");
        assert_eq!(config.learning.llm.model, "quality-model");
        assert_eq!(config.learning.llm.thinking_effort, "high");
        assert_eq!(config.llm.race_targets[1].service_id, "custom");
        assert!(config.model_services[1]
            .models
            .contains(&"backup-model".to_string()));
        let export = crate::config::ConfigStore::export_json(&config).unwrap();
        assert_eq!(export.matches("shared-placeholder").count(), 1);
        let restored = crate::config::ConfigStore::import_json(&export)
            .unwrap()
            .config;
        assert_eq!(restored.model_services, config.model_services);
        assert_eq!(restored.learning_model(), config.learning_model());
        assert_eq!(restored.llm.race_targets, config.llm.race_targets);
    }
}
