use tauri::{Manager, Monitor, Wry};

#[cfg(target_os = "macos")]
use std::{
    process::Stdio,
    time::{Duration, Instant},
};

pub(crate) fn active_overlay_monitor(
    app: &tauri::AppHandle<Wry>,
) -> tauri::Result<Option<Monitor>> {
    if let Some((x, y)) = focused_foreground_window_center() {
        if let Some(monitor) = monitor_from_point_variants(app, x, y)? {
            return Ok(Some(monitor));
        }
    }

    if let Ok(cursor) = app.cursor_position() {
        if let Some(monitor) = app.monitor_from_point(cursor.x, cursor.y)? {
            return Ok(Some(monitor));
        }
    }

    if let Some(main) = app.get_webview_window("main") {
        if let Some(monitor) = main.current_monitor()? {
            return Ok(Some(monitor));
        }
    }

    app.primary_monitor()
}

#[cfg(target_os = "macos")]
fn focused_foreground_window_center() -> Option<(f64, f64)> {
    let script = r#"
tell application "System Events"
  set frontApp to first application process whose frontmost is true
  if name of frontApp is "BoltScribe" then
    return ""
  end if
  set targetWindow to missing value
  repeat with candidateWindow in windows of frontApp
    try
      if value of attribute "AXFocused" of candidateWindow is true then
        set targetWindow to candidateWindow
        exit repeat
      end if
    end try
  end repeat
  if targetWindow is missing value and exists window 1 of frontApp then
    set targetWindow to window 1 of frontApp
  end if
  if targetWindow is missing value then
    return ""
  end if
  set {x, y} to position of targetWindow
  set {w, h} to size of targetWindow
  return (((x + (w / 2)) as integer) as text) & "," & (((y + (h / 2)) as integer) as text)
end tell
"#;
    let output = run_osascript_with_timeout(script, Duration::from_millis(750))?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let (x, y) = text.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

#[cfg(target_os = "macos")]
fn run_osascript_with_timeout(script: &str, timeout: Duration) -> Option<std::process::Output> {
    let mut child = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started_at = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if started_at.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
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
