use super::{LearningSample, MAX_RESPONSE_BYTES, MAX_TERMS, MAX_TEXT_UNITS};
use anyhow::{bail, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Candidate {
    pub term: String,
    pub term_common: bool,
    #[serde(deserialize_with = "required_nullable_source")]
    pub from: Option<String>,
    pub from_common: bool,
}

fn required_nullable_source<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    terms: Vec<Candidate>,
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

pub(super) fn parse_candidates(
    raw: &str,
    sample: &LearningSample,
) -> Result<(Vec<Candidate>, usize)> {
    if raw.len() > MAX_RESPONSE_BYTES {
        bail!("Learning response exceeded 16 KiB");
    }
    let response: Response = serde_json::from_str(raw)
        .map_err(|_| anyhow::anyhow!("Learning model returned invalid candidate JSON"))?;
    if response.terms.len() > MAX_TERMS {
        bail!("Learning model returned more than 8 candidates");
    }
    let count = response.terms.len();
    let mut accepted: Vec<Candidate> = Vec::new();
    for candidate in response.terms {
        if candidate.term_common
            || candidate.from_common
            || !valid_term(&candidate.term)
            || !sample.current.contains(&candidate.term)
            // A term already present in the insertion is not a newly corrected spelling.
            || sample.original.contains(&candidate.term)
        {
            continue;
        }
        // Automatic admission requires a grounded replacement. A new noun without
        // a source can be a continuation of the dictated sentence, rather than a correction.
        let Some(from) = &candidate.from else {
            continue;
        };
        if !valid_term(from)
            || from == &candidate.term
            || !sample.original.contains(from)
            || sample.current.contains(from)
        {
            continue;
        }
        if accepted
            .iter_mut()
            .any(|item| key(&item.term) == key(&candidate.term))
        {
            continue;
        }
        accepted.push(candidate);
    }
    let rejected = count - accepted.len();
    Ok((accepted, rejected))
}

pub(super) fn key(value: &str) -> String {
    value.trim().to_lowercase()
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

    fn sample() -> LearningSample {
        LearningSample {
            activity: None,
            original: "用波特思考写文档".into(),
            current: "用BoltScribe写文档".into(),
            record_id: "test".into(),
        }
    }

    #[test]
    fn accepts_grounded_name_without_shared_characters() {
        let raw = r#"{"terms":[{"term":"BoltScribe","term_common":false,"from":"波特思考","from_common":false}]}"#;
        let (terms, rejected) = parse_candidates(raw, &sample()).unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(rejected, 0);
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
    fn rejects_hallucinated_common_unchanged_and_invalid_sources() {
        let raw = r#"{"terms":[
          {"term":"OtherName","term_common":false,"from":null,"from_common":false},
          {"term":"BoltScribe","term_common":true,"from":null,"from_common":false},
          {"term":"文档","term_common":false,"from":null,"from_common":false},
          {"term":"BoltScribe","term_common":false,"from":"不存在","from_common":false},
          {"term":"BoltScribe","term_common":false,"from":"文档","from_common":false},
          {"term":"BoltScribe","term_common":false,"from":"波特思考","from_common":true}
        ]}"#;
        assert_eq!(parse_candidates(raw, &sample()).unwrap(), (Vec::new(), 6));
    }

    #[test]
    fn rejects_entire_oversized_or_malformed_batch() {
        let candidate =
            r#"{"term":"BoltScribe","term_common":false,"from":null,"from_common":false}"#;
        let raw = format!("{{\"terms\":[{}]}}", [candidate; 9].join(","));
        assert!(parse_candidates(&raw, &sample()).is_err());
        assert!(parse_candidates(&" ".repeat(MAX_RESPONSE_BYTES + 1), &sample()).is_err());
        assert!(parse_candidates(r#"{"terms":[{"term":"BoltScribe"}]}"#, &sample()).is_err());
        assert!(parse_candidates(
            r#"{"terms":[{"term":"BoltScribe","term_common":false,"from_common":false}]}"#,
            &sample()
        )
        .is_err());
    }

    #[test]
    fn deduplicates_grounded_candidates() {
        let raw = r#"{"terms":[{"term":"BoltScribe","term_common":false,"from":"波特思考","from_common":false},{"term":"BoltScribe","term_common":false,"from":"波特思考","from_common":false}]}"#;
        let (terms, rejected) = parse_candidates(raw, &sample()).unwrap();
        assert_eq!(terms[0].from.as_deref(), Some("波特思考"));
        assert_eq!(terms.len(), 1);
        assert_eq!(rejected, 1);
    }

    #[test]
    fn does_not_learn_a_noun_that_was_only_appended() {
        let sample = LearningSample {
            activity: None,
            original: "写文档".into(),
            current: "写文档，使用BoltScribe".into(),
            record_id: "test".into(),
        };
        let raw = r#"{"terms":[{"term":"BoltScribe","term_common":false,"from":null,"from_common":false}]}"#;
        assert_eq!(parse_candidates(raw, &sample).unwrap(), (Vec::new(), 1));
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
