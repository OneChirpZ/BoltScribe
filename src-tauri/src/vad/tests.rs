use super::{
    gate_profile, initial_silence_timed_out, PcmConverter, PreActivationAudio, SpeechGate,
    FRAME_MS, FRAME_SAMPLES,
};
use crate::recorder::AudioChunk;
use std::time::Duration;

const TEST_NOISE_WINDOW_MS: u32 = 600;

#[test]
fn custom_gate_defaults_and_frame_alignment_are_stable() {
    let default_profile = gate_profile(12, 480, 2_000);
    let custom_profile = gate_profile(27, 913, 1_249);

    assert_eq!(super::WEBRTC_VAD_MODE, 3);
    assert_eq!(default_profile.noise_margin_db, 12);
    assert_eq!(default_profile.confirmation_ms, 480);
    assert_eq!(default_profile.noise_window_ms, 2_000);
    assert_eq!(default_profile.noise_window_frames, 100);
    assert_eq!(default_profile.required_voice_frames, 24);
    assert_eq!(default_profile.required_consecutive_frames, 12);
    assert_eq!(default_profile.window_frames, 48);
    assert_eq!(custom_profile.noise_margin_db, 27);
    assert_eq!(custom_profile.confirmation_ms, 920);
    assert_eq!(custom_profile.noise_window_ms, 1_200);
    assert_eq!(custom_profile.noise_window_frames, 60);
    assert_eq!(custom_profile.required_voice_frames, 46);
    assert_eq!(custom_profile.required_consecutive_frames, 12);
    assert_eq!(custom_profile.window_frames, 92);
}

#[test]
fn pre_activation_audio_preserves_long_input_in_order_and_stops_after_activation() {
    let mut pending = PreActivationAudio::new();
    for marker in 0..300i16 {
        pending.push(&AudioChunk {
            samples: vec![marker; 20],
            sample_rate: 1_000,
            channels: 1,
        });
    }

    let buffered = pending.take_for_activation();
    assert_eq!(buffered.len(), 300);
    assert_eq!(buffered.front().unwrap().samples[0], 0);
    assert_eq!(buffered.back().unwrap().samples[0], 299);

    pending.push(&AudioChunk {
        samples: vec![300],
        sample_rate: 1_000,
        channels: 1,
    });
    assert!(pending.take_for_activation().is_empty());
}

#[test]
fn initial_silence_timeout_applies_even_while_audio_keeps_arriving() {
    let timeout = Duration::from_secs(15);
    assert!(!initial_silence_timed_out(
        false,
        Duration::from_millis(14_999),
        timeout
    ));
    assert!(initial_silence_timed_out(false, timeout, timeout));
    assert!(!initial_silence_timed_out(
        true,
        Duration::from_secs(60),
        timeout
    ));
}

#[test]
fn steady_fan_marked_as_voice_remains_unconfirmed_instead_of_triggering() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    let mut activated = false;

    for frame in 0..80 {
        let decision = gate.process(true, -32.0, frame * FRAME_MS);
        activated |= decision.activated;
    }

    assert!(!gate.noise.calibrated());
    assert_eq!(gate.noise.floor_dbfs, -96.0);
    assert!(!activated);
}

#[test]
fn steady_fan_starting_after_quiet_calibration_does_not_trigger() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    for frame in 0..30 {
        gate.process(false, -55.0, frame * FRAME_MS);
    }
    assert!(gate.noise.calibrated());

    let mut activated = false;
    for frame in 30..130 {
        activated |= gate.process(true, -32.0, frame * FRAME_MS).activated;
    }

    assert!(!activated);
    assert_eq!(gate.noise.floor_dbfs, -55.0);
}

#[test]
fn speech_present_at_start_is_not_absorbed_as_the_noise_floor() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    let bootstrap = [-46.0, -32.0, -25.0, -37.0, -23.0, -29.0];
    let mut activation = None;
    for frame in 0..60 {
        let decision = gate.process(
            true,
            bootstrap[frame as usize % bootstrap.len()],
            frame * FRAME_MS,
        );
        if decision.activated {
            activation = Some(decision);
            break;
        }
    }

    assert!(gate.noise.calibrated());
    assert!(gate.noise.floor_dbfs < -37.0);
    assert_eq!(
        activation
            .expect("the provisional horizon should replay and activate startup speech")
            .first_qualified_ms,
        Some(0)
    );
}

