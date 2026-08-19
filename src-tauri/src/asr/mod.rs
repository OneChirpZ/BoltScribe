mod file;
mod live;
mod pcm;
mod protocol;
mod types;

pub use file::VolcengineFileAsr;
pub use live::VolcengineLiveAsrSession;
pub use protocol::AsrCancellation;
pub use types::{
    AsrOutput, AsrProvider, LiveAsrActivity, LiveAsrDiagnostics, LiveAsrResult,
    TranscriptionOutcome,
};

#[cfg(test)]
use file::should_retry_file_connection;
#[cfg(test)]
use live::{
    buffer_live_audio_chunk_with_limit, finish_live_audio_frames,
    live_connection_attempts_exhausted, live_connection_retry_delay, recv_live_audio_chunk,
    should_start_file_race_after_live_failure, LiveAudioFramer, STREAM_CHUNK_BYTES,
    STREAM_FRAME_SAMPLES,
};
#[cfg(test)]
use pcm::{wav_bytes_from_mono, StreamingPcmConverter, TARGET_SAMPLE_RATE};
#[cfg(test)]
use protocol::{
    audio_request, build_ws_request, classify_handshake_error, classify_tls_error,
    connection_retry_delay, extract_text, fair_address_connect_timeout,
    file_connection_attempt_timeout, live_connection_attempt_timeout, validate_config,
    AsrResponseState,
};

#[cfg(test)]
mod tests;
