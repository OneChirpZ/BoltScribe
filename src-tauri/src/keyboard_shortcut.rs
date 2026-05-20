use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyboardShortcut {
    command: bool,
    option: bool,
    control: bool,
    shift: bool,
    key: ShortcutKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutKey {
    Character(char),
    Space,
    Function(u8),
}

pub(crate) fn parse(value: &str) -> std::result::Result<KeyboardShortcut, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("shortcut is empty".to_string());
    }

    let mut command = false;
    let mut option = false;
    let mut control = false;
    let mut shift = false;
    let mut key = None;

    for raw_token in value.split('+') {
        let token = raw_token.trim();
        if token.is_empty() {
            return Err(format!("shortcut '{value}' contains an empty token"));
        }
        let normalized = token.to_ascii_lowercase();
        match normalized.as_str() {
            "cmd" | "command" => set_modifier(&mut command, token)?,
            "opt" | "option" | "alt" => set_modifier(&mut option, token)?,
            "ctrl" | "control" => set_modifier(&mut control, token)?,
            "shift" => set_modifier(&mut shift, token)?,
            _ => {
                if key.is_some() {
                    return Err(format!("shortcut '{value}' contains multiple main keys"));
                }
                key = Some(parse_key(token)?);
            }
        }
    }

    if !command && !option && !control && !shift {
        return Err(format!(
            "shortcut '{value}' must include at least one modifier"
        ));
    }

    let key = key.ok_or_else(|| format!("shortcut '{value}' has no main key"))?;
    Ok(KeyboardShortcut {
        command,
        option,
        control,
        shift,
        key,
    })
}

pub(crate) fn send(value: &str) -> Result<()> {
    let shortcut =
        parse(value).map_err(|err| anyhow!("Invalid SoundSource mute shortcut: {err}"))?;
    platform::send(shortcut)
}

fn set_modifier(value: &mut bool, token: &str) -> std::result::Result<(), String> {
    if *value {
        return Err(format!("shortcut contains duplicate modifier '{token}'"));
    }
    *value = true;
    Ok(())
}

