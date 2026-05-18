use tauri::Wry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MouseShortcutButton {
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) struct MouseShortcutModifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MouseShortcut {
    modifiers: MouseShortcutModifiers,
    button: MouseShortcutButton,
}

pub(crate) fn parse(value: &str) -> Result<Option<MouseShortcut>, String> {
    let mut modifiers = MouseShortcutModifiers::default();
    let mut button = None;
    let mut unsupported_keys = Vec::new();

    for raw_token in value.split('+') {
        let token = raw_token.trim();
        if token.is_empty() {
            continue;
        }

        if apply_modifier_token(token, &mut modifiers) {
            continue;
        }

        if let Some(next_button) = parse_button(token) {
            if button.replace(next_button).is_some() {
                return Err(format!(
                    "Mouse shortcut '{value}' contains multiple mouse buttons"
                ));
            }
        } else {
            unsupported_keys.push(token.to_string());
        }
    }

    let Some(button) = button else {
        return Ok(None);
    };
    if !unsupported_keys.is_empty() {
        return Err(format!(
            "Mouse shortcut '{value}' cannot include keyboard key '{}'",
            unsupported_keys.join("+")
        ));
    }

    Ok(Some(MouseShortcut { modifiers, button }))
}

pub(crate) fn apply(
    app: &tauri::AppHandle<Wry>,
    shortcuts: &[MouseShortcut],
) -> Result<(), String> {
    platform::apply(app, shortcuts)
}

fn apply_modifier_token(token: &str, modifiers: &mut MouseShortcutModifiers) -> bool {
    match normalized_token(token).as_str() {
        "CTRL" | "CONTROL" => {
            modifiers.ctrl = true;
            true
        }
        "ALT" | "OPTION" => {
            modifiers.alt = true;
            true
        }
        "SHIFT" => {
            modifiers.shift = true;
            true
        }
        "CMD" | "COMMAND" | "SUPER" | "WIN" | "WINDOWS" | "META" => {
            modifiers.meta = true;
            true
        }
        "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCONTROL" | "CMDORCTRL" => {
            if cfg!(target_os = "macos") {
                modifiers.meta = true;
            } else {
                modifiers.ctrl = true;
            }
            true
        }
        _ => false,
    }
}

fn parse_button(token: &str) -> Option<MouseShortcutButton> {
    match normalized_token(token).as_str() {
        "MOUSEMIDDLE" | "MIDDLEMOUSE" | "MOUSEBUTTONMIDDLE" | "MIDDLEMOUSEBUTTON" | "MBUTTON" => {
            Some(MouseShortcutButton::Middle)
        }
        "MOUSEBACK" | "BACKMOUSE" | "MOUSEBUTTONBACK" | "BACKMOUSEBUTTON" | "XBUTTON1" => {
            Some(MouseShortcutButton::Back)
        }
        "MOUSEFORWARD" | "FORWARDMOUSE" | "MOUSEBUTTONFORWARD" | "FORWARDMOUSEBUTTON"
        | "XBUTTON2" => Some(MouseShortcutButton::Forward),
        _ => None,
    }
}

