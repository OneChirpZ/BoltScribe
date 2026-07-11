use crate::{
    audio_devices, autostart, config, data_dir, fn_trigger, history, injector, output_volume,
    paths, recorder, shortcuts, tray, windows, workflow,
};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;
use tauri::{Emitter, State, Wry};
use tauri_plugin_dialog::DialogExt;

const AUDIO_DEVICE_REFRESH_TIMEOUT: Duration = Duration::from_secs(5);
const GITHUB_REPOSITORY_URL: &str = "https://github.com/OneChirpZ/BoltScribe";
static AUDIO_INPUT_DEVICE_REFRESH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static AUDIO_OUTPUT_DEVICE_REFRESH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub(crate) fn load_config() -> Result<config::AppConfig, String> {
    config::ConfigStore::load().map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn save_config(
    app: tauri::AppHandle<Wry>,
    config: config::AppConfig,
) -> Result<config::AppConfig, String> {
    apply_and_save_config(&app, config)
}

#[tauri::command]
pub(crate) fn export_config(config: config::AppConfig) -> Result<String, String> {
    config::ConfigStore::export_file(&config)
        .map(|path| path.display().to_string())
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn import_config(
    app: tauri::AppHandle<Wry>,
    raw: String,
) -> Result<config::ConfigImportResult, String> {
    let imported = config::ConfigStore::import_json(&raw).map_err(|err| err.to_string())?;
    let config = apply_and_save_config(&app, imported.config)?;
    Ok(config::ConfigImportResult {
        config,
        report: imported.report,
    })
}

#[tauri::command]
pub(crate) fn load_audio_input_devices() -> Result<Vec<audio_devices::AudioInputDevice>, String> {
    run_audio_device_refresh_with_timeout(
        "Audio input device refresh",
        "audio-input-device-refresh",
        &AUDIO_INPUT_DEVICE_REFRESH_IN_PROGRESS,
        AUDIO_DEVICE_REFRESH_TIMEOUT,
        audio_devices::list_input_devices,
    )
}

#[tauri::command]
pub(crate) fn load_audio_output_devices() -> Result<Vec<output_volume::AudioOutputDevice>, String> {
    run_audio_device_refresh_with_timeout(
        "Audio output device refresh",
        "audio-output-device-refresh",
        &AUDIO_OUTPUT_DEVICE_REFRESH_IN_PROGRESS,
        AUDIO_DEVICE_REFRESH_TIMEOUT,
        output_volume::list_output_devices,
    )
}

fn run_audio_device_refresh_with_timeout<T, F>(
    label: &'static str,
    thread_name: &'static str,
    in_progress: &'static AtomicBool,
    timeout: Duration,
    refresh: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    if in_progress.swap(true, Ordering::AcqRel) {
        return Err(format!(
            "{label} is already waiting for the system audio service; try again after it finishes or restart the audio service."
        ));
    }

    let (sender, receiver) = mpsc::channel();
    let spawn_result = std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || {
            let reset = AudioDeviceRefreshInProgressReset(in_progress);
            let result = refresh();
            drop(reset);
            let _ = sender.send(result);
        });

    if let Err(err) = spawn_result {
        in_progress.store(false, Ordering::Release);
        return Err(format!("{label} worker failed to start: {err}"));
    }

    match receiver.recv_timeout(timeout) {
        Ok(result) => result.map_err(|err| err.to_string()),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "{label} timed out after {}. The system audio service may be unresponsive; try again later or restart the audio service.",
            format_duration(timeout)
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(format!("{label} worker exited before returning"))
        }
    }
}