fn parse_key(token: &str) -> std::result::Result<ShortcutKey, String> {
    if token.eq_ignore_ascii_case("space") {
        return Ok(ShortcutKey::Space);
    }

    if let Some(number) = token.strip_prefix('F').or_else(|| token.strip_prefix('f')) {
        let value = number
            .parse::<u8>()
            .map_err(|_| format!("unknown shortcut key '{token}'"))?;
        if (1..=20).contains(&value) {
            return Ok(ShortcutKey::Function(value));
        }
        return Err(format!("function key '{token}' is outside F1-F20"));
    }

    let mut chars = token.chars();
    let Some(ch) = chars.next() else {
        return Err("shortcut key is empty".to_string());
    };
    if chars.next().is_none() && ch.is_ascii_alphanumeric() {
        return Ok(ShortcutKey::Character(ch.to_ascii_uppercase()));
    }

    Err(format!("unknown shortcut key '{token}'"))
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{KeyboardShortcut, ShortcutKey};
    use anyhow::{anyhow, Result};
    use std::ffi::c_void;
    use std::ptr;
    use std::time::Duration;

    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_FLAG_MASK_SHIFT: u64 = 1 << 17;
    const K_CG_EVENT_FLAG_MASK_CONTROL: u64 = 1 << 18;
    const K_CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 1 << 19;
    const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 1 << 20;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventCreateKeyboardEvent(
            source: *const c_void,
            virtual_key: u16,
            key_down: bool,
        ) -> *mut c_void;
        fn CGEventSetFlags(event: *mut c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *mut c_void);
        fn CFRelease(cf: *const c_void);
    }

    pub(super) fn send(shortcut: KeyboardShortcut) -> Result<()> {
        if !crate::injector::accessibility_permission_granted() {
            return Err(anyhow!(
                "Missing Accessibility permission. Enable BoltScribe in System Settings > Privacy & Security > Accessibility, then try again."
            ));
        }

        let key_code = key_code(shortcut.key)?;
        let flags = event_flags(shortcut);
        post_key(key_code, flags, true)?;
        std::thread::sleep(Duration::from_millis(80));
        post_key(key_code, flags, false)?;
        Ok(())
    }

    fn post_key(key_code: u16, flags: u64, key_down: bool) -> Result<()> {
        unsafe {
            let event = CGEventCreateKeyboardEvent(ptr::null(), key_code, key_down);
            if event.is_null() {
                return Err(anyhow!("Failed to create keyboard shortcut event"));
            }
            CGEventSetFlags(event, flags);
            CGEventPost(K_CG_HID_EVENT_TAP, event);
            CFRelease(event);
            Ok(())
        }
    }

    fn event_flags(shortcut: KeyboardShortcut) -> u64 {
        let mut flags = 0;
        if shortcut.command {
            flags |= K_CG_EVENT_FLAG_MASK_COMMAND;
        }
        if shortcut.option {
            flags |= K_CG_EVENT_FLAG_MASK_ALTERNATE;
        }
        if shortcut.control {
            flags |= K_CG_EVENT_FLAG_MASK_CONTROL;
        }
        if shortcut.shift {
            flags |= K_CG_EVENT_FLAG_MASK_SHIFT;
        }
        flags
    }

    fn key_code(key: ShortcutKey) -> Result<u16> {
        match key {
            ShortcutKey::Character('A') => Ok(0x00),
            ShortcutKey::Character('S') => Ok(0x01),
            ShortcutKey::Character('D') => Ok(0x02),
            ShortcutKey::Character('F') => Ok(0x03),
            ShortcutKey::Character('H') => Ok(0x04),
            ShortcutKey::Character('G') => Ok(0x05),
            ShortcutKey::Character('Z') => Ok(0x06),
            ShortcutKey::Character('X') => Ok(0x07),
            ShortcutKey::Character('C') => Ok(0x08),
            ShortcutKey::Character('V') => Ok(0x09),
            ShortcutKey::Character('B') => Ok(0x0B),
            ShortcutKey::Character('Q') => Ok(0x0C),
            ShortcutKey::Character('W') => Ok(0x0D),
            ShortcutKey::Character('E') => Ok(0x0E),
            ShortcutKey::Character('R') => Ok(0x0F),
            ShortcutKey::Character('Y') => Ok(0x10),
            ShortcutKey::Character('T') => Ok(0x11),
            ShortcutKey::Character('1') => Ok(0x12),
            ShortcutKey::Character('2') => Ok(0x13),
            ShortcutKey::Character('3') => Ok(0x14),
            ShortcutKey::Character('4') => Ok(0x15),
            ShortcutKey::Character('6') => Ok(0x16),
            ShortcutKey::Character('5') => Ok(0x17),
            ShortcutKey::Character('9') => Ok(0x19),
            ShortcutKey::Character('7') => Ok(0x1A),
            ShortcutKey::Character('8') => Ok(0x1C),
            ShortcutKey::Character('0') => Ok(0x1D),
            ShortcutKey::Character('O') => Ok(0x1F),
            ShortcutKey::Character('U') => Ok(0x20),
            ShortcutKey::Character('I') => Ok(0x22),
            ShortcutKey::Character('P') => Ok(0x23),
            ShortcutKey::Character('L') => Ok(0x25),
            ShortcutKey::Character('J') => Ok(0x26),
            ShortcutKey::Character('K') => Ok(0x28),
            ShortcutKey::Character('N') => Ok(0x2D),
            ShortcutKey::Character('M') => Ok(0x2E),
            ShortcutKey::Space => Ok(0x31),
            ShortcutKey::Function(1) => Ok(0x7A),
            ShortcutKey::Function(2) => Ok(0x78),
            ShortcutKey::Function(3) => Ok(0x63),
            ShortcutKey::Function(4) => Ok(0x76),
            ShortcutKey::Function(5) => Ok(0x60),
            ShortcutKey::Function(6) => Ok(0x61),
            ShortcutKey::Function(7) => Ok(0x62),
            ShortcutKey::Function(8) => Ok(0x64),
            ShortcutKey::Function(9) => Ok(0x65),
            ShortcutKey::Function(10) => Ok(0x6D),
            ShortcutKey::Function(11) => Ok(0x67),
            ShortcutKey::Function(12) => Ok(0x6F),
            ShortcutKey::Function(13) => Ok(0x69),
            ShortcutKey::Function(14) => Ok(0x6B),
            ShortcutKey::Function(15) => Ok(0x71),
            ShortcutKey::Function(16) => Ok(0x6A),
            ShortcutKey::Function(17) => Ok(0x40),
            ShortcutKey::Function(18) => Ok(0x4F),
            ShortcutKey::Function(19) => Ok(0x50),
            ShortcutKey::Function(20) => Ok(0x5A),
            _ => Err(anyhow!("Unsupported macOS shortcut key")),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::KeyboardShortcut;
    use anyhow::{anyhow, Result};

    pub(super) fn send(_shortcut: KeyboardShortcut) -> Result<()> {
        Err(anyhow!(
            "SoundSource shortcut fallback is supported on macOS only"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_soundsource_default_shortcut() {
        let shortcut = parse("Cmd+Opt+Ctrl+A").unwrap();

        assert!(shortcut.command);
        assert!(shortcut.option);
        assert!(shortcut.control);
        assert!(!shortcut.shift);
        assert_eq!(shortcut.key, ShortcutKey::Character('A'));
    }

    #[test]
    fn parses_lowercase_shortcut_aliases() {
        let shortcut = parse("cmd+alt+control+space").unwrap();

        assert!(shortcut.command);
        assert!(shortcut.option);
        assert!(shortcut.control);
        assert_eq!(shortcut.key, ShortcutKey::Space);
    }

    #[test]
    fn parses_function_keys() {
        let shortcut = parse("Cmd+F20").unwrap();

        assert!(shortcut.command);
        assert_eq!(shortcut.key, ShortcutKey::Function(20));
    }

    #[test]
    fn rejects_invalid_shortcuts() {
        assert!(parse("").is_err());
        assert!(parse("Cmd+Opt").is_err());
        assert!(parse("Cmd+A+B").is_err());
        assert!(parse("Cmd+Unknown").is_err());
        assert!(parse("Cmd+F21").is_err());
        assert!(parse("A").is_err());
    }
}
