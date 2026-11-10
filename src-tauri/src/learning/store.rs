use super::validation::{key, valid_term, Candidate};
use super::LearnedEntry;
use crate::config::CorrectionConfig;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

static STORE_LOCK: Mutex<()> = Mutex::new(());
const MAX_STORE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ACTIVE_ENTRIES: usize = 512;
const MAX_ENTRY_SLOTS: usize = 4_096;
const MAX_PROMPT_UNITS: usize = 4_096;
const MAX_PROMPT_RULES: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    #[serde(flatten)]
    entry: LearnedEntry,
    #[serde(default)]
    deleted: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Database {
    version: u32,
    entries: Vec<StoredEntry>,
}

impl Default for Database {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

pub fn with_store_lock<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Learning store is unavailable"))?;
    operation()
}

pub(super) fn path() -> Result<PathBuf> {
    Ok(crate::paths::app_dir()
        .map_err(anyhow::Error::msg)?
        .join("learned-corrections.json"))
}

pub fn list_entries() -> Result<Vec<LearnedEntry>> {
    with_store_lock(|| Ok(load(&path()?)?.visible()))
}

pub fn delete_entry(id: &str) -> Result<Vec<LearnedEntry>> {
    with_store_lock(|| {
        let path = path()?;
        let mut database = load(&path)?;
        let item = database
            .entries
            .iter_mut()
            .find(|item| item.entry.id == id && !item.deleted)
            .ok_or_else(|| anyhow::anyhow!("Learned entry no longer exists"))?;
        // Retain a recoverable tombstone so delayed or repeated learning cannot re-add it.
        item.deleted = true;
        item.entry.enabled = false;
        save(&path, &database)?;
        Ok(database.visible())
    })
}

pub fn set_entry_enabled(id: &str, enabled: bool) -> Result<Vec<LearnedEntry>> {
    with_store_lock(|| {
        let path = path()?;
        let mut database = load(&path)?;
        let item = database
            .entries
            .iter_mut()
            .find(|item| item.entry.id == id && !item.deleted)
            .ok_or_else(|| anyhow::anyhow!("Learned entry no longer exists"))?;
        item.entry.enabled = enabled;
        save(&path, &database)?;
        Ok(database.visible())
    })
}

impl Database {
    fn visible(&self) -> Vec<LearnedEntry> {
        self.entries
            .iter()
            .rev()
            .filter(|item| !item.deleted)
            .map(|item| item.entry.clone())
            .collect()
    }

    fn add(
        &mut self,
        candidates: &[Candidate],
        manual: &CorrectionConfig,
        record_id: &str,
    ) -> usize {
        let mut added = 0;
        let mut active = self.entries.iter().filter(|item| !item.deleted).count();
        for candidate in candidates {
            if active >= MAX_ACTIVE_ENTRIES || self.entries.len() >= MAX_ENTRY_SLOTS {
                break;
            }
            // Disabled and deleted entries participate too: learning never reenables them.
            if manual_conflict(manual, &candidate.term, candidate.from.as_deref())
                || self.entries.iter().any(|item| {
                    key(&item.entry.term) == key(&candidate.term)
                        || candidate
                            .from
                            .as_ref()
                            .is_some_and(|from| key(from) == key(&item.entry.term))
                        || item
                            .entry
                            .from
                            .as_ref()
                            .is_some_and(|from| key(from) == key(&candidate.term))
                        || match (&item.entry.from, &candidate.from) {
                            (Some(a), Some(b)) => key(a) == key(b),
                            _ => false,
                        }
                })
            {
                continue;
            }
            self.entries.push(StoredEntry {
                entry: LearnedEntry {
                    id: Uuid::new_v4().to_string(),
                    term: candidate.term.clone(),
                    from: candidate.from.clone(),
                    created_at: Utc::now().to_rfc3339(),
                    record_id: record_id.to_owned(),
                    enabled: true,
                },
                deleted: false,
            });
            active += 1;
            added += 1;
        }
        added
    }
}

pub(super) fn add_candidates(
    expected_path: &Path,
    candidates: &[Candidate],
    manual: &CorrectionConfig,
    record_id: &str,
) -> Result<Vec<LearnedEntry>> {
    with_store_lock(|| {
        let current_path = path()?;
        if current_path != expected_path {
            bail!("Learning result discarded after data directory changed");
        }
        let mut database = load(&current_path)?;
        let previous_count = database.entries.len();
        let added = database.add(candidates, manual, record_id);
        if added > 0 {
            save(&current_path, &database)?;
        }
        Ok(database.entries[previous_count..]
            .iter()
            .map(|item| item.entry.clone())
            .collect())
    })
}

