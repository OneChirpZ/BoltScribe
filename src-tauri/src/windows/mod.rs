mod main_window;
mod monitor;
mod overlay;

use tauri::{AppHandle, Theme, Wry};

pub(crate) use main_window::ensure_main_window;
pub(crate) use main_window::{hide_main_window, show_main_window, sync_dock_visibility};
pub(crate) use overlay::{ensure_overlay_window, sync_overlay_window};

pub(crate) fn sync_app_theme(app: &AppHandle<Wry>, preference: &str) {
    let theme = match preference {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    };
    app.set_theme(theme);
}
