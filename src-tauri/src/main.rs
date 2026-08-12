#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod asr;
mod audio_devices;
mod autostart;
mod commands;
mod config;
mod corrector;
mod data_dir;
mod fn_trigger;
mod history;
mod injector;
mod keyboard_shortcut;
mod mouse_shortcuts;
mod output_volume;
mod paths;
mod recorder;
mod shortcuts;
#[cfg(target_os = "macos")]
mod startup_single_instance;
mod system_audio;
mod tray;
mod vad;
mod vad_test;
mod windows;
mod workflow;

use commands::{
    accessibility_permission_granted, apply_fn_trigger, cancel_current_workflow, choose_data_dir,
    cleanup_recording_files, copy_text_to_clipboard, delete_history_record, export_config,
    get_data_dir, get_status, get_vad_test_status, hide_main_window, import_config,
    input_monitoring_permission_granted, load_audio_input_devices, load_audio_output_devices,
    load_config, load_history, load_stats, open_accessibility_settings, open_app_dir,
    open_github_repository, open_input_monitoring_settings, preview_recording_cleanup,
    request_accessibility_permission, request_input_monitoring_permission,
    request_microphone_permission, reset_data_dir, restart_system_audio_service,
    retry_history_record, save_config, set_data_dir, start_vad_test, stop_vad_test,
    toggle_recording, update_vad_test_settings,
};
use tauri::{Emitter, RunEvent};

fn main() {
    #[cfg(target_os = "macos")]
    let builder =
        tauri::Builder::default().plugin(startup_single_instance::startup_race_guard_plugin());
    #[cfg(not(target_os = "macos"))]
    let builder = tauri::Builder::default();

    // Single-instance must be the first plugin so a second process cannot
    // create another global shortcut listener and overlay window. On macOS the
    // startup race guard above serializes simultaneous launches before this
    // plugin creates its Unix socket.
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Err(err) = windows::show_main_window(app) {
            eprintln!("failed to show main window from second instance: {err:?}");
        }
    }));
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    let app = builder
        .plugin(shortcuts::global_shortcut_plugin())
        .plugin(tauri_plugin_dialog::init())
        .manage(workflow::AppState::default())
        .setup(|app| {
            let config = config::ConfigStore::load().unwrap_or_default();
            windows::sync_app_theme(app.handle(), &config.ui.theme);
            windows::ensure_main_window(app.handle())
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;
            windows::ensure_overlay_window(app.handle())
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;
            tray::setup(app.handle()).map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;
            if let Err(err) = history::HistoryStore::prune(&config.retention) {
                eprintln!("failed to apply startup history retention: {err:#}");
            }
            if config.system.launch_at_login {
                if let Err(err) = autostart::apply_launch_at_login(true) {
                    eprintln!("failed to refresh launch-at-login registration: {err:?}");
                }
            }
            if let Err(err) = shortcuts::apply_startup_mouse_shortcuts(app.handle(), &config) {
                eprintln!("failed to apply startup mouse shortcuts: {err}");
            }
            if let Err(err) = fn_trigger::apply(
                app.handle(),
                config.system.fn_long_press_enabled,
                config.system.fn_long_press_duration_ms,
            ) {
                eprintln!("failed to apply Fn long-press trigger: {err:?}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(err) = window.emit("config://close-requested", ()) {
                    eprintln!("failed to request save before close: {err:?}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            export_config,
            import_config,
            load_audio_input_devices,
            load_audio_output_devices,
            restart_system_audio_service,
            load_history,
            load_stats,
            retry_history_record,
            delete_history_record,
            cleanup_recording_files,
            preview_recording_cleanup,
            get_status,
            toggle_recording,
            cancel_current_workflow,
            open_app_dir,
            open_github_repository,
            get_data_dir,
            choose_data_dir,
            set_data_dir,
            reset_data_dir,
            hide_main_window,
            accessibility_permission_granted,
            request_accessibility_permission,
            open_accessibility_settings,
            input_monitoring_permission_granted,
            request_input_monitoring_permission,
            apply_fn_trigger,
            open_input_monitoring_settings,
            request_microphone_permission,
            start_vad_test,
            update_vad_test_settings,
            stop_vad_test,
            get_vad_test_status,
            copy_text_to_clipboard
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app, event| match event {
        RunEvent::Ready => {
            if let Err(err) = windows::ensure_main_window(app) {
                eprintln!("failed to prepare main window: {err:?}");
            }
            if let Err(err) = windows::ensure_overlay_window(app) {
                eprintln!("failed to prepare overlay window: {err:?}");
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Err(err) = windows::show_main_window(app) {
                eprintln!("failed to reopen main window: {err:?}");
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Opened { .. } => {
            if let Err(err) = windows::show_main_window(app) {
                eprintln!("failed to show main window after open event: {err:?}");
            }
        }
        _ => {}
    });
}
