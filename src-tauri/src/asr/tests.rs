use super::*;
use crate::config::AsrConfig;
use crate::recorder::AudioChunk;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

#[test]
fn extracts_object_result_text() {
    let value = json!({"result": {"text": "  你好，LDFC。 "}});
    assert_eq!(extract_text(&value).unwrap(), "你好，LDFC。");
}

#[test]
fn extracts_array_result_text() {
    let value = json!({"result": [{"text": "你好，"}, {"text": "LDFC。"}]});
    assert_eq!(extract_text(&value).unwrap(), "你好，LDFC。");
}

#[test]
fn builds_wav_header_for_16k_mono() {
    let wav = wav_bytes_from_mono(&[0, 1, -1], TARGET_SAMPLE_RATE);
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
    assert_eq!(
        u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
        16_000
    );
}

#[test]
fn marks_final_audio_request() {
    let request = audio_request(&[1, 2, 3], true).unwrap();
    assert_eq!(request[1] & 0x0f, 0x02);
}

#[test]
fn validates_new_console_api_key_without_app_key() {
    let config = test_asr_config("api_key", "", "new-api-key");
    assert!(validate_config(&config).is_ok());
}

#[test]
fn legacy_console_auth_requires_app_key() {
    let config = test_asr_config("legacy", "", "legacy-access-token");
    assert!(validate_config(&config)
        .unwrap_err()
        .to_string()
        .contains("app_key is required"));
}

#[test]
fn builds_new_console_api_key_headers_when_app_key_is_empty() {
    let config = test_asr_config("api_key", "", "new-api-key");
    let request = build_ws_request(&config, "task-id").unwrap();
    let headers = request.headers();

    assert_eq!(headers.get("X-Api-Key").unwrap(), "new-api-key");
    assert!(headers.get("X-Api-App-Key").is_none());
    assert!(headers.get("X-Api-Access-Key").is_none());
    assert_eq!(
        headers.get("X-Api-Resource-Id").unwrap(),
        "volc.seedasr.sauc.duration"
    );
    assert_eq!(headers.get("X-Api-Sequence").unwrap(), "-1");
}

#[test]
fn explicit_new_console_auth_ignores_leftover_app_key() {
    let config = test_asr_config("api_key", "legacy-app-id", "new-api-key");
    let request = build_ws_request(&config, "task-id").unwrap();
    let headers = request.headers();

    assert_eq!(headers.get("X-Api-Key").unwrap(), "new-api-key");
    assert!(headers.get("X-Api-App-Key").is_none());
    assert!(headers.get("X-Api-Access-Key").is_none());
}

#[test]
fn builds_legacy_console_headers_when_app_key_is_present() {
    let config = test_asr_config("legacy", "legacy-app-id", "legacy-access-token");
    let request = build_ws_request(&config, "task-id").unwrap();
    let headers = request.headers();

    assert_eq!(headers.get("X-Api-App-Key").unwrap(), "legacy-app-id");
    assert_eq!(
        headers.get("X-Api-Access-Key").unwrap(),
        "legacy-access-token"
    );
    assert!(headers.get("X-Api-Key").is_none());
}

#[test]
fn response_state_waits_for_protocol_final_frame() {
    let mut state = AsrResponseState::default();
    let partial = json!({"result": {"text": "中间结果"}});
    assert!(!state.apply_result(&partial, false));
    assert_eq!(state.best_text, "中间结果");

    let definite_utterance = json!({
        "result": {
            "text": "已定稿分句",
            "utterances": [{"text": "已定稿分句", "definite": true}]
        },
        "audio_info": {"duration": 1200}
    });
    assert!(!state.apply_result(&definite_utterance, false));
    assert_eq!(state.best_text, "已定稿分句");
    assert_eq!(state.service_duration_ms, Some(1200));

    let final_value = json!({
        "result": {
            "text": "最终结果",
            "utterances": [{"text": "最终结果", "definite": true}]
        },
        "audio_info": {"duration": 2400}
    });
    assert!(state.apply_result(&final_value, true));
    assert_eq!(state.best_text, "最终结果");
    assert_eq!(state.service_duration_ms, Some(2400));
}

