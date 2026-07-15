use crate::audio_devices;
use crate::config::{AudioConfig, ConfigStore};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use cpal::traits::{DeviceTrait, StreamTrait};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use uuid::Uuid;

pub type AudioSink = mpsc::Sender<AudioChunk>;

#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone)]
pub struct RecorderController {
    sender: mpsc::Sender<RecorderCommand>,
}

enum RecorderCommand {
    Start {
        audio_sink: Option<AudioSink>,
        audio_config: AudioConfig,
        reply: mpsc::Sender<Result<()>>,
    },
    Stop {
        recordings_dir: PathBuf,
        reply: mpsc::Sender<Result<RecordedAudio>>,
    },
    Cancel {
        reply: mpsc::Sender<Result<()>>,
    },
}

struct PreparedInputStream {
    stream: cpal::Stream,
    capture: Arc<Mutex<CaptureState>>,
    unhealthy: Arc<AtomicBool>,
    spec: InputStreamSpec,
    sample_rate: u32,
    channels: u16,
}

struct InputStreamSpec {
    device_id: String,
    device_name: String,
}

#[derive(Default)]
struct CaptureState {
    samples: Vec<i16>,
    audio_sink: Option<AudioSink>,
    started_at: Option<DateTime<Utc>>,
    recording: bool,
}

#[derive(Debug, Clone)]
pub struct RecordedAudio {
    pub id: String,
    pub path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub sample_count: usize,
}

impl RecorderController {
    pub fn spawn() -> Self {
        let (sender, receiver) = mpsc::channel::<RecorderCommand>();
        std::thread::spawn(move || recorder_worker(receiver));
        Self { sender }
    }

    pub fn start_with_config(
        &self,
        audio_sink: Option<AudioSink>,
        audio_config: AudioConfig,
    ) -> Result<()> {
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Start {
                audio_sink,
                audio_config,
                reply: reply_sender,
            })
            .map_err(|_| anyhow!("Recorder worker is not running"))?;
        reply_receiver
            .recv()
            .map_err(|_| anyhow!("Recorder worker did not reply"))?
    }

    pub fn stop(&self, recordings_dir: PathBuf) -> Result<RecordedAudio> {
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Stop {
                recordings_dir,
                reply: reply_sender,
            })
            .map_err(|_| anyhow!("Recorder worker is not running"))?;
        reply_receiver
            .recv()
            .map_err(|_| anyhow!("Recorder worker did not reply"))?
    }

    pub fn cancel(&self) -> Result<()> {
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Cancel {
                reply: reply_sender,
            })
            .map_err(|_| anyhow!("Recorder worker is not running"))?;
        reply_receiver
            .recv()
            .map_err(|_| anyhow!("Recorder worker did not reply"))?
    }
}

impl Default for RecorderController {
    fn default() -> Self {
        Self::spawn()
    }
}

pub fn request_microphone_permission() -> Result<bool> {
    let audio_config = microphone_permission_audio_config();
    let mut stream = PreparedInputStream::new_and_start(&audio_config, None)?;
    std::thread::sleep(Duration::from_millis(300));
    stream.cancel();
    Ok(true)
}

fn microphone_permission_audio_config() -> AudioConfig {
    ConfigStore::load()
        .map(|config| config.audio)
        .unwrap_or_default()
}

fn recorder_worker(receiver: mpsc::Receiver<RecorderCommand>) {
    let mut prepared: Option<PreparedInputStream> = None;
    let mut active = false;
    while let Ok(command) = receiver.recv() {
        match command {
            RecorderCommand::Start {
                audio_sink,
                audio_config,
                reply,
            } => {
                let result = if active {
                    Err(anyhow!("Recording is already active"))
                } else {
                    prepared.take();
                    match PreparedInputStream::new_and_start(&audio_config, audio_sink) {
                        Ok(stream) => {
                            prepared = Some(stream);
                            active = true;
                            Ok(())
                        }
                        Err(err) => Err(err),
                    }
                };
                let _ = reply.send(result);
            }
            RecorderCommand::Stop {
                recordings_dir,
                reply,
            } => {
                let result = if active {
                    active = false;
                    match prepared.as_mut() {
                        Some(stream) => stream.stop(&recordings_dir),
                        None => Err(anyhow!("Recorder stream is not prepared")),
                    }
                } else {
                    Err(anyhow!("Recording is not active"))
                };
                let _ = reply.send(result);
            }
            RecorderCommand::Cancel { reply } => {
                if active {
                    if let Some(stream) = prepared.as_mut() {
                        stream.cancel();
                    }
                    active = false;
                }
                let _ = reply.send(Ok(()));
            }
        }
    }
}

