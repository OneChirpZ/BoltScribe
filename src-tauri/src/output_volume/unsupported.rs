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
