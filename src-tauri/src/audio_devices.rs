use crate::config::AudioConfig;
use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AudioInputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub platform: String,
}

pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
    platform::list_input_devices()
}

pub fn resolve_input_device(config: &AudioConfig) -> Result<cpal::Device> {
    platform::resolve_input_device(config)
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
        list_cpal_input_devices("windows")
    }

    pub fn resolve_input_device(config: &AudioConfig) -> Result<cpal::Device> {
        resolve_cpal_input_device(config)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
        list_cpal_input_devices("macos")
    }

    pub fn resolve_input_device(config: &AudioConfig) -> Result<cpal::Device> {
        // Keep this platform boundary explicit so a future macOS implementation can
        // resolve stable CoreAudio UIDs without touching recorder workflow code.
        resolve_cpal_input_device(config)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use super::*;

    pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
        list_cpal_input_devices("other")
    }

    pub fn resolve_input_device(config: &AudioConfig) -> Result<cpal::Device> {
        resolve_cpal_input_device(config)
    }
}

fn list_cpal_input_devices(platform: &str) -> Result<Vec<AudioInputDevice>> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let mut devices = Vec::new();

    for (index, device) in host
        .input_devices()
        .context("Failed to enumerate input devices")?
        .enumerate()
    {
        let name = device
            .name()
            .unwrap_or_else(|_| format!("Input device {}", index + 1));
        let is_default = default_name.as_deref() == Some(name.as_str());
        devices.push(AudioInputDevice {
            id: cpal_device_id(index, &name),
            name,
            is_default,
            platform: platform.to_string(),
        });
    }

    Ok(devices)
}

fn resolve_cpal_input_device(config: &AudioConfig) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if config.uses_system_default_input_device() {
        return host
            .default_input_device()
            .ok_or_else(|| anyhow!("No default input device found"));
    }

    let requested_id = config.input_device_id.as_deref().unwrap_or_default();
    let requested_name = config.input_device_name.as_deref().unwrap_or_default();
    let mut name_match = None;

    for (index, device) in host
        .input_devices()
        .context("Failed to enumerate input devices")?
        .enumerate()
    {
        let name = device.name().unwrap_or_default();
        if !requested_id.is_empty() && cpal_device_id(index, &name) == requested_id {
            return Ok(device);
        }
        if name_match.is_none() && !requested_name.is_empty() && name == requested_name {
            name_match = Some(device);
        }
    }

    if let Some(device) = name_match {
        return Ok(device);
    }

    Err(anyhow!(
        "Selected input device is not available: {}",
        if requested_name.is_empty() {
            requested_id
        } else {
            requested_name
        }
    ))
}

fn cpal_device_id(index: usize, name: &str) -> String {
    format!("cpal:{index}:{name}")
}
