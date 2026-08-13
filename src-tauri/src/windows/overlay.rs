use super::monitor::active_overlay_monitor;
use crate::config::ConfigStore;
use crate::workflow_status::{WorkflowMode, WorkflowStatus};
use serde::Serialize;
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{
    Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, Wry,
};

#[cfg(not(target_os = "macos"))]
use tauri::{LogicalSize, Position, Size};

#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSAppKitVersionNumber, NSAppKitVersionNumber13_0, NSPanel as AppKitPanel,
    NSScreenSaverWindowLevel, NSWindow as AppKitWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSPoint as AppKitPoint, NSSize as AppKitSize};
#[cfg(target_os = "macos")]
use tauri_nspanel::{tauri_panel, WebviewWindowExt};

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(OverlayPanel {
        config: {
            can_become_key_window: false,
            can_become_main_window: false,
            hides_on_deactivate: false,
            is_floating_panel: true,
            becomes_key_only_if_needed: true,
        }
    })
}

const OVERLAY_BASE_WIDTH_ZH: f64 = 340.0;
const OVERLAY_BASE_WIDTH_EN: f64 = 400.0;
const OVERLAY_BASE_HEIGHT: f64 = 78.0;
const OVERLAY_BOTTOM_MARGIN: f64 = 28.0;
const OVERLAY_SNAP_GUIDE_LABEL: &str = "overlay-snap-guide";
const OVERLAY_SNAP_ENTER_RADIUS: f64 = 32.0;
const OVERLAY_SNAP_EXIT_RADIUS: f64 = 44.0;
const OVERLAY_SNAP_RANGE_PERCENT_DEFAULT: u16 = 200;
const OVERLAY_SNAP_RANGE_PERCENT_MIN: u16 = 50;
const OVERLAY_SNAP_RANGE_PERCENT_MAX: u16 = 500;
const OVERLAY_SNAP_PREVIEW_INTERVAL: Duration = Duration::from_millis(16);
const OVERLAY_DRAG_RELEASE_POLL_INTERVAL: Duration = Duration::from_millis(12);
const OVERLAY_SNAP_ANIMATION_FRAMES: u32 = 10;
const OVERLAY_SNAP_ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);
pub(crate) const OVERLAY_POSITION_UPDATED_EVENT: &str = "overlay://position-updated";
static OVERLAY_MONITOR_LOCK: Mutex<Option<LockedMonitor>> = Mutex::new(None);
static OVERLAY_STATUS_REVISION: AtomicU64 = AtomicU64::new(0);
static OVERLAY_DRAG_GENERATION: AtomicU64 = AtomicU64::new(0);
static OVERLAY_DRAG_SESSION: Mutex<Option<OverlayDragSession>> = Mutex::new(None);
static OVERLAY_CURRENT_LAYOUT: Mutex<Option<OverlayLayout>> = Mutex::new(None);
#[cfg(target_os = "windows")]
static OVERLAY_WINDOWS_RELEASE_GENERATION: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "windows")]
static OVERLAY_WINDOWS_SUBCLASS_INSTALLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy)]
struct LockedMonitor {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct OverlayDragSession {
    id: u64,
    layout: OverlayLayout,
    moved: bool,
    snap_active: bool,
    guide_target: Option<OverlaySnapTarget>,
    last_preview_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct OverlaySnapTarget {
    #[cfg(target_os = "macos")]
    logical_origin: (f64, f64),
    #[cfg(not(target_os = "macos"))]
    physical_origin: PhysicalPosition<i32>,
    #[cfg(not(target_os = "macos"))]
    physical_size: PhysicalSize<u32>,
    #[cfg(target_os = "macos")]
    primary_height: f64,
}

#[derive(Clone, Copy, Debug)]
struct OverlaySnapEvaluation {
    #[cfg(target_os = "macos")]
    current_logical_origin: (f64, f64),
    current_physical_origin: PhysicalPosition<i32>,
    target: OverlaySnapTarget,
    distance_squared: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct OverlayPositionUpdate {
    pub(crate) recording_overlay_offset_x: i32,
    pub(crate) recording_overlay_offset_y: i32,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
struct OverlayPlacement {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    primary_height: f64,
}

#[cfg(not(target_os = "macos"))]
type OverlayPlacement = ();

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
    .visible_on_all_workspaces(!cfg!(target_os = "macos"))
    .skip_taskbar(true)
    .visible(false)
    .focused(false)
    .focusable(false)
    .accept_first_mouse(true)
    .shadow(false)
    .build()?;

    #[cfg(target_os = "macos")]
    {
        let panel = window.to_panel::<OverlayPanel>()?;
        panel.set_style_mask(NSWindowStyleMask::NonactivatingPanel);
        panel.set_hides_on_deactivate(false);
        panel.set_floating_panel(true);
        panel.set_becomes_key_only_if_needed(true);
        panel.set_released_when_closed(false);
        panel.set_ignores_mouse_events(true);
    }

    #[cfg(target_os = "windows")]
    install_overlay_windows_drag_subclass(&window)?;

    configure_overlay_window(app, &window, false, false, true, 0)
}

#[cfg(target_os = "windows")]
fn install_overlay_windows_drag_subclass(window: &WebviewWindow<Wry>) -> tauri::Result<()> {
    use windows::Win32::UI::Shell::SetWindowSubclass;

    const SUBCLASS_ID: usize = 0x4253_4452;
    let raw_hwnd = window.hwnd()?;
    let hwnd = windows::Win32::Foundation::HWND(raw_hwnd.0 as isize);
    let installed = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(overlay_windows_drag_subclass_proc),
            SUBCLASS_ID,
            0,
        )
    }
    .as_bool();
    OVERLAY_WINDOWS_SUBCLASS_INSTALLED.store(installed, Ordering::Release);
    if !installed {
        eprintln!("failed to install overlay native drag-end observer");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn overlay_windows_drag_subclass_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    subclass_id: usize,
    _reference_data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{WM_EXITSIZEMOVE, WM_NCDESTROY};

    if message == WM_EXITSIZEMOVE {
        OVERLAY_WINDOWS_RELEASE_GENERATION.store(
            OVERLAY_DRAG_GENERATION.load(Ordering::Acquire),
            Ordering::Release,
        );
    } else if message == WM_NCDESTROY {
        let _ = unsafe {
            RemoveWindowSubclass(hwnd, Some(overlay_windows_drag_subclass_proc), subclass_id)
        };
        OVERLAY_WINDOWS_SUBCLASS_INSTALLED.store(false, Ordering::Release);
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

pub(crate) fn ensure_overlay_snap_guide_window(app: &tauri::AppHandle<Wry>) -> tauri::Result<()> {
    if app.get_webview_window(OVERLAY_SNAP_GUIDE_LABEL).is_some() {
        return Ok(());
    }

    let layout = overlay_layout();
    let window = WebviewWindowBuilder::new(
        app,
        OVERLAY_SNAP_GUIDE_LABEL,
        WebviewUrl::App("index.html?window=overlay-snap-guide".into()),
    )
    .title("BoltScribe Overlay Snap Guide")
    .inner_size(layout.width, layout.height)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .visible_on_all_workspaces(!cfg!(target_os = "macos"))
    .skip_taskbar(true)
    .visible(false)
    .focused(false)
    .focusable(false)
    .accept_first_mouse(false)
    .shadow(false)
    .build()?;

    window.set_ignore_cursor_events(true)?;

    #[cfg(target_os = "macos")]
    {
        let panel = window.to_panel::<OverlayPanel>()?;
        panel.set_style_mask(NSWindowStyleMask::NonactivatingPanel);
        panel.set_hides_on_deactivate(false);
        panel.set_floating_panel(true);
        panel.set_becomes_key_only_if_needed(true);
        panel.set_released_when_closed(false);
        panel.set_ignores_mouse_events(true);
    }

    Ok(())
}

fn configure_overlay_window(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
    visible: bool,
    reset_monitor: bool,
    reposition: bool,
    revision: u64,
) -> tauri::Result<()> {
    let layout = overlay_layout();
    cache_overlay_layout(layout);

    #[cfg(not(target_os = "macos"))]
    {
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
    }

    if reset_monitor {
        clear_overlay_monitor_lock();
    }

    let placement = if reposition {
        if let Some(monitor) = locked_overlay_monitor(app)? {
            #[cfg(target_os = "macos")]
            {
                Some(macos_overlay_placement(app, &monitor, layout)?)
            }
            #[cfg(not(target_os = "macos"))]
            {
                position_overlay_window(window, &monitor, layout)?;
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    if visible {
        show_overlay_window(app, window, placement, revision)?;
    } else if window.is_visible().unwrap_or(false) {
        hide_overlay_window(app, window, revision)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn show_overlay_window(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
    placement: Option<OverlayPlacement>,
    revision: u64,
) -> tauri::Result<()> {
    let window = window.clone();
    app.run_on_main_thread(move || {
        if !overlay_revision_is_current(revision) {
            return;
        }
        if let Err(err) = configure_and_show_macos_overlay(&window, placement) {
            eprintln!("failed to configure native macOS overlay panel: {err:?}");
        }
    })
}

#[cfg(not(target_os = "macos"))]
fn show_overlay_window(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
    _placement: Option<OverlayPlacement>,
    revision: u64,
) -> tauri::Result<()> {
    let window = window.clone();
    app.run_on_main_thread(move || {
        if !overlay_revision_is_current(revision) {
            return;
        }
        if let Err(err) = window.set_ignore_cursor_events(false) {
            eprintln!("failed to enable overlay cursor events: {err:?}");
        }
        if let Err(err) = window.show() {
            eprintln!("failed to show overlay window: {err:?}");
        }
    })
}

#[cfg(target_os = "macos")]
fn hide_overlay_window(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
    revision: u64,
) -> tauri::Result<()> {
    let window = window.clone();
    app.run_on_main_thread(move || {
        if !overlay_revision_is_current(revision) {
            return;
        }
        match window.ns_window() {
            Ok(ns_window) if !ns_window.is_null() => {
                let ns_window = unsafe { &*(ns_window as *const AppKitWindow) };
                ns_window.setIgnoresMouseEvents(true);
                ns_window.orderOut(None);
            }
            Ok(_) => {}
            Err(err) => eprintln!("failed to hide native macOS overlay panel: {err:?}"),
        }
    })
}

#[cfg(target_os = "macos")]
fn show_overlay_snap_guide(
    app: &tauri::AppHandle<Wry>,
    id: u64,
    target: OverlaySnapTarget,
    layout: OverlayLayout,
) -> tauri::Result<()> {
    ensure_overlay_snap_guide_window(app)?;
    let Some(window) = app.get_webview_window(OVERLAY_SNAP_GUIDE_LABEL) else {
        return Ok(());
    };
    app.run_on_main_thread(move || {
        if !overlay_drag_is_current(id) {
            return;
        }
        let result = (|| -> tauri::Result<()> {
            let native_window = window.ns_window()?;
            if native_window.is_null() {
                return Ok(());
            }
            let ns_window = unsafe { &*(native_window as *const AppKitWindow) };
            let ns_panel = unsafe { &*(native_window as *const AppKitPanel) };
            ns_window.setContentSize(AppKitSize::new(layout.width, layout.height));
            ns_window.setFrameTopLeftPoint(AppKitPoint::new(
                target.logical_origin.0,
                target.primary_height - target.logical_origin.1,
            ));
            ns_window.setHidesOnDeactivate(false);
            ns_window.setCollectionBehavior(overlay_collection_behavior(
                supports_joining_all_applications(),
            ));
            ns_panel.setFloatingPanel(true);
            ns_panel.setBecomesKeyOnlyIfNeeded(true);
            ns_window.setIgnoresMouseEvents(true);
            ns_window.setLevel(NSScreenSaverWindowLevel);
            ns_window.orderFrontRegardless();
            Ok(())
        })();
        if let Err(err) = result {
            eprintln!("failed to show overlay snap guide: {err:?}");
        }
    })
}

#[cfg(not(target_os = "macos"))]
fn show_overlay_snap_guide(
    app: &tauri::AppHandle<Wry>,
    id: u64,
    target: OverlaySnapTarget,
    _layout: OverlayLayout,
) -> tauri::Result<()> {
    ensure_overlay_snap_guide_window(app)?;
    let Some(window) = app.get_webview_window(OVERLAY_SNAP_GUIDE_LABEL) else {
        return Ok(());
    };
    app.run_on_main_thread(move || {
        if !overlay_drag_is_current(id) {
            return;
        }
        let result = (|| -> tauri::Result<()> {
            window.set_size(Size::Physical(target.physical_size))?;
            window.set_position(Position::Physical(target.physical_origin))?;
            window.set_ignore_cursor_events(true)?;
            window.show()
        })();
        if let Err(err) = result {
            eprintln!("failed to show overlay snap guide: {err:?}");
        }
    })
}

#[cfg(target_os = "macos")]
fn hide_overlay_snap_guide(
    app: &tauri::AppHandle<Wry>,
    expected_id: Option<u64>,
) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(OVERLAY_SNAP_GUIDE_LABEL) else {
        return Ok(());
    };
    app.run_on_main_thread(move || {
        if expected_id.is_some_and(|id| !overlay_drag_is_current(id)) {
            return;
        }
        match window.ns_window() {
            Ok(native_window) if !native_window.is_null() => {
                let ns_window = unsafe { &*(native_window as *const AppKitWindow) };
                ns_window.setIgnoresMouseEvents(true);
                ns_window.orderOut(None);
            }
            Ok(_) => {}
            Err(err) => eprintln!("failed to hide overlay snap guide: {err:?}"),
        }
    })
}

#[cfg(not(target_os = "macos"))]
fn hide_overlay_snap_guide(
    app: &tauri::AppHandle<Wry>,
    expected_id: Option<u64>,
) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(OVERLAY_SNAP_GUIDE_LABEL) else {
        return Ok(());
    };
    app.run_on_main_thread(move || {
        if expected_id.is_some_and(|id| !overlay_drag_is_current(id)) {
            return;
        }
        if let Err(err) = window.set_ignore_cursor_events(true) {
            eprintln!("failed to keep overlay snap guide mouse-transparent: {err:?}");
        }
        if let Err(err) = window.hide() {
            eprintln!("failed to hide overlay snap guide: {err:?}");
        }
    })
}

#[cfg(not(target_os = "macos"))]
fn hide_overlay_window(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
    revision: u64,
) -> tauri::Result<()> {
    let window = window.clone();
    app.run_on_main_thread(move || {
        if !overlay_revision_is_current(revision) {
            return;
        }
        if let Err(err) = window.set_ignore_cursor_events(true) {
            eprintln!("failed to disable overlay cursor events: {err:?}");
        }
        if let Err(err) = window.hide() {
            eprintln!("failed to hide overlay window: {err:?}");
        }
    })
}

#[cfg(target_os = "macos")]
fn configure_and_show_macos_overlay(
    window: &WebviewWindow<Wry>,
    placement: Option<OverlayPlacement>,
) -> tauri::Result<()> {
    let native_window = window.ns_window()?;
    if native_window.is_null() {
        return Ok(());
    }

    let ns_window = unsafe { &*(native_window as *const AppKitWindow) };
    let ns_panel = unsafe { &*(native_window as *const AppKitPanel) };

    // Detach only a hidden panel from its previous Space. Re-showing an
    // already visible panel would create a noticeable Starting -> Recording
    // flash between orderOut and orderFrontRegardless.
    if !ns_window.isVisible() {
        ns_window.orderOut(None);
    }

    if let Some(placement) = placement {
        ns_window.setContentSize(AppKitSize::new(placement.width, placement.height));
        ns_window.setFrameTopLeftPoint(AppKitPoint::new(
            placement.x,
            placement.primary_height - placement.y,
        ));
    }

    ns_window.setHidesOnDeactivate(false);
    ns_window.setCollectionBehavior(overlay_collection_behavior(
        supports_joining_all_applications(),
    ));
    ns_panel.setFloatingPanel(true);
    ns_panel.setBecomesKeyOnlyIfNeeded(true);
    ns_window.setIgnoresMouseEvents(false);
    ns_window.setLevel(NSScreenSaverWindowLevel);
    ns_window.orderFrontRegardless();

    let frame = ns_window.frame();
    eprintln!(
        "overlay panel shown frame=({:.1}, {:.1}, {:.1}, {:.1}) level={} active_space={} behavior={:#x}",
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
        ns_window.level(),
        ns_window.isOnActiveSpace(),
        ns_window.collectionBehavior().bits()
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn supports_joining_all_applications() -> bool {
    unsafe { NSAppKitVersionNumber >= NSAppKitVersionNumber13_0 }
}

#[cfg(target_os = "macos")]
fn overlay_collection_behavior(supports_all_applications: bool) -> NSWindowCollectionBehavior {
    let mut behavior = NSWindowCollectionBehavior::MoveToActiveSpace
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Transient
        | NSWindowCollectionBehavior::IgnoresCycle;
    if supports_all_applications {
        behavior.insert(NSWindowCollectionBehavior::CanJoinAllApplications);
    }
    behavior
}

pub(crate) fn sync_overlay_window(app: &tauri::AppHandle, status: &WorkflowStatus) {
    if !claim_overlay_revision(status.revision) {
        return;
    }
    let should_show = overlay_should_show(&status.mode);

    let app = app.clone();
    if let Err(err) = ensure_overlay_window(&app) {
        eprintln!("failed to ensure overlay window: {err:?}");
        return;
    }

    if let Some(window) = app.get_webview_window("overlay") {
        if should_show {
            let was_visible = window.is_visible().unwrap_or(false);
            if should_request_overlay_show(&status.mode, was_visible) {
                let reset_monitor = status.mode == WorkflowMode::Starting;
                if let Err(err) = configure_overlay_window(
                    &app,
                    &window,
                    true,
                    reset_monitor,
                    true,
                    status.revision,
                ) {
                    eprintln!("failed to show overlay window: {err:?}");
                }
            }
        } else {
            cancel_overlay_drag(&app);
            clear_overlay_monitor_lock();
            #[cfg(target_os = "macos")]
            if let Err(err) = hide_overlay_window(&app, &window, status.revision) {
                eprintln!("failed to hide overlay window: {err:?}");
            }
            #[cfg(not(target_os = "macos"))]
            {
                if let Err(err) = hide_overlay_window(&app, &window, status.revision) {
                    eprintln!("failed to hide overlay window: {err:?}");
                }
            }
        }
    }
}

pub(crate) fn start_overlay_drag(app: &tauri::AppHandle<Wry>) -> anyhow::Result<()> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| anyhow::anyhow!("recording overlay window is unavailable"))?;
    let layout = current_overlay_layout();
    let id = {
        let mut current = OVERLAY_DRAG_SESSION
            .lock()
            .map_err(|_| anyhow::anyhow!("overlay drag session lock is unavailable"))?;
        let id = OVERLAY_DRAG_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
        *current = Some(OverlayDragSession {
            id,
            layout,
            moved: false,
            snap_active: false,
            guide_target: None,
            last_preview_at: None,
        });
        id
    };

    if let Err(err) = window.start_dragging() {
        cancel_overlay_drag(app);
        return Err(err.into());
    }

    match evaluate_overlay_snap(app, layout) {
        Ok(evaluation) => {
            let snap_active = snap_target_is_active(
                false,
                evaluation.distance_squared,
                layout.snap_range_percent,
            );
            let accepted = OVERLAY_DRAG_SESSION
                .lock()
                .ok()
                .and_then(|mut current| {
                    let session = current.as_mut()?;
                    if session.id != id {
                        return None;
                    }
                    session.snap_active = snap_active;
                    session.guide_target = Some(evaluation.target);
                    Some(())
                })
                .is_some();
            if accepted {
                if let Err(err) = show_overlay_snap_guide(app, id, evaluation.target, layout) {
                    eprintln!("failed to show overlay snap guide for a new drag: {err:?}");
                }
            }
        }
        Err(err) => eprintln!("failed to evaluate initial overlay snap guide: {err:?}"),
    }

    let watcher_app = app.clone();
    if let Err(err) = thread::Builder::new()
        .name("overlay-drag-release".to_string())
        .spawn(move || watch_overlay_drag_release(watcher_app, id))
    {
        cancel_overlay_drag(app);
        return Err(err.into());
    }
    Ok(())
}

pub(crate) fn record_overlay_moved(app: &tauri::AppHandle<Wry>) {
    let now = Instant::now();
    let Some((id, layout, was_active)) =
        OVERLAY_DRAG_SESSION.lock().ok().and_then(|mut current| {
            let session = current.as_mut()?;
            session.moved = true;
            if session
                .last_preview_at
                .is_some_and(|last| now.duration_since(last) < OVERLAY_SNAP_PREVIEW_INTERVAL)
            {
                return None;
            }
            session.last_preview_at = Some(now);
            Some((session.id, session.layout, session.snap_active))
        })
    else {
        return;
    };

    let evaluation = match evaluate_overlay_snap(app, layout) {
        Ok(evaluation) => evaluation,
        Err(err) => {
            eprintln!("failed to evaluate overlay snap preview: {err:?}");
            return;
        }
    };
    let snap_active = snap_target_is_active(
        was_active,
        evaluation.distance_squared,
        layout.snap_range_percent,
    );

    let target_changed = OVERLAY_DRAG_SESSION
        .lock()
        .ok()
        .and_then(|mut current| {
            let session = current.as_mut()?;
            if session.id != id {
                return None;
            }
            let target_changed = session.guide_target != Some(evaluation.target);
            session.snap_active = snap_active;
            session.guide_target = Some(evaluation.target);
            Some(target_changed)
        })
        .unwrap_or(false);

    if !target_changed {
        return;
    }
    if let Err(err) = show_overlay_snap_guide(app, id, evaluation.target, layout) {
        eprintln!("failed to move overlay snap guide: {err:?}");
    }
}

fn watch_overlay_drag_release(app: tauri::AppHandle<Wry>, id: u64) {
    #[cfg(target_os = "windows")]
    if OVERLAY_WINDOWS_SUBCLASS_INSTALLED.load(Ordering::Acquire) {
        loop {
            thread::sleep(OVERLAY_DRAG_RELEASE_POLL_INTERVAL);
            if !overlay_drag_is_current(id) {
                return;
            }
            if OVERLAY_WINDOWS_RELEASE_GENERATION.load(Ordering::Acquire) == id {
                break;
            }
        }
        finish_overlay_drag_after_native_release(&app, id);
        return;
    }

    let mut observed_down = false;
    let mut consecutive_up_samples = 0u8;
    loop {
        thread::sleep(OVERLAY_DRAG_RELEASE_POLL_INTERVAL);
        if !overlay_drag_is_current(id) {
            return;
        }
        if primary_pointer_button_is_down() {
            observed_down = true;
            consecutive_up_samples = 0;
            continue;
        }
        consecutive_up_samples = consecutive_up_samples.saturating_add(1);
        let required_samples = if observed_down { 2 } else { 4 };
        if consecutive_up_samples >= required_samples {
            break;
        }
    }
    finish_overlay_drag_after_native_release(&app, id);
}

fn finish_overlay_drag_after_native_release(app: &tauri::AppHandle<Wry>, id: u64) {
    // Let the native move loop publish its last geometry before capturing it.
    thread::sleep(OVERLAY_SNAP_PREVIEW_INTERVAL);
    if let Err(err) = finish_overlay_drag(app, id) {
        eprintln!("failed to finish overlay drag: {err:?}");
    }
}

fn finish_overlay_drag(app: &tauri::AppHandle<Wry>, id: u64) -> anyhow::Result<()> {
    let Some(session) = OVERLAY_DRAG_SESSION.lock().ok().and_then(|mut current| {
        if current.as_ref().is_some_and(|session| session.id == id) {
            current.take()
        } else {
            None
        }
    }) else {
        return Ok(());
    };

    let result = if !session.moved || !overlay_drag_is_current(id) {
        Ok(())
    } else {
        finish_moved_overlay_drag(app, id, session)
    };
    if let Err(err) = hide_overlay_snap_guide(app, Some(id)) {
        eprintln!("failed to clean up overlay snap guide: {err:?}");
    }
    result
}

fn finish_moved_overlay_drag(
    app: &tauri::AppHandle<Wry>,
    id: u64,
    session: OverlayDragSession,
) -> anyhow::Result<()> {
    let evaluation = evaluate_overlay_snap(app, session.layout)?;
    let should_snap = snap_target_is_active(
        session.snap_active,
        evaluation.distance_squared,
        session.layout.snap_range_percent,
    );
    let update = if should_snap {
        if session.guide_target != Some(evaluation.target) {
            if let Err(err) = show_overlay_snap_guide(app, id, evaluation.target, session.layout) {
                eprintln!("failed to show final overlay snap guide: {err:?}");
            }
        }
        match animate_overlay_to_snap_target(app, id, evaluation) {
            Ok(true) => commit_overlay_position(app, id, None)?,
            Ok(false) => None,
            Err(err) => {
                eprintln!(
                    "overlay snap animation failed, preserving the current position: {err:?}"
                );
                let origin = app
                    .get_webview_window("overlay")
                    .ok_or_else(|| anyhow::anyhow!("recording overlay window is unavailable"))?
                    .outer_position()?;
                commit_overlay_position(app, id, Some(origin))?
            }
        }
    } else {
        commit_overlay_position(app, id, Some(evaluation.current_physical_origin))?
    };

    if let Some(update) = update {
        app.emit(OVERLAY_POSITION_UPDATED_EVENT, update)?;
    }
    Ok(())
}

fn commit_overlay_position(
    app: &tauri::AppHandle<Wry>,
    id: u64,
    origin: Option<PhysicalPosition<i32>>,
) -> anyhow::Result<Option<OverlayPositionUpdate>> {
    let _session_guard = OVERLAY_DRAG_SESSION
        .lock()
        .map_err(|_| anyhow::anyhow!("overlay drag session lock is unavailable"))?;
    if !overlay_drag_is_current(id) {
        return Ok(None);
    }
    let update = match origin {
        Some(origin) => persist_overlay_position(app, origin)?,
        None => save_default_overlay_position()?,
    };
    Ok(Some(update))
}

fn cancel_overlay_drag(app: &tauri::AppHandle<Wry>) {
    let id = if let Ok(mut current) = OVERLAY_DRAG_SESSION.lock() {
        *current = None;
        OVERLAY_DRAG_GENERATION.fetch_add(1, Ordering::AcqRel) + 1
    } else {
        OVERLAY_DRAG_GENERATION.fetch_add(1, Ordering::AcqRel) + 1
    };
    if let Err(err) = hide_overlay_snap_guide(app, Some(id)) {
        eprintln!("failed to hide overlay snap guide while cancelling drag: {err:?}");
    }
}

fn overlay_drag_is_current(id: u64) -> bool {
    OVERLAY_DRAG_GENERATION.load(Ordering::Acquire) == id
}

fn snap_target_is_active(was_active: bool, distance_squared: f64, snap_range_percent: u16) -> bool {
    let base_radius = if was_active {
        OVERLAY_SNAP_EXIT_RADIUS
    } else {
        OVERLAY_SNAP_ENTER_RADIUS
    };
    let percent = snap_range_percent.clamp(
        OVERLAY_SNAP_RANGE_PERCENT_MIN,
        OVERLAY_SNAP_RANGE_PERCENT_MAX,
    ) as f64;
    let radius = base_radius * percent / 100.0;
    distance_squared <= radius * radius
}

#[cfg(target_os = "macos")]
fn primary_pointer_button_is_down() -> bool {
    objc2_core_graphics::CGEventSource::button_state(
        objc2_core_graphics::CGEventSourceStateID::CombinedSessionState,
        objc2_core_graphics::CGMouseButton::Left,
    )
}

#[cfg(target_os = "windows")]
fn primary_pointer_button_is_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};

    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    let key = if swapped { VK_RBUTTON } else { VK_LBUTTON };
    unsafe { GetAsyncKeyState(i32::from(key.0)) as u16 & 0x8000 != 0 }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn primary_pointer_button_is_down() -> bool {
    false
}

fn claim_overlay_revision(revision: u64) -> bool {
    revision >= OVERLAY_STATUS_REVISION.fetch_max(revision, Ordering::AcqRel)
}

fn overlay_revision_is_current(revision: u64) -> bool {
    OVERLAY_STATUS_REVISION.load(Ordering::Acquire) == revision
}

fn overlay_should_show(mode: &WorkflowMode) -> bool {
    matches!(
        mode,
        WorkflowMode::Starting
            | WorkflowMode::Recording
            | WorkflowMode::Processing
            | WorkflowMode::Error
    )
}

fn should_request_overlay_show(mode: &WorkflowMode, is_visible: bool) -> bool {
    *mode == WorkflowMode::Starting || !is_visible
}

#[cfg(test)]
mod cross_platform_tests {
    use super::*;

    fn layout(offset_x: i32, offset_y: i32) -> OverlayLayout {
        OverlayLayout {
            width: 170.0,
            height: 39.0,
            snap_range_percent: OVERLAY_SNAP_RANGE_PERCENT_DEFAULT,
            offset_x,
            offset_y,
        }
    }

    #[test]
    fn overlay_is_visible_for_starting_recording_processing_and_error() {
        for mode in [
            WorkflowMode::Starting,
            WorkflowMode::Recording,
            WorkflowMode::Processing,
            WorkflowMode::Error,
        ] {
            assert!(overlay_should_show(&mode));
        }
        assert!(!overlay_should_show(&WorkflowMode::Idle));
    }

    #[test]
    fn only_starting_forces_a_fresh_show_for_an_already_visible_panel() {
        assert!(should_request_overlay_show(&WorkflowMode::Starting, true));
        assert!(!should_request_overlay_show(&WorkflowMode::Recording, true));
        assert!(!should_request_overlay_show(&WorkflowMode::Error, true));
        assert!(!should_request_overlay_show(
            &WorkflowMode::Processing,
            true
        ));
        assert!(should_request_overlay_show(
            &WorkflowMode::Processing,
            false
        ));
    }

    #[test]
    fn forward_and_inverse_geometry_round_trip_with_negative_origin() {
        let work_area = LogicalRect {
            x: -954.0,
            y: -644.0,
            width: 954.0,
            height: 1_696.0,
        };
        let layout = layout(123, -15);

        let origin = overlay_origin(work_area, layout);

        assert_eq!(
            overlay_offsets_from_origin(work_area, layout, origin),
            (123, -15)
        );
    }

    #[test]
    fn mixed_scale_physical_coordinates_round_trip_through_logical_offsets() {
        let scale = 2.0;
        let work_area = logical_rect_from_physical(-3_840, 48, 3_840, 2_112, scale);
        let layout = layout(-210, 160);
        let logical_origin = overlay_origin(work_area, layout);
        let moved_physical_origin = (
            (logical_origin.0 * scale).round(),
            (logical_origin.1 * scale).round(),
        );

        assert_eq!(
            overlay_offsets_from_origin(
                work_area,
                layout,
                (
                    moved_physical_origin.0 / scale,
                    moved_physical_origin.1 / scale,
                ),
            ),
            (-210, 160)
        );
    }

    #[test]
    fn destination_selection_uses_final_center_across_displays() {
        let monitors = [
            PhysicalRectGeometry {
                x: 0.0,
                y: 0.0,
                width: 1_920.0,
                height: 1_080.0,
            },
            PhysicalRectGeometry {
                x: -2_560.0,
                y: -360.0,
                width: 2_560.0,
                height: 1_440.0,
            },
        ];

        assert_eq!(
            select_destination_monitor(
                &monitors,
                PhysicalPoint {
                    x: -320.0,
                    y: 700.0,
                },
                Some(0),
            ),
            Some(1)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_mixed_scale_selection_uses_one_logical_coordinate_space() {
        let logical_monitors = [
            PhysicalRectGeometry {
                x: 0.0,
                y: 0.0,
                width: 1_440.0,
                height: 900.0,
            },
            PhysicalRectGeometry {
                x: 1_440.0,
                y: 0.0,
                width: 1_920.0,
                height: 1_080.0,
            },
        ];
        let center = scaled_global_window_center(
            PhysicalPosition { x: 1_600, y: 500 },
            PhysicalSize {
                width: 170,
                height: 39,
            },
            1.0,
        );

        assert_eq!(
            select_destination_monitor(&logical_monitors, center, Some(0)),
            Some(1)
        );
    }

    #[test]
    fn destination_selection_falls_back_to_lock_then_nearest_monitor() {
        let monitors = [
            PhysicalRectGeometry {
                x: 0.0,
                y: 0.0,
                width: 1_000.0,
                height: 800.0,
            },
            PhysicalRectGeometry {
                x: 1_200.0,
                y: 0.0,
                width: 1_000.0,
                height: 800.0,
            },
        ];
        let gap = PhysicalPoint {
            x: 1_150.0,
            y: 400.0,
        };

        assert_eq!(select_destination_monitor(&monitors, gap, Some(0)), Some(0));
        assert_eq!(select_destination_monitor(&monitors, gap, None), Some(1));
    }

    #[test]
    fn restoration_clamps_without_changing_inverse_drag_geometry() {
        let work_area = LogicalRect {
            x: 100.0,
            y: 50.0,
            width: 800.0,
            height: 600.0,
        };
        let layout = OverlayLayout {
            width: 300.0,
            height: 80.0,
            snap_range_percent: OVERLAY_SNAP_RANGE_PERCENT_DEFAULT,
            offset_x: 4_000,
            offset_y: -4_000,
        };

        assert_eq!(overlay_origin(work_area, layout), (600.0, 570.0));
        assert_eq!(
            overlay_offsets_from_origin(work_area, layout, (1_100.0, 800.0)),
            (750, -258)
        );
    }

    #[test]
    fn snap_activation_uses_logical_distance_hysteresis() {
        assert!(snap_target_is_active(false, 32.0_f64.powi(2), 100));
        assert!(!snap_target_is_active(false, 32.1_f64.powi(2), 100));
        assert!(snap_target_is_active(true, 43.9_f64.powi(2), 100));
        assert!(!snap_target_is_active(true, 44.1_f64.powi(2), 100));
    }

    #[test]
    fn snap_range_percentage_scales_both_hysteresis_radii() {
        assert!(snap_target_is_active(false, 64.0_f64.powi(2), 200));
        assert!(!snap_target_is_active(false, 64.1_f64.powi(2), 200));
        assert!(snap_target_is_active(true, 87.9_f64.powi(2), 200));
        assert!(!snap_target_is_active(true, 88.1_f64.powi(2), 200));
    }

    #[test]
    fn default_snap_target_clamps_inside_a_smaller_work_area() {
        let work_area = LogicalRect {
            x: 100.0,
            y: 50.0,
            width: 120.0,
            height: 60.0,
        };
        let default_layout = OverlayLayout {
            width: 170.0,
            height: 80.0,
            snap_range_percent: OVERLAY_SNAP_RANGE_PERCENT_DEFAULT,
            offset_x: 0,
            offset_y: 0,
        };

        assert_eq!(overlay_origin(work_area, default_layout), (100.0, 50.0));
    }

    #[test]
    fn snap_animation_easing_is_monotonic_with_exact_endpoints() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        let samples = (0..=OVERLAY_SNAP_ANIMATION_FRAMES)
            .map(|frame| ease_out_cubic(frame as f64 / OVERLAY_SNAP_ANIMATION_FRAMES as f64))
            .collect::<Vec<_>>();
        assert!(samples.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(samples[1] - samples[0] > samples[10] - samples[9]);
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

pub(crate) fn persist_overlay_position(
    app: &tauri::AppHandle<Wry>,
    _settled_origin: PhysicalPosition<i32>,
) -> anyhow::Result<OverlayPositionUpdate> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| anyhow::anyhow!("recording overlay window is unavailable"))?;
    #[cfg(target_os = "macos")]
    let (origin, size, window_scale) = stable_scaled_window_geometry(&window)?;
    #[cfg(not(target_os = "macos"))]
    let (origin, size, window_scale) = (_settled_origin, window.outer_size()?, 1.0);
    let monitor = destination_overlay_monitor(app, origin, size, window_scale)?
        .ok_or_else(|| anyhow::anyhow!("no display is available for the recording overlay"))?;
    #[cfg(target_os = "macos")]
    let origin_scale = window_scale;
    #[cfg(not(target_os = "macos"))]
    let origin_scale = monitor.scale_factor().max(f64::EPSILON);
    let work_area = logical_work_area(&monitor);
    let config = ConfigStore::load()?;
    let layout = overlay_layout_from_ui(&config.ui);
    let logical_origin = (
        origin.x as f64 / origin_scale,
        origin.y as f64 / origin_scale,
    );
    let (offset_x, offset_y) = overlay_offsets_from_origin(work_area, layout, logical_origin);
    let saved = ConfigStore::update(|config| {
        config.ui.recording_overlay_offset_x = offset_x;
        config.ui.recording_overlay_offset_y = offset_y;
    })?;

    Ok(OverlayPositionUpdate {
        recording_overlay_offset_x: saved.ui.recording_overlay_offset_x,
        recording_overlay_offset_y: saved.ui.recording_overlay_offset_y,
    })
}

fn save_default_overlay_position() -> anyhow::Result<OverlayPositionUpdate> {
    let saved = ConfigStore::update(|config| {
        config.ui.recording_overlay_offset_x = 0;
        config.ui.recording_overlay_offset_y = 0;
    })?;
    Ok(OverlayPositionUpdate {
        recording_overlay_offset_x: saved.ui.recording_overlay_offset_x,
        recording_overlay_offset_y: saved.ui.recording_overlay_offset_y,
    })
}

#[cfg(target_os = "macos")]
fn stable_scaled_window_geometry(
    window: &WebviewWindow<Wry>,
) -> anyhow::Result<(PhysicalPosition<i32>, PhysicalSize<u32>, f64)> {
    for _ in 0..3 {
        let scale_before = window.scale_factor()?.max(f64::EPSILON);
        let origin = window.outer_position()?;
        let size = window.outer_size()?;
        let scale_after = window.scale_factor()?.max(f64::EPSILON);
        if (scale_before - scale_after).abs() < f64::EPSILON {
            return Ok((origin, size, scale_after));
        }
    }
    Err(anyhow::anyhow!(
        "recording overlay display scale did not settle"
    ))
}

fn destination_overlay_monitor(
    app: &tauri::AppHandle<Wry>,
    origin: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    _window_scale: f64,
) -> tauri::Result<Option<Monitor>> {
    let monitors = app.available_monitors()?;
    if monitors.is_empty() {
        return Ok(None);
    }

    let locked = OVERLAY_MONITOR_LOCK.lock().ok().and_then(|lock| *lock);
    let locked_index = locked.and_then(|locked| {
        monitors.iter().position(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            position.x == locked.x
                && position.y == locked.y
                && size.width == locked.width
                && size.height == locked.height
        })
    });
    #[cfg(target_os = "macos")]
    let monitor_bounds = monitors
        .iter()
        .map(logical_monitor_rect)
        .collect::<Vec<_>>();
    #[cfg(not(target_os = "macos"))]
    let monitor_bounds = monitors
        .iter()
        .map(physical_monitor_rect)
        .collect::<Vec<_>>();
    #[cfg(target_os = "macos")]
    let center = scaled_global_window_center(origin, size, _window_scale);
    #[cfg(not(target_os = "macos"))]
    let center = PhysicalPoint {
        x: origin.x as f64 + size.width as f64 / 2.0,
        y: origin.y as f64 + size.height as f64 / 2.0,
    };
    let Some(index) = select_destination_monitor(&monitor_bounds, center, locked_index) else {
        return Ok(None);
    };
    let monitor = monitors[index].clone();
    set_overlay_monitor_lock(&monitor);
    Ok(Some(monitor))
}

#[derive(Clone, Copy, Debug)]
struct OverlayLayout {
    width: f64,
    height: f64,
    snap_range_percent: u16,
    offset_x: i32,
    offset_y: i32,
}

fn overlay_layout() -> OverlayLayout {
    let ui = ConfigStore::load()
        .map(|config| config.ui)
        .unwrap_or_default();
    overlay_layout_from_ui(&ui)
}

fn cache_overlay_layout(layout: OverlayLayout) {
    if let Ok(mut current) = OVERLAY_CURRENT_LAYOUT.lock() {
        *current = Some(layout);
    }
}

pub(crate) fn sync_overlay_layout_cache(ui: &crate::config::UiConfig) {
    cache_overlay_layout(overlay_layout_from_ui(ui));
}

fn current_overlay_layout() -> OverlayLayout {
    OVERLAY_CURRENT_LAYOUT
        .lock()
        .ok()
        .and_then(|current| *current)
        .unwrap_or(OverlayLayout {
            width: (OVERLAY_BASE_WIDTH_ZH * 0.5).ceil(),
            height: (OVERLAY_BASE_HEIGHT * 0.5).ceil(),
            snap_range_percent: OVERLAY_SNAP_RANGE_PERCENT_DEFAULT,
            offset_x: 0,
            offset_y: 0,
        })
}

fn overlay_layout_from_ui(ui: &crate::config::UiConfig) -> OverlayLayout {
    let scale = ui.recording_overlay_scale.clamp(0.25, 1.0) as f64;
    let base_width = if ui.app_language == "en-US" {
        OVERLAY_BASE_WIDTH_EN
    } else {
        OVERLAY_BASE_WIDTH_ZH
    };

    OverlayLayout {
        width: (base_width * scale).ceil(),
        height: (OVERLAY_BASE_HEIGHT * scale).ceil(),
        snap_range_percent: ui.recording_overlay_snap_range_percent.clamp(
            OVERLAY_SNAP_RANGE_PERCENT_MIN,
            OVERLAY_SNAP_RANGE_PERCENT_MAX,
        ),
        offset_x: ui.recording_overlay_offset_x,
        offset_y: ui.recording_overlay_offset_y,
    }
}

fn evaluate_overlay_snap(
    app: &tauri::AppHandle<Wry>,
    layout: OverlayLayout,
) -> anyhow::Result<OverlaySnapEvaluation> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| anyhow::anyhow!("recording overlay window is unavailable"))?;
    #[cfg(target_os = "macos")]
    let (origin, size, window_scale) = stable_scaled_window_geometry(&window)?;
    #[cfg(not(target_os = "macos"))]
    let (origin, size, window_scale) = (window.outer_position()?, window.outer_size()?, 1.0);

    let monitor = destination_overlay_monitor(app, origin, size, window_scale)?
        .ok_or_else(|| anyhow::anyhow!("no display is available for the recording overlay"))?;
    #[cfg(target_os = "macos")]
    let origin_scale = window_scale;
    #[cfg(not(target_os = "macos"))]
    let origin_scale = monitor.scale_factor().max(f64::EPSILON);

    let current_logical_origin = (
        origin.x as f64 / origin_scale,
        origin.y as f64 / origin_scale,
    );
    let default_layout = OverlayLayout {
        offset_x: 0,
        offset_y: 0,
        ..layout
    };
    let target_logical_origin = overlay_origin(logical_work_area(&monitor), default_layout);
    #[cfg(not(target_os = "macos"))]
    let target_scale = monitor.scale_factor().max(f64::EPSILON);
    #[cfg(not(target_os = "macos"))]
    let target_physical_origin = PhysicalPosition {
        x: (target_logical_origin.0 * target_scale).round() as i32,
        y: (target_logical_origin.1 * target_scale).round() as i32,
    };
    #[cfg(not(target_os = "macos"))]
    let target_physical_size = PhysicalSize {
        width: (layout.width * target_scale).round().max(1.0) as u32,
        height: (layout.height * target_scale).round().max(1.0) as u32,
    };

    #[cfg(target_os = "macos")]
    let primary_height = app
        .primary_monitor()?
        .map(|primary| primary.size().height as f64 / primary.scale_factor().max(f64::EPSILON))
        .unwrap_or_else(|| logical_work_area(&monitor).height);
    let dx = current_logical_origin.0 - target_logical_origin.0;
    let dy = current_logical_origin.1 - target_logical_origin.1;
    Ok(OverlaySnapEvaluation {
        #[cfg(target_os = "macos")]
        current_logical_origin,
        current_physical_origin: origin,
        target: OverlaySnapTarget {
            #[cfg(target_os = "macos")]
            logical_origin: target_logical_origin,
            #[cfg(not(target_os = "macos"))]
            physical_origin: target_physical_origin,
            #[cfg(not(target_os = "macos"))]
            physical_size: target_physical_size,
            #[cfg(target_os = "macos")]
            primary_height,
        },
        distance_squared: dx * dx + dy * dy,
    })
}

fn ease_out_cubic(progress: f64) -> f64 {
    let clamped = progress.clamp(0.0, 1.0);
    1.0 - (1.0 - clamped).powi(3)
}

#[cfg(target_os = "macos")]
fn animate_overlay_to_snap_target(
    app: &tauri::AppHandle<Wry>,
    id: u64,
    evaluation: OverlaySnapEvaluation,
) -> anyhow::Result<bool> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| anyhow::anyhow!("recording overlay window is unavailable"))?;
    let start = evaluation.current_logical_origin;
    let target = evaluation.target.logical_origin;

    for frame in 1..=OVERLAY_SNAP_ANIMATION_FRAMES {
        if !overlay_drag_is_current(id) {
            return Ok(false);
        }
        let eased = ease_out_cubic(frame as f64 / OVERLAY_SNAP_ANIMATION_FRAMES as f64);
        let x = start.0 + (target.0 - start.0) * eased;
        let y = start.1 + (target.1 - start.1) * eased;
        if !set_macos_overlay_top_left(app, &window, id, (x, y), evaluation.target.primary_height)?
        {
            return Ok(false);
        }
        if frame < OVERLAY_SNAP_ANIMATION_FRAMES {
            thread::sleep(OVERLAY_SNAP_ANIMATION_FRAME_INTERVAL);
        }
    }
    Ok(overlay_drag_is_current(id))
}

#[cfg(target_os = "macos")]
fn set_macos_overlay_top_left(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
    id: u64,
    logical_origin: (f64, f64),
    primary_height: f64,
) -> anyhow::Result<bool> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let window = window.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<bool, String> {
            if !overlay_drag_is_current(id) {
                return Ok(false);
            }
            let native_window = window.ns_window().map_err(|err| err.to_string())?;
            if native_window.is_null() {
                return Err("recording overlay native window is unavailable".to_string());
            }
            let ns_window = unsafe { &*(native_window as *const AppKitWindow) };
            ns_window.setFrameTopLeftPoint(AppKitPoint::new(
                logical_origin.0,
                primary_height - logical_origin.1,
            ));
            Ok(true)
        })();
        let _ = sender.send(result);
    })?;
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|err| anyhow::anyhow!("overlay animation main-thread timeout: {err}"))?
        .map_err(anyhow::Error::msg)
}

