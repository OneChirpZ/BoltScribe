use super::LiveAsrActivity;
use crate::config::AsrConfig;
use anyhow::{anyhow, bail, Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{client::IntoClientRequest, Message};

pub(super) type VolcengineSocket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;
const TARGET_SAMPLE_RATE: u32 = 16_000;
const LIVE_DRAIN_TIMEOUT: Duration = Duration::from_millis(5);
const LIVE_FINAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(6);
const STREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(super) struct ConnectFailure {
    pub(super) category: &'static str,
    pub(super) retryable: bool,
    pub(super) error: anyhow::Error,
}

impl ConnectFailure {
    pub(super) fn retryable(category: &'static str, error: anyhow::Error) -> Self {
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

pub(super) fn connect_live_socket(
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

pub(super) fn classify_handshake_error(
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

pub(super) fn build_full_request(config: &AsrConfig, audio_format: &str) -> Value {
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

pub(super) struct AsrResponseState {
    pub(super) best_text: String,
    pub(super) service_duration_ms: Option<u64>,
    activity: Option<LiveAsrActivity>,
    last_definite_end_ms: Option<u64>,
}

impl AsrResponseState {
    pub(super) fn new(activity: Option<LiveAsrActivity>) -> Self {
        Self {
            best_text: String::new(),
            service_duration_ms: None,
            activity,
            last_definite_end_ms: None,
        }
    }

    pub(super) fn apply_result(&mut self, value: &Value, final_frame: bool) -> bool {
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

pub(super) fn validate_config(config: &AsrConfig) -> Result<()> {
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

pub(super) fn build_ws_request(
    config: &AsrConfig,
    task_id: &str,
) -> Result<tungstenite::http::Request<()>> {
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

pub(super) fn set_socket_read_timeout(
    socket: &mut VolcengineSocket,
    timeout: Option<Duration>,
) -> Result<()> {
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

pub(super) fn full_client_request(value: &Value) -> Result<Vec<u8>> {
    let payload = gzip(serde_json::to_vec(value)?)?;
    Ok(binary_message([0x11, 0x10, 0x11, 0x00], &payload))
}

pub(super) fn audio_request(payload: &[u8], is_final: bool) -> Result<Vec<u8>> {
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

pub(super) fn drain_available_responses(
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

pub(super) fn wait_for_final_response(
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

pub(super) fn is_timeout_error(err: &(dyn std::error::Error + 'static)) -> bool {
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

pub(super) enum ServerMessage {
    Result { value: Value, final_frame: bool },
    Error { code: u32, message: String },
}

pub(super) fn parse_server_message(bytes: &[u8]) -> Result<ServerMessage> {
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

pub(super) fn extract_text(value: &Value) -> Result<String> {
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
