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
        kAudioDevicePropertyMute, kAudioDevicePropertyStreamConfiguration,
        kAudioDevicePropertyVolumeScalar, kAudioHardwareNoError,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMaster, kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, AudioBuffer, AudioBufferList,
        AudioDeviceID, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
        AudioObjectHasProperty, AudioObjectIsPropertySettable, AudioObjectPropertyAddress,
        AudioObjectSetPropertyData, Boolean, CFStringRef, OSStatus,
    };
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::ffi::CStr;
    use std::mem;
    use std::process::{Command, Output, Stdio};
    use std::ptr::null;
    use std::slice;
    use std::thread;
    use std::time::{Duration, Instant};

    const VIRTUAL_MASTER_VOLUME_SELECTOR: u32 = four_char_code(*b"vmvc");
    const SOUNDSOURCE_DUCK_SHORTCUT_NAME: &str = "BoltScribe SoundSource Duck";
    const SOUNDSOURCE_RESTORE_SHORTCUT_NAME: &str = "BoltScribe SoundSource Restore";
    const SOUNDSOURCE_SHORTCUT_TIMEOUT: Duration = Duration::from_secs(3);

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

    #[derive(Clone, Copy)]
    enum DuckingControl {
        Volume(VolumeControl),
        Mute(MuteControl),
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
        SoundSource {
            restore_payload: Value,
        },
    }

    #[derive(Debug, Deserialize)]
    struct SoundSourceDuckResponse {
        #[serde(default)]
        applied: bool,
        #[serde(default)]
        restore_payload: Option<Value>,
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
                DuckingSessionState::SoundSource { restore_payload } => {
                    restore_soundsource_ducking(restore_payload)
                }
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

        match start_native_ducking_session(device_id, device_name.clone(), config) {
            Ok(session) => return Ok(session),
            Err(err) if config.soundsource_enabled => {
                eprintln!("native output volume ducking unavailable; trying SoundSource: {err:?}");
            }
            Err(err) => return Err(err),
        }

        start_soundsource_ducking_session(&device_name, config)
    }

    fn start_native_ducking_session(
        device_id: AudioDeviceID,
        device_name: String,
        config: &OutputVolumeDuckingConfig,
    ) -> Result<Option<OutputVolumeDuckingSession>> {
        let control = volume_control_for_device(device_id)?
            .map(DuckingControl::Volume)
            .or(mute_control_for_device(device_id)?.map(DuckingControl::Mute))
            .ok_or_else(|| {
                anyhow!("Default output device does not expose writable volume or mute control")
            })?;
        match control {
            DuckingControl::Volume(control) => {
                let original_volume = read_output_volume(device_id, control)?;
                let ducked_volume = ducked_volume(original_volume, config.reduction_percent);
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
            DuckingControl::Mute(control) => {
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
        }
    }

    fn start_soundsource_ducking_session(
        device_name: &str,
        config: &OutputVolumeDuckingConfig,
    ) -> Result<Option<OutputVolumeDuckingSession>> {
        let request = json!({
            "action": "duck",
            "backend": "soundsource-shortcuts",
            "version": 1,
            "source_name": "Output",
            "device_name": device_name,
            "reduction_percent": config.reduction_percent.clamp(0, 100),
            "restore_shortcut": SOUNDSOURCE_RESTORE_SHORTCUT_NAME,
        });
        let output = run_shortcut(SOUNDSOURCE_DUCK_SHORTCUT_NAME, &request)
            .with_context(|| format!("Failed to run {SOUNDSOURCE_DUCK_SHORTCUT_NAME}"))?;
        let response = parse_soundsource_duck_response(&output)?;
        if !response.applied {
            return Ok(None);
        }
        let restore_payload = response
            .restore_payload
            .ok_or_else(|| anyhow!("SoundSource duck shortcut did not return restore_payload"))?;
        Ok(Some(OutputVolumeDuckingSession {
            state: DuckingSessionState::SoundSource { restore_payload },
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

    fn restore_soundsource_ducking(restore_payload: Value) -> Result<()> {
        run_shortcut(SOUNDSOURCE_RESTORE_SHORTCUT_NAME, &restore_payload)
            .with_context(|| format!("Failed to run {SOUNDSOURCE_RESTORE_SHORTCUT_NAME}"))?;
        Ok(())
    }

    fn parse_soundsource_duck_response(output: &str) -> Result<SoundSourceDuckResponse> {
        let output = output.trim();
        if output.is_empty() {
            return Err(anyhow!("SoundSource duck shortcut returned empty output"));
        }
        serde_json::from_str(output)
            .with_context(|| format!("SoundSource duck shortcut returned invalid JSON: {output}"))
    }

    fn run_shortcut(name: &str, payload: &Value) -> Result<String> {
        let payload = serde_json::to_string(payload)?;
        let script = r#"
on run argv
  set shortcutName to item 1 of argv
  set shortcutInput to item 2 of argv
  tell application "Shortcuts Events"
    set shortcutResult to run shortcut shortcutName with input shortcutInput
  end tell
  return shortcutResult as text
end run
"#;
        let mut command = Command::new("osascript");
        command
            .arg("-e")
            .arg(script)
            .arg(name)
            .arg(payload)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = run_with_timeout(command, SOUNDSOURCE_SHORTCUT_TIMEOUT)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(anyhow!(
                "Shortcut {name} failed: {}",
                if stderr.is_empty() {
                    format!("exit status {}", output.status)
                } else {
                    stderr
                }
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<Output> {
        let mut child = command.spawn().context("Failed to start shortcut runner")?;
        let started = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                return child
                    .wait_with_output()
                    .context("Failed to read shortcut runner output");
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!(
                    "Shortcut runner timed out after {}s",
                    timeout.as_secs()
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
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