#[test]
fn partial_quiet_then_speech_uses_the_quiet_floor_and_preserves_the_boundary() {
    let mut gate = SpeechGate::new(12, 480, 2_000);
    for frame in 0..20 {
        gate.process(false, -55.0, frame * FRAME_MS);
    }

    let speech = [-40.0, -36.0, -32.0, -38.0, -29.0, -34.0];
    for frame in 20..80 {
        gate.process(
            true,
            speech[frame as usize % speech.len()],
            frame * FRAME_MS,
        );
    }
    let mut activation = None;
    for frame in 80..110 {
        let decision = gate.process(
            true,
            speech[frame as usize % speech.len()],
            frame * FRAME_MS,
        );
        if decision.activated {
            activation = Some(decision);
            break;
        }
    }

    assert!(
        (gate.noise.floor_dbfs - -55.0).abs() < 0.1,
        "unexpected floor: {}",
        gate.noise.floor_dbfs
    );
    assert_eq!(
        activation
            .expect("partial quiet evidence should keep weak speech above the threshold")
            .first_qualified_ms,
        Some(400)
    );
}

#[test]
fn default_noise_window_is_a_maximum_and_calibrates_after_enough_quiet_evidence() {
    let mut gate = SpeechGate::new(12, 480, 2_000);
    for frame in 0..39 {
        assert!(
            !gate
                .process(false, -52.0, frame * FRAME_MS)
                .noise_calibrated
        );
    }

    let calibrated = gate.process(false, -52.0, 39 * FRAME_MS);

    assert!(calibrated.noise_calibrated);
    assert_eq!(gate.noise.window_frames, 100);
    assert!((gate.noise.floor_dbfs - -52.0).abs() < f32::EPSILON);
}

#[test]
fn recorded_ambient_voice_runs_below_the_provisional_horizon_do_not_activate() {
    let mut gate = SpeechGate::new(12, 480, 2_000);
    let first_run = [-40.0, -34.0, -29.0, -23.0, -20.0, -31.0];
    let second_run = [-50.0, -37.0, -29.0, -24.0, -22.0, -36.0];
    let mut activated = false;

    for frame in 0..160 {
        let (raw_voice, level) = if frame < 44 {
            (true, first_run[frame as usize % first_run.len()])
        } else if (71..128).contains(&frame) {
            (true, second_run[frame as usize % second_run.len()])
        } else {
            (false, -52.0 + (frame % 4) as f32 * 0.4)
        };
        activated |= gate.process(raw_voice, level, frame * FRAME_MS).activated;
    }

    assert!(gate.noise.calibrated());
    assert!(!activated);
}

#[test]
fn quiet_prefix_does_not_promote_short_recorded_ambient_runs() {
    let cases: [(usize, &[f32]); 2] = [
        (44, &[-40.0, -34.0, -29.0, -23.0, -20.0, -31.0]),
        (57, &[-50.0, -37.0, -29.0, -24.0, -22.0, -36.0]),
    ];

    for (run_frames, levels) in cases {
        let mut gate = SpeechGate::new(12, 480, 2_000);
        let mut activated = false;
        for frame in 0..12 {
            activated |= gate.process(false, -52.0, frame * FRAME_MS).activated;
        }
        for offset in 0..run_frames {
            let frame = 12 + offset;
            activated |= gate
                .process(true, levels[offset % levels.len()], frame as u64 * FRAME_MS)
                .activated;
        }
        for frame in (12 + run_frames)..160 {
            activated |= gate
                .process(false, -52.0, frame as u64 * FRAME_MS)
                .activated;
        }

        assert!(!activated, "ambient run of {run_frames} frames activated");
    }
}

