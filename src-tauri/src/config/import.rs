use super::model::{AppConfig, ConfigImportReport, ConfigImportResult};
use super::normalize::{
    asr_auth_mode_field_present, correction_text_field_presence,
    fill_missing_correction_text_fields,
};
use super::store::ConfigStore;
use crate::paths;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

pub(super) const CONFIG_EXPORT_FORMAT: &str = "boltscribe.config";
pub(super) const CONFIG_EXPORT_VERSION: u32 = 2;

#[derive(Debug, Serialize)]
struct ConfigExportEnvelope {
    format: String,
    version: u32,
    exported_at: String,
    config: AppConfig,
}

impl ConfigStore {
    pub fn export_json(config: &AppConfig) -> Result<String> {
        let mut config = config.clone();
        config.normalize();
        let envelope = ConfigExportEnvelope {
            format: CONFIG_EXPORT_FORMAT.to_string(),
            version: CONFIG_EXPORT_VERSION,
            exported_at: Utc::now().to_rfc3339(),
            config,
        };
        serde_json::to_string_pretty(&envelope).context("Failed to serialize config export")
    }

    pub fn export_file(config: &AppConfig) -> Result<PathBuf> {
        let raw = Self::export_json(config)?;
        let dir = paths::app_dir().map_err(anyhow::Error::msg)?;
        fs::create_dir_all(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;
        let filename = format!(
            "boltscribe-config-{}.json",
            Utc::now().format("%Y%m%d-%H%M%S")
        );
        let path = dir.join(filename);
        fs::write(&path, raw).with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(path)
    }

    pub fn import_json(raw: &str) -> Result<ConfigImportResult> {
        let value: serde_json::Value =
            serde_json::from_str(raw).context("Failed to parse imported config JSON")?;
        let mut report = ConfigImportReport::default();
        let mut imported_config = extract_imported_config(value, &mut report)?;
        let correction_text_presence = correction_text_field_presence(&imported_config);
        let asr_auth_mode_present = asr_auth_mode_field_present(&imported_config);
        let legacy_models = imported_config.get("model_services").is_none();
        let mut default_config = serde_json::to_value(AppConfig::default())?;
        let mut schema = config_schema_value()?;
        if legacy_models {
            super::model_services::legacy_import_defaults(
                &mut default_config,
                &mut schema,
                &mut imported_config,
            );
        }

        collect_missing_fields(
            &imported_config,
            &default_config,
            &schema,
            "",
            &mut report.missing_fields,
        );
        collect_unknown_fields(&imported_config, &schema, "", &mut report.unknown_fields);

        let mut merged_config = default_config;
        merge_imported_value(
            &mut merged_config,
            imported_config,
            &schema,
            "",
            &mut report.invalid_fields,
        );
        let mut config: AppConfig = serde_json::from_value(merged_config)
            .context("Failed to apply imported configuration")?;
        if !asr_auth_mode_present {
            config.asr.auth_mode.clear();
        }
        if legacy_models {
            config.migrate_legacy_model_services();
        }
        config.normalize();
        fill_missing_correction_text_fields(&mut config, correction_text_presence);

        Ok(ConfigImportResult { config, report })
    }
}

pub(super) fn extract_imported_config(
    value: serde_json::Value,
    report: &mut ConfigImportReport,
) -> Result<serde_json::Value> {
    let Some(object) = value.as_object() else {
        anyhow::bail!("Imported config must be a JSON object");
    };

    let looks_like_envelope = object.contains_key("config")
        || object.contains_key("format")
        || object.contains_key("version")
        || object.contains_key("exported_at");
    if !looks_like_envelope {
        report
            .notes
            .push("Imported legacy plain config JSON without export metadata.".to_string());
        return Ok(value);
    }

    report.format = object
        .get("format")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    report.version = object
        .get("version")
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());

    match report.format.as_deref() {
        Some(CONFIG_EXPORT_FORMAT) => {}
        Some(format) => report.notes.push(format!(
            "Import format is \"{format}\", expected \"{CONFIG_EXPORT_FORMAT}\"."
        )),
        None => report
            .notes
            .push("Import file has no format field.".to_string()),
    }

