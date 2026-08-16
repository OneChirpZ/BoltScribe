use super::pcm::{pcm_bytes, StreamingPcmConverter, TARGET_SAMPLE_RATE};
use super::protocol::{
    audio_request, build_full_request, build_ws_request, connection_retry_delay,
    drain_available_responses, full_client_request, live_connection_attempt_timeout,
    set_socket_read_timeout, spawn_connect_socket_attempt, validate_config,
    wait_for_final_response, AsrCancellation, AsrResponseState, ConnectFailure, VolcengineSocket,
};
use super::{AsrOutput, LiveAsrActivity, LiveAsrDiagnostics, LiveAsrResult, TranscriptionOutcome};
use crate::config::AsrConfig;
use crate::recorder::{AudioChunk, AudioSink};
use anyhow::{anyhow, bail, Context, Result};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tungstenite::Message;
use uuid::Uuid;

#[cfg(test)]
pub(super) const STREAM_CHUNK_BYTES: usize = 6_400;
pub(super) const STREAM_FRAME_SAMPLES: usize = (TARGET_SAMPLE_RATE as usize) / 5;
const FINAL_READ_TIMEOUT: Duration = Duration::from_millis(250);
const LIVE_AUDIO_BUFFER_LIMIT: usize = 128 * 1024 * 1024;

pub struct VolcengineLiveAsrSession {
    sender: Option<AudioSink>,
    handle: Option<JoinHandle<LiveAsrResult>>,
    activity: Option<LiveAsrActivity>,
    cancellation: AsrCancellation,
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
        let cancellation = AsrCancellation::new();
        let worker_cancellation = cancellation.clone();
        let handle = std::thread::spawn(move || {
            live_asr_worker(config, receiver, worker_activity, worker_cancellation)
        });