#[test]
fn digital_silence_can_calibrate_without_absorbing_later_speech() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    for frame in 0..30 {
        gate.process(false, -96.0, frame * FRAME_MS);
    }

    let mut activated = false;
    let bootstrap = [-55.0, -42.0, -34.0, -48.0, -30.0, -39.0];
    for frame in 30..60 {
        gate.process(
            true,
            bootstrap[frame as usize % bootstrap.len()],
            frame * FRAME_MS,
        );
    }
    for frame in 60..120 {
        let level = [-40.0, -31.0, -36.0, -28.0][frame as usize % 4];
        activated |= gate.process(true, level, frame * FRAME_MS).activated;
    }

    assert!(gate.noise.floor_dbfs < -45.0);
    assert!(activated);
}

#[test]
fn digital_silence_then_ambiguous_fan_restarts_calibration_without_triggering() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    for frame in 0..30 {
        gate.process(false, -96.0, frame * FRAME_MS);
    }

    let mut activated = false;
    for frame in 30..90 {
        activated |= gate.process(true, -32.0, frame * FRAME_MS).activated;
    }

    assert!(!gate.noise.calibrated());
    assert!(!activated);
}

#[test]
fn speech_mixed_into_the_noise_window_uses_only_non_voice_frames_for_the_floor() {
    let mut gate = SpeechGate::new(12, 480, 2_000);
    for frame in 0..40 {
        gate.process(false, -55.0, frame * FRAME_MS);
    }
    for frame in 40..100 {
        let level = [-38.0, -27.0, -23.0, -31.0][frame as usize % 4];
        gate.process(true, level, frame * FRAME_MS);
    }

    assert!(gate.noise.calibrated());
    assert!((gate.noise.floor_dbfs - -55.0).abs() < f32::EPSILON);

    let mut activated = false;
    for frame in 100..130 {
        let level = [-30.0, -24.0, -27.0][frame as usize % 3];
        activated |= gate.process(true, level, frame * FRAME_MS).activated;
    }
    assert!(activated);
}

#[test]
fn speech_covering_the_full_noise_window_is_not_absorbed_as_ambient() {
    let mut gate = SpeechGate::new(12, 480, 2_000);
    let speech = [-48.0, -36.0, -25.0, -41.0, -22.0, -31.0];
    for frame in 0..100 {
        gate.process(
            true,
            speech[frame as usize % speech.len()],
            frame * FRAME_MS,
        );
    }

    assert!(gate.noise.calibrated());
    assert!(gate.noise.floor_dbfs < -48.0);

    let mut activated = false;
    for frame in 100..130 {
        let level = [-32.0, -24.0, -29.0, -21.0][frame as usize % 4];
        activated |= gate.process(true, level, frame * FRAME_MS).activated;
    }
    assert!(activated);
}

#[test]
fn dynamic_speech_over_a_steady_fan_eventually_calibrates_and_activates() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    for frame in 0..60 {
        gate.process(true, -34.0, frame * FRAME_MS);
    }
    assert!(!gate.noise.calibrated());

    let speech = [-35.0, -25.0, -20.0, -31.0, -23.0, -28.0];
    for frame in 60..90 {
        gate.process(
            true,
            speech[frame as usize % speech.len()],
            frame * FRAME_MS,
        );
    }
    assert!(gate.noise.calibrated());

    let mut activated = false;
    for frame in 90..120 {
        let level = [-27.0, -20.0, -25.0, -18.0][frame as usize % 4];
        activated |= gate.process(true, level, frame * FRAME_MS).activated;
    }
    assert!(activated);
}

#[test]
fn a_short_impact_is_rejected_even_with_webrtc_hangover() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    for frame in 0..30 {
        gate.process(false, -55.0, frame * FRAME_MS);
    }

    let impact_levels = [-15.0, -26.0, -38.0, -46.0, -51.0, -54.0, -55.0, -55.0];
    let mut activated = false;
    for (index, level) in impact_levels.into_iter().enumerate() {
        let decision = gate.process(true, level, (30 + index as u64) * FRAME_MS);
        activated |= decision.activated;
    }

    assert!(!activated);
}