fn load(path: &Path) -> Result<Database> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Database::default()),
        Err(err) => return Err(err).context("Cannot open learned corrections"),
    };
    let mut bytes = Vec::new();
    file.take(MAX_STORE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_STORE_BYTES {
        bail!("Learned correction store exceeded its size limit");
    }
    let database: Database =
        serde_json::from_slice(&bytes).context("Cannot parse learned corrections")?;
    if database.version != 1 || database.entries.len() > MAX_ENTRY_SLOTS {
        bail!("Unsupported learned correction store");
    }
    let mut ids = HashSet::new();
    if database.entries.iter().any(|item| {
        Uuid::parse_str(&item.entry.id).is_err()
            || !ids.insert(&item.entry.id)
            || !valid_term(&item.entry.term)
            || item
                .entry
                .from
                .as_ref()
                .is_some_and(|from| !valid_term(from))
            || item.entry.record_id.len() > 128
            || item.entry.created_at.len() > 64
    }) {
        bail!("Invalid learned correction entry");
    }
    Ok(database)
}

fn save(path: &Path, database: &Database) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(database)?;
    if bytes.len() > MAX_STORE_BYTES {
        bail!("Learned correction store exceeded its size limit");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pending = path.with_extension("json.pending");
    let mut file = File::create(&pending)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    replace(&pending, path)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn replace(pending: &Path, path: &Path) -> Result<()> {
    fs::rename(pending, path).context("Cannot save learned corrections")
}

#[cfg(target_os = "windows")]
pub(super) fn replace(pending: &Path, path: &Path) -> Result<()> {
    if !path.exists() {
        return fs::rename(pending, path).context("Cannot save learned corrections");
    }
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS};
    let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let source: Vec<u16> = pending.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        ReplaceFileW(
            PCWSTR(target.as_ptr()),
            PCWSTR(source.as_ptr()),
            PCWSTR::null(),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    }
    .context("Cannot save learned corrections")
}

/// Learning never expands the live prompt with unrelated entries. Case-insensitive
/// target matches also preserve names whose spoken spelling only needs capitalization.
pub fn augment_correction_for_text(
    manual: &CorrectionConfig,
    entries: &[LearnedEntry],
    raw_text: &str,
) -> CorrectionConfig {
    let input = raw_text.to_lowercase();
    let mut selected = Vec::new();
    let mut used_units = 0;
    for entry in entries {
        if selected.len() >= MAX_PROMPT_RULES {
            break;
        }
        if !entry.enabled {
            continue;
        }
        let relevant = input.contains(&key(&entry.term))
            || entry
                .from
                .as_ref()
                .is_some_and(|from| input.contains(&key(from)));
        if !relevant {
            continue;
        }
        if manual_conflict(manual, &entry.term, entry.from.as_deref()) {
            continue;
        }
        let rule_units = entry
            .from
            .as_ref()
            .map(|from| {
                serde_json::to_string(from).unwrap().encode_utf16().count()
                    + serde_json::to_string(&entry.term)
                        .unwrap()
                        .encode_utf16()
                        .count()
                    + 5
            })
            .unwrap_or_default();
        let units = entry.term.encode_utf16().count() + 1 + rule_units;
        if used_units + units > MAX_PROMPT_UNITS {
            continue;
        }
        selected.push(entry.clone());
        used_units += units;
    }
    augment_correction(manual, &selected)
}

fn augment_correction(manual: &CorrectionConfig, entries: &[LearnedEntry]) -> CorrectionConfig {
    let mut combined = manual.clone();
    let mut terms: HashSet<String> = manual.dictionary_text.lines().map(key).collect();
    let mut sources = HashSet::new();
    for entry in entries {
        if !entry.enabled
            || !valid_term(&entry.term)
            || entry.from.as_ref().is_some_and(|from| !valid_term(from))
            || manual_conflict(manual, &entry.term, entry.from.as_deref())
        {
            continue;
        }
        if let Some(from) = &entry.from {
            if !sources.insert(key(from)) {
                continue;
            }
            let rule = format!(
                "{} -> {}",
                serde_json::to_string(from).unwrap(),
                serde_json::to_string(&entry.term).unwrap()
            );
            append_line(&mut combined.correction_rules_text, &rule);
        }
        if terms.insert(key(&entry.term)) {
            append_line(&mut combined.dictionary_text, &entry.term);
        }
    }
    combined
}

