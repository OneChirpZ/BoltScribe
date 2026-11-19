use super::{
    validation::sensitive, CorrectionEvidence, CorrectionState, InventoryEntry, PromptEntries,
    MAX_TEXT_UNITS,
};
use crate::{config::CorrectionConfig, corrector::enabled_dictionary_text};

const MAX_SECTION_BYTES: usize = 4_096;
const MAX_INVENTORY_BYTES: usize = 32 * 1024;

pub fn capture_correction(
    config: &CorrectionConfig,
    raw_text: &str,
    pasted_text: &str,
    failed: bool,
) -> CorrectionEvidence {
    let outcome = if !config.enabled {
        CorrectionState::Disabled
    } else if failed {
        CorrectionState::Failed
    } else {
        CorrectionState::Succeeded
    };
    let words = format!("{raw_text}\n{pasted_text}").to_lowercase();
    let dictionary =
        (config.enabled && config.prompt_template.contains("{{dictionary}}")).then(|| {
            excerpt(&enabled_dictionary_text(config), |line| {
                words.contains(&line.to_lowercase())
            })
        });
    let rules =
        (config.enabled && config.prompt_template.contains("{{correction_rules}}")).then(|| {
            excerpt(&config.correction_rules_text, |line| {
                super::store::parse_rule(line).is_some_and(|(sources, target)| {
                    words.contains(&target.to_lowercase())
                        || sources
                            .iter()
                            .any(|source| words.contains(&source.to_lowercase()))
                })
            })
        });
    CorrectionEvidence {
        raw_text: (raw_text.encode_utf16().count() <= MAX_TEXT_UNITS && !sensitive(raw_text))
            .then(|| raw_text.to_owned()),
        pasted_text: (pasted_text.encode_utf16().count() <= MAX_TEXT_UNITS
            && !sensitive(pasted_text))
        .then(|| pasted_text.to_owned()),
        dictionary,
        rules,
        outcome,
    }
}

fn excerpt(text: &str, relevant: impl Fn(&str) -> bool) -> PromptEntries {
    let mut lines: Vec<_> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines.sort_by_cached_key(|line| !relevant(line));
    let mut used = 0;
    let mut retained = Vec::new();
    let total = lines.len();
    for line in lines {
        // Bound serialized size and avoid sending sensitive remarks or terms.
        let bytes = serde_json::to_string(line)
            .expect("string is serializable")
            .len()
            + 1;
        if used + bytes <= MAX_SECTION_BYTES && !sensitive(line) {
            used += bytes;
            retained.push(line.to_owned());
        }
    }
    PromptEntries {
        complete: retained.len() == total,
        lines: retained,
    }
}

pub(super) fn select_inventory(
    mut entries: Vec<InventoryEntry>,
    sample: &super::LearningSample,
) -> (Vec<InventoryEntry>, bool) {
    let mut text = format!("{}\n{}", sample.original, sample.current).to_lowercase();
    for evidence in sample.correction.iter().flatten() {
        if let Some(raw) = &evidence.raw_text {
            text.push('\n');
            text.push_str(&raw.to_lowercase());
        }
    }
    entries.sort_by_cached_key(|entry| {
        !(text.contains(&entry.value.term.to_lowercase())
            || entry
                .value
                .from
                .as_ref()
                .is_some_and(|from| text.contains(&from.to_lowercase())))
    });
    let total = entries.len();
    let mut used = 0;
    entries.retain(|entry| {
        let bytes = serde_json::to_vec(entry)
            .expect("entry is serializable")
            .len()
            + 1;
        if used + bytes > MAX_INVENTORY_BYTES
            || sensitive(&entry.value.term)
            || entry.value.from.as_deref().is_some_and(sensitive)
        {
            return false;
        }
        used += bytes;
        true
    });
    let complete = entries.len() == total;
    (entries, complete)
}