#[test]
fn sustained_speech_above_the_noise_floor_activates() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    for frame in 0..30 {
        gate.process(false, -55.0, frame * FRAME_MS);
    }

    let mut activation = None;
    for frame in 30..60 {
        let level = [-32.0, -24.0, -29.0, -21.0][frame as usize % 4];
        let decision = gate.process(true, level, frame * FRAME_MS);
        if decision.activated {
            activation = Some(decision);
            break;
        }
    }

    let activation = activation.expect("sustained speech should activate");
    assert_eq!(activation.trigger_progress, 1.0);
    assert_eq!(activation.first_qualified_ms, Some(0));
}

#[test]
fn activation_without_a_reliable_startup_boundary_keeps_the_noise_window() {
    let mut gate = SpeechGate::new(12, 480, 2_000);
    gate.process(true, -80.0, 0);
    for frame in 1..41 {
        gate.process(false, -55.0, frame * FRAME_MS);
    }
    assert!(gate.noise.calibrated());

    let speech = [-40.0, -36.0, -32.0, -38.0, -29.0, -34.0];
    let mut activation = None;
    for frame in 41..75 {
        let decision = gate.process(
            true,
            speech[frame as usize % speech.len()],
            frame * FRAME_MS,
        );
        if decision.activated {
            activation = Some(decision);
            break;
        }
    }

    assert_eq!(
        activation
            .expect("calibrated speech should activate")
            .first_qualified_ms,
        Some(0)
    );
}

#[test]
fn custom_settings_update_resets_pending_voice_progress() {
    let mut gate = SpeechGate::new(12, 480, TEST_NOISE_WINDOW_MS);
    for frame in 0..30 {
        gate.process(false, -55.0, frame * FRAME_MS);
    }
    for frame in 30..35 {
        gate.process(true, -30.0, frame * FRAME_MS);
    }

    gate.update_settings(24, 800, TEST_NOISE_WINDOW_MS);
    let decision = gate.process(true, -30.0, 35 * FRAME_MS);

    assert_eq!(gate.profile.noise_margin_db, 24);
    assert_eq!(gate.profile.confirmation_ms, 800);
    assert_eq!(gate.profile.noise_window_ms, TEST_NOISE_WINDOW_MS);
    assert_eq!(gate.profile.required_voice_frames, 40);
    assert_eq!(gate.profile.required_consecutive_frames, 12);
    assert!(decision.trigger_progress < 1.0);
}

#[test]
fn changing_the_noise_window_restarts_calibration_without_restarting_the_microphone() {
    let mut gate = SpeechGate::new(12, 480, 1_200);
    for frame in 0..23 {
        let decision = gate.process(false, -55.0, frame * FRAME_MS);
        assert!(!decision.noise_calibrated);
    }
    assert!(gate.process(false, -55.0, 23 * FRAME_MS).noise_calibrated);

    gate.update_settings(12, 480, 2_000);
    assert!(!gate.noise.calibrated());
    for frame in 24..63 {
        let decision = gate.process(false, -54.0, frame * FRAME_MS);
        assert!(!decision.noise_calibrated);
    }
    let recalibrated = gate.process(false, -54.0, 63 * FRAME_MS);

    assert!(recalibrated.noise_calibrated);
    assert_eq!(gate.profile.noise_window_ms, 2_000);
    assert!((gate.noise.floor_dbfs - -54.0).abs() < f32::EPSILON);
}

#[test]
fn configured_noise_window_also_limits_the_adaptive_history() {
    let mut gate = SpeechGate::new(12, 480, 400);
    for frame in 0..20 {
        gate.process(false, -55.0, frame * FRAME_MS);
    }
    for frame in 20..80 {
        let level = -58.0 + (frame % 6) as f32;
        gate.process(false, level, frame * FRAME_MS);
    }

    assert!(gate.noise.calibrated());
    assert_eq!(gate.noise.window_frames, 20);
    assert_eq!(gate.noise.samples.len(), 20);
}

#[test]
fn converter_produces_16khz_mono_samples() {
    let mut converter = PcmConverter::default();
    let output = converter
        .push_chunk(&AudioChunk {
            samples: vec![1; 480 * 2],
            sample_rate: 48_000,
            channels: 2,
        })
        .unwrap();
    assert!(!output.is_empty());
    assert!(output.len() <= FRAME_SAMPLES);
}