#[cfg(not(target_os = "macos"))]
fn animate_overlay_to_snap_target(
    app: &tauri::AppHandle<Wry>,
    id: u64,
    evaluation: OverlaySnapEvaluation,
) -> anyhow::Result<bool> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| anyhow::anyhow!("recording overlay window is unavailable"))?;
    let start = evaluation.current_physical_origin;
    let target = evaluation.target.physical_origin;

    for frame in 1..=OVERLAY_SNAP_ANIMATION_FRAMES {
        if !overlay_drag_is_current(id) {
            return Ok(false);
        }
        let eased = ease_out_cubic(frame as f64 / OVERLAY_SNAP_ANIMATION_FRAMES as f64);
        let x = start.x as f64 + (target.x - start.x) as f64 * eased;
        let y = start.y as f64 + (target.y - start.y) as f64 * eased;
        if !set_overlay_physical_position(
            app,
            &window,
            id,
            PhysicalPosition {
                x: x.round() as i32,
                y: y.round() as i32,
            },
        )? {
            return Ok(false);
        }
        if frame < OVERLAY_SNAP_ANIMATION_FRAMES {
            thread::sleep(OVERLAY_SNAP_ANIMATION_FRAME_INTERVAL);
        }
    }
    Ok(overlay_drag_is_current(id))
}

