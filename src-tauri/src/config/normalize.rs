use super::defaults::*;
use super::model::*;
use anyhow::Result;

#[derive(Debug, Clone, Copy)]
pub(super) struct CorrectionTextFieldPresence {
    pub(super) dictionary_text: bool,
    pub(super) correction_rules_text: bool,
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
        self.ui.theme = normalize_ui_theme(&self.ui.theme);
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
        self.voice_activity_detection.normalize();
    }
}

impl VoiceActivityDetectionConfig {
    pub fn normalize(&mut self) {
        self.noise_margin_db = self
            .noise_margin_db
            .clamp(VAD_MIN_NOISE_MARGIN_DB, VAD_MAX_NOISE_MARGIN_DB);
        self.confirmation_ms = self
            .confirmation_ms
            .clamp(VAD_MIN_CONFIRMATION_MS, VAD_MAX_CONFIRMATION_MS);
        self.confirmation_ms = ((self.confirmation_ms + VAD_CONFIRMATION_STEP_MS / 2)
            / VAD_CONFIRMATION_STEP_MS)
            * VAD_CONFIRMATION_STEP_MS;
        self.noise_window_ms = self
            .noise_window_ms
            .clamp(VAD_MIN_NOISE_WINDOW_MS, VAD_MAX_NOISE_WINDOW_MS);
        self.noise_window_ms = ((self.noise_window_ms + VAD_NOISE_WINDOW_STEP_MS / 2)
            / VAD_NOISE_WINDOW_STEP_MS)
            * VAD_NOISE_WINDOW_STEP_MS;
        self.initial_silence_timeout_secs = self.initial_silence_timeout_secs.clamp(5, 60);
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

impl SystemConfig {
    pub fn normalize(&mut self) {
        self.fn_long_press_duration_ms = self.fn_long_press_duration_ms.clamp(50, 5_000);
    }
}

impl RetentionConfig {
    pub fn normalize(&mut self) {
        self.max_history_records = self
            .max_history_records
            .clamp(1, default_max_history_records());
        self.max_storage_bytes = self.max_storage_bytes.clamp(1, default_max_storage_bytes());
    }
}

pub(super) fn normalize_model_presets(presets: &[ModelPreset]) -> Vec<ModelPreset> {
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

pub(super) fn normalize_provider_settings(
    settings: &[LlmProviderSettings],
) -> Vec<LlmProviderSettings> {
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

pub(super) fn normalize_race_targets(targets: &[RaceModelTarget]) -> Vec<RaceModelTarget> {
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

pub(super) fn normalize_app_language(language: &str) -> String {
    match language.trim() {
        "en" | "en-US" => "en-US".to_string(),
        _ => default_app_language(),
    }
}

pub(super) fn normalize_ui_theme(theme: &str) -> String {
    match theme.trim() {
        "light" => "light".to_string(),
        "dark" => "dark".to_string(),
        _ => default_ui_theme(),
    }
}

pub(super) fn normalize_asr_auth_mode(auth_mode: &str, app_key: &str) -> String {
    match auth_mode.trim() {
        "api_key" | "new_console" | "x_api_key" => "api_key".to_string(),
        "legacy" | "old_console" => "legacy".to_string(),
        "" if !app_key.trim().is_empty() => "legacy".to_string(),
        _ => default_asr_auth_mode(),
    }
}

pub(super) fn normalize_dictionary(dictionary: &[DictionaryEntry]) -> Vec<DictionaryEntry> {
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

pub(super) fn normalize_correction_rules(rules: &[CorrectionRule]) -> Vec<CorrectionRule> {
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

pub(super) fn migrate_dictionary_rules(
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

pub(super) fn parse_rule_line(value: &str) -> Option<(String, String)> {
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

pub(super) fn trim_rule_part(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim()
        .to_string()
}

pub(super) fn correction_text_field_presence(
    value: &serde_json::Value,
) -> CorrectionTextFieldPresence {
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

pub(super) fn asr_auth_mode_field_present(value: &serde_json::Value) -> bool {
    value
        .get("asr")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|asr| asr.contains_key("auth_mode"))
}

pub(super) fn fill_missing_correction_text_fields(
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

pub(super) fn legacy_dictionary_text_from_entries(dictionary: &[DictionaryEntry]) -> String {
    dictionary
        .iter()
        .filter_map(|entry| {
            let term = entry.term.trim();
            (!term.is_empty()).then(|| term.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn legacy_correction_rules_text_from_rules(rules: &[CorrectionRule]) -> String {
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

pub(super) fn is_removed_volc_model(model: &str) -> bool {
    matches!(
        model.trim(),
        "doubao-seed-2-0-pro-260428" | "doubao-seed-1-6-251015"
    )
}

pub(super) fn normalize_string_list(items: &[String]) -> Vec<String> {
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

pub(super) fn normalize_disabled_dictionary_terms(
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

pub(super) fn normalize_audio_input_device_refs(
    items: &[AudioInputDeviceRef],
) -> Vec<AudioInputDeviceRef> {
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

pub(super) fn normalized_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}
