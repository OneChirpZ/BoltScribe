use crate::audio_devices;
use crate::config::{AudioConfig, ConfigStore};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use cpal::traits::{DeviceTrait, StreamTrait};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub type AudioSink = mpsc::Sender<AudioChunk>;
const EMPTY_AUDIO_LEVEL: f32 = f32::NEG_INFINITY;
const RECORDER_START_TIMEOUT: Duration = Duration::from_secs(8);
const INITIAL_AUDIO_READY_TIMEOUT: Duration = Duration::from_millis(600);
const INITIAL_AUDIO_READY_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone)]
pub struct AudioLevelMeter {
    level_bits: Arc<AtomicU32>,
    active: Arc<AtomicBool>,
}

impl AudioLevelMeter {
    pub fn new() -> Self {
        Self {
            level_bits: Arc::new(AtomicU32::new(EMPTY_AUDIO_LEVEL.to_bits())),
            active: Arc::new(AtomicBool::new(true)),
        }
    }

    fn publish(&self, level: f32) {
        let level = level.clamp(-96.0, 0.0);
        let mut current = self.level_bits.load(Ordering::Relaxed);
        while f32::from_bits(current) < level {
            match self.level_bits.compare_exchange_weak(
                current,
                level.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
    }

    pub fn take_level(&self) -> Option<f32> {
        let level = f32::from_bits(
            self.level_bits
                .swap(EMPTY_AUDIO_LEVEL.to_bits(), Ordering::Relaxed),
        );
        level.is_finite().then_some(level)
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.active.store(false, Ordering::Relaxed);
        self.level_bits
            .store(EMPTY_AUDIO_LEVEL.to_bits(), Ordering::Relaxed);
    }
}

impl Default for AudioLevelMeter {
    fn default() -> Self {
        Self::new()
    }
}

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

pub(crate) struct PendingRecordingStop {
    reply_receiver: mpsc::Receiver<Result<RecordedAudio>>,
}

enum RecorderCommand {
    Start {
        audio_sink: Option<AudioSink>,
        audio_level_meter: Option<AudioLevelMeter>,
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
    initial_audio_readiness: InitialAudioReadiness,
    spec: InputStreamSpec,
    sample_rate: u32,
    channels: u16,
}

#[derive(Clone, Default)]
struct InitialAudioReadiness {
    signal_seen: Arc<AtomicBool>,
}

impl InitialAudioReadiness {
    fn reset(&self) {
        self.signal_seen.store(false, Ordering::SeqCst);
    }

    fn observe(&self, samples: &[i16]) {
        if has_audio_signal(samples) {
            self.signal_seen.store(true, Ordering::SeqCst);
        }
    }

    fn wait(&self, unhealthy: &AtomicBool, timeout: Duration) -> Result<()> {
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
        audio_level_meter: Option<AudioLevelMeter>,
        audio_config: AudioConfig,
    ) -> Result<()> {
        self.start_with_config_timeout(
            audio_sink,
            audio_level_meter,
            audio_config,
            RECORDER_START_TIMEOUT,
        )
    }

    fn start_with_config_timeout(
        &self,
        audio_sink: Option<AudioSink>,
        audio_level_meter: Option<AudioLevelMeter>,
        audio_config: AudioConfig,
        timeout: Duration,
    ) -> Result<()> {
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Start {
                audio_sink,
                audio_level_meter,
                audio_config,
                reply: reply_sender,
            })
            .map_err(|_| anyhow!("Recorder worker is not running"))?;
        match reply_receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.queue_cancel().map_err(|err| {
                    anyhow!(
                        "Timed out after {}ms while starting the recorder; failed to queue cleanup: {err:#}",
                        timeout.as_millis()
                    )
                })?;
                Err(anyhow!(
                    "Timed out after {}ms while starting the recorder",
                    timeout.as_millis()
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(anyhow!("Recorder worker did not reply"))
            }
        }
    }

    fn queue_cancel(&self) -> Result<()> {
        let (reply_sender, _reply_receiver) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Cancel {
                reply: reply_sender,
            })
            .map_err(|_| anyhow!("Recorder worker is not running"))
    }

    pub(crate) fn begin_stop(&self, recordings_dir: PathBuf) -> Result<PendingRecordingStop> {
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Stop {
                recordings_dir,
                reply: reply_sender,
            })
            .map_err(|_| anyhow!("Recorder worker is not running"))?;
        Ok(PendingRecordingStop { reply_receiver })
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

impl PendingRecordingStop {
    pub(crate) fn wait(self) -> Result<RecordedAudio> {
        self.reply_receiver
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
    let mut stream = PreparedInputStream::new_and_start(&audio_config, None, None)?;
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
                audio_level_meter,
                audio_config,
                reply,
            } => {
                let result = if active {
                    Err(anyhow!("Recording is already active"))
                } else {
                    prepared.take();
                    match PreparedInputStream::new_and_start(
                        &audio_config,
                        audio_sink,
                        audio_level_meter,
                    ) {
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
    fn new_and_start(
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

fn has_audio_signal(samples: &[i16]) -> bool {
    samples.iter().any(|sample| *sample != 0)
}

fn audio_level_dbfs(samples: &[i16]) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::{
        audio_level_dbfs, has_audio_signal, AudioLevelMeter, InitialAudioReadiness,
        RecorderCommand, RecorderController,
    };
    use crate::config::AudioConfig;
    use std::sync::{atomic::AtomicBool, mpsc};
    use std::time::Duration;

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

    #[test]
    fn audio_level_reports_dbfs_and_increases_with_signal() {
        let quiet = audio_level_dbfs(&[64; 512]);
        let ambient_noise = audio_level_dbfs(&[256; 512]);
        let speech = audio_level_dbfs(&[4_000; 512]);
        let loud = audio_level_dbfs(&[20_000; 512]);

        assert_eq!(audio_level_dbfs(&[]), -96.0);
        assert_eq!(audio_level_dbfs(&[0; 512]), -96.0);
        assert!((-43.0..-41.0).contains(&ambient_noise));
        assert!(quiet < ambient_noise);
        assert!(ambient_noise < speech);
        assert!(speech < loud);
        assert!(loud <= 0.0);
    }

    #[test]
    fn audio_level_meter_keeps_peak_until_consumed() {
        let meter = AudioLevelMeter::new();
        meter.publish(-40.0);
        meter.publish(-20.0);
        meter.publish(-30.0);

        assert_eq!(meter.take_level(), Some(-20.0));
        assert_eq!(meter.take_level(), None);
        assert!(meter.is_active());
        meter.stop();
        assert!(!meter.is_active());
    }

    #[test]
    fn initial_audio_readiness_ignores_digital_silence() {
        let readiness = InitialAudioReadiness::default();
        let unhealthy = AtomicBool::new(false);

        readiness.observe(&[0, 0, 0]);
        let error = readiness.wait(&unhealthy, Duration::ZERO).unwrap_err();

        assert!(error.to_string().contains("No audio signal"));
    }

    #[test]
    fn initial_audio_readiness_accepts_the_first_nonzero_sample() {
        let readiness = InitialAudioReadiness::default();
        let unhealthy = AtomicBool::new(false);

        readiness.observe(&[0, 1, 0]);

        assert!(readiness.wait(&unhealthy, Duration::ZERO).is_ok());
    }

    #[test]
    fn initial_audio_readiness_rejects_a_failed_stream() {
        let readiness = InitialAudioReadiness::default();
        let unhealthy = AtomicBool::new(true);

        let error = readiness
            .wait(&unhealthy, Duration::from_secs(1))
            .unwrap_err();

        assert!(error.to_string().contains("Input stream failed"));
    }

    #[test]
    fn recorder_start_timeout_queues_cancel_before_a_future_retry() {
        let (sender, receiver) = mpsc::channel();
        let controller = RecorderController { sender };
        let worker = std::thread::spawn(move || {
            let start_reply = match receiver.recv().unwrap() {
                RecorderCommand::Start { reply, .. } => reply,
                _ => panic!("expected start command"),
            };
            std::thread::sleep(Duration::from_millis(30));
            let saw_cancel = matches!(
                receiver.recv_timeout(Duration::from_millis(100)),
                Ok(RecorderCommand::Cancel { .. })
            );
            drop(start_reply);
            saw_cancel
        });

        let error = controller
            .start_with_config_timeout(None, None, AudioConfig::default(), Duration::from_millis(5))
            .unwrap_err();

        assert!(error.to_string().contains("Timed out"));
        assert!(worker.join().unwrap());
    }

    #[test]
    fn begin_stop_queues_the_stop_before_returning_to_the_caller() {
        let (sender, receiver) = mpsc::channel();
        let controller = RecorderController { sender };
        let pending = controller
            .begin_stop(std::path::PathBuf::from("recordings"))
            .unwrap();

        let reply = match receiver.try_recv().unwrap() {
            RecorderCommand::Stop {
                recordings_dir,
                reply,
            } => {
                assert_eq!(recordings_dir, std::path::PathBuf::from("recordings"));
                reply
            }
            _ => panic!("expected stop command"),
        };
        drop(reply);

        assert!(pending.wait().is_err());
    }
}