    match report.version {
        Some(version) if version > CONFIG_EXPORT_VERSION => report.notes.push(format!(
            "Import version {version} is newer than supported version {CONFIG_EXPORT_VERSION}; unknown fields were ignored."
        )),
        Some(_) => {}
        None => report
            .notes
            .push("Import file has no numeric version field.".to_string()),
    }

    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "format" | "version" | "exported_at" | "config"
        ) {
            report.unknown_fields.push(format!("envelope.{key}"));
        }
    }

    object
        .get("config")
        .cloned()
        .context("Import file does not contain a config object")
}

pub(super) fn config_schema_value() -> Result<serde_json::Value> {
    let mut schema = serde_json::to_value(AppConfig::default())?;
    let mut service = serde_json::to_value(super::model_services::default_services().remove(0))?;
    service["models"] = serde_json::json!([""]);
    set_schema_array(&mut schema, &["model_services"], service);
    set_schema_array(&mut schema, &["hotkeys"], serde_json::json!(""));
    set_schema_array(&mut schema, &["hotkey_enabled"], serde_json::json!(false));
    set_schema_array(
        &mut schema,
        &["audio", "input_device_priority"],
        serde_json::json!({
            "id": "",
            "name": ""
        }),
    );
    set_schema_array(
        &mut schema,
        &["audio", "input_device_blacklist"],
        serde_json::json!({
            "id": "",
            "name": ""
        }),
    );
    set_schema_array(
        &mut schema,
        &["audio", "output_volume_ducking", "device_name_whitelist"],
        serde_json::json!(""),
    );
    set_schema_array(
        &mut schema,
        &["llm", "provider_settings"],
        serde_json::json!({
            "provider": "",
            "endpoint": "",
            "api_format": "",
            "api_key": ""
        }),
    );
    set_schema_array(&mut schema, &["llm", "race_models"], serde_json::json!(""));
    set_schema_array(
        &mut schema,
        &["llm", "race_targets"],
        serde_json::json!({
            "service_id": "",
            "model": ""
        }),
    );
    set_schema_array(
        &mut schema,
        &["llm", "model_presets"],
        serde_json::json!({
            "provider": "",
            "model": ""
        }),
    );
    // The independent learning model has the same provider schema, including
    // array item types, while its defaults remain separate from live correction.
    schema["learning"]["llm"] = schema["llm"].clone();
    set_schema_array(
        &mut schema,
        &["correction", "variables"],
        serde_json::json!({
            "name": "",
            "value": ""
        }),
    );
    set_schema_array(
        &mut schema,
        &["correction", "disabled_dictionary_terms"],
        serde_json::json!(""),
    );
    set_schema_array(
        &mut schema,
        &["correction", "correction_rules"],
        serde_json::json!({
            "source": "",
            "target": "",
            "note": ""
        }),
    );
    set_schema_array(
        &mut schema,
        &["correction", "dictionary"],
        serde_json::json!({
            "term": "",
            "aliases": [""],
            "note": ""
        }),
    );
    Ok(schema)
}

pub(super) fn set_schema_array(
    schema: &mut serde_json::Value,
    path: &[&str],
    item: serde_json::Value,
) {
    let mut current = schema;
    for key in &path[..path.len().saturating_sub(1)] {
        let Some(next) = current.get_mut(*key) else {
            return;
        };
        current = next;
    }
    if let Some(last) = path.last() {
        current[*last] = serde_json::Value::Array(vec![item]);
    }
}

pub(super) fn collect_missing_fields(
    imported: &serde_json::Value,
    defaults: &serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
    missing_fields: &mut Vec<String>,
) {
    match (imported, defaults, schema) {
        (
            serde_json::Value::Object(imported_object),
            serde_json::Value::Object(default_object),
            serde_json::Value::Object(schema_object),
        ) => {
            for (key, default_value) in default_object {
                let field_path = join_field_path(path, key);
                let Some(imported_value) = imported_object.get(key) else {
                    missing_fields.push(field_path);
                    continue;
                };
                let schema_value = schema_object.get(key).unwrap_or(default_value);
                collect_missing_fields(
                    imported_value,
                    default_value,
                    schema_value,
                    &field_path,
                    missing_fields,
                );
            }
        }
        (serde_json::Value::Array(imported_array), _, serde_json::Value::Array(schema_array)) => {
            let Some(schema_item) = schema_array.first() else {
                return;
            };
            for (index, item) in imported_array.iter().enumerate() {
                collect_missing_fields(
                    item,
                    schema_item,
                    schema_item,
                    &format!("{path}[{index}]"),
                    missing_fields,
                );
            }
        }
        _ => {}
    }
}

