use crate::config::ConfigStore;
use tauri::{LogicalSize, Manager, Size, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};

const MAIN_WINDOW_WIDTH: f64 = 1120.0;
const MAIN_WINDOW_HEIGHT: f64 = 760.0;
const MAIN_WINDOW_MIN_WIDTH: f64 = 1120.0;
const MAIN_WINDOW_MIN_HEIGHT: f64 = 680.0;

pub(crate) fn ensure_main_window(app: &tauri::AppHandle<Wry>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        return configure_main_window(&window);
    }

    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("BoltScribe")
        .inner_size(MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT)
        .min_inner_size(MAIN_WINDOW_MIN_WIDTH, MAIN_WINDOW_MIN_HEIGHT)
        .center()
        .visible(true)
        .focused(true)
        .decorations(true)
        .content_protected(false)
        .build()?;
    configure_main_window(&window)
}

pub(crate) fn show_main_window(app: &tauri::AppHandle<Wry>) -> tauri::Result<()> {
    set_dock_visible(app, true)?;
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }

    ensure_main_window(app)
}

pub(crate) fn hide_main_window(app: &tauri::AppHandle<Wry>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide()?;
    }
    sync_dock_visibility(app)?;
    Ok(())
}

pub(crate) fn sync_dock_visibility(app: &tauri::AppHandle<Wry>) -> tauri::Result<()> {
    let config = ConfigStore::load().unwrap_or_default();
    let main_visible = match app.get_webview_window("main") {
        Some(window) => window.is_visible().unwrap_or(false),
        None => false,
    };
    set_dock_visible(app, main_visible || !config.system.hide_dock_icon)
}

fn configure_main_window(window: &WebviewWindow<Wry>) -> tauri::Result<()> {
    window.set_min_size(Some(Size::Logical(LogicalSize {
        width: MAIN_WINDOW_MIN_WIDTH,
        height: MAIN_WINDOW_MIN_HEIGHT,
    })))?;
    window.set_size(Size::Logical(LogicalSize {
        width: MAIN_WINDOW_WIDTH,
        height: MAIN_WINDOW_HEIGHT,
    }))?;
    window.center()?;
    window.show()?;
    window.set_focus()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_dock_visible(app: &tauri::AppHandle<Wry>, visible: bool) -> tauri::Result<()> {
    app.set_dock_visibility(visible)
}

#[cfg(not(target_os = "macos"))]
fn set_dock_visible(_app: &tauri::AppHandle<Wry>, _visible: bool) -> tauri::Result<()> {
    Ok(())
}
