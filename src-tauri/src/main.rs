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
mod tray;
mod windows;
mod workflow;

use commands::{
    accessibility_permission_granted, apply_fn_trigger, cancel_current_workflow, choose_data_dir,
    copy_text_to_clipboard, export_config, get_data_dir, get_status, hide_main_window,
    import_config, input_monitoring_permission_granted, load_audio_input_devices,
    load_audio_output_devices, load_config, load_history, load_stats, open_accessibility_settings,
    open_app_dir, open_github_repository, open_input_monitoring_settings,
    request_accessibility_permission, request_input_monitoring_permission,
    request_microphone_permission, reset_data_dir, save_config, set_data_dir, toggle_recording,
};
use tauri::{Emitter, RunEvent};

fn main() {
    let app = tauri::Builder::default()
        .plugin(shortcuts::global_shortcut_plugin())
        .plugin(tauri_plugin_dialog::init())
        .manage(workflow::AppState::default())
        .setup(|app| {
            windows::ensure_main_window(app.handle())
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;
            windows::ensure_overlay_window(app.handle())
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;
            tray::setup(app.handle()).map_err(|err| Box::new(err) as Box<dyn std::error::Error>)?;
            let config = config::ConfigStore::load().unwrap_or_default();
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
            load_history,
            load_stats,
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
