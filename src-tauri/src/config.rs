use crate::paths;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const CONFIG_EXPORT_FORMAT: &str = "boltscribe.config";
const CONFIG_EXPORT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default)]
    pub hotkeys: Vec<String>,
    #[serde(default)]
    pub hotkey_enabled: Vec<bool>,
    #[serde(default)]
    pub audio: AudioConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioConfig {
    #[serde(default = "default_input_device_mode")]
    pub input_device_mode: String,
    #[serde(default)]
    pub input_device_id: Option<String>,
    #[serde(default)]
    pub input_device_name: Option<String>,
    #[serde(default)]
    pub input_device_priority: Vec<AudioInputDeviceRef>,
    #[serde(default)]
    pub input_device_blacklist: Vec<AudioInputDeviceRef>,
    #[serde(default)]
    pub output_volume_ducking: OutputVolumeDuckingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioInputDeviceRef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputVolumeDuckingConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mute_instead_of_reduce: bool,
    #[serde(default = "default_output_volume_ducking_reduction_percent")]
    pub reduction_percent: u32,
    #[serde(default)]
    pub device_name_whitelist: Vec<String>,
    #[serde(default)]
    pub sound_source_hotkey_fallback_enabled: bool,
    #[serde(default = "default_sound_source_toggle_mute_hotkey")]
    pub sound_source_toggle_mute_hotkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AsrConfig {
    pub provider: String,
    #[serde(default)]
    pub auth_mode: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemConfig {
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub hide_dock_icon: bool,
    #[serde(default = "default_tray_left_click_recording_enabled")]
    pub tray_left_click_recording_enabled: bool,
    #[serde(default = "default_fn_long_press_enabled")]
    pub fn_long_press_enabled: bool,
    #[serde(default = "default_fn_long_press_duration_ms")]
    pub fn_long_press_duration_ms: u64,
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
    pub dictionary_text: String,
    #[serde(default)]
    pub disabled_dictionary_terms: Vec<String>,
    #[serde(default)]
    pub correction_rules_text: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ConfigImportReport {
    pub format: Option<String>,
    pub version: Option<u32>,
    pub missing_fields: Vec<String>,
    pub unknown_fields: Vec<String>,
    pub invalid_fields: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigImportResult {
    pub config: AppConfig,
    pub report: ConfigImportReport,
}

#[derive(Debug, Clone, Copy)]
struct CorrectionTextFieldPresence {
    dictionary_text: bool,
    correction_rules_text: bool,
}

#[derive(Debug, Serialize)]
struct ConfigExportEnvelope {
    format: String,
    version: u32,
    exported_at: String,
    config: AppConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            hotkeys: default_hotkey_slots(),
            hotkey_enabled: default_hotkey_enabled_slots(),
            audio: AudioConfig::default(),
            asr: AsrConfig {
                provider: "volcengine".to_string(),
                auth_mode: default_asr_auth_mode(),
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
                dictionary_text: String::new(),
                disabled_dictionary_terms: Vec::new(),
                correction_rules_text: String::new(),
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
        self.asr.normalize();
        self.audio.normalize();
        self.ui.recording_overlay_scale = self.ui.recording_overlay_scale.clamp(0.25, 1.0);
        self.ui.recording_overlay_offset_x = self.ui.recording_overlay_offset_x.clamp(-4000, 4000);
        self.ui.recording_overlay_offset_y = self.ui.recording_overlay_offset_y.clamp(-4000, 4000);
        self.retention.normalize();
        self.system.normalize();
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
        self.correction.disabled_dictionary_terms = normalize_disabled_dictionary_terms(
            &self.correction.dictionary_text,
            &self.correction.disabled_dictionary_terms,
        );
        let (dictionary, migrated_rules) = migrate_dictionary_rules(&self.correction.dictionary);
        self.correction.dictionary = normalize_dictionary(&dictionary);
        self.correction.correction_rules.extend(migrated_rules);
        self.correction.correction_rules =
            normalize_correction_rules(&self.correction.correction_rules);
    }

    pub fn validate(&self) -> Result<()> {
        self.audio.output_volume_ducking.validate()
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

impl AsrConfig {
    pub fn normalize(&mut self) {
        self.auth_mode = normalize_asr_auth_mode(&self.auth_mode, &self.app_key);
    }
}

impl AudioConfig {
    pub fn normalize(&mut self) {
        if self.input_device_mode == "manual" {
            self.input_device_id = normalized_optional_string(self.input_device_id.as_deref());
            self.input_device_name = normalized_optional_string(self.input_device_name.as_deref());
        } else {
            self.input_device_id = None;
            self.input_device_name = None;
        }

        self.input_device_priority = normalize_audio_input_device_refs(&self.input_device_priority);
        if self.input_device_priority.is_empty() && self.input_device_mode == "manual" {
            let legacy = AudioInputDeviceRef {
                id: self.input_device_id.clone().unwrap_or_default(),
                name: self.input_device_name.clone().unwrap_or_default(),
            };
            if !legacy.id.is_empty() || !legacy.name.is_empty() {
                self.input_device_priority.push(legacy);
            }
        }
        self.input_device_blacklist =
            normalize_audio_input_device_refs(&self.input_device_blacklist);

        if let Some(preferred) = self.input_device_priority.first() {
            self.input_device_mode = "manual".to_string();
            self.input_device_id = normalized_optional_string(Some(preferred.id.as_str()));
            self.input_device_name = normalized_optional_string(Some(preferred.name.as_str()));
        } else {
            self.input_device_mode = default_input_device_mode();
            self.input_device_id = None;
            self.input_device_name = None;
        }
        self.output_volume_ducking.normalize();
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device_mode: default_input_device_mode(),
            input_device_id: None,
            input_device_name: None,
            input_device_priority: Vec::new(),
            input_device_blacklist: Vec::new(),
            output_volume_ducking: OutputVolumeDuckingConfig::default(),
        }
    }
}

impl AudioInputDeviceRef {
    pub fn matches_device(&self, id: &str, name: &str) -> bool {
        (!self.id.is_empty() && self.id == id)
            || (!self.name.is_empty() && self.name.eq_ignore_ascii_case(name))
    }

    fn matches_ref(&self, other: &Self) -> bool {
        self.matches_device(&other.id, &other.name)
    }
}

impl OutputVolumeDuckingConfig {
    pub fn normalize(&mut self) {
        self.reduction_percent = self.reduction_percent.clamp(0, 100);
        self.device_name_whitelist = normalize_string_list(&self.device_name_whitelist);
        self.sound_source_toggle_mute_hotkey =
            self.sound_source_toggle_mute_hotkey.trim().to_string();
    }

    pub fn validate(&self) -> Result<()> {
        if self.sound_source_hotkey_fallback_enabled {
            crate::keyboard_shortcut::parse(&self.sound_source_toggle_mute_hotkey)
                .map_err(|err| anyhow::anyhow!("Invalid SoundSource mute shortcut: {err}"))?;
        }
        Ok(())
    }
}

impl Default for OutputVolumeDuckingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mute_instead_of_reduce: false,
            reduction_percent: default_output_volume_ducking_reduction_percent(),
            device_name_whitelist: Vec::new(),
            sound_source_hotkey_fallback_enabled: false,
            sound_source_toggle_mute_hotkey: default_sound_source_toggle_mute_hotkey(),
        }
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            hide_dock_icon: false,
            tray_left_click_recording_enabled: default_tray_left_click_recording_enabled(),
            fn_long_press_enabled: default_fn_long_press_enabled(),
            fn_long_press_duration_ms: default_fn_long_press_duration_ms(),
        }
    }
}

impl SystemConfig {
    pub fn normalize(&mut self) {
        self.fn_long_press_duration_ms = self.fn_long_press_duration_ms.clamp(50, 5_000);
    }
}

fn default_input_device_mode() -> String {
    "system_default".to_string()
}

fn default_output_volume_ducking_reduction_percent() -> u32 {
    70
}

fn default_sound_source_toggle_mute_hotkey() -> String {
    "Cmd+Opt+Ctrl+A".to_string()
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

fn default_tray_left_click_recording_enabled() -> bool {
    true
}

fn default_fn_long_press_enabled() -> bool {
    false
}

fn default_fn_long_press_duration_ms() -> u64 {
    200
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

fn normalize_asr_auth_mode(auth_mode: &str, app_key: &str) -> String {
    match auth_mode.trim() {
        "api_key" | "new_console" | "x_api_key" => "api_key".to_string(),
        "legacy" | "old_console" => "legacy".to_string(),
        "" if !app_key.trim().is_empty() => "legacy".to_string(),
        _ => default_asr_auth_mode(),
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

fn correction_text_field_presence(value: &serde_json::Value) -> CorrectionTextFieldPresence {
    let correction = value
        .get("correction")
        .and_then(serde_json::Value::as_object);
    CorrectionTextFieldPresence {
        dictionary_text: correction
            .is_some_and(|correction| correction.contains_key("dictionary_text")),
        correction_rules_text: correction
            .is_some_and(|correction| correction.contains_key("correction_rules_text")),
    }
}

fn asr_auth_mode_field_present(value: &serde_json::Value) -> bool {
    value
        .get("asr")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|asr| asr.contains_key("auth_mode"))
}

fn fill_missing_correction_text_fields(
    config: &mut AppConfig,
    presence: CorrectionTextFieldPresence,
) {
    if !presence.dictionary_text {
        config.correction.dictionary_text =
            legacy_dictionary_text_from_entries(&config.correction.dictionary);
    }
    if !presence.correction_rules_text {
        config.correction.correction_rules_text =
            legacy_correction_rules_text_from_rules(&config.correction.correction_rules);
    }
    config.correction.disabled_dictionary_terms = normalize_disabled_dictionary_terms(
        &config.correction.dictionary_text,
        &config.correction.disabled_dictionary_terms,
    );
}

fn legacy_dictionary_text_from_entries(dictionary: &[DictionaryEntry]) -> String {
    dictionary
        .iter()
        .filter_map(|entry| {
            let term = entry.term.trim();
            (!term.is_empty()).then(|| term.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn legacy_correction_rules_text_from_rules(rules: &[CorrectionRule]) -> String {
    rules
        .iter()
        .filter_map(|rule| {
            let source = rule.source.trim();
            let target = rule.target.trim();
            if source.is_empty() || target.is_empty() {
                return None;
            }
            let line = if rule.note.trim().is_empty() {
                format!("\"{source}\" -> \"{target}\"")
            } else {
                format!("\"{source}\" -> \"{target}\"（{}）", rule.note.trim())
            };
            Some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
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

fn normalize_disabled_dictionary_terms(
    dictionary_text: &str,
    disabled_terms: &[String],
) -> Vec<String> {
    let dictionary_terms = dictionary_text
        .lines()
        .filter_map(|line| {
            let term = line.trim();
            (!term.is_empty()).then(|| (term.to_lowercase(), term.to_string()))
        })
        .collect::<Vec<_>>();
    let mut normalized = Vec::new();
    let mut seen_keys = Vec::new();
    for disabled_term in disabled_terms {
        let key = disabled_term.trim().to_lowercase();
        if key.is_empty() || seen_keys.contains(&key) {
            continue;
        }
        let Some((_, term)) = dictionary_terms
            .iter()
            .find(|(dictionary_key, _)| dictionary_key == &key)
        else {
            continue;
        };
        seen_keys.push(key);
        normalized.push(term.clone());
    }
    normalized
}

fn normalize_audio_input_device_refs(items: &[AudioInputDeviceRef]) -> Vec<AudioInputDeviceRef> {
    let mut normalized = Vec::new();
    for item in items {
        let value = AudioInputDeviceRef {
            id: item.id.trim().to_string(),
            name: item.name.trim().to_string(),
        };
        if (value.id.is_empty() && value.name.is_empty())
            || normalized
                .iter()
                .any(|existing: &AudioInputDeviceRef| existing.matches_ref(&value))
        {
            continue;
        }
        normalized.push(value);
    }
    normalized
}

fn normalized_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
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

fn default_asr_auth_mode() -> String {
    "api_key".to_string()
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
        let raw_value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse config {}", path.display()))?;
        let correction_text_presence = correction_text_field_presence(&raw_value);
        let asr_auth_mode_present = asr_auth_mode_field_present(&raw_value);
        let mut config: AppConfig = serde_json::from_value(raw_value)
            .with_context(|| format!("Failed to parse config {}", path.display()))?;
        if !asr_auth_mode_present {
            config.asr.auth_mode.clear();
        }
        config.normalize();
        fill_missing_correction_text_fields(&mut config, correction_text_presence);
        Ok(config)
    }

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
        let imported_config = extract_imported_config(value, &mut report)?;
        let correction_text_presence = correction_text_field_presence(&imported_config);
        let asr_auth_mode_present = asr_auth_mode_field_present(&imported_config);
        let default_config = serde_json::to_value(AppConfig::default())?;
        let schema = config_schema_value()?;

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
        config.normalize();
        fill_missing_correction_text_fields(&mut config, correction_text_presence);

        Ok(ConfigImportResult { config, report })
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
        let raw_value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse legacy config {}", legacy_path.display()))?;
        let correction_text_presence = correction_text_field_presence(&raw_value);
        let asr_auth_mode_present = asr_auth_mode_field_present(&raw_value);
        let mut config: AppConfig = serde_json::from_value(raw_value)
            .with_context(|| format!("Failed to parse legacy config {}", legacy_path.display()))?;
        if !asr_auth_mode_present {
            config.asr.auth_mode.clear();
        }
        config.normalize();
        fill_missing_correction_text_fields(&mut config, correction_text_presence);

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
        config.validate()?;
        let raw = serde_json::to_string_pretty(&config)?;
        fs::write(&path, raw).with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(config)
    }
}

fn extract_imported_config(
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

fn config_schema_value() -> Result<serde_json::Value> {
    let mut schema = serde_json::to_value(AppConfig::default())?;
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
            "provider": "",
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

fn set_schema_array(schema: &mut serde_json::Value, path: &[&str], item: serde_json::Value) {
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

fn collect_missing_fields(
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

fn collect_unknown_fields(
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

fn merge_imported_value(
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

fn value_matches_schema(value: &serde_json::Value, schema: &serde_json::Value, path: &str) -> bool {
    match (value, schema) {
        (serde_json::Value::Null, serde_json::Value::Null)
        | (serde_json::Value::Bool(_), serde_json::Value::Bool(_))
        | (serde_json::Value::Number(_), serde_json::Value::Number(_))
        | (serde_json::Value::String(_), serde_json::Value::String(_))
        | (serde_json::Value::Array(_), serde_json::Value::Array(_))
        | (serde_json::Value::Object(_), serde_json::Value::Object(_)) => true,
        (serde_json::Value::Number(_), serde_json::Value::Null)
        | (serde_json::Value::Null, serde_json::Value::Number(_)) => {
            path == "llm.max_output_tokens"
        }
        _ => false,
    }
}

fn join_field_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn report_path(path: &str) -> String {
    if path.is_empty() {
        "config".to_string()
    } else {
        path.to_string()
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
        assert_eq!(config.asr.auth_mode, "api_key");
        assert!(config.system.tray_left_click_recording_enabled);
        assert!(!config.system.fn_long_press_enabled);
        assert_eq!(config.system.fn_long_press_duration_ms, 200);
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
    fn missing_fn_long_press_enabled_defaults_to_disabled() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["system"]
            .as_object_mut()
            .unwrap()
            .remove("fn_long_press_enabled");

        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert!(!config.system.fn_long_press_enabled);
    }

    #[test]
    fn missing_tray_left_click_recording_defaults_to_enabled() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["system"]
            .as_object_mut()
            .unwrap()
            .remove("tray_left_click_recording_enabled");

        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert!(config.system.tray_left_click_recording_enabled);
    }

    #[test]
    fn missing_fn_long_press_duration_defaults_to_200_ms() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["system"]
            .as_object_mut()
            .unwrap()
            .remove("fn_long_press_duration_ms");

        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert_eq!(config.system.fn_long_press_duration_ms, 200);
    }

    #[test]
    fn fn_long_press_duration_is_clamped() {
        let mut config = AppConfig::default();
        config.system.fn_long_press_duration_ms = 0;
        config.normalize();
        assert_eq!(config.system.fn_long_press_duration_ms, 50);

        config.system.fn_long_press_duration_ms = 10_000;
        config.normalize();
        assert_eq!(config.system.fn_long_press_duration_ms, 5_000);
    }

    #[test]
    fn missing_asr_auth_mode_with_app_key_uses_legacy_console() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["asr"].as_object_mut().unwrap().remove("auth_mode");
        value["asr"]["app_key"] = serde_json::json!("legacy-app-id");
        value["asr"]["access_key"] = serde_json::json!("legacy-access-token");

        let mut config: AppConfig = serde_json::from_value(value).unwrap();
        config.normalize();

        assert_eq!(config.asr.auth_mode, "legacy");
    }

    #[test]
    fn explicit_api_key_auth_mode_survives_leftover_app_key() {
        let mut config = AppConfig::default();
        config.asr.auth_mode = "api_key".to_string();
        config.asr.app_key = "legacy-app-id".to_string();

        config.normalize();

        assert_eq!(config.asr.auth_mode, "api_key");
    }

    #[test]
    fn missing_asr_auth_mode_without_app_key_uses_new_console() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["asr"].as_object_mut().unwrap().remove("auth_mode");

        let mut config: AppConfig = serde_json::from_value(value).unwrap();
        config.normalize();

        assert_eq!(config.asr.auth_mode, "api_key");
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
    fn missing_disabled_dictionary_terms_defaults_to_empty_list() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["correction"]
            .as_object_mut()
            .unwrap()
            .remove("disabled_dictionary_terms");

        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert!(config.correction.disabled_dictionary_terms.is_empty());
    }

    #[test]
    fn disabled_dictionary_terms_are_trimmed_and_deduplicated() {
        let mut config = AppConfig::default();
        config.correction.dictionary_text = "Codex\nBoltScribe".to_string();
        config.correction.disabled_dictionary_terms = vec![
            " codex ".to_string(),
            "CODEX".to_string(),
            String::new(),
            "BoltScribe".to_string(),
            "Removed term".to_string(),
        ];

        config.normalize();

        assert_eq!(
            config.correction.disabled_dictionary_terms,
            vec!["Codex".to_string(), "BoltScribe".to_string()]
        );
    }

    #[test]
    fn import_reports_invalid_disabled_dictionary_term_items() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["correction"]["dictionary_text"] = serde_json::json!("Codex");
        value["correction"]["disabled_dictionary_terms"] = serde_json::json!(["Codex", 42]);

        let result = ConfigStore::import_json(&value.to_string()).unwrap();

        assert!(result
            .report
            .invalid_fields
            .contains(&"correction.disabled_dictionary_terms[1]".to_string()));
        assert_eq!(
            result.config.correction.disabled_dictionary_terms,
            vec!["Codex".to_string()]
        );
    }

    #[test]
    fn missing_ui_config_defaults_overlay_scale() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("audio");
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
        assert!(config.system.tray_left_click_recording_enabled);
        assert_eq!(config.audio.input_device_mode, "system_default");
        assert!(config.audio.input_device_priority.is_empty());
    }

    #[test]
    fn audio_config_defaults_to_system_input_device() {
        let mut config = AppConfig {
            audio: AudioConfig {
                input_device_mode: "manual".to_string(),
                input_device_id: Some(" ".to_string()),
                input_device_name: None,
                ..Default::default()
            },
            ..Default::default()
        };

        config.normalize();

        assert_eq!(config.audio.input_device_mode, "system_default");
        assert!(config.audio.input_device_id.is_none());
        assert!(config.audio.input_device_name.is_none());
    }

    #[test]
    fn legacy_manual_input_device_migrates_to_priority() {
        let mut config = AppConfig {
            audio: AudioConfig {
                input_device_mode: "manual".to_string(),
                input_device_id: Some(" legacy-id ".to_string()),
                input_device_name: Some(" Preferred Mic ".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        config.normalize();

        assert_eq!(
            config.audio.input_device_priority,
            vec![AudioInputDeviceRef {
                id: "legacy-id".to_string(),
                name: "Preferred Mic".to_string(),
            }]
        );
        assert_eq!(config.audio.input_device_mode, "manual");
        assert_eq!(config.audio.input_device_id.as_deref(), Some("legacy-id"));
        assert_eq!(
            config.audio.input_device_name.as_deref(),
            Some("Preferred Mic")
        );
    }

    #[test]
    fn audio_device_policy_normalizes_without_losing_blacklisted_priority() {
        let mut config = AppConfig::default();
        config.audio.input_device_priority = vec![
            AudioInputDeviceRef {
                id: " mic-1 ".to_string(),
                name: " Main Mic ".to_string(),
            },
            AudioInputDeviceRef {
                id: "mic-1".to_string(),
                name: "Duplicate Name".to_string(),
            },
            AudioInputDeviceRef {
                id: String::new(),
                name: String::new(),
            },
        ];
        config.audio.input_device_blacklist = vec![AudioInputDeviceRef {
            id: " mic-1 ".to_string(),
            name: " Main Mic ".to_string(),
        }];

        config.normalize();

        assert_eq!(config.audio.input_device_priority.len(), 1);
        assert_eq!(config.audio.input_device_priority[0].id, "mic-1");
        assert_eq!(config.audio.input_device_priority[0].name, "Main Mic");
        assert_eq!(config.audio.input_device_blacklist.len(), 1);
        assert_eq!(config.audio.input_device_blacklist[0].id, "mic-1");
    }

    #[test]
    fn missing_audio_device_policy_fields_default_to_empty_lists() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        let audio = value["audio"].as_object_mut().unwrap();
        audio.remove("input_device_priority");
        audio.remove("input_device_blacklist");

        let config: AppConfig = serde_json::from_value(value).unwrap();

        assert!(config.audio.input_device_priority.is_empty());
        assert!(config.audio.input_device_blacklist.is_empty());
    }

    #[test]
    fn config_import_preserves_audio_device_policy_objects() {
        let mut config_value = serde_json::to_value(AppConfig::default()).unwrap();
        config_value["audio"]["input_device_priority"] = serde_json::json!([
            { "id": "preferred", "name": "Preferred Mic" }
        ]);
        config_value["audio"]["input_device_blacklist"] = serde_json::json!([
            { "id": "blocked", "name": "Capture Card" }
        ]);
        let raw = serde_json::json!({
            "format": CONFIG_EXPORT_FORMAT,
            "version": CONFIG_EXPORT_VERSION,
            "config": config_value
        })
        .to_string();

        let result = ConfigStore::import_json(&raw).unwrap();

        assert_eq!(result.config.audio.input_device_priority[0].id, "preferred");
        assert_eq!(
            result.config.audio.input_device_blacklist[0].name,
            "Capture Card"
        );
        assert!(result.report.invalid_fields.is_empty());
    }

    #[test]
    fn output_volume_ducking_defaults_and_normalizes() {
        let mut config = AppConfig::default();
        config.audio.output_volume_ducking.enabled = true;
        config.audio.output_volume_ducking.reduction_percent = 150;
        config.audio.output_volume_ducking.device_name_whitelist = vec![
            " External Speaker ".to_string(),
            "External Speaker".to_string(),
            String::new(),
            "Display Audio".to_string(),
        ];
        config
            .audio
            .output_volume_ducking
            .sound_source_toggle_mute_hotkey = " Cmd+Opt+Ctrl+A ".to_string();

        config.normalize();

        assert!(config.audio.output_volume_ducking.enabled);
        assert!(!config.audio.output_volume_ducking.mute_instead_of_reduce);
        assert_eq!(config.audio.output_volume_ducking.reduction_percent, 100);
        assert_eq!(
            config.audio.output_volume_ducking.device_name_whitelist,
            vec!["External Speaker".to_string(), "Display Audio".to_string()]
        );
        assert!(
            !config
                .audio
                .output_volume_ducking
                .sound_source_hotkey_fallback_enabled
        );
        assert_eq!(
            config
                .audio
                .output_volume_ducking
                .sound_source_toggle_mute_hotkey,
            "Cmd+Opt+Ctrl+A"
        );

        let default_ducking = OutputVolumeDuckingConfig::default();
        assert!(!default_ducking.enabled);
        assert!(!default_ducking.mute_instead_of_reduce);
        assert_eq!(default_ducking.reduction_percent, 70);
        assert!(default_ducking.device_name_whitelist.is_empty());
        assert!(!default_ducking.sound_source_hotkey_fallback_enabled);
        assert_eq!(
            default_ducking.sound_source_toggle_mute_hotkey,
            "Cmd+Opt+Ctrl+A"
        );
    }

    #[test]
    fn sound_source_hotkey_is_validated_only_when_enabled() {
        let mut config = AppConfig::default();
        config
            .audio
            .output_volume_ducking
            .sound_source_toggle_mute_hotkey = "Cmd+Unknown".to_string();
        config.normalize();
        assert!(config.validate().is_ok());

        config
            .audio
            .output_volume_ducking
            .sound_source_hotkey_fallback_enabled = true;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("Invalid SoundSource mute shortcut"));
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
    fn missing_correction_text_fields_are_migrated_from_legacy_arrays() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        let correction = value["correction"].as_object_mut().unwrap();
        correction.remove("dictionary_text");
        correction.remove("correction_rules_text");
        correction.insert(
            "dictionary".to_string(),
            serde_json::json!([
                {
                    "term": "BoltScribe",
                    "aliases": [],
                    "note": ""
                }
            ]),
        );
        correction.insert(
            "correction_rules".to_string(),
            serde_json::json!([
                {
                    "source": "DocX",
                    "target": "docx",
                    "note": "文件格式"
                }
            ]),
        );

        let result = ConfigStore::import_json(&value.to_string()).unwrap();

        assert_eq!(result.config.correction.dictionary_text, "BoltScribe");
        assert_eq!(
            result.config.correction.correction_rules_text,
            "\"DocX\" -> \"docx\"（文件格式）"
        );
    }

    #[test]
    fn present_empty_correction_text_fields_are_not_backfilled() {
        let mut value = serde_json::to_value(AppConfig::default()).unwrap();
        value["correction"]["dictionary_text"] = serde_json::json!("");
        value["correction"]["correction_rules_text"] = serde_json::json!("");
        value["correction"]["dictionary"] = serde_json::json!([
            {
                "term": "BoltScribe",
                "aliases": [],
                "note": ""
            }
        ]);
        value["correction"]["correction_rules"] = serde_json::json!([
            {
                "source": "DocX",
                "target": "docx",
                "note": ""
            }
        ]);

        let result = ConfigStore::import_json(&value.to_string()).unwrap();

        assert!(result.config.correction.dictionary_text.is_empty());
        assert!(result.config.correction.correction_rules_text.is_empty());
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
    fn config_export_uses_versioned_envelope() {
        let raw = ConfigStore::export_json(&AppConfig::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(value["format"], CONFIG_EXPORT_FORMAT);
        assert_eq!(value["version"], CONFIG_EXPORT_VERSION);
        assert!(value["exported_at"].as_str().is_some());
        assert_eq!(value["config"]["hotkey"], "PageUp");
    }

    #[test]
    fn config_import_fills_missing_fields_and_reports_unknown_fields() {
        let mut config_value = serde_json::to_value(AppConfig::default()).unwrap();
        config_value.as_object_mut().unwrap().remove("hotkeys");
        config_value["llm"]
            .as_object_mut()
            .unwrap()
            .remove("thinking_effort");
        config_value["audio"]
            .as_object_mut()
            .unwrap()
            .remove("output_volume_ducking");
        config_value["llm"]["provider_settings"] = serde_json::json!([
            {
                "provider": "custom",
                "endpoint": "https://example.test/v1",
                "api_format": "responses",
                "api_key": "secret",
                "future_field": true
            }
        ]);
        config_value["future_top_level"] = serde_json::json!(true);

        let raw = serde_json::json!({
            "format": CONFIG_EXPORT_FORMAT,
            "version": CONFIG_EXPORT_VERSION,
            "exported_at": "2026-05-16T00:00:00Z",
            "config": config_value,
            "future_envelope_field": "ignored"
        })
        .to_string();

        let result = ConfigStore::import_json(&raw).unwrap();

        assert!(result
            .report
            .missing_fields
            .contains(&"hotkeys".to_string()));
        assert!(result
            .report
            .missing_fields
            .contains(&"llm.thinking_effort".to_string()));
        assert!(result
            .report
            .missing_fields
            .contains(&"audio.output_volume_ducking".to_string()));
        assert!(result
            .report
            .unknown_fields
            .contains(&"future_top_level".to_string()));
        assert!(result
            .report
            .unknown_fields
            .contains(&"llm.provider_settings[0].future_field".to_string()));
        assert!(result
            .report
            .unknown_fields
            .contains(&"envelope.future_envelope_field".to_string()));
        assert_eq!(result.config.hotkeys, default_hotkey_slots());
        assert_eq!(result.config.llm.thinking_effort, default_thinking_effort());
        assert_eq!(
            result.config.audio.output_volume_ducking,
            OutputVolumeDuckingConfig::default()
        );
        assert_eq!(result.config.llm.provider_settings.len(), 1);
        assert_eq!(result.config.llm.provider_settings[0].provider, "custom");
    }

    #[test]
    fn config_import_infers_legacy_asr_auth_mode_for_old_exports() {
        let mut config_value = serde_json::to_value(AppConfig::default()).unwrap();
        config_value["asr"]
            .as_object_mut()
            .unwrap()
            .remove("auth_mode");
        config_value["asr"]["app_key"] = serde_json::json!("legacy-app-id");
        config_value["asr"]["access_key"] = serde_json::json!("legacy-access-token");
        let raw = serde_json::json!({
            "format": CONFIG_EXPORT_FORMAT,
            "version": CONFIG_EXPORT_VERSION,
            "exported_at": "2026-05-16T00:00:00Z",
            "config": config_value
        })
        .to_string();

        let result = ConfigStore::import_json(&raw).unwrap();

        assert_eq!(result.config.asr.auth_mode, "legacy");
    }

    #[test]
    fn config_import_fills_missing_soundsource_ducking_fields() {
        let mut config_value = serde_json::to_value(AppConfig::default()).unwrap();
        config_value["audio"]["output_volume_ducking"]
            .as_object_mut()
            .unwrap()
            .remove("mute_instead_of_reduce");
        config_value["audio"]["output_volume_ducking"]
            .as_object_mut()
            .unwrap()
            .remove("sound_source_hotkey_fallback_enabled");
        config_value["audio"]["output_volume_ducking"]
            .as_object_mut()
            .unwrap()
            .remove("sound_source_toggle_mute_hotkey");

        let raw = serde_json::json!({
            "format": CONFIG_EXPORT_FORMAT,
            "version": CONFIG_EXPORT_VERSION,
            "exported_at": "2026-05-20T00:00:00Z",
            "config": config_value
        })
        .to_string();

        let result = ConfigStore::import_json(&raw).unwrap();

        assert!(result
            .report
            .missing_fields
            .contains(&"audio.output_volume_ducking.mute_instead_of_reduce".to_string()));
        assert!(result.report.missing_fields.contains(
            &"audio.output_volume_ducking.sound_source_hotkey_fallback_enabled".to_string()
        ));
        assert!(result
            .report
            .missing_fields
            .contains(&"audio.output_volume_ducking.sound_source_toggle_mute_hotkey".to_string()));
        assert!(
            !result
                .config
                .audio
                .output_volume_ducking
                .mute_instead_of_reduce
        );
        assert!(
            !result
                .config
                .audio
                .output_volume_ducking
                .sound_source_hotkey_fallback_enabled
        );
        assert_eq!(
            result
                .config
                .audio
                .output_volume_ducking
                .sound_source_toggle_mute_hotkey,
            "Cmd+Opt+Ctrl+A"
        );
    }

    #[test]
    fn config_import_accepts_legacy_plain_config_json() {
        let raw = serde_json::to_string(&AppConfig::default()).unwrap();

        let result = ConfigStore::import_json(&raw).unwrap();

        assert_eq!(result.config.hotkey, "PageUp");
        assert!(result.report.format.is_none());
        assert!(result
            .report
            .notes
            .iter()
            .any(|note| note.contains("legacy plain config")));
    }

    #[test]
    fn config_import_reports_newer_version() {
        let raw = serde_json::json!({
            "format": CONFIG_EXPORT_FORMAT,
            "version": CONFIG_EXPORT_VERSION + 1,
            "config": AppConfig::default()
        })
        .to_string();

        let result = ConfigStore::import_json(&raw).unwrap();

        assert_eq!(result.report.version, Some(CONFIG_EXPORT_VERSION + 1));
        assert!(result
            .report
            .notes
            .iter()
            .any(|note| note.contains("newer than supported")));
    }

    #[test]
    fn config_import_reports_invalid_field_types_and_keeps_defaults() {
        let mut config_value = serde_json::to_value(AppConfig::default()).unwrap();
        config_value["hotkeys"] = serde_json::json!(["PageUp", 42]);
        config_value["retention"]["max_history_records"] = serde_json::json!("many");
        config_value["llm"]["max_output_tokens"] = serde_json::json!(4096);
        config_value["correction"]["dictionary"] = serde_json::json!([
            {
                "term": "BoltScribe",
                "aliases": ["bolt", true],
                "note": ""
            }
        ]);

        let raw = serde_json::json!({
            "format": CONFIG_EXPORT_FORMAT,
            "version": CONFIG_EXPORT_VERSION,
            "config": config_value
        })
        .to_string();

        let result = ConfigStore::import_json(&raw).unwrap();

        assert!(result
            .report
            .invalid_fields
            .contains(&"hotkeys[1]".to_string()));
        assert!(result
            .report
            .invalid_fields
            .contains(&"retention.max_history_records".to_string()));
        assert!(result
            .report
            .invalid_fields
            .contains(&"correction.dictionary[0].aliases[1]".to_string()));
        assert_eq!(
            result.config.hotkeys,
            vec!["PageUp".to_string(), String::new()]
        );
        assert_eq!(
            result.config.retention.max_history_records,
            default_max_history_records()
        );
        assert_eq!(result.config.llm.max_output_tokens, Some(4096));
        assert_eq!(
            result.config.correction.dictionary[0].aliases,
            vec!["bolt".to_string()]
        );
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
