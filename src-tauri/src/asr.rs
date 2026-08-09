use crate::config::AsrConfig;
use crate::recorder::{AudioChunk, AudioSink};
use anyhow::{anyhow, bail, Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{client::IntoClientRequest, Message};
use uuid::Uuid;

const TARGET_SAMPLE_RATE: u32 = 16_000;
const STREAM_CHUNK_BYTES: usize = 6_400;
const STREAM_FRAME_SAMPLES: usize = (TARGET_SAMPLE_RATE as usize) / 5;
const LIVE_DRAIN_TIMEOUT: Duration = Duration::from_millis(5);
const FINAL_READ_TIMEOUT: Duration = Duration::from_millis(250);
const LIVE_FINAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const FILE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(6);
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(15);
const STREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const LIVE_AUDIO_BUFFER_LIMIT: usize = 128 * 1024 * 1024;
const LIVE_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(250),
    Duration::from_millis(750),
    Duration::from_millis(1_500),
    Duration::from_secs(3),
    Duration::from_secs(5),
];

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LiveAsrDiagnostics {
    pub connection_attempts: u32,
    pub first_connected_after_ms: Option<u64>,
    pub peak_buffered_bytes: u64,
    pub last_error_category: Option<String>,
    pub fallback_reason: Option<String>,
}

#[derive(Debug)]
pub struct LiveAsrResult {
    pub output: Result<AsrOutput>,
    pub diagnostics: LiveAsrDiagnostics,
}

type VolcengineSocket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

pub trait AsrProvider {
    fn transcribe(&self, audio_path: &Path, config: &AsrConfig) -> Result<AsrOutput>;
}

#[derive(Debug, Clone)]
pub struct AsrOutput {
    pub text: String,
    pub provider: String,
    pub task_id: Option<String>,
    pub duration_ms: Option<u64>,
}

pub struct VolcengineFileAsr;

pub struct VolcengineLiveAsrSession {
    sender: Option<AudioSink>,
    handle: Option<JoinHandle<LiveAsrResult>>,
    activity: Option<LiveAsrActivity>,
}

/// Optional progress signal consumed by the local VAD gate.  Some service
/// endpoints return only a final result, therefore local VAD remains the
/// primary activity signal when this value is absent.
#[derive(Clone)]
pub struct LiveAsrActivity {
    started_at: Instant,
    last_progress_ms: Arc<AtomicU64>,
}

impl LiveAsrActivity {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            last_progress_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn last_progress_ms(&self) -> Option<u64> {
        match self.last_progress_ms.load(Ordering::Acquire) {
            0 => None,
            value => Some(value),
        }
    }

    fn note_progress(&self) {
        let elapsed = self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.last_progress_ms
            .store(elapsed.max(1), Ordering::Release);
    }
}

impl VolcengineLiveAsrSession {
    pub fn start(config: &AsrConfig) -> Result<Self> {
        Self::start_with_activity(config, None)
    }

    pub fn start_with_activity(
        config: &AsrConfig,
        activity: Option<LiveAsrActivity>,
    ) -> Result<Self> {
        validate_config(config)?;
        build_ws_request(config, "configuration-check")?;

        let config = config.clone();
        let (sender, receiver) = mpsc::channel::<AudioChunk>();
        let worker_activity = activity.clone();
        let handle = std::thread::spawn(move || live_asr_worker(config, receiver, worker_activity));

        Ok(Self {
            sender: Some(sender),
            handle: Some(handle),
            activity,
        })
    }

    pub fn activity(&self) -> Option<LiveAsrActivity> {
        self.activity.clone()
    }

    pub fn audio_sender(&self) -> Result<AudioSink> {
        self.sender
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("Live ASR session is already finishing"))
    }

    pub fn finish(mut self) -> LiveAsrResult {
        self.sender.take();
        let Some(handle) = self.handle.take() else {
            return LiveAsrResult {
                output: Err(anyhow!("Live ASR worker is not running")),
                diagnostics: LiveAsrDiagnostics {
                    fallback_reason: Some("worker_missing".to_string()),
                    ..Default::default()
                },
            };
        };
        handle.join().unwrap_or_else(|_| LiveAsrResult {
            output: Err(anyhow!("Live ASR worker panicked")),
            diagnostics: LiveAsrDiagnostics {
                fallback_reason: Some("worker_panicked".to_string()),
                ..Default::default()
            },
        })
    }
}

impl AsrProvider for VolcengineFileAsr {
    fn transcribe(&self, audio_path: &Path, config: &AsrConfig) -> Result<AsrOutput> {
        validate_config(config)?;

        let task_id = Uuid::new_v4().to_string();
        let audio = normalized_wav_bytes(audio_path)
            .with_context(|| format!("Failed to prepare {}", audio_path.display()))?;
        let (mut socket, log_id) = connect_live_socket(config, &task_id).map_err(|failure| {
            failure
                .error
                .context("Failed to connect Volcengine ASR websocket")
        })?;

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
            bail!(
                "Volcengine ASR response did not contain text, log_id={:?}",
                log_id
            );
        }

        Ok(AsrOutput {
            text: response_state.best_text,
            provider: "volcengine_ws_file".to_string(),
            task_id: Some(task_id),
            duration_ms: response_state.service_duration_ms,
        })
    }
}