struct AudioDeviceRefreshInProgressReset(&'static AtomicBool);

impl Drop for AudioDeviceRefreshInProgressReset {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.subsec_millis() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

fn apply_and_save_config(
    app: &tauri::AppHandle<Wry>,
    config: config::AppConfig,
) -> Result<config::AppConfig, String> {
    let previous = config::ConfigStore::load().unwrap_or_default();
    let mut next = config;
    next.normalize();
    next.validate().map_err(|err| err.to_string())?;

    shortcuts::apply_global_shortcuts(app, &next).inspect_err(|_| {
        if let Err(err) = shortcuts::apply_global_shortcuts(app, &previous) {
            eprintln!("failed to restore previous shortcuts: {err}");
        }
    })?;
    autostart::apply_launch_at_login(next.system.launch_at_login).map_err(|err| {
        if let Err(restore_err) = shortcuts::apply_global_shortcuts(app, &previous) {
            eprintln!("failed to restore previous shortcuts after autostart error: {restore_err}");
        }
        err.to_string()
    })?;

    let saved = config::ConfigStore::save(&next).map_err(|err| {
        if let Err(restore_err) = shortcuts::apply_global_shortcuts(app, &previous) {
            eprintln!("failed to restore previous shortcuts after save error: {restore_err}");
        }
        if let Err(restore_err) = autostart::apply_launch_at_login(previous.system.launch_at_login)
        {
            eprintln!(
                "failed to restore previous autostart setting after save error: {restore_err}"
            );
        }
        err.to_string()
    })?;
    history::HistoryStore::prune(&saved.retention).map_err(|err| err.to_string())?;
    windows::sync_dock_visibility(app).map_err(|err| err.to_string())?;
    if let Err(err) = fn_trigger::apply(
        app,
        saved.system.fn_long_press_enabled,
        saved.system.fn_long_press_duration_ms,
    ) {
        eprintln!("failed to apply Fn long-press trigger: {err:?}");
    }
    if let Err(err) = tray::sync_llm_correction_label(app, saved.correction.enabled) {
        eprintln!("failed to sync tray LLM correction item: {err}");
    }
    let _ = app.emit("config://updated", &saved);
    Ok(saved)
}

#[tauri::command]
pub(crate) fn load_history(
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<history::HistoryRecord>, String> {
    history::HistoryStore::load(limit.unwrap_or(100), offset.unwrap_or(0))
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn load_stats() -> Result<history::InputStats, String> {
    history::HistoryStore::stats().map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn get_status(state: State<'_, workflow::AppState>) -> workflow::WorkflowStatus {
    state.status()
}

#[tauri::command]
pub(crate) fn toggle_recording(
    app: tauri::AppHandle,
    state: State<'_, workflow::AppState>,
) -> Result<workflow::WorkflowStatus, String> {
    workflow::toggle_recording(app, state.inner()).map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn cancel_current_workflow(
    app: tauri::AppHandle,
    state: State<'_, workflow::AppState>,
) -> Result<workflow::WorkflowStatus, String> {
    workflow::cancel_current_workflow(app, state.inner()).map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn open_app_dir() -> Result<(), String> {
    let dir = paths::app_dir()?;
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    open_path(&dir).map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command]
pub(crate) fn open_github_repository() -> Result<(), String> {
    open_url(GITHUB_REPOSITORY_URL).map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command]
pub(crate) fn get_data_dir() -> Result<data_dir::DataDirInfo, String> {
    data_dir::info().map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) async fn choose_data_dir(app: tauri::AppHandle<Wry>) -> Result<Option<String>, String> {
    app.dialog()
        .file()
        .set_title("Select BoltScribe Data Folder")
        .blocking_pick_folder()
        .map(|path| {
            path.into_path()
                .map(|path| path.display().to_string())
                .map_err(|err| err.to_string())
        })
        .transpose()
}

#[tauri::command]
pub(crate) fn set_data_dir(
    path: String,
    state: State<'_, workflow::AppState>,
) -> Result<data_dir::DataDirInfo, String> {
    state
        .run_while_inactive(|| data_dir::set_data_dir(PathBuf::from(path.trim())))
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn reset_data_dir(
    state: State<'_, workflow::AppState>,
) -> Result<data_dir::DataDirInfo, String> {
    state
        .run_while_inactive(data_dir::reset_data_dir)
        .map_err(|err| err.to_string())
}

#[cfg(target_os = "macos")]
fn open_path(path: &Path) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("open").arg(path).status()
}

#[cfg(target_os = "windows")]
fn open_path(path: &Path) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("explorer").arg(path).status()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_path(path: &Path) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("xdg-open").arg(path).status()
}

#[cfg(target_os = "macos")]
fn open_url(url: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("open").arg(url).status()
}

#[cfg(target_os = "windows")]
fn open_url(url: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("rundll32")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .status()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_url(url: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("xdg-open").arg(url).status()
}

#[tauri::command]
pub(crate) fn hide_main_window(app: tauri::AppHandle<Wry>) -> Result<(), String> {
    windows::hide_main_window(&app).map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn accessibility_permission_granted() -> bool {
    injector::accessibility_permission_granted()
}

#[tauri::command]
pub(crate) fn request_accessibility_permission() -> bool {
    injector::request_accessibility_permission()
}

#[tauri::command]
pub(crate) fn open_accessibility_settings() -> Result<(), String> {
    injector::open_accessibility_settings().map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn input_monitoring_permission_granted(app: tauri::AppHandle<Wry>) -> bool {
    let granted = fn_trigger::input_monitoring_permission_granted();
    if granted {
        apply_saved_fn_trigger(&app);
    }
    granted
}

#[tauri::command]
pub(crate) fn request_input_monitoring_permission(app: tauri::AppHandle<Wry>) -> bool {
    let granted = fn_trigger::request_input_monitoring_permission();
    if granted {
        apply_saved_fn_trigger(&app);
    }
    granted
}

#[tauri::command]
pub(crate) fn apply_fn_trigger(
    app: tauri::AppHandle<Wry>,
    enabled: bool,
    long_press_duration_ms: u64,
) -> Result<(), String> {
    fn_trigger::apply(&app, enabled, long_press_duration_ms).map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn open_input_monitoring_settings() -> Result<(), String> {
    fn_trigger::open_input_monitoring_settings().map_err(|err| err.to_string())
}

fn apply_saved_fn_trigger(app: &tauri::AppHandle<Wry>) {
    let config = config::ConfigStore::load().unwrap_or_default();
    if !config.system.fn_long_press_enabled {
        return;
    }
    if let Err(err) = fn_trigger::apply(app, true, config.system.fn_long_press_duration_ms) {
        eprintln!("failed to apply Fn long-press trigger after permission grant: {err:?}");
    }
}

#[tauri::command]
pub(crate) fn request_microphone_permission() -> Result<bool, String> {
    recorder::request_microphone_permission().map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn copy_text_to_clipboard(text: String) -> Result<(), String> {
    injector::copy_text(&text).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_device_refresh_timeout_returns_error() {
        static IN_PROGRESS: AtomicBool = AtomicBool::new(false);
        IN_PROGRESS.store(false, Ordering::Release);

        let error = run_audio_device_refresh_with_timeout(
            "Test audio device refresh",
            "test-audio-device-refresh-timeout",
            &IN_PROGRESS,
            Duration::from_millis(5),
            || {
                std::thread::sleep(Duration::from_millis(40));
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.contains("timed out"));
        assert!(IN_PROGRESS.load(Ordering::Acquire));
        wait_until_not_in_progress(&IN_PROGRESS);
        assert!(!IN_PROGRESS.load(Ordering::Acquire));
    }

    #[test]
    fn audio_device_refresh_rejects_duplicate_waiter() {
        static IN_PROGRESS: AtomicBool = AtomicBool::new(false);
        IN_PROGRESS.store(true, Ordering::Release);

        let error = run_audio_device_refresh_with_timeout(
            "Test audio device refresh",
            "test-audio-device-refresh-duplicate",
            &IN_PROGRESS,
            Duration::from_secs(1),
            || Ok::<_, anyhow::Error>(()),
        )
        .unwrap_err();

        assert!(error.contains("already waiting"));
        IN_PROGRESS.store(false, Ordering::Release);
    }

    #[test]
    fn audio_device_refresh_success_clears_in_progress_marker() {
        static IN_PROGRESS: AtomicBool = AtomicBool::new(false);
        IN_PROGRESS.store(false, Ordering::Release);

        let value = run_audio_device_refresh_with_timeout(
            "Test audio device refresh",
            "test-audio-device-refresh-success",
            &IN_PROGRESS,
            Duration::from_secs(1),
            || Ok::<_, anyhow::Error>(7),
        )
        .unwrap();

        assert_eq!(value, 7);
        assert!(!IN_PROGRESS.load(Ordering::Acquire));
    }

    #[test]
    fn audio_device_refresh_error_clears_in_progress_marker() {
        static IN_PROGRESS: AtomicBool = AtomicBool::new(false);
        IN_PROGRESS.store(false, Ordering::Release);

        let error = run_audio_device_refresh_with_timeout::<(), _>(
            "Test audio device refresh",
            "test-audio-device-refresh-error",
            &IN_PROGRESS,
            Duration::from_secs(1),
            || Err(anyhow::anyhow!("system audio service failed")),
        )
        .unwrap_err();

        assert!(error.contains("system audio service failed"));
        assert!(!IN_PROGRESS.load(Ordering::Acquire));
    }

    fn wait_until_not_in_progress(in_progress: &AtomicBool) {
        let started_at = std::time::Instant::now();
        while in_progress.load(Ordering::Acquire) {
            if started_at.elapsed() > Duration::from_secs(1) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
