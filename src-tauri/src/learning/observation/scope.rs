use super::{Insertion, MAX_EDITOR_TEXT_UNITS, MAX_TEXT_UNITS};
use std::ops::Range;

/// Verified dictation regions in editor order. The intervening draft stays
/// in the local anchors, outside the text given to the learning model.
pub(super) struct Scope {
    prefix: String,
    regions: Vec<Region>,
}

struct Region {
    original: String,
    following: String,
}

impl Scope {
    pub(super) fn new(insertion: Insertion) -> Self {
        Self {
            prefix: insertion.prefix,
            regions: vec![Region {
                original: insertion.original,
                following: insertion.suffix,
            }],
        }
    }

    pub(super) fn original(&self) -> String {
        self.regions
            .iter()
            .map(|region| region.original.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn extract(&self, snapshot: &str) -> Option<String> {
        let current = self
            .ranges(snapshot)?
            .into_iter()
            .map(|range| &snapshot[range])
            .collect::<Vec<_>>()
            .join("\n");
        (!current.trim().is_empty()).then_some(current)
    }

    fn ranges(&self, snapshot: &str) -> Option<Vec<Range<usize>>> {
        if snapshot.encode_utf16().count() > MAX_EDITOR_TEXT_UNITS {
            return None;
        }
        let (last, regions) = self.regions.split_last()?;
        let middle = snapshot
            .strip_prefix(&self.prefix)?
            .strip_suffix(&last.following)?;
        let mut ranges = Vec::with_capacity(self.regions.len());
        let mut cursor = 0;
        for region in regions {
            let gap = cursor + middle[cursor..].find(&region.following)?;
            ranges.push(self.prefix.len() + cursor..self.prefix.len() + gap);
            cursor = gap + region.following.len();
        }
        ranges.push(self.prefix.len() + cursor..self.prefix.len() + middle.len());

        // A repeated anchor is safe only if every ordered match allocates
        // the same text to the voice regions.
        cursor = middle.len();
        for (region, range) in regions.iter().zip(&ranges).rev() {
            let gap = middle[..cursor].rfind(&region.following)?;
            if self.prefix.len() + gap != range.end {
                return None;
            }
            cursor = gap;
        }
        Some(ranges)
    }

    pub(super) fn merge(&self, baseline: &str, next: &Insertion) -> Option<Self> {
        let (cut, added_after) = replaced_layout(baseline, next)?;
        let ranges = self.ranges_before_splice(baseline, &cut)?;
        let inserted = next.prefix.len()..next.prefix.len() + next.original.len();
        let tail_start = inserted.end + added_after;
        let post = format!("{}{}{}", next.prefix, next.original, next.suffix);
        if post.encode_utf16().count() > MAX_EDITOR_TEXT_UNITS {
            return None;
        }
        let mut parts = Vec::with_capacity(self.regions.len() + 1);
        for (range, region) in ranges.into_iter().zip(&self.regions) {
            let part = &baseline[range.clone()];
            let before = part.trim_end_matches(is_layout);
            let after = part.trim_start_matches(is_layout);
            let before_end = range.start + before.len();
            let after_start = range.end - after.len();

            if range.end < cut.start {
                parts.push((range, region.original.as_str()));
            } else if range.start > cut.end {
                let start = tail_start + range.start - cut.end;
                let end = tail_start + range.end - cut.end;
                parts.push((start..end, region.original.as_str()));
            } else if range.start <= cut.start && before_end <= cut.start {
                let trim_break =
                    has_break(&part[before.len()..]) && !has_trailing_break(&region.original);
                let end = if trim_break {
                    before_end
                } else {
                    range.end.min(cut.start)
                };
                parts.push((range.start..end, region.original.as_str()));
            } else if range.end >= cut.end && after_start >= cut.end {
                let trim_break = has_break(&part[..part.len() - after.len()])
                    && !has_leading_break(&region.original);
                let start = if trim_break {
                    after_start
                } else {
                    range.start.max(cut.end)
                };
                let start = tail_start + start - cut.end;
                let end = tail_start + range.end - cut.end;
                parts.push((start..end, region.original.as_str()));
            } else if cut.is_empty() && part == region.original {
                let (before, after) = region.original.split_at(cut.start - range.start);
                parts.push((range.start..cut.start, before));
                parts.push((tail_start..tail_start + after.len(), after));
            } else {
                // The new insertion overlaps existing voice text. Its
                // position in the original may be hidden by a prior edit.
                return None;
            }
        }
        let index = parts.partition_point(|(range, _)| range.end <= inserted.start);
        parts.insert(index, (inserted, &next.original));
        Self::from_parts(&post, parts)
    }

    fn ranges_before_splice(
        &self,
        baseline: &str,
        cut: &Range<usize>,
    ) -> Option<Vec<Range<usize>>> {
        self.ranges(baseline).or_else(|| {
            // A new paragraph at the end can repeat a known interior gap.
            // The confirmed append position identifies the new edge layout.
            let last = self.regions.last()?;
            if cut.end != baseline.len() || !last.following.is_empty() {
                return None;
            }
            let trimmed = baseline[..cut.start].trim_end_matches(is_layout);
            (has_break(&baseline[trimmed.len()..]) && !has_trailing_break(&last.original))
                .then(|| self.ranges(trimmed))
                .flatten()
        })
    }

    fn from_parts(post: &str, parts: Vec<(Range<usize>, &str)>) -> Option<Self> {
        let prefix = post[..parts.first()?.0.start].to_owned();
        let mut ranges: Vec<Range<usize>> = Vec::with_capacity(parts.len());
        let mut regions: Vec<Region> = Vec::with_capacity(parts.len());
        let mut units = 0;
        for (range, text) in parts {
            units += text.encode_utf16().count();
            if let Some(previous) = ranges.last_mut() {
                let region = regions.last_mut()?;
                if previous.end == range.start {
                    previous.end = range.end;
                    region.original.push_str(text);
                    continue;
                }
                region.following = post.get(previous.end..range.start)?.to_owned();
                units += 1;
            }
            ranges.push(range);
            regions.push(Region {
                original: text.to_owned(),
                following: String::new(),
            });
        }
        if units > MAX_TEXT_UNITS {
            return None;
        }
        regions.last_mut()?.following = post[ranges.last()?.end..].to_owned();
        let scope = Self { prefix, regions };
        (scope.ranges(post)? == ranges).then_some(scope)
    }
}

/// In the ordinary case the confirmed anchors name the exact selection.
/// Web editors can also normalize layout at the insertion boundary. Compare
/// both untouched sides and accept only layout changes at that boundary.
fn replaced_layout(baseline: &str, next: &Insertion) -> Option<(Range<usize>, usize)> {
    if let Some(selected) = baseline
        .strip_prefix(&next.prefix)
        .and_then(|rest| rest.strip_suffix(&next.suffix))
    {
        return Some((next.prefix.len()..next.prefix.len() + selected.len(), 0));
    }
    let start = baseline
        .chars()
        .zip(next.prefix.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = baseline[start..]
        .chars()
        .rev()
        .zip(next.suffix.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let end = baseline.len() - suffix;
    let added_after = next.suffix.len() - suffix;
    (baseline[start..end].chars().all(is_layout)
        && next.prefix[start..].chars().all(is_layout)
        && next.suffix[..added_after].chars().all(is_layout))
    .then_some((start..end, added_after))
}

fn is_layout(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t' | '\n' | '\r' | '\u{a0}' | '\u{200b}' | '\u{2028}' | '\u{2029}'
    )
}

fn has_break(text: &str) -> bool {
    text.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
}

fn has_trailing_break(text: &str) -> bool {
    has_break(&text[text.trim_end_matches(is_layout).len()..])
}

fn has_leading_break(text: &str) -> bool {
    has_break(&text[..text.len() - text.trim_start_matches(is_layout).len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insertion(prefix: &str, original: &str, suffix: &str) -> Insertion {
        Insertion {
            prefix: prefix.into(),
            original: original.into(),
            suffix: suffix.into(),
        }
    }

    #[test]
    fn adjacent_dictations_keep_edits_made_before_the_next_paste() {
        let first = Scope::new(insertion("已有😀", "第一段科德克斯。", "末尾"));
        let next = insertion("已有😀第一段Codex。", "第二段波特思考。", "末尾");
        let merged = first.merge("已有😀第一段Codex。末尾", &next).unwrap();
        assert_eq!(merged.original(), "第一段科德克斯。第二段波特思考。");
        assert_eq!(
            merged
                .extract("已有😀第一段Codex。第二段波特思考。末尾")
                .as_deref(),
            Some("第一段Codex。第二段波特思考。")
        );
        let next = insertion("已有😀第一段Codex。第二段BoltScribe。", "第三段。", "末尾");
        let merged = merged
            .merge("已有😀第一段Codex。第二段BoltScribe。末尾", &next)
            .unwrap();
        assert_eq!(
            merged.original(),
            "第一段科德克斯。第二段波特思考。第三段。"
        );
        assert_eq!(
            merged
                .extract("已有😀第一段Codex。第二段BoltScribe。第三段。末尾")
                .as_deref(),
            Some("第一段Codex。第二段BoltScribe。第三段。")
        );
    }

    #[test]
    fn prepends_and_filters_the_draft_between_voice_regions() {
        let first = Scope::new(insertion("抬头旧文。已有草稿/", "第一段", "尾注"));
        let next = insertion("抬头", "第二段", "。已有草稿/第一节尾注");
        let merged = first.merge("抬头旧文。已有草稿/第一节尾注", &next).unwrap();
        assert_eq!(merged.original(), "第二段\n第一段");
        assert_eq!(
            merged.extract("抬头第二节。已有草稿/第一节尾注").as_deref(),
            Some("第二节\n第一节")
        );
        assert!(merged.extract("抬头第二节。其他草稿/第一节尾注").is_none());
        assert!(merged
            .extract("抬头第二节。已有草稿/。已有草稿/第一节尾注")
            .is_none());
    }

    #[test]
    fn keeps_typed_paragraph_breaks_outside_adjacent_dictations() {
        let first = Scope::new(insertion("", "第一段", ""));
        let merged = first
            .merge("第一节\n\n", &insertion("第一节\n\n", "第二段", ""))
            .unwrap();
        assert_eq!(merged.original(), "第一段\n第二段");
        assert_eq!(
            merged.extract("第一节\n\n第二节").as_deref(),
            Some("第一节\n第二节")
        );
        let third = merged
            .merge(
                "第一节\n\n第二节\n\n",
                &insertion("第一节\n\n第二节\n\n", "第三段", ""),
            )
            .unwrap();
        assert_eq!(third.original(), "第一段\n第二段\n第三段");
        assert_eq!(
            third.extract("第一节\n\n第二节\n\n第三节").as_deref(),
            Some("第一节\n第二节\n第三节")
        );
    }

    #[test]
    fn places_a_dictation_inside_an_unchanged_voice_region() {
        let first = Scope::new(insertion("前文", "开头😀结尾", "后文"));
        let next = insertion("前文开头😀", "插入", "结尾后文");
        let merged = first.merge("前文开头😀结尾后文", &next).unwrap();
        assert_eq!(merged.original(), "开头😀插入结尾");
        assert_eq!(
            merged.extract("前文开头😀纠正结尾后文").as_deref(),
            Some("开头😀纠正结尾")
        );
    }

    #[test]
    fn merges_when_the_editor_normalizes_layout_at_the_paste_boundary() {
        let first = Scope::new(insertion("原有草稿", "第一段", "\n\n"));
        let next = insertion("原有草稿第一节\n", "第二段", "");
        let merged = first.merge("原有草稿第一节\n\n", &next).unwrap();
        assert_eq!(merged.original(), "第一段\n第二段");
        assert_eq!(
            merged.extract("原有草稿第一节\n第二节").as_deref(),
            Some("第一节\n第二节")
        );
        assert!(first
            .merge(
                "原有草稿第一节\n\n",
                &insertion("别的草稿第一节\n", "第二段", "")
            )
            .is_none());
    }

    #[test]
    fn spaces_added_by_the_editor_do_not_shift_the_prior_region() {
        let first = Scope::new(insertion("前文", "旧名", "后文"));
        let next = insertion("前文", "补充", "\n新名后文");
        let merged = first.merge("前文新名后文", &next).unwrap();
        assert_eq!(merged.original(), "补充\n旧名");
        assert_eq!(
            merged.extract("前文已改\n新名后文").as_deref(),
            Some("已改\n新名")
        );
    }

    #[test]
    fn rejects_overwriting_prior_voice_and_oversized_merged_scope() {
        let first = Scope::new(insertion("已有", "第一段", "末尾"));
        assert!(first
            .merge("已有第一节末尾", &insertion("已有第", "第二段", "节末尾"))
            .is_none());
        let first = Scope::new(insertion("", &"文".repeat(MAX_TEXT_UNITS), ""));
        assert!(first
            .merge(
                &"文".repeat(MAX_TEXT_UNITS),
                &insertion(&"文".repeat(MAX_TEXT_UNITS), "语音", "")
            )
            .is_none());
        assert!(first
            .extract(&"文".repeat(MAX_EDITOR_TEXT_UNITS + 1))
            .is_none());
    }
}