fn live_asr_worker(
    config: AsrConfig,
    receiver: mpsc::Receiver<AudioChunk>,
    activity: Option<LiveAsrActivity>,
) -> LiveAsrResult {
    let started_at = Instant::now();
    let mut diagnostics = LiveAsrDiagnostics::default();
    let mut buffered = Vec::new();
    let mut buffered_bytes = 0usize;
    let mut recording_finished = false;
    let mut final_attempt_used = false;

    loop {
        if recording_finished {
            final_attempt_used = true;
        }
        diagnostics.connection_attempts = diagnostics.connection_attempts.saturating_add(1);
        let task_id = Uuid::new_v4().to_string();
        let (attempt_sender, attempt_receiver) = mpsc::channel();
        let attempt_started_at = Instant::now();
        let attempt_config = config.clone();
        let attempt_task_id = task_id.clone();
        std::thread::spawn(move || {
            let _ = attempt_sender.send(connect_live_socket(&attempt_config, &attempt_task_id));
        });

        let connection = loop {
            match attempt_receiver.try_recv() {
                Ok(result) => break result,
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(ConnectFailure::retryable(
                        "connection_worker",
                        anyhow!("Live ASR connection worker exited unexpectedly"),
                    ));
                }
                Err(mpsc::TryRecvError::Empty)
                    if attempt_started_at.elapsed() >= CONNECT_ATTEMPT_TIMEOUT =>
                {
                    break Err(ConnectFailure::retryable(
                        "timeout",
                        anyhow!("Live ASR connection attempt timed out"),
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }

            if !recording_finished {
                match receiver.recv_timeout(Duration::from_millis(10)) {
                    Ok(chunk) => {
                        if let Err(error) = buffer_live_audio_chunk(
                            chunk,
                            &mut buffered,
                            &mut buffered_bytes,
                            &mut diagnostics,
                        ) {
                            diagnostics.fallback_reason = Some("buffer_limit_exceeded".to_string());
                            return LiveAsrResult {
                                output: Err(error),
                                diagnostics,
                            };
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => recording_finished = true,
                }
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        };

        match connection {
            Ok((socket, log_id)) => {
                diagnostics.first_connected_after_ms =
                    Some(started_at.elapsed().as_millis() as u64);
                let output = stream_live_audio(
                    &config,
                    task_id,
                    socket,
                    log_id,
                    buffered,
                    receiver,
                    activity.clone(),
                );
                if output.is_err() {
                    diagnostics.last_error_category = Some("stream_interrupted".to_string());
                    diagnostics.fallback_reason = Some("stream_interrupted".to_string());
                }
                return LiveAsrResult {
                    output,
                    diagnostics,
                };
            }
            Err(failure) => {
                eprintln!(
                    "live ASR connection attempt {} failed ({}): {:#}",
                    diagnostics.connection_attempts, failure.category, failure.error
                );
                diagnostics.last_error_category = Some(failure.category.to_string());
                if !failure.retryable {
                    diagnostics.fallback_reason =
                        Some("non_retryable_connection_error".to_string());
                    return LiveAsrResult {
                        output: Err(failure.error),
                        diagnostics,
                    };
                }

                if recording_finished {
                    if final_attempt_used {
                        diagnostics.fallback_reason =
                            Some("connection_attempts_exhausted".to_string());
                        return LiveAsrResult {
                            output: Err(failure.error),
                            diagnostics,
                        };
                    }
                    final_attempt_used = true;
                    continue;
                }

                let delay = live_retry_delay(diagnostics.connection_attempts as usize);
                if let Err(error) = buffer_live_audio_for_delay(
                    &receiver,
                    delay,
                    &mut buffered,
                    &mut buffered_bytes,
                    &mut diagnostics,
                    &mut recording_finished,
                ) {
                    diagnostics.fallback_reason = Some("buffer_limit_exceeded".to_string());
                    return LiveAsrResult {
                        output: Err(error),
                        diagnostics,
                    };
                }
            }
        }
    }
}

#[derive(Debug)]
struct ConnectFailure {
    category: &'static str,
    retryable: bool,
    error: anyhow::Error,
}

impl ConnectFailure {
    fn retryable(category: &'static str, error: anyhow::Error) -> Self {
        Self {
            category,
            retryable: true,
            error,
        }
    }

    fn permanent(category: &'static str, error: anyhow::Error) -> Self {
        Self {
            category,
            retryable: false,
            error,
        }
    }
}

fn connect_live_socket(
    config: &AsrConfig,
    task_id: &str,
) -> std::result::Result<(VolcengineSocket, Option<String>), ConnectFailure> {
    let request = build_ws_request(config, task_id)
        .map_err(|error| ConnectFailure::permanent("configuration", error))?;
    let host = request
        .uri()
        .host()
        .ok_or_else(|| ConnectFailure::permanent("invalid_url", anyhow!("ASR URL has no host")))?
        .to_string();
    let port = request.uri().port_u16().unwrap_or_else(|| {
        if request.uri().scheme_str() == Some("ws") {
            80
        } else {
            443
        }
    });
    let addresses = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| ConnectFailure::retryable("dns", error.into()))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(ConnectFailure::retryable(
            "dns",
            anyhow!("ASR host resolved to no addresses"),
        ));
    }

    let mut last_error = None;
    let mut tcp = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let tcp = tcp.ok_or_else(|| {
        ConnectFailure::retryable(
            "tcp",
            last_error
                .map(anyhow::Error::from)
                .unwrap_or_else(|| anyhow!("Failed to connect ASR TCP socket")),
        )
    })?;
    tcp.set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|error| ConnectFailure::retryable("timeout", error.into()))?;
    tcp.set_write_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|error| ConnectFailure::retryable("timeout", error.into()))?;

    let (mut socket, response) =
        tungstenite::client_tls(request, tcp).map_err(classify_handshake_error)?;
    let log_id = header_value(response.headers(), "x-tt-logid");
    set_socket_write_timeout(&mut socket, Some(STREAM_WRITE_TIMEOUT))
        .map_err(|error| ConnectFailure::retryable("socket", error))?;
    set_socket_read_timeout(&mut socket, Some(LIVE_DRAIN_TIMEOUT))
        .map_err(|error| ConnectFailure::retryable("socket", error))?;
    Ok((socket, log_id))
}

fn classify_handshake_error(
    error: tungstenite::HandshakeError<
        tungstenite::handshake::client::ClientHandshake<MaybeTlsStream<TcpStream>>,
    >,
) -> ConnectFailure {
    match error {
        tungstenite::HandshakeError::Failure(error) => match error {
            tungstenite::Error::Http(response) => {
                let status = response.status();
                let retryable =
                    status.is_server_error() || status.as_u16() == 408 || status.as_u16() == 429;
                let error = anyhow!("Live ASR websocket handshake returned HTTP {status}");
                if retryable {
                    ConnectFailure::retryable("http_temporary", error)
                } else {
                    ConnectFailure::permanent("http_client", error)
                }
            }
            tungstenite::Error::Tls(error) => {
                ConnectFailure::permanent("tls_certificate", error.into())
            }
            tungstenite::Error::Url(error) => {
                ConnectFailure::permanent("invalid_url", error.into())
            }
            tungstenite::Error::HttpFormat(error) => {
                ConnectFailure::permanent("configuration", error.into())
            }
            tungstenite::Error::Io(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                ConnectFailure::retryable("timeout", error.into())
            }
            tungstenite::Error::Io(error) => ConnectFailure::retryable("network", error.into()),
            other => ConnectFailure::retryable("handshake", other.into()),
        },
        tungstenite::HandshakeError::Interrupted(_) => ConnectFailure::retryable(
            "timeout",
            anyhow!("Live ASR websocket handshake did not finish within the timeout"),
        ),
    }
}

fn live_retry_delay(completed_attempts: usize) -> Duration {
    LIVE_RETRY_DELAYS[completed_attempts
        .saturating_sub(1)
        .min(LIVE_RETRY_DELAYS.len() - 1)]
}

fn audio_chunk_bytes(chunk: &AudioChunk) -> usize {
    chunk
        .samples
        .len()
        .saturating_mul(std::mem::size_of::<i16>())
}

fn buffer_live_audio_chunk(
    chunk: AudioChunk,
    buffered: &mut Vec<AudioChunk>,
    buffered_bytes: &mut usize,
    diagnostics: &mut LiveAsrDiagnostics,
) -> Result<()> {
    buffer_live_audio_chunk_with_limit(
        chunk,
        buffered,
        buffered_bytes,
        diagnostics,
        LIVE_AUDIO_BUFFER_LIMIT,
    )
}

fn buffer_live_audio_chunk_with_limit(
    chunk: AudioChunk,
    buffered: &mut Vec<AudioChunk>,
    buffered_bytes: &mut usize,
    diagnostics: &mut LiveAsrDiagnostics,
    limit: usize,
) -> Result<()> {
    let next_bytes = buffered_bytes.saturating_add(audio_chunk_bytes(&chunk));
    if next_bytes > limit {
        bail!("Live ASR audio buffer exceeded 128 MiB");
    }
    *buffered_bytes = next_bytes;
    diagnostics.peak_buffered_bytes = diagnostics.peak_buffered_bytes.max(next_bytes as u64);
    buffered.push(chunk);
    Ok(())
}

fn buffer_live_audio_for_delay(
    receiver: &mpsc::Receiver<AudioChunk>,
    delay: Duration,
    buffered: &mut Vec<AudioChunk>,
    buffered_bytes: &mut usize,
    diagnostics: &mut LiveAsrDiagnostics,
    recording_finished: &mut bool,
) -> Result<()> {
    let deadline = Instant::now() + delay;
    while !*recording_finished {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(20))) {
            Ok(chunk) => buffer_live_audio_chunk(chunk, buffered, buffered_bytes, diagnostics)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => *recording_finished = true,
        }
    }
    Ok(())
}

