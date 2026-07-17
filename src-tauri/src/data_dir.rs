use crate::paths;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct DataDirInfo {
    pub path: String,
    pub default_path: String,
    pub is_default: bool,
    pub cleanup_warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirSnapshot {
    dirs: BTreeSet<PathBuf>,
    files: BTreeMap<PathBuf, u64>,
}

pub fn info() -> Result<DataDirInfo> {
    info_with_warning(None)
}

pub fn set_data_dir(path: PathBuf) -> Result<DataDirInfo> {
    let current = normalize_absolute_path(&paths::app_dir().map_err(anyhow::Error::msg)?)?;
    let target = normalize_target_path(&path)?;
    let default = normalize_absolute_path(&paths::default_app_dir().map_err(anyhow::Error::msg)?)?;

    if paths_match(&current, &target) {
        persist_data_dir_choice(&target, &default)?;
        return info();
    }

    migrate_data_dir_contents(&current, &target)?;
    persist_data_dir_choice(&target, &default)?;

    let cleanup_warning = if current.exists() {
        remove_dir_contents(&current).err().map(|err| {
            format!(
                "Data directory was changed, but cleanup of {} failed: {err}",
                current.display()
            )
        })
    } else {
        None
    };

    info_with_warning(cleanup_warning)
}

pub fn reset_data_dir() -> Result<DataDirInfo> {
    let default = paths::default_app_dir().map_err(anyhow::Error::msg)?;
    set_data_dir(default)
}

fn info_with_warning(cleanup_warning: Option<String>) -> Result<DataDirInfo> {
    let path = normalize_absolute_path(&paths::app_dir().map_err(anyhow::Error::msg)?)?;
    let default_path =
        normalize_absolute_path(&paths::default_app_dir().map_err(anyhow::Error::msg)?)?;
    Ok(DataDirInfo {
        is_default: paths_match(&path, &default_path),
        path: path.display().to_string(),
        default_path: default_path.display().to_string(),
        cleanup_warning,
    })
}

fn persist_data_dir_choice(target: &Path, default: &Path) -> Result<()> {
    if paths_match(target, default) {
        paths::clear_custom_app_dir().map_err(anyhow::Error::msg)
    } else {
        paths::set_custom_app_dir(target).map_err(anyhow::Error::msg)
    }
}

fn normalize_target_path(path: &Path) -> Result<PathBuf> {
    let path = normalize_absolute_path(path)?;
    if path.file_name().is_none() {
        bail!("Data directory cannot be a filesystem root");
    }
    Ok(path)
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("Data directory must be an absolute path");
    }
    if path.exists() {
        return dunce::canonicalize(path)
            .with_context(|| format!("Failed to resolve {}", path.display()));
    }

    let Some(parent) = path.parent() else {
        return Ok(path.to_path_buf());
    };
    let Some(name) = path.file_name() else {
        return Ok(path.to_path_buf());
    };
    if parent.exists() {
        return Ok(dunce::canonicalize(parent)
            .with_context(|| format!("Failed to resolve {}", parent.display()))?
            .join(name));
    }
    Ok(path.to_path_buf())
}

fn paths_match(left: &Path, right: &Path) -> bool {
    left == right
}

fn migrate_data_dir_contents(source: &Path, target: &Path) -> Result<()> {
    if source.exists() && !source.is_dir() {
        bail!(
            "Current data directory is not a directory: {}",
            source.display()
        );
    }
    if source.exists() && target.starts_with(source) {
        bail!("New data directory cannot be inside the current data directory");
    }
    if target.exists() && source.starts_with(target) {
        bail!("New data directory cannot be a parent of the current data directory");
    }

    ensure_empty_target_dir(target)?;
    if !source.exists() {
        return Ok(());
    }

    let source_snapshot = collect_snapshot(source)?;
    if source_snapshot.dirs.is_empty() && source_snapshot.files.is_empty() {
        return Ok(());
    }

    if let Err(err) = copy_dir_contents(source, target) {
        let _ = remove_dir_contents(target);
        return Err(err);
    }

    let target_snapshot = collect_snapshot(target)?;
    if target_snapshot != source_snapshot {
        let _ = remove_dir_contents(target);
        bail!("Data directory migration verification failed");
    }

    if let Err(err) = rewrite_history_audio_paths(target, source, target) {
        let _ = remove_dir_contents(target);
        return Err(err);
    }
    if let Err(err) = verify_migrated_target(&source_snapshot, target, source, target) {
        let _ = remove_dir_contents(target);
        return Err(err);
    }

    Ok(())
}

fn ensure_empty_target_dir(path: &Path) -> Result<()> {
    if path.exists() {
        if !path.is_dir() {
            bail!(
                "Selected data directory is not a directory: {}",
                path.display()
            );
        }
        if fs::read_dir(path)
            .with_context(|| format!("Failed to read {}", path.display()))?
            .next()
            .is_some()
        {
            bail!("Selected data directory must be empty");
        }
        return Ok(());
    }

    fs::create_dir_all(path).with_context(|| format!("Failed to create {}", path.display()))
}