fn append_line(text: &mut String, line: &str) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
}

fn manual_conflict(manual: &CorrectionConfig, term: &str, from: Option<&str>) -> bool {
    let term_key = key(term);
    if manual
        .disabled_dictionary_terms
        .iter()
        .any(|item| key(item) == term_key)
    {
        return true;
    }
    let dictionary: HashSet<String> = manual.dictionary_text.lines().map(key).collect();
    if from.is_none() && dictionary.contains(&term_key) {
        return true;
    }
    if from.is_some_and(|from| dictionary.contains(&key(from))) {
        return true;
    }
    for line in manual
        .correction_rules_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let Some((source, target)) = parse_rule(line) else {
            // Preserve unparsed user syntax; do not layer a learned replacement over it.
            return true;
        };
        if key(&source) == term_key
            || from.is_some_and(|from| key(from) == key(&source) || key(from) == key(&target))
        {
            return true;
        }
    }
    false
}

fn parse_rule(line: &str) -> Option<(String, String)> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut arrow = None;
    let mut end = line.len();
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(closer) = quote {
            if ch == '\\' {
                escaped = true;
            } else if ch == closer {
                quote = None;
            }
            continue;
        }
        if let Some(closer) = match ch {
            '"' | '\'' | '`' => Some(ch),
            '“' => Some('”'),
            '‘' => Some('’'),
            _ => None,
        } {
            quote = Some(closer);
        } else if ch == '#' {
            end = index;
            break;
        } else if line[index..].starts_with("->") || line[index..].starts_with("=>") || ch == '→'
        {
            if arrow.is_some() {
                return None;
            }
            arrow = Some((index, if ch == '→' { ch.len_utf8() } else { 2 }));
        }
    }
    if quote.is_some() {
        return None;
    }
    let (index, width) = arrow?;
    Some((
        unquote(line[..index].trim())?,
        unquote(line[index + width..end].trim())?,
    ))
}

