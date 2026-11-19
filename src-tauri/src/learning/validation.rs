use super::{
    CorrectionEvidence, CorrectionState, EntryKind, EntryValue, InventoryEntry, LearningOperation,
    LearningSample, ValidatedAction, MAX_RESPONSE_BYTES, MAX_TERMS, MAX_TEXT_UNITS,
};
use anyhow::{bail, Result};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposedAction {
    operation: LearningOperation,
    kind: EntryKind,
    target_id: Option<String>,
    term: Option<String>,
    from: Option<String>,
    observed_from: String,
    observed_to: String,
    term_common: bool,
    from_common: bool,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoveredPair {
    from: String,
    term: String,
    observed_from: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    actions: Vec<ProposedAction>,
    covered: Option<CoveredPair>,
}

pub(super) fn eligible_sample(sample: &LearningSample) -> bool {
    !sample.original.trim().is_empty()
        && !sample.current.trim().is_empty()
        && sample.original != sample.current
        && sample.original.encode_utf16().count() <= MAX_TEXT_UNITS
        && sample.current.encode_utf16().count() <= MAX_TEXT_UNITS
        && sample.record_id.len() <= 128
        && !sensitive(&sample.original)
        && !sensitive(&sample.current)
}

pub(super) fn sample_rejection(sample: &LearningSample) -> Option<&'static str> {
    if !eligible_sample(sample) {
        return Some("invalid_sample");
    }
    if !related_edit(&sample.original, &sample.current) {
        return Some("unrelated_edit");
    }
    None
}