impl PreparedInputStream {
    fn new_and_start(audio_config: &AudioConfig, audio_sink: Option<AudioSink>) -> Result<Self> {
        let candidates = audio_devices::input_device_candidates(audio_config)?;
        let mut failures = Vec::new();

        for candidate in candidates {
            let label = candidate.name.clone();
            match Self::from_candidate(candidate) {
                Ok(mut stream) => match stream.start(audio_sink.clone()) {
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

        let unhealthy_for_error = unhealthy.clone();
        let err_fn = move |err| {
            unhealthy_for_error.store(true, Ordering::SeqCst);
            eprintln!("audio input stream error: {err}");
        };
        let stream = match supported_config.sample_format() {
            cpal::SampleFormat::F32 => {
                let writer_capture = capture.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| {
                        let converted = f32_to_i16(data);
                        write_samples(converted, &writer_capture, sample_rate, channels);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let writer_capture = capture.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| {
                        write_samples(data.to_vec(), &writer_capture, sample_rate, channels);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let writer_capture = capture.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _| {
                        let converted = u16_to_i16(data);
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
            spec,
            sample_rate,
            channels,
        })
    }

    fn start(&mut self, audio_sink: Option<AudioSink>) -> Result<()> {
        {
            let mut capture = self
                .capture
                .lock()
                .map_err(|_| anyhow!("Failed to lock recorder capture state"))?;
            capture.start(audio_sink);
        }

        if let Err(err) = self.stream.play() {
            self.clear_capture();
            return Err(err).context("Failed to start microphone stream");
        }

        Ok(())
    }

    fn stop(&mut self, recordings_dir: &Path) -> Result<RecordedAudio> {
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
            started_at,
            finished_at,
            sample_count: samples.len(),
        })
    }

    fn cancel(&mut self) {
        self.clear_capture();
        if let Err(err) = self.stream.pause() {
            self.unhealthy.store(true, Ordering::SeqCst);
            eprintln!("audio input stream pause failed: {err}");
        }
    }

    fn clear_capture(&self) {
        if let Ok(mut capture) = self.capture.lock() {
            capture.cancel();
        }
    }
}

impl CaptureState {
    fn start(&mut self, audio_sink: Option<AudioSink>) {
        self.samples.clear();
        self.audio_sink = audio_sink;
        self.started_at = Some(Utc::now());
        self.recording = true;
    }

    fn stop(&mut self) -> Result<(Vec<i16>, DateTime<Utc>)> {
        self.recording = false;
        self.audio_sink = None;
        let started_at = self
            .started_at
            .take()
            .ok_or_else(|| anyhow!("Recording start time is missing"))?;
        Ok((std::mem::take(&mut self.samples), started_at))
    }

    fn cancel(&mut self) {
        self.recording = false;
        self.audio_sink = None;
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
    let audio_sink = if let Ok(mut capture) = capture.lock() {
        if !capture.recording {
            return;
        }
        capture.samples.extend_from_slice(&data);
        capture.audio_sink.clone()
    } else {
        None
    };

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

fn has_audio_signal(samples: &[i16]) -> bool {
    samples.iter().any(|sample| *sample != 0)
}

#[cfg(test)]
mod tests {
    use super::has_audio_signal;

    #[test]
    fn empty_and_bit_perfect_zero_samples_have_no_signal() {
        assert!(!has_audio_signal(&[]));
        assert!(!has_audio_signal(&[0, 0, 0, 0]));
    }

    #[test]
    fn any_nonzero_sample_counts_as_signal() {
        assert!(has_audio_signal(&[0, 1, 0]));
        assert!(has_audio_signal(&[0, -1, 0]));
    }
}