        Ok(Self {
            sender: Some(sender),
            handle: Some(handle),
            activity,
            cancellation,
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

    pub fn cancellation(&self) -> AsrCancellation {
        self.cancellation.clone()
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

impl Drop for VolcengineLiveAsrSession {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.sender.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn live_asr_worker(
    config: AsrConfig,
    receiver: mpsc::Receiver<AudioChunk>,
    activity: Option<LiveAsrActivity>,
    cancellation: AsrCancellation,
) -> LiveAsrResult {
    let started_at = Instant::now();
    let mut diagnostics = LiveAsrDiagnostics::default();
    let mut buffered = Vec::new();
    let mut buffered_bytes = 0usize;
    let mut recording_finished = false;
    let mut post_stop_attempts = 0u32;
    let connect_timeout = live_connection_attempt_timeout(&config);

    loop {
        if cancellation.is_cancelled() {
            mark_live_asr_cancelled(&mut diagnostics);
            return LiveAsrResult {
                output: Err(anyhow!("ASR operation cancelled")),
                diagnostics,
            };
        }
        if recording_finished {
            post_stop_attempts = post_stop_attempts.saturating_add(1);
        }
        diagnostics.connection_attempts = diagnostics.connection_attempts.saturating_add(1);
        let task_id = Uuid::new_v4().to_string();
        let (attempt_receiver, attempt_cancellation) =
            spawn_connect_socket_attempt(&config, &task_id, connect_timeout, cancellation.clone());
        let attempt_started_at = Instant::now();

        let connection = loop {
            if cancellation.is_cancelled() {
                attempt_cancellation.cancel();
                break Err(ConnectFailure::retryable(
                    "cancelled",
                    anyhow!("ASR operation cancelled"),
                ));
            }
            match attempt_receiver.try_recv() {
                Ok(result) => break result,
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(ConnectFailure::retryable(
                        "connection_worker",
                        anyhow!("Live ASR connection worker exited unexpectedly"),
                    ));
                }
                Err(mpsc::TryRecvError::Empty)
                    if attempt_started_at.elapsed() >= connect_timeout =>
                {
                    attempt_cancellation.cancel();
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
                if let Err(error) = cancellation.register_websocket(&socket) {
                    mark_live_asr_cancelled(&mut diagnostics);
                    return LiveAsrResult {
                        output: Err(error),
                        diagnostics,
                    };
                }
                diagnostics.first_connected_after_ms =
                    Some(started_at.elapsed().as_millis() as u64);
                let output = stream_live_audio(
                    &config,
                    ConnectedLiveSocket {
                        task_id,
                        socket,
                        log_id,
                    },
                    buffered,
                    receiver,
                    activity.clone(),
                    &cancellation,
                );
                cancellation.clear_socket();
                if cancellation.is_cancelled() {
                    mark_live_asr_cancelled(&mut diagnostics);
                } else if output.is_err() {
                    diagnostics.last_error_category = Some("stream_interrupted".to_string());
                    diagnostics.fallback_reason = Some("stream_interrupted".to_string());
                }
                return LiveAsrResult {
                    output,
                    diagnostics,
                };
            }
            Err(failure) => {
                if cancellation.is_cancelled() {
                    mark_live_asr_cancelled(&mut diagnostics);
                    return LiveAsrResult {
                        output: Err(anyhow!("ASR operation cancelled")),
                        diagnostics,
                    };
                }
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

                if live_connection_attempts_exhausted(
                    recording_finished,
                    post_stop_attempts,
                    config.connection_retry.live_post_stop_max_attempts,
                ) {
                    diagnostics.fallback_reason = Some("connection_attempts_exhausted".to_string());
                    return LiveAsrResult {
                        output: Err(failure.error),
                        diagnostics,
                    };
                }

                let delay = connection_retry_delay(
                    &config.connection_retry,
                    diagnostics.connection_attempts,
                );
                if let Err(error) = buffer_live_audio_for_delay(
                    &receiver,
                    delay,
                    &mut buffered,
                    &mut buffered_bytes,
                    &mut diagnostics,
                    &mut recording_finished,
                    &cancellation,
                ) {
                    if cancellation.is_cancelled() {
                        mark_live_asr_cancelled(&mut diagnostics);
                    } else {
                        diagnostics.fallback_reason = Some("buffer_limit_exceeded".to_string());
                    }
                    return LiveAsrResult {
                        output: Err(error),
                        diagnostics,
                    };
                }
            }
        }
    }
}

fn mark_live_asr_cancelled(diagnostics: &mut LiveAsrDiagnostics) {
    diagnostics
        .last_error_category
        .get_or_insert_with(|| "cancelled".to_string());
    diagnostics.fallback_reason = Some("cancelled".to_string());
}

pub(super) fn live_connection_attempts_exhausted(
    recording_finished: bool,
    post_stop_attempts: u32,
    max_attempts: u32,
) -> bool {
    recording_finished && post_stop_attempts >= max_attempts
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

pub(super) fn buffer_live_audio_chunk_with_limit(
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
    cancellation: &AsrCancellation,
) -> Result<()> {
    let deadline = Instant::now() + delay;
    loop {
        cancellation.check()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if *recording_finished {
            if !cancellation.wait(remaining) {
                bail!("ASR operation cancelled");
            }
            return Ok(());
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(20))) {
            Ok(chunk) => buffer_live_audio_chunk(chunk, buffered, buffered_bytes, diagnostics)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => *recording_finished = true,
        }
    }
    Ok(())
}

struct ConnectedLiveSocket {
    task_id: String,
    socket: VolcengineSocket,
    log_id: Option<String>,
}

fn stream_live_audio(
    config: &AsrConfig,
    connection: ConnectedLiveSocket,
    buffered: Vec<AudioChunk>,
    receiver: mpsc::Receiver<AudioChunk>,
    activity: Option<LiveAsrActivity>,
    cancellation: &AsrCancellation,
) -> Result<TranscriptionOutcome> {
    let ConnectedLiveSocket {
        task_id,
        mut socket,
        log_id,
    } = connection;
    let full_request = build_full_request(config, "pcm");
    cancellation.check()?;
    socket
        .send(Message::Binary(full_client_request(&full_request)?.into()))
        .context("Failed to send Volcengine live ASR request metadata")?;

    let mut converter = StreamingPcmConverter::default();
    let mut framer = LiveAudioFramer::default();
    let mut delayed_frame: Option<Vec<u8>> = None;
    let mut response_state = AsrResponseState::new(activity);

    {
        let mut process_chunk = |chunk: AudioChunk| -> Result<()> {
            cancellation.check()?;
            let pcm = converter.push_chunk(&chunk)?;
            for frame in framer.push_samples(&pcm) {
                if let Some(frame_to_send) = delayed_frame.replace(frame) {
                    cancellation.check()?;
                    socket
                        .send(Message::Binary(
                            audio_request(&frame_to_send, false)?.into(),
                        ))
                        .context("Failed to send Volcengine live ASR audio chunk")?;
                    drain_available_responses(
                        &mut socket,
                        &mut response_state,
                        &log_id,
                        cancellation,
                    )?;
                }
            }
            Ok(())
        };

        for chunk in buffered {
            process_chunk(chunk)?;
        }
        while let Some(chunk) = recv_live_audio_chunk(&receiver, cancellation)? {
            process_chunk(chunk)?;
        }
    }

    let tail = converter.finish();
    for frame in framer.push_samples(&tail) {
        if let Some(frame_to_send) = delayed_frame.replace(frame) {
            cancellation.check()?;
            socket
                .send(Message::Binary(
                    audio_request(&frame_to_send, false)?.into(),
                ))
                .context("Failed to send Volcengine live ASR audio chunk")?;
            drain_available_responses(&mut socket, &mut response_state, &log_id, cancellation)?;
        }
    }

    let final_frames = finish_live_audio_frames(delayed_frame, &mut framer);
    if final_frames.is_empty() {
        bail!("No audio samples captured for live ASR");
    }

    for (frame, is_final) in final_frames {
        cancellation.check()?;
        let context = if is_final {
            "Failed to send Volcengine live ASR final audio chunk"
        } else {
            "Failed to send Volcengine live ASR audio chunk"
        };
        socket
            .send(Message::Binary(audio_request(&frame, is_final)?.into()))
            .context(context)?;
        if !is_final {
            drain_available_responses(&mut socket, &mut response_state, &log_id, cancellation)?;
        }
    }
    set_socket_read_timeout(&mut socket, Some(FINAL_READ_TIMEOUT))?;
    wait_for_final_response(&mut socket, &mut response_state, &log_id, cancellation)?;

    if response_state.best_text.trim().is_empty() {
        return Ok(TranscriptionOutcome::NoSpeech);
    }

    Ok(TranscriptionOutcome::Text(AsrOutput {
        text: response_state.best_text,
        provider: "volcengine_ws_live".to_string(),
        task_id: Some(task_id),
        duration_ms: response_state.service_duration_ms,
    }))
}

pub(super) fn recv_live_audio_chunk(
    receiver: &mpsc::Receiver<AudioChunk>,
    cancellation: &AsrCancellation,
) -> Result<Option<AudioChunk>> {
    loop {
        cancellation.check()?;
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => return Ok(Some(chunk)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
        }
    }
}

#[derive(Default)]
pub(super) struct LiveAudioFramer {
    pending_samples: Vec<i16>,
}

impl LiveAudioFramer {
    pub(super) fn push_samples(&mut self, samples: &[i16]) -> Vec<Vec<u8>> {
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

    pub(super) fn finish(&mut self) -> Option<Vec<u8>> {
        if self.pending_samples.is_empty() {
            return None;
        }
        let frame = pcm_bytes(&self.pending_samples);
        self.pending_samples.clear();
        Some(frame)
    }
}

pub(super) fn finish_live_audio_frames(
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
