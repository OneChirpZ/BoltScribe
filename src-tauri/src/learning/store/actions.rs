use super::super::{
    ChangeResolution, ChangeStatus, EntryKind, EntryOrigin, EntryValue, InventoryEntry,
    LearnedEntry, LearningChange, LearningOperation, ValidatedAction,
};
use super::manual;
use super::{
    key, load, path, save, valid_term, with_store_lock, Database, StoredEntry, MAX_ACTIVE_ENTRIES,
    MAX_ENTRY_SLOTS,
};
use crate::config::CorrectionConfig;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

const MAX_CHANGES: usize = 1_024;
const MAX_EVIDENCE: usize = 512;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredChange {
    #[serde(flatten)]
    pub public: LearningChange,
    target_id: String,
    pub manual_line: Option<String>,
    #[serde(default, skip_serializing_if = "super::is_zero")]
    user_generation: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Evidence {
    operation: LearningOperation,
    kind: EntryKind,
    target_id: Option<String>,
    before: Option<EntryValue>,
    after: EntryValue,
    record_id: String,
}

#[derive(Default)]
pub(crate) struct AnalysisCommit {
    pub entries: Vec<LearnedEntry>,
    pub changed: usize,
    pub pending: usize,
    pub awaiting_evidence: usize,
}

pub(crate) fn snapshot(
    expected_path: &Path,
    manual: &CorrectionConfig,
) -> Result<Vec<InventoryEntry>> {
    with_store_lock(|| {
        let current = path()?;
        if current != expected_path {
            bail!("Learning data directory changed before analysis");
        }
        Ok(load(&current)?.inventory(manual, true))
    })
}

pub(crate) fn commit_analysis(
    expected_path: &Path,
    manual: &CorrectionConfig,
    record_id: &str,
    activity_id: Option<&str>,
    inventory: &[InventoryEntry],
    actions: &[ValidatedAction],
) -> Result<AnalysisCommit> {
    with_store_lock(|| {
        let current_path = path()?;
        if current_path != expected_path {
            bail!("Learning result discarded after data directory changed");
        }
        let mut database = load(&current_path)?;
        let mut outcome = AnalysisCommit::default();
        for action in actions {
            match database.collect_action(manual, record_id, activity_id, inventory, action) {
                Collected::Skipped => {}
                Collected::AwaitingEvidence => outcome.awaiting_evidence += 1,
                Collected::Pending => {
                    outcome.changed += 1;
                    outcome.pending += 1;
                }
                Collected::Changed(entry) => {
                    outcome.changed += 1;
                    outcome.entries.extend(entry);
                }
            }
        }
        if outcome.changed + outcome.awaiting_evidence > 0 {
            save(&current_path, &database)?;
        }
        Ok(outcome)
    })
}

pub fn list_changes(manual: &CorrectionConfig) -> Result<Vec<LearningChange>> {
    with_store_lock(|| Ok(load(&path()?)?.visible_changes(manual)))
}

pub fn resolve_change(
    manual: &CorrectionConfig,
    id: &str,
    resolution: ChangeResolution,
) -> Result<Vec<LearningChange>> {
    with_store_lock(|| {
        let path = path()?;
        let mut database = load(&path)?;
        database.resolve(manual, id, resolution)?;
        save(&path, &database)?;
        Ok(database.visible_changes(manual))
    })
}

enum Collected {
    Skipped,
    AwaitingEvidence,
    Pending,
    Changed(Option<LearnedEntry>),
}

impl Database {
    pub(super) fn clear_evidence_for(&mut self, id: &str) {
        self.evidence
            .retain(|evidence| evidence.target_id.as_deref() != Some(id));
    }

    fn inventory(&self, manual: &CorrectionConfig, export: bool) -> Vec<InventoryEntry> {
        let mut entries: Vec<_> = manual::inventory(manual, &self.changes)
            .into_iter()
            .filter(|item| !export || manual::exportable(item))
            .map(|item| item.item)
            .collect();
        entries.extend(self.entries.iter().rev().filter_map(|item| {
            (!export || !item.deleted).then(|| InventoryEntry {
                id: item.entry.id.clone(),
                kind: item.entry.kind,
                origin: EntryOrigin::Learned,
                value: entry_value(&item.entry),
                editable: !item.deleted,
            })
        }));
        entries
    }

    fn visible_changes(&self, manual: &CorrectionConfig) -> Vec<LearningChange> {
        let manual_items = manual::inventory(manual, &self.changes);
        self.changes
            .iter()
            .rev()
            .map(|change| {
                let mut public = change.public.clone();
                if change.manual_line.is_some() && public.status == ChangeStatus::Applied {
                    public.effective = Some(
                        manual_items
                            .iter()
                            .any(|item| item.applied_change.as_ref() == Some(&public.id)),
                    );
                }
                public
            })
            .collect()
    }

    fn collect_action(
        &mut self,
        manual: &CorrectionConfig,
        record_id: &str,
        activity_id: Option<&str>,
        inventory: &[InventoryEntry],
        action: &ValidatedAction,
    ) -> Collected {
        let mut current = self.inventory(manual, false);
        let target = match action.operation {
            LearningOperation::Add => None,
            _ => {
                let Some(id) = action.target_id.as_deref() else {
                    return Collected::Skipped;
                };
                let sent = unique_target(inventory, id);
                let latest = unique_target(&current, id);
                let Some(item) = latest.filter(|item| {
                    sent == Some(*item)
                        && item.editable
                        && item.value.enabled
                        && item.kind == action.kind
                }) else {
                    return Collected::Skipped;
                };
                Some(item.clone())
            }
        };
        let before = target.as_ref().map(|item| item.value.clone());
        let after = match action.operation {
            LearningOperation::Disable => before.clone().map(|mut value| {
                value.enabled = false;
                value
            }),
            _ => action.value.clone().filter(|value| value.enabled),
        };
        let Some(after) = after else {
            return Collected::Skipped;
        };
        if before.as_ref() == Some(&after) || (after.enabled && !valid_value(action.kind, &after)) {
            return Collected::Skipped;
        }
        if let Some(target) = &target {
            current.retain(|item| item.id != target.id);
        }
        if after.enabled && conflicts(action.kind, &after, &current) {
            return Collected::Skipped;
        }
        if target.is_none() && (action.kind == EntryKind::Legacy || !self.has_entry_capacity()) {
            return Collected::Skipped;
        }

        let origin = target
            .as_ref()
            .map_or(EntryOrigin::Learned, |item| item.origin);
        let target_id = target.map(|item| item.id);
        let needs_more_evidence = origin == EntryOrigin::Learned
            && (action.operation != LearningOperation::Add || action.kind == EntryKind::Rule);
        let record = Evidence {
            operation: action.operation,
            kind: action.kind,
            target_id,
            before,
            after,
            record_id: record_id.to_owned(),
        };
        if self.changes.iter().any(|change| {
            (change.public.status == ChangeStatus::Pending
                && Some(change.target_id.as_str()) == record.target_id.as_deref())
                || (change.public.status == ChangeStatus::Dismissed
                    && Some(change.target_id.as_str()) == record.target_id.as_deref()
                    && change.public.operation == record.operation
                    && change.public.before == record.before
                    && change.public.after.as_ref() == Some(&record.after))
        }) {
            return Collected::Skipped;
        }
        let prior = self
            .evidence
            .iter()
            .position(|prior| prior.same_action(&record));
        if needs_more_evidence {
            match prior {
                Some(index) if self.evidence[index].record_id == record_id => {
                    return Collected::AwaitingEvidence;
                }
                Some(_) => {}
                None => {
                    if self.evidence.len() >= MAX_EVIDENCE {
                        self.evidence.remove(0);
                    }
                    self.evidence.push(record);
                    return Collected::AwaitingEvidence;
                }
            }
        }

        let manual_line = if origin == EntryOrigin::Manual {
            let item = manual::inventory(manual, &self.changes)
                .into_iter()
                .find(|item| Some(&item.item.id) == record.target_id.as_ref());
            let Some(item) = item else {
                return Collected::Skipped;
            };
            Some(item.line)
        } else {
            None
        };
        if !self.reserve_change_slot() {
            return Collected::Skipped;
        }
        if let Some(index) = prior {
            self.evidence.remove(index);
        }
        let now = Utc::now().to_rfc3339();
        let target_id = record
            .target_id
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let mut user_generation = 0;
        let added = if action.operation == LearningOperation::Add {
            let entry = LearnedEntry {
                id: target_id.clone(),
                kind: action.kind,
                term: record.after.term.clone(),
                from: record.after.from.clone(),
                enabled: true,
                record_id: record_id.to_owned(),
                created_at: now.clone(),
            };
            self.entries.push(StoredEntry {
                entry: entry.clone(),
                deleted: false,
                user_generation,
            });
            Some(entry)
        } else {
            if origin == EntryOrigin::Learned {
                if let Some(item) = self
                    .entries
                    .iter_mut()
                    .find(|item| item.entry.id == target_id)
                {
                    assign_value(&mut item.entry, &record.after);
                    user_generation = item.user_generation;
                }
            }
            None
        };
        self.changes.push(StoredChange {
            public: LearningChange {
                id: Uuid::new_v4().to_string(),
                activity_id: activity_id.map(str::to_owned),
                record_id: record_id.to_owned(),
                kind: action.kind,
                operation: action.operation,
                origin,
                before: record.before,
                after: Some(record.after),
                status: if origin == EntryOrigin::Manual {
                    ChangeStatus::Pending
                } else {
                    ChangeStatus::Applied
                },
                reason: action.reason.clone(),
                created_at: now,
                effective: None,
            },
            target_id,
            manual_line,
            user_generation,
        });
        if origin == EntryOrigin::Manual {
            Collected::Pending
        } else {
            Collected::Changed(added)
        }
    }

    fn has_entry_capacity(&self) -> bool {
        self.entries.len() < MAX_ENTRY_SLOTS
            && self.entries.iter().filter(|item| !item.deleted).count() < MAX_ACTIVE_ENTRIES
    }

    fn reserve_change_slot(&mut self) -> bool {
        if self.changes.len() < MAX_CHANGES {
            return true;
        }
        let obsolete = self.changes.iter().position(|change| {
            matches!(
                change.public.status,
                ChangeStatus::Dismissed | ChangeStatus::Undone
            ) || (change.manual_line.is_none() && change.public.status == ChangeStatus::Applied)
        });
        if let Some(index) = obsolete {
            self.changes.remove(index);
        }
        obsolete.is_some()
    }

    fn resolve(
        &mut self,
        manual: &CorrectionConfig,
        id: &str,
        resolution: ChangeResolution,
    ) -> Result<()> {
        let index = self
            .changes
            .iter()
            .position(|change| change.public.id == id)
            .context("Learning change no longer exists")?;
        let change = &self.changes[index];
        match (&change.public.status, resolution) {
            (ChangeStatus::Pending, ChangeResolution::Dismiss) => {
                self.changes[index].public.status = ChangeStatus::Dismissed;
            }
            (ChangeStatus::Pending, ChangeResolution::Approve) => {
                let manual_line = change
                    .manual_line
                    .as_ref()
                    .context("Expected manual learning change")?;
                let target = manual::inventory(manual, &self.changes)
                    .into_iter()
                    .find(|item| {
                        item.line == *manual_line
                            && item.item.kind == change.public.kind
                            && change.public.before.as_ref() == Some(&item.item.value)
                            && item.item.editable
                    })
                    .context("The manual entry changed after the suggestion was made")?;
                if let Some(after) = change.public.after.as_ref().filter(|after| after.enabled) {
                    let others: Vec<_> = self
                        .inventory(manual, false)
                        .into_iter()
                        .filter(|item| item.id != target.item.id)
                        .collect();
                    if conflicts(change.public.kind, after, &others) {
                        bail!(
                            "The proposed spelling conflicts with the current dictionary or rules"
                        );
                    }
                }
                self.changes[index].public.status = ChangeStatus::Applied;
            }
            (ChangeStatus::Applied, ChangeResolution::Undo) => {
                if self.changes[index + 1..].iter().any(|later| {
                    later.public.status == ChangeStatus::Applied
                        && later.public.kind == change.public.kind
                        && (later.target_id == change.target_id
                            || (change.manual_line.is_some()
                                && later.manual_line == change.manual_line
                                && later
                                    .public
                                    .before
                                    .as_ref()
                                    .and_then(|value| value.from.as_ref())
                                    == change
                                        .public
                                        .before
                                        .as_ref()
                                        .and_then(|value| value.from.as_ref())))
                }) {
                    bail!("Undo the newer change to this item first");
                }
                if change.public.origin == EntryOrigin::Learned {
                    let item = self
                        .entries
                        .iter()
                        .find(|item| item.entry.id == change.target_id && !item.deleted)
                        .context("Learned entry no longer exists")?;
                    if change.public.after.as_ref() != Some(&entry_value(&item.entry))
                        || item.user_generation != change.user_generation
                    {
                        bail!("The learned entry changed after this change was applied");
                    }
                    let restored = change.public.before.clone().unwrap_or_else(|| {
                        let mut value = entry_value(&item.entry);
                        value.enabled = false;
                        value
                    });
                    let mut others = self.inventory(manual, false);
                    others.retain(|item| item.id != change.target_id);
                    if restored.enabled && conflicts(change.public.kind, &restored, &others) {
                        bail!("The prior spelling conflicts with the current dictionary or rules");
                    }
                    let item = self
                        .entries
                        .iter_mut()
                        .find(|item| item.entry.id == change.target_id)
                        .expect("entry was already verified");
                    assign_value(&mut item.entry, &restored);
                    let target_id = change.target_id.clone();
                    self.clear_evidence_for(&target_id);
                }
                self.changes[index].public.status = ChangeStatus::Undone;
            }
            // A retry after a successfully persisted operation is harmless.
            (ChangeStatus::Applied, ChangeResolution::Approve)
            | (ChangeStatus::Dismissed, ChangeResolution::Dismiss)
            | (ChangeStatus::Undone, ChangeResolution::Undo) => {}
            _ => bail!("This learning change is no longer available for that action"),
        }
        Ok(())
    }
}

impl Evidence {
    fn same_action(&self, other: &Self) -> bool {
        self.operation == other.operation
            && self.kind == other.kind
            && self.target_id == other.target_id
            && self.before == other.before
            && self.after == other.after
    }
}

fn unique_target<'a>(inventory: &'a [InventoryEntry], id: &str) -> Option<&'a InventoryEntry> {
    let mut items = inventory.iter().filter(|item| item.id == id);
    let item = items.next()?;
    items.next().is_none().then_some(item)
}

