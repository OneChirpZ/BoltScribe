use super::{
    audio_level_dbfs, has_audio_signal, trim_samples, AudioLevelMeter, AudioTrim,
    InitialAudioReadiness, RecordedAudio, RecorderCommand, RecorderController,
};
use crate::config::AudioConfig;
use chrono::Utc;
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
fn trim_samples_preserves_capture_start_and_only_trims_the_tail() {
    let samples: Vec<i16> = (0..64_000).map(|sample| sample as i16).collect();
    let expected = samples[..44_800].to_vec();
    let (trimmed, start_ms) = trim_samples(
        samples,
        16_000,
        1,
        AudioTrim {
            last_activity_ms: 2_000,
        },
    );
    assert_eq!(start_ms, 0);
    assert_eq!(trimmed, expected);
}

#[test]
fn trim_samples_keeps_the_full_capture_when_tail_is_out_of_bounds() {
    let samples: Vec<i16> = (0..32_000).map(|sample| sample as i16).collect();
    let expected = samples.clone();
    let (trimmed, start_ms) = trim_samples(
        samples,
        16_000,
        1,
        AudioTrim {
            last_activity_ms: 10_000,
        },
    );
    assert_eq!(start_ms, 0);
    assert_eq!(trimmed, expected);
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
fn stopped_audio_level_meter_rejects_late_capture_callbacks() {
    let meter = AudioLevelMeter::new();
    meter.stop();

    meter.publish(-3.0);

    assert_eq!(meter.take_level(), None);
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
            trim: _,
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

#[test]
fn recorded_audio_discard_removes_a_new_file_and_tolerates_a_missing_file() {
    let path =
        std::env::temp_dir().join(format!("boltscribe-discard-{}.wav", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"test").unwrap();
    let recorded = RecordedAudio {
        id: "discard-test".to_string(),
        path: path.clone(),
        sample_rate: 16_000,
        channels: 1,
        started_at: Utc::now(),
        finished_at: Utc::now(),
        sample_count: 0,
    };

    recorded.clone().discard().unwrap();
    assert!(!path.exists());
    recorded.discard().unwrap();
}
