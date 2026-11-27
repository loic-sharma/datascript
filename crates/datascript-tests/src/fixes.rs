//! Applying machine-applicable suggestions, so golden tests can check that a fix really fixes
//! the program (M1-03, M2-30).

use std::ops::Range;

use datascript_diagnostics::{Applicability, Code, Diagnostic, FileId, TextEdit};

/// The result of applying machine-applicable suggestions to a query, together with where each
/// applied fix landed in the rewritten text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppliedFixes {
    /// The query text after all non-overlapping machine-applicable suggestions were applied.
    pub(crate) text: String,
    /// One entry per diagnostic whose machine-applicable suggestion was applied.
    pub(crate) applied: Vec<AppliedFix>,
    /// Indices into the `diagnostics` slice passed to [`apply_suggestions_detailed`] whose
    /// suggestion was actually applied (not skipped due to overlap).
    pub(crate) applied_indices: Vec<usize>,
    mapped_edits: Vec<MappedEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MappedEdit {
    orig_start: u32,
    orig_end: u32,
    new_start: u32,
    new_end: u32,
    fix_index: usize,
}

/// One diagnostic whose machine-applicable suggestion was applied, and the byte ranges it maps to
/// in the fixed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppliedFix {
    pub(crate) code: Code,
    pub(crate) had_primary: bool,
    pub(crate) fixed_ranges: Vec<Range<u32>>,
}

impl AppliedFixes {
    /// Maps a byte range in the original text to its corresponding byte range in `self.text`.
    pub(crate) fn map_range(&self, orig: Range<u32>) -> Range<u32> {
        map_range_with_edits(&self.mapped_edits, orig)
    }

    /// Whether `diagnostic` (from re-checking the fixed query) still has the same code at one of
    /// the fixed locations.
    pub(crate) fn matches_remaining(&self, fixed_file: FileId, diagnostic: &Diagnostic) -> bool {
        self.applied.iter().any(|fix| {
            if fix.code != diagnostic.code {
                return false;
            }
            match &diagnostic.primary {
                None => !fix.had_primary,
                Some(primary) if primary.span.file == fixed_file => {
                    let span = primary.span.start..primary.span.end;
                    fix.fixed_ranges
                        .iter()
                        .any(|r| ranges_overlap(r.clone(), span.clone()))
                }
                Some(_) => false,
            }
        })
    }
}

fn map_range_with_edits(mapped_edits: &[MappedEdit], orig: Range<u32>) -> Range<u32> {
    let mut start_delta: i64 = 0;
    let mut end_delta: i64 = 0;
    let mut start_override: Option<u32> = None;
    let mut end_override: Option<u32> = None;

    for e in mapped_edits {
        let delta = (e.new_end - e.new_start) as i64 - (e.orig_end - e.orig_start) as i64;
        if e.orig_start < e.orig_end && e.orig_start <= orig.start && orig.start < e.orig_end {
            start_override = Some(e.new_start);
        } else if e.orig_end <= orig.start {
            start_delta += delta;
        }

        if e.orig_start < e.orig_end && e.orig_start < orig.end && orig.end <= e.orig_end {
            end_override = Some(e.new_end);
        } else if e.orig_end < orig.end || (e.orig_end == orig.end && e.orig_start < e.orig_end) {
            end_delta += delta;
        }
    }

    let new_start = start_override.unwrap_or_else(|| (orig.start as i64 + start_delta) as u32);
    let new_end = end_override
        .unwrap_or_else(|| (orig.end as i64 + end_delta) as u32)
        .max(new_start);
    new_start..new_end
}

/// Applies the machine-applicable suggestions of `diagnostics` that edit file `file` to `text`.
///
/// Suggestions are taken in order. One whose edits overlap an edit already taken is skipped as a
/// whole, so the result never mixes half of a fix. Returns `None` if there was nothing to apply.
pub fn apply_suggestions(text: &str, file: FileId, diagnostics: &[Diagnostic]) -> Option<String> {
    apply_suggestions_detailed(text, file, diagnostics).map(|a| a.text)
}