pub(super) fn collect_unknown_fields(
    imported: &serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
    unknown_fields: &mut Vec<String>,
) {
    match (imported, schema) {
        (serde_json::Value::Object(imported_object), serde_json::Value::Object(schema_object)) => {
            for (key, imported_value) in imported_object {
                let field_path = join_field_path(path, key);
                let Some(schema_value) = schema_object.get(key) else {
                    unknown_fields.push(field_path);
                    continue;
                };
                collect_unknown_fields(imported_value, schema_value, &field_path, unknown_fields);
            }
        }
        (serde_json::Value::Array(imported_array), serde_json::Value::Array(schema_array)) => {
            let Some(schema_item) = schema_array.first() else {
                return;
            };
            for (index, item) in imported_array.iter().enumerate() {
                collect_unknown_fields(
                    item,
                    schema_item,
                    &format!("{path}[{index}]"),
                    unknown_fields,
                );
            }
        }
        _ => {}
    }
}

pub(super) fn merge_imported_value(
    target: &mut serde_json::Value,
    imported: serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
    invalid_fields: &mut Vec<String>,
) {
    match (target, imported, schema) {
        (
            serde_json::Value::Object(target_object),
            serde_json::Value::Object(imported_object),
            serde_json::Value::Object(schema_object),
        ) => {
            for (key, imported_value) in imported_object {
                let field_path = join_field_path(path, &key);
                let (Some(target_value), Some(schema_value)) =
                    (target_object.get_mut(&key), schema_object.get(&key))
                else {
                    continue;
                };
                merge_imported_value(
                    target_value,
                    imported_value,
                    schema_value,
                    &field_path,
                    invalid_fields,
                );
            }
        }
        (serde_json::Value::Object(_), imported_value, serde_json::Value::Object(_)) => {
            if !imported_value.is_object() {
                invalid_fields.push(report_path(path));
            }
        }
        (
            target_value @ serde_json::Value::Array(_),
            serde_json::Value::Array(imported_array),
            serde_json::Value::Array(schema_array),
        ) => {
            let Some(schema_item) = schema_array.first() else {
                *target_value = serde_json::Value::Array(imported_array);
                return;
            };
            let merged_array = imported_array
                .into_iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let item_path = format!("{path}[{index}]");
                    if item.is_object() && schema_item.is_object() {
                        let mut target_item = schema_item.clone();
                        merge_imported_value(
                            &mut target_item,
                            item,
                            schema_item,
                            &item_path,
                            invalid_fields,
                        );
                        Some(target_item)
                    } else if value_matches_schema(&item, schema_item, &item_path) {
                        Some(item)
                    } else {
                        invalid_fields.push(item_path);
                        None
                    }
                })
                .collect();
            *target_value = serde_json::Value::Array(merged_array);
        }
        (serde_json::Value::Array(_), imported_value, serde_json::Value::Array(_)) => {
            if !imported_value.is_array() {
                invalid_fields.push(report_path(path));
            }
        }
        (target_value, imported_value, _) => {
            if value_matches_schema(&imported_value, schema, path) {
                *target_value = imported_value;
            } else {
                invalid_fields.push(report_path(path));
            }
        }
    }
}

pub(super) fn value_matches_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
) -> bool {
    match (value, schema) {
        (serde_json::Value::Null, serde_json::Value::Null)
        | (serde_json::Value::Bool(_), serde_json::Value::Bool(_))
        | (serde_json::Value::Number(_), serde_json::Value::Number(_))
        | (serde_json::Value::String(_), serde_json::Value::String(_))
        | (serde_json::Value::Array(_), serde_json::Value::Array(_))
        | (serde_json::Value::Object(_), serde_json::Value::Object(_)) => true,
        (serde_json::Value::Number(_), serde_json::Value::Null)
        | (serde_json::Value::Null, serde_json::Value::Number(_)) => {
            matches!(
                path,
                "llm.max_output_tokens" | "learning.llm.max_output_tokens"
            )
        }
        _ => false,
    }
}

pub(super) fn join_field_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

pub(super) fn report_path(path: &str) -> String {
    if path.is_empty() {
        "config".to_string()
    } else {
        path.to_string()
    }
}
