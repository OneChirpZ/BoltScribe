use super::*;
use ::windows::core::{PCWSTR, PWSTR};
use ::windows::Win32::Devices::Properties::DEVPKEY_Device_FriendlyName;
use ::windows::Win32::Foundation::{BOOL, RPC_E_CHANGED_MODE};
use ::windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use ::windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use ::windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use ::windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED, STGM_READ,
};
use ::windows::Win32::System::Variant::VT_LPWSTR;
use anyhow::{anyhow, Context, Result};
use std::ffi::c_void;
use std::ptr::null;
use std::slice;

pub struct OutputVolumeDuckingSession {
    state: DuckingSessionState,
}

enum DuckingSessionState {
    Volume {
        device_id: String,
        device_name: String,
        original_volume: f32,
        ducked_volume: f32,
    },
    Mute {
        device_id: String,
        device_name: String,
    },
}

struct ComGuard {
    should_uninitialize: bool,
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

impl OutputVolumeDuckingSession {
    pub fn restore(self) -> Result<()> {
        let _com = initialize_com()?;
        let enumerator = device_enumerator()?;
        match self.state {
            DuckingSessionState::Volume {
                device_id,
                device_name,
                original_volume,
                ducked_volume,
            } => {
                let device = device_by_id(&enumerator, &device_id)
                    .with_context(|| format!("Failed to reopen output device {device_name}"))?;
                let endpoint = endpoint_volume_for_device(&device)?;
                restore_volume(&endpoint, &device_name, original_volume, ducked_volume)
            }
            DuckingSessionState::Mute {
                device_id,
                device_name,
            } => {
                let device = device_by_id(&enumerator, &device_id)
                    .with_context(|| format!("Failed to reopen output device {device_name}"))?;
                let endpoint = endpoint_volume_for_device(&device)?;
                restore_mute(&endpoint, &device_name)
            }
        }
    }
}

pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>> {
    let _com = initialize_com()?;
    let enumerator = device_enumerator()?;
    let default_id = default_output_device(&enumerator)
        .ok()
        .and_then(|device| device_id(&device).ok());
    let collection = unsafe {
        enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .context("Failed to enumerate Windows output devices")?
    };
    let count = unsafe {
        collection
            .GetCount()
            .context("Failed to count Windows output devices")?
    };
    let mut devices = Vec::with_capacity(count as usize);

    for index in 0..count {
        let device = unsafe {
            collection
                .Item(index)
                .with_context(|| format!("Failed to read Windows output device {index}"))?
        };
        let id = device_id(&device).unwrap_or_else(|_| format!("windows-output:{index}"));
        let name = device_friendly_name(&device)
            .unwrap_or_else(|_| format!("Output device {}", index + 1));
        let (supports_volume_control, supports_mute_control) =
            match endpoint_volume_for_device(&device) {
                Ok(endpoint) => (
                    read_output_volume(&endpoint).is_ok(),
                    read_output_mute(&endpoint).is_ok(),
                ),
                Err(_) => (false, false),
            };
        devices.push(AudioOutputDevice {
            id: format!("windows:{id}"),
            name,
            is_default: default_id.as_deref() == Some(id.as_str()),
            platform: "windows".to_string(),
            supports_volume_control,
            supports_mute_control,
        });
    }

    Ok(devices)
}

pub fn start_ducking_session(
    config: &OutputVolumeDuckingConfig,
) -> Result<Option<OutputVolumeDuckingSession>> {
    let _com = initialize_com()?;
    let enumerator = device_enumerator()?;
    let device =
        default_output_device(&enumerator).context("No default Windows output device found")?;
    let device_id = device_id(&device).context("Failed to read default output device id")?;
    let device_name =
        device_friendly_name(&device).unwrap_or_else(|_| "Default output device".to_string());
    if !should_duck_device(&device_name, &config.device_name_whitelist) {
        return Ok(None);
    }

    let endpoint = endpoint_volume_for_device(&device)?;
    if config.mute_instead_of_reduce {
        match read_output_mute(&endpoint) {
            Ok(true) => return Ok(None),
            Ok(false) => match write_output_mute(&endpoint, true) {
                Ok(()) => {
                    return Ok(Some(OutputVolumeDuckingSession {
                        state: DuckingSessionState::Mute {
                            device_id,
                            device_name,
                        },
                    }));
                }
                Err(err) => {
                    eprintln!("failed to mute Windows output, trying volume fallback: {err:?}");
                }
            },
            Err(err) => {
                eprintln!("failed to read Windows output mute, trying volume fallback: {err:?}");
            }
        }
    }

    if let Ok(original_volume) = read_output_volume(&endpoint) {
        let ducked_volume = target_volume(original_volume, config);
        if volume_matches(original_volume, ducked_volume) {
            return Ok(None);
        }
        match write_output_volume(&endpoint, ducked_volume) {
            Ok(()) => {
                return Ok(Some(OutputVolumeDuckingSession {
                    state: DuckingSessionState::Volume {
                        device_id,
                        device_name,
                        original_volume,
                        ducked_volume,
                    },
                }));
            }
            Err(err) => {
                eprintln!("failed to set Windows output volume, trying mute fallback: {err:?}");
            }
        }
    }

    if read_output_mute(&endpoint)? {
        return Ok(None);
    }
    write_output_mute(&endpoint, true)?;
    Ok(Some(OutputVolumeDuckingSession {
        state: DuckingSessionState::Mute {
            device_id,
            device_name,
        },
    }))
}

fn initialize_com() -> Result<ComGuard> {
    let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if result.is_ok() {
        return Ok(ComGuard {
            should_uninitialize: true,
        });
    }
    if result == RPC_E_CHANGED_MODE {
        return Ok(ComGuard {
            should_uninitialize: false,
        });
    }
    result
        .ok()
        .context("Failed to initialize COM for Windows output volume control")?;
    unreachable!()
}

fn device_enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .context("Failed to create Windows audio device enumerator")
}

