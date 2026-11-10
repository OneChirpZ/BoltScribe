use super::model::HistoryRecord;
use crate::paths;
use anyhow::{Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const HISTORY_READ_CHUNK_SIZE: u64 = 16 * 1024;

struct HistoryPage {
    records: Vec<HistoryRecord>,
    visible_count: usize,
}

pub(super) fn load_from_paths(
    paths: &[PathBuf],
    limit: usize,
    offset: usize,
) -> Result<Vec<HistoryRecord>> {
    let mut records = Vec::new();
    let mut remaining_offset = offset;

    for path in paths {
        if records.len() >= limit {
            break;
        }

        let page = load_page_from_path(path, limit - records.len(), remaining_offset)?;
        remaining_offset = remaining_offset.saturating_sub(page.visible_count);
        records.extend(page.records);
    }

    Ok(records)
}

#[cfg(test)]
pub(super) fn load_from_path(
    path: &PathBuf,
    limit: usize,
    offset: usize,
) -> Result<Vec<HistoryRecord>> {
    Ok(load_page_from_path(path, limit, offset)?.records)
}

fn load_page_from_path(path: &PathBuf, limit: usize, offset: usize) -> Result<HistoryPage> {
    if limit == 0 {
        return Ok(HistoryPage {
            records: Vec::new(),
            visible_count: 0,
        });
    }
    if !path.exists() {
        return Ok(HistoryPage {
            records: Vec::new(),
            visible_count: 0,
        });
    }

    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut position = file.metadata()?.len();
    let mut carry = Vec::new();
    let mut skipped = 0usize;
    let mut records = Vec::new();
    let mut visible_count = 0usize;

    while position > 0 && records.len() < limit {
        let read_size = HISTORY_READ_CHUNK_SIZE.min(position);
        position -= read_size;
        file.seek(SeekFrom::Start(position))?;

        let mut buffer = vec![0; read_size as usize];
        file.read_exact(&mut buffer)?;
        buffer.extend_from_slice(&carry);

        let mut end = buffer.len();
        while records.len() < limit {
            let Some(newline_index) = buffer[..end].iter().rposition(|byte| *byte == b'\n') else {
                break;
            };
            collect_history_line(
                &buffer[newline_index + 1..end],
                offset,
                &mut skipped,
                &mut visible_count,
                &mut records,
            );
            end = newline_index;
        }

        carry = buffer[..end].to_vec();
    }

    if records.len() < limit && !carry.is_empty() {
        collect_history_line(
            &carry,
            offset,
            &mut skipped,
            &mut visible_count,
            &mut records,
        );
    }

    Ok(HistoryPage {
        records,
        visible_count,
    })
}

fn collect_history_line(
    line: &[u8],
    offset: usize,
    skipped: &mut usize,
    visible_count: &mut usize,
    records: &mut Vec<HistoryRecord>,
) {
    if line.iter().all(|byte| byte.is_ascii_whitespace()) {
        return;
    }

    let Ok(record) = serde_json::from_slice::<HistoryRecord>(line) else {
        return;
    };
    if should_hide_history_record(&record) {
        return;
    }
    *visible_count += 1;
    if *skipped < offset {
        *skipped += 1;
        return;
    }

    records.push(record);
}

pub(super) fn should_hide_history_record(record: &HistoryRecord) -> bool {
    record
        .workflow_error
        .as_deref()
        .is_some_and(super::is_empty_asr_text_error)
}

pub(super) fn is_empty_asr_text_error_impl(message: &str) -> bool {
    message.contains("Volcengine ASR response did not contain text")
        || message.contains("Volcengine live ASR response did not contain text")
        || message.contains("ASR returned empty text")
}

pub(super) fn read_records_in_file_order(path: &Path) -> Result<Vec<HistoryRecord>> {
    let file =
        fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<HistoryRecord>(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

pub(super) fn history_read_paths() -> Result<Vec<PathBuf>> {
    let path = paths::history_path()?;
    let legacy_path = paths::legacy_history_path()?;
    let mut paths = Vec::new();

    if path.exists() {
        paths.push(path.clone());
    }
    if legacy_path.exists() && legacy_path != path {
        paths.push(legacy_path);
    }
    if paths.is_empty() {
        paths.push(path);
    }

    Ok(paths)
}

/// Resolve a source record by identifier without loading all history into memory.
pub(super) fn find_from_paths(paths: &[PathBuf], record_id: &str) -> Result<Option<HistoryRecord>> {
    for path in paths {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("Cannot open source history"),
        };
        let mut found = None;
        for line in BufReader::new(file).lines() {
            let line = line?;
            if let Ok(record) = serde_json::from_str::<HistoryRecord>(&line) {
                if record.id == record_id && !should_hide_history_record(&record) {
                    found = Some(record);
                }
            }
        }
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}