fn stream_live_audio(
    config: &AsrConfig,
    task_id: String,
    mut socket: VolcengineSocket,
    log_id: Option<String>,
    buffered: Vec<AudioChunk>,
    receiver: mpsc::Receiver<AudioChunk>,
    activity: Option<LiveAsrActivity>,
) -> Result<AsrOutput> {
    let full_request = build_full_request(config, "pcm");
    socket
        .send(Message::Binary(full_client_request(&full_request)?.into()))
        .context("Failed to send Volcengine live ASR request metadata")?;

    let mut converter = StreamingPcmConverter::default();
    let mut framer = LiveAudioFramer::default();
    let mut delayed_frame: Option<Vec<u8>> = None;
    let mut response_state = AsrResponseState::new(activity);

    for chunk in buffered.into_iter().chain(receiver) {
        let pcm = converter.push_chunk(&chunk)?;
        for frame in framer.push_samples(&pcm) {
            if let Some(frame_to_send) = delayed_frame.replace(frame) {
                socket
                    .send(Message::Binary(
                        audio_request(&frame_to_send, false)?.into(),
                    ))
                    .context("Failed to send Volcengine live ASR audio chunk")?;
                drain_available_responses(&mut socket, &mut response_state, &log_id)?;
            }
        }
    }

    let tail = converter.finish();
    for frame in framer.push_samples(&tail) {
        if let Some(frame_to_send) = delayed_frame.replace(frame) {
            socket
                .send(Message::Binary(
                    audio_request(&frame_to_send, false)?.into(),
                ))
                .context("Failed to send Volcengine live ASR audio chunk")?;
            drain_available_responses(&mut socket, &mut response_state, &log_id)?;
        }
    }

    let final_frames = finish_live_audio_frames(delayed_frame, &mut framer);
    if final_frames.is_empty() {
        bail!("No audio samples captured for live ASR");
    }

    for (frame, is_final) in final_frames {
        let context = if is_final {
            "Failed to send Volcengine live ASR final audio chunk"
        } else {
            "Failed to send Volcengine live ASR audio chunk"
        };
        socket
            .send(Message::Binary(audio_request(&frame, is_final)?.into()))
            .context(context)?;
        if !is_final {
            drain_available_responses(&mut socket, &mut response_state, &log_id)?;
        }
    }
    set_socket_read_timeout(&mut socket, Some(FINAL_READ_TIMEOUT))?;
    wait_for_final_response(&mut socket, &mut response_state, &log_id)?;

    if response_state.best_text.trim().is_empty() {
        bail!(
            "Volcengine live ASR response did not contain text, log_id={:?}",
            log_id
        );
    }

    Ok(AsrOutput {
        text: response_state.best_text,
        provider: "volcengine_ws_live".to_string(),
        task_id: Some(task_id),
        duration_ms: response_state.service_duration_ms,
    })
}

