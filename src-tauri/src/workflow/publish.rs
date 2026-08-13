use super::state::*;
use super::status::AudioLevelSample;
use super::*;

pub(super) fn set_status_for_task(app: &AppHandle, task_id: &str, status: WorkflowStatus) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    let Ok(mut runtime) = state.runtime.lock() else {
        return false;
    };
    let Some(status) = runtime.update_task_status(task_id, status) else {
        return false;
    };
    drop(runtime);
    publish_status(app, &status);
    true
}

pub(super) fn publish_status(app: &AppHandle, status: &WorkflowStatus) {
    if !status_is_current(app, status.revision) {
        return;
    }
    crate::windows::sync_overlay_window(app, status);
    if !status_is_current(app, status.revision) {
        return;
    }
    if let Err(err) = crate::tray::sync_voice_input_label(app, status) {
        eprintln!("failed to sync tray voice input item: {err}");
    }
    if !status_is_current(app, status.revision) {
        return;
    }
    if let Err(err) = app.emit("workflow://status", status) {
        eprintln!("failed to emit workflow status: {err}");
    }
}

pub(super) fn status_is_current(app: &AppHandle, revision: u64) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return true;
    };
    state
        .runtime
        .lock()
        .map(|runtime| runtime.status.revision == revision)
        .unwrap_or(false)
}

// Keep the measured DJI Mic Mini idle floor hidden while allowing low-output
// receivers such as Wireless Mic Rx to show quiet speech. The response curve
// is purely instantaneous: it does not retain prior samples or smooth changes
// over time.
const AUDIO_LEVEL_FLOOR_DBFS: f32 = -42.5;
const AUDIO_LEVEL_CEILING_DBFS: f32 = 0.0;
const AUDIO_LEVEL_RESPONSE_EXPONENT: f32 = 0.7;

pub(super) fn dbfs_to_display_level(dbfs: Option<f32>) -> f32 {
    let Some(dbfs) = dbfs.filter(|level| level.is_finite()) else {
        return 0.0;
    };
    let normalized = ((dbfs - AUDIO_LEVEL_FLOOR_DBFS)
        / (AUDIO_LEVEL_CEILING_DBFS - AUDIO_LEVEL_FLOOR_DBFS))
        .clamp(0.0, 1.0);
    normalized.powf(AUDIO_LEVEL_RESPONSE_EXPONENT)
}

pub(super) fn audio_level_session_revision(
    runtime: &WorkflowRuntime,
    task_id: &str,
) -> Option<u64> {
    matches!(
        &runtime.activity,
        WorkflowActivity::Recording(session)
            if session.task.id == task_id && session.audio_level_meter.is_active()
    )
    .then_some(runtime.status.revision)
}

pub(super) fn current_audio_level_session_revision(app: &AppHandle, task_id: &str) -> Option<u64> {
    let state = app.try_state::<AppState>()?;
    state
        .runtime
        .lock()
        .ok()
        .and_then(|runtime| audio_level_session_revision(&runtime, task_id))
}

pub(super) fn spawn_audio_level_emitter(app: AppHandle, meter: AudioLevelMeter, task_id: String) {
    std::thread::spawn(move || {
        while meter.is_active() {
            if current_audio_level_session_revision(&app, &task_id).is_none() {
                break;
            }
            // Each event represents the most recent 50 ms measurement window.
            // Do not add attack, release, hold, or other temporal smoothing.
            let level = dbfs_to_display_level(meter.take_level());
            // The frontend listener uses Tauri's global `Any` target. A targeted
            // `emit_to("overlay", ...)` does not reach that listener in Tauri 2.
            let Some(recording_revision) = current_audio_level_session_revision(&app, &task_id)
            else {
                break;
            };
            let _ = app.emit(
                "audio://level",
                AudioLevelSample {
                    level,
                    recording_revision,
                },
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Some(recording_revision) = current_audio_level_session_revision(&app, &task_id) {
            let _ = app.emit(
                "audio://level",
                AudioLevelSample {
                    level: 0.0,
                    recording_revision,
                },
            );
        }
    });
}
