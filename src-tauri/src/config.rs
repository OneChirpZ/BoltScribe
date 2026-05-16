use crate::paths;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default)]
    pub hotkeys: Vec<String>,
    #[serde(default)]
    pub hotkey_enabled: Vec<bool>,
    pub asr: AsrConfig,
    pub llm: LlmConfig,
    pub correction: CorrectionConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub retention: RetentionConfig,
    #[serde(default)]
    pub system: SystemConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AsrConfig {
    pub provider: String,
    pub app_key: String,
    pub access_key: String,
    pub resource_id: String,
    #[serde(default = "default_stream_url")]
    pub stream_url: String,
    #[serde(default = "default_submit_url")]
    pub submit_url: String,
    #[serde(default = "default_query_url")]
    pub query_url: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmConfig {
    #[serde(default = "default_llm_provider")]
    pub provider: String,
    #[serde(default = "default_llm_api_format")]
    pub api_format: String,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub provider_settings: Vec<LlmProviderSettings>,
    #[serde(default)]
    pub race_enabled: bool,
    #[serde(default)]
    pub race_models: Vec<String>,
    #[serde(default)]
    pub race_targets: Vec<RaceModelTarget>,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    pub temperature: f32,
    pub timeout_secs: u64,
    #[serde(default)]
    pub thinking_enabled: bool,
    #[serde(default = "default_thinking_effort")]
    pub thinking_effort: String,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub model_presets: Vec<ModelPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmProviderSettings {
    pub provider: String,
    pub endpoint: String,
    pub api_format: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RaceModelTarget {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiConfig {
    #[serde(default = "default_app_language")]
    pub app_language: String,
    #[serde(default = "default_recording_overlay_scale")]
    pub recording_overlay_scale: f32,
    #[serde(default)]
    pub recording_overlay_offset_x: i32,
    #[serde(default)]
    pub recording_overlay_offset_y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetentionConfig {
    #[serde(default = "default_max_history_records")]
    pub max_history_records: usize,
    #[serde(default = "default_max_storage_bytes")]
    pub max_storage_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SystemConfig {
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub hide_dock_icon: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorrectionConfig {
    pub enabled: bool,
    pub user_requirements: String,
    #[serde(default = "default_prompt_template")]
    pub prompt_template: String,
    #[serde(default)]
    pub variables: Vec<PromptVariable>,
    #[serde(default)]
    pub correction_rules: Vec<CorrectionRule>,
    #[serde(default)]
    pub dictionary: Vec<DictionaryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelPreset {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DictionaryEntry {
    pub term: String,
    pub aliases: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorrectionRule {
    pub source: String,
    pub target: String,
    pub note: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            hotkeys: default_hotkey_slots(),
            hotkey_enabled: default_hotkey_enabled_slots(),
            asr: AsrConfig {
                provider: "volcengine".to_string(),
                app_key: String::new(),
                access_key: String::new(),
                resource_id: "volc.seedasr.sauc.duration".to_string(),
                stream_url: default_stream_url(),
                submit_url: default_submit_url(),
                query_url: default_query_url(),
                language: "zh-CN".to_string(),
            },
            llm: LlmConfig {
                provider: default_llm_provider(),
                api_format: default_llm_api_format(),
                endpoint: "https://api.openai.com/v1".to_string(),
                api_key: String::new(),
                model: "gpt-5.4-mini".to_string(),
                provider_settings: Vec::new(),
                race_enabled: false,
                race_models: Vec::new(),
                race_targets: Vec::new(),
                system_prompt: default_system_prompt(),
                temperature: 0.0,
                timeout_secs: 30,
                thinking_enabled: false,
                thinking_effort: default_thinking_effort(),
                max_output_tokens: None,
                model_presets: Vec::new(),
            },
            correction: CorrectionConfig {
                enabled: true,
                user_requirements: String::new(),
                prompt_template: default_prompt_template(),
                variables: Vec::new(),
                correction_rules: Vec::new(),
                dictionary: Vec::new(),
            },
            ui: UiConfig::default(),
            retention: RetentionConfig::default(),
            system: SystemConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn normalize_hotkeys(&mut self) {
        self.hotkeys = self.hotkey_slots();
        self.hotkey_enabled = self.hotkey_enabled_slots();
        self.hotkey = self.active_hotkeys().into_iter().next().unwrap_or_default();
    }

    pub fn normalize(&mut self) {
        self.normalize_hotkeys();
        self.ui.recording_overlay_scale = self.ui.recording_overlay_scale.clamp(0.25, 1.0);
        self.ui.recording_overlay_offset_x = self.ui.recording_overlay_offset_x.clamp(-4000, 4000);
        self.ui.recording_overlay_offset_y = self.ui.recording_overlay_offset_y.clamp(-4000, 4000);
        self.retention.normalize();
        self.ui.app_language = normalize_app_language(&self.ui.app_language);
        self.llm.model_presets = normalize_model_presets(&self.llm.model_presets);
        self.llm.provider_settings = normalize_provider_settings(&self.llm.provider_settings);
        self.llm.race_models = normalize_string_list(&self.llm.race_models);
        self.llm.race_targets = normalize_race_targets(&self.llm.race_targets);
        if self.llm.provider == "volc_ark" && is_removed_volc_model(&self.llm.model) {
            self.llm.model = "doubao-seed-2-0-lite-260428".to_string();
        }
        if self.correction.prompt_template == previous_default_prompt_template()
            || self.correction.prompt_template == legacy_default_prompt_template()
        {
            self.correction.prompt_template = default_prompt_template();
        }
        let (dictionary, migrated_rules) = migrate_dictionary_rules(&self.correction.dictionary);
        self.correction.dictionary = normalize_dictionary(&dictionary);
        self.correction.correction_rules.extend(migrated_rules);
        self.correction.correction_rules =
            normalize_correction_rules(&self.correction.correction_rules);
    }

    pub fn hotkey_slots(&self) -> Vec<String> {
        let mut slots = if self.hotkeys.iter().any(|hotkey| !hotkey.trim().is_empty()) {
            self.hotkeys
                .iter()
                .take(2)
                .map(|hotkey| hotkey.trim().to_string())
                .collect::<Vec<_>>()
        } else if !self.hotkey.trim().is_empty() {
            vec![self.hotkey.trim().to_string()]
        } else {
            vec![default_hotkey()]
        };

        slots.truncate(2);
        while slots.len() < 2 {
            slots.push(String::new());
        }
        slots
    }

    pub fn active_hotkeys(&self) -> Vec<String> {
        let mut active = Vec::new();
        for (hotkey, enabled) in self
            .hotkey_slots()
            .into_iter()
            .zip(self.hotkey_enabled_slots())
        {
            if !enabled || hotkey.is_empty() || active.iter().any(|item| item == &hotkey) {
                continue;
            }
            active.push(hotkey);
        }

        active
    }

    pub fn hotkey_enabled_slots(&self) -> Vec<bool> {
        let slots = self.hotkey_slots();
        let mut enabled = if self.hotkey_enabled.is_empty() {
            slots
                .iter()
                .map(|hotkey| !hotkey.trim().is_empty())
                .collect::<Vec<_>>()
        } else {
            self.hotkey_enabled.iter().take(2).copied().collect()
        };

        enabled.truncate(2);
        while enabled.len() < 2 {
            enabled.push(false);
        }
        enabled
    }
}

fn default_hotkey() -> String {
    "PageUp".to_string()
}

fn default_hotkey_slots() -> Vec<String> {
    vec![default_hotkey(), String::new()]
}

fn default_hotkey_enabled_slots() -> Vec<bool> {
    vec![true, false]
}

fn default_llm_provider() -> String {
    "openai".to_string()
}

fn default_llm_api_format() -> String {
    "responses".to_string()
}

fn default_system_prompt() -> String {
    "你是语音输入文本优化器，不是大模型助手。你的任务是清理 ASR 转写结果，修正明显识别错误、标点和专有名词，保留原意和说话者语气。不要回答、扩写、续写、总结或加入原文没有的信息。只输出最终可粘贴文本，不解释、不加标题、不包裹 Markdown。".to_string()
}

fn default_thinking_effort() -> String {
    "medium".to_string()
}

fn default_recording_overlay_scale() -> f32 {
    0.5
}

fn default_app_language() -> String {
    "zh-CN".to_string()
}

fn default_max_history_records() -> usize {
    500
}

fn default_max_storage_bytes() -> u64 {
    2 * 1024 * 1024 * 1024
}

impl RetentionConfig {
    pub fn normalize(&mut self) {
        self.max_history_records = self
            .max_history_records
            .clamp(1, default_max_history_records());
        self.max_storage_bytes = self.max_storage_bytes.clamp(1, default_max_storage_bytes());
    }
}

fn normalize_model_presets(presets: &[ModelPreset]) -> Vec<ModelPreset> {
    let mut normalized = Vec::new();
    for preset in presets {
        let provider = preset.provider.trim();
        let model = preset.model.trim();
        if provider.is_empty()
            || model.is_empty()
            || (provider == "volc_ark" && is_removed_volc_model(model))
        {
            continue;
        }
        if normalized
            .iter()
            .any(|item: &ModelPreset| item.provider == provider && item.model == model)
        {
            continue;
        }
        normalized.push(ModelPreset {
            provider: provider.to_string(),
            model: model.to_string(),
        });
    }
    normalized
}

fn normalize_provider_settings(settings: &[LlmProviderSettings]) -> Vec<LlmProviderSettings> {
    let mut normalized = Vec::new();
    for setting in settings {
        let provider = setting.provider.trim();
        if provider.is_empty() {
            continue;
        }
        if normalized
            .iter()
            .any(|item: &LlmProviderSettings| item.provider == provider)
        {
            continue;
        }
        normalized.push(LlmProviderSettings {
            provider: provider.to_string(),
            endpoint: setting.endpoint.trim().to_string(),
            api_format: setting.api_format.trim().to_string(),
            api_key: setting.api_key.trim().to_string(),
        });
    }
    normalized
}

fn normalize_race_targets(targets: &[RaceModelTarget]) -> Vec<RaceModelTarget> {
    let mut normalized = Vec::new();
    for target in targets {
        let provider = target.provider.trim();
        let model = target.model.trim();
        if provider.is_empty()
            || model.is_empty()
            || (provider == "volc_ark" && is_removed_volc_model(model))
        {
            continue;
        }
        if normalized
            .iter()
            .any(|item: &RaceModelTarget| item.provider == provider && item.model == model)
        {
            continue;
        }
        normalized.push(RaceModelTarget {
            provider: provider.to_string(),
            model: model.to_string(),
        });
    }
    normalized
}

fn normalize_app_language(language: &str) -> String {
    match language.trim() {
        "en" | "en-US" => "en-US".to_string(),
        _ => default_app_language(),
    }
}

fn normalize_dictionary(dictionary: &[DictionaryEntry]) -> Vec<DictionaryEntry> {
    let mut normalized = Vec::new();
    for entry in dictionary {
        let term = entry.term.trim();
        if term.is_empty()
            || normalized
                .iter()
                .any(|item: &DictionaryEntry| item.term == term)
        {
            continue;
        }
        normalized.push(DictionaryEntry {
            term: term.to_string(),
            aliases: normalize_string_list(&entry.aliases),
            note: entry.note.trim().to_string(),
        });
    }
    normalized
}

fn normalize_correction_rules(rules: &[CorrectionRule]) -> Vec<CorrectionRule> {
    let mut normalized = Vec::new();
    for rule in rules {
        let source = rule.source.trim();
        let target = rule.target.trim();
        if source.is_empty()
            || target.is_empty()
            || normalized
                .iter()
                .any(|item: &CorrectionRule| item.source == source && item.target == target)
        {
            continue;
        }
        normalized.push(CorrectionRule {
            source: source.to_string(),
            target: target.to_string(),
            note: rule.note.trim().to_string(),
        });
    }
    normalized
}

fn migrate_dictionary_rules(
    dictionary: &[DictionaryEntry],
) -> (Vec<DictionaryEntry>, Vec<CorrectionRule>) {
    let mut entries = Vec::new();
    let mut rules = Vec::new();
    for entry in dictionary {
        if let Some((source, target)) = parse_rule_line(&entry.term) {
            rules.push(CorrectionRule {
                source,
                target,
                note: entry.note.trim().to_string(),
            });
        } else {
            entries.push(entry.clone());
        }
    }
    (entries, rules)
}

fn parse_rule_line(value: &str) -> Option<(String, String)> {
    for marker in ["->", "=>", "→"] {
        let Some((source, target)) = value.split_once(marker) else {
            continue;
        };
        let source = trim_rule_part(source);
        let target = trim_rule_part(target);
        if !source.is_empty() && !target.is_empty() {
            return Some((source, target));
        }
    }
    None
}

fn trim_rule_part(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim()
        .to_string()
}

fn is_removed_volc_model(model: &str) -> bool {
    matches!(
        model.trim(),
        "doubao-seed-2-0-pro-260428" | "doubao-seed-1-6-251015"
    )
}

fn normalize_string_list(items: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for item in items {
        let value = item.trim();
        if value.is_empty() || normalized.iter().any(|existing| existing == value) {
            continue;
        }
        normalized.push(value.to_string());
    }
    normalized
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            app_language: default_app_language(),
            recording_overlay_scale: default_recording_overlay_scale(),
            recording_overlay_offset_x: 0,
            recording_overlay_offset_y: 0,
        }
    }
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_history_records: default_max_history_records(),
            max_storage_bytes: default_max_storage_bytes(),
        }
    }
}

fn default_prompt_template() -> String {
    "纠错任务：\n请把原始 ASR 转写整理成可直接粘贴的文本。\n\n用户要求：\n{{user_requirements}}\n\n用户词典：\n{{dictionary}}\n\n易错词纠正：\n{{correction_rules}}\n\n原始转写文本：\n```text\n{{raw_text}}\n```\n\n请根据以上信息纠错。不要新增原文没有的信息，只输出最终文本。\n\n额外约束：\n- 用户词典只用于理解专有名词、产品名、人名、项目名和固定写法；不要把语义正确且上下文合理的词强行替换为词典词。\n- 易错词纠正是明确的错听替换规则，只有当原始转写出现规则左侧内容或高度近似误识别时，才替换为右侧内容。\n- 如果原文中的英文词、产品名、技术名词或普通词语本身合理，且没有明显对应到词典别名或易错规则，不要因为存在相似条目而替换它。\n- 输出末尾不需要补句号；如果原文末尾没有句号，不要额外添加句号。".to_string()
}

fn previous_default_prompt_template() -> String {
    "用户要求：\n{{user_requirements}}\n\n用户词典：\n{{dictionary}}\n\n原始转写文本：\n```text\n{{raw_text}}\n```\n\n请根据用户要求和用户词典纠错。不要新增原文没有的信息，只输出最终文本。".to_string()
}

fn legacy_default_prompt_template() -> String {
    "用户要求：\n{{user_requirements}}\n\n用户词典：\n{{dictionary}}\n\n原始转写文本：\n{{raw_text}}\n\n请根据用户要求和用户词典纠错。不要新增原文没有的信息，只输出最终文本。".to_string()
}

fn default_stream_url() -> String {
    "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream".to_string()
}

fn default_submit_url() -> String {
    "https://openspeech.bytedance.com/api/v3/auc/bigmodel/submit".to_string()
}

fn default_query_url() -> String {
    "https://openspeech.bytedance.com/api/v3/auc/bigmodel/query".to_string()
}

pub struct ConfigStore;

impl ConfigStore {
    pub fn load() -> Result<AppConfig> {
        let path = paths::config_path()?;
        if !path.exists() {
            if let Some(config) = Self::migrate_legacy_config(&path)? {
                return Ok(config);
            }
            let config = AppConfig::default();
            Self::save(&config)?;
            return Ok(config);
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config {}", path.display()))?;
        let mut config: AppConfig = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse config {}", path.display()))?;
        config.normalize();
        Ok(config)
    }

    fn migrate_legacy_config(path: &std::path::Path) -> Result<Option<AppConfig>> {
        let legacy_paths = [
            paths::legacy_hidden_config_path()?,
            paths::legacy_config_path()?,
        ];
        let Some(legacy_path) = legacy_paths.into_iter().find(|path| path.exists()) else {
            return Ok(None);
        };

        let raw = fs::read_to_string(&legacy_path)
            .with_context(|| format!("Failed to read legacy config {}", legacy_path.display()))?;
        let mut config: AppConfig = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse legacy config {}", legacy_path.display()))?;
        config.normalize();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(&config)?;
        fs::write(path, raw).with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(Some(config))
    }

    pub fn save(config: &AppConfig) -> Result<AppConfig> {
        let path = paths::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let mut config = config.clone();
        config.normalize();
        let raw = serde_json::to_string_pretty(&config)?;
        fs::write(&path, raw).with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_mvp_providers() {
        let config = AppConfig::default();
        assert_eq!(config.hotkey, "PageUp");
        assert_eq!(config.hotkeys, vec!["PageUp".to_string(), String::new()]);
        assert_eq!(config.hotkey_enabled, vec![true, false]);
        assert_eq!(config.asr.provider, "volcengine");
        assert_eq!(config.llm.endpoint, "https://api.openai.com/v1");
        assert_eq!(config.llm.api_format, "responses");
        assert!(config.asr.app_key.is_empty());
        assert!(config.asr.access_key.is_empty());
        assert!(config.llm.api_key.is_empty());
        assert!(!config.llm.race_enabled);
        assert!(config.llm.race_models.is_empty());
        assert!(config.correction.enabled);
        assert!(config.correction.user_requirements.is_empty());
        assert!(config.correction.dictionary.is_empty());
        assert!(config.correction.correction_rules.is_empty());
    }

    #[test]
    fn legacy_hotkey_is_promoted_to_first_hotkey_slot() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("hotkeys");
        value.as_object_mut().unwrap().remove("hotkey_enabled");
        value["hotkey"] = serde_json::json!("F8");

        let mut config: AppConfig = serde_json::from_value(value).unwrap();
        config.normalize_hotkeys();

        assert_eq!(config.hotkey, "F8");
        assert_eq!(config.hotkeys, vec!["F8".to_string(), String::new()]);
        assert_eq!(config.hotkey_enabled, vec![true, false]);
        assert_eq!(config.active_hotkeys(), vec!["F8".to_string()]);
    }

    #[test]
    fn legacy_prompt_template_is_promoted_to_current_default() {
        let mut config = AppConfig::default();
        config.correction.prompt_template = legacy_default_prompt_template();

        config.normalize();

        assert_eq!(config.correction.prompt_template, default_prompt_template());
        assert!(config
            .correction
            .prompt_template
            .contains("```text\n{{raw_text}}\n```"));
        assert!(config
            .correction
            .prompt_template
            .contains("不要把语义正确且上下文合理的词强行替换为词典词"));
        assert!(config
            .correction
            .prompt_template
            .contains("{{correction_rules}}"));
    }

    #[test]
    fn previous_default_prompt_template_is_promoted_to_current_default() {
        let mut config = AppConfig::default();
        config.correction.prompt_template = previous_default_prompt_template();

        config.normalize();

        assert_eq!(config.correction.prompt_template, default_prompt_template());
    }

    #[test]
    fn missing_prompt_variables_default_to_empty_list() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["correction"]
            .as_object_mut()
            .unwrap()
            .remove("variables");

        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert!(config.correction.variables.is_empty());
    }

    #[test]
    fn missing_ui_config_defaults_overlay_scale() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("ui");
        value.as_object_mut().unwrap().remove("retention");
        value.as_object_mut().unwrap().remove("system");

        let mut config: AppConfig = serde_json::from_value(value).unwrap();
        config.normalize();

        assert_eq!(
            config.ui.recording_overlay_scale,
            default_recording_overlay_scale()
        );
        assert_eq!(config.ui.app_language, "zh-CN");
        assert_eq!(config.ui.recording_overlay_offset_x, 0);
        assert_eq!(config.ui.recording_overlay_offset_y, 0);
        assert_eq!(
            config.retention.max_history_records,
            default_max_history_records()
        );
        assert_eq!(
            config.retention.max_storage_bytes,
            default_max_storage_bytes()
        );
        assert!(!config.system.launch_at_login);
        assert!(!config.system.hide_dock_icon);
    }

    #[test]
    fn llm_race_models_are_normalized() {
        let mut config = AppConfig::default();
        config.llm.race_models = vec![
            " gpt-5.4-mini ".to_string(),
            "gpt-5.4-mini".to_string(),
            String::new(),
            "gpt-5.4".to_string(),
        ];

        config.normalize();

        assert_eq!(
            config.llm.race_models,
            vec!["gpt-5.4-mini".to_string(), "gpt-5.4".to_string()]
        );
    }

    #[test]
    fn llm_race_targets_and_removed_volc_models_are_normalized() {
        let mut config = AppConfig::default();
        config.llm.model_presets = vec![
            ModelPreset {
                provider: "volc_ark".to_string(),
                model: "doubao-seed-2-0-pro-260428".to_string(),
            },
            ModelPreset {
                provider: "volc_ark".to_string(),
                model: "doubao-seed-2-0-mini-260428".to_string(),
            },
        ];
        config.llm.race_targets = vec![
            RaceModelTarget {
                provider: "volc_ark".to_string(),
                model: "doubao-seed-1-6-251015".to_string(),
            },
            RaceModelTarget {
                provider: "volc_ark".to_string(),
                model: "doubao-seed-2-0-mini-260428".to_string(),
            },
            RaceModelTarget {
                provider: "volc_ark".to_string(),
                model: "doubao-seed-2-0-mini-260428".to_string(),
            },
        ];

        config.normalize();

        assert_eq!(
            config.llm.model_presets,
            vec![ModelPreset {
                provider: "volc_ark".to_string(),
                model: "doubao-seed-2-0-mini-260428".to_string(),
            }]
        );
        assert_eq!(
            config.llm.race_targets,
            vec![RaceModelTarget {
                provider: "volc_ark".to_string(),
                model: "doubao-seed-2-0-mini-260428".to_string(),
            }]
        );
    }

    #[test]
    fn overlay_scale_is_clamped() {
        let mut config = AppConfig::default();
        config.ui.recording_overlay_scale = 9.0;
        config.ui.recording_overlay_offset_x = 9000;
        config.ui.recording_overlay_offset_y = -9000;
        config.ui.app_language = "en".to_string();

        config.normalize();

        assert_eq!(config.ui.recording_overlay_scale, 1.0);
        assert_eq!(config.ui.recording_overlay_offset_x, 4000);
        assert_eq!(config.ui.recording_overlay_offset_y, -4000);
        assert_eq!(config.ui.app_language, "en-US");

        config.ui.recording_overlay_scale = 0.1;

        config.normalize();

        assert_eq!(config.ui.recording_overlay_scale, 0.25);
    }

    #[test]
    fn dictionary_arrow_lines_are_migrated_to_correction_rules() {
        let mut config = AppConfig::default();
        config.correction.dictionary = vec![
            DictionaryEntry {
                term: "\"艾迪\" -> \"ID\"".to_string(),
                aliases: Vec::new(),
                note: "英文缩写".to_string(),
            },
            DictionaryEntry {
                term: "BoltScribe".to_string(),
                aliases: Vec::new(),
                note: String::new(),
            },
        ];

        config.normalize();

        assert_eq!(config.correction.dictionary.len(), 1);
        assert_eq!(config.correction.dictionary[0].term, "BoltScribe");
        assert_eq!(
            config.correction.correction_rules,
            vec![CorrectionRule {
                source: "艾迪".to_string(),
                target: "ID".to_string(),
                note: "英文缩写".to_string(),
            }]
        );
    }

    #[test]
    fn retention_limits_are_clamped() {
        let mut config = AppConfig::default();
        config.retention.max_history_records = 10_000;
        config.retention.max_storage_bytes = 9 * 1024 * 1024 * 1024;

        config.normalize();

        assert_eq!(
            config.retention.max_history_records,
            default_max_history_records()
        );
        assert_eq!(
            config.retention.max_storage_bytes,
            default_max_storage_bytes()
        );

        config.retention.max_history_records = 0;
        config.retention.max_storage_bytes = 0;
        config.normalize();

        assert_eq!(config.retention.max_history_records, 1);
        assert_eq!(config.retention.max_storage_bytes, 1);
    }

    #[test]
    fn hotkeys_are_limited_to_two_slots() {
        let mut config = AppConfig {
            hotkeys: vec![
                "PageUp".to_string(),
                "CmdOrCtrl+Shift+Space".to_string(),
                "F8".to_string(),
            ],
            hotkey_enabled: Vec::new(),
            ..Default::default()
        };
        config.normalize_hotkeys();

        assert_eq!(
            config.hotkeys,
            vec!["PageUp".to_string(), "CmdOrCtrl+Shift+Space".to_string()]
        );
        assert_eq!(config.hotkey_enabled, vec![true, true]);
        assert_eq!(config.hotkey, "PageUp");
    }

    #[test]
    fn disabled_hotkeys_are_not_active() {
        let mut config = AppConfig {
            hotkeys: vec!["PageUp".to_string(), "CmdOrCtrl+Shift+Space".to_string()],
            hotkey_enabled: vec![false, true],
            ..Default::default()
        };
        config.normalize_hotkeys();

        assert_eq!(config.hotkey, "CmdOrCtrl+Shift+Space");
        assert_eq!(config.hotkey_enabled, vec![false, true]);
        assert_eq!(
            config.active_hotkeys(),
            vec!["CmdOrCtrl+Shift+Space".to_string()]
        );

        config.hotkey_enabled = vec![false, false];
        config.normalize_hotkeys();

        assert!(config.hotkey.is_empty());
        assert!(config.active_hotkeys().is_empty());
    }
}
