use super::defaults::{
    default_hotkey_slots, default_max_history_records, default_max_storage_bytes,
    default_prompt_template, default_recording_overlay_scale,
    default_recording_overlay_snap_range_percent, default_thinking_effort,
    legacy_default_prompt_template, previous_default_prompt_template,
};
use super::import::{CONFIG_EXPORT_FORMAT, CONFIG_EXPORT_VERSION};
use super::*;

#[test]
fn legacy_config_leaves_learning_disabled_and_migrates_shared_credentials() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value.as_object_mut().unwrap().remove("learning");
    value.as_object_mut().unwrap().remove("model_services");
    value["llm"]["endpoint"] = serde_json::json!("https://example.test/v1");
    value["llm"]["api_key"] = serde_json::json!("live-only-placeholder");
    value["llm"]["model"] = serde_json::json!("latency-sensitive-model");
    let mut loaded: AppConfig = serde_json::from_value(value.clone()).unwrap();
    loaded.migrate_legacy_model_services();
    assert!(!loaded.learning.enabled);
    assert!(!loaded.learning.can_observe());
    assert!(loaded.validate().is_ok());
    assert!(loaded.learning.llm.api_key.is_empty());
    assert!(loaded.learning.llm.model.is_empty());
    let imported = ConfigStore::import_json(&value.to_string()).unwrap();
    assert_eq!(imported.config.learning, loaded.learning);
    assert_eq!(
        imported.config.resolve_llm(&imported.config.llm).api_key,
        "live-only-placeholder"
    );
    assert_eq!(
        imported.config.learning.llm.service_id,
        imported.config.llm.service_id
    );
    assert!(imported.config.llm.api_key.is_empty());
}

#[test]
fn independent_learning_model_survives_export_and_import() {
    let mut config = AppConfig::default();
    let original_live = config.llm.clone();
    config.learning.enabled = true;
    config.learning.llm.service_id = "custom".into();
    config.model_services[2].endpoint = "https://models.example.test/v1".into();
    config.model_services[2].api_key = "learning-only-placeholder".into();
    config.learning.llm.model = "quality-model".into();
    config.model_services[2].api_format = "responses".into();
    config.learning.llm.thinking_effort = "high".into();
    config.learning.llm.timeout_secs = 90;
    config.validate().unwrap();
    let imported = ConfigStore::import_json(&ConfigStore::export_json(&config).unwrap()).unwrap();
    assert_eq!(imported.config.learning, config.learning);
    assert_eq!(imported.config.llm, original_live);
    assert_eq!(imported.config.model_services, config.model_services);
    assert_eq!(
        imported.config.learning_model().llm.api_key,
        "learning-only-placeholder"
    );
    assert!(imported.report.unknown_fields.is_empty());
    assert!(imported.report.invalid_fields.is_empty());
}

#[test]
fn learning_requires_opt_in_and_a_configured_model() {
    let mut config = AppConfig::default();
    assert!(!config.learning.enabled);
    assert!(config.validate().is_ok());
    config.model_services[0].api_key = "shared-placeholder".into();
    config.learning.llm.model = "quality-model".into();
    assert!(!config.learning_model().can_observe());
    config.learning.enabled = true;
    assert!(config.learning_model().can_observe());

    let mut partial = serde_json::to_value(&config).unwrap();
    partial["learning"]
        .as_object_mut()
        .unwrap()
        .remove("enabled");
    let loaded: AppConfig = serde_json::from_value(partial.clone()).unwrap();
    assert!(!loaded.learning.enabled);
    let imported = ConfigStore::import_json(&partial.to_string()).unwrap();
    assert!(!imported.config.learning.enabled);

    config.learning.llm.model.clear();
    assert!(config.validate().is_ok());
    assert!(!config.learning_model().can_observe());
}

#[test]
fn explicit_learning_setting_survives_load_export_and_import() {
    for enabled in [false, true] {
        let mut config = AppConfig::default();
        config.learning.enabled = enabled;
        let loaded: AppConfig =
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
        assert_eq!(loaded.learning.enabled, enabled);
        let imported =
            ConfigStore::import_json(&ConfigStore::export_json(&config).unwrap()).unwrap();
        assert_eq!(imported.config.learning.enabled, enabled);
    }
}