fn test_asr_config(auth_mode: &str, app_key: &str, access_key: &str) -> AsrConfig {
    AsrConfig {
        provider: "volcengine".to_string(),
        auth_mode: auth_mode.to_string(),
        app_key: app_key.to_string(),
        access_key: access_key.to_string(),
        resource_id: "volc.seedasr.sauc.duration".to_string(),
        stream_url: "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream".to_string(),
        submit_url: String::new(),
        query_url: String::new(),
        language: "zh-CN".to_string(),
    }
}

#[test]
fn retry_backoff_reaches_five_second_ceiling() {
    assert_eq!(live_retry_delay(1), Duration::from_millis(250));
    assert_eq!(live_retry_delay(2), Duration::from_millis(750));
    assert_eq!(live_retry_delay(3), Duration::from_millis(1_500));
    assert_eq!(live_retry_delay(4), Duration::from_secs(3));
    assert_eq!(live_retry_delay(5), Duration::from_secs(5));
    assert_eq!(live_retry_delay(50), Duration::from_secs(5));
}

#[test]
fn live_audio_buffer_preserves_order_and_enforces_limit() {
    let first = AudioChunk {
        samples: vec![1, 2],
        sample_rate: 16_000,
        channels: 1,
    };
    let second = AudioChunk {
        samples: vec![3, 4],
        sample_rate: 16_000,
        channels: 1,
    };
    let mut buffered = Vec::new();
    let mut buffered_bytes = 0;
    let mut diagnostics = LiveAsrDiagnostics::default();

    buffer_live_audio_chunk_with_limit(
        first,
        &mut buffered,
        &mut buffered_bytes,
        &mut diagnostics,
        8,
    )
    .unwrap();
    buffer_live_audio_chunk_with_limit(
        second,
        &mut buffered,
        &mut buffered_bytes,
        &mut diagnostics,
        8,
    )
    .unwrap();

    assert_eq!(buffered[0].samples, vec![1, 2]);
    assert_eq!(buffered[1].samples, vec![3, 4]);
    assert_eq!(diagnostics.peak_buffered_bytes, 8);
    assert!(buffer_live_audio_chunk_with_limit(
        AudioChunk {
            samples: vec![5],
            sample_rate: 16_000,
            channels: 1,
        },
        &mut buffered,
        &mut buffered_bytes,
        &mut diagnostics,
        8,
    )
    .is_err());
}

#[test]
fn explicit_http_client_errors_are_not_retried() {
    let response = tungstenite::http::Response::builder()
        .status(401)
        .body(None)
        .unwrap();
    let failure = classify_handshake_error(tungstenite::HandshakeError::Failure(
        tungstenite::Error::Http(Box::new(response)),
    ));

    assert!(!failure.retryable);
    assert_eq!(failure.category, "http_client");
}

#[test]
fn temporary_server_errors_are_retried() {
    let response = tungstenite::http::Response::builder()
        .status(503)
        .body(None)
        .unwrap();
    let failure = classify_handshake_error(tungstenite::HandshakeError::Failure(
        tungstenite::Error::Http(Box::new(response)),
    ));

    assert!(failure.retryable);
    assert_eq!(failure.category, "http_temporary");
}

#[test]
fn streaming_converter_downmixes_and_resamples() {
    let mut converter = StreamingPcmConverter::default();
    let chunk = AudioChunk {
        samples: vec![100, 300, 200, 400, 300, 500, 400, 600],
        sample_rate: 32_000,
        channels: 2,
    };

    let first = converter.push_chunk(&chunk).unwrap();
    let tail = converter.finish();
    let all = first.into_iter().chain(tail).collect::<Vec<_>>();

    assert!(!all.is_empty());
    assert!(all.len() <= 4);
    assert!(all.iter().all(|sample| *sample >= 200));
}

#[test]
fn streaming_converter_keeps_boundary_sample_between_chunks() {
    let mut converter = StreamingPcmConverter::default();
    let mut output = Vec::new();
    for _ in 0..4 {
        let chunk = AudioChunk {
            samples: vec![100; 320],
            sample_rate: 48_000,
            channels: 1,
        };
        output.extend(converter.push_chunk(&chunk).unwrap());
    }
    output.extend(converter.finish());

    assert!(!output.is_empty());
}

