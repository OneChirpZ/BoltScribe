use crate::config::AudioConfig;
use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AudioInputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub platform: String,
}

pub struct AudioInputDeviceCandidate {
    pub device: cpal::Device,
    pub id: String,
    pub name: String,
}

pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
    platform::list_input_devices()
}

pub fn input_device_candidates(config: &AudioConfig) -> Result<Vec<AudioInputDeviceCandidate>> {
    let available = list_input_devices()?;
    let ranked = rank_input_devices(config, &available);
    if ranked.is_empty() {
        return Err(anyhow!(
            "No eligible input device found; all available devices may be blacklisted"
        ));
    }

    let mut cpal_devices = enumerate_cpal_input_devices()?;
    let mut candidates = Vec::new();
    for info in ranked {
        let Some(index) = cpal_devices
            .iter()
            .position(|candidate| candidate.id == info.id)
            .or_else(|| {
                cpal_devices
                    .iter()
                    .position(|candidate| candidate.name == info.name)
            })
        else {
            continue;
        };
        let candidate = cpal_devices.remove(index);
        candidates.push(AudioInputDeviceCandidate {
            device: candidate.device,
            id: info.id,
            name: info.name,
        });
    }

    if candidates.is_empty() {
        Err(anyhow!("No eligible input device could be opened"))
    } else {
        Ok(candidates)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
        list_cpal_input_devices("windows")
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use core_foundation_sys::string::{
        kCFStringEncodingUTF8, CFStringGetCString, CFStringGetCStringPtr,
        CFStringRef as CoreFoundationStringRef,
    };
    use coreaudio_sys::{
        kAudioDevicePropertyDeviceNameCFString, kAudioDevicePropertyDeviceUID,
        kAudioDevicePropertyStreamConfiguration, kAudioHardwareNoError,
        kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMaster, kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeInput, kAudioObjectSystemObject, AudioBuffer, AudioBufferList,
        AudioDeviceID, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
        AudioObjectPropertyAddress, CFStringRef, OSStatus,
    };
    use std::ffi::CStr;
    use std::mem;
    use std::ptr::null;
    use std::slice;

    struct CoreAudioInputDevice {
        uid: String,
        name: String,
    }

    pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
        let default_uid = default_input_device_id()
            .and_then(|id| string_property(id, kAudioDevicePropertyDeviceUID).ok());

        Ok(coreaudio_input_devices()?
            .into_iter()
            .map(|device| AudioInputDevice {
                id: coreaudio_device_id(&device.uid),
                name: device.name,
                is_default: default_uid.as_deref() == Some(device.uid.as_str()),
                platform: "macos".to_string(),
            })
            .collect())
    }

    fn coreaudio_input_devices() -> Result<Vec<CoreAudioInputDevice>> {
        let mut devices = Vec::new();
        for id in audio_device_ids()? {
            if !device_has_input_channels(id)? {
                continue;
            }
            let Some(uid) = string_property(id, kAudioDevicePropertyDeviceUID).ok() else {
                continue;
            };
            let name = string_property(id, kAudioDevicePropertyDeviceNameCFString)
                .unwrap_or_else(|_| format!("Input device {}", devices.len() + 1));
            devices.push(CoreAudioInputDevice { uid, name });
        }
        Ok(devices)
    }

    fn audio_device_ids() -> Result<Vec<AudioDeviceID>> {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let data_size = property_data_size(kAudioObjectSystemObject, &address)?;
        let device_count = data_size as usize / mem::size_of::<AudioDeviceID>();
        let mut devices = Vec::<AudioDeviceID>::with_capacity(device_count);
        let mut data_size = data_size;
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &address,
                0,
                null(),
                &mut data_size,
                devices.as_mut_ptr().cast(),
            )
        };
        check_status(status, "Failed to enumerate CoreAudio devices")?;
        unsafe {
            devices.set_len(device_count);
        }
        Ok(devices)
    }

    fn default_input_device_id() -> Option<AudioDeviceID> {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultInputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let mut device_id = 0;
        let mut data_size = mem::size_of::<AudioDeviceID>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &address,
                0,
                null(),
                &mut data_size,
                (&mut device_id as *mut AudioDeviceID).cast(),
            )
        };
        (status == kAudioHardwareNoError as OSStatus && device_id != 0).then_some(device_id)
    }

    fn device_has_input_channels(device_id: AudioDeviceID) -> Result<bool> {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyStreamConfiguration,
            mScope: kAudioObjectPropertyScopeInput,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let data_size = property_data_size(device_id, &address)?;
        if data_size == 0 {
            return Ok(false);
        }

        let mut data = vec![0u8; data_size as usize];
        let mut data_size = data_size;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                null(),
                &mut data_size,
                data.as_mut_ptr().cast(),
            )
        };
        check_status(
            status,
            "Failed to read CoreAudio input stream configuration",
        )?;

        let buffer_list = data.as_ptr().cast::<AudioBufferList>();
        let buffer_count = unsafe { (*buffer_list).mNumberBuffers as usize };
        if buffer_count == 0 {
            return Ok(false);
        }
        let first_buffer = unsafe { (*buffer_list).mBuffers.as_ptr() };
        let buffers = unsafe { slice::from_raw_parts(first_buffer, buffer_count) };
        Ok(buffers
            .iter()
            .any(|buffer: &AudioBuffer| buffer.mNumberChannels > 0))
    }

    fn string_property(device_id: AudioDeviceID, selector: u32) -> Result<String> {
        let address = AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let mut cf_string: CFStringRef = null();
        let mut data_size = mem::size_of::<CFStringRef>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                null(),
                &mut data_size,
                (&mut cf_string as *mut CFStringRef).cast(),
            )
        };
        check_status(status, "Failed to read CoreAudio device string")?;
        if cf_string.is_null() {
            return Err(anyhow!("CoreAudio device string is empty"));
        }
        cf_string_to_string(cf_string)
    }

    fn cf_string_to_string(value: CFStringRef) -> Result<String> {
        let value = value as CoreFoundationStringRef;
        let c_string = unsafe { CFStringGetCStringPtr(value, kCFStringEncodingUTF8) };
        if !c_string.is_null() {
            return Ok(unsafe { CStr::from_ptr(c_string) }
                .to_string_lossy()
                .into_owned());
        }

        let mut buffer = vec![0i8; 1024];
        let copied = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as _,
                kCFStringEncodingUTF8,
            )
        };
        if copied == 0 {
            return Err(anyhow!("Failed to convert CoreAudio device string"));
        }
        Ok(unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned())
    }

    fn property_data_size(
        object_id: AudioDeviceID,
        address: &AudioObjectPropertyAddress,
    ) -> Result<u32> {
        let mut data_size = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(object_id, address, 0, null(), &mut data_size)
        };
        check_status(status, "Failed to read CoreAudio property size")?;
        Ok(data_size)
    }

    fn check_status(status: OSStatus, message: &str) -> Result<()> {
        if status == kAudioHardwareNoError as OSStatus {
            Ok(())
        } else {
            Err(anyhow!("{message}: OSStatus {status}"))
        }
    }

    fn coreaudio_device_id(uid: &str) -> String {
        format!("coreaudio:{uid}")
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use super::*;

    pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
        list_cpal_input_devices("other")
    }
}