#[test]
fn default_config_uses_mvp_providers() {
    let config = AppConfig::default();
    assert_eq!(config.hotkey, "PageUp");
    assert_eq!(config.hotkeys, vec!["PageUp".to_string(), String::new()]);
    assert_eq!(config.hotkey_enabled, vec![true, false]);
    assert_eq!(config.asr.provider, "volcengine");
    assert_eq!(config.asr.auth_mode, "api_key");
    assert!(config.system.tray_left_click_recording_enabled);
    assert!(config.system.show_menu_bar_icon);
    assert!(!config.system.fn_long_press_enabled);
    assert_eq!(config.system.fn_long_press_duration_ms, 200);
    assert_eq!(
        config.resolve_llm(&config.llm).endpoint,
        "https://api.openai.com/v1"
    );
    assert_eq!(config.resolve_llm(&config.llm).api_format, "responses");
    assert!(config.asr.app_key.is_empty());
    assert!(config.asr.access_key.is_empty());
    assert!(config.llm.api_key.is_empty());
    assert!(config.llm.fast_mode_enabled);
    assert!(!config.llm.race_enabled);
    assert!(config.llm.race_models.is_empty());
    assert!(config.correction.enabled);
    assert!(config.correction.user_requirements.is_empty());
    assert!(config.correction.dictionary.is_empty());
    assert!(config.correction.correction_rules.is_empty());
}

#[test]
fn legacy_llm_config_enables_fast_mode_on_load_and_import() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value["llm"]
        .as_object_mut()
        .unwrap()
        .remove("fast_mode_enabled");

    let mut loaded: AppConfig = serde_json::from_value(value.clone()).unwrap();
    loaded.normalize();
    assert!(loaded.llm.fast_mode_enabled);

    let imported = ConfigStore::import_json(&value.to_string()).unwrap();
    assert!(imported.config.llm.fast_mode_enabled);
    assert!(imported
        .report
        .missing_fields
        .contains(&"llm.fast_mode_enabled".to_string()));
}

#[test]
fn explicitly_disabled_fast_mode_survives_load_export_and_import() {
    let mut config = AppConfig::default();
    config.llm.fast_mode_enabled = false;

    let loaded: AppConfig = serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
    assert!(!loaded.llm.fast_mode_enabled);

    let imported = ConfigStore::import_json(&ConfigStore::export_json(&config).unwrap()).unwrap();
    assert!(!imported.config.llm.fast_mode_enabled);
    assert!(!imported
        .report
        .missing_fields
        .contains(&"llm.fast_mode_enabled".to_string()));
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
fn missing_menu_bar_icon_visibility_defaults_to_visible() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value["system"]
        .as_object_mut()
        .unwrap()
        .remove("show_menu_bar_icon");

    let config: AppConfig = serde_json::from_value(value).unwrap();

    assert!(config.system.show_menu_bar_icon);
}

#[test]
fn hidden_menu_bar_icon_survives_config_export_and_import() {
    let mut config = AppConfig::default();
    config.system.show_menu_bar_icon = false;

    let raw = ConfigStore::export_json(&config).unwrap();
    let imported = ConfigStore::import_json(&raw).unwrap();

    assert!(!imported.config.system.show_menu_bar_icon);
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
fn asr_connection_retry_defaults_when_missing() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value["asr"]
        .as_object_mut()
        .unwrap()
        .remove("connection_retry");

    let config: AppConfig = serde_json::from_value(value).unwrap();

    assert_eq!(
        config.asr.connection_retry,
        AsrConnectionRetryConfig::default()
    );
    assert_eq!(config.asr.connection_retry.live_post_stop_max_attempts, 3);
    assert_eq!(config.asr.connection_retry.file_max_attempts, 3);
    assert_eq!(config.asr.connection_retry.live_connect_timeout_ms, 3_000);
    assert_eq!(config.asr.connection_retry.connect_timeout_ms, 10_000);
    assert_eq!(config.asr.connection_retry.initial_delay_ms, 250);
    assert_eq!(config.asr.connection_retry.max_delay_ms, 5_000);
}

