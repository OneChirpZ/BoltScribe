mod capture;
mod controller;
mod model;

pub(crate) use controller::PendingRecordingStop;
pub use controller::{request_microphone_permission, RecorderController};
pub use model::{AudioChunk, AudioLevelMeter, AudioSink, AudioTrim, RecordedAudio};

#[cfg(test)]
use capture::{audio_level_dbfs, has_audio_signal, trim_samples, InitialAudioReadiness};
#[cfg(test)]
use controller::RecorderCommand;

#[cfg(test)]
mod tests;
