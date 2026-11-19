mod actions;
mod manual;

use super::validation::{key, valid_term};
use super::{EntryKind, LearnedEntry};
use crate::config::CorrectionConfig;
use anyhow::{bail, Context, Result};
#[cfg(test)]
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

pub(super) use actions::{commit_analysis, snapshot};
pub use actions::{list_changes, resolve_change};

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
    #[serde(default, skip_serializing_if = "is_zero")]
    user_generation: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Serialize, Deserialize)]
struct Database {
    version: u32,
    entries: Vec<StoredEntry>,
    #[serde(default)]
    changes: Vec<actions::StoredChange>,
    #[serde(default)]
    evidence: Vec<actions::Evidence>,
}

impl Default for Database {
    fn default() -> Self {
        Self {
            version: 2,
            entries: Vec::new(),
            changes: Vec::new(),
            evidence: Vec::new(),
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
        database.clear_evidence_for(id);
        save(&path, &database)?;
        Ok(database.visible())
    })
}

pub fn set_entry_enabled(id: &str, enabled: bool) -> Result<Vec<LearnedEntry>> {
    with_store_lock(|| {
        let path = path()?;
        let mut database = load(&path)?;
        if database.set_enabled(id, enabled)? {
            save(&path, &database)?;
        }
        Ok(database.visible())
    })
}

impl Database {
    fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<bool> {
        let item = self
            .entries
            .iter_mut()
            .find(|item| item.entry.id == id && !item.deleted)
            .context("Learned entry no longer exists")?;
        if item.entry.enabled == enabled {
            return Ok(false);
        }
        item.entry.enabled = enabled;
        item.user_generation = item.user_generation.saturating_add(1);
        self.clear_evidence_for(id);
        Ok(true)
    }

    fn visible(&self) -> Vec<LearnedEntry> {
        self.entries
            .iter()
            .rev()
            .filter(|item| !item.deleted)
            .map(|item| item.entry.clone())
            .collect()
    }

    #[cfg(test)]
    fn add(
        &mut self,
        candidates: &[tests::LegacyCandidate],
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
                    kind: EntryKind::Legacy,
                    term: candidate.term.clone(),
                    from: candidate.from.clone(),
                    created_at: Utc::now().to_rfc3339(),
                    record_id: record_id.to_owned(),
                    enabled: true,
                },
                deleted: false,
                user_generation: 0,
            });
            active += 1;
            added += 1;
        }
        added
    }
}

