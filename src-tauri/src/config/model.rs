use super::defaults;
use serde::{Deserialize, Serialize};

pub const VAD_MIN_NOISE_MARGIN_DB: u32 = 1;
pub const VAD_MAX_NOISE_MARGIN_DB: u32 = 40;
pub const VAD_MIN_CONFIRMATION_MS: u32 = 240;
pub const VAD_MAX_CONFIRMATION_MS: u32 = 2_000;
pub const VAD_CONFIRMATION_STEP_MS: u32 = 20;
pub const VAD_MIN_NOISE_WINDOW_MS: u32 = 400;
pub const VAD_MAX_NOISE_WINDOW_MS: u32 = 3_000;
pub const VAD_NOISE_WINDOW_STEP_MS: u32 = 100;
pub const VAD_CONTINUOUS_SPEECH_MS: u32 = 240;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    #[serde(default = "defaults::default_hotkey")]
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
    #[serde(default = "defaults::default_input_device_mode")]
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
    #[serde(default)]
    pub voice_activity_detection: VoiceActivityDetectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceActivityDetectionConfig {
    #[serde(default = "defaults::default_vad_enabled")]
    pub enabled: bool,
    #[serde(default = "defaults::default_vad_noise_margin_db")]
    pub noise_margin_db: u32,
    #[serde(default = "defaults::default_vad_confirmation_ms")]
    pub confirmation_ms: u32,
    #[serde(default = "defaults::default_vad_noise_window_ms")]
    pub noise_window_ms: u32,
    #[serde(default = "defaults::default_vad_initial_silence_timeout_secs")]
    pub initial_silence_timeout_secs: u32,
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
    #[serde(default = "defaults::default_output_volume_ducking_reduction_percent")]
    pub reduction_percent: u32,
    #[serde(default)]
    pub device_name_whitelist: Vec<String>,
    #[serde(default)]
    pub sound_source_hotkey_fallback_enabled: bool,
    #[serde(default = "defaults::default_sound_source_toggle_mute_hotkey")]
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
    #[serde(default = "defaults::default_stream_url")]
    pub stream_url: String,
    #[serde(default = "defaults::default_submit_url")]
    pub submit_url: String,
    #[serde(default = "defaults::default_query_url")]
    pub query_url: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmConfig {
    #[serde(default = "defaults::default_llm_provider")]
    pub provider: String,
    #[serde(default = "defaults::default_llm_api_format")]
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
    #[serde(default = "defaults::default_system_prompt")]
    pub system_prompt: String,
    pub temperature: f32,
    pub timeout_secs: u64,
    #[serde(default)]
    pub thinking_enabled: bool,
    #[serde(default = "defaults::default_thinking_effort")]
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
    #[serde(default = "defaults::default_app_language")]
    pub app_language: String,
    #[serde(default = "defaults::default_ui_theme")]
    pub theme: String,
    #[serde(default = "defaults::default_recording_overlay_scale")]
    pub recording_overlay_scale: f32,
    #[serde(default)]
    pub recording_overlay_offset_x: i32,
    #[serde(default)]
    pub recording_overlay_offset_y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetentionConfig {
    #[serde(default = "defaults::default_max_history_records")]
    pub max_history_records: usize,
    #[serde(default = "defaults::default_max_storage_bytes")]
    pub max_storage_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemConfig {
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub hide_dock_icon: bool,
    #[serde(default = "defaults::default_tray_left_click_recording_enabled")]
    pub tray_left_click_recording_enabled: bool,
    #[serde(default = "defaults::default_fn_long_press_enabled")]
    pub fn_long_press_enabled: bool,
    #[serde(default = "defaults::default_fn_long_press_duration_ms")]
    pub fn_long_press_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorrectionConfig {
    pub enabled: bool,
    pub user_requirements: String,
    #[serde(default = "defaults::default_prompt_template")]
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: defaults::default_hotkey(),
            hotkeys: defaults::default_hotkey_slots(),
            hotkey_enabled: defaults::default_hotkey_enabled_slots(),
            audio: AudioConfig::default(),
            asr: AsrConfig {
                provider: "volcengine".to_string(),
                auth_mode: defaults::default_asr_auth_mode(),
                app_key: String::new(),
                access_key: String::new(),
                resource_id: "volc.seedasr.sauc.duration".to_string(),
                stream_url: defaults::default_stream_url(),
                submit_url: defaults::default_submit_url(),
                query_url: defaults::default_query_url(),
                language: "zh-CN".to_string(),
            },
            llm: LlmConfig {
                provider: defaults::default_llm_provider(),
                api_format: defaults::default_llm_api_format(),
                endpoint: "https://api.openai.com/v1".to_string(),
                api_key: String::new(),
                model: "gpt-5.4-mini".to_string(),
                provider_settings: Vec::new(),
                race_enabled: false,
                race_models: Vec::new(),
                race_targets: Vec::new(),
                system_prompt: defaults::default_system_prompt(),
                temperature: 0.0,
                timeout_secs: 30,
                thinking_enabled: false,
                thinking_effort: defaults::default_thinking_effort(),
                max_output_tokens: None,
                model_presets: Vec::new(),
            },
            correction: CorrectionConfig {
                enabled: true,
                user_requirements: String::new(),
                prompt_template: defaults::default_prompt_template(),
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

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device_mode: defaults::default_input_device_mode(),
            input_device_id: None,
            input_device_name: None,
            input_device_priority: Vec::new(),
            input_device_blacklist: Vec::new(),
            output_volume_ducking: OutputVolumeDuckingConfig::default(),
            voice_activity_detection: VoiceActivityDetectionConfig::default(),
        }
    }
}

impl Default for VoiceActivityDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_vad_enabled(),
            noise_margin_db: defaults::default_vad_noise_margin_db(),
            confirmation_ms: defaults::default_vad_confirmation_ms(),
            noise_window_ms: defaults::default_vad_noise_window_ms(),
            initial_silence_timeout_secs: defaults::default_vad_initial_silence_timeout_secs(),
        }
    }
}

impl Default for OutputVolumeDuckingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mute_instead_of_reduce: false,
            reduction_percent: defaults::default_output_volume_ducking_reduction_percent(),
            device_name_whitelist: Vec::new(),
            sound_source_hotkey_fallback_enabled: false,
            sound_source_toggle_mute_hotkey: defaults::default_sound_source_toggle_mute_hotkey(),
        }
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            hide_dock_icon: false,
            tray_left_click_recording_enabled: defaults::default_tray_left_click_recording_enabled(
            ),
            fn_long_press_enabled: defaults::default_fn_long_press_enabled(),
            fn_long_press_duration_ms: defaults::default_fn_long_press_duration_ms(),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            app_language: defaults::default_app_language(),
            theme: defaults::default_ui_theme(),
            recording_overlay_scale: defaults::default_recording_overlay_scale(),
            recording_overlay_offset_x: 0,
            recording_overlay_offset_y: 0,
        }
    }
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_history_records: defaults::default_max_history_records(),
            max_storage_bytes: defaults::default_max_storage_bytes(),
        }
    }
}
