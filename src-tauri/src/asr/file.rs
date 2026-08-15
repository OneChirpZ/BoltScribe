use super::pcm::normalized_wav_bytes;
use super::protocol::{
    audio_request, build_full_request, connection_attempt_timeout, connection_retry_delay,
    full_client_request, is_timeout_error, parse_server_message, set_socket_read_timeout,
    spawn_connect_socket_attempt, validate_config, AsrResponseState, ConnectFailure, ServerMessage,
    VolcengineSocket,
};
use super::{AsrOutput, AsrProvider, TranscriptionOutcome};
use crate::config::AsrConfig;
use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tungstenite::Message;
use uuid::Uuid;

const STREAM_CHUNK_BYTES: usize = 6_400;
const FINAL_READ_TIMEOUT: Duration = Duration::from_millis(250);
const FILE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct VolcengineFileAsr;
impl AsrProvider for VolcengineFileAsr {
    fn transcribe(&self, audio_path: &Path, config: &AsrConfig) -> Result<TranscriptionOutcome> {
        validate_config(config)?;

        let audio = normalized_wav_bytes(audio_path)
            .with_context(|| format!("Failed to prepare {}", audio_path.display()))?;
        let (mut socket, log_id, task_id) = connect_file_socket(config)?;

        let full_request = build_full_request(config, "wav");

        socket
            .send(Message::Binary(full_client_request(&full_request)?.into()))
            .context("Failed to send Volcengine ASR request metadata")?;

        let chunk_count = audio.len().div_ceil(STREAM_CHUNK_BYTES);
        for (index, chunk) in audio.chunks(STREAM_CHUNK_BYTES).enumerate() {
            let is_final = index + 1 == chunk_count;
            socket
                .send(Message::Binary(audio_request(chunk, is_final)?.into()))
                .context("Failed to send Volcengine ASR audio chunk")?;
        }
        set_socket_read_timeout(&mut socket, Some(FINAL_READ_TIMEOUT))?;

        let started_at = Instant::now();
        let mut response_state = AsrResponseState::default();
        loop {
            if started_at.elapsed() >= FILE_RESPONSE_TIMEOUT {
                bail!("Volcengine ASR websocket timed out");
            }

            let message = match socket.read() {
                Ok(message) => message,
                Err(err) if is_timeout_error(&err) => continue,
                Err(err) => {
                    return Err(err).context("Failed to read Volcengine ASR websocket response")
                }
            };
            let Message::Binary(bytes) = message else {
                continue;
            };
            let response = parse_server_message(bytes.as_ref())?;
            match response {
                ServerMessage::Result { value, final_frame } => {
                    if response_state.apply_result(&value, final_frame) {
                        break;
                    }
                }
                ServerMessage::Error { code, message } => {
                    return Err(anyhow!(
                        "Volcengine ASR websocket error: code={}, message={}, log_id={:?}",
                        code,
                        message,
                        log_id
                    ));
                }
            }
        }

        if response_state.best_text.trim().is_empty() {
            return Ok(TranscriptionOutcome::NoSpeech);
        }

        Ok(TranscriptionOutcome::Text(AsrOutput {
            text: response_state.best_text,
            provider: "volcengine_ws_file".to_string(),
            task_id: Some(task_id),
            duration_ms: response_state.service_duration_ms,
        }))
    }
}

fn connect_file_socket(config: &AsrConfig) -> Result<(VolcengineSocket, Option<String>, String)> {
    let retry = &config.connection_retry;
    let timeout = connection_attempt_timeout(config);

    for attempt in 1..=retry.file_max_attempts {
        let task_id = Uuid::new_v4().to_string();
        let receiver = spawn_connect_socket_attempt(config, &task_id);
        let result = match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(ConnectFailure::retryable(
                "timeout",
                anyhow!("File ASR connection attempt timed out"),
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(ConnectFailure::retryable(
                "connection_worker",
                anyhow!("File ASR connection worker exited unexpectedly"),
            )),
        };

        match result {
            Ok((socket, log_id)) => return Ok((socket, log_id, task_id)),
            Err(failure) => {
                eprintln!(
                    "file ASR connection attempt {} failed ({}): {:#}",
                    attempt, failure.category, failure.error
                );
                if !should_retry_file_connection(&failure, attempt, retry.file_max_attempts) {
                    let category = failure.category;
                    return Err(failure.error).with_context(|| {
                        format!(
                            "Failed to connect Volcengine ASR websocket after {attempt} attempt(s) ({category})"
                        )
                    });
                }
                std::thread::sleep(connection_retry_delay(retry, attempt));
            }
        }
    }

    unreachable!("file_max_attempts is normalized to at least one")
}

pub(super) fn should_retry_file_connection(
    failure: &ConnectFailure,
    attempt: u32,
    max_attempts: u32,
) -> bool {
    failure.retryable && attempt < max_attempts
}
