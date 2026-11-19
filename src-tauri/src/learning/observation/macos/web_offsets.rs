use super::Insertion;

/// Chromium's AXValue can contain paragraph separators that its caret offset
/// does not count. Require one exact insertion at a compatible boundary.
pub(super) fn confirm(
    baseline: &str,
    caret: usize,
    original: &str,
    verified: &str,
) -> Option<Insertion> {
    if !baseline.contains('\n') || original.is_empty() {
        return None;
    }
    let mut matched = None;
    for (prefix, suffix) in anchors_at(baseline, caret) {
        let Some((prefix, middle)) = scoped_readback(prefix, suffix, verified) else {
            continue;
        };
        if middle == original && matched.replace((prefix, suffix)).is_some() {
            return None;
        }
    }
    let (prefix, suffix) = matched?;
    Some(Insertion {
        prefix: prefix.to_owned(),
        suffix: suffix.to_owned(),
        original: original.to_owned(),
    })
}

/// ProseMirror can report a stale caret and change its layout characters when
/// a paragraph gains text. Require one exact insertion and an intact draft.
pub(super) fn confirm_layout(baseline: &str, original: &str, verified: &str) -> Option<Insertion> {
    let first_char = original.chars().next()?;
    let start = verified.find(original)?;
    // Start the second search one scalar later to catch overlapping matches.
    if verified[start + first_char.len_utf8()..].contains(original) {
        return None;
    }
    let prefix = &verified[..start];
    let suffix = &verified[start + original.len()..];
    let draft_suffix = baseline.strip_prefix(prefix.trim_end_matches(is_editor_layout))?;
    if draft_suffix.trim_matches(is_editor_layout) != suffix.trim_matches(is_editor_layout) {
        return None;
    }
    Some(Insertion {
        prefix: prefix.to_owned(),
        suffix: suffix.to_owned(),
        original: original.to_owned(),
    })
}

fn is_editor_layout(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t' | '\n' | '\r' | '\u{a0}' | '\u{200b}' | '\u{2028}' | '\u{2029}'
    )
}

/// Incomplete echo permits another read of the same editor. Only full
/// confirmation can establish a learning sample.
pub(super) fn is_partial(baseline: &str, caret: usize, original: &str, verified: &str) -> bool {
    if !baseline.contains('\n') || original.is_empty() {
        return false;
    }
    anchors_at(baseline, caret).any(|(prefix, suffix)| {
        let Some((_, middle)) = scoped_readback(prefix, suffix, verified) else {
            return false;
        };
        if middle.is_empty() {
            return baseline == verified;
        }
        middle.len() < original.len() && original.starts_with(middle)
    })
}

fn anchors_at(baseline: &str, caret: usize) -> impl Iterator<Item = (&str, &str)> {
    baseline
        .char_indices()
        .map(|(index, ch)| (index, if ch == '\n' { 0 } else { ch.len_utf16() }))
        .chain(std::iter::once((baseline.len(), 0)))
        .scan(0, move |offset, (index, units)| {
            let at = *offset;
            *offset += units;
            (at <= caret).then_some((at, index))
        })
        .filter(move |(at, _)| *at == caret)
        .map(|(_, index)| baseline.split_at(index))
}

