//! Resource-bounded learning from edits to a confirmed dictation insertion.
pub mod activity;
mod model;
pub mod observation;
mod store;
mod validation;
mod worker;

use crate::config::LlmConfig;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub use store::{
    augment_correction_for_text, delete_entry, list_entries, set_entry_enabled, with_store_lock,
};
pub use worker::{cancel, status, submit};

pub const MAX_TEXT_UNITS: usize = 8_000;
pub const MAX_TERMS: usize = 8;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LearningConfig {
    pub enabled: bool,
    pub llm: LlmConfig,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            llm: LlmConfig {
                service_id: "openai".into(),
                provider: "openai".into(),
                api_format: "responses".into(),
                endpoint: String::new(),
                api_key: String::new(),
                model: String::new(),
                provider_settings: Vec::new(),
                resolved_services: Vec::new(),
                race_enabled: false,
                race_models: Vec::new(),
                race_targets: Vec::new(),
                system_prompt: String::new(),
                temperature: 0.2,
                timeout_secs: 60,
                fast_mode_enabled: false,
                thinking_enabled: true,
                thinking_effort: "high".into(),
                max_output_tokens: Some(8_192),
                model_presets: Vec::new(),
            },
        }
    }
}

impl LearningConfig {
    pub fn is_configured(&self) -> bool {
        !self.llm.model.trim().is_empty()
            && !self.llm.api_key.trim().is_empty()
            && !self.llm.endpoint.trim().is_empty()
    }

    pub fn can_observe(&self) -> bool {
        self.enabled && self.is_configured() && self.validate().is_ok()
    }

    pub fn validate(&self) -> Result<()> {
        // The learning model can be configured after opting in without
        // blocking unrelated settings.
        if !self.enabled || !self.is_configured() {
            return Ok(());
        }
        let url = reqwest::Url::parse(self.llm.endpoint.trim())
            .map_err(|_| anyhow::anyhow!("Learning model endpoint must be a valid HTTP(S) URL"))?;
        if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
            bail!("Learning model endpoint must be a valid HTTP(S) URL");
        }
        if !matches!(
            self.llm.api_format.as_str(),
            "chat_completions" | "responses"
        ) {
            bail!("Learning model API format is unsupported");
        }
        if !(1..=180).contains(&self.llm.timeout_secs) {
            bail!("Learning model timeout must be between 1 and 180 seconds");
        }
        if self
            .llm
            .max_output_tokens
            .is_some_and(|n| n == 0 || n > 16_384)
        {
            bail!("Learning model output limit must be between 1 and 16384 tokens");
        }
        if !self.llm.temperature.is_finite() || !(0.0..=2.0).contains(&self.llm.temperature) {
            bail!("Learning model temperature must be between 0 and 2");
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LearningSample {
    pub activity: Option<activity::ActivityHandle>,
    pub original: String,
    pub current: String,
    pub record_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnedEntry {
    pub id: String,
    pub term: String,
    pub from: Option<String>,
    pub created_at: String,
    pub record_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LearningStatus {
    pub processing: bool,
    pub pending: bool,
    pub submitted: u64,
    pub analyzed: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub skipped: u64,
    pub last_error: Option<String>,
    pub observation: observation::ObservationStats,
}