fn build_full_request(config: &AsrConfig, audio_format: &str) -> Value {
    json!({
        "user": { "uid": request_uid(config) },
        "audio": {
            "format": audio_format,
            "codec": "raw",
            "rate": TARGET_SAMPLE_RATE,
            "bits": 16,
            "channel": 1,
            "language": normalized_language(&config.language)
        },
        "request": {
            "model_name": "bigmodel",
            "enable_itn": true,
            "enable_punc": true,
            "enable_ddc": false,
            "show_utterances": true
        }
    })
}

fn request_uid(config: &AsrConfig) -> &str {
    if uses_legacy_auth(config) {
        let uid = config.app_key.trim();
        if !uid.is_empty() {
            return uid;
        }
    }
    "boltscribe"
}

struct AsrResponseState {
    best_text: String,
    service_duration_ms: Option<u64>,
    activity: Option<LiveAsrActivity>,
    last_definite_end_ms: Option<u64>,
}

impl AsrResponseState {
    fn new(activity: Option<LiveAsrActivity>) -> Self {
        Self {
            best_text: String::new(),
            service_duration_ms: None,
            activity,
            last_definite_end_ms: None,
        }
    }

    fn apply_result(&mut self, value: &Value, final_frame: bool) -> bool {
        if let Ok(text) = extract_text(value) {
            if !text.trim().is_empty() && text != self.best_text {
                self.best_text = text;
                if let Some(activity) = &self.activity {
                    activity.note_progress();
                }
            }
        }

        if let Some(end_ms) = definite_utterance_end(value) {
            let progressed = self
                .last_definite_end_ms
                .map_or(true, |previous| end_ms > previous);
            if progressed {
                self.last_definite_end_ms = Some(end_ms);
            }
            if progressed {
                if let Some(activity) = &self.activity {
                    activity.note_progress();
                }
            }
        }

        if let Some(duration) = value
            .get("audio_info")
            .and_then(|info| info.get("duration"))
            .and_then(|duration| duration.as_u64())
        {
            self.service_duration_ms = Some(duration);
        }

        final_frame
    }
}

impl Default for AsrResponseState {
    fn default() -> Self {
        Self::new(None)
    }
}

fn definite_utterance_end(value: &Value) -> Option<u64> {
    value
        .get("result")
        .and_then(|result| result.get("utterances"))
        .and_then(Value::as_array)
        .and_then(|utterances| {
            utterances
                .iter()
                .filter(|utterance| {
                    utterance.get("definite").and_then(Value::as_bool) == Some(true)
                })
                .filter_map(|utterance| {
                    utterance
                        .get("end_time_ms")
                        .and_then(Value::as_u64)
                        .or_else(|| utterance.get("end_time").and_then(Value::as_u64))
                })
                .max()
        })
}

#[derive(Default)]
struct StreamingPcmConverter {
    sample_rate: Option<u32>,
    channels: Option<u16>,
    pending_mono: Vec<i16>,
    next_src_pos: f64,
}

impl StreamingPcmConverter {
    fn push_chunk(&mut self, chunk: &AudioChunk) -> Result<Vec<i16>> {
        if chunk.samples.is_empty() {
            return Ok(Vec::new());
        }
        if chunk.sample_rate == 0 || chunk.channels == 0 {
            bail!("Invalid live audio format");
        }

        match (self.sample_rate, self.channels) {
            (None, None) => {
                self.sample_rate = Some(chunk.sample_rate);
                self.channels = Some(chunk.channels);
            }
            (Some(rate), Some(channels))
                if rate == chunk.sample_rate && channels == chunk.channels => {}
            _ => bail!("Live audio format changed during recording"),
        }

        self.pending_mono
            .extend(downmix_to_mono(&chunk.samples, chunk.channels));
        Ok(self.drain_resampled(false))
    }

    fn finish(&mut self) -> Vec<i16> {
        self.drain_resampled(true)
    }

    fn drain_resampled(&mut self, final_chunk: bool) -> Vec<i16> {
        let Some(source_rate) = self.sample_rate else {
            return Vec::new();
        };
        if self.pending_mono.is_empty() {
            return Vec::new();
        }

        let step = source_rate as f64 / TARGET_SAMPLE_RATE as f64;
        let mut out = Vec::new();
        while self.next_src_pos + 1.0 < self.pending_mono.len() as f64 {
            out.push(interpolate_sample(&self.pending_mono, self.next_src_pos));
            self.next_src_pos += step;
        }

        if final_chunk {
            while self.next_src_pos < self.pending_mono.len() as f64 {
                out.push(interpolate_sample(&self.pending_mono, self.next_src_pos));
                self.next_src_pos += step;
            }
            self.pending_mono.clear();
            self.next_src_pos = 0.0;
        } else {
            let consumed =
                (self.next_src_pos.floor() as usize).min(self.pending_mono.len().saturating_sub(1));
            if consumed > 0 {
                self.pending_mono.drain(0..consumed);
                self.next_src_pos -= consumed as f64;
            }
        }

        out
    }
}

#[derive(Default)]
struct LiveAudioFramer {
    pending_samples: Vec<i16>,
}

