use super::capture::PreparedInputStream;
use super::{AudioLevelMeter, AudioSink, AudioTrim, RecordedAudio};
use crate::config::{AudioConfig, ConfigStore};
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
const RECORDER_START_TIMEOUT: Duration = Duration::from_secs(8);
#[derive(Clone)]
pub struct RecorderController {
    pub(super) sender: mpsc::Sender<RecorderCommand>,
}

pub(crate) struct PendingRecordingStop {
    reply_receiver: mpsc::Receiver<Result<RecordedAudio>>,
}

pub(super) enum RecorderCommand {
    Start {
        audio_sink: Option<AudioSink>,
        audio_level_meter: Option<AudioLevelMeter>,
        audio_config: AudioConfig,
        reply: mpsc::Sender<Result<()>>,
    },
    Stop {
        recordings_dir: PathBuf,
        trim: Option<AudioTrim>,
        reply: mpsc::Sender<Result<RecordedAudio>>,
    },
    Cancel {
        reply: mpsc::Sender<Result<()>>,
    },
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

    pub(super) fn start_with_config_timeout(
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
        self.begin_stop_with_trim(recordings_dir, None)
    }

    pub(crate) fn begin_stop_with_trim(
        &self,
        recordings_dir: PathBuf,
        trim: Option<AudioTrim>,
    ) -> Result<PendingRecordingStop> {
        let (reply_sender, reply_receiver) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Stop {
                recordings_dir,
                trim,
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
                trim,
                reply,
            } => {
                let result = if active {
                    active = false;
                    match prepared.as_mut() {
                        Some(stream) => stream.stop(&recordings_dir, trim),
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
