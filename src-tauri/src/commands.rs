use crate::{
    audio_devices, autostart, config, history, injector, output_volume, paths, recorder, shortcuts,
    tray, windows, workflow,
};
use std::path::Path;
use tauri::{Emitter, State, Wry};

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
    audio_devices::list_input_devices().map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn load_audio_output_devices() -> Result<Vec<output_volume::AudioOutputDevice>, String> {
    output_volume::list_output_devices().map_err(|err| err.to_string())
}

fn apply_and_save_config(
    app: &tauri::AppHandle<Wry>,
    config: config::AppConfig,
) -> Result<config::AppConfig, String> {
    let previous = config::ConfigStore::load().unwrap_or_default();
    let mut next = config;
    next.normalize();

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
pub(crate) fn request_microphone_permission() -> Result<bool, String> {
    recorder::request_microphone_permission().map_err(|err| err.to_string())
}

#[tauri::command]
pub(crate) fn copy_text_to_clipboard(text: String) -> Result<(), String> {
    injector::copy_text(&text).map_err(|err| err.to_string())
}