// Both strings are already bounded to MAX_TEXT_UNITS and scoped to the
// verified insertion. Use Unicode scalar counts for the edit budget.
fn related_edit(original: &str, current: &str) -> bool {
    let original: Vec<_> = original.chars().collect();
    let current: Vec<_> = current.chars().collect();
    let length = original.len().max(current.len());
    if length <= 16 && looks_like_transliteration(&original, &current) {
        return true;
    }
    let limit = length * 3 / 4;
    if original.len().abs_diff(current.len()) > limit {
        return false;
    }
    let prefix = original
        .iter()
        .zip(&current)
        .take_while(|(a, b)| a == b)
        .count();
    let (original, current) = (&original[prefix..], &current[prefix..]);
    let suffix = original
        .iter()
        .rev()
        .zip(current.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    within_edit_budget(
        &original[..original.len() - suffix],
        &current[..current.len() - suffix],
        limit,
    )
}

// A short phonetic name such as 波特思考 → BoltScribe may change every character.
fn looks_like_transliteration(original: &[char], current: &[char]) -> bool {
    let has_ideograph = original.iter().any(|ch| {
        matches!(ch, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{2fa1f}' | '\u{30000}'..='\u{3347f}')
    });
    has_ideograph
        && original.iter().all(|ch| ch.is_alphanumeric())
        && current.iter().any(char::is_ascii_alphabetic)
        && current
            .iter()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+' | '#'))
}

fn within_edit_budget(original: &[char], current: &[char], limit: usize) -> bool {
    if original.len().max(current.len()) <= limit {
        return true;
    }
    let outside = limit + 1;
    // Insertions minus deletions must account for the final length change, and
    // their sum cannot exceed the entire budget. Paths outside this band fail.
    let max_insertions = (limit + current.len() - original.len()) / 2;
    let max_deletions = (limit + original.len() - current.len()) / 2;
    let mut previous: Vec<_> = (0..=current.len()).collect();
    let mut row = vec![outside; current.len() + 1];
    for (index, ch) in original.iter().enumerate() {
        let i = index + 1;
        let start = i.saturating_sub(max_deletions).max(1);
        let end = (i + max_insertions).min(current.len());
        row[start - 1] = if start == 1 { i } else { outside };
        let mut best = row[start - 1];
        for j in start..=end {
            let substitution = previous[j - 1] + usize::from(*ch != current[j - 1]);
            row[j] = substitution.min(previous[j] + 1).min(row[j - 1] + 1);
            best = best.min(row[j]);
        }
        if best > limit {
            return false;
        }
        if end < current.len() {
            row[end + 1] = outside;
        }
        std::mem::swap(&mut previous, &mut row);
    }
    previous[current.len()] <= limit
}

pub(super) struct Analysis {
    pub actions: Vec<ValidatedAction>,
    pub rejected: usize,
    pub already_covered: bool,
}

pub(super) fn parse_analysis(
    raw: &str,
    sample: &LearningSample,
    inventory: &[InventoryEntry],
) -> Result<Analysis> {
    if raw.len() > MAX_RESPONSE_BYTES {
        bail!("Learning response exceeded 16 KiB");
    }
    let response: Response = serde_json::from_str(raw)
        .map_err(|_| anyhow::anyhow!("Learning model returned invalid candidate JSON"))?;
    if response.actions.len() > MAX_TERMS {
        bail!("Learning model returned more than 8 actions");
    }
    let already_covered = response.covered.is_some_and(|pair| {
        valid_term(&pair.from)
            && grounded_replacement(sample, &pair.observed_from, &pair.term)
            && evidence_for(sample, &pair.observed_from).any(|evidence| {
                evidence.outcome == CorrectionState::Succeeded
                    && raw_contains(evidence, &pair.from)
                    && rule_was_sent(evidence, &pair.from, &pair.term)
            })
    });
    let count = response.actions.len();
    let mut seen = HashSet::new();
    let actions: Vec<_> = response
        .actions
        .into_iter()
        .filter_map(|proposal| {
            let action = validate_action(proposal, sample, inventory)?;
            let identity = action.target_id.clone().unwrap_or_else(|| {
                let value = action.value.as_ref().expect("add has a value");
                format!(
                    "{:?}:{}",
                    action.kind,
                    key(value.from.as_deref().unwrap_or(&value.term))
                )
            });
            seen.insert(identity).then_some(action)
        })
        .collect();
    Ok(Analysis {
        rejected: count - actions.len(),
        actions,
        already_covered,
    })
}

fn validate_action(
    proposal: ProposedAction,
    sample: &LearningSample,
    inventory: &[InventoryEntry],
) -> Option<ValidatedAction> {
    let reason = proposal.reason.trim();
    if reason.is_empty()
        || reason.chars().count() > 240
        || reason.chars().any(char::is_control)
        || sensitive(reason)
        || !grounded_replacement(sample, &proposal.observed_from, &proposal.observed_to)
    {
        return None;
    }

    let target = match proposal.operation {
        LearningOperation::Add => {
            if proposal.target_id.is_some() || proposal.kind == EntryKind::Legacy {
                return None;
            }
            None
        }
        LearningOperation::Update | LearningOperation::Disable => {
            Some(inventory.iter().find(|entry| {
                Some(&entry.id) == proposal.target_id.as_ref()
                    && entry.editable
                    && entry.value.enabled
                    && entry.kind == proposal.kind
                    && entry.value.term == proposal.observed_from
                    && evidence_for(sample, &proposal.observed_from).any(|evidence| {
                        evidence.outcome == CorrectionState::Succeeded
                            && entry_was_applicable(evidence, entry)
                    })
            })?)
        }
    };

    let value = if proposal.operation == LearningOperation::Disable {
        if proposal.term.is_some() || proposal.from.is_some() {
            return None;
        }
        None
    } else {
        let term = proposal.term?;
        if proposal.term_common || term != proposal.observed_to || has_prompt_placeholder(&term) {
            return None;
        }
        match (proposal.kind, proposal.from.as_deref()) {
            (EntryKind::Dictionary | EntryKind::Legacy, None)
                if target.map_or(true, |entry| entry.value.from.is_none()) => {}
            (EntryKind::Rule | EntryKind::Legacy, Some(from))
                if valid_term(from) && key(from) != key(&term) =>
            {
                if let Some(target) = target {
                    if target.value.from.as_deref() != Some(from) {
                        return None;
                    }
                } else if proposal.from_common
                    || has_prompt_placeholder(from)
                    || !evidence_for(sample, &proposal.observed_from)
                        .any(|evidence| raw_contains(evidence, from))
                    || evidence_for(sample, &proposal.observed_from).any(|evidence| {
                        evidence.outcome == CorrectionState::Succeeded
                            && raw_contains(evidence, from)
                            && rule_was_sent(evidence, from, &term)
                    })
                {
                    return None;
                }
            }
            _ => return None,
        }
        Some(EntryValue {
            term,
            from: proposal.from,
            enabled: true,
        })
    };

    Some(ValidatedAction {
        kind: proposal.kind,
        operation: proposal.operation,
        target_id: proposal.target_id,
        value,
        reason: reason.to_owned(),
    })
}

fn evidence_for<'a>(
    sample: &'a LearningSample,
    observed_from: &'a str,
) -> impl Iterator<Item = &'a CorrectionEvidence> {
    sample.correction.iter().flatten().filter(move |evidence| {
        evidence
            .pasted_text
            .as_deref()
            .is_some_and(|text| text.contains(observed_from))
    })
}