impl LiveAudioFramer {
    fn push_samples(&mut self, samples: &[i16]) -> Vec<Vec<u8>> {
        self.pending_samples.extend_from_slice(samples);
        let mut frames = Vec::new();
        while self.pending_samples.len() >= STREAM_FRAME_SAMPLES {
            let frame_samples = self
                .pending_samples
                .drain(0..STREAM_FRAME_SAMPLES)
                .collect::<Vec<_>>();
            frames.push(pcm_bytes(&frame_samples));
        }
        frames
    }

    fn finish(&mut self) -> Option<Vec<u8>> {
        if self.pending_samples.is_empty() {
            return None;
        }
        let frame = pcm_bytes(&self.pending_samples);
        self.pending_samples.clear();
        Some(frame)
    }
}

fn finish_live_audio_frames(
    delayed_frame: Option<Vec<u8>>,
    framer: &mut LiveAudioFramer,
) -> Vec<(Vec<u8>, bool)> {
    let partial_frame = framer.finish();
    match (delayed_frame, partial_frame) {
        (Some(delayed), Some(final_frame)) => vec![(delayed, false), (final_frame, true)],
        (Some(final_frame), None) | (None, Some(final_frame)) => vec![(final_frame, true)],
        (None, None) => Vec::new(),
    }
}

fn validate_config(config: &AsrConfig) -> Result<()> {
    if uses_legacy_auth(config) && config.app_key.trim().is_empty() {
        return Err(anyhow!(
            "Volcengine app_key is required for legacy ASR auth"
        ));
    }
    if config.access_key.trim().is_empty() {
        return Err(anyhow!("Volcengine access_key/X-Api-Key is required"));
    }
    if config.resource_id.trim().is_empty() {
        return Err(anyhow!("Volcengine resource_id is required"));
    }
    if config.stream_url.trim().is_empty() {
        return Err(anyhow!("Volcengine stream_url is required"));
    }
    Ok(())
}

fn build_ws_request(config: &AsrConfig, task_id: &str) -> Result<tungstenite::http::Request<()>> {
    use tungstenite::http::HeaderValue;

    let mut request = config
        .stream_url
        .as_str()
        .into_client_request()
        .context("Invalid Volcengine ASR websocket URL")?;
    let headers = request.headers_mut();
    if uses_legacy_auth(config) {
        headers.insert(
            "X-Api-App-Key",
            HeaderValue::from_str(config.app_key.trim())?,
        );
        headers.insert(
            "X-Api-Access-Key",
            HeaderValue::from_str(config.access_key.trim())?,
        );
    } else {
        headers.insert(
            "X-Api-Key",
            HeaderValue::from_str(config.access_key.trim())?,
        );
    }
    headers.insert(
        "X-Api-Resource-Id",
        HeaderValue::from_str(config.resource_id.trim())?,
    );
    headers.insert("X-Api-Request-Id", HeaderValue::from_str(task_id)?);
    headers.insert("X-Api-Connect-Id", HeaderValue::from_str(task_id)?);
    headers.insert("X-Api-Sequence", HeaderValue::from_static("-1"));
    Ok(request)
}

fn uses_legacy_auth(config: &AsrConfig) -> bool {
    match config.auth_mode.trim() {
        "legacy" | "old_console" => true,
        "api_key" | "new_console" | "x_api_key" => false,
        "" => !config.app_key.trim().is_empty(),
        _ => false,
    }
}

fn set_socket_read_timeout(socket: &mut VolcengineSocket, timeout: Option<Duration>) -> Result<()> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(timeout)?,
        MaybeTlsStream::NativeTls(stream) => stream.get_ref().set_read_timeout(timeout)?,
        #[allow(unreachable_patterns)]
        _ => {}
    }
    Ok(())
}

fn set_socket_write_timeout(
    socket: &mut VolcengineSocket,
    timeout: Option<Duration>,
) -> Result<()> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_write_timeout(timeout)?,
        MaybeTlsStream::NativeTls(stream) => stream.get_ref().set_write_timeout(timeout)?,
        #[allow(unreachable_patterns)]
        _ => {}
    }
    Ok(())
}

fn full_client_request(value: &Value) -> Result<Vec<u8>> {
    let payload = gzip(serde_json::to_vec(value)?)?;
    Ok(binary_message([0x11, 0x10, 0x11, 0x00], &payload))
}

fn audio_request(payload: &[u8], is_final: bool) -> Result<Vec<u8>> {
    let compressed = gzip(payload)?;
    let flags = if is_final { 0x02 } else { 0x00 };
    Ok(binary_message(
        [0x11, 0x20 | flags, 0x01, 0x00],
        &compressed,
    ))
}

