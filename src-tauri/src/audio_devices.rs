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

    pub fn resolve_input_device(config: &AudioConfig) -> Result<cpal::Device> {
        if config.uses_system_default_input_device() {
            return resolve_cpal_input_device(config);
        }

        if let Some(uid) = config
            .input_device_id
            .as_deref()
            .and_then(coreaudio_uid_from_config_id)
        {
            if let Some(device_info) = coreaudio_input_devices()?
                .into_iter()
                .find(|device| device.uid == uid)
            {
                if let Some(device) = cpal_input_device_by_name(&device_info.name)? {
                    return Ok(device);
                }
            }
        }

        // Keep legacy cpal IDs and saved names working for configs written before
        // macOS switched to stable CoreAudio UIDs.
        resolve_cpal_input_device(config)
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

    fn cpal_input_device_by_name(name: &str) -> Result<Option<cpal::Device>> {
        for device in cpal::default_host()
            .input_devices()
            .context("Failed to enumerate input devices")?
        {
            if device.name().unwrap_or_default() == name {
                return Ok(Some(device));
            }
        }
        Ok(None)
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

    fn coreaudio_uid_from_config_id(id: &str) -> Option<&str> {
        id.strip_prefix("coreaudio:").filter(|uid| !uid.is_empty())
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