fn raw_contains(evidence: &CorrectionEvidence, from: &str) -> bool {
    evidence
        .raw_text
        .as_deref()
        .is_some_and(|text| text.contains(from))
}

fn rule_was_sent(evidence: &CorrectionEvidence, from: &str, term: &str) -> bool {
    evidence.rules.as_ref().is_some_and(|section| {
        section.lines.iter().any(|line| {
            super::store::parse_rule(line).is_some_and(|(sources, target)| {
                key(&target) == key(term) && sources.iter().any(|source| key(source) == key(from))
            })
        })
    })
}

fn entry_was_applicable(evidence: &CorrectionEvidence, entry: &InventoryEntry) -> bool {
    if let Some(from) = &entry.value.from {
        return raw_contains(evidence, from) && rule_was_sent(evidence, from, &entry.value.term);
    }
    evidence.dictionary.as_ref().is_some_and(|section| {
        section
            .lines
            .iter()
            .any(|line| key(line) == key(&entry.value.term))
    })
}

fn grounded_replacement(sample: &LearningSample, from: &str, to: &str) -> bool {
    if from == to || !valid_term(from) || !valid_term(to) {
        return false;
    }
    sample.original.match_indices(from).any(|(start, _)| {
        let (before, tail) = sample.original.split_at(start);
        let after = &tail[from.len()..];
        sample.current.match_indices(to).any(|(start, _)| {
            let (new_before, new_tail) = sample.current.split_at(start);
            let new_after = &new_tail[to.len()..];
            if before == new_before && after == new_after {
                return true;
            }
            // Permit independent edits elsewhere, using the unchanged text around this span.
            let left = before
                .chars()
                .rev()
                .zip(new_before.chars().rev())
                .take_while(|(left, right)| left == right)
                .take(4)
                .count();
            let right = after
                .chars()
                .zip(new_after.chars())
                .take_while(|(left, right)| left == right)
                .take(4)
                .count();
            (left > 0 || (before.is_empty() && new_before.is_empty()))
                && (right > 0 || (after.is_empty() && new_after.is_empty()))
                && left + right >= 3
        })
    })
}

pub(super) fn key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn has_prompt_placeholder(value: &str) -> bool {
    value.contains("{{") || value.contains("}}")
}

pub(super) fn valid_term(value: &str) -> bool {
    let count = value.chars().count();
    !value.is_empty()
        && value.trim() == value
        && (2..=80).contains(&count)
        && !value.chars().any(|ch| ch.is_control())
        && !["->", "=>", "→", "#", "|"]
            .iter()
            .any(|s| value.contains(s))
        && value.chars().any(char::is_alphabetic)
        && !sensitive(value)
}

