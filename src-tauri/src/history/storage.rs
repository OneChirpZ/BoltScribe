use super::model::HistoryRecord;
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub(super) fn merge_json_fields(
    target: &mut serde_json::Value,
    replacement: &serde_json::Value,
) -> Result<()> {
    let target = target
        .as_object_mut()
        .ok_or_else(|| anyhow!("Stored JSON value is not an object"))?;
    let replacement = replacement
        .as_object()
        .ok_or_else(|| anyhow!("Replacement JSON value is not an object"))?;
    for (key, value) in replacement {
        target.insert(key.clone(), value.clone());
    }
    Ok(())
}

pub(super) struct MutableHistoryLine {
    pub(super) raw: String,
    pub(super) value: Option<serde_json::Value>,
    pub(super) record: Option<HistoryRecord>,
    pub(super) removed: bool,
    pub(super) changed: bool,
}

pub(super) struct MutableHistoryFile {
    pub(super) path: PathBuf,
    pub(super) lines: Vec<MutableHistoryLine>,
    pub(super) changed: bool,
}

pub(super) fn load_mutable_history_files(paths: &[PathBuf]) -> Result<Vec<MutableHistoryFile>> {
    paths
        .iter()
        .filter(|path| path.exists())
        .map(|path| {
            let file = fs::File::open(path)
                .with_context(|| format!("Failed to open {}", path.display()))?;
            let mut lines = Vec::new();
            for raw in BufReader::new(file).lines() {
                let raw = raw?;
                let value = serde_json::from_str::<serde_json::Value>(&raw).ok();
                let record = value
                    .as_ref()
                    .and_then(|value| serde_json::from_value(value.clone()).ok());
                lines.push(MutableHistoryLine {
                    raw,
                    value,
                    record,
                    removed: false,
                    changed: false,
                });
            }
            Ok(MutableHistoryFile {
                path: path.clone(),
                lines,
                changed: false,
            })
        })
        .collect()
}

pub(super) fn write_mutable_history_files_atomically(
    history_files: &[MutableHistoryFile],
) -> Result<()> {
    let replacements = history_files
        .iter()
        .filter(|history_file| history_file.changed)
        .map(|history_file| {
            Ok((
                history_file.path.clone(),
                mutable_history_file_contents(history_file)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    write_file_replacements_with_rollback(&replacements, write_file_bytes_atomically)
}

pub(super) fn mutable_history_file_contents(history_file: &MutableHistoryFile) -> Result<Vec<u8>> {
    let mut contents = Vec::new();
    for line in &history_file.lines {
        if line.removed {
            continue;
        }
        if line.changed {
            if let Some(value) = &line.value {
                writeln!(contents, "{}", serde_json::to_string(value)?)?;
            }
        } else {
            writeln!(contents, "{}", line.raw)?;
        }
    }
    Ok(contents)
}

pub(super) fn write_file_replacements_with_rollback<F>(
    replacements: &[(PathBuf, Vec<u8>)],
    mut write_replacement: F,
) -> Result<()>
where
    F: FnMut(&Path, &[u8]) -> Result<()>,
{
    let originals = replacements
        .iter()
        .map(|(path, _)| {
            if path.exists() {
                fs::read(path).map(Some).with_context(|| {
                    format!("Failed to read {} before replacement", path.display())
                })
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>>>()?;

    for (index, (path, contents)) in replacements.iter().enumerate() {
        if let Err(error) = write_replacement(path, contents) {
            let mut rollback_errors = Vec::new();
            for rollback_index in (0..=index).rev() {
                let rollback_path = &replacements[rollback_index].0;
                let rollback_result = match &originals[rollback_index] {
                    Some(original) => write_replacement(rollback_path, original),
                    None if rollback_path.exists() => fs::remove_file(rollback_path)
                        .with_context(|| format!("Failed to remove {}", rollback_path.display())),
                    None => Ok(()),
                };
                if let Err(rollback_error) = rollback_result {
                    rollback_errors
                        .push(format!("{}: {rollback_error:#}", rollback_path.display()));
                }
            }

            if rollback_errors.is_empty() {
                return Err(error);
            }
            bail!(
                "{error:#}; failed to roll back file replacements: {}",
                rollback_errors.join("; ")
            );
        }
    }
    Ok(())
}

pub(super) fn write_file_bytes_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let temp_path = path.with_extension("jsonl.tmp");
    {
        let mut file = fs::File::create(&temp_path)
            .with_context(|| format!("Failed to create {}", temp_path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("Failed to write {}", temp_path.display()))?;
        file.sync_all()?;
    }
    replace_history_file(&temp_path, path)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn replace_history_file(temp_path: &Path, path: &Path) -> Result<()> {
    fs::rename(temp_path, path).with_context(|| format!("Failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn replace_history_file(temp_path: &Path, path: &Path) -> Result<()> {
    let backup_path = path.with_extension("jsonl.replacing");
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .with_context(|| format!("Failed to remove {}", backup_path.display()))?;
    }
    if path.exists() {
        fs::rename(path, &backup_path)
            .with_context(|| format!("Failed to back up {}", path.display()))?;
    }
    if let Err(err) = fs::rename(temp_path, path) {
        if backup_path.exists() {
            let _ = fs::rename(&backup_path, path);
        }
        return Err(err).with_context(|| format!("Failed to replace {}", path.display()));
    }
    if backup_path.exists() {
        if let Err(error) = fs::remove_file(&backup_path) {
            eprintln!(
                "Warning: replaced {} but failed to remove backup {}: {error}",
                path.display(),
                backup_path.display()
            );
        }
    }
    Ok(())
}