fn collect_snapshot(root: &Path) -> Result<DirSnapshot> {
    let mut snapshot = DirSnapshot {
        dirs: BTreeSet::new(),
        files: BTreeMap::new(),
    };
    collect_snapshot_inner(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn collect_snapshot_inner(root: &Path, dir: &Path, snapshot: &mut DirSnapshot) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "Data directory migration does not support symlinks: {}",
                path.display()
            );
        }

        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("Failed to relativize {}", path.display()))?
            .to_path_buf();
        if file_type.is_dir() {
            snapshot.dirs.insert(relative);
            collect_snapshot_inner(root, &path, snapshot)?;
        } else if file_type.is_file() {
            snapshot.files.insert(relative, entry.metadata()?.len());
        }
    }
    Ok(())
}

fn copy_dir_contents(source: &Path, target: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("Failed to read {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "Data directory migration does not support symlinks: {}",
                source_path.display()
            );
        }
        if file_type.is_dir() {
            fs::create_dir_all(&target_path)
                .with_context(|| format!("Failed to create {}", target_path.display()))?;
            copy_dir_contents(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn rewrite_history_audio_paths(
    data_dir: &Path,
    old_data_dir: &Path,
    new_data_dir: &Path,
) -> Result<()> {
    let history_path = data_dir.join("history.jsonl");
    if !history_path.exists() {
        return Ok(());
    }

    let old_recordings_dir = old_data_dir.join("recordings");
    let new_recordings_dir = new_data_dir.join("recordings");
    let temp_path = history_path.with_extension("jsonl.migrating");
    let input = fs::File::open(&history_path)
        .with_context(|| format!("Failed to open {}", history_path.display()))?;
    let reader = BufReader::new(input);
    let mut output = fs::File::create(&temp_path)
        .with_context(|| format!("Failed to create {}", temp_path.display()))?;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            writeln!(output)?;
            continue;
        }

        match rewrite_history_line(&line, &old_recordings_dir, &new_recordings_dir)? {
            Some(line) => writeln!(output, "{line}")?,
            None => writeln!(output, "{line}")?,
        }
    }
    output.sync_all()?;
    drop(output);

    fs::remove_file(&history_path)
        .with_context(|| format!("Failed to replace {}", history_path.display()))?;
    fs::rename(&temp_path, &history_path)
        .with_context(|| format!("Failed to replace {}", history_path.display()))?;
    Ok(())
}

fn rewrite_history_line(
    line: &str,
    old_recordings_dir: &Path,
    new_recordings_dir: &Path,
) -> Result<Option<String>> {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Ok(None);
    };
    let Some(audio_path_value) = value.get_mut("audio_path") else {
        return Ok(None);
    };
    let Some(audio_path) = audio_path_value.as_str() else {
        return Ok(None);
    };
    let audio_path = PathBuf::from(audio_path);
    let Ok(relative_path) = audio_path.strip_prefix(old_recordings_dir) else {
        return Ok(None);
    };

    *audio_path_value =
        serde_json::Value::String(new_recordings_dir.join(relative_path).display().to_string());
    Ok(Some(serde_json::to_string(&value)?))
}

fn verify_migrated_target(
    source_snapshot: &DirSnapshot,
    target: &Path,
    old_data_dir: &Path,
    new_data_dir: &Path,
) -> Result<()> {
    let target_snapshot = collect_snapshot(target)?;
    if target_snapshot.dirs != source_snapshot.dirs {
        bail!("Data directory migration verification failed: directory list changed");
    }

    let source_files = source_snapshot.files.keys().collect::<BTreeSet<_>>();
    let target_files = target_snapshot.files.keys().collect::<BTreeSet<_>>();
    if target_files != source_files {
        bail!("Data directory migration verification failed: file list changed");
    }

    let history_path = PathBuf::from("history.jsonl");
    for (path, source_size) in &source_snapshot.files {
        if path == &history_path {
            continue;
        }
        if target_snapshot.files.get(path) != Some(source_size) {
            bail!(
                "Data directory migration verification failed: file size changed for {}",
                path.display()
            );
        }
    }

    verify_history_audio_paths(target, old_data_dir, new_data_dir)
}

fn verify_history_audio_paths(
    data_dir: &Path,
    old_data_dir: &Path,
    new_data_dir: &Path,
) -> Result<()> {
    let history_path = data_dir.join("history.jsonl");
    if !history_path.exists() {
        return Ok(());
    }
    let old_recordings_dir = old_data_dir.join("recordings");
    let new_recordings_dir = new_data_dir.join("recordings");
    let input = fs::File::open(&history_path)
        .with_context(|| format!("Failed to open {}", history_path.display()))?;
    let reader = BufReader::new(input);

    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(audio_path) = value.get("audio_path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let audio_path = PathBuf::from(audio_path);
        if audio_path.starts_with(&old_recordings_dir) {
            bail!(
                "Data directory migration verification failed: history still references {}",
                old_recordings_dir.display()
            );
        }
        if audio_path.starts_with(data_dir.join("recordings"))
            && !audio_path.starts_with(&new_recordings_dir)
        {
            bail!("Data directory migration verification failed: history recording path is inconsistent");
        }
    }

    Ok(())
}

