mod detector;
mod model;
mod pcm;
mod runtime;

pub use model::{GatedAsrResult, VadPhase, VadSnapshot};
pub use runtime::{VadGate, VadMonitorHandle};

#[cfg(test)]
use detector::{gate_profile, SpeechGate, FRAME_MS, FRAME_SAMPLES, WEBRTC_VAD_MODE};
#[cfg(test)]
use pcm::PcmConverter;
#[cfg(test)]
use runtime::{initial_silence_timed_out, PreActivationAudio};

#[cfg(test)]
mod tests;
