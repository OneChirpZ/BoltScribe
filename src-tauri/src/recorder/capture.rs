use super::{AudioChunk, AudioLevelMeter, AudioSink, AudioTrim, RecordedAudio};
use crate::audio_devices;
use crate::config::AudioConfig;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use cpal::traits::{DeviceTrait, StreamTrait};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

const INITIAL_AUDIO_READY_TIMEOUT: Duration = Duration::from_millis(600);
const INITIAL_AUDIO_READY_POLL_INTERVAL: Duration = Duration::from_millis(10);
pub(super) struct PreparedInputStream {
    stream: cpal::Stream,
    capture: Arc<Mutex<CaptureState>>,
    unhealthy: Arc<AtomicBool>,
    initial_audio_readiness: InitialAudioReadiness,
    spec: InputStreamSpec,
    sample_rate: u32,
    channels: u16,
}

#[derive(Clone, Default)]
pub(super) struct InitialAudioReadiness {
    signal_seen: Arc<AtomicBool>,
}

impl InitialAudioReadiness {
    fn reset(&self) {
        self.signal_seen.store(false, Ordering::SeqCst);
    }

    pub(super) fn observe(&self, samples: &[i16]) {
        if has_audio_signal(samples) {
            self.signal_seen.store(true, Ordering::SeqCst);
        }
    }

    pub(super) fn wait(&self, unhealthy: &AtomicBool, timeout: Duration) -> Result<()> {
        let started_at = Instant::now();
        loop {
            if self.signal_seen.load(Ordering::SeqCst) {
                return Ok(());
            }
            if unhealthy.load(Ordering::SeqCst) {
                return Err(anyhow!(
                    "Input stream failed before delivering an audio signal"
                ));
            }
            if started_at.elapsed() >= timeout {
                return Err(anyhow!(
                    "No audio signal arrived within {}ms",
                    timeout.as_millis()
                ));
            }
            std::thread::sleep(INITIAL_AUDIO_READY_POLL_INTERVAL);
        }
    }
}

struct InputStreamSpec {
    device_id: String,
    device_name: String,
}

#[derive(Default)]
struct CaptureState {
    samples: Vec<i16>,
    audio_sink: Option<AudioSink>,
    audio_level_meter: Option<AudioLevelMeter>,
    started_at: Option<DateTime<Utc>>,
    recording: bool,
}

impl PreparedInputStream {
    pub(super) fn new_and_start(
        audio_config: &AudioConfig,
        audio_sink: Option<AudioSink>,
        audio_level_meter: Option<AudioLevelMeter>,
    ) -> Result<Self> {
        let candidates = audio_devices::input_device_candidates(audio_config)?;
        let mut failures = Vec::new();

        for candidate in candidates {
            let label = candidate.name.clone();
            match Self::from_candidate(candidate) {
                Ok(mut stream) => match stream.start(audio_sink.clone(), audio_level_meter.clone())
                {
                    Ok(()) => {
                        eprintln!(
                            "audio input selected: {} ({})",
                            stream.spec.device_name, stream.spec.device_id
                        );
                        return Ok(stream);
                    }
                    Err(err) => failures.push(format!("{label}: start failed: {err:#}")),
                },
                Err(err) => failures.push(format!("{label}: prepare failed: {err:#}")),
            }
        }

        Err(anyhow!(
            "No eligible input device could be started: {}",
            failures.join("; ")
        ))
    }