fn binary_message(header: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn drain_available_responses(
    socket: &mut VolcengineSocket,
    response_state: &mut AsrResponseState,
    log_id: &Option<String>,
) -> Result<()> {
    loop {
        match read_server_response(socket, response_state, log_id) {
            Ok(_) => {}
            Err(err) if is_timeout_error(err.as_ref()) => break,
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn wait_for_final_response(
    socket: &mut VolcengineSocket,
    response_state: &mut AsrResponseState,
    log_id: &Option<String>,
) -> Result<()> {
    let started_at = Instant::now();
    loop {
        if started_at.elapsed() >= LIVE_FINAL_RESPONSE_TIMEOUT {
            bail!(
                "Volcengine live ASR websocket timed out, log_id={:?}",
                log_id
            );
        }

        match read_server_response(socket, response_state, log_id) {
            Ok(final_response) if final_response => return Ok(()),
            Ok(_) => {}
            Err(err) if is_timeout_error(err.as_ref()) => {}
            Err(err) => return Err(err),
        }
    }
}

fn read_server_response(
    socket: &mut VolcengineSocket,
    response_state: &mut AsrResponseState,
    log_id: &Option<String>,
) -> Result<bool> {
    let message = socket
        .read()
        .context("Failed to read Volcengine ASR websocket response")?;
    let Message::Binary(bytes) = message else {
        return Ok(false);
    };
    let response = parse_server_message(bytes.as_ref())?;
    match response {
        ServerMessage::Result { value, final_frame } => {
            Ok(response_state.apply_result(&value, final_frame))
        }
        ServerMessage::Error { code, message } => Err(anyhow!(
            "Volcengine ASR websocket error: code={}, message={}, log_id={:?}",
            code,
            message,
            log_id
        )),
    }
}

fn is_timeout_error(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(err);
    while let Some(error) = current {
        if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
            if matches!(
                io_error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) {
                return true;
            }
        }
        current = error.source();
    }
    false
}

fn gzip(payload: impl AsRef<[u8]>) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(payload.as_ref())?;
    Ok(encoder.finish()?)
}

fn gunzip(payload: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(payload);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

enum ServerMessage {
    Result { value: Value, final_frame: bool },
    Error { code: u32, message: String },
}

fn parse_server_message(bytes: &[u8]) -> Result<ServerMessage> {
    if bytes.len() < 8 {
        bail!("Volcengine ASR response frame is too short");
    }

    let header_size = ((bytes[0] & 0x0f) as usize) * 4;
    let message_type = bytes[1] >> 4;
    let flags = bytes[1] & 0x0f;
    let compression = bytes[2] & 0x0f;
    let mut offset = header_size;

    match message_type {
        0x09 => {
            if flags == 0x01 || flags == 0x03 {
                offset += 4;
            }
            if bytes.len() < offset + 4 {
                bail!("Volcengine ASR result frame is missing payload size");
            }
            let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into()?) as usize;
            offset += 4;
            if bytes.len() < offset + size {
                bail!("Volcengine ASR result frame payload is incomplete");
            }
            let payload = decode_payload(&bytes[offset..offset + size], compression)?;
            Ok(ServerMessage::Result {
                value: serde_json::from_slice(&payload)?,
                final_frame: flags == 0x02 || flags == 0x03,
            })
        }
        0x0f => {
            if bytes.len() < offset + 8 {
                bail!("Volcengine ASR error frame is incomplete");
            }
            let code = u32::from_be_bytes(bytes[offset..offset + 4].try_into()?);
            offset += 4;
            let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into()?) as usize;
            offset += 4;
            if bytes.len() < offset + size {
                bail!("Volcengine ASR error frame payload is incomplete");
            }
            let message = String::from_utf8_lossy(&bytes[offset..offset + size]).to_string();
            Ok(ServerMessage::Error { code, message })
        }
        other => bail!("Unsupported Volcengine ASR message type: {other:#x}"),
    }
}

fn decode_payload(payload: &[u8], compression: u8) -> Result<Vec<u8>> {
    match compression {
        0x00 => Ok(payload.to_vec()),
        0x01 => gunzip(payload),
        other => bail!("Unsupported Volcengine ASR compression method: {other:#x}"),
    }
}

fn normalized_language(language: &str) -> String {
    match language.trim() {
        "" | "zh" | "zh_CN" | "zh-CN" => "zh-CN".to_string(),
        other => other.to_string(),
    }
}

fn header_value(headers: &tungstenite::http::HeaderMap, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn extract_text(value: &Value) -> Result<String> {
    if let Some(text) = value
        .get("result")
        .and_then(|result| result.get("text"))
        .and_then(|text| text.as_str())
    {
        return Ok(text.trim().to_string());
    }

    if let Some(items) = value.get("result").and_then(|result| result.as_array()) {
        let text = items
            .iter()
            .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
            .collect::<Vec<_>>()
            .join("");
        if !text.trim().is_empty() {
            return Ok(text.trim().to_string());
        }
    }

    Err(anyhow!(
        "Volcengine ASR response did not contain result.text"
    ))
}

fn downmix_to_mono(samples: &[i16], channels: u16) -> Vec<i16> {
    let channels = channels.max(1) as usize;
    samples
        .chunks(channels)
        .map(|frame| {
            let sum: i32 = frame.iter().map(|sample| *sample as i32).sum();
            (sum / frame.len() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

fn interpolate_sample(samples: &[i16], position: f64) -> i16 {
    let left = position.floor() as usize;
    let right = (left + 1).min(samples.len().saturating_sub(1));
    let frac = position - left as f64;
    let value = samples[left] as f64 * (1.0 - frac) + samples[right] as f64 * frac;
    value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

fn pcm_bytes(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn normalized_wav_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
        bail!("Only 16-bit PCM WAV recordings are supported");
    }

    let samples = reader.samples::<i16>().collect::<Result<Vec<_>, _>>()?;
    let mono = downmix_to_mono(&samples, spec.channels);

    let resampled = if spec.sample_rate == TARGET_SAMPLE_RATE {
        mono
    } else {
        resample_linear(&mono, spec.sample_rate, TARGET_SAMPLE_RATE)
    };
    Ok(wav_bytes_from_mono(&resampled, TARGET_SAMPLE_RATE))
}

fn resample_linear(samples: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if samples.is_empty() || from_rate == 0 {
        return Vec::new();
    }

    let out_len = ((samples.len() as u64 * to_rate as u64) / from_rate as u64).max(1) as usize;
    let ratio = from_rate as f64 / to_rate as f64;
    (0..out_len)
        .map(|index| {
            let src = index as f64 * ratio;
            let left = src.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let frac = src - left as f64;
            let value = samples[left] as f64 * (1.0 - frac) + samples[right] as f64 * frac;
            value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
        })
        .collect()
}

fn wav_bytes_from_mono(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_object_result_text() {
        let value = json!({"result": {"text": "  你好，LDFC。 "}});
        assert_eq!(extract_text(&value).unwrap(), "你好，LDFC。");
    }

    #[test]
    fn extracts_array_result_text() {
        let value = json!({"result": [{"text": "你好，"}, {"text": "LDFC。"}]});
        assert_eq!(extract_text(&value).unwrap(), "你好，LDFC。");
    }

    #[test]
    fn builds_wav_header_for_16k_mono() {
        let wav = wav_bytes_from_mono(&[0, 1, -1], TARGET_SAMPLE_RATE);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
    }

    #[test]
    fn marks_final_audio_request() {
        let request = audio_request(&[1, 2, 3], true).unwrap();
        assert_eq!(request[1] & 0x0f, 0x02);
    }

    #[test]
    fn validates_new_console_api_key_without_app_key() {
        let config = test_asr_config("api_key", "", "new-api-key");
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn legacy_console_auth_requires_app_key() {
        let config = test_asr_config("legacy", "", "legacy-access-token");
        assert!(validate_config(&config)
            .unwrap_err()
            .to_string()
            .contains("app_key is required"));
    }

    #[test]
    fn builds_new_console_api_key_headers_when_app_key_is_empty() {
        let config = test_asr_config("api_key", "", "new-api-key");
        let request = build_ws_request(&config, "task-id").unwrap();
        let headers = request.headers();

        assert_eq!(headers.get("X-Api-Key").unwrap(), "new-api-key");
        assert!(headers.get("X-Api-App-Key").is_none());
        assert!(headers.get("X-Api-Access-Key").is_none());
        assert_eq!(
            headers.get("X-Api-Resource-Id").unwrap(),
            "volc.seedasr.sauc.duration"
        );
        assert_eq!(headers.get("X-Api-Sequence").unwrap(), "-1");
    }

    #[test]
    fn explicit_new_console_auth_ignores_leftover_app_key() {
        let config = test_asr_config("api_key", "legacy-app-id", "new-api-key");
        let request = build_ws_request(&config, "task-id").unwrap();
        let headers = request.headers();

        assert_eq!(headers.get("X-Api-Key").unwrap(), "new-api-key");
        assert!(headers.get("X-Api-App-Key").is_none());
        assert!(headers.get("X-Api-Access-Key").is_none());
    }

    #[test]
    fn builds_legacy_console_headers_when_app_key_is_present() {
        let config = test_asr_config("legacy", "legacy-app-id", "legacy-access-token");
        let request = build_ws_request(&config, "task-id").unwrap();
        let headers = request.headers();

        assert_eq!(headers.get("X-Api-App-Key").unwrap(), "legacy-app-id");
        assert_eq!(
            headers.get("X-Api-Access-Key").unwrap(),
            "legacy-access-token"
        );
        assert!(headers.get("X-Api-Key").is_none());
    }

    #[test]
    fn response_state_waits_for_protocol_final_frame() {
        let mut state = AsrResponseState::default();
        let partial = json!({"result": {"text": "中间结果"}});
        assert!(!state.apply_result(&partial, false));
        assert_eq!(state.best_text, "中间结果");

        let definite_utterance = json!({
            "result": {
                "text": "已定稿分句",
                "utterances": [{"text": "已定稿分句", "definite": true}]
            },
            "audio_info": {"duration": 1200}
        });
        assert!(!state.apply_result(&definite_utterance, false));
        assert_eq!(state.best_text, "已定稿分句");
        assert_eq!(state.service_duration_ms, Some(1200));

        let final_value = json!({
            "result": {
                "text": "最终结果",
                "utterances": [{"text": "最终结果", "definite": true}]
            },
            "audio_info": {"duration": 2400}
        });
        assert!(state.apply_result(&final_value, true));
        assert_eq!(state.best_text, "最终结果");
        assert_eq!(state.service_duration_ms, Some(2400));
    }

    fn test_asr_config(auth_mode: &str, app_key: &str, access_key: &str) -> AsrConfig {
        AsrConfig {
            provider: "volcengine".to_string(),
            auth_mode: auth_mode.to_string(),
            app_key: app_key.to_string(),
            access_key: access_key.to_string(),
            resource_id: "volc.seedasr.sauc.duration".to_string(),
            stream_url: "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream".to_string(),
            submit_url: String::new(),
            query_url: String::new(),
            language: "zh-CN".to_string(),
        }
    }

    #[test]
    fn retry_backoff_reaches_five_second_ceiling() {
        assert_eq!(live_retry_delay(1), Duration::from_millis(250));
        assert_eq!(live_retry_delay(2), Duration::from_millis(750));
        assert_eq!(live_retry_delay(3), Duration::from_millis(1_500));
        assert_eq!(live_retry_delay(4), Duration::from_secs(3));
        assert_eq!(live_retry_delay(5), Duration::from_secs(5));
        assert_eq!(live_retry_delay(50), Duration::from_secs(5));
    }

    #[test]
    fn live_audio_buffer_preserves_order_and_enforces_limit() {
        let first = AudioChunk {
            samples: vec![1, 2],
            sample_rate: 16_000,
            channels: 1,
        };
        let second = AudioChunk {
            samples: vec![3, 4],
            sample_rate: 16_000,
            channels: 1,
        };
        let mut buffered = Vec::new();
        let mut buffered_bytes = 0;
        let mut diagnostics = LiveAsrDiagnostics::default();

        buffer_live_audio_chunk_with_limit(
            first,
            &mut buffered,
            &mut buffered_bytes,
            &mut diagnostics,
            8,
        )
        .unwrap();
        buffer_live_audio_chunk_with_limit(
            second,
            &mut buffered,
            &mut buffered_bytes,
            &mut diagnostics,
            8,
        )
        .unwrap();

        assert_eq!(buffered[0].samples, vec![1, 2]);
        assert_eq!(buffered[1].samples, vec![3, 4]);
        assert_eq!(diagnostics.peak_buffered_bytes, 8);
        assert!(buffer_live_audio_chunk_with_limit(
            AudioChunk {
                samples: vec![5],
                sample_rate: 16_000,
                channels: 1,
            },
            &mut buffered,
            &mut buffered_bytes,
            &mut diagnostics,
            8,
        )
        .is_err());
    }

    #[test]
    fn explicit_http_client_errors_are_not_retried() {
        let response = tungstenite::http::Response::builder()
            .status(401)
            .body(None)
            .unwrap();
        let failure = classify_handshake_error(tungstenite::HandshakeError::Failure(
            tungstenite::Error::Http(Box::new(response)),
        ));

        assert!(!failure.retryable);
        assert_eq!(failure.category, "http_client");
    }

    #[test]
    fn temporary_server_errors_are_retried() {
        let response = tungstenite::http::Response::builder()
            .status(503)
            .body(None)
            .unwrap();
        let failure = classify_handshake_error(tungstenite::HandshakeError::Failure(
            tungstenite::Error::Http(Box::new(response)),
        ));

        assert!(failure.retryable);
        assert_eq!(failure.category, "http_temporary");
    }

    #[test]
    fn streaming_converter_downmixes_and_resamples() {
        let mut converter = StreamingPcmConverter::default();
        let chunk = AudioChunk {
            samples: vec![100, 300, 200, 400, 300, 500, 400, 600],
            sample_rate: 32_000,
            channels: 2,
        };

        let first = converter.push_chunk(&chunk).unwrap();
        let tail = converter.finish();
        let all = first.into_iter().chain(tail).collect::<Vec<_>>();

        assert!(!all.is_empty());
        assert!(all.len() <= 4);
        assert!(all.iter().all(|sample| *sample >= 200));
    }

    #[test]
    fn streaming_converter_keeps_boundary_sample_between_chunks() {
        let mut converter = StreamingPcmConverter::default();
        let mut output = Vec::new();
        for _ in 0..4 {
            let chunk = AudioChunk {
                samples: vec![100; 320],
                sample_rate: 48_000,
                channels: 1,
            };
            output.extend(converter.push_chunk(&chunk).unwrap());
        }
        output.extend(converter.finish());

        assert!(!output.is_empty());
    }

    #[test]
    fn live_audio_framer_emits_200ms_pcm_frames() {
        let mut framer = LiveAudioFramer::default();
        let samples = vec![7; STREAM_FRAME_SAMPLES + 10];
        let frames = framer.push_samples(&samples);

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), STREAM_CHUNK_BYTES);
        assert_eq!(framer.finish().unwrap().len(), 20);
    }

    #[test]
    fn live_audio_finish_sends_partial_tail_after_delayed_frame() {
        let mut framer = LiveAudioFramer::default();
        let samples = vec![7; STREAM_FRAME_SAMPLES + 10];
        let frames = framer.push_samples(&samples);
        assert_eq!(frames.len(), 1);

        let outgoing = finish_live_audio_frames(Some(frames[0].clone()), &mut framer);

        assert_eq!(outgoing.len(), 2);
        assert_eq!(outgoing[0].0.len(), STREAM_CHUNK_BYTES);
        assert!(!outgoing[0].1);
        assert_eq!(outgoing[1].0.len(), 20);
        assert!(outgoing[1].1);
        assert!(framer.finish().is_none());
    }

    #[test]
    fn live_audio_finish_marks_single_available_frame_final() {
        let mut empty_framer = LiveAudioFramer::default();
        let outgoing =
            finish_live_audio_frames(Some(vec![7; STREAM_CHUNK_BYTES]), &mut empty_framer);

        assert_eq!(outgoing, vec![(vec![7; STREAM_CHUNK_BYTES], true)]);

        let mut partial_framer = LiveAudioFramer::default();
        let frames = partial_framer.push_samples(&[7; 10]);
        assert!(frames.is_empty());

        let outgoing = finish_live_audio_frames(None, &mut partial_framer);
        let expected_partial = [7i16; 10]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();

        assert_eq!(outgoing, vec![(expected_partial, true)]);
        assert!(partial_framer.finish().is_none());
    }

    #[test]
    fn live_audio_finish_returns_empty_without_audio() {
        let mut framer = LiveAudioFramer::default();

        assert!(finish_live_audio_frames(None, &mut framer).is_empty());
    }

    #[test]
    #[ignore]
    fn transcribes_live_audio_from_env() {
        let audio_path = std::env::var("LIGHTNING_SPEAKING_TEST_AUDIO")
            .expect("LIGHTNING_SPEAKING_TEST_AUDIO must point to a WAV file");
        let config = crate::config::ConfigStore::load().unwrap();
        let output = VolcengineFileAsr
            .transcribe(Path::new(&audio_path), &config.asr)
            .unwrap();
        eprintln!("ASR_TEXT={}", output.text);
        assert!(!output.text.trim().is_empty());
    }

    #[test]
    #[ignore]
    fn transcribes_streaming_audio_from_env() {
        let audio_path = std::env::var("LIGHTNING_SPEAKING_TEST_AUDIO")
            .expect("LIGHTNING_SPEAKING_TEST_AUDIO must point to a WAV file");
        let config = crate::config::ConfigStore::load().unwrap();
        let session = VolcengineLiveAsrSession::start(&config.asr).unwrap();
        let sender = session.audio_sender().unwrap();

        let mut reader = hound::WavReader::open(audio_path).unwrap();
        let spec = reader.spec();
        let samples = reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let samples_per_chunk = ((spec.sample_rate as usize / 10).max(1)) * spec.channels as usize;
        for chunk in samples.chunks(samples_per_chunk) {
            sender
                .send(AudioChunk {
                    samples: chunk.to_vec(),
                    sample_rate: spec.sample_rate,
                    channels: spec.channels,
                })
                .unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        drop(sender);

        let output = session.finish().output.unwrap();
        eprintln!("LIVE_ASR_TEXT={}", output.text);
        assert_eq!(output.provider, "volcengine_ws_live");
        assert!(!output.text.trim().is_empty());
    }
}
