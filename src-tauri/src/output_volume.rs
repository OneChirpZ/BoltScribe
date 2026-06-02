use crate::config::OutputVolumeDuckingConfig;
use anyhow::Result;
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

fn target_volume(original_volume: f32, config: &OutputVolumeDuckingConfig) -> f32 {
    if config.mute_instead_of_reduce {
        0.0
    } else {
        ducked_volume(original_volume, config.reduction_percent)
    }
}

fn volume_matches(left: f32, right: f32) -> bool {
    (left - right).abs() <= 0.01
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuckingStrategy {
    Volume,
    NativeMute,
    SoundSourceHotkey,
    Unsupported,
}

#[cfg(any(target_os = "macos", test))]
fn ducking_strategy(
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
    } else {
        if supports_volume_control {
            DuckingStrategy::Volume
        } else if supports_mute_control {
            DuckingStrategy::NativeMute
        } else if sound_source_hotkey_fallback_enabled {
            DuckingStrategy::SoundSourceHotkey
        } else {
            DuckingStrategy::Unsupported
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use crate::keyboard_shortcut;
    use anyhow::{anyhow, Context, Result};
    use core_foundation_sys::string::{
        kCFStringEncodingUTF8, CFStringGetCString, CFStringGetCStringPtr,
        CFStringRef as CoreFoundationStringRef,
    };
    use coreaudio_sys::{
        kAudioDevicePropertyDeviceNameCFString, kAudioDevicePropertyDeviceUID,
        kAudioDevicePropertyMute, kAudioDevicePropertyStreamConfiguration,
        kAudioDevicePropertyVolumeScalar, kAudioHardwareNoError,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMaster, kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, AudioBuffer, AudioBufferList,
        AudioDeviceID, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
        AudioObjectHasProperty, AudioObjectIsPropertySettable, AudioObjectPropertyAddress,
        AudioObjectSetPropertyData, Boolean, CFStringRef, OSStatus,
    };
    use std::ffi::CStr;
    use std::mem;
    use std::ptr::null;
    use std::slice;

    const VIRTUAL_MASTER_VOLUME_SELECTOR: u32 = four_char_code(*b"vmvc");

    const fn four_char_code(bytes: [u8; 4]) -> u32 {
        ((bytes[0] as u32) << 24)
            | ((bytes[1] as u32) << 16)
            | ((bytes[2] as u32) << 8)
            | (bytes[3] as u32)
    }

    struct CoreAudioOutputDevice {
        id: AudioDeviceID,
        uid: Option<String>,
        name: String,
    }

    #[derive(Clone, Copy)]
    struct VolumeControl {
        selector: u32,
        scope: u32,
        element: u32,
    }

    impl VolumeControl {
        fn address(self) -> AudioObjectPropertyAddress {
            AudioObjectPropertyAddress {
                mSelector: self.selector,
                mScope: self.scope,
                mElement: self.element,
            }
        }
    }

    #[derive(Clone, Copy)]
    struct MuteControl {
        scope: u32,
        element: u32,
    }

    impl MuteControl {
        fn address(self) -> AudioObjectPropertyAddress {
            AudioObjectPropertyAddress {
                mSelector: kAudioDevicePropertyMute,
                mScope: self.scope,
                mElement: self.element,
            }
        }
    }

    pub struct OutputVolumeDuckingSession {
        state: DuckingSessionState,
    }

    enum DuckingSessionState {
        Volume {
            device_id: AudioDeviceID,
            device_name: String,
            control: VolumeControl,
            original_volume: f32,
            ducked_volume: f32,
        },
        Mute {
            device_id: AudioDeviceID,
            device_name: String,
            control: MuteControl,
        },
        SoundSourceHotkey {
            device_name: String,
            hotkey: String,
        },
    }

    impl OutputVolumeDuckingSession {
        pub fn restore(self) -> Result<()> {
            match self.state {
                DuckingSessionState::Volume {
                    device_id,
                    device_name,
                    control,
                    original_volume,
                    ducked_volume,
                } => restore_volume(
                    device_id,
                    &device_name,
                    control,
                    original_volume,
                    ducked_volume,
                ),
                DuckingSessionState::Mute {
                    device_id,
                    device_name,
                    control,
                } => restore_mute(device_id, &device_name, control),
                DuckingSessionState::SoundSourceHotkey {
                    device_name,
                    hotkey,
                } => restore_sound_source_hotkey(&device_name, &hotkey),
            }
        }
    }

    pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>> {
        let default_id = default_output_device_id();
        Ok(coreaudio_output_devices()?
            .into_iter()
            .map(|device| {
                let supports_volume_control = match volume_control_for_device(device.id) {
                    Ok(Some(_)) => true,
                    _ => false,
                };
                let supports_mute_control = match mute_control_for_device(device.id) {
                    Ok(Some(_)) => true,
                    _ => false,
                };
                AudioOutputDevice {
                    id: output_device_id(&device),
                    name: device.name,
                    is_default: default_id == Some(device.id),
                    platform: "macos".to_string(),
                    supports_volume_control,
                    supports_mute_control,
                }
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

        let volume_control = volume_control_for_device(device_id)?;
        let mute_control = mute_control_for_device(device_id)?;
        match ducking_strategy(
            volume_control.is_some(),
            mute_control.is_some(),
            config.mute_instead_of_reduce,
            config.sound_source_hotkey_fallback_enabled,
        ) {
            DuckingStrategy::Volume => {
                let control = volume_control.expect("volume control exists for volume strategy");
                let original_volume = read_output_volume(device_id, control)?;
                let ducked_volume = target_volume(original_volume, config);
                if volume_matches(original_volume, ducked_volume) {
                    return Ok(None);
                }
                write_output_volume(device_id, control, ducked_volume)?;
                Ok(Some(OutputVolumeDuckingSession {
                    state: DuckingSessionState::Volume {
                        device_id,
                        device_name,
                        control,
                        original_volume,
                        ducked_volume,
                    },
                }))
            }
            DuckingStrategy::NativeMute => {
                let control = mute_control.expect("mute control exists for mute strategy");
                if read_output_mute(device_id, control)? {
                    return Ok(None);
                }
                write_output_mute(device_id, control, true)?;
                Ok(Some(OutputVolumeDuckingSession {
                    state: DuckingSessionState::Mute {
                        device_id,
                        device_name,
                        control,
                    },
                }))
            }
            DuckingStrategy::SoundSourceHotkey => {
                keyboard_shortcut::send(&config.sound_source_toggle_mute_hotkey).with_context(
                    || format!("Failed to send SoundSource mute shortcut for {device_name}"),
                )?;
                Ok(Some(OutputVolumeDuckingSession {
                    state: DuckingSessionState::SoundSourceHotkey {
                        device_name,
                        hotkey: config.sound_source_toggle_mute_hotkey.clone(),
                    },
                }))
            }
            DuckingStrategy::Unsupported => Err(anyhow!(
                "Default output device does not expose writable volume or mute control"
            )),
        }
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

    fn restore_volume(
        device_id: AudioDeviceID,
        device_name: &str,
        control: VolumeControl,
        original_volume: f32,
        ducked_volume: f32,
    ) -> Result<()> {
        let current_volume = read_output_volume(device_id, control)
            .with_context(|| format!("Failed to read output volume for {device_name}"))?;
        if !volume_matches(current_volume, ducked_volume) {
            eprintln!("output volume changed during recording; skipping restore for {device_name}",);
            return Ok(());
        }
        write_output_volume(device_id, control, original_volume)
            .with_context(|| format!("Failed to restore output volume for {device_name}"))
    }

    fn restore_mute(
        device_id: AudioDeviceID,
        device_name: &str,
        control: MuteControl,
    ) -> Result<()> {
        let current_muted = read_output_mute(device_id, control)
            .with_context(|| format!("Failed to read output mute for {device_name}"))?;
        if !current_muted {
            eprintln!("output mute changed during recording; skipping restore for {device_name}",);
            return Ok(());
        }
        write_output_mute(device_id, control, false)
            .with_context(|| format!("Failed to restore output mute for {device_name}"))
    }

    fn restore_sound_source_hotkey(device_name: &str, hotkey: &str) -> Result<()> {
        keyboard_shortcut::send(hotkey).with_context(|| {
            format!("Failed to restore SoundSource mute shortcut for {device_name}")
        })
    }

    fn volume_control_for_device(device_id: AudioDeviceID) -> Result<Option<VolumeControl>> {
        for control in volume_control_candidates() {
            let address = control.address();
            if !has_property(device_id, &address) || !property_is_settable(device_id, &address)? {
                continue;
            }
            if read_output_volume(device_id, control).is_ok() {
                return Ok(Some(control));
            }
        }
        Ok(None)
    }

    fn volume_control_candidates() -> [VolumeControl; 4] {
        [
            VolumeControl {
                selector: kAudioDevicePropertyVolumeScalar,
                scope: kAudioObjectPropertyScopeOutput,
                element: kAudioObjectPropertyElementMaster,
            },
            VolumeControl {
                selector: VIRTUAL_MASTER_VOLUME_SELECTOR,
                scope: kAudioObjectPropertyScopeOutput,
                element: kAudioObjectPropertyElementMaster,
            },
            VolumeControl {
                selector: kAudioDevicePropertyVolumeScalar,
                scope: kAudioObjectPropertyScopeGlobal,
                element: kAudioObjectPropertyElementMaster,
            },
            VolumeControl {
                selector: VIRTUAL_MASTER_VOLUME_SELECTOR,
                scope: kAudioObjectPropertyScopeGlobal,
                element: kAudioObjectPropertyElementMaster,
            },
        ]
    }

    fn mute_control_for_device(device_id: AudioDeviceID) -> Result<Option<MuteControl>> {
        for control in mute_control_candidates() {
            let address = control.address();
            if !has_property(device_id, &address) || !property_is_settable(device_id, &address)? {
                continue;
            }
            if read_output_mute(device_id, control).is_ok() {
                return Ok(Some(control));
            }
        }
        Ok(None)
    }

    fn mute_control_candidates() -> [MuteControl; 2] {
        [
            MuteControl {
                scope: kAudioObjectPropertyScopeOutput,
                element: kAudioObjectPropertyElementMaster,
            },
            MuteControl {
                scope: kAudioObjectPropertyScopeGlobal,
                element: kAudioObjectPropertyElementMaster,
            },
        ]
    }

    fn read_output_volume(device_id: AudioDeviceID, control: VolumeControl) -> Result<f32> {
        let address = control.address();
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

    fn write_output_volume(
        device_id: AudioDeviceID,
        control: VolumeControl,
        volume: f32,
    ) -> Result<()> {
        let address = control.address();
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

    fn read_output_mute(device_id: AudioDeviceID, control: MuteControl) -> Result<bool> {
        let address = control.address();
        if !has_property(device_id, &address) {
            return Err(anyhow!(
                "Default output device does not expose mute control"
            ));
        }
        let mut muted = 0u32;
        let mut data_size = mem::size_of::<u32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                null(),
                &mut data_size,
                (&mut muted as *mut u32).cast(),
            )
        };
        check_status(status, "Failed to read output mute")?;
        Ok(muted != 0)
    }

    fn write_output_mute(
        device_id: AudioDeviceID,
        control: MuteControl,
        muted: bool,
    ) -> Result<()> {
        let address = control.address();
        if !has_property(device_id, &address) {
            return Err(anyhow!(
                "Default output device does not expose mute control"
            ));
        }
        if !property_is_settable(device_id, &address)? {
            return Err(anyhow!("Default output device mute is not settable"));
        }
        let muted = u32::from(muted);
        let status = unsafe {
            AudioObjectSetPropertyData(
                device_id,
                &address,
                0,
                null(),
                mem::size_of::<u32>() as u32,
                (&muted as *const u32).cast(),
            )
        };
        check_status(status, "Failed to set output mute")
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

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use anyhow::{anyhow, Context, Result};
    use std::ffi::c_void;
    use std::ptr::null;
    use std::slice;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Devices::Properties::DEVPKEY_Device_FriendlyName;
    use windows::Win32::Foundation::{BOOL, RPC_E_CHANGED_MODE};
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;

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
                    eprintln!(
                        "failed to read Windows output mute, trying volume fallback: {err:?}"
                    );
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

    fn property_string(value: &windows::core::PROPVARIANT) -> Result<String> {
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
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
}