#[cfg(not(target_os = "macos"))]
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

struct CpalInputDevice {
    device: cpal::Device,
    id: String,
    name: String,
}

fn enumerate_cpal_input_devices() -> Result<Vec<CpalInputDevice>> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .context("Failed to enumerate input devices")?
        .enumerate()
        .map(|(index, device)| {
            let name = device
                .name()
                .unwrap_or_else(|_| format!("Input device {}", index + 1));
            CpalInputDevice {
                id: cpal_device_id(index, &name),
                name,
                device,
            }
        })
        .collect::<Vec<_>>();
    Ok(devices)
}

fn rank_input_devices(
    config: &AudioConfig,
    available: &[AudioInputDevice],
) -> Vec<AudioInputDevice> {
    let blocked_names = available
        .iter()
        .filter(|device| {
            config
                .input_device_blacklist
                .iter()
                .any(|blocked| blocked.matches_device(&device.id, &device.name))
        })
        .map(|device| device.name.as_str())
        .collect::<Vec<_>>();
    let eligible = available
        .iter()
        .filter(|device| {
            !blocked_names
                .iter()
                .any(|blocked_name| blocked_name.eq_ignore_ascii_case(&device.name))
        })
        .collect::<Vec<_>>();
    let mut ranked = Vec::new();

    for preferred in &config.input_device_priority {
        let id_match = (!preferred.id.is_empty()).then(|| {
            eligible.iter().copied().find(|device| {
                device.id == preferred.id
                    && !ranked
                        .iter()
                        .any(|ranked: &AudioInputDevice| ranked.id == device.id)
            })
        });
        let device = id_match.flatten().or_else(|| {
            (!preferred.name.is_empty())
                .then(|| {
                    eligible.iter().copied().find(|device| {
                        preferred.name.eq_ignore_ascii_case(&device.name)
                            && !ranked
                                .iter()
                                .any(|ranked: &AudioInputDevice| ranked.id == device.id)
                    })
                })
                .flatten()
        });
        if let Some(device) = device {
            ranked.push(device.clone());
        }
    }

    for device in eligible.iter().copied().filter(|device| device.is_default) {
        if !ranked
            .iter()
            .any(|ranked: &AudioInputDevice| ranked.id == device.id)
        {
            ranked.push(device.clone());
        }
    }

    for device in eligible {
        if !ranked
            .iter()
            .any(|ranked: &AudioInputDevice| ranked.id == device.id)
        {
            ranked.push(device.clone());
        }
    }
    ranked
}