fn scoped_readback<'a>(
    prefix: &str,
    suffix: &str,
    verified: &'a str,
) -> Option<(&'a str, &'a str)> {
    let readback = verified.strip_suffix(suffix)?;
    let middle = readback.strip_prefix(prefix).or_else(|| {
        readback
            .strip_prefix(prefix.strip_suffix('\u{a0}')?)?
            .strip_prefix(' ')
    })?;
    let prefix = &readback[..readback.len() - middle.len()];
    Some((prefix, middle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirms_appending_when_the_caret_omits_paragraph_separators() {
        let confirmed = confirm("前段\n后段", 4, "语音", "前段\n后段语音").unwrap();
        assert_eq!(confirmed.prefix, "前段\n后段");
        assert!(confirmed.suffix.is_empty());
        assert_eq!(confirmed.extract("前段\n后段修改").as_deref(), Some("修改"));
    }

    #[test]
    fn confirms_insertion_in_a_nonempty_middle_paragraph() {
        let confirmed = confirm("前段\n甲乙\n尾段", 3, "语音", "前段\n甲语音乙\n尾段").unwrap();
        assert_eq!(confirmed.prefix, "前段\n甲");
        assert_eq!(confirmed.suffix, "乙\n尾段");
        assert_eq!(
            confirmed.extract("前段\n甲修改乙\n尾段").as_deref(),
            Some("修改")
        );
    }

    #[test]
    fn confirms_exact_web_insertion_when_boundary_or_tail_layout_changes() {
        let original = "语音";
        for (baseline, prefix, suffix) in [
            ("前段\n后段\n", "前段\n后段 ", ""),
            ("前段\n后段\n", "前段\n后段", ""),
            ("前段\n甲乙\n", "前段\n甲\u{200b}", "\u{a0}乙"),
            ("前段\n甲\r\n乙", "前段\n甲\n", "\n乙\n"),
        ] {
            let verified = format!("{prefix}{original}{suffix}");
            let confirmed = confirm_layout(baseline, original, &verified).unwrap();
            assert_eq!(confirmed.prefix, prefix);
            assert_eq!(confirmed.suffix, suffix);
            assert_eq!(
                confirmed
                    .extract(&format!("{prefix}修改{suffix}"))
                    .as_deref(),
                Some("修改")
            );
        }
    }

    #[test]
    fn layout_confirmation_keeps_draft_content_and_rejects_ambiguous_matches() {
        for invalid in [
            "改动\n甲语音乙",
            "前段\n甲语音丙",
            "前段 甲语音乙",
            "前段\n甲语音乙新增",
            "前段\n甲语乙",
            "前段\n甲语音乙语音",
        ] {
            assert!(confirm_layout("前段\n甲乙", "语音", invalid).is_none());
        }
        assert!(confirm_layout("哈", "哈哈", "哈哈哈").is_none());
        assert!(confirm_layout("前段\u{200d}", "语音", "前段语音").is_none());
    }

    #[test]
    fn keeps_only_the_verified_space_normalization_at_the_insertion() {
        let confirmed = confirm("前段\n后段\u{a0}", 5, "语音", "前段\n后段 语音").unwrap();
        assert_eq!(confirmed.prefix, "前段\n后段 ");
        assert!(confirm("前段\u{a0}\n后段", 5, "语音", "前段 \n后段语音").is_none());
        assert!(confirm("前段\n后段 ", 5, "语音", "前段\n后段\u{a0}语音").is_none());
    }

    #[test]
    fn uses_utf16_and_keeps_all_draft_anchors() {
        let baseline = "😀\n后文";
        let original = "语音";
        let verified = "😀\n后语音文";
        assert!(confirm(baseline, 3, original, verified).is_some());
        assert!(confirm(baseline, 2, original, verified).is_none());
        assert!(confirm(baseline, 1, original, verified).is_none());
        assert!(confirm(baseline, 3, original, "😀\n后语音字").is_none());
        assert!(confirm(baseline, 3, original, "😁\n后语音文").is_none());
        assert!(confirm(baseline, 3, original, "😀\n后修改文").is_none());
        assert!(confirm("前段\n后段", 4, "", "前段\n后段").is_none());
        assert!(confirm("前段后段", 4, original, "前段后段语音").is_none());
    }

    #[test]
    fn rejects_multiple_compatible_insertions_at_a_paragraph_separator() {
        assert!(confirm("前段\n后段", 2, "\n", "前段\n\n后段").is_none());
    }

    #[test]
    fn recognizes_only_a_shorter_echo_with_the_entire_draft_intact() {
        let baseline = "前段\n甲乙\n尾段";
        let original = "语音回读";
        assert!(is_partial(baseline, 3, original, baseline));
        assert!(is_partial(baseline, 3, original, "前段\n甲语音乙\n尾段"));
        assert!(!is_partial(
            baseline,
            3,
            original,
            "前段\n甲语音回读乙\n尾段"
        ));
        assert!(!is_partial(baseline, 3, original, "前段\n甲改字乙\n尾段"));
        assert!(!is_partial(baseline, 3, original, "前段\n甲语音丙\n尾段"));
        assert!(!is_partial(baseline, 2, original, "前段\n甲语音乙\n尾段"));
    }

    #[test]
    fn recognizes_partial_echo_with_the_known_boundary_space_conversion() {
        let baseline = "😀\n后段\u{a0}";
        let original = "语音回读";
        assert!(is_partial(baseline, 5, original, "😀\n后段 语"));
        assert!(!is_partial(baseline, 5, original, "😀\n后段 "));
        assert!(!is_partial(baseline, 5, original, "😀\n后段 回读"));
        assert!(!is_partial("😀\u{a0}\n后段", 5, original, "😀 \n后段语"));
    }
}
