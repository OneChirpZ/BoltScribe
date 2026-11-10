use crate::config::{CorrectionConfig, LlmConfig, PromptVariable};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fmt;
use std::time::{Duration, Instant};

pub trait LlmProvider {
    fn correct(
        &self,
        raw_text: &str,
        llm: &LlmConfig,
        correction: &CorrectionConfig,
    ) -> std::result::Result<LlmCorrectionOutput, LlmCorrectionError>;
}

pub struct OpenAiCompatibleCorrector;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallLog {
    pub provider: String,
    pub model: String,
    pub api_format: String,
    pub endpoint: String,
    pub duration_ms: u128,
    pub success: bool,
    pub request_id: Option<String>,
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub requested_service_tier: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    pub usage: Option<LlmUsage>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LlmCorrectionOutput {
    pub text: String,
    pub log: LlmCallLog,
}

#[derive(Debug, Clone)]
pub struct LlmCorrectionError {
    pub message: String,
    pub log: Option<Box<LlmCallLog>>,
}

impl LlmCorrectionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            log: None,
        }
    }

    fn with_log(message: impl Into<String>, mut log: LlmCallLog) -> Self {
        let message = message.into();
        log.success = false;
        log.error = Some(message.clone());
        Self {
            message,
            log: Some(Box::new(log)),
        }
    }
}

impl fmt::Display for LlmCorrectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for LlmCorrectionError {}

impl LlmProvider for OpenAiCompatibleCorrector {
    fn correct(
        &self,
        raw_text: &str,
        llm: &LlmConfig,
        correction: &CorrectionConfig,
    ) -> std::result::Result<LlmCorrectionOutput, LlmCorrectionError> {
        if llm.api_key.trim().is_empty() {
            return Err(LlmCorrectionError::new("LLM api_key is required"));
        }
        if llm.model.trim().is_empty() {
            return Err(LlmCorrectionError::new("LLM model is required"));
        }

        let prompt = build_correction_prompt(raw_text, correction);
        let client = Client::builder()
            .timeout(Duration::from_secs(llm.timeout_secs.max(1)))
            .build()
            .map_err(|err| {
                LlmCorrectionError::new(format!("Failed to build LLM HTTP client: {err}"))
            })?;

        let (url, body) = match llm.api_format.trim() {
            "responses" => (
                endpoint_url(&llm.endpoint, "/responses"),
                build_responses_body(llm, &prompt),
            ),
            _ => (
                endpoint_url(&llm.endpoint, "/chat/completions"),
                build_chat_completions_body(llm, &prompt),
            ),
        };

        let started_at = Instant::now();
        let mut log = LlmCallLog {
            provider: llm.provider.clone(),
            model: llm.model.clone(),
            api_format: llm.api_format.clone(),
            endpoint: url.clone(),
            duration_ms: 0,
            success: false,
            request_id: None,
            finish_reason: None,
            requested_service_tier: requested_service_tier(llm).map(str::to_string),
            service_tier: None,
            usage: None,
            error: None,
        };

        let response = match client
            .post(url)
            .bearer_auth(&llm.api_key)
            .json(&body)
            .send()
        {
            Ok(response) => response,
            Err(err) => {
                log.duration_ms = started_at.elapsed().as_millis();
                return Err(LlmCorrectionError::with_log(
                    format!("Failed to call LLM correction API: {err}"),
                    log,
                ));
            }
        };

        log.request_id = response_request_id(&response);

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            log.duration_ms = started_at.elapsed().as_millis();
            return Err(LlmCorrectionError::with_log(
                format!("LLM correction failed: status={status}, body={body}"),
                log,
            ));
        }

        let value: Value = match response.json() {
            Ok(value) => value,
            Err(err) => {
                log.duration_ms = started_at.elapsed().as_millis();
                return Err(LlmCorrectionError::with_log(
                    format!("Failed to parse LLM response: {err}"),
                    log,
                ));
            }
        };
        log.service_tier = value
            .get("service_tier")
            .and_then(Value::as_str)
            .map(str::to_string);
        let corrected = extract_corrected_text(&value, &llm.api_format);