/// Like [`apply_suggestions`], also returning the mapped byte ranges of every applied fix so the
/// caller can verify that re-checking the fixed text resolved those diagnostics.
pub(crate) fn apply_suggestions_detailed(
    text: &str,
    file: FileId,
    diagnostics: &[Diagnostic],
) -> Option<AppliedFixes> {
    struct TakenEdit<'a> {
        edit: &'a TextEdit,
        fix_index: usize,
        order: usize,
    }
    struct RawFix {
        code: Code,
        primary: Option<Range<u32>>,
    }

    let mut taken: Vec<TakenEdit<'_>> = Vec::new();
    let mut raw_fixes: Vec<RawFix> = Vec::new();
    let mut applied_indices: Vec<usize> = Vec::new();

    for (diag_index, diagnostic) in diagnostics.iter().enumerate() {
        let mut applied_this = false;
        let suggestions = diagnostic
            .help
            .iter()
            .filter_map(|help| help.suggestion.as_ref())
            .filter(|s| s.applicability == Applicability::MachineApplicable);
        for suggestion in suggestions {
            let edits: Vec<&TextEdit> = suggestion
                .edits
                .iter()
                .filter(|e| e.span.file == file)
                .collect();
            if edits.is_empty() {
                continue;
            }
            let overlaps = edits
                .iter()
                .any(|new| taken.iter().any(|old| overlap(old.edit, new)));
            if !overlaps {
                applied_this = true;
                let fix_index = raw_fixes.len();
                let primary = diagnostic
                    .primary
                    .as_ref()
                    .filter(|p| p.span.file == file)
                    .map(|p| p.span.start..p.span.end);
                raw_fixes.push(RawFix {
                    code: diagnostic.code,
                    primary,
                });
                for edit in edits {
                    let order = taken.len();
                    taken.push(TakenEdit {
                        edit,
                        fix_index,
                        order,
                    });
                }
            }
        }
        if applied_this {
            applied_indices.push(diag_index);
        }
    }
    if taken.is_empty() {
        return None;
    }

    // Apply in forward offset order. At one offset, an insertion (`start == end`) goes before a
    // replacement (`start < end`), so the insertion lands before the replaced text (`!b&&c`: the
    // `)` closing `not(b` goes before ` and`), whatever order the diagnostics came in.
    taken.sort_by_key(|t| (t.edit.span.start, t.edit.span.end, t.order));

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut mapped_edits: Vec<MappedEdit> = Vec::with_capacity(taken.len());

    for t in &taken {
        let orig_start = t.edit.span.start as usize;
        let orig_end = t.edit.span.end as usize;
        out.push_str(&text[cursor..orig_start]);
        let new_start = out.len() as u32;
        out.push_str(&t.edit.replacement);
        let new_end = out.len() as u32;
        cursor = orig_end;
        mapped_edits.push(MappedEdit {
            orig_start: t.edit.span.start,
            orig_end: t.edit.span.end,
            new_start,
            new_end,
            fix_index: t.fix_index,
        });
    }
    out.push_str(&text[cursor..]);

    let mut applied: Vec<AppliedFix> = raw_fixes
        .into_iter()
        .map(|raw| {
            let mut fixed_ranges = Vec::new();
            if let Some(primary) = raw.primary.clone() {
                fixed_ranges.push(map_range_with_edits(&mapped_edits, primary));
            }
            AppliedFix {
                code: raw.code,
                had_primary: raw.primary.is_some(),
                fixed_ranges,
            }
        })
        .collect();

    for e in &mapped_edits {
        applied[e.fix_index]
            .fixed_ranges
            .push(e.new_start..e.new_end);
    }

    Some(AppliedFixes {
        text: out,
        applied,
        applied_indices,
        mapped_edits,
    })
}

/// Whether two edits touch the same text. Two insertions at one offset do overlap: their order
/// would be ambiguous.
fn overlap(a: &TextEdit, b: &TextEdit) -> bool {
    let (a, b) = (a.span, b.span);
    if a.start == a.end && b.start == b.end {
        return a.start == b.start;
    }
    a.start < b.end && b.start < a.end
}

fn ranges_overlap(fixed: Range<u32>, diag: Range<u32>) -> bool {
    if fixed.start == fixed.end {
        // An empty fixed range (e.g. a deletion) only matches an empty diagnostic at the exact
        // same offset or a diagnostic strictly spanning across it, never an adjacent diagnostic
        // that merely starts or ends at the deletion point.
        diag == fixed || (diag.start < fixed.start && fixed.start < diag.end)
    } else if diag.start == diag.end {
        fixed.start < diag.start && diag.start < fixed.end
    } else {
        fixed.start < diag.end && diag.start < fixed.end
    }
}

#[cfg(test)]
mod tests {
    use datascript_diagnostics::{Diagnostic, Span, Suggestion, codes};

    use super::*;

    const F: FileId = FileId(0);

    fn fix(start: u32, end: u32, text: &str) -> Diagnostic {
        Diagnostic::error(codes::DS0110, "x")
            .help_with("fix", Suggestion::replace(Span::new(F, start, end), text))
    }

    #[test]
    fn applies_in_any_order() {
        let text = "a = 1 and b <> 2";
        let ds = [fix(12, 14, "!="), fix(2, 3, "==")];
        assert_eq!(
            apply_suggestions(text, F, &ds).as_deref(),
            Some("a == 1 and b != 2")
        );
    }

    #[test]
    fn skips_overlapping_and_non_machine_applicable() {
        let maybe = Diagnostic::error(codes::DS0103, "x").help_with(
            "maybe",
            Suggestion::new(
                vec![TextEdit::new(Span::new(F, 0, 1), "z")],
                Applicability::MaybeIncorrect,
            ),
        );
        let ds = [fix(0, 3, "xyz"), fix(2, 4, "!!"), maybe];
        assert_eq!(apply_suggestions("abcd", F, &ds).as_deref(), Some("xyzd"));
        assert_eq!(apply_suggestions("abcd", F, &[]), None);
    }

