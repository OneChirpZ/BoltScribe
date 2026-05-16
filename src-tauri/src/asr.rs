use crate::config::AsrConfig;
use crate::recorder::{AudioChunk, AudioSink};
use anyhow::{anyhow, bail, Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::mpsc;
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
const FINAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

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
    handle: Option<JoinHandle<Result<AsrOutput>>>,
}

impl VolcengineLiveAsrSession {
    pub fn start(config: &AsrConfig) -> Result<Self> {
        validate_config(config)?;

        let task_id = Uuid::new_v4().to_string();
        let config = config.clone();
        let (sender, receiver) = mpsc::channel::<AudioChunk>();
        let handle = std::thread::spawn(move || live_asr_worker(config, task_id, receiver));

        Ok(Self {
            sender: Some(sender),
            handle: Some(handle),
        })
    }

    pub fn audio_sender(&self) -> Result<AudioSink> {
        self.sender
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("Live ASR session is already finishing"))
    }

    pub fn finish(mut self) -> Result<AsrOutput> {
        self.sender.take();
        let handle = self
            .handle
            .take()
            .ok_or_else(|| anyhow!("Live ASR worker is not running"))?;
        handle
            .join()
            .map_err(|_| anyhow!("Live ASR worker panicked"))?
    }
}

impl AsrProvider for VolcengineFileAsr {
    fn transcribe(&self, audio_path: &Path, config: &AsrConfig) -> Result<AsrOutput> {
        validate_config(config)?;

        let task_id = Uuid::new_v4().to_string();
        let audio = normalized_wav_bytes(audio_path)
            .with_context(|| format!("Failed to prepare {}", audio_path.display()))?;
        let request = build_ws_request(config, &task_id)?;
        let (mut socket, response) =
            tungstenite::connect(request).context("Failed to connect Volcengine ASR websocket")?;
        let log_id = header_value(response.headers(), "x-tt-logid");

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

        let started_at = Instant::now();
        let mut response_state = AsrResponseState::default();
        loop {
            if started_at.elapsed() > Duration::from_secs(120) {
                bail!("Volcengine ASR websocket timed out");
            }

            let message = socket
                .read()
                .context("Failed to read Volcengine ASR websocket response")?;
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
    task_id: String,
    receiver: mpsc::Receiver<AudioChunk>,
) -> Result<AsrOutput> {
    let request = build_ws_request(&config, &task_id)?;
    let (mut socket, response) =
        tungstenite::connect(request).context("Failed to connect Volcengine live ASR websocket")?;
    let log_id = header_value(response.headers(), "x-tt-logid");
    set_socket_read_timeout(&mut socket, Some(LIVE_DRAIN_TIMEOUT))?;

    let full_request = build_full_request(&config, "pcm");
    socket
        .send(Message::Binary(full_client_request(&full_request)?.into()))
        .context("Failed to send Volcengine live ASR request metadata")?;

    let mut converter = StreamingPcmConverter::default();
    let mut framer = LiveAudioFramer::default();
    let mut delayed_frame: Option<Vec<u8>> = None;
    let mut response_state = AsrResponseState::default();

    for chunk in receiver {
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

    let final_frame = delayed_frame.or_else(|| framer.finish());
    let Some(final_frame) = final_frame else {
        bail!("No audio samples captured for live ASR");
    };

    socket
        .send(Message::Binary(audio_request(&final_frame, true)?.into()))
        .context("Failed to send Volcengine live ASR final audio chunk")?;
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
        "user": { "uid": config.app_key },
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

#[derive(Default)]
struct AsrResponseState {
    best_text: String,
    service_duration_ms: Option<u64>,
}

impl AsrResponseState {
    fn apply_result(&mut self, value: &Value, final_frame: bool) -> bool {
        if let Ok(text) = extract_text(value) {
            if !text.trim().is_empty() {
                self.best_text = text;
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

fn validate_config(config: &AsrConfig) -> Result<()> {
    if config.app_key.trim().is_empty() {
        return Err(anyhow!("Volcengine app_key is required"));
    }
    if config.access_key.trim().is_empty() {
        return Err(anyhow!("Volcengine access_key is required"));
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
    headers.insert("X-Api-App-Key", HeaderValue::from_str(&config.app_key)?);
    headers.insert(
        "X-Api-Access-Key",
        HeaderValue::from_str(&config.access_key)?,
    );
    headers.insert(
        "X-Api-Resource-Id",
        HeaderValue::from_str(&config.resource_id)?,
    );
    headers.insert("X-Api-Request-Id", HeaderValue::from_str(task_id)?);
    headers.insert("X-Api-Connect-Id", HeaderValue::from_str(task_id)?);
    headers.insert("X-Api-Sequence", HeaderValue::from_static("-1"));
    Ok(request)
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
        if started_at.elapsed() > FINAL_RESPONSE_TIMEOUT {
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
        let value = json!({"result": {"text": "  你好，Acme。 "}});
        assert_eq!(extract_text(&value).unwrap(), "你好，Acme。");
    }

    #[test]
    fn extracts_array_result_text() {
        let value = json!({"result": [{"text": "你好，"}, {"text": "Acme。"}]});
        assert_eq!(extract_text(&value).unwrap(), "你好，Acme。");
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

        let output = session.finish().unwrap();
        eprintln!("LIVE_ASR_TEXT={}", output.text);
        assert_eq!(output.provider, "volcengine_ws_live");
        assert!(!output.text.trim().is_empty());
    }
}