#[test]
fn asr_connection_retry_fields_default_individually_and_normalize() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value["asr"]["connection_retry"] = serde_json::json!({
        "live_post_stop_max_attempts": 8
    });
    let mut config: AppConfig = serde_json::from_value(value).unwrap();
    assert_eq!(config.asr.connection_retry.live_post_stop_max_attempts, 8);
    assert_eq!(config.asr.connection_retry.file_max_attempts, 3);
    assert_eq!(config.asr.connection_retry.live_connect_timeout_ms, 3_000);
    assert_eq!(config.asr.connection_retry.connect_timeout_ms, 10_000);

    config.asr.connection_retry.live_post_stop_max_attempts = 0;
    config.asr.connection_retry.file_max_attempts = 99;
    config.asr.connection_retry.live_connect_timeout_ms = 20;
    config.asr.connection_retry.connect_timeout_ms = 10;
    config.asr.connection_retry.initial_delay_ms = 99_000;
    config.asr.connection_retry.max_delay_ms = 1;
    config.normalize();

    assert_eq!(
        config.asr.connection_retry.live_post_stop_max_attempts,
        ASR_MIN_LIVE_POST_STOP_MAX_ATTEMPTS
    );
    assert_eq!(
        config.asr.connection_retry.file_max_attempts,
        ASR_MAX_FILE_MAX_ATTEMPTS
    );
    assert_eq!(
        config.asr.connection_retry.live_connect_timeout_ms,
        ASR_MIN_LIVE_CONNECT_TIMEOUT_MS
    );
    assert_eq!(
        config.asr.connection_retry.connect_timeout_ms,
        ASR_MIN_CONNECT_TIMEOUT_MS
    );
    assert_eq!(
        config.asr.connection_retry.initial_delay_ms,
        ASR_MAX_RETRY_INITIAL_DELAY_MS
    );
    assert_eq!(
        config.asr.connection_retry.max_delay_ms,
        ASR_MAX_RETRY_INITIAL_DELAY_MS
    );
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
    assert_eq!(config.ui.theme, "system");
    assert_eq!(
        config.ui.recording_overlay_snap_range_percent,
        default_recording_overlay_snap_range_percent()
    );
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
fn missing_overlay_snap_range_uses_the_larger_default() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value["ui"]
        .as_object_mut()
        .unwrap()
        .remove("recording_overlay_snap_range_percent");

    let mut config: AppConfig = serde_json::from_value(value).unwrap();
    config.normalize();

    assert_eq!(
        config.ui.recording_overlay_snap_range_percent,
        default_recording_overlay_snap_range_percent()
    );
}