#[cfg(not(target_os = "macos"))]
fn set_overlay_physical_position(
    app: &tauri::AppHandle<Wry>,
    window: &WebviewWindow<Wry>,
    id: u64,
    position: PhysicalPosition<i32>,
) -> anyhow::Result<bool> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let window = window.clone();
    app.run_on_main_thread(move || {
        let result = if overlay_drag_is_current(id) {
            window
                .set_position(Position::Physical(position))
                .map(|_| true)
                .map_err(|err| err.to_string())
        } else {
            Ok(false)
        };
        let _ = sender.send(result);
    })?;
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|err| anyhow::anyhow!("overlay animation main-thread timeout: {err}"))?
        .map_err(anyhow::Error::msg)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LogicalRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PhysicalRectGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PhysicalPoint {
    x: f64,
    y: f64,
}

#[cfg(not(target_os = "macos"))]
fn physical_monitor_rect(monitor: &Monitor) -> PhysicalRectGeometry {
    let position = monitor.position();
    let size = monitor.size();
    PhysicalRectGeometry {
        x: position.x as f64,
        y: position.y as f64,
        width: size.width as f64,
        height: size.height as f64,
    }
}

#[cfg(target_os = "macos")]
fn logical_monitor_rect(monitor: &Monitor) -> PhysicalRectGeometry {
    let scale = monitor.scale_factor().max(f64::EPSILON);
    let position = monitor.position();
    let size = monitor.size();
    PhysicalRectGeometry {
        x: position.x as f64 / scale,
        y: position.y as f64 / scale,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
    }
}

