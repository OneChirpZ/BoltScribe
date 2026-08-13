use crate::config::AudioConfig;
use crate::vad_test_session::{idle_status, VadTestSession};
use anyhow::{anyhow, Result};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub use crate::vad_test_session::VadTestStatus;

pub fn emit_status(app: &AppHandle, status: VadTestStatus) {
    let _ = app.emit("vad-test://status", status);
}

pub fn start(
    app: AppHandle,
    state: &crate::workflow::AppState,
    audio_config: AudioConfig,
) -> Result<VadTestStatus> {
    {
        let runtime = state
            .runtime_for_vad_test()
            .map_err(|err| anyhow!("{err}"))?;
        if runtime {
            return Err(anyhow!(
                "Cannot start VAD test while recording or processing"
            ));
        }
    }
    let mut slot = state.vad_test_slot().map_err(|err| anyhow!("{err}"))?;
    if slot.is_some() {
        return Err(anyhow!("VAD test is already active"));
    }
    let session = VadTestSession::start(state.recorder_ref(), audio_config)?;
    let status = session.status();
    *slot = Some(session);
    drop(slot);
    emit_status(&app, status.clone());
    spawn_monitor(app);
    Ok(status)
}

pub fn stop(app: AppHandle, state: &crate::workflow::AppState) -> Result<VadTestStatus> {
    let mut slot = state.vad_test_slot().map_err(|err| anyhow!("{err}"))?;
    let Some(session) = slot.take() else {
        let status = idle_status();
        emit_status(&app, status.clone());
        return Ok(status);
    };
    session.stop(state.recorder_ref());
    let idle = idle_status();
    emit_status(&app, idle.clone());
    Ok(idle)
}

pub fn update_settings(
    app: AppHandle,
    state: &crate::workflow::AppState,
    noise_margin_db: u32,
    confirmation_ms: u32,
    noise_window_ms: u32,
) -> Result<VadTestStatus> {
    let slot = state.vad_test_slot().map_err(|err| anyhow!("{err}"))?;
    let Some(session) = slot.as_ref() else {
        return Err(anyhow!("VAD test is not active"));
    };
    session.update_settings(noise_margin_db, confirmation_ms, noise_window_ms)?;
    let status = session.status();
    emit_status(&app, status.clone());
    Ok(status)
}

pub fn get_status(state: &crate::workflow::AppState) -> VadTestStatus {
    state
        .vad_test_slot()
        .ok()
        .and_then(|slot| slot.as_ref().map(VadTestSession::status))
        .unwrap_or_default()
}

fn spawn_monitor(app: AppHandle) {
    std::thread::Builder::new()
        .name("boltscribe-vad-test-monitor".to_string())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let Some(state) = app.try_state::<crate::workflow::AppState>() else {
                return;
            };
            let expired = state
                .vad_test_slot()
                .ok()
                .and_then(|slot| slot.as_ref().map(VadTestSession::should_expire))
                .unwrap_or(false);
            if expired {
                let _ = stop(app.clone(), state.inner());
                return;
            }
            let status = get_status(state.inner());
            if status.mode == "idle" {
                return;
            }
            emit_status(&app, status);
        })
        .ok();
}
