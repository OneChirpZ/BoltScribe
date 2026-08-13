use super::model::AppConfig;
use super::normalize::{
    asr_auth_mode_field_present, correction_text_field_presence,
    fill_missing_correction_text_fields,
};
use crate::paths;
use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::Write;

pub struct ConfigStore;

impl ConfigStore {
    pub fn load() -> Result<AppConfig> {
        let path = paths::config_path()?;
        #[cfg(target_os = "windows")]
        restore_interrupted_windows_replace(&path)?;
        if !path.exists() {
            if let Some(config) = Self::migrate_legacy_config(&path)? {
                return Ok(config);
            }
            let config = AppConfig::default();
            Self::save(&config)?;
            return Ok(config);
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config {}", path.display()))?;
        let raw_value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse config {}", path.display()))?;
        let correction_text_presence = correction_text_field_presence(&raw_value);
        let asr_auth_mode_present = asr_auth_mode_field_present(&raw_value);
        let mut config: AppConfig = serde_json::from_value(raw_value)
            .with_context(|| format!("Failed to parse config {}", path.display()))?;
        if !asr_auth_mode_present {
            config.asr.auth_mode.clear();
        }
        config.normalize();
        fill_missing_correction_text_fields(&mut config, correction_text_presence);
        Ok(config)
    }

    fn migrate_legacy_config(path: &std::path::Path) -> Result<Option<AppConfig>> {
        let legacy_paths = [
            paths::legacy_hidden_config_path()?,
            paths::legacy_config_path()?,
        ];
        let Some(legacy_path) = legacy_paths.into_iter().find(|path| path.exists()) else {
            return Ok(None);
        };

        let raw = fs::read_to_string(&legacy_path)
            .with_context(|| format!("Failed to read legacy config {}", legacy_path.display()))?;
        let raw_value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse legacy config {}", legacy_path.display()))?;
        let correction_text_presence = correction_text_field_presence(&raw_value);
        let asr_auth_mode_present = asr_auth_mode_field_present(&raw_value);
        let mut config: AppConfig = serde_json::from_value(raw_value)
            .with_context(|| format!("Failed to parse legacy config {}", legacy_path.display()))?;
        if !asr_auth_mode_present {
            config.asr.auth_mode.clear();
        }
        config.normalize();
        fill_missing_correction_text_fields(&mut config, correction_text_presence);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(&config)?;
        write_config_atomically(path, raw.as_bytes())?;
        Ok(Some(config))
    }

    pub fn save(config: &AppConfig) -> Result<AppConfig> {
        let path = paths::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let mut config = config.clone();
        config.normalize();
        config.validate()?;
        let raw = serde_json::to_string_pretty(&config)?;
        write_config_atomically(&path, raw.as_bytes())?;
        Ok(config)
    }
}

fn write_config_atomically(path: &std::path::Path, contents: &[u8]) -> Result<()> {
    let temp_path = path.with_extension("json.tmp");
    {
        let mut file = File::create(&temp_path)
            .with_context(|| format!("Failed to create {}", temp_path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("Failed to write {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("Failed to sync {}", temp_path.display()))?;
    }

    replace_config_file(&temp_path, path)
}

#[cfg(not(target_os = "windows"))]
fn replace_config_file(temp_path: &std::path::Path, path: &std::path::Path) -> Result<()> {
    fs::rename(temp_path, path).with_context(|| format!("Failed to replace {}", path.display()))
}

#[cfg(target_os = "windows")]
fn replace_config_file(temp_path: &std::path::Path, path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        return fs::rename(temp_path, path)
            .with_context(|| format!("Failed to install {}", path.display()));
    }

    let backup_path = path.with_extension("json.replacing");
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .with_context(|| format!("Failed to remove {}", backup_path.display()))?;
    }
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS};

    let path_wide = windows_path(path);
    let temp_wide = windows_path(temp_path);
    let backup_wide = windows_path(&backup_path);
    unsafe {
        ReplaceFileW(
            PCWSTR(path_wide.as_ptr()),
            PCWSTR(temp_wide.as_ptr()),
            PCWSTR(backup_wide.as_ptr()),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    }
    .with_context(|| format!("Failed to replace {}", path.display()))?;

    if let Err(err) = fs::remove_file(&backup_path) {
        eprintln!(
            "failed to remove committed config backup {}: {err}",
            backup_path.display()
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn restore_interrupted_windows_replace(path: &std::path::Path) -> Result<()> {
    let backup_path = path.with_extension("json.replacing");
    if !path.exists() && backup_path.exists() {
        fs::rename(&backup_path, path)
            .with_context(|| format!("Failed to restore {}", path.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_path(path: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