#[cfg(target_os = "macos")]
fn scaled_global_window_center(
    origin: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    scale: f64,
) -> PhysicalPoint {
    let scale = scale.max(f64::EPSILON);
    PhysicalPoint {
        x: (origin.x as f64 + size.width as f64 / 2.0) / scale,
        y: (origin.y as f64 + size.height as f64 / 2.0) / scale,
    }
}

fn select_destination_monitor(
    monitors: &[PhysicalRectGeometry],
    center: PhysicalPoint,
    locked_index: Option<usize>,
) -> Option<usize> {
    monitors
        .iter()
        .position(|monitor| monitor.contains(center))
        .or_else(|| locked_index.filter(|index| *index < monitors.len()))
        .or_else(|| {
            monitors
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| {
                    left.distance_squared(center)
                        .total_cmp(&right.distance_squared(center))
                })
                .map(|(index, _)| index)
        })
}

impl PhysicalRectGeometry {
    fn contains(self, point: PhysicalPoint) -> bool {
        point.x >= self.x
            && point.x < self.x + self.width
            && point.y >= self.y
            && point.y < self.y + self.height
    }

    fn distance_squared(self, point: PhysicalPoint) -> f64 {
        let max_x = self.x + self.width;
        let max_y = self.y + self.height;
        let dx = if point.x < self.x {
            self.x - point.x
        } else if point.x > max_x {
            point.x - max_x
        } else {
            0.0
        };
        let dy = if point.y < self.y {
            self.y - point.y
        } else if point.y > max_y {
            point.y - max_y
        } else {
            0.0
        };
        dx * dx + dy * dy
    }
}

