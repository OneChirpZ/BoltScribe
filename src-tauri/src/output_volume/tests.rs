use super::*;

#[test]
fn whitelist_empty_matches_any_device() {
    assert!(should_duck_device("External Speaker", &[]));
}

#[test]
fn whitelist_matches_device_name_exactly() {
    let whitelist = vec!["External Speaker".to_string()];

    assert!(should_duck_device("External Speaker", &whitelist));
    assert!(!should_duck_device("Headphones", &whitelist));
}

#[test]
fn ducked_volume_uses_reduction_percent() {
    assert!(volume_matches(ducked_volume(0.8, 70), 0.24));
    assert!(volume_matches(ducked_volume(0.8, 0), 0.8));
    assert!(volume_matches(ducked_volume(0.8, 150), 0.0));
}

#[test]
fn target_volume_can_mute_instead_of_reduce() {
    let mut config = OutputVolumeDuckingConfig {
        enabled: true,
        mute_instead_of_reduce: false,
        reduction_percent: 70,
        device_name_whitelist: Vec::new(),
        sound_source_hotkey_fallback_enabled: false,
        sound_source_toggle_mute_hotkey: "Cmd+Opt+Ctrl+A".to_string(),
    };

    assert!(volume_matches(target_volume(0.8, &config), 0.24));

    config.mute_instead_of_reduce = true;
    assert!(volume_matches(target_volume(0.8, &config), 0.0));
}

#[test]
fn ducking_strategy_prefers_native_controls() {
    assert_eq!(
        ducking_strategy(true, true, false, true),
        DuckingStrategy::Volume
    );
    assert_eq!(
        ducking_strategy(false, true, false, true),
        DuckingStrategy::NativeMute
    );
    assert_eq!(
        ducking_strategy(false, false, false, true),
        DuckingStrategy::SoundSourceHotkey
    );
    assert_eq!(
        ducking_strategy(false, false, false, false),
        DuckingStrategy::Unsupported
    );
}

#[test]
fn ducking_strategy_prefers_mute_when_requested() {
    assert_eq!(
        ducking_strategy(true, true, true, true),
        DuckingStrategy::NativeMute
    );
    assert_eq!(
        ducking_strategy(true, false, true, true),
        DuckingStrategy::Volume
    );
    assert_eq!(
        ducking_strategy(false, false, true, true),
        DuckingStrategy::SoundSourceHotkey
    );
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[test]
fn unsupported_platform_ducking_is_noop() {
    let config = OutputVolumeDuckingConfig {
        enabled: true,
        mute_instead_of_reduce: false,
        reduction_percent: 70,
        device_name_whitelist: Vec::new(),
        sound_source_hotkey_fallback_enabled: true,
        sound_source_toggle_mute_hotkey: "Cmd+Opt+Ctrl+A".to_string(),
    };

    assert!(start_ducking_session(&config).unwrap().is_none());
}