        if corrected.is_empty() {
            log.duration_ms = started_at.elapsed().as_millis();
            log.usage = extract_usage(&value);
            log.finish_reason = extract_finish_reason(&value);
            return Err(LlmCorrectionError::with_log(
                "LLM returned empty correction",
                log,
            ));
        }

        log.duration_ms = started_at.elapsed().as_millis();
        log.success = true;
        log.usage = extract_usage(&value);
        log.finish_reason = extract_finish_reason(&value);
        Ok(LlmCorrectionOutput {
            text: corrected,
            log,
        })
    }
}

pub fn build_correction_prompt(raw_text: &str, correction: &CorrectionConfig) -> String {
    let dictionary_text = enabled_dictionary_text(
        &correction.dictionary_text,
        &correction.disabled_dictionary_terms,
    );
    let prompt = correction
        .prompt_template
        .replace("{{user_requirements}}", correction.user_requirements.trim())
        .replace("{{dictionary}}", &dictionary_text)
        .replace("{{correction_rules}}", &correction.correction_rules_text)
        .replace("{{raw_text}}", raw_text.trim());

    apply_prompt_variables(prompt, &correction.variables)
}

fn enabled_dictionary_text(dictionary_text: &str, disabled_terms: &[String]) -> String {
    let disabled_keys = disabled_terms
        .iter()
        .map(|term| term.trim().to_lowercase())
        .collect::<HashSet<_>>();
    dictionary_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !disabled_keys.contains(&line.to_lowercase()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn apply_prompt_variables(mut prompt: String, variables: &[PromptVariable]) -> String {
    for variable in variables {
        let name = variable.name.trim();
        if name.is_empty() || is_builtin_variable(name) {
            continue;
        }
        prompt = prompt.replace(&format!("{{{{{name}}}}}"), variable.value.trim());
    }
    prompt
}

fn is_builtin_variable(name: &str) -> bool {
    matches!(
        name,
        "user_requirements" | "dictionary" | "correction_rules" | "raw_text"
    )
}

pub(crate) fn endpoint_url(endpoint: &str, suffix: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.ends_with(suffix) {
        endpoint.to_string()
    } else {
        format!("{endpoint}{suffix}")
    }
}

pub(crate) fn build_chat_completions_body(llm: &LlmConfig, prompt: &str) -> Value {
    let mut body = Map::from_iter([
        ("model".to_string(), json!(llm.model)),
        ("temperature".to_string(), json!(llm.temperature)),
        (
            "messages".to_string(),
            json!([
                {
                    "role": "system",
                    "content": llm.system_prompt
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ]),
        ),
    ]);

    if let Some(service_tier) = requested_service_tier(llm) {
        body.insert("service_tier".to_string(), json!(service_tier));
    }
    if llm.provider == "volc_ark" {
        body.insert(
            "thinking".to_string(),
            json!({ "type": if llm.thinking_enabled { "enabled" } else { "disabled" } }),
        );
    }
    if let Some(effort) = requested_reasoning_effort(llm) {
        body.insert("reasoning_effort".to_string(), json!(effort));
    }
    if let Some(max_tokens) = llm.max_output_tokens {
        let key = if llm.provider == "openai" {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body.insert(key.to_string(), json!(max_tokens));
    }

    Value::Object(body)
}

pub(crate) fn build_responses_body(llm: &LlmConfig, prompt: &str) -> Value {
    let mut body = Map::from_iter([
        ("model".to_string(), json!(llm.model)),
        ("temperature".to_string(), json!(llm.temperature)),
        (
            "input".to_string(),
            json!([
                {
                    "role": "system",
                    "content": [
                        {
                            "type": "input_text",
                            "text": llm.system_prompt
                        }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": prompt
                        }
                    ]
                }
            ]),
        ),
    ]);

    if let Some(service_tier) = requested_service_tier(llm) {
        body.insert("service_tier".to_string(), json!(service_tier));
    }
    if llm.provider == "volc_ark" {
        body.insert(
            "thinking".to_string(),
            json!({ "type": if llm.thinking_enabled { "enabled" } else { "disabled" } }),
        );
    }
    if let Some(effort) = requested_reasoning_effort(llm) {
        body.insert("reasoning".to_string(), json!({ "effort": effort }));
    }
    if let Some(max_tokens) = llm.max_output_tokens {
        body.insert("max_output_tokens".to_string(), json!(max_tokens));
    }

    Value::Object(body)
}

fn requested_service_tier(llm: &LlmConfig) -> Option<&'static str> {
    match llm.provider.as_str() {
        "openai" | "volc_ark" => Some(if llm.fast_mode_enabled {
            "fast"
        } else {
            "default"
        }),
        _ => None,
    }
}

fn requested_reasoning_effort(llm: &LlmConfig) -> Option<&str> {
    if llm.thinking_enabled {
        return Some(normalize_thinking_effort(llm));
    }

    // These verified models support `none`; older and reasoning-only models
    // keep their existing request shape when thinking is disabled.
    if llm.provider == "openai" && supports_disabled_reasoning(llm.model.trim()) {
        Some("none")
    } else {
        None
    }
}

fn supports_disabled_reasoning(model: &str) -> bool {
    ["gpt-6-luna", "gpt-5.4-mini"].iter().any(|alias| {
        model == *alias
            || model
                .strip_prefix(alias)
                .and_then(|suffix| suffix.strip_prefix('-'))
                .is_some_and(|date| {
                    date.len() == 10 && chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
                })
    })
}

fn normalize_thinking_effort(llm: &LlmConfig) -> &str {
    match llm.thinking_effort.trim() {
        "none" | "minimal" if llm.provider == "volc_ark" => "minimal",
        "xhigh" if llm.provider == "volc_ark" => "high",
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" => llm.thinking_effort.trim(),
        _ => "medium",
    }
}

fn extract_chat_completions_text(value: &Value) -> String {
    value
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(value_to_text)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(crate) fn extract_corrected_text(value: &Value, api_format: &str) -> String {
    match api_format.trim() {
        "responses" => {
            let text = extract_responses_text(value);
            if text.is_empty() {
                extract_chat_completions_text(value)
            } else {
                text
            }
        }
        _ => extract_chat_completions_text(value),
    }
}

fn response_request_id(response: &reqwest::blocking::Response) -> Option<String> {
    ["x-request-id", "x-tt-logid", "x-tt-trace-id", "request-id"]
        .iter()
        .find_map(|name| {
            response
                .headers()
                .get(*name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        })
}

fn extract_usage(value: &Value) -> Option<LlmUsage> {
    let usage = value.get("usage")?;
    Some(LlmUsage {
        input_tokens: first_u64(usage, &["input_tokens", "prompt_tokens"]),
        output_tokens: first_u64(usage, &["output_tokens", "completion_tokens"]),
        total_tokens: first_u64(usage, &["total_tokens"]),
        reasoning_tokens: usage
            .get("output_tokens_details")
            .or_else(|| usage.get("completion_tokens_details"))
            .and_then(|details| first_u64(details, &["reasoning_tokens"]))
            .or_else(|| first_u64(usage, &["reasoning_tokens"])),
    })
}

fn first_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_u64()))
}

fn extract_finish_reason(value: &Value) -> Option<String> {
    value
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(|reason| reason.as_str())
        .or_else(|| value.get("status").and_then(|status| status.as_str()))
        .map(str::to_string)
}

fn extract_responses_text(value: &Value) -> String {
    if let Some(text) = value.get("output_text").and_then(|text| text.as_str()) {
        return text.trim().to_string();
    }

    value
        .get("output")
        .and_then(|output| output.as_array())
        .map(|items| {
            items
                .iter()
                .flat_map(|item| {
                    item.get("content")
                        .and_then(|content| content.as_array())
                        .into_iter()
                        .flatten()
                })
                .filter_map(|content| {
                    let content_type = content.get("type").and_then(|item_type| item_type.as_str());
                    match content_type {
                        Some("output_text") | Some("text") | None => {
                            content.get("text").and_then(|text| text.as_str())
                        }
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CorrectionConfig, PromptVariable};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    fn mock_api_response(
        status: &str,
        response: Value,
    ) -> (String, std::thread::JoinHandle<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&stream);
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(length) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = length.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).unwrap();
            let request = serde_json::from_slice(&body).unwrap();
            let response = response.to_string();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
            request
        });
        (endpoint, handle)
    }

    #[test]
    fn prompt_contains_requirements_dictionary_and_raw_text() {
        let correction = CorrectionConfig {
            enabled: true,
            user_requirements: "不要扩写".to_string(),
            prompt_template: "要求={{user_requirements}}\n词典={{dictionary}}\n原文={{raw_text}}"
                .to_string(),
            variables: Vec::new(),
            dictionary_text: "LDFC".to_string(),
            disabled_dictionary_terms: Vec::new(),
            correction_rules_text: String::new(),
            correction_rules: Vec::new(),
            dictionary: Vec::new(),
        };
        let prompt = build_correction_prompt("这是 ldfc", &correction);
        assert!(prompt.contains("不要扩写"));
        assert!(prompt.contains("LDFC"));
        assert!(prompt.contains("这是 ldfc"));
    }

    #[test]
    fn prompt_uses_simple_dictionary_lines_and_variables() {
        let correction = CorrectionConfig {
            enabled: true,
            user_requirements: "保留口吻".to_string(),
            prompt_template:
                "场景={{scene}}\n词典={{dictionary}}\n原文：\n```text\n{{raw_text}}\n```"
                    .to_string(),
            variables: vec![PromptVariable {
                name: "scene".to_string(),
                value: "技术讨论".to_string(),
            }],
            dictionary_text: "LDFC\nCodex".to_string(),
            disabled_dictionary_terms: Vec::new(),
            correction_rules_text: "艾迪 -> ID".to_string(),
            correction_rules: Vec::new(),
            dictionary: Vec::new(),
        };

        let prompt = build_correction_prompt("这是 ldfc", &correction);

        assert!(prompt.contains("场景=技术讨论"));
        assert!(prompt.contains("词典=LDFC\nCodex"));
        assert!(!prompt.contains("艾迪"));
        assert!(prompt.contains("```text\n这是 ldfc\n```"));
    }

    #[test]
    fn prompt_excludes_disabled_dictionary_terms_without_removing_them_from_config() {
        let correction = CorrectionConfig {
            enabled: true,
            user_requirements: String::new(),
            prompt_template: "词典={{dictionary}}\n原文={{raw_text}}".to_string(),
            variables: Vec::new(),
            dictionary_text: "BoltScribe\nCodex\nCodex CLI\nLDFC".to_string(),
            disabled_dictionary_terms: vec![" codex ".to_string()],
            correction_rules_text: String::new(),
            correction_rules: Vec::new(),
            dictionary: Vec::new(),
        };

        let prompt = build_correction_prompt("测试", &correction);

        assert_eq!(prompt, "词典=BoltScribe\nCodex CLI\nLDFC\n原文=测试");
        assert_eq!(
            correction.dictionary_text,
            "BoltScribe\nCodex\nCodex CLI\nLDFC"
        );
    }

    #[test]
    fn prompt_uses_raw_correction_rules_text() {
        let correction = CorrectionConfig {
            enabled: true,
            user_requirements: "保留口吻".to_string(),
            prompt_template: "规则={{correction_rules}}\n原文={{raw_text}}".to_string(),
            variables: Vec::new(),
            dictionary_text: String::new(),
            disabled_dictionary_terms: Vec::new(),
            correction_rules_text: "艾迪 => ID # 英文缩写".to_string(),
            correction_rules: Vec::new(),
            dictionary: Vec::new(),
        };

        let prompt = build_correction_prompt("这是艾迪", &correction);

        assert!(prompt.contains("艾迪 => ID # 英文缩写"));
    }

    #[test]
    fn responses_output_text_is_extracted() {
        let value = json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "已纠错文本"
                        }
                    ]
                }
            ]
        });

        assert_eq!(extract_responses_text(&value), "已纠错文本");
    }

    #[test]
    fn responses_top_level_output_text_wins() {
        let value = json!({
            "output_text": "最终文本",
            "output": []
        });

        assert_eq!(extract_responses_text(&value), "最终文本");
    }

    #[test]
    fn responses_body_uses_reasoning_when_enabled() {
        let mut llm = crate::config::AppConfig::default().llm;
        llm.api_format = "responses".to_string();
        llm.thinking_enabled = true;
        llm.thinking_effort = "high".to_string();
        llm.max_output_tokens = Some(128);

        let body = build_responses_body(&llm, "原文");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["max_output_tokens"], 128);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn service_tier_switch_covers_both_api_formats_and_supported_providers() {
        for provider in ["openai", "volc_ark", "custom"] {
            for fast_mode_enabled in [true, false] {
                let mut llm = crate::config::AppConfig::default().llm;
                llm.provider = provider.to_string();
                llm.fast_mode_enabled = fast_mode_enabled;
                for body in [
                    build_responses_body(&llm, "原文"),
                    build_chat_completions_body(&llm, "原文"),
                ] {
                    let expected = if provider == "custom" {
                        None
                    } else if fast_mode_enabled {
                        Some("fast")
                    } else {
                        Some("default")
                    };
                    assert_eq!(
                        body.get("service_tier").and_then(Value::as_str),
                        expected,
                        "provider={provider}, fast_mode_enabled={fast_mode_enabled}"
                    );
                }
            }
        }
    }

    #[test]
    fn verified_openai_models_explicitly_disable_reasoning_in_both_formats() {
        for model in [
            "gpt-6-luna",
            "gpt-5.4-mini",
            "gpt-5.4-mini-2026-03-17",
            "gpt-6-luna-2026-10-01",
        ] {
            let mut llm = crate::config::AppConfig::default().llm;
            llm.model = model.to_string();
            llm.thinking_effort = "high".to_string();
            assert_eq!(
                build_responses_body(&llm, "原文")["reasoning"]["effort"],
                "none"
            );
            assert_eq!(
                build_chat_completions_body(&llm, "原文")["reasoning_effort"],
                "none"
            );

            llm.thinking_enabled = true;
            assert_eq!(
                build_responses_body(&llm, "原文")["reasoning"]["effort"],
                "high"
            );
            assert_eq!(
                build_chat_completions_body(&llm, "原文")["reasoning_effort"],
                "high"
            );
        }
    }

    #[test]
    fn disabled_reasoning_does_not_add_unsupported_openai_parameters() {
        for model in [
            "gpt-4.1-mini",
            "gpt-6.1-sol",
            "gpt-6-astra",
            "gpt-6-luna-preview",
            "gpt-6-luna-2026-02-31",
            "gpt-6-luna-2026-1-01",
            "gpt-5.4-mini-custom",
        ] {
            let mut llm = crate::config::AppConfig::default().llm;
            llm.model = model.to_string();
            assert!(
                build_responses_body(&llm, "原文")
                    .get("reasoning")
                    .is_none(),
                "{model}"
            );
            assert!(
                build_chat_completions_body(&llm, "原文")
                    .get("reasoning_effort")
                    .is_none(),
                "{model}"
            );
        }
        for provider in ["custom", "volc_ark"] {
            let mut llm = crate::config::AppConfig::default().llm;
            llm.provider = provider.to_string();
            assert!(build_responses_body(&llm, "原文")
                .get("reasoning")
                .is_none());
            assert!(build_chat_completions_body(&llm, "原文")
                .get("reasoning_effort")
                .is_none());
            if provider == "volc_ark" {
                assert_eq!(
                    build_responses_body(&llm, "原文")["thinking"]["type"],
                    "disabled"
                );
                assert_eq!(
                    build_chat_completions_body(&llm, "原文")["thinking"]["type"],
                    "disabled"
                );
            }
        }
    }

    #[test]
    fn correction_logs_actual_service_tier_including_priority_and_downgrade() {
        for api_format in ["responses", "chat_completions"] {
            for actual_tier in ["fast", "priority", "default"] {
                let response = if api_format == "responses" {
                    json!({"output_text": "已纠错", "status": "completed", "service_tier": actual_tier})
                } else {
                    json!({"choices": [{"message": {"content": "已纠错"}, "finish_reason": "stop"}], "service_tier": actual_tier})
                };
                let (endpoint, request) = mock_api_response("200 OK", response);
                let mut config = crate::config::AppConfig::default();
                config.llm.endpoint = endpoint;
                config.llm.api_key = "test-key".to_string();
                config.llm.api_format = api_format.to_string();
                let output = OpenAiCompatibleCorrector
                    .correct("原文", &config.llm, &config.correction)
                    .unwrap();
                assert_eq!(request.join().unwrap()["service_tier"], "fast");
                assert_eq!(output.log.requested_service_tier.as_deref(), Some("fast"));
                assert_eq!(output.log.service_tier.as_deref(), Some(actual_tier));
                assert_eq!(output.text, "已纠错");
            }
        }
    }

    #[test]
    fn unavailable_fast_mode_returns_provider_error_without_silent_fallback() {
        let (endpoint, request) = mock_api_response(
            "404 Not Found",
            json!({"error": {"code": "ModelNotOpen", "message": "Fast mode is not activated"}}),
        );
        let mut config = crate::config::AppConfig::default();
        config.llm.provider = "volc_ark".to_string();
        config.llm.endpoint = endpoint;
        config.llm.api_key = "test-key".to_string();
        let error = OpenAiCompatibleCorrector
            .correct("原文", &config.llm, &config.correction)
            .unwrap_err();
        assert_eq!(request.join().unwrap()["service_tier"], "fast");
        assert!(error.message.contains("ModelNotOpen"));
        let log = error.log.unwrap();
        assert_eq!(log.requested_service_tier.as_deref(), Some("fast"));
        assert!(log.service_tier.is_none());
        assert!(!log.success);
    }

    #[test]
    fn old_correction_logs_load_without_service_tier_fields() {
        let log: LlmCallLog = serde_json::from_value(json!({
            "provider": "openai",
            "model": "gpt-5.4-mini",
            "api_format": "responses",
            "endpoint": "https://api.openai.com/v1/responses",
            "duration_ms": 123,
            "success": true,
            "request_id": null,
            "finish_reason": "completed",
            "usage": null,
            "error": null
        }))
        .unwrap();
        assert!(log.requested_service_tier.is_none());
        assert!(log.service_tier.is_none());
    }

    #[test]
    fn chat_body_uses_openai_max_completion_tokens_without_tools() {
        let mut llm = crate::config::AppConfig::default().llm;
        llm.api_format = "chat_completions".to_string();
        llm.max_output_tokens = Some(128);

        let body = build_chat_completions_body(&llm, "原文");
        assert_eq!(body["max_completion_tokens"], 128);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn chat_body_keeps_legacy_max_tokens_for_compatible_providers() {
        let mut llm = crate::config::AppConfig::default().llm;
        llm.provider = "custom".to_string();
        llm.api_format = "chat_completions".to_string();
        llm.max_output_tokens = Some(128);

        let body = build_chat_completions_body(&llm, "原文");
        assert_eq!(body["max_tokens"], 128);
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn chat_content_array_is_extracted() {
        let value = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            {
                                "type": "text",
                                "text": "兼容文本"
                            }
                        ]
                    }
                }
            ]
        });

        assert_eq!(extract_chat_completions_text(&value), "兼容文本");
    }

    #[test]
    fn responses_format_falls_back_to_chat_shape() {
        let value = json!({
            "choices": [
                {
                    "message": {
                        "content": "兼容 Chat 返回"
                    }
                }
            ]
        });

        assert_eq!(
            extract_corrected_text(&value, "responses"),
            "兼容 Chat 返回"
        );
    }

    #[test]
    fn usage_extracts_reasoning_tokens_from_responses_shape() {
        let value = json!({
            "usage": {
                "input_tokens": 12,
                "output_tokens": 8,
                "total_tokens": 20,
                "output_tokens_details": {
                    "reasoning_tokens": 3
                }
            }
        });

        let usage = extract_usage(&value).unwrap();

        assert_eq!(usage.input_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(8));
        assert_eq!(usage.total_tokens, Some(20));
        assert_eq!(usage.reasoning_tokens, Some(3));
    }
}
