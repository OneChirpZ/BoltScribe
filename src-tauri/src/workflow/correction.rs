use super::state::*;
use super::*;

pub(super) fn correct_recording_text(
    raw_text: &str,
    config: &AppConfig,
) -> (String, Option<String>, Vec<LlmCallLog>) {
    if !config.correction.enabled {
        return (raw_text.to_string(), None, Vec::new());
    }

    match correct_with_config(raw_text, &config.llm, &config.correction) {
        Ok(outcome) => (outcome.text, None, outcome.logs),
        Err(err) => (raw_text.to_string(), Some(err.message), err.logs),
    }
}

pub(super) fn build_retried_record(
    mut record: HistoryRecord,
    asr_output: AsrOutput,
    config: &AppConfig,
    asr_duration_ms: u64,
) -> Result<HistoryRecord> {
    let raw_text = asr_output.text.trim().to_string();
    if raw_text.is_empty() {
        return Err(anyhow!("ASR returned empty text"));
    }
    let (corrected_text, correction_error, correction_logs) =
        correct_recording_text(&raw_text, config);

    record.asr_provider = asr_output.provider;
    record.asr_task_id = asr_output.task_id;
    record.raw_text = raw_text;
    record.corrected_text = corrected_text;
    // A manual retry updates history only. It must never paste into whichever
    // application happens to be focused while the history window is open.
    record.pasted_text.clear();
    record.correction_enabled = config.correction.enabled;
    record.correction_error = correction_error;
    record.correction_logs = correction_logs;
    record.injection_error = None;
    record.workflow_error = None;
    record.asr_duration_ms = Some(asr_duration_ms);
    record.service_audio_duration_ms = asr_output.duration_ms;
    Ok(record)
}

pub(super) fn correct_with_config(
    raw_text: &str,
    llm: &LlmConfig,
    correction: &CorrectionConfig,
) -> std::result::Result<CorrectionOutcome, CorrectionFailure> {
    let race_targets = active_race_targets(llm);
    if race_targets.len() < 2 {
        return OpenAiCompatibleCorrector
            .correct(raw_text, llm, correction)
            .map(|output| CorrectionOutcome {
                text: output.text,
                logs: vec![output.log],
            })
            .map_err(|err| CorrectionFailure {
                message: err.message,
                logs: err.log.into_iter().map(|log| *log).collect(),
            });
    }

    correct_with_race(raw_text, llm, correction, race_targets)
}

pub(super) fn active_race_targets(llm: &LlmConfig) -> Vec<RaceModelTarget> {
    if !llm.race_enabled {
        return Vec::new();
    }

    let source_targets = if llm.race_targets.is_empty() {
        llm.race_models
            .iter()
            .map(|model| RaceModelTarget {
                provider: llm.provider.clone(),
                model: model.clone(),
            })
            .collect::<Vec<_>>()
    } else {
        llm.race_targets.clone()
    };

    let mut targets = Vec::new();
    for target in source_targets {
        let provider = target.provider.trim();
        let model = target.model.trim();
        if provider.is_empty()
            || model.is_empty()
            || targets.iter().any(|existing: &RaceModelTarget| {
                existing.provider == provider && existing.model == model
            })
        {
            continue;
        }
        targets.push(RaceModelTarget {
            provider: provider.to_string(),
            model: model.to_string(),
        });
    }
    targets
}

fn correct_with_race(
    raw_text: &str,
    llm: &LlmConfig,
    correction: &CorrectionConfig,
    targets: Vec<RaceModelTarget>,
) -> std::result::Result<CorrectionOutcome, CorrectionFailure> {
    let worker_count = targets.len();
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut handles = Vec::with_capacity(worker_count);

    for target in targets {
        let sender = sender.clone();
        let raw_text = raw_text.to_string();
        let llm = llm_for_race_target(llm, &target);
        let correction = correction.clone();
        handles.push(std::thread::spawn(move || {
            let result = OpenAiCompatibleCorrector.correct(&raw_text, &llm, &correction);
            let _ = sender.send((target, result));
        }));
    }
    drop(sender);

    let mut errors = Vec::new();
    let mut logs = Vec::new();
    for _ in 0..worker_count {
        let (target, result) = receiver.recv().map_err(|_| CorrectionFailure {
            message: "LLM race correction workers exited before returning".to_string(),
            logs: logs.clone(),
        })?;
        match result {
            Ok(output) => {
                logs.push(output.log.clone());
                return Ok(CorrectionOutcome {
                    text: output.text,
                    logs,
                });
            }
            Err(err) => {
                if let Some(log) = err.log {
                    logs.push(*log);
                }
                errors.push(format!(
                    "{} / {}: {}",
                    target.provider, target.model, err.message
                ));
            }
        }
    }
    drop(handles);

    Err(CorrectionFailure {
        message: format!("LLM race correction failed: {}", errors.join(" | ")),
        logs,
    })
}

fn llm_for_race_target(base: &LlmConfig, target: &RaceModelTarget) -> LlmConfig {
    let mut llm = base.clone();
    llm.provider = target.provider.clone();
    llm.model = target.model.clone();
    if target.provider == base.provider {
        return llm;
    }

    if let Some(settings) = base
        .provider_settings
        .iter()
        .find(|settings| settings.provider == target.provider)
    {
        llm.endpoint = settings.endpoint.clone();
        llm.api_format = settings.api_format.clone();
        llm.api_key = settings.api_key.clone();
        return llm;
    }

    if let Some((endpoint, api_format)) = provider_defaults(&target.provider) {
        llm.endpoint = endpoint.to_string();
        llm.api_format = api_format.to_string();
    }
    llm.api_key.clear();
    llm
}

fn provider_defaults(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        "openai" => Some(("https://api.openai.com/v1", "responses")),
        "volc_ark" => Some(("https://ark.cn-beijing.volces.com/api/v3", "responses")),
        "custom" => Some(("", "chat_completions")),
        _ => None,
    }
}

pub(super) fn append_failed_history(
    recorded: RecordedAudio,
    workflow_error: String,
    total_started_at: Instant,
    retention: &crate::config::RetentionConfig,
    live_asr_diagnostics: Option<LiveAsrDiagnostics>,
) -> Result<()> {
    let record = HistoryRecord {
        id: recorded.id,
        created_at: Utc::now(),
        audio_path: Some(recorded.path),
        asr_provider: "volcengine".to_string(),
        asr_task_id: None,
        audio_started_at: recorded.started_at,
        audio_finished_at: recorded.finished_at,
        audio_sample_rate: recorded.sample_rate,
        audio_channels: recorded.channels,
        audio_sample_count: recorded.sample_count,
        raw_text: String::new(),
        corrected_text: String::new(),
        pasted_text: String::new(),
        correction_enabled: false,
        correction_error: None,
        correction_logs: Vec::new(),
        injection_error: None,
        workflow_error: Some(workflow_error),
        asr_duration_ms: None,
        service_audio_duration_ms: None,
        live_asr_diagnostics,
        total_duration_ms: total_started_at.elapsed().as_millis(),
    };
    HistoryStore::append(&record, retention)
}