#[test]
fn live_audio_framer_emits_200ms_pcm_frames() {
    let mut framer = LiveAudioFramer::default();
    let samples = vec![7; STREAM_FRAME_SAMPLES + 10];
    let frames = framer.push_samples(&samples);

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].len(), STREAM_CHUNK_BYTES);
    assert_eq!(framer.finish().unwrap().len(), 20);
}

#[test]
fn live_audio_finish_sends_partial_tail_after_delayed_frame() {
    let mut framer = LiveAudioFramer::default();
    let samples = vec![7; STREAM_FRAME_SAMPLES + 10];
    let frames = framer.push_samples(&samples);
    assert_eq!(frames.len(), 1);

    let outgoing = finish_live_audio_frames(Some(frames[0].clone()), &mut framer);

    assert_eq!(outgoing.len(), 2);
    assert_eq!(outgoing[0].0.len(), STREAM_CHUNK_BYTES);
    assert!(!outgoing[0].1);
    assert_eq!(outgoing[1].0.len(), 20);
    assert!(outgoing[1].1);
    assert!(framer.finish().is_none());
}

#[test]
fn live_audio_finish_marks_single_available_frame_final() {
    let mut empty_framer = LiveAudioFramer::default();
    let outgoing = finish_live_audio_frames(Some(vec![7; STREAM_CHUNK_BYTES]), &mut empty_framer);

    assert_eq!(outgoing, vec![(vec![7; STREAM_CHUNK_BYTES], true)]);

    let mut partial_framer = LiveAudioFramer::default();
    let frames = partial_framer.push_samples(&[7; 10]);
    assert!(frames.is_empty());

    let outgoing = finish_live_audio_frames(None, &mut partial_framer);
    let expected_partial = [7i16; 10]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();

    assert_eq!(outgoing, vec![(expected_partial, true)]);
    assert!(partial_framer.finish().is_none());
}

#[test]
fn live_audio_finish_returns_empty_without_audio() {
    let mut framer = LiveAudioFramer::default();

    assert!(finish_live_audio_frames(None, &mut framer).is_empty());
}

#[test]
#[ignore]
fn transcribes_live_audio_from_env() {
    let audio_path = std::env::var("LIGHTNING_SPEAKING_TEST_AUDIO")
        .expect("LIGHTNING_SPEAKING_TEST_AUDIO must point to a WAV file");
    let config = crate::config::ConfigStore::load().unwrap();
    let output = VolcengineFileAsr
        .transcribe(Path::new(&audio_path), &config.asr)
        .unwrap();
    let TranscriptionOutcome::Text(output) = output else {
        panic!("expected transcribed text");
    };
    eprintln!("ASR_TEXT={}", output.text);
    assert!(!output.text.trim().is_empty());
}

#[test]
#[ignore]
fn transcribes_streaming_audio_from_env() {
    let audio_path = std::env::var("LIGHTNING_SPEAKING_TEST_AUDIO")
        .expect("LIGHTNING_SPEAKING_TEST_AUDIO must point to a WAV file");
    let config = crate::config::ConfigStore::load().unwrap();
    let session = VolcengineLiveAsrSession::start(&config.asr).unwrap();
    let sender = session.audio_sender().unwrap();

    let mut reader = hound::WavReader::open(audio_path).unwrap();
    let spec = reader.spec();
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let samples_per_chunk = ((spec.sample_rate as usize / 10).max(1)) * spec.channels as usize;
    for chunk in samples.chunks(samples_per_chunk) {
        sender
            .send(AudioChunk {
                samples: chunk.to_vec(),
                sample_rate: spec.sample_rate,
                channels: spec.channels,
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(sender);

    let output = session.finish().output.unwrap();
    let TranscriptionOutcome::Text(output) = output else {
        panic!("expected transcribed text");
    };
    eprintln!("LIVE_ASR_TEXT={}", output.text);
    assert_eq!(output.provider, "volcengine_ws_live");
    assert!(!output.text.trim().is_empty());
}
