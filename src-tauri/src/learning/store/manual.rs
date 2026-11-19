use super::super::{
    validation::sensitive, ChangeStatus, EntryKind, EntryOrigin, EntryValue, InventoryEntry,
};
use super::actions::StoredChange;
use super::{key, parse_rule_with_note, render_rule_sources};
use crate::config::CorrectionConfig;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};

pub(super) struct ManualItem {
    pub item: InventoryEntry,
    pub line: String,
    pub applied_change: Option<String>,
}

pub(super) fn inventory(manual: &CorrectionConfig, changes: &[StoredChange]) -> Vec<ManualItem> {
    let disabled: HashSet<_> = manual
        .disabled_dictionary_terms
        .iter()
        .map(|line| key(line))
        .collect();
    let mut items = Vec::new();
    for line in manual
        .dictionary_text
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        items.push(manual_item(
            EntryKind::Dictionary,
            line,
            line.trim().to_owned(),
            None,
            !disabled.contains(&key(line)),
            true,
        ));
    }
    for line in manual
        .correction_rules_text
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
    {
        match parse_rule_with_note(line) {
            Some((sources, term, _)) => {
                items.extend(sources.into_iter().map(|source| {
                    manual_item(
                        EntryKind::Rule,
                        line,
                        term.clone(),
                        Some(source),
                        true,
                        true,
                    )
                }));
            }
            None => items.push(manual_item(
                EntryKind::Rule,
                line,
                line.trim().to_owned(),
                None,
                true,
                false,
            )),
        }
    }
    let mut counts = HashMap::new();
    for entry in &items {
        let identity = (
            entry.item.kind == EntryKind::Rule,
            identity(&entry.item.value),
        );
        *counts.entry(identity).or_insert(0) += 1;
    }
    for entry in &mut items {
        let identity = (
            entry.item.kind == EntryKind::Rule,
            identity(&entry.item.value),
        );
        entry.item.editable &= counts.get(&identity) == Some(&1) && entry.line.len() <= 1_024;
        if !entry.item.editable {
            continue;
        }
        for change in changes.iter().filter(|change| {
            change.public.status == ChangeStatus::Applied
                && change.public.kind == entry.item.kind
                && change.manual_line.as_ref() == Some(&entry.line)
        }) {
            if change.public.before.as_ref() == Some(&entry.item.value) {
                if let Some(after) = &change.public.after {
                    entry.item.value = after.clone();
                    entry.applied_change = Some(change.public.id.clone());
                }
            }
        }
    }
    items
}

/// Ambiguous and opaque lines may be part of the model's context. Never send
/// credentials or unusually large freeform lines that can be kept local.
pub(super) fn exportable(item: &ManualItem) -> bool {
    let value = &item.item.value;
    value.term.len() <= 320
        && !sensitive(&value.term)
        && !value
            .from
            .as_ref()
            .is_some_and(|source| source.len() > 320 || sensitive(source))
}

fn identity(value: &EntryValue) -> String {
    key(value.from.as_deref().unwrap_or(&value.term))
}

fn manual_item(
    kind: EntryKind,
    line: &str,
    term: String,
    from: Option<String>,
    enabled: bool,
    editable: bool,
) -> ManualItem {
    // The model only receives the fingerprint. Local application also verifies
    // the exact saved line and expected entry value before patching it.
    let mut hasher = DefaultHasher::new();
    (kind == EntryKind::Rule, line, &from).hash(&mut hasher);
    ManualItem {
        item: InventoryEntry {
            id: format!("manual-{:016x}", hasher.finish()),
            kind,
            origin: EntryOrigin::Manual,
            value: EntryValue {
                term,
                from,
                enabled,
            },
            editable,
        },
        line: line.to_owned(),
        applied_change: None,
    }
}

pub(super) fn effective_config(
    manual: &CorrectionConfig,
    changes: &[StoredChange],
) -> (CorrectionConfig, HashSet<String>) {
    let mut effective = manual.clone();
    let mut disabled_sources = HashSet::new();
    if !changes
        .iter()
        .any(|change| change.manual_line.is_some() && change.public.status == ChangeStatus::Applied)
    {
        return (effective, disabled_sources);
    }
    let items = inventory(manual, changes);
    let mut dictionary = HashMap::new();
    let mut touched_rules = HashSet::new();
    for entry in items.iter().filter(|entry| entry.applied_change.is_some()) {
        let value = &entry.item.value;
        if entry.item.kind == EntryKind::Dictionary {
            // Keep disabled spellings in the configuration for conflict checks
            // and let the existing prompt builder omit them.
            if !value.enabled {
                effective.disabled_dictionary_terms.push(value.term.clone());
            }
            dictionary.insert(entry.line.as_str(), vec![value.term.clone()]);
        } else {
            touched_rules.insert(entry.line.as_str());
            if !value.enabled {
                if let Some(source) = &value.from {
                    disabled_sources.insert(key(source));
                }
            }
        }
    }
    let rules = rules_with_overrides(&items, &touched_rules);
    effective.dictionary_text = replace_lines(&manual.dictionary_text, &dictionary);
    effective.correction_rules_text = replace_lines(&manual.correction_rules_text, &rules);
    (effective, disabled_sources)
}

fn rules_with_overrides<'a>(
    items: &'a [ManualItem],
    touched: &HashSet<&'a str>,
) -> HashMap<&'a str, Vec<String>> {
    let mut lines: HashMap<&str, Vec<(&str, Vec<&str>)>> = HashMap::new();
    for item in items
        .iter()
        .filter(|item| item.item.kind == EntryKind::Rule && touched.contains(item.line.as_str()))
    {
        let groups = lines.entry(item.line.as_str()).or_default();
        if let Some(source) = item
            .item
            .value
            .from
            .as_deref()
            .filter(|_| item.item.value.enabled)
        {
            let term = item.item.value.term.as_str();
            if let Some((_, sources)) = groups.iter_mut().find(|(target, _)| *target == term) {
                sources.push(source);
            } else {
                groups.push((term, vec![source]));
            }
        }
    }
    lines
        .into_iter()
        .filter_map(|(line, groups)| {
            let (_, _, note) = parse_rule_with_note(line)?;
            let lines = groups
                .into_iter()
                .map(|(target, sources)| {
                    let rule = render_rule_sources(sources, target);
                    if note.is_empty() {
                        rule
                    } else {
                        format!("{rule} {note}")
                    }
                })
                .collect();
            Some((line, lines))
        })
        .collect()
}

fn replace_lines(text: &str, replacements: &HashMap<&str, Vec<String>>) -> String {
    if replacements.is_empty() {
        return text.to_owned();
    }
    let mut result = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let trimmed = line
            .strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .unwrap_or(line);
        match replacements.get(trimmed) {
            Some(replacements) if !replacements.is_empty() => {
                let ending = &line[trimmed.len()..];
                result.push_str(&replacements.join(if ending.is_empty() { "\n" } else { ending }));
                result.push_str(ending);
            }
            Some(_) => {}
            None => result.push_str(line),
        }
    }
    result
}
