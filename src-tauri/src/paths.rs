use anyhow::{anyhow, Result};
use std::path::PathBuf;

pub fn app_dir() -> Result<PathBuf, String> {
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
    fn history_and_recordings_stay_in_application_support() {
        assert!(history_path()
            .unwrap()
            .ends_with("BoltScribe/history.jsonl"));
        assert!(input_stats_path()
            .unwrap()
            .ends_with("BoltScribe/input_stats.jsonl"));
        assert!(recordings_dir().unwrap().ends_with("BoltScribe/recordings"));
    }

    #[test]
    fn legacy_history_path_keeps_previous_application_support_location() {
        assert!(legacy_history_path()
            .unwrap()
            .ends_with("LightningSpeaking/history.jsonl"));
    }
}