fn default_output_device(enumerator: &IMMDeviceEnumerator) -> Result<IMMDevice> {
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .context("Failed to get default Windows output device")
}

fn device_by_id(enumerator: &IMMDeviceEnumerator, id: &str) -> Result<IMMDevice> {
    let id = wide_null(id);
    unsafe { enumerator.GetDevice(PCWSTR::from_raw(id.as_ptr())) }
        .context("Failed to get Windows output device by id")
}

fn endpoint_volume_for_device(device: &IMMDevice) -> Result<IAudioEndpointVolume> {
    unsafe { device.Activate(CLSCTX_ALL, None) }
        .context("Failed to activate Windows endpoint volume control")
}

fn restore_volume(
    endpoint: &IAudioEndpointVolume,
    device_name: &str,
    original_volume: f32,
    ducked_volume: f32,
) -> Result<()> {
    let current_volume = read_output_volume(endpoint)
        .with_context(|| format!("Failed to read output volume for {device_name}"))?;
    if !volume_matches(current_volume, ducked_volume) {
        eprintln!("output volume changed during recording; skipping restore for {device_name}");
        return Ok(());
    }
    write_output_volume(endpoint, original_volume)
        .with_context(|| format!("Failed to restore output volume for {device_name}"))
}

fn restore_mute(endpoint: &IAudioEndpointVolume, device_name: &str) -> Result<()> {
    let current_muted = read_output_mute(endpoint)
        .with_context(|| format!("Failed to read output mute for {device_name}"))?;
    if !current_muted {
        eprintln!("output mute changed during recording; skipping restore for {device_name}");
        return Ok(());
    }
    write_output_mute(endpoint, false)
        .with_context(|| format!("Failed to restore output mute for {device_name}"))
}

fn read_output_volume(endpoint: &IAudioEndpointVolume) -> Result<f32> {
    let volume = unsafe {
        endpoint
            .GetMasterVolumeLevelScalar()
            .context("Failed to read Windows output volume")?
    };
    Ok(volume.clamp(0.0, 1.0))
}

fn write_output_volume(endpoint: &IAudioEndpointVolume, volume: f32) -> Result<()> {
    unsafe {
        endpoint
            .SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), null())
            .context("Failed to set Windows output volume")
    }
}

fn read_output_mute(endpoint: &IAudioEndpointVolume) -> Result<bool> {
    let muted = unsafe {
        endpoint
            .GetMute()
            .context("Failed to read Windows output mute")?
    };
    Ok(muted.as_bool())
}

fn write_output_mute(endpoint: &IAudioEndpointVolume, muted: bool) -> Result<()> {
    unsafe {
        endpoint
            .SetMute(BOOL::from(muted), null())
            .context("Failed to set Windows output mute")
    }
}

fn device_id(device: &IMMDevice) -> Result<String> {
    let value = unsafe {
        device
            .GetId()
            .context("Failed to read Windows output device id")?
    };
    let result = pwstr_to_string(value);
    unsafe { CoTaskMemFree(Some(value.as_ptr() as *const c_void)) };
    result
}

fn pwstr_to_string(value: PWSTR) -> Result<String> {
    if value.is_null() {
        return Err(anyhow!("Windows string pointer is null"));
    }
    unsafe { value.to_string() }.context("Failed to convert Windows string")
}

fn device_friendly_name(device: &IMMDevice) -> Result<String> {
    let store = unsafe {
        device
            .OpenPropertyStore(STGM_READ)
            .context("Failed to open Windows output device property store")?
    };
    let mut value = unsafe {
        store
            .GetValue(&DEVPKEY_Device_FriendlyName as *const _ as *const _)
            .context("Failed to read Windows output device friendly name")?
    };
    let result = property_string(&value);
    unsafe {
        if let Err(err) = PropVariantClear(&mut value) {
            eprintln!("failed to clear Windows output device property value: {err:?}");
        }
    }
    result
}

fn property_string(value: &::windows::core::PROPVARIANT) -> Result<String> {
    let raw = unsafe { &value.as_raw().Anonymous.Anonymous };
    if raw.vt != VT_LPWSTR.0 {
        return Err(anyhow!("Windows output device name is not a string"));
    }
    let ptr_utf16 = unsafe { *(&raw.Anonymous as *const _ as *const *const u16) };
    if ptr_utf16.is_null() {
        return Err(anyhow!("Windows output device name is null"));
    }

    let mut len = 0usize;
    unsafe {
        while *ptr_utf16.add(len) != 0 {
            len += 1;
        }
    }
    let name = unsafe { slice::from_raw_parts(ptr_utf16, len) };
    Ok(String::from_utf16_lossy(name))
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