fn remove_dir_contents(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.file_name().is_none() {
        bail!("Refusing to clean filesystem root: {}", path.display());
    }
    for entry in fs::read_dir(path).with_context(|| format!("Failed to read {}", path.display()))? {
        let entry = entry?;
        let entry_path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::remove_dir_all(&entry_path)
                .with_context(|| format!("Failed to remove {}", entry_path.display()))?;
        } else {
            fs::remove_file(&entry_path)
                .with_context(|| format!("Failed to remove {}", entry_path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistoryRecord;
    use chrono::Utc;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "boltscribe-data-dir-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_millis()
        ))
    }

    fn sample_record(audio_path: PathBuf) -> HistoryRecord {
        HistoryRecord {
            id: "record-1".to_string(),
            created_at: Utc::now(),
            audio_path: Some(audio_path),
            asr_provider: "mock".to_string(),
            asr_task_id: None,
            audio_started_at: Utc::now(),
            audio_finished_at: Utc::now(),
            audio_sample_rate: 16000,
            audio_channels: 1,
            audio_sample_count: 16000,
            raw_text: "raw".to_string(),
            corrected_text: "corrected".to_string(),
            pasted_text: "corrected".to_string(),
            correction_enabled: true,
            correction_error: None,
            correction_logs: Vec::new(),
            injection_error: None,
            workflow_error: None,
            asr_duration_ms: None,
            service_audio_duration_ms: None,
            total_duration_ms: 1,
        }
    }

    #[test]
    fn migration_copies_files_and_rewrites_history_audio_paths() {
        let base = temp_dir("copy-rewrite");
        let source = base.join("source");
        let target = base.join("target");
        let source_recordings = source.join("recordings");
        fs::create_dir_all(&source_recordings).unwrap();
        let source_audio = source_recordings.join("recording.wav");
        fs::write(&source_audio, b"audio").unwrap();
        fs::write(
            source.join("history.jsonl"),
            format!(
                "{}\n",
                serde_json::to_string(&sample_record(source_audio)).unwrap()
            ),
        )
        .unwrap();
        fs::write(source.join("input_stats.jsonl"), b"{}\n").unwrap();

        migrate_data_dir_contents(&source, &target).unwrap();

        assert!(source.join("history.jsonl").exists());
        assert!(target.join("recordings").join("recording.wav").exists());
        assert_eq!(fs::read(target.join("input_stats.jsonl")).unwrap(), b"{}\n");
        let history = fs::read_to_string(target.join("history.jsonl")).unwrap();
        let record: HistoryRecord = serde_json::from_str(history.trim()).unwrap();
        assert_eq!(
            record.audio_path,
            Some(target.join("recordings").join("recording.wav"))
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn migration_preserves_unknown_history_fields() {
        let base = temp_dir("preserve-history-fields");
        let source = base.join("source");
        let target = base.join("target");
        let source_recordings = source.join("recordings");
        fs::create_dir_all(&source_recordings).unwrap();
        let source_audio = source_recordings.join("recording.wav");
        fs::write(&source_audio, b"audio").unwrap();

        let mut record = serde_json::to_value(sample_record(source_audio)).unwrap();
        record["future_field"] = serde_json::json!({ "kept": true });
        fs::write(source.join("history.jsonl"), format!("{record}\n")).unwrap();

        migrate_data_dir_contents(&source, &target).unwrap();

        let history = fs::read_to_string(target.join("history.jsonl")).unwrap();
        let migrated: serde_json::Value = serde_json::from_str(history.trim()).unwrap();
        assert_eq!(migrated["future_field"]["kept"], true);
        assert_eq!(
            migrated["audio_path"],
            serde_json::json!(target.join("recordings").join("recording.wav"))
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn migration_rejects_non_empty_target() {
        let base = temp_dir("non-empty-target");
        let source = base.join("source");
        let target = base.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("existing.txt"), b"existing").unwrap();

        let error = migrate_data_dir_contents(&source, &target).unwrap_err();

        assert!(error.to_string().contains("must be empty"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn remove_dir_contents_cleans_children_but_keeps_directory() {
        let base = temp_dir("cleanup");
        let child = base.join("child");
        fs::create_dir_all(&child).unwrap();
        fs::write(child.join("file.txt"), b"content").unwrap();

        remove_dir_contents(&base).unwrap();

        assert!(base.exists());
        assert!(fs::read_dir(&base).unwrap().next().is_none());
        let _ = fs::remove_dir_all(base);
    }
}
