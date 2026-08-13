mod file;
mod live;
mod pcm;
mod protocol;
mod types;

pub use file::VolcengineFileAsr;
pub use live::VolcengineLiveAsrSession;
pub use types::{
    AsrOutput, AsrProvider, LiveAsrActivity, LiveAsrDiagnostics, LiveAsrResult,
    TranscriptionOutcome,
};

#[cfg(test)]
use live::{
    buffer_live_audio_chunk_with_limit, finish_live_audio_frames, live_retry_delay,
    LiveAudioFramer, STREAM_CHUNK_BYTES, STREAM_FRAME_SAMPLES,
};
#[cfg(test)]
use pcm::{wav_bytes_from_mono, StreamingPcmConverter, TARGET_SAMPLE_RATE};
#[cfg(test)]
use protocol::{
    audio_request, build_ws_request, classify_handshake_error, extract_text, validate_config,
    AsrResponseState,
};

#[cfg(test)]
mod tests;
