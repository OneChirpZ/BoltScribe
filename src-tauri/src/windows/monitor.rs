use tauri::{Manager, Monitor, Wry};

#[cfg(target_os = "macos")]
use core_foundation::{
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
#[cfg(target_os = "macos")]
use core_graphics::{
    geometry::CGRect,
    window::{
        create_description_from_array, create_window_list, kCGNullWindowID, kCGWindowAlpha,
        kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
    },
};
#[cfg(target_os = "macos")]
use objc2_app_kit::NSWorkspace;

pub(crate) fn active_overlay_monitor(
    app: &tauri::AppHandle<Wry>,
) -> tauri::Result<Option<Monitor>> {
    #[cfg(target_os = "macos")]
    if let Some(monitor) = focused_foreground_monitor(app)? {
        log_selected_monitor("frontmost-window", &monitor);
        return Ok(Some(monitor));
    }

    #[cfg(not(target_os = "macos"))]
    if let Some((x, y)) = focused_foreground_window_center() {
        if let Some(monitor) = monitor_from_point_variants(app, x, y)? {
            log_selected_monitor("frontmost-window", &monitor);
            return Ok(Some(monitor));
        }
    }

    if let Ok(cursor) = app.cursor_position() {
        #[cfg(target_os = "macos")]
        if let Some(primary) = app.primary_monitor()? {
            let scale = primary.scale_factor();
            if scale > 0.0 {
                if let Some(monitor) =
                    monitor_from_point_variants(app, cursor.x / scale, cursor.y / scale)?
                {
                    log_selected_monitor("cursor", &monitor);
                    return Ok(Some(monitor));
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        if let Some(monitor) = app.monitor_from_point(cursor.x, cursor.y)? {
            log_selected_monitor("cursor", &monitor);
            return Ok(Some(monitor));
        }
    }

    if let Some(main) = app.get_webview_window("main") {
        if let Some(monitor) = main.current_monitor()? {
            log_selected_monitor("main-window", &monitor);
            return Ok(Some(monitor));
        }
    }

    let monitor = app.primary_monitor()?;
    if let Some(monitor) = &monitor {
        log_selected_monitor("primary", monitor);
    }
    Ok(monitor)
}

#[cfg(target_os = "macos")]
fn focused_foreground_monitor(app: &tauri::AppHandle<Wry>) -> tauri::Result<Option<Monitor>> {
    let workspace = NSWorkspace::sharedWorkspace();
    let Some(frontmost) = workspace.frontmostApplication() else {
        return Ok(None);
    };
    let process_id = frontmost.processIdentifier();
    if process_id <= 0 || process_id as u32 == std::process::id() {
        return Ok(None);
    }

    let window_bounds = foreground_window_bounds(process_id);
    if window_bounds.is_empty() {
        return Ok(None);
    }

    let monitors = app.available_monitors()?;
    let monitor_rects: Vec<ScreenRect> = monitors.iter().map(logical_monitor_rect).collect();
    Ok(select_monitor_index(&monitor_rects, &window_bounds)
        .and_then(|index| monitors.into_iter().nth(index)))
}

#[cfg(target_os = "macos")]
fn foreground_window_bounds(process_id: i32) -> Vec<ScreenRect> {
    let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let Some(window_ids) = create_window_list(options, kCGNullWindowID) else {
        return Vec::new();
    };
    let Some(window_info) = create_description_from_array(window_ids) else {
        return Vec::new();
    };

    let owner_pid_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) };
    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };
    let alpha_key = unsafe { CFString::wrap_under_get_rule(kCGWindowAlpha) };
    let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };

    window_info
        .iter()
        .filter_map(|info| {
            let owner_pid = dictionary_number(&info, &owner_pid_key)?.to_i32()?;
            let layer = dictionary_number(&info, &layer_key)?.to_i32()?;
            let alpha = dictionary_number(&info, &alpha_key)?.to_f64()?;
            if owner_pid != process_id || layer != 0 || alpha <= 0.01 {
                return None;
            }

            let bounds = info
                .find(&bounds_key)?
                .downcast::<CFDictionary>()
                .and_then(|bounds| CGRect::from_dict_representation(&bounds))?;
            let bounds = ScreenRect {
                x: bounds.origin.x,
                y: bounds.origin.y,
                width: bounds.size.width,
                height: bounds.size.height,
            };
            (bounds.width >= 64.0 && bounds.height >= 64.0).then_some(bounds)
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn dictionary_number(
    dictionary: &CFDictionary<CFString, CFType>,
    key: &CFString,
) -> Option<CFNumber> {
    dictionary.find(key)?.downcast::<CFNumber>()
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScreenRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
impl ScreenRect {
    fn center(self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

#[cfg(target_os = "macos")]
fn logical_monitor_rect(monitor: &Monitor) -> ScreenRect {
    let scale = monitor.scale_factor().max(f64::EPSILON);
    let position = monitor.position();
    let size = monitor.size();
    ScreenRect {
        x: position.x as f64 / scale,
        y: position.y as f64 / scale,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
    }
}

#[cfg(target_os = "macos")]
fn select_monitor_index(monitors: &[ScreenRect], windows: &[ScreenRect]) -> Option<usize> {
    for window in windows {
        let (center_x, center_y) = window.center();
        if let Some(index) = monitors
            .iter()
            .position(|monitor| monitor.contains(center_x, center_y))
        {
            return Some(index);
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn focused_foreground_window_center() -> Option<(f64, f64)> {
    use ::windows::Win32::Foundation::{HWND, RECT};
    use ::windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd == HWND(0) || foreground_process_id(hwnd) == Some(std::process::id()) {
        return None;
    }

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return None;
    }

    Some((
        rect.left as f64 + width as f64 / 2.0,
        rect.top as f64 + height as f64 / 2.0,
    ))
}

#[cfg(target_os = "windows")]
fn foreground_process_id(hwnd: ::windows::Win32::Foundation::HWND) -> Option<u32> {
    let mut process_id = 0u32;
    unsafe {
        ::windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            hwnd,
            Some(&mut process_id as *mut u32),
        )
    };
    if process_id == 0 {
        None
    } else {
        Some(process_id)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn focused_foreground_window_center() -> Option<(f64, f64)> {
    None
}

fn monitor_from_point_variants(
    app: &tauri::AppHandle<Wry>,
    x: f64,
    y: f64,
) -> tauri::Result<Option<Monitor>> {
    if let Some(monitor) = app.monitor_from_point(x, y)? {
        return Ok(Some(monitor));
    }

    for monitor in app.available_monitors()? {
        if monitor_contains_logical_point(&monitor, x, y)
            || monitor_contains_physical_point(&monitor, x, y)
        {
            return Ok(Some(monitor));
        }
    }

    Ok(None)
}

fn monitor_contains_physical_point(monitor: &Monitor, x: f64, y: f64) -> bool {
    let position = monitor.position();
    let size = monitor.size();
    x >= position.x as f64
        && y >= position.y as f64
        && x < position.x as f64 + size.width as f64
        && y < position.y as f64 + size.height as f64
}

fn monitor_contains_logical_point(monitor: &Monitor, x: f64, y: f64) -> bool {
    let scale = monitor.scale_factor();
    if scale <= 0.0 {
        return false;
    }

    let position = monitor.position();
    let size = monitor.size();
    let left = position.x as f64 / scale;
    let top = position.y as f64 / scale;
    let right = left + size.width as f64 / scale;
    let bottom = top + size.height as f64 / scale;
    x >= left && y >= top && x < right && y < bottom
}

fn log_selected_monitor(source: &str, monitor: &Monitor) {
    let position = monitor.position();
    let size = monitor.size();
    eprintln!(
        "overlay monitor source={source} position=({}, {}) size={}x{} scale={}",
        position.x,
        position.y,
        size.width,
        size.height,
        monitor.scale_factor()
    );
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn full_screen_window_on_negative_coordinate_monitor_wins() {
        let monitors = [
            ScreenRect {
                x: 0.0,
                y: 0.0,
                width: 1728.0,
                height: 1117.0,
            },
            ScreenRect {
                x: -954.0,
                y: -644.0,
                width: 954.0,
                height: 1696.0,
            },
        ];
        let windows = [ScreenRect {
            x: -954.0,
            y: -576.0,
            width: 954.0,
            height: 1628.0,
        }];

        assert_eq!(select_monitor_index(&monitors, &windows), Some(1));
    }

    #[test]
    fn regular_window_uses_its_center_monitor() {
        let monitors = [
            ScreenRect {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
            ScreenRect {
                x: 1440.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
        ];
        let windows = [ScreenRect {
            x: 1800.0,
            y: 120.0,
            width: 900.0,
            height: 700.0,
        }];

        assert_eq!(select_monitor_index(&monitors, &windows), Some(1));
    }

    #[test]
    fn multiple_full_screen_windows_keep_front_to_back_order() {
        let monitors = [
            ScreenRect {
                x: 0.0,
                y: 0.0,
                width: 1728.0,
                height: 1117.0,
            },
            ScreenRect {
                x: -954.0,
                y: -644.0,
                width: 954.0,
                height: 1696.0,
            },
        ];
        let windows = [
            ScreenRect {
                x: -954.0,
                y: -576.0,
                width: 954.0,
                height: 1628.0,
            },
            ScreenRect {
                x: 0.0,
                y: 0.0,
                width: 1728.0,
                height: 1117.0,
            },
        ];

        assert_eq!(select_monitor_index(&monitors, &windows), Some(1));
    }

    #[test]
    fn focused_regular_window_wins_over_background_full_screen_window() {
        let monitors = [
            ScreenRect {
                x: 0.0,
                y: 0.0,
                width: 1728.0,
                height: 1117.0,
            },
            ScreenRect {
                x: -954.0,
                y: -644.0,
                width: 954.0,
                height: 1696.0,
            },
        ];
        let windows = [
            ScreenRect {
                x: 240.0,
                y: 120.0,
                width: 1100.0,
                height: 760.0,
            },
            ScreenRect {
                x: -954.0,
                y: -644.0,
                width: 954.0,
                height: 1696.0,
            },
        ];

        assert_eq!(select_monitor_index(&monitors, &windows), Some(0));
    }
}
