mod common;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

use crate::config::OutputVolumeDuckingConfig;
use anyhow::Result;
#[cfg(test)]
use common::ducked_volume;
pub use common::AudioOutputDevice;
#[cfg(any(target_os = "macos", test))]
use common::{ducking_strategy, DuckingStrategy};
use common::{should_duck_device, target_volume, volume_matches};

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use unsupported as platform;
#[cfg(target_os = "windows")]
use windows as platform;

pub use platform::OutputVolumeDuckingSession;

pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>> {
    platform::list_output_devices()
}

pub fn start_ducking_session(
    config: &OutputVolumeDuckingConfig,
) -> Result<Option<OutputVolumeDuckingSession>> {
    if !config.enabled {
        return Ok(None);
    }
    platform::start_ducking_session(config)
}

#[cfg(test)]
mod tests;
