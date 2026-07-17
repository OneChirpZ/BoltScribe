use super::monitor::active_overlay_monitor;
use crate::config::ConfigStore;
use crate::workflow;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use tauri::{Manager, Monitor, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};

#[cfg(not(target_os = "macos"))]
use tauri::{LogicalSize, PhysicalPosition, Position, Size};

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
static OVERLAY_MONITOR_LOCK: Mutex<Option<LockedMonitor>> = Mutex::new(None);
static OVERLAY_STATUS_REVISION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct LockedMonitor {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
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

    configure_overlay_window(app, &window, false, false, true, 0)
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

pub(crate) fn sync_overlay_window(app: &tauri::AppHandle, status: &workflow::WorkflowStatus) {
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
                let reset_monitor = status.mode == workflow::WorkflowMode::Starting;
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

fn claim_overlay_revision(revision: u64) -> bool {
    revision >= OVERLAY_STATUS_REVISION.fetch_max(revision, Ordering::AcqRel)
}

fn overlay_revision_is_current(revision: u64) -> bool {
    OVERLAY_STATUS_REVISION.load(Ordering::Acquire) == revision
}

fn overlay_should_show(mode: &workflow::WorkflowMode) -> bool {
    matches!(
        mode,
        workflow::WorkflowMode::Starting
            | workflow::WorkflowMode::Recording
            | workflow::WorkflowMode::Processing
            | workflow::WorkflowMode::Error
    )
}

fn should_request_overlay_show(mode: &workflow::WorkflowMode, is_visible: bool) -> bool {
    *mode == workflow::WorkflowMode::Starting || !is_visible
}

#[cfg(test)]
mod cross_platform_tests {
    use super::*;

    #[test]
    fn overlay_is_visible_for_starting_recording_processing_and_error() {
        for mode in [
            workflow::WorkflowMode::Starting,
            workflow::WorkflowMode::Recording,
            workflow::WorkflowMode::Processing,
            workflow::WorkflowMode::Error,
        ] {
            assert!(overlay_should_show(&mode));
        }
        assert!(!overlay_should_show(&workflow::WorkflowMode::Idle));
    }

    #[test]
    fn only_starting_forces_a_fresh_show_for_an_already_visible_panel() {
        assert!(should_request_overlay_show(
            &workflow::WorkflowMode::Starting,
            true
        ));
        assert!(!should_request_overlay_show(
            &workflow::WorkflowMode::Recording,
            true
        ));
        assert!(!should_request_overlay_show(
            &workflow::WorkflowMode::Error,
            true
        ));
        assert!(!should_request_overlay_show(
            &workflow::WorkflowMode::Processing,
            true
        ));
        assert!(should_request_overlay_show(
            &workflow::WorkflowMode::Processing,
            false
        ));
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
    let base_width = if ui.app_language == "en-US" {
        OVERLAY_BASE_WIDTH_EN
    } else {
        OVERLAY_BASE_WIDTH_ZH
    };

    OverlayLayout {
        width: (base_width * scale).ceil(),
        height: (OVERLAY_BASE_HEIGHT * scale).ceil(),
        offset_x: ui.recording_overlay_offset_x,
        offset_y: ui.recording_overlay_offset_y,
    }
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(target_os = "macos")]
fn macos_overlay_placement(
    app: &tauri::AppHandle<Wry>,
    monitor: &Monitor,
    layout: OverlayLayout,
) -> tauri::Result<OverlayPlacement> {
    let scale = monitor.scale_factor().max(f64::EPSILON);
    let work_area = monitor.work_area();
    let work_area = LogicalRect {
        x: work_area.position.x as f64 / scale,
        y: work_area.position.y as f64 / scale,
        width: work_area.size.width as f64 / scale,
        height: work_area.size.height as f64 / scale,
    };
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

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
struct LogicalRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
fn overlay_origin(work_area: LogicalRect, layout: OverlayLayout) -> (f64, f64) {
    let min_x = work_area.x;
    let max_x = work_area.x + work_area.width - layout.width;
    let min_y = work_area.y;
    let max_y = work_area.y + work_area.height - layout.height;
    let base_x = work_area.x + ((work_area.width - layout.width) / 2.0).max(0.0);
    let base_y = work_area.y + work_area.height - layout.height - OVERLAY_BOTTOM_MARGIN;
    let x = (base_x + layout.offset_x as f64).clamp(min_x, max_x.max(min_x));
    let y = (base_y - layout.offset_y as f64).clamp(min_y, max_y.max(min_y));
    (x, y)
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
            offset_x: 4000,
            offset_y: -4000,
        };

        assert_eq!(overlay_origin(work_area, layout), (600.0, 570.0));
    }
}