fn logical_work_area(monitor: &Monitor) -> LogicalRect {
    let scale = monitor.scale_factor().max(f64::EPSILON);
    let work_area = monitor.work_area();
    logical_rect_from_physical(
        work_area.position.x,
        work_area.position.y,
        work_area.size.width,
        work_area.size.height,
        scale,
    )
}

fn logical_rect_from_physical(x: i32, y: i32, width: u32, height: u32, scale: f64) -> LogicalRect {
    let scale = scale.max(f64::EPSILON);
    LogicalRect {
        x: x as f64 / scale,
        y: y as f64 / scale,
        width: width as f64 / scale,
        height: height as f64 / scale,
    }
}

fn overlay_anchor(work_area: LogicalRect, layout: OverlayLayout) -> (f64, f64) {
    (
        work_area.x + ((work_area.width - layout.width) / 2.0).max(0.0),
        work_area.y + work_area.height - layout.height - OVERLAY_BOTTOM_MARGIN,
    )
}

fn overlay_origin(work_area: LogicalRect, layout: OverlayLayout) -> (f64, f64) {
    let min_x = work_area.x;
    let max_x = work_area.x + work_area.width - layout.width;
    let min_y = work_area.y;
    let max_y = work_area.y + work_area.height - layout.height;
    let (base_x, base_y) = overlay_anchor(work_area, layout);
    let x = (base_x + layout.offset_x as f64).clamp(min_x, max_x.max(min_x));
    let y = (base_y - layout.offset_y as f64).clamp(min_y, max_y.max(min_y));
    (x, y)
}