    #[test]
    fn insertion_goes_before_replacement_at_same_offset() {
        let text = "!b&&c";
        let insert = Diagnostic::error(codes::DS0111, "x")
            .help_with("fix", Suggestion::replace(Span::empty(F, 2), ")"));
        for ds in [
            [fix(2, 4, " and "), insert.clone()],
            [insert.clone(), fix(2, 4, " and ")],
        ] {
            assert_eq!(
                apply_suggestions(text, F, &ds).as_deref(),
                Some("!b) and c")
            );
        }
    }

    #[test]
    fn multi_edit_suggestion_moves_text() {
        let text = "t | render x | take 1";
        let d = Diagnostic::error(codes::DS0120, "x").help_with(
            "move",
            Suggestion::new(
                vec![
                    TextEdit::new(Span::new(F, 1, 12), ""),
                    TextEdit::new(Span::empty(F, 21), " | render x"),
                ],
                Applicability::MachineApplicable,
            ),
        );
        assert_eq!(
            apply_suggestions(text, F, &[d]).as_deref(),
            Some("t | take 1 | render x")
        );
    }

    #[test]
    fn maps_fixed_locations_across_insertions_and_replacements() {
        // Two errors in one query: `amout` replaced with `amount` (+1 byte), and a second
        // unfixed error `zzzzz` later in the query with the same code DS1001.
        let text = "from t | project amout, zzzzz";
        let d_fix = Diagnostic::error(codes::DS1001, "unknown column")
            .primary(Span::new(F, 17, 22), "unknown")
            .help_with("fix", Suggestion::replace(Span::new(F, 17, 22), "amount"));
        let applied = apply_suggestions_detailed(text, F, &[d_fix]).unwrap();
        assert_eq!(applied.text, "from t | project amount, zzzzz");

        let fixed_file = FileId(1);
        // Same code at the fixed span (`amount`: 17..23) still matches.
        let still_broken = Diagnostic::error(codes::DS1001, "still unknown")
            .primary(Span::new(fixed_file, 17, 23), "unknown");
        assert!(applied.matches_remaining(fixed_file, &still_broken));

        // Same code at a different span (`zzzzz`: 25..30) does NOT match.
        let other_error = Diagnostic::error(codes::DS1001, "other unknown")
            .primary(Span::new(fixed_file, 25, 30), "unknown");
        assert!(!applied.matches_remaining(fixed_file, &other_error));

        // Different code at the fixed span does NOT match.
        let different_code = Diagnostic::error(codes::DS2003, "type mismatch")
            .primary(Span::new(fixed_file, 17, 23), "wrong type");
        assert!(!applied.matches_remaining(fixed_file, &different_code));
    }

    #[test]
    fn maps_insertion_fixes_preserving_primary_span() {
        // Like DS2103 (`sort by a`): primary is `a` (17..18), fix inserts ` asc` at 18..18.
        let text = "from t | sort by a";
        let d = Diagnostic::warning(codes::DS2103, "missing direction")
            .primary(Span::new(F, 17, 18), "no direction")
            .help_with("fix", Suggestion::replace(Span::empty(F, 18), " asc"));
        let applied = apply_suggestions_detailed(text, F, &[d]).unwrap();
        assert_eq!(applied.text, "from t | sort by a asc");

        let fixed_file = FileId(1);
        // If DS2103 still fired on `a` (17..18), it is caught.
        let still_missing = Diagnostic::warning(codes::DS2103, "missing direction")
            .primary(Span::new(fixed_file, 17, 18), "no direction");
        assert!(applied.matches_remaining(fixed_file, &still_missing));
    }

    #[test]
    fn deletion_fix_does_not_match_adjacent_diagnostic() {
        // Delete `from ` (0..5 -> "") right before `badcol` (5..11), which shifts to 0..6.
        // An unrelated same-code diagnostic at 0..6 right next to the deletion point 0..0
        // must NOT match the deleted fix.
        let text = "from badcol";
        let deleted = Diagnostic::error(codes::DS0152, "unexpected `from`")
            .primary(Span::new(F, 0, 5), "remove this")
            .help_with("remove", Suggestion::replace(Span::new(F, 0, 5), ""));
        let applied = apply_suggestions_detailed(text, F, &[deleted]).unwrap();
        assert_eq!(applied.text, "badcol");

        let fixed_file = FileId(1);
        let adjacent = Diagnostic::error(codes::DS0152, "another error right next to it")
            .primary(Span::new(fixed_file, 0, 6), "adjacent");
        assert!(
            !applied.matches_remaining(fixed_file, &adjacent),
            "empty deletion range must not match an adjacent non-empty diagnostic"
        );
    }
}
