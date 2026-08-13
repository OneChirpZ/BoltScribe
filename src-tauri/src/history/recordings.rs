use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn audio_path_from_value(value: &serde_json::Value) -> Option<PathBuf> {
    value
        .get("audio_path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

pub(super) fn normalized_retry_recording_path(
    path: &Path,
    recordings_dirs: &[PathBuf],
) -> Result<PathBuf> {
    if path.extension().and_then(|value| value.to_str()) != Some("wav") {
        bail!("Retry recording must be a .wav file");
    }
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("Retry recording path is outside the recordings directory");
    }

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Retry recording {} is unavailable", path.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) {
        bail!("Retry recording cannot be a symbolic link");
    }
    if !metadata.file_type().is_file() {
        bail!("Retry recording must be a regular file");
    }

    let normalized_path = path
        .canonicalize()
        .with_context(|| format!("Failed to resolve retry recording {}", path.display()))?;
    let normalized_parent = normalized_path
        .parent()
        .ok_or_else(|| anyhow!("Retry recording path has no parent directory"))?;
    let allowed = recordings_dirs.iter().any(|recordings_dir| {
        normalized_recordings_directory(recordings_dir).is_ok_and(|directory| {
            directory
                .as_deref()
                .is_some_and(|directory| directory == normalized_parent)
        })
    });
    if !allowed {
        bail!("Retry recording path is outside the recordings directory");
    }

    Ok(normalized_path)
}

pub(super) fn delete_recording_paths(
    paths: &HashSet<PathBuf>,
    recordings_dirs: &[PathBuf],
) -> Result<(usize, u64)> {
    let mut paths = paths.iter().collect::<Vec<_>>();
    paths.sort();
    let mut deleted_files = 0usize;
    let mut freed_bytes = 0u64;
    let mut errors = Vec::new();

    for path in paths {
        let Some(path) = normalized_safe_recording_path(path, recordings_dirs)? else {
            continue;
        };
        let Some(metadata) = regular_recording_metadata(&path, recordings_dirs)? else {
            continue;
        };
        match fs::remove_file(&path) {
            Ok(()) => {
                deleted_files += 1;
                freed_bytes = freed_bytes.saturating_add(metadata.len());
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => errors.push(format!("{}: {err}", path.display())),
        }
    }

    if !errors.is_empty() {
        bail!("Failed to remove recording files: {}", errors.join("; "));
    }
    Ok((deleted_files, freed_bytes))
}

pub(super) fn regular_recording_metadata(
    path: &Path,
    recordings_dirs: &[PathBuf],
) -> Result<Option<fs::Metadata>> {
    let Some(path) = normalized_safe_recording_path(path, recordings_dirs)? else {
        return Ok(None);
    };
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() => {
            Ok(None)
        }
        Ok(metadata) => Ok(Some(metadata)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

pub(super) fn is_app_managed_recording_path(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some((timestamp, id)) = stem.split_once('_') else {
        return false;
    };
    timestamp.parse::<i64>().is_ok() && uuid::Uuid::parse_str(id).is_ok()
}

pub(super) fn metadata_is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(super) fn normalized_recordings_directory(path: &Path) -> Result<Option<PathBuf>> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Ok(None);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() => {
            Ok(None)
        }
        Ok(_) => path
            .canonicalize()
            .map(Some)
            .with_context(|| format!("Failed to resolve {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Some(path.to_path_buf())),
        Err(err) => Err(err).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

fn is_safe_recording_path(path: &Path, recordings_dir: &Path) -> Result<bool> {
    if path.extension().and_then(|value| value.to_str()) != Some("wav") {
        return Ok(false);
    }
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Ok(false);
    }

    let Some(recordings_dir) = normalized_recordings_directory(recordings_dir)? else {
        return Ok(false);
    };
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let parent = match fs::symlink_metadata(parent) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() => {
            return Ok(false);
        }
        Ok(_) => parent
            .canonicalize()
            .with_context(|| format!("Failed to resolve {}", parent.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => parent.to_path_buf(),
        Err(err) => {
            return Err(err).with_context(|| format!("Failed to inspect {}", parent.display()));
        }
    };
    Ok(parent == recordings_dir)
}

pub(super) fn normalized_safe_recording_path(
    path: &Path,
    recordings_dirs: &[PathBuf],
) -> Result<Option<PathBuf>> {
    for recordings_dir in recordings_dirs {
        if is_safe_recording_path(path, recordings_dir)? {
            let Some(file_name) = path.file_name() else {
                return Ok(None);
            };
            let parent = path.parent().unwrap_or_else(|| Path::new(""));
            let parent = parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf());
            return Ok(Some(parent.join(file_name)));
        }
    }
    Ok(None)
}

pub(super) fn history_recordings_dirs() -> Result<Vec<PathBuf>> {
    let current_dir = paths::recordings_dir()?;
    let legacy_dir = paths::legacy_history_path()?
        .parent()
        .map(|parent| parent.join("recordings"));
    let mut dirs = vec![current_dir];
    if let Some(legacy_dir) = legacy_dir {
        if !dirs.contains(&legacy_dir) {
            dirs.push(legacy_dir);
        }
    }
    Ok(dirs)
}
