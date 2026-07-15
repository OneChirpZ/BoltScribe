use super::monitor::active_overlay_monitor;
use crate::config::ConfigStore;
use crate::workflow;
use std::sync::Mutex;
use tauri::{
    LogicalSize, Manager, Monitor, PhysicalPosition, Position, Size, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, Wry,
};

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};

const OVERLAY_BASE_WIDTH: f64 = 314.0;
const OVERLAY_BASE_HEIGHT: f64 = 78.0;
const OVERLAY_BOTTOM_MARGIN: f64 = 28.0;
static OVERLAY_MONITOR_LOCK: Mutex<Option<LockedMonitor>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct LockedMonitor {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

pub(crate) fn ensure_overlay_window(app: &tauri::AppHandle<Wry>) -> tauri::Result<()> {
    if app.get_webview_window("overlay").is_some() {
        return Ok(());
    }
    let layout = overlay_layout();
    let window = WebviewWindowBuilder::new(
        app,
        "overlay",
        WebviewUrl::App("index.html?window=overlay".into()),
    )
    .title("BoltScribe Overlay")
    .inner_size(layout.width, layout.height)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .visible(false)
    .focused(false)
    .focusable(false)
    .accept_first_mouse(true)
    .shadow(false)
    .build()?;
    configure_overlay_window(app, &window, false, false, true)
}

fn configure_overlay_window(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
    visible: bool,
    reset_monitor: bool,
    reposition: bool,
) -> tauri::Result<()> {
    let layout = overlay_layout();
    window.set_size(Size::Logical(LogicalSize {
        width: layout.width,
        height: layout.height,
    }))?;
    window.set_resizable(false)?;
    window.set_decorations(false)?;
    window.set_always_on_top(true)?;
    window.set_visible_on_all_workspaces(true)?;
    window.set_skip_taskbar(true)?;
    window.set_focusable(false)?;
    let _ = window.set_ignore_cursor_events(!visible);
    if reset_monitor {
        clear_overlay_monitor_lock();
    }
    if reposition {
        if let Some(monitor) = locked_overlay_monitor(app)? {
            position_overlay_window(window, &monitor, layout)?;
        }
    }
    if visible {
        show_overlay_window(app, window)?;
    } else if window.is_visible().unwrap_or(false) {
        window.hide()?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn show_overlay_window(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
) -> tauri::Result<()> {
    let window = window.clone();
    app.run_on_main_thread(move || {
        if let Err(err) = configure_and_show_macos_overlay(&window) {
            eprintln!("failed to configure native macOS overlay window: {err:?}");
        }
    })
}

#[cfg(not(target_os = "macos"))]
fn show_overlay_window(
    _app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
) -> tauri::Result<()> {
    window.show()
}

#[cfg(target_os = "macos")]
fn configure_and_show_macos_overlay(window: &WebviewWindow<Wry>) -> tauri::Result<()> {
    let ns_window = window.ns_window()? as *const NSWindow;
    if ns_window.is_null() {
        return Ok(());
    }

    let ns_window = unsafe { &*ns_window };
    let mut behavior = ns_window.collectionBehavior();
    behavior.remove(
        NSWindowCollectionBehavior::MoveToActiveSpace
            | NSWindowCollectionBehavior::FullScreenPrimary
            | NSWindowCollectionBehavior::FullScreenNone,
    );
    behavior.insert(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    ns_window.setCollectionBehavior(behavior);
    ns_window.setLevel(NSStatusWindowLevel);
    ns_window.orderFrontRegardless();
    Ok(())
}

pub(crate) fn sync_overlay_window(app: &tauri::AppHandle, status: &workflow::WorkflowStatus) {
    let should_show = matches!(
        status.mode,
        workflow::WorkflowMode::Recording | workflow::WorkflowMode::Processing
    );

    let app = app.clone();
    if let Err(err) = ensure_overlay_window(&app) {
        eprintln!("failed to ensure overlay window: {err:?}");
        return;
    }

    if let Some(window) = app.get_webview_window("overlay") {
        if should_show {
            let was_visible = window.is_visible().unwrap_or(false);
            if !was_visible {
                let reset_monitor = status.mode == workflow::WorkflowMode::Recording;
                if let Err(err) = configure_overlay_window(&app, &window, true, reset_monitor, true)
                {
                    eprintln!("failed to show overlay window: {err:?}");
                }
            }
        } else {
            clear_overlay_monitor_lock();
            let _ = window.set_ignore_cursor_events(true);
            if window.is_visible().unwrap_or(false) {
                if let Err(err) = window.hide() {
                    eprintln!("failed to hide overlay window: {err:?}");
                }
            }
        }
    }
}

fn locked_overlay_monitor(app: &tauri::AppHandle<Wry>) -> tauri::Result<Option<Monitor>> {
    if let Some(monitor) = resolve_locked_monitor(app)? {
        return Ok(Some(monitor));
    }

    let Some(monitor) = active_overlay_monitor(app)? else {
        return Ok(None);
    };
    set_overlay_monitor_lock(&monitor);
    Ok(Some(monitor))
}

fn resolve_locked_monitor(app: &tauri::AppHandle<Wry>) -> tauri::Result<Option<Monitor>> {
    let locked = OVERLAY_MONITOR_LOCK.lock().ok().and_then(|lock| *lock);
    let Some(locked) = locked else {
        return Ok(None);
    };

    Ok(app.available_monitors()?.into_iter().find(|monitor| {
        let position = monitor.position();
        let size = monitor.size();
        position.x == locked.x
            && position.y == locked.y
            && size.width == locked.width
            && size.height == locked.height
    }))
}

fn set_overlay_monitor_lock(monitor: &Monitor) {
    if let Ok(mut lock) = OVERLAY_MONITOR_LOCK.lock() {
        let position = monitor.position();
        let size = monitor.size();
        *lock = Some(LockedMonitor {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        });
    }
}

fn clear_overlay_monitor_lock() {
    if let Ok(mut lock) = OVERLAY_MONITOR_LOCK.lock() {
        *lock = None;
    }
}

#[derive(Clone, Copy)]
struct OverlayLayout {
    width: f64,
    height: f64,
    offset_x: i32,
    offset_y: i32,
}

fn overlay_layout() -> OverlayLayout {
    let ui = ConfigStore::load()
        .map(|config| config.ui)
        .unwrap_or_default();
    let scale = ui.recording_overlay_scale.clamp(0.25, 1.0) as f64;

    OverlayLayout {
        width: (OVERLAY_BASE_WIDTH * scale).ceil(),
        height: (OVERLAY_BASE_HEIGHT * scale).ceil(),
        offset_x: ui.recording_overlay_offset_x,
        offset_y: ui.recording_overlay_offset_y,
    }
}

fn position_overlay_window(
    window: &WebviewWindow<Wry>,
    monitor: &Monitor,
    layout: OverlayLayout,
) -> tauri::Result<()> {
    let scale = monitor.scale_factor();
    let overlay_width = (layout.width * scale).round() as i32;
    let overlay_height = (layout.height * scale).round() as i32;
    let margin = (OVERLAY_BOTTOM_MARGIN * scale).round() as i32;
    let work_area = monitor.work_area();
    let min_x = work_area.position.x;
    let max_x = work_area.position.x + work_area.size.width as i32 - overlay_width;
    let min_y = work_area.position.y;
    let max_y = work_area.position.y + work_area.size.height as i32 - overlay_height;
    let offset_x = (layout.offset_x as f64 * scale).round() as i32;
    let offset_y = (layout.offset_y as f64 * scale).round() as i32;
    let base_x = work_area.position.x + ((work_area.size.width as i32 - overlay_width) / 2).max(0);
    let base_y = work_area.position.y + work_area.size.height as i32 - overlay_height - margin;
    let x = (base_x + offset_x).clamp(min_x, max_x.max(min_x));
    let y = (base_y - offset_y).clamp(min_y, max_y.max(min_y));

    window.set_position(Position::Physical(PhysicalPosition { x, y }))?;
    Ok(())
}
