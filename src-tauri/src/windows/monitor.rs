use tauri::{Manager, Monitor, Wry};

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

fn focused_foreground_window_center() -> Option<(f64, f64)> {
    let script = r#"
tell application "System Events"
  set frontApp to first application process whose frontmost is true
  if name of frontApp is "BoltScribe" then
    return ""
  end if
  set targetElement to missing value
  try
    set targetElement to value of attribute "AXFocusedUIElement" of frontApp
  end try
  if targetElement is not missing value then
    try
      set {x, y} to position of targetElement
      set {w, h} to size of targetElement
      if w > 0 and h > 0 then
        return (((x + (w / 2)) as integer) as text) & "," & (((y + (h / 2)) as integer) as text)
      end if
    end try
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
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let (x, y) = text.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
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