/// Deliberately applies before a request as well as before persistence.
pub(super) fn sensitive(value: &str) -> bool {
    let lower = value.to_lowercase();
    if [
        "password",
        "passwd",
        "api_key",
        "api key",
        "apikey",
        "secret",
        "bearer ",
        "验证码",
        "密码",
        "身份证",
        "银行卡",
        "账号",
        "账户",
        "密钥",
        "住址",
        "家庭地址",
        "收货地址",
        "sk-",
        "sk_",
        "ghp_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "-----begin",
        "eyj",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return true;
    }
    // Email/account handles, URLs, and long digit runs (including common phone separators).
    if value.contains('@') || lower.contains("://") || lower.contains("www.") {
        return true;
    }
    let mut digits = 0;
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            digits += 1;
            if digits >= 6 {
                return true;
            }
        } else if !matches!(ch, ' ' | '-' | '(' | ')') {
            digits = 0;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::{EntryOrigin, PromptEntries};
    use serde_json::{json, Value};

    fn sample() -> LearningSample {
        LearningSample {
            activity: None,
            original: "用波特思考写文档".into(),
            current: "用BoltScribe写文档".into(),
            record_id: "test".into(),
            correction: None,
        }
    }

    fn response(actions: &[Value]) -> String {
        json!({"actions": actions, "covered": null}).to_string()
    }

    fn add(kind: &str, from: Option<&str>) -> Value {
        json!({
            "operation": "add", "kind": kind, "target_id": null,
            "term": "BoltScribe", "from": from, "observed_from": "波特思考",
            "observed_to": "BoltScribe", "term_common": false, "from_common": false,
            "reason": "人工修正了项目名"
        })
    }

    fn with_correction(mut sample: LearningSample, rules: &[&str]) -> LearningSample {
        sample.correction = Some(vec![CorrectionEvidence {
            outcome: CorrectionState::Succeeded,
            pasted_text: Some(sample.original.clone()),
            raw_text: Some(sample.original.clone()),
            dictionary: None,
            rules: Some(PromptEntries {
                lines: rules.iter().map(|line| (*line).to_owned()).collect(),
                complete: true,
            }),
        }]);
        sample
    }

    #[test]
    fn accepts_dictionary_and_rule_as_independent_actions() {
        let sample = with_correction(sample(), &[]);
        let mut dictionary = add("dictionary", None);
        dictionary["from_common"] = json!(true);
        let raw = response(&[dictionary, add("rule", Some("波特思考"))]);
        let result = parse_analysis(&raw, &sample, &[]).unwrap();
        assert_eq!(result.rejected, 0);
        assert_eq!(result.actions[0].kind, EntryKind::Dictionary);
        assert_eq!(result.actions[0].value.as_ref().unwrap().from, None);
        assert_eq!(result.actions[1].kind, EntryKind::Rule);
    }

    #[test]
    fn allows_short_transliterated_terms_but_filters_unrelated_short_prompts() {
        assert!(related_edit("波特思考", "BoltScribe"));
        assert!(related_edit("西加加", "C++"));
        assert!(!related_edit("hello", "world"));
        assert!(!related_edit("试一试", "review this"));
    }

    #[test]
    fn edit_distance_counts_emoji_as_characters() {
        let prefix = "😀".repeat(4);
        let original = prefix.clone() + &"a".repeat(13);
        let current = prefix + &"b".repeat(13);
        assert!(!related_edit(&original, &current));
        let prefix = "🧐".repeat(5);
        assert!(related_edit(
            &(prefix.clone() + &"a".repeat(12)),
            &(prefix + &"😀".repeat(12))
        ));
    }

    #[test]
    fn longer_edits_must_stay_within_three_quarters_of_the_larger_length() {
        let original = "abcdefghijklmnopqrst";
        assert!(related_edit(original, "abcde_______________"));
        assert!(!related_edit(original, "abcd________________"));
        assert!(related_edit(original, "_______hijkl________"));
        assert!(!related_edit(original, "_______hijk_________"));
        assert!(related_edit(original, "bcdef"));
        assert!(!related_edit(original, "bcde"));
        // Insertion and deletion shift the intervening characters; counting
        // mismatched positions would reject a rotation even at low distance.
        assert!(related_edit(original, "bcdefghijklmnopqrsta"));
    }

    #[test]
    fn skips_unrelated_rewrites_before_queueing_and_prioritizes_sensitive_checks() {
        let mut sample = LearningSample {
            original: "今天天气晴朗，下午我们一起去公园散步。".into(),
            current: "ModelServiceRegistry".into(),
            ..sample()
        };
        assert_eq!(sample_rejection(&sample), Some("unrelated_edit"));
        sample.current.push_str(" api_key");
        assert_eq!(sample_rejection(&sample), Some("invalid_sample"));
        sample.original = "旧".repeat(MAX_TEXT_UNITS - 1) + "稿";
        sample.current = "新".into();
        assert_eq!(sample_rejection(&sample), Some("unrelated_edit"));
        sample.current = "旧".repeat(MAX_TEXT_UNITS - 1) + "文";
        assert_eq!(sample_rejection(&sample), None);
    }

    #[test]
    fn rejects_unknown_asr_sources_common_rule_sources_and_duplicate_actions() {
        let sample = with_correction(sample(), &[]);
        let mut common = add("rule", Some("波特思考"));
        common["from_common"] = json!(true);
        let raw = response(&[
            common,
            add("rule", Some("没出现过")),
            add("dictionary", None),
            add("dictionary", None),
        ]);
        let result = parse_analysis(&raw, &sample, &[]).unwrap();
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.rejected, 3);
    }

    #[test]
    fn does_not_learn_an_appended_word_or_accept_deletion_operation() {
        let mut sample = sample();
        sample.current = "用波特思考写文档，打开BoltScribe".into();
        assert!(
            parse_analysis(&response(&[add("dictionary", None)]), &sample, &[])
                .unwrap()
                .actions
                .is_empty()
        );
        let mut delete = add("dictionary", None);
        delete["operation"] = json!("delete");
        assert!(parse_analysis(&response(&[delete]), &sample, &[]).is_err());
        assert!(parse_analysis(&" ".repeat(MAX_RESPONSE_BYTES + 1), &sample, &[]).is_err());
        assert!(
            parse_analysis(&response(&vec![add("dictionary", None); 9]), &sample, &[]).is_err()
        );
    }

    #[test]
    fn existing_rule_in_successful_prompt_is_coverage_instead_of_duplicate_add() {
        let sample = with_correction(sample(), &["波特思考 -> BoltScribe"]);
        let raw = json!({
            "actions": [add("rule", Some("波特思考"))],
            "covered": {"from": "波特思考", "term": "BoltScribe", "observed_from": "波特思考"}
        })
        .to_string();
        let result = parse_analysis(&raw, &sample, &[]).unwrap();
        assert!(result.already_covered);
        assert!(result.actions.is_empty());
    }

    #[test]
    fn update_requires_a_known_entry_and_evidence_it_could_have_affected_correction() {
        let mut sample = with_correction(sample(), &["波特 -> 波特思考"]);
        sample.correction.as_mut().unwrap()[0].raw_text = Some("用波特写文档".into());
        let target = InventoryEntry {
            id: "manual-rule".into(),
            kind: EntryKind::Rule,
            origin: EntryOrigin::Manual,
            value: EntryValue {
                from: Some("波特".into()),
                term: "波特思考".into(),
                enabled: true,
            },
            editable: true,
        };
        let mut proposal = add("rule", Some("波特"));
        proposal["operation"] = json!("update");
        proposal["target_id"] = json!("manual-rule");
        let raw = response(&[proposal]);
        assert!(parse_analysis(&raw, &sample, &[])
            .unwrap()
            .actions
            .is_empty());
        assert_eq!(
            parse_analysis(&raw, &sample, std::slice::from_ref(&target))
                .unwrap()
                .actions
                .len(),
            1
        );
        sample.correction.as_mut().unwrap()[0].rules = None;
        assert!(parse_analysis(&raw, &sample, &[target])
            .unwrap()
            .actions
            .is_empty());
    }

    #[test]
    fn excludes_sensitive_samples_before_network_and_counts_utf16() {
        for secret in [
            "账号 test",
            "mail a@example.com",
            "138 1234 5678",
            "sk-test-key",
            "密码123",
            "https://example.com",
        ] {
            let mut value = sample();
            value.original.push_str(secret);
            assert!(!eligible_sample(&value), "{secret}");
        }
        let mut value = sample();
        value.current = "😀".repeat(MAX_TEXT_UNITS / 2 + 1);
        assert!(!eligible_sample(&value));
        assert!(eligible_sample(&sample()));
    }
}
