use crate::config::OutputVolumeDuckingConfig;
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AudioOutputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub platform: String,
}

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

pub use platform::OutputVolumeDuckingSession;

fn should_duck_device(device_name: &str, whitelist: &[String]) -> bool {
    whitelist.is_empty() || whitelist.iter().any(|name| name == device_name)
}

fn ducked_volume(original_volume: f32, reduction_percent: u32) -> f32 {
    let multiplier = 1.0 - (reduction_percent.clamp(0, 100) as f32 / 100.0);
    (original_volume * multiplier).clamp(0.0, 1.0)
}

fn volume_matches(left: f32, right: f32) -> bool {
    (left - right).abs() <= 0.01
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use anyhow::{anyhow, Context, Result};
    use core_foundation_sys::string::{
        kCFStringEncodingUTF8, CFStringGetCString, CFStringGetCStringPtr,
        CFStringRef as CoreFoundationStringRef,
    };
    use coreaudio_sys::{
        kAudioDevicePropertyDeviceNameCFString, kAudioDevicePropertyDeviceUID,
        kAudioDevicePropertyStreamConfiguration, kAudioDevicePropertyVolumeScalar,
        kAudioHardwareNoError, kAudioHardwarePropertyDefaultOutputDevice,
        kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMaster,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
        AudioBuffer, AudioBufferList, AudioDeviceID, AudioObjectGetPropertyData,
        AudioObjectGetPropertyDataSize, AudioObjectHasProperty, AudioObjectIsPropertySettable,
        AudioObjectPropertyAddress, AudioObjectSetPropertyData, Boolean, CFStringRef, OSStatus,
    };
    use std::ffi::CStr;
    use std::mem;
    use std::ptr::null;
    use std::slice;

    struct CoreAudioOutputDevice {
        id: AudioDeviceID,
        uid: Option<String>,
        name: String,
    }

    pub struct OutputVolumeDuckingSession {
        device_id: AudioDeviceID,
        device_name: String,
        original_volume: f32,
        ducked_volume: f32,
    }

    impl OutputVolumeDuckingSession {
        pub fn restore(self) -> Result<()> {
            let current_volume = read_output_volume(self.device_id).with_context(|| {
                format!("Failed to read output volume for {}", self.device_name)
            })?;
            if !volume_matches(current_volume, self.ducked_volume) {
                eprintln!(
                    "output volume changed during recording; skipping restore for {}",
                    self.device_name
                );
                return Ok(());
            }
            write_output_volume(self.device_id, self.original_volume).with_context(|| {
                format!("Failed to restore output volume for {}", self.device_name)
            })
        }
    }

    pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>> {
        let default_id = default_output_device_id();
        Ok(coreaudio_output_devices()?
            .into_iter()
            .map(|device| AudioOutputDevice {
                id: output_device_id(&device),
                name: device.name,
                is_default: default_id == Some(device.id),
                platform: "macos".to_string(),
            })
            .collect())
    }

    pub fn start_ducking_session(
        config: &OutputVolumeDuckingConfig,
    ) -> Result<Option<OutputVolumeDuckingSession>> {
        let device_id =
            default_output_device_id().ok_or_else(|| anyhow!("No default output device found"))?;
        let device_name = string_property(device_id, kAudioDevicePropertyDeviceNameCFString)
            .unwrap_or_else(|_| "Default output device".to_string());
        if !should_duck_device(&device_name, &config.device_name_whitelist) {
            return Ok(None);
        }

        let original_volume = read_output_volume(device_id)?;
        let ducked_volume = ducked_volume(original_volume, config.reduction_percent);
        if volume_matches(original_volume, ducked_volume) {
            return Ok(None);
        }
        write_output_volume(device_id, ducked_volume)?;
        Ok(Some(OutputVolumeDuckingSession {
            device_id,
            device_name,
            original_volume,
            ducked_volume,
        }))
    }

    fn coreaudio_output_devices() -> Result<Vec<CoreAudioOutputDevice>> {
        let mut devices = Vec::new();
        for id in audio_device_ids()? {
            if !device_has_output_channels(id)? {
                continue;
            }
            let name = string_property(id, kAudioDevicePropertyDeviceNameCFString)
                .unwrap_or_else(|_| format!("Output device {}", devices.len() + 1));
            let uid = string_property(id, kAudioDevicePropertyDeviceUID).ok();
            devices.push(CoreAudioOutputDevice { id, uid, name });
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

    fn default_output_device_id() -> Option<AudioDeviceID> {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
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

    fn device_has_output_channels(device_id: AudioDeviceID) -> Result<bool> {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyStreamConfiguration,
            mScope: kAudioObjectPropertyScopeOutput,
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
            "Failed to read CoreAudio output stream configuration",
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

    fn read_output_volume(device_id: AudioDeviceID) -> Result<f32> {
        let address = output_volume_address();
        if !has_property(device_id, &address) {
            return Err(anyhow!(
                "Default output device does not expose volume control"
            ));
        }
        let mut volume = 0.0f32;
        let mut data_size = mem::size_of::<f32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                null(),
                &mut data_size,
                (&mut volume as *mut f32).cast(),
            )
        };
        check_status(status, "Failed to read output volume")?;
        Ok(volume.clamp(0.0, 1.0))
    }

    fn write_output_volume(device_id: AudioDeviceID, volume: f32) -> Result<()> {
        let address = output_volume_address();
        if !has_property(device_id, &address) {
            return Err(anyhow!(
                "Default output device does not expose volume control"
            ));
        }
        if !property_is_settable(device_id, &address)? {
            return Err(anyhow!("Default output device volume is not settable"));
        }
        let volume = volume.clamp(0.0, 1.0);
        let status = unsafe {
            AudioObjectSetPropertyData(
                device_id,
                &address,
                0,
                null(),
                mem::size_of::<f32>() as u32,
                (&volume as *const f32).cast(),
            )
        };
        check_status(status, "Failed to set output volume")
    }

    fn output_volume_address() -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyVolumeScalar,
            mScope: kAudioObjectPropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMaster,
        }
    }

    fn has_property(device_id: AudioDeviceID, address: &AudioObjectPropertyAddress) -> bool {
        unsafe { AudioObjectHasProperty(device_id, address) != 0 }
    }

    fn property_is_settable(
        device_id: AudioDeviceID,
        address: &AudioObjectPropertyAddress,
    ) -> Result<bool> {
        let mut settable: Boolean = 0;
        let status = unsafe { AudioObjectIsPropertySettable(device_id, address, &mut settable) };
        check_status(status, "Failed to check output volume mutability")?;
        Ok(settable != 0)
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

    fn output_device_id(device: &CoreAudioOutputDevice) -> String {
        match device.uid.as_deref() {
            Some(uid) if !uid.is_empty() => format!("coreaudio:{uid}"),
            _ => format!("coreaudio-output:{}", device.id),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub struct OutputVolumeDuckingSession;

    impl OutputVolumeDuckingSession {
        pub fn restore(self) -> Result<()> {
            Ok(())
        }
    }

    pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>> {
        Ok(Vec::new())
    }

    pub fn start_ducking_session(
        _config: &OutputVolumeDuckingConfig,
    ) -> Result<Option<OutputVolumeDuckingSession>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_ducking_is_noop() {
        let config = OutputVolumeDuckingConfig {
            enabled: true,
            reduction_percent: 70,
            device_name_whitelist: Vec::new(),
        };

        assert!(start_ducking_session(&config).unwrap().is_none());
    }
}
