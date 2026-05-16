use crate::{config, workflow};
use tauri::Wry;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

fn initial_hotkeys() -> Vec<String> {
    let config = config::ConfigStore::load().unwrap_or_default();
    config.active_hotkeys()
}

pub(crate) fn global_shortcut_plugin() -> tauri::plugin::TauriPlugin<Wry> {
    let hotkeys = initial_hotkeys();
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
    if hotkeys.is_empty() {
        return Ok(());
    }

    app.global_shortcut()
        .register_multiple(hotkeys.iter().map(String::as_str))
        .map_err(|err| format!("Failed to register shortcuts {hotkeys:?}: {err}"))?;
    Ok(())
}

fn validate_hotkeys(config: &config::AppConfig) -> Result<Vec<String>, String> {
    let hotkeys = config
        .hotkey_slots()
        .into_iter()
        .zip(config.hotkey_enabled_slots())
        .filter_map(|(hotkey, enabled)| {
            if enabled && !hotkey.is_empty() {
                Some(hotkey)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut ids = std::collections::HashSet::new();
    for hotkey in &hotkeys {
        let shortcut = hotkey
            .parse::<Shortcut>()
            .map_err(|err| format!("Invalid shortcut '{hotkey}': {err}"))?;
        if !ids.insert(shortcut.id()) {
            return Err(format!("Duplicate shortcut '{hotkey}'"));
        }
    }
    Ok(hotkeys)
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
            validate_hotkeys(&config).unwrap(),
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

        assert!(validate_hotkeys(&config).unwrap().is_empty());
    }
}