fn normalized_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, ' ' | '_' | '-'))
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{MouseShortcut, MouseShortcutButton, MouseShortcutModifiers};
    use crate::workflow;
    use std::sync::{mpsc, Mutex, OnceLock};
    use tauri::Wry;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
        VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        HC_ACTION, HHOOK, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_MBUTTONDOWN, WM_XBUTTONDOWN,
        XBUTTON1, XBUTTON2,
    };

    static SHORTCUTS: Mutex<Vec<MouseShortcut>> = Mutex::new(Vec::new());
    static EVENT_SENDER: OnceLock<mpsc::Sender<()>> = OnceLock::new();
    static HOOK_INIT: OnceLock<Result<(), String>> = OnceLock::new();

    pub fn apply(app: &tauri::AppHandle<Wry>, shortcuts: &[MouseShortcut]) -> Result<(), String> {
        {
            let mut stored = SHORTCUTS
                .lock()
                .map_err(|_| "Failed to lock mouse shortcuts".to_string())?;
            *stored = shortcuts.to_vec();
        }

        if shortcuts.is_empty() {
            return Ok(());
        }

        ensure_event_worker(app);
        HOOK_INIT.get_or_init(install_hook_thread).clone()
    }

    fn ensure_event_worker(app: &tauri::AppHandle<Wry>) {
        EVENT_SENDER.get_or_init(|| {
            let (sender, receiver) = mpsc::channel();
            let app = app.clone();
            std::thread::spawn(move || {
                while receiver.recv().is_ok() {
                    if let Err(err) = workflow::toggle_recording_from_app(app.clone()) {
                        eprintln!("mouse shortcut failed: {err:?}");
                    }
                }
            });
            sender
        });
    }

    fn install_hook_thread() -> Result<(), String> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let hook_result = unsafe {
                let module = GetModuleHandleW(PCWSTR::null())
                    .map_err(|err| format!("Failed to get module handle: {err}"));
                module.and_then(|module| {
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), HINSTANCE(module.0), 0)
                        .map_err(|err| format!("Failed to install mouse hook: {err}"))
                })
            };

            let _hook = match hook_result {
                Ok(hook) => hook,
                Err(err) => {
                    let _ = sender.send(Err(err));
                    return;
                }
            };
            let _ = sender.send(Ok(()));

            let mut message = MSG::default();
            while unsafe { GetMessageW(&mut message, HWND(0), 0, 0).as_bool() } {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        });

        receiver
            .recv()
            .map_err(|_| "Mouse hook thread exited before initialization".to_string())?
    }

    unsafe extern "system" fn mouse_hook_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code == HC_ACTION as i32 {
            let hook = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            if let Some(button) = event_button(wparam, hook) {
                let modifiers = current_modifiers();
                if matches_shortcut(button, modifiers) {
                    if let Some(sender) = EVENT_SENDER.get() {
                        let _ = sender.send(());
                    }
                    return LRESULT(1);
                }
            }
        }

        CallNextHookEx(HHOOK(0), code, wparam, lparam)
    }

    fn event_button(wparam: WPARAM, hook: &MSLLHOOKSTRUCT) -> Option<MouseShortcutButton> {
        match wparam.0 as u32 {
            WM_MBUTTONDOWN => Some(MouseShortcutButton::Middle),
            WM_XBUTTONDOWN => {
                let xbutton = ((hook.mouseData >> 16) & 0xffff) as u16;
                match xbutton {
                    XBUTTON1 => Some(MouseShortcutButton::Back),
                    XBUTTON2 => Some(MouseShortcutButton::Forward),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn current_modifiers() -> MouseShortcutModifiers {
        MouseShortcutModifiers {
            ctrl: key_pressed(VK_CONTROL) || key_pressed(VK_LCONTROL) || key_pressed(VK_RCONTROL),
            alt: key_pressed(VK_MENU) || key_pressed(VK_LMENU) || key_pressed(VK_RMENU),
            shift: key_pressed(VK_SHIFT) || key_pressed(VK_LSHIFT) || key_pressed(VK_RSHIFT),
            meta: key_pressed(VK_LWIN) || key_pressed(VK_RWIN),
        }
    }

    fn key_pressed(key: VIRTUAL_KEY) -> bool {
        unsafe { GetAsyncKeyState(i32::from(key.0)) as u16 & 0x8000 != 0 }
    }

    fn matches_shortcut(button: MouseShortcutButton, modifiers: MouseShortcutModifiers) -> bool {
        let Ok(shortcuts) = SHORTCUTS.lock() else {
            return false;
        };
        shortcuts
            .iter()
            .any(|shortcut| shortcut.button == button && shortcut.modifiers == modifiers)
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::MouseShortcut;
    use tauri::Wry;

    pub fn apply(_app: &tauri::AppHandle<Wry>, shortcuts: &[MouseShortcut]) -> Result<(), String> {
        if shortcuts.is_empty() {
            Ok(())
        } else {
            Err("Mouse-button shortcuts are currently supported on Windows only".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mouse_shortcut_with_modifiers() {
        let shortcut = parse("Ctrl+Shift+MouseBack").unwrap().unwrap();

        assert_eq!(shortcut.button, MouseShortcutButton::Back);
        assert!(shortcut.modifiers.ctrl);
        assert!(shortcut.modifiers.shift);
        assert!(!shortcut.modifiers.alt);
        assert!(!shortcut.modifiers.meta);
    }

    #[test]
    fn ignores_keyboard_shortcuts() {
        assert!(parse("Ctrl+Shift+Space").unwrap().is_none());
    }

    #[test]
    fn rejects_mouse_shortcut_with_keyboard_key() {
        assert!(parse("MouseMiddle+Space")
            .unwrap_err()
            .contains("keyboard key"));
    }
}