pub fn correction_for_text(manual: &CorrectionConfig, raw_text: &str) -> Result<CorrectionConfig> {
    with_store_lock(|| {
        let database = load(&path()?)?;
        let (effective, disabled_sources) = manual::effective_config(manual, &database.changes);
        let mut entries = database.visible();
        entries.retain(|entry| {
            !entry
                .from
                .as_deref()
                .is_some_and(|from| disabled_sources.contains(&key(from)))
        });
        Ok(augment_correction_for_text(&effective, &entries, raw_text))
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
    let mut database: Database =
        serde_json::from_slice(&bytes).context("Cannot parse learned corrections")?;
    if !matches!(database.version, 1 | 2) || database.entries.len() > MAX_ENTRY_SLOTS {
        bail!("Unsupported learned correction store");
    }
    database.version = 2;
    let mut ids = HashSet::new();
    if database.entries.iter().any(|item| {
        Uuid::parse_str(&item.entry.id).is_err()
            || !ids.insert(&item.entry.id)
            || !valid_term(&item.entry.term)
            || (item.entry.kind == EntryKind::Rule && item.entry.from.is_none())
            || (item.entry.kind == EntryKind::Dictionary && item.entry.from.is_some())
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

/// Give relevant rules the prompt budget first. Dictionary terms also help for
/// spellings that have never occurred in ASR, so use the remaining budget for them.
pub fn augment_correction_for_text(
    manual: &CorrectionConfig,
    entries: &[LearnedEntry],
    raw_text: &str,
) -> CorrectionConfig {
    let input = raw_text.to_lowercase();
    let mut selected = Vec::new();
    let mut used_units = 0;
    let mut rule_count = 0;
    let relevant = |entry: &LearnedEntry| {
        entry
            .from
            .as_ref()
            .is_some_and(|from| input.contains(&key(from)))
            || (entry.kind != EntryKind::Rule && input.contains(&key(&entry.term)))
    };
    let matching_rules = entries
        .iter()
        .filter(|entry| entry.from.is_some() && relevant(entry));
    let matching_terms = entries
        .iter()
        .filter(|entry| entry.from.is_none() && relevant(entry));
    let other_dictionary_terms = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Dictionary && !relevant(entry));
    for entry in matching_rules
        .chain(matching_terms)
        .chain(other_dictionary_terms)
    {
        if !entry.enabled
            || (entry.from.is_some() && rule_count >= MAX_PROMPT_RULES)
            || entry_conflicts_with_manual(manual, entry)
        {
            continue;
        }
        let rule_units = entry.from.as_ref().map_or(0, |from| {
            render_rule(from, &entry.term).encode_utf16().count() + 1
        });
        let dictionary_units = if entry.kind == EntryKind::Rule {
            0
        } else {
            entry.term.encode_utf16().count() + 1
        };
        let units = rule_units + dictionary_units;
        if used_units + units > MAX_PROMPT_UNITS {
            continue;
        }
        selected.push(entry);
        used_units += units;
        rule_count += usize::from(entry.from.is_some());
    }
    append_entries(manual, selected)
}

#[cfg(test)]
fn augment_correction(manual: &CorrectionConfig, entries: &[LearnedEntry]) -> CorrectionConfig {
    append_entries(manual, entries.iter())
}

fn append_entries<'a>(
    manual: &CorrectionConfig,
    entries: impl IntoIterator<Item = &'a LearnedEntry>,
) -> CorrectionConfig {
    let mut combined = manual.clone();
    let mut terms: HashSet<String> = manual.dictionary_text.lines().map(key).collect();
    let mut sources = HashSet::new();
    for entry in entries {
        if !entry.enabled
            || !valid_term(&entry.term)
            || entry.from.as_ref().is_some_and(|from| !valid_term(from))
            || entry_conflicts_with_manual(manual, entry)
        {
            continue;
        }
        if let Some(from) = &entry.from {
            if !sources.insert(key(from)) {
                continue;
            }
            append_line(
                &mut combined.correction_rules_text,
                &render_rule(from, &entry.term),
            );
        }
        if entry.kind != EntryKind::Rule && terms.insert(key(&entry.term)) {
            append_line(&mut combined.dictionary_text, &entry.term);
        }
    }
    combined
}

fn render_rule(from: &str, term: &str) -> String {
    render_rule_sources([from], term)
}

fn render_rule_sources<'a>(sources: impl IntoIterator<Item = &'a str>, term: &str) -> String {
    let sources = sources
        .into_iter()
        .map(|source| serde_json::to_string(source).expect("strings are serializable"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{sources} -> {}",
        serde_json::to_string(term).expect("strings are serializable")
    )
}

fn entry_conflicts_with_manual(manual: &CorrectionConfig, entry: &LearnedEntry) -> bool {
    match entry.kind {
        EntryKind::Legacy => manual_conflict(manual, &entry.term, entry.from.as_deref()),
        EntryKind::Dictionary => dictionary_has(manual, &entry.term),
        EntryKind::Rule => entry
            .from
            .as_ref()
            .is_some_and(|from| manual_conflict_for_rule(manual, from, &entry.term)),
    }
}

fn dictionary_has(manual: &CorrectionConfig, term: &str) -> bool {
    let term = key(term);
    manual
        .dictionary_text
        .lines()
        .map(key)
        .any(|line| line == term)
        || manual
            .disabled_dictionary_terms
            .iter()
            .map(|line| key(line))
            .any(|line| line == term)
}

fn manual_conflict_for_rule(manual: &CorrectionConfig, from: &str, term: &str) -> bool {
    let source_key = key(from);
    let term_key = key(term);
    dictionary_has(manual, from)
        || manual
            .correction_rules_text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .any(|line| match parse_rule(line) {
                Some((sources, target)) => {
                    sources
                        .iter()
                        .map(|source| key(source))
                        .any(|source| source == source_key || source == term_key)
                        || key(&target) == source_key
                }
                None => key(line).contains(&source_key),
            })
}

fn append_line(text: &mut String, line: &str) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
}

fn manual_conflict(manual: &CorrectionConfig, term: &str, from: Option<&str>) -> bool {
    let term_key = key(term);
    let from_key = from.map(key);
    if manual
        .disabled_dictionary_terms
        .iter()
        .any(|item| key(item) == term_key)
    {
        return true;
    }
    let dictionary: HashSet<String> = manual.dictionary_text.lines().map(key).collect();
    if from_key.is_none() && dictionary.contains(&term_key) {
        return true;
    }
    if from_key
        .as_ref()
        .is_some_and(|from| dictionary.contains(from))
    {
        return true;
    }
    manual
        .correction_rules_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .any(|line| {
            let Some((sources, target)) = parse_rule(line) else {
                // Unknown syntax still reaches the correction model. Only
                // suppress a learned rule that overlaps that manual line.
                let line = key(line);
                return line.contains(&term_key)
                    || from_key.as_ref().is_some_and(|from| line.contains(from));
            };
            sources
                .iter()
                .map(|source| key(source))
                .any(|source| source == term_key || from_key.as_ref() == Some(&source))
                || from_key.as_ref() == Some(&key(&target))
        })
}

const RULE_QUOTES: [char; 5] = ['"', '\'', '`', '“', '‘'];

pub(crate) fn parse_rule(line: &str) -> Option<(Vec<String>, String)> {
    let (sources, target, _) = parse_rule_with_note(line)?;
    Some((sources, target))
}

fn parse_rule_with_note(line: &str) -> Option<(Vec<String>, String, &str)> {
    let line = line.trim();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut arrow = None;
    let mut end = line.len();
    let mut sources = Vec::new();
    let mut source_start = 0;
    let quoted_source = line.starts_with(RULE_QUOTES);
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
        } else if arrow.is_none() && quoted_source && matches!(ch, ',' | '，' | '、') {
            sources.push(unquote(line[source_start..index].trim())?);
            source_start = index + ch.len_utf8();
        } else if ch == '（' && legacy_note(line, arrow, index) {
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
    sources.push(unquote(line[source_start..index].trim())?);
    Some((
        sources,
        unquote(line[index + width..end].trim())?,
        &line[end..],
    ))
}

fn legacy_note(line: &str, arrow: Option<(usize, usize)>, index: usize) -> bool {
    let Some((start, width)) = arrow else {
        return false;
    };
    if line[start + width..index].trim().is_empty() {
        return false;
    }
    let mut depth = 0;
    for (offset, ch) in line[index..].char_indices() {
        match ch {
            '（' => depth += 1,
            '）' => {
                depth -= 1;
                if depth == 0 {
                    let suffix = line[index + offset + ch.len_utf8()..].trim();
                    return suffix.is_empty() || suffix.starts_with('#');
                }
            }
            _ => {}
        }
    }
    false
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
        if let Some(contents) = value
            .strip_prefix(open)
            .and_then(|value| value.strip_suffix(close))
        {
            return (!contents.is_empty()).then(|| contents.to_owned());
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
    pub(super) struct LegacyCandidate {
        pub term: String,
        pub from: Option<String>,
    }

    fn candidate() -> LegacyCandidate {
        LegacyCandidate {
            term: "BoltScribe".into(),
            from: Some("波特思考".into()),
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
            Some((vec!["A -> B".into()], "C#D".into()))
        );
        assert_eq!(
            parse_rule("艾迪 → ID"),
            Some((vec!["艾迪".into()], "ID".into()))
        );
        assert!(parse_rule("A -> B -> C").is_none());
        assert!(parse_rule("\"broken -> C").is_none());
    }

    #[test]
    fn legacy_aliases_and_parenthetical_notes_preserve_manual_precedence() {
        assert_eq!(
            parse_rule(r#""Comma, Name","Arrow -> Name"、"Preferred" -> "Project"（保留 "说明"）"#),
            Some((
                vec![
                    "Comma, Name".into(),
                    "Arrow -> Name".into(),
                    "Preferred".into()
                ],
                "Project".into(),
            ))
        );

        let mut database = Database::default();
        let mut manual = manual();
        manual.correction_rules_text =
            "\"旧名称\",\"波特思考\" -> \"ManualName\"\n\"读法\" -> \"Other Name\"（说明）".into();
        assert_eq!(database.add(&[candidate()], &manual, "test"), 0);
        assert!(manual_conflict(&manual, "NewTerm", Some("Other Name")));
        manual.correction_rules_text = "读法 -> ManualName（旧写法备注）".into();
        assert!(manual_conflict(&manual, "NewTerm", Some("ManualName")));
        assert!(!manual_conflict(&manual, "Unrelated", Some("Different")));
    }

    #[test]
    fn unparseable_manual_lines_do_not_block_unrelated_learning() {
        let mut database = Database::default();
        let mut manual = manual();
        manual.correction_rules_text = "格式未知的说明\n\"另一个读法\" -> \"Existing\"".into();
        assert_eq!(database.add(&[candidate()], &manual, "test"), 1);

        let combined =
            augment_correction_for_text(&manual, &database.visible(), "使用波特思考写文档");
        assert_eq!(
            combined.correction_rules_text,
            format!(
                "{}\n\"波特思考\" -> \"BoltScribe\"",
                manual.correction_rules_text
            )
        );
        assert!(manual_conflict(&manual, "OtherTerm", Some("另一个读法")));
        manual.correction_rules_text = "源词 -> 中间词 -> 最终词".into();
        assert!(manual_conflict(&manual, "NewTerm", Some("源词")));
        assert!(!manual_conflict(&manual, "Unrelated", Some("Different")));
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
            .map(|index| LegacyCandidate {
                term: format!("{}T{index:03}", "长".repeat(70)),
                from: Some(format!("{}F{index:03}", "错".repeat(70))),
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

    #[test]
    fn old_store_entries_load_with_legacy_projection() {
        let id = Uuid::new_v4();
        let input = serde_json::json!({"version":1,"entries":[{
            "id":id,"term":"BoltScribe","from":"波特思考","enabled":true,
            "created_at":"2026-11-19T12:00:00Z","record_id":"old"
        }]});
        let old: Database = serde_json::from_value(input).unwrap();
        assert_eq!(old.entries[0].entry.kind, EntryKind::Legacy);
        assert!(old.changes.is_empty() && old.evidence.is_empty());
        let prompt = augment_correction_for_text(&manual(), &old.visible(), "波特思考");
        assert!(prompt.dictionary_text.contains("BoltScribe"));
        assert!(prompt.correction_rules_text.contains("波特思考"));
    }
}
