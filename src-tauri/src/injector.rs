use anyhow::{anyhow, Context, Result};

pub fn paste_text(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Err(anyhow!("Cannot paste empty text"));
    }

    type_text(text)
}

pub fn copy_text(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("Failed to access clipboard")?;
    clipboard
        .set_text(text.to_string())
        .context("Failed to write clipboard")?;

    Ok(())
}

pub fn request_accessibility_permission() -> bool {
    platform::accessibility_trusted(true)
}

pub fn accessibility_permission_granted() -> bool {
    platform::accessibility_trusted(false)
}

pub fn open_accessibility_settings() -> Result<()> {
    platform::open_accessibility_settings()
}

fn type_text(text: &str) -> Result<()> {
    if !accessibility_permission_granted() {
        return Err(anyhow!(
            "Missing Accessibility permission. Enable BoltScribe in System Settings > Privacy & Security > Accessibility, then try again."
        ));
    }

    platform::type_text(text)
}

#[cfg(target_os = "macos")]
mod platform {
    use anyhow::{anyhow, Context, Result};
    use std::ffi::c_void;
    use std::process::Command;
    use std::ptr;
    use std::time::Duration;

    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_VK_ANSI_A: u16 = 0x00;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        static kAXTrustedCheckOptionPrompt: *const c_void;
        static kCFBooleanTrue: *const c_void;

        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        fn CGEventCreateKeyboardEvent(
            source: *const c_void,
            virtual_key: u16,
            key_down: bool,
        ) -> *mut c_void;
        fn CGEventSetFlags(event: *mut c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *mut c_void);
        fn CGEventKeyboardSetUnicodeString(
            event: *mut c_void,
            string_length: usize,
            unicode_string: *const u16,
        );
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    pub fn accessibility_trusted(prompt: bool) -> bool {
        unsafe {
            if !prompt {
                return AXIsProcessTrustedWithOptions(ptr::null());
            }

            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                ptr::null(),
                ptr::null(),
            );
            if options.is_null() {
                return AXIsProcessTrustedWithOptions(ptr::null());
            }

            let trusted = AXIsProcessTrustedWithOptions(options);
            CFRelease(options);
            trusted
        }
    }

    pub fn open_accessibility_settings() -> Result<()> {
        Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .status()
            .context("Failed to open Accessibility settings")?;
        Ok(())
    }

    pub fn type_text(text: &str) -> Result<()> {
        for ch in text.chars() {
            post_unicode_char(ch)?;
            std::thread::sleep(Duration::from_micros(700));
        }
        Ok(())
    }

    fn post_unicode_char(ch: char) -> Result<()> {
        let mut buffer = [0u16; 2];
        let units = ch.encode_utf16(&mut buffer);
        unsafe {
            let key_down = CGEventCreateKeyboardEvent(ptr::null(), K_VK_ANSI_A, true);
            let key_up = CGEventCreateKeyboardEvent(ptr::null(), K_VK_ANSI_A, false);
            if key_down.is_null() || key_up.is_null() {
                if !key_down.is_null() {
                    CFRelease(key_down);
                }
                if !key_up.is_null() {
                    CFRelease(key_up);
                }
                return Err(anyhow!("Failed to create text keyboard event"));
            }

            CGEventSetFlags(key_down, 0);
            CGEventSetFlags(key_up, 0);
            CGEventKeyboardSetUnicodeString(key_down, units.len(), units.as_ptr());
            CGEventPost(K_CG_HID_EVENT_TAP, key_down);
            CGEventPost(K_CG_HID_EVENT_TAP, key_up);
            CFRelease(key_down);
            CFRelease(key_up);
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use anyhow::{anyhow, Result};

    pub fn accessibility_trusted(_prompt: bool) -> bool {
        true
    }

    pub fn open_accessibility_settings() -> Result<()> {
        Ok(())
    }

    pub fn type_text(_text: &str) -> Result<()> {
        Err(anyhow!("Text injection is only implemented on macOS"))
    }
}
