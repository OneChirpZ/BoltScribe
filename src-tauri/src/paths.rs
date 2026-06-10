use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub fn app_dir() -> Result<PathBuf, String> {
    if let Some(path) = custom_app_dir()? {
        return Ok(path);
    }
    default_app_dir()
}

pub fn default_app_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|dir| dir.join("BoltScribe"))
        .ok_or_else(|| "Cannot resolve user Application Support directory".to_string())
}

pub fn config_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|dir| dir.join(".boltscribe"))
        .ok_or_else(|| "Cannot resolve user home directory".to_string())
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()
        .map_err(|err| anyhow!(err))?
        .join("config.json"))
}

pub fn data_dir_pointer_path() -> Result<PathBuf> {
    Ok(config_dir()
        .map_err(|err| anyhow!(err))?
        .join("data_dir.txt"))
}

pub fn set_custom_app_dir(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Custom data directory must be an absolute path".to_string());
    }
    let pointer_path = data_dir_pointer_path().map_err(|err| err.to_string())?;
    if let Some(parent) = pointer_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    fs::write(&pointer_path, path.display().to_string())
        .map_err(|err| format!("Failed to write {}: {err}", pointer_path.display()))
}

pub fn clear_custom_app_dir() -> Result<(), String> {
    let pointer_path = data_dir_pointer_path().map_err(|err| err.to_string())?;
    if !pointer_path.exists() {
        return Ok(());
    }
    fs::remove_file(&pointer_path)
        .map_err(|err| format!("Failed to remove {}: {err}", pointer_path.display()))
}

pub fn legacy_config_path() -> Result<PathBuf> {
    Ok(legacy_app_dir()
        .map_err(|err| anyhow!(err))?
        .join("config.json"))
}

pub fn legacy_hidden_config_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow!("Cannot resolve user home directory"))?
        .join(".lightning-speaking")
        .join("config.json"))
}

pub fn history_path() -> Result<PathBuf> {
    Ok(app_dir().map_err(|err| anyhow!(err))?.join("history.jsonl"))
}

pub fn input_stats_path() -> Result<PathBuf> {
    Ok(app_dir()
        .map_err(|err| anyhow!(err))?
        .join("input_stats.jsonl"))
}

pub fn legacy_history_path() -> Result<PathBuf> {
    Ok(legacy_app_dir()
        .map_err(|err| anyhow!(err))?
        .join("history.jsonl"))
}

pub fn recordings_dir() -> Result<PathBuf> {
    Ok(app_dir().map_err(|err| anyhow!(err))?.join("recordings"))
}

fn legacy_app_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|dir| dir.join("LightningSpeaking"))
        .ok_or_else(|| "Cannot resolve user Application Support directory".to_string())
}

fn custom_app_dir() -> Result<Option<PathBuf>, String> {
    let pointer_path = data_dir_pointer_path().map_err(|err| err.to_string())?;
    if !pointer_path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&pointer_path)
        .map_err(|err| format!("Failed to read {}: {err}", pointer_path.display()))?;
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }

    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(format!(
            "Custom data directory in {} must be absolute",
            pointer_path.display()
        ));
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_uses_hidden_home_directory() {
        let path = config_path().unwrap();
        assert!(path.ends_with(".boltscribe/config.json"));
    }

    #[test]
    fn legacy_hidden_config_path_keeps_previous_hidden_location() {
        let path = legacy_hidden_config_path().unwrap();
        assert!(path.ends_with(".lightning-speaking/config.json"));
    }

    #[test]
    fn legacy_config_path_keeps_application_support_location() {
        let path = legacy_config_path().unwrap();
        assert!(path.ends_with("LightningSpeaking/config.json"));
    }

    #[test]
    fn default_app_dir_stays_in_application_support() {
        assert!(default_app_dir().unwrap().ends_with("BoltScribe"));
    }

    #[test]
    fn legacy_history_path_keeps_previous_application_support_location() {
        assert!(legacy_history_path()
            .unwrap()
            .ends_with("LightningSpeaking/history.jsonl"));
    }
}