    fn from_candidate(candidate: audio_devices::AudioInputDeviceCandidate) -> Result<Self> {
        let audio_devices::AudioInputDeviceCandidate {
            device,
            id: device_id,
            name: device_name,
        } = candidate;
        let supported_config = device
            .default_input_config()
            .with_context(|| format!("Failed to get default input config for {device_name}"))?;
        let sample_rate = supported_config.sample_rate().0;
        let channels = supported_config.channels();
        let stream_config = supported_config.config();
        let spec = InputStreamSpec {
            device_id,
            device_name,
        };
        let capture = Arc::new(Mutex::new(CaptureState::default()));
        let unhealthy = Arc::new(AtomicBool::new(false));
        let initial_audio_readiness = InitialAudioReadiness::default();

        let unhealthy_for_error = unhealthy.clone();
        let err_fn = move |err| {
            unhealthy_for_error.store(true, Ordering::SeqCst);
            eprintln!("audio input stream error: {err}");
        };
        let stream = match supported_config.sample_format() {
            cpal::SampleFormat::F32 => {
                let writer_capture = capture.clone();
                let writer_readiness = initial_audio_readiness.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| {
                        let converted = f32_to_i16(data);
                        writer_readiness.observe(&converted);
                        write_samples(converted, &writer_capture, sample_rate, channels);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let writer_capture = capture.clone();
                let writer_readiness = initial_audio_readiness.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| {
                        writer_readiness.observe(data);
                        write_samples(data.to_vec(), &writer_capture, sample_rate, channels);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let writer_capture = capture.clone();
                let writer_readiness = initial_audio_readiness.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _| {
                        let converted = u16_to_i16(data);
                        writer_readiness.observe(&converted);
                        write_samples(converted, &writer_capture, sample_rate, channels);
                    },
                    err_fn,
                    None,
                )?
            }
            sample_format => {
                return Err(anyhow!(
                    "Unsupported input sample format: {sample_format:?}"
                ));
            }
        };

        Ok(Self {
            stream,
            capture,
            unhealthy,
            initial_audio_readiness,
            spec,
            sample_rate,
            channels,
        })
    }

    fn start(
        &mut self,
        audio_sink: Option<AudioSink>,
        audio_level_meter: Option<AudioLevelMeter>,
    ) -> Result<()> {
        self.unhealthy.store(false, Ordering::SeqCst);
        self.initial_audio_readiness.reset();
        {
            let mut capture = self
                .capture
                .lock()
                .map_err(|_| anyhow!("Failed to lock recorder capture state"))?;
            capture.start(audio_sink, audio_level_meter);
        }

        if let Err(err) = self.stream.play() {
            self.clear_capture(false);
            return Err(err).context("Failed to start microphone stream");
        }

        if let Err(err) = self
            .initial_audio_readiness
            .wait(&self.unhealthy, INITIAL_AUDIO_READY_TIMEOUT)
        {
            if let Err(pause_err) = self.stream.pause() {
                self.unhealthy.store(true, Ordering::SeqCst);
                eprintln!("audio input stream pause failed: {pause_err}");
            }
            self.clear_capture(false);
            return Err(err).with_context(|| {
                format!(
                    "Input device '{}' did not become ready",
                    self.spec.device_name
                )
            });
        }

        Ok(())
    }

    pub(super) fn stop(
        &mut self,
        recordings_dir: &Path,
        trim: Option<AudioTrim>,
    ) -> Result<RecordedAudio> {
        let (samples, started_at) = {
            let mut capture = self
                .capture
                .lock()
                .map_err(|_| anyhow!("Failed to lock recorder capture state"))?;
            capture.stop()?
        };

        let finished_at = Utc::now();
        if let Err(err) = self.stream.pause() {
            self.unhealthy.store(true, Ordering::SeqCst);
            eprintln!("audio input stream pause failed: {err}");
        }
        if samples.is_empty() || !has_audio_signal(&samples) {
            self.unhealthy.store(true, Ordering::SeqCst);
            return Err(anyhow!(
                "Input device '{}' captured no audio signal; add it to the blacklist or adjust the microphone priority and try again",
                self.spec.device_name
            ));
        }

        let (samples, trim_start_ms) = match trim {
            Some(trim) => trim_samples(samples, self.sample_rate, self.channels, trim),
            None => (samples, 0),
        };

        std::fs::create_dir_all(recordings_dir)
            .with_context(|| format!("Failed to create {}", recordings_dir.display()))?;
        let id = Uuid::new_v4().to_string();
        let file_name = format!("{}_{}.wav", finished_at.timestamp(), id);
        let path = recordings_dir.join(file_name);

        let spec = hound::WavSpec {
            channels: self.channels,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec)
            .with_context(|| format!("Failed to create {}", path.display()))?;
        for sample in &samples {
            writer.write_sample(*sample)?;
        }
        writer.finalize()?;

        Ok(RecordedAudio {
            id,
            path,
            sample_rate: self.sample_rate,
            channels: self.channels,
            started_at: started_at + chrono::Duration::milliseconds(trim_start_ms as i64),
            finished_at,
            sample_count: samples.len(),
        })
    }

    pub(super) fn cancel(&mut self) {
        self.clear_capture(true);
        if let Err(err) = self.stream.pause() {
            self.unhealthy.store(true, Ordering::SeqCst);
            eprintln!("audio input stream pause failed: {err}");
        }
    }

    fn clear_capture(&self, stop_meter: bool) {
        if let Ok(mut capture) = self.capture.lock() {
            capture.cancel(stop_meter);
        }
    }
}

pub(super) fn trim_samples(
    samples: Vec<i16>,
    sample_rate: u32,
    channels: u16,
    trim: AudioTrim,
) -> (Vec<i16>, u64) {
    if sample_rate == 0 || channels == 0 || samples.is_empty() {
        return (samples, 0);
    }
    let total_frames = samples.len() / channels as usize;
    let total_ms = (total_frames as u64 * 1000) / sample_rate as u64;
    let start_ms = 0;
    let end_ms = trim
        .last_activity_ms
        .saturating_add(800)
        .min(total_ms)
        .max(start_ms);
    let end_frame = ((end_ms as u128 * sample_rate as u128) / 1000) as usize;
    let start = 0;
    let mut end = (end_frame * channels as usize).clamp(start, samples.len());
    if end == start && start < samples.len() {
        end = (start + channels as usize).min(samples.len());
    }
    (samples[start..end].to_vec(), start_ms)
}

impl CaptureState {
    fn start(&mut self, audio_sink: Option<AudioSink>, audio_level_meter: Option<AudioLevelMeter>) {
        self.samples.clear();
        self.audio_sink = audio_sink;
        self.audio_level_meter = audio_level_meter;
        self.started_at = Some(Utc::now());
        self.recording = true;
    }