fn cpal_device_id(index: usize, name: &str) -> String {
    format!("cpal:{index}:{name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AudioInputDeviceRef;

    fn device(id: &str, name: &str, is_default: bool) -> AudioInputDevice {
        AudioInputDevice {
            id: id.to_string(),
            name: name.to_string(),
            is_default,
            platform: "test".to_string(),
        }
    }

    fn reference(id: &str, name: &str) -> AudioInputDeviceRef {
        AudioInputDeviceRef {
            id: id.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn priority_uses_exact_id_before_stale_name() {
        let available = vec![
            device("device-b", "Saved Name", false),
            device("device-a", "Renamed Device", true),
        ];
        let mut config = AudioConfig::default();
        config.input_device_priority = vec![reference("device-a", "Saved Name")];

        let ranked = rank_input_devices(&config, &available);

        assert_eq!(ranked[0].id, "device-a");
    }

    #[test]
    fn priority_falls_back_to_name_when_saved_id_is_missing() {
        let available = vec![
            device("new-id", "Preferred Mic", false),
            device("default", "Default Mic", true),
        ];
        let mut config = AudioConfig::default();
        config.input_device_priority = vec![reference("old-id", "Preferred Mic")];

        let ranked = rank_input_devices(&config, &available);

        assert_eq!(ranked[0].id, "new-id");
        assert_eq!(ranked[1].id, "default");
    }

    #[test]
    fn unavailable_priorities_fall_through_to_default_then_remaining_devices() {
        let available = vec![
            device("other", "Other Mic", false),
            device("default", "Default Mic", true),
        ];
        let mut config = AudioConfig::default();
        config.input_device_priority = vec![reference("missing", "Missing Mic")];

        let ranked = rank_input_devices(&config, &available);

        assert_eq!(
            ranked
                .iter()
                .map(|device| device.id.as_str())
                .collect::<Vec<_>>(),
            vec!["default", "other"]
        );
    }

    #[test]
    fn blacklist_wins_over_priority_default_and_name_changes() {
        let available = vec![
            device("blocked-id", "Capture Card", true),
            device("safe-id", "Safe Mic", false),
        ];
        let mut config = AudioConfig::default();
        config.input_device_priority = vec![reference("blocked-id", "Old Capture Name")];
        config.input_device_blacklist = vec![reference("old-id", "capture card")];

        let ranked = rank_input_devices(&config, &available);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "safe-id");
    }

    #[test]
    fn all_blacklisted_devices_produce_no_candidates() {
        let available = vec![device("only", "Only Mic", true)];
        let mut config = AudioConfig::default();
        config.input_device_blacklist = vec![reference("only", "Only Mic")];

        assert!(rank_input_devices(&config, &available).is_empty());
    }

    #[test]
    fn stable_id_blacklist_blocks_ambiguous_same_name_endpoints() {
        let available = vec![
            device("blocked", "Duplicate Mic", true),
            device("other", "Duplicate Mic", false),
            device("safe", "Safe Mic", false),
        ];
        let mut config = AudioConfig::default();
        config.input_device_blacklist = vec![reference("blocked", "")];

        let ranked = rank_input_devices(&config, &available);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "safe");
    }
}