fn unquote(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value)
            .ok()
            .filter(|value| !value.is_empty());
    }
    for (open, close) in [('‘', '’'), ('“', '”'), ('\'', '\''), ('`', '`')] {
        if value.starts_with(open) && value.ends_with(close) {
            return Some(value[open.len_utf8()..value.len() - close.len_utf8()].to_string());
        }
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual() -> CorrectionConfig {
        crate::config::AppConfig::default().correction
    }
    fn candidate() -> Candidate {
        Candidate {
            term: "BoltScribe".into(),
            from: Some("波特思考".into()),
            term_common: false,
            from_common: false,
        }
    }

    #[test]
    fn learned_terms_are_separate_and_manual_overrides_win() {
        let mut database = Database::default();
        let mut manual = manual();
        assert_eq!(database.add(&[candidate()], &manual, "test"), 1);
        let original = manual.clone();
        let combined = augment_correction(&manual, &database.visible());
        assert_eq!(manual, original);
        assert!(combined.dictionary_text.contains("BoltScribe"));
        assert!(combined.correction_rules_text.contains("波特思考"));
        manual.correction_rules_text = "\"波特思考\" -> \"OtherTerm\" # manual".into();
        assert_eq!(augment_correction(&manual, &database.visible()), manual);
    }

    #[test]
    fn deletion_and_disable_are_not_undone_by_delayed_result() {
        let mut database = Database::default();
        database.add(&[candidate()], &manual(), "test");
        database.entries[0].entry.enabled = false;
        assert_eq!(database.add(&[candidate()], &manual(), "late"), 0);
        assert_eq!(augment_correction(&manual(), &database.visible()), manual());
        database.entries[0].deleted = true;
        assert!(database.visible().is_empty());
        assert_eq!(database.add(&[candidate()], &manual(), "late"), 0);
    }

    #[test]
    fn rejects_conflicting_sources_and_disabled_manual_terms() {
        let mut database = Database::default();
        database.add(&[candidate()], &manual(), "test");
        let mut other = candidate();
        other.term = "OtherTerm".into();
        assert_eq!(database.add(&[other], &manual(), "test"), 0);
        let mut manual = manual();
        manual.disabled_dictionary_terms.push("boltscribe".into());
        assert_eq!(augment_correction(&manual, &database.visible()), manual);
    }

    #[test]
    fn rejects_reverse_or_chained_learned_rules() {
        let mut database = Database::default();
        database.add(&[candidate()], &manual(), "test");
        let mut reverse = candidate();
        reverse.term = "波特思考".into();
        reverse.from = Some("BoltScribe".into());
        assert_eq!(database.add(&[reverse], &manual(), "test"), 0);
        let mut chained = candidate();
        chained.term = "AnotherTerm".into();
        chained.from = Some("BoltScribe".into());
        assert_eq!(database.add(&[chained], &manual(), "test"), 0);
    }

    #[test]
    fn understands_quoted_manual_rules_without_splitting_embedded_arrows() {
        assert_eq!(
            parse_rule("\"A -> B\" => \"C#D\" # note"),
            Some(("A -> B".into(), "C#D".into()))
        );
        assert_eq!(parse_rule("艾迪 → ID"), Some(("艾迪".into(), "ID".into())));
        assert!(parse_rule("A -> B -> C").is_none());
        assert!(parse_rule("\"broken -> C").is_none());
    }

    #[test]
    fn comments_do_not_block_learning_but_manual_rule_targets_do() {
        let mut manual = manual();
        manual.correction_rules_text = "# My explicit rules\n".into();
        assert!(!manual_conflict(&manual, "BoltScribe", Some("波特思考")));
        manual
            .correction_rules_text
            .push_str("OtherTerm -> 波特思考");
        assert!(manual_conflict(&manual, "BoltScribe", Some("波特思考")));
    }

    #[test]
    fn live_prompt_only_uses_relevant_learned_rules_and_preserves_manual_text() {
        let mut database = Database::default();
        let mut manual = manual();
        manual.dictionary_text = "ManualTerm".into();
        database.add(&[candidate()], &manual, "test");
        assert_eq!(
            augment_correction_for_text(&manual, &database.visible(), "其他语音内容"),
            manual
        );
        let matched =
            augment_correction_for_text(&manual, &database.visible(), "使用波特思考写文档");
        assert!(matched.dictionary_text.starts_with("ManualTerm"));
        assert!(matched.dictionary_text.contains("BoltScribe"));
        let casing = augment_correction_for_text(&manual, &database.visible(), "use boltscribe");
        assert!(casing.dictionary_text.contains("BoltScribe"));
    }

    #[test]
    fn live_prompt_learned_contribution_has_a_utf16_budget() {
        let mut database = Database::default();
        let candidates: Vec<_> = (0..80)
            .map(|index| Candidate {
                term: format!("{}T{index:03}", "长".repeat(70)),
                from: Some(format!("{}F{index:03}", "错".repeat(70))),
                term_common: false,
                from_common: false,
            })
            .collect();
        database.add(&candidates, &manual(), "test");
        let raw = candidates
            .iter()
            .map(|candidate| candidate.from.as_deref().unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        let combined = augment_correction_for_text(&manual(), &database.visible(), &raw);
        let units = combined.dictionary_text.encode_utf16().count()
            + combined.correction_rules_text.encode_utf16().count();
        assert!(units > 0 && units <= MAX_PROMPT_UNITS, "units={units}");
        assert!(combined.correction_rules_text.lines().count() <= MAX_PROMPT_RULES);
        assert!(combined.correction_rules_text.lines().count() < candidates.len());
    }

    #[test]
    fn isolated_store_roundtrip_preserves_deletion_tombstone() {
        // The OS temp directory owns this harmless fixture; no user data/config path is read.
        let directory =
            std::env::temp_dir().join(format!("boltscribe-learning-test-{}", Uuid::new_v4()));
        let path = directory.join("learned-corrections.json");
        let mut database = Database::default();
        assert_eq!(database.add(&[candidate()], &manual(), "fixture"), 1);
        save(&path, &database).unwrap();
        let mut restored = load(&path).unwrap();
        assert_eq!(restored.visible(), database.visible());
        restored.entries[0].deleted = true;
        restored.entries[0].entry.enabled = false;
        save(&path, &restored).unwrap();
        let mut deleted = load(&path).unwrap();
        assert!(deleted.visible().is_empty());
        assert_eq!(deleted.add(&[candidate()], &manual(), "delayed"), 0);
        assert_eq!(deleted.entries[0].entry.term, "BoltScribe");
        assert!(!path.with_extension("json.pending").exists());
    }
}
