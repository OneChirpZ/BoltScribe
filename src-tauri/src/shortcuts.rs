use crate::{config, mouse_shortcuts, workflow};
use std::collections::HashSet;
use tauri::Wry;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

#[derive(Debug)]
struct ValidatedHotkeys {
    keyboard: Vec<String>,
    mouse: Vec<mouse_shortcuts::MouseShortcut>,
}

fn initial_keyboard_hotkeys() -> Vec<String> {
    let config = config::ConfigStore::load().unwrap_or_default();
    config
        .active_hotkeys()
        .into_iter()
        .filter(|hotkey| !matches!(mouse_shortcuts::parse(hotkey), Ok(Some(_))))
        .collect()
}

pub(crate) fn global_shortcut_plugin() -> tauri::plugin::TauriPlugin<Wry> {
    let hotkeys = initial_keyboard_hotkeys();
    let builder =
        tauri_plugin_global_shortcut::Builder::<Wry>::new().with_handler(global_shortcut_handler);
    if hotkeys.is_empty() {
        return builder.build();
    }

    match builder.with_shortcuts(hotkeys.iter().map(String::as_str)) {
        Ok(builder) => builder.build(),
        Err(err) => {
            eprintln!("invalid hotkeys {hotkeys:?}, falling back to PageUp: {err}");
            tauri_plugin_global_shortcut::Builder::<Wry>::new()
                .with_handler(global_shortcut_handler)
                .with_shortcuts(["PageUp"])
                .expect("PageUp shortcut should be valid")
                .build()
        }
    }
}

pub(crate) fn apply_global_shortcuts(
    app: &tauri::AppHandle<Wry>,
    config: &config::AppConfig,
) -> Result<(), String> {
    let hotkeys = validate_hotkeys(config)?;
    app.global_shortcut()
        .unregister_all()
        .map_err(|err| format!("Failed to unregister existing shortcuts: {err}"))?;
    mouse_shortcuts::apply(app, &[])?;

    if !hotkeys.keyboard.is_empty() {
        app.global_shortcut()
            .register_multiple(hotkeys.keyboard.iter().map(String::as_str))
            .map_err(|err| format!("Failed to register shortcuts {:?}: {err}", hotkeys.keyboard))?;
    }

    mouse_shortcuts::apply(app, &hotkeys.mouse)?;
    Ok(())
}

pub(crate) fn apply_startup_mouse_shortcuts(
    app: &tauri::AppHandle<Wry>,
    config: &config::AppConfig,
) -> Result<(), String> {
    let mouse = validate_mouse_hotkeys(config)?;
    mouse_shortcuts::apply(app, &mouse)
}

fn validate_hotkeys(config: &config::AppConfig) -> Result<ValidatedHotkeys, String> {
    let mut keyboard = Vec::new();
    let mut mouse = Vec::new();
    let mut keyboard_ids = HashSet::new();
    let mut mouse_ids = HashSet::new();

    for hotkey in active_config_hotkeys(config) {
        if let Some(mouse_shortcut) = mouse_shortcuts::parse(&hotkey)? {
            if !mouse_ids.insert(mouse_shortcut) {
                return Err(format!("Duplicate shortcut '{hotkey}'"));
            }
            mouse.push(mouse_shortcut);
            continue;
        }

        let shortcut = hotkey
            .parse::<Shortcut>()
            .map_err(|err| format!("Invalid shortcut '{hotkey}': {err}"))?;
        if !keyboard_ids.insert(shortcut.id()) {
            return Err(format!("Duplicate shortcut '{hotkey}'"));
        }
        keyboard.push(hotkey);
    }

    Ok(ValidatedHotkeys { keyboard, mouse })
}

fn validate_mouse_hotkeys(
    config: &config::AppConfig,
) -> Result<Vec<mouse_shortcuts::MouseShortcut>, String> {
    let mut mouse = Vec::new();
    let mut mouse_ids = HashSet::new();
    for hotkey in active_config_hotkeys(config) {
        let Some(mouse_shortcut) = mouse_shortcuts::parse(&hotkey)? else {
            continue;
        };
        if !mouse_ids.insert(mouse_shortcut) {
            return Err(format!("Duplicate shortcut '{hotkey}'"));
        }
        mouse.push(mouse_shortcut);
    }
    Ok(mouse)
}

fn active_config_hotkeys(config: &config::AppConfig) -> Vec<String> {
    config
        .hotkey_slots()
        .into_iter()
        .zip(config.hotkey_enabled_slots())
        .filter_map(|(hotkey, enabled)| {
            let hotkey = hotkey.trim();
            if enabled && !hotkey.is_empty() {
                Some(hotkey.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn global_shortcut_handler(
    app: &tauri::AppHandle<Wry>,
    _shortcut: &tauri_plugin_global_shortcut::Shortcut,
    event: tauri_plugin_global_shortcut::ShortcutEvent,
) {
    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = workflow::toggle_recording_from_app(app) {
                eprintln!("global shortcut failed: {err:?}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn config_with_hotkeys(hotkeys: Vec<&str>) -> config::AppConfig {
        let mut config = config::AppConfig {
            hotkeys: hotkeys.into_iter().map(ToOwned::to_owned).collect(),
            hotkey_enabled: vec![true, true],
            ..Default::default()
        };
        config.normalize_hotkeys();
        config
    }

    #[test]
    fn validates_two_shortcuts_with_combo_key() {
        let config = config_with_hotkeys(vec!["PageUp", "CmdOrCtrl+Shift+Space"]);

        assert_eq!(
            validate_hotkeys(&config).unwrap().keyboard,
            vec!["PageUp".to_string(), "CmdOrCtrl+Shift+Space".to_string()]
        );
    }

    #[test]
    fn rejects_duplicate_shortcuts() {
        let config = config_with_hotkeys(vec!["Ctrl+Shift+Space", "Shift+Ctrl+Space"]);

        assert!(validate_hotkeys(&config)
            .unwrap_err()
            .contains("Duplicate shortcut"));
    }

    #[test]
    fn rejects_exact_duplicate_shortcuts() {
        let config = config_with_hotkeys(vec!["PageUp", "PageUp"]);

        assert!(validate_hotkeys(&config)
            .unwrap_err()
            .contains("Duplicate shortcut"));
    }

    #[test]
    fn accepts_all_shortcuts_disabled() {
        let mut config = config::AppConfig {
            hotkeys: vec!["PageUp".to_string(), "CmdOrCtrl+Shift+Space".to_string()],
            hotkey_enabled: vec![false, false],
            ..Default::default()
        };
        config.normalize_hotkeys();

        let validated = validate_hotkeys(&config).unwrap();
        assert!(validated.keyboard.is_empty());
        assert!(validated.mouse.is_empty());
    }

    #[test]
    fn validates_mouse_shortcut() {
        let config = config_with_hotkeys(vec!["PageUp", "Ctrl+MouseBack"]);

        let validated = validate_hotkeys(&config).unwrap();

        assert_eq!(validated.keyboard, vec!["PageUp".to_string()]);
        assert_eq!(validated.mouse.len(), 1);
    }

    #[test]
    fn rejects_duplicate_mouse_shortcuts() {
        let config = config_with_hotkeys(vec!["Ctrl+MouseBack", "Control+MouseBack"]);

        assert!(validate_hotkeys(&config)
            .unwrap_err()
            .contains("Duplicate shortcut"));
    }
}