fn overlay_offsets_from_origin(
    work_area: LogicalRect,
    layout: OverlayLayout,
    origin: (f64, f64),
) -> (i32, i32) {
    let (base_x, base_y) = overlay_anchor(work_area, layout);
    (
        (origin.0 - base_x).round() as i32,
        (base_y - origin.1).round() as i32,
    )
}

#[cfg(not(target_os = "macos"))]
fn position_overlay_window(
    window: &WebviewWindow<Wry>,
    monitor: &Monitor,
    layout: OverlayLayout,
) -> tauri::Result<()> {
    let scale = monitor.scale_factor();
    let (logical_x, logical_y) = overlay_origin(logical_work_area(monitor), layout);
    let x = (logical_x * scale).round() as i32;
    let y = (logical_y * scale).round() as i32;

    window.set_position(Position::Physical(PhysicalPosition { x, y }))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_overlay_placement(
    app: &tauri::AppHandle<Wry>,
    monitor: &Monitor,
    layout: OverlayLayout,
) -> tauri::Result<OverlayPlacement> {
    let work_area = logical_work_area(monitor);
    let (x, y) = overlay_origin(work_area, layout);
    let primary_height = app
        .primary_monitor()?
        .map(|primary| primary.size().height as f64 / primary.scale_factor().max(f64::EPSILON))
        .unwrap_or(work_area.height);

    Ok(OverlayPlacement {
        x,
        y,
        width: layout.width,
        height: layout.height,
        primary_height,
    })
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn overlay_behavior_follows_active_space_and_can_join_current_full_screen() {
        let behavior = overlay_collection_behavior(true);

        assert_eq!(behavior.bits(), 0x4014a);
        assert!(behavior.contains(NSWindowCollectionBehavior::MoveToActiveSpace));
        assert!(behavior.contains(NSWindowCollectionBehavior::FullScreenAuxiliary));
        assert!(behavior.contains(NSWindowCollectionBehavior::CanJoinAllApplications));
        assert!(behavior.contains(NSWindowCollectionBehavior::Transient));
        assert!(behavior.contains(NSWindowCollectionBehavior::IgnoresCycle));
        assert!(!behavior.intersects(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenPrimary
                | NSWindowCollectionBehavior::FullScreenNone
                | NSWindowCollectionBehavior::Primary
                | NSWindowCollectionBehavior::Auxiliary
                | NSWindowCollectionBehavior::Managed
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::ParticipatesInCycle
        ));
    }

    #[test]
    fn overlay_behavior_keeps_pre_macos_13_compatibility() {
        let behavior = overlay_collection_behavior(false);

        assert_eq!(behavior.bits(), 0x14a);
        assert!(behavior.contains(NSWindowCollectionBehavior::MoveToActiveSpace));
        assert!(behavior.contains(NSWindowCollectionBehavior::FullScreenAuxiliary));
        assert!(behavior.contains(NSWindowCollectionBehavior::Transient));
        assert!(!behavior.contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
        assert!(!behavior.contains(NSWindowCollectionBehavior::CanJoinAllApplications));
    }

    #[test]
    fn overlay_origin_handles_negative_coordinate_full_screen_monitor() {
        let work_area = LogicalRect {
            x: -954.0,
            y: -644.0,
            width: 954.0,
            height: 1696.0,
        };
        let layout = OverlayLayout {
            width: 157.0,
            height: 39.0,
            snap_range_percent: OVERLAY_SNAP_RANGE_PERCENT_DEFAULT,
            offset_x: 0,
            offset_y: 0,
        };

        let (x, y) = overlay_origin(work_area, layout);

        assert!((x - -555.5).abs() < f64::EPSILON);
        assert!((y - 985.0).abs() < f64::EPSILON);
    }

    #[test]
    fn overlay_origin_clamps_offsets_inside_work_area() {
        let work_area = LogicalRect {
            x: 100.0,
            y: 50.0,
            width: 800.0,
            height: 600.0,
        };
        let layout = OverlayLayout {
            width: 300.0,
            height: 80.0,
            snap_range_percent: OVERLAY_SNAP_RANGE_PERCENT_DEFAULT,
            offset_x: 4000,
            offset_y: -4000,
        };

        assert_eq!(overlay_origin(work_area, layout), (600.0, 570.0));
    }
}