    fn stop(&mut self) -> Result<(Vec<i16>, DateTime<Utc>)> {
        self.recording = false;
        self.audio_sink = None;
        if let Some(meter) = self.audio_level_meter.take() {
            meter.stop();
        }
        let started_at = self
            .started_at
            .take()
            .ok_or_else(|| anyhow!("Recording start time is missing"))?;
        Ok((std::mem::take(&mut self.samples), started_at))
    }

    fn cancel(&mut self, stop_meter: bool) {
        self.recording = false;
        self.audio_sink = None;
        if let Some(meter) = self.audio_level_meter.take() {
            if stop_meter {
                meter.stop();
            }
        }
        self.started_at = None;
        self.samples.clear();
    }
}

fn write_samples(
    data: Vec<i16>,
    capture: &Arc<Mutex<CaptureState>>,
    sample_rate: u32,
    channels: u16,
) {
    let (audio_sink, audio_level_meter) = if let Ok(mut capture) = capture.lock() {
        if !capture.recording {
            return;
        }
        capture.samples.extend_from_slice(&data);
        (
            capture.audio_sink.clone(),
            capture.audio_level_meter.clone(),
        )
    } else {
        (None, None)
    };

    if let Some(meter) = audio_level_meter {
        meter.publish(audio_level_dbfs(&data));
    }

    if let Some(sender) = audio_sink {
        let _ = sender.send(AudioChunk {
            samples: data,
            sample_rate,
            channels,
        });
    }
}

fn f32_to_i16(data: &[f32]) -> Vec<i16> {
    data.iter()
        .map(|sample| {
            let sample = if sample.is_finite() { *sample } else { 0.0 };
            (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
        })
        .collect()
}

fn u16_to_i16(data: &[u16]) -> Vec<i16> {
    data.iter()
        .map(|sample| {
            let centered = *sample as i32 - 32768;
            centered.clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

pub(super) fn has_audio_signal(samples: &[i16]) -> bool {
    samples.iter().any(|sample| *sample != 0)
}

pub(super) fn audio_level_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return -96.0;
    }

    let mean_square = samples
        .iter()
        .map(|sample| {
            let normalized = *sample as f64 / i16::MAX as f64;
            normalized * normalized
        })
        .sum::<f64>()
        / samples.len() as f64;
    if mean_square <= f64::EPSILON {
        return -96.0;
    }

    (20.0 * mean_square.sqrt().log10()).clamp(-96.0, 0.0) as f32
}
