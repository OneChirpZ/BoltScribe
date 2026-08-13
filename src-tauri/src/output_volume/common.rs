use crate::config::OutputVolumeDuckingConfig;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AudioOutputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub platform: String,
    pub supports_volume_control: bool,
    pub supports_mute_control: bool,
}

pub(super) fn should_duck_device(device_name: &str, whitelist: &[String]) -> bool {
    whitelist.is_empty() || whitelist.iter().any(|name| name == device_name)
}

pub(super) fn ducked_volume(original_volume: f32, reduction_percent: u32) -> f32 {
    let multiplier = 1.0 - (reduction_percent.clamp(0, 100) as f32 / 100.0);
    (original_volume * multiplier).clamp(0.0, 1.0)
}

pub(super) fn target_volume(original_volume: f32, config: &OutputVolumeDuckingConfig) -> f32 {
    if config.mute_instead_of_reduce {
        0.0
    } else {
        ducked_volume(original_volume, config.reduction_percent)
    }
}

pub(super) fn volume_matches(left: f32, right: f32) -> bool {
    (left - right).abs() <= 0.01
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DuckingStrategy {
    Volume,
    NativeMute,
    SoundSourceHotkey,
    Unsupported,
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn ducking_strategy(
    supports_volume_control: bool,
    supports_mute_control: bool,
    mute_instead_of_reduce: bool,
    sound_source_hotkey_fallback_enabled: bool,
) -> DuckingStrategy {
    if mute_instead_of_reduce {
        if supports_mute_control {
            DuckingStrategy::NativeMute
        } else if supports_volume_control {
            DuckingStrategy::Volume
        } else if sound_source_hotkey_fallback_enabled {
            DuckingStrategy::SoundSourceHotkey
        } else {
            DuckingStrategy::Unsupported
        }
    } else if supports_volume_control {
        DuckingStrategy::Volume
    } else if supports_mute_control {
        DuckingStrategy::NativeMute
    } else if sound_source_hotkey_fallback_enabled {
        DuckingStrategy::SoundSourceHotkey
    } else {
        DuckingStrategy::Unsupported
    }
}