fn valid_value(kind: EntryKind, value: &EntryValue) -> bool {
    valid_term(&value.term)
        && match &value.from {
            None => kind != EntryKind::Rule,
            Some(from) => {
                kind != EntryKind::Dictionary && valid_term(from) && key(from) != key(&value.term)
            }
        }
}

fn entry_value(entry: &LearnedEntry) -> EntryValue {
    EntryValue {
        term: entry.term.clone(),
        from: entry.from.clone(),
        enabled: entry.enabled,
    }
}

fn assign_value(entry: &mut LearnedEntry, value: &EntryValue) {
    entry.term = value.term.clone();
    entry.from = value.from.clone();
    entry.enabled = value.enabled;
}

fn conflicts(kind: EntryKind, value: &EntryValue, available: &[InventoryEntry]) -> bool {
    let target = key(&value.term);
    let source = value.from.as_deref().map(key);
    available.iter().any(|existing| {
        let other_target = key(&existing.value.term);
        let other_source = existing.value.from.as_deref().map(key);
        if kind != EntryKind::Rule && existing.kind != EntryKind::Rule && target == other_target {
            return true;
        }
        if let Some(source) = &source {
            if existing.kind == EntryKind::Rule && other_source.is_none() {
                // Opaque manual rules still reach the live prompt.
                return other_target.contains(source);
            }
            if &other_target == source
                || other_source.as_ref() == Some(source)
                || other_source.as_ref() == Some(&target)
            {
                return true;
            }
        }
        kind != EntryKind::Rule && other_source.as_ref() == Some(&target)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn manual() -> CorrectionConfig {
        AppConfig::default().correction
    }

    fn value(term: &str, from: Option<&str>) -> EntryValue {
        EntryValue {
            term: term.into(),
            from: from.map(str::to_owned),
            enabled: true,
        }
    }

    fn action(
        kind: EntryKind,
        operation: LearningOperation,
        target_id: Option<String>,
        value: Option<EntryValue>,
    ) -> ValidatedAction {
        ValidatedAction {
            kind,
            operation,
            target_id,
            value,
            reason: "verified edit".into(),
        }
    }

    fn suggest(
        database: &mut Database,
        manual: &CorrectionConfig,
        record: &str,
        action: &ValidatedAction,
    ) -> Collected {
        let inventory = database.inventory(manual, true);
        database.collect_action(manual, record, Some(record), &inventory, action)
    }

    #[test]
    fn dictionary_and_multiple_confirmed_rules_are_independent() {
        let mut db = Database::default();
        let config = manual();
        let add_word = action(
            EntryKind::Dictionary,
            LearningOperation::Add,
            None,
            Some(value("BoltScribe", None)),
        );
        assert!(matches!(
            suggest(&mut db, &config, "r1", &add_word),
            Collected::Changed(Some(_))
        ));
        for source in ["波特思考", "博尔特笔记"] {
            let add_rule = action(
                EntryKind::Rule,
                LearningOperation::Add,
                None,
                Some(value("BoltScribe", Some(source))),
            );
            assert!(matches!(
                suggest(&mut db, &config, "r2", &add_rule),
                Collected::AwaitingEvidence
            ));
            assert!(matches!(
                suggest(&mut db, &config, "r2", &add_rule),
                Collected::AwaitingEvidence
            ));
            assert!(matches!(
                suggest(&mut db, &config, "r3", &add_rule),
                Collected::Changed(Some(_))
            ));
            assert!(matches!(
                suggest(&mut db, &config, "r4", &add_rule),
                Collected::Skipped
            ));
        }
        let unfamiliar =
            super::super::augment_correction_for_text(&config, &db.visible(), "一段不同的转写");
        assert_eq!(unfamiliar.dictionary_text, "BoltScribe");
        assert!(unfamiliar.correction_rules_text.is_empty());
        let matched =
            super::super::augment_correction_for_text(&config, &db.visible(), "我打开波特思考");
        assert_eq!(matched.dictionary_text, "BoltScribe");
        assert_eq!(
            matched.correction_rules_text,
            "\"波特思考\" -> \"BoltScribe\""
        );
        db.entries[0].entry.enabled = false;
        let rules_only = super::super::augment_correction_for_text(
            &config,
            &db.visible(),
            "波特思考和博尔特笔记",
        );
        assert!(rules_only.dictionary_text.is_empty());
        assert_eq!(rules_only.correction_rules_text.lines().count(), 2);
        assert!(matches!(
            suggest(&mut db, &config, "r5", &add_word),
            Collected::Skipped
        ));
    }

    #[test]
    fn manual_rule_changes_require_approval_preserve_original_text_and_can_be_undone() {
        let mut db = Database::default();
        let mut config = manual();
        config.correction_rules_text =
            "波特思考 → OldName # preserve comment\r\nopaque line\r\n".into();
        let inventory = db.inventory(&config, true);
        let change = action(
            EntryKind::Rule,
            LearningOperation::Update,
            Some(inventory[0].id.clone()),
            Some(value("BoltScribe", Some("波特思考"))),
        );
        assert!(matches!(
            suggest(&mut db, &config, "r1", &change),
            Collected::Pending
        ));
        let id = db.changes[0].public.id.clone();
        assert_eq!(manual::effective_config(&config, &db.changes).0, config);
        let mut stale = config.clone();
        stale.correction_rules_text = "波特思考 -> UpdatedByUser".into();
        assert!(db.resolve(&stale, &id, ChangeResolution::Approve).is_err());
        db.resolve(&config, &id, ChangeResolution::Approve).unwrap();
        let effective = manual::effective_config(&config, &db.changes).0;
        assert_eq!(
            effective.correction_rules_text,
            "\"波特思考\" -> \"BoltScribe\" # preserve comment\r\nopaque line\r\n"
        );
        assert_eq!(db.visible_changes(&config)[0].effective, Some(true));
        assert_eq!(db.visible_changes(&stale)[0].effective, Some(false));
        db.resolve(&config, &id, ChangeResolution::Undo).unwrap();
        assert_eq!(manual::effective_config(&config, &db.changes).0, config);
    }

    #[test]
    fn learned_updates_and_disables_need_distinct_recordings_and_support_undo() {
        let mut db = Database::default();
        let config = manual();
        let add = action(
            EntryKind::Dictionary,
            LearningOperation::Add,
            None,
            Some(value("BoltScribe", None)),
        );
        assert!(matches!(
            suggest(&mut db, &config, "r1", &add),
            Collected::Changed(Some(_))
        ));
        let target_id = db.visible()[0].id.clone();
        let update = action(
            EntryKind::Dictionary,
            LearningOperation::Update,
            Some(target_id.clone()),
            Some(value("BoltScribe Desktop", None)),
        );
        assert!(matches!(
            suggest(&mut db, &config, "r2", &update),
            Collected::AwaitingEvidence
        ));
        assert!(matches!(
            suggest(&mut db, &config, "r2", &update),
            Collected::AwaitingEvidence
        ));
        assert_eq!(db.visible()[0].term, "BoltScribe");
        assert!(matches!(
            suggest(&mut db, &config, "r3", &update),
            Collected::Changed(None)
        ));
        let update_id = db.changes[1].public.id.clone();
        let disable = action(
            EntryKind::Dictionary,
            LearningOperation::Disable,
            Some(target_id),
            None,
        );
        assert!(matches!(
            suggest(&mut db, &config, "r4", &disable),
            Collected::AwaitingEvidence
        ));
        assert!(matches!(
            suggest(&mut db, &config, "r5", &disable),
            Collected::Changed(None)
        ));
        assert!(!db.visible()[0].enabled);
        assert!(db
            .resolve(&config, &update_id, ChangeResolution::Undo)
            .is_err());
        let disable_id = db.changes[2].public.id.clone();
        db.resolve(&config, &disable_id, ChangeResolution::Undo)
            .unwrap();
        assert!(db.visible()[0].enabled);
        db.resolve(&config, &update_id, ChangeResolution::Undo)
            .unwrap();
        assert_eq!(db.visible()[0].term, "BoltScribe");
        let add_id = db.changes[0].public.id.clone();
        db.resolve(&config, &add_id, ChangeResolution::Undo)
            .unwrap();
        assert!(!db.visible()[0].enabled);
        assert!(matches!(
            suggest(&mut db, &config, "r6", &add),
            Collected::Skipped
        ));
    }

    #[test]
    fn manual_aliases_can_be_edited_and_undone_independently() {
        let mut db = Database::default();
        let mut config = manual();
        config.correction_rules_text =
            "\"甲左\"、\"乙左\" -> Both # aliases\r\n单独前缀 -> Separate\r\n单独前缀 -> Other"
                .into();
        let inventory = db.inventory(&config, true);
        assert_eq!(inventory.len(), 4);
        assert!(inventory[0].editable && inventory[1].editable);
        assert!(!inventory[2].editable && !inventory[3].editable);
        let disable = action(
            EntryKind::Rule,
            LearningOperation::Disable,
            Some(inventory[0].id.clone()),
            None,
        );
        assert!(matches!(
            suggest(&mut db, &config, "r1", &disable),
            Collected::Pending
        ));
        let id = db.changes[0].public.id.clone();
        db.resolve(&config, &id, ChangeResolution::Approve).unwrap();
        let (effective, reserved) = manual::effective_config(&config, &db.changes);
        assert!(!effective.correction_rules_text.contains("甲左"));
        assert!(effective
            .correction_rules_text
            .starts_with("\"乙左\" -> \"Both\" # aliases\r\n"));
        assert!(reserved.contains("甲左"));
        let readd = action(
            EntryKind::Rule,
            LearningOperation::Add,
            None,
            Some(value("Both", Some("甲左"))),
        );
        assert!(matches!(
            suggest(&mut db, &config, "r2", &readd),
            Collected::Skipped
        ));
        let update = action(
            EntryKind::Rule,
            LearningOperation::Update,
            Some(inventory[1].id.clone()),
            Some(value("Specific", Some("乙左"))),
        );
        assert!(matches!(
            suggest(&mut db, &config, "r3", &update),
            Collected::Pending
        ));
        let update_id = db.changes[1].public.id.clone();
        db.resolve(&config, &update_id, ChangeResolution::Approve)
            .unwrap();
        db.resolve(&config, &id, ChangeResolution::Undo).unwrap();
        let (effective, reserved) = manual::effective_config(&config, &db.changes);
        assert!(reserved.is_empty());
        assert!(effective.correction_rules_text.starts_with(
            "\"甲左\" -> \"Both\" # aliases\r\n\"乙左\" -> \"Specific\" # aliases\r\n"
        ));
    }

    #[test]
    fn explicit_toggles_reset_evidence_and_are_preserved_by_undo() {
        let mut db = Database::default();
        let config = manual();
        let add = action(
            EntryKind::Dictionary,
            LearningOperation::Add,
            None,
            Some(value("BoltScribe", None)),
        );
        assert!(matches!(
            suggest(&mut db, &config, "r1", &add),
            Collected::Changed(Some(_))
        ));
        let entry = db.visible().remove(0);
        let disable = action(
            EntryKind::Dictionary,
            LearningOperation::Disable,
            Some(entry.id.clone()),
            None,
        );
        assert!(matches!(
            suggest(&mut db, &config, "r2", &disable),
            Collected::AwaitingEvidence
        ));
        db.set_enabled(&entry.id, false).unwrap();
        db.set_enabled(&entry.id, true).unwrap();
        assert!(matches!(
            suggest(&mut db, &config, "r3", &disable),
            Collected::AwaitingEvidence
        ));
        assert!(matches!(
            suggest(&mut db, &config, "r4", &disable),
            Collected::Changed(None)
        ));
        let disable_id = db.changes[1].public.id.clone();
        db.set_enabled(&entry.id, true).unwrap();
        db.set_enabled(&entry.id, false).unwrap();
        assert!(db
            .resolve(&config, &disable_id, ChangeResolution::Undo)
            .is_err());
        assert!(!db.visible()[0].enabled);
    }
}