#[test]
fn ui_theme_defaults_and_preserves_supported_values() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value["ui"].as_object_mut().unwrap().remove("theme");

    let mut config: AppConfig = serde_json::from_value(value).unwrap();
    config.normalize();
    assert_eq!(config.ui.theme, "system");

    for theme in ["light", "dark"] {
        config.ui.theme = theme.to_string();
        config.normalize();
        assert_eq!(config.ui.theme, theme);
    }
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
fn voice_activity_detection_defaults_and_normalizes() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value["audio"]
        .as_object_mut()
        .unwrap()
        .remove("voice_activity_detection");
    let mut config: AppConfig = serde_json::from_value(value).unwrap();
    assert!(!config.audio.voice_activity_detection.enabled);
    assert_eq!(config.audio.voice_activity_detection.noise_margin_db, 12);
    assert_eq!(config.audio.voice_activity_detection.confirmation_ms, 480);
    assert_eq!(config.audio.voice_activity_detection.noise_window_ms, 800);
    assert_eq!(
        config
            .audio
            .voice_activity_detection
            .initial_silence_timeout_secs,
        15
    );

    config.audio.voice_activity_detection.noise_margin_db = 99;
    config.audio.voice_activity_detection.confirmation_ms = 251;
    config.audio.voice_activity_detection.noise_window_ms = 3_999;
    config
        .audio
        .voice_activity_detection
        .initial_silence_timeout_secs = 1;
    config.normalize();
    assert_eq!(config.audio.voice_activity_detection.noise_margin_db, 40);
    assert_eq!(config.audio.voice_activity_detection.confirmation_ms, 260);
    assert_eq!(config.audio.voice_activity_detection.noise_window_ms, 3_000);
    assert_eq!(
        config
            .audio
            .voice_activity_detection
            .initial_silence_timeout_secs,
        5
    );

    let mut legacy_value = serde_json::to_value(AppConfig::default()).unwrap();
    legacy_value["audio"]["voice_activity_detection"] = serde_json::json!({
        "enabled": true,
        "sensitivity": 4,
        "initial_silence_timeout_secs": 15
    });
    let legacy: AppConfig = serde_json::from_value(legacy_value).unwrap();
    assert_eq!(legacy.audio.voice_activity_detection.noise_margin_db, 12);
    assert_eq!(legacy.audio.voice_activity_detection.confirmation_ms, 480);
    assert_eq!(legacy.audio.voice_activity_detection.noise_window_ms, 800);
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
            service_id: "volc_ark".to_string(),
            model: "doubao-seed-1-6-251015".to_string(),
        },
        RaceModelTarget {
            service_id: "volc_ark".to_string(),
            model: "doubao-seed-2-0-mini-260428".to_string(),
        },
        RaceModelTarget {
            service_id: "volc_ark".to_string(),
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
            service_id: "volc_ark".to_string(),
            model: "doubao-seed-2-0-mini-260428".to_string(),
        }]
    );
}

#[test]
fn overlay_appearance_settings_are_clamped() {
    let mut config = AppConfig::default();
    config.ui.recording_overlay_scale = 9.0;
    config.ui.recording_overlay_snap_range_percent = 900;
    config.ui.recording_overlay_offset_x = 9000;
    config.ui.recording_overlay_offset_y = -9000;
    config.ui.app_language = "en".to_string();
    config.ui.theme = "sepia".to_string();

    config.normalize();

    assert_eq!(config.ui.recording_overlay_scale, 1.0);
    assert_eq!(config.ui.recording_overlay_snap_range_percent, 500);
    assert_eq!(config.ui.recording_overlay_offset_x, 4000);
    assert_eq!(config.ui.recording_overlay_offset_y, -4000);
    assert_eq!(config.ui.app_language, "en-US");
    assert_eq!(config.ui.theme, "system");

    config.ui.recording_overlay_scale = 0.1;
    config.ui.recording_overlay_snap_range_percent = 1;

    config.normalize();

    assert_eq!(config.ui.recording_overlay_scale, 0.25);
    assert_eq!(config.ui.recording_overlay_snap_range_percent, 50);
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
    config_value
        .as_object_mut()
        .unwrap()
        .remove("model_services");
    config_value.as_object_mut().unwrap().remove("hotkeys");
    config_value["llm"]
        .as_object_mut()
        .unwrap()
        .remove("thinking_effort");
    config_value["audio"]
        .as_object_mut()
        .unwrap()
        .remove("output_volume_ducking");
    config_value["asr"]
        .as_object_mut()
        .unwrap()
        .remove("connection_retry");
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
        .missing_fields
        .contains(&"asr.connection_retry".to_string()));
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
    assert_eq!(
        result.config.asr.connection_retry,
        AsrConnectionRetryConfig::default()
    );
    let custom = result
        .config
        .model_services
        .iter()
        .find(|s| s.provider == "custom")
        .unwrap();
    assert_eq!(custom.api_key, "secret");
    assert!(result.config.llm.provider_settings.is_empty());
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
    assert!(result
        .report
        .missing_fields
        .contains(&"audio.output_volume_ducking.sound_source_hotkey_fallback_enabled".to_string()));
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
