//! Applying machine-applicable suggestions, so golden tests can check that a fix really fixes
//! the program (M1-03).

use datascript_diagnostics::{Applicability, Diagnostic, FileId, TextEdit};

/// Applies the machine-applicable suggestions of `diagnostics` that edit file `file` to `text`.
///
/// Suggestions are taken in order. One whose edits overlap an edit already taken is skipped as a
/// whole, so the result never mixes half of a fix. Returns `None` if there was nothing to apply.
pub fn apply_suggestions(text: &str, file: FileId, diagnostics: &[Diagnostic]) -> Option<String> {
    let mut taken: Vec<&TextEdit> = Vec::new();
    let suggestions = diagnostics
        .iter()
        .flat_map(|d| &d.help)
        .filter_map(|help| help.suggestion.as_ref())
        .filter(|s| s.applicability == Applicability::MachineApplicable);
    for suggestion in suggestions {
        let edits: Vec<&TextEdit> = suggestion
            .edits
            .iter()
            .filter(|e| e.span.file == file)
            .collect();
        let overlaps = edits
            .iter()
            .any(|new| taken.iter().any(|old| overlap(old, new)));
        if !overlaps {
            taken.extend(edits);
        }
    }
    if taken.is_empty() {
        return None;
    }
    // Apply from the end so earlier offsets stay valid. Insertions at the same offset keep the
    // order they were taken in.
    let mut ordered: Vec<(usize, &TextEdit)> = taken.into_iter().enumerate().collect();
    ordered.sort_by_key(|(i, e)| (std::cmp::Reverse(e.span.start), std::cmp::Reverse(*i)));
    let mut out = text.to_string();
    for (_, edit) in ordered {
        out.replace_range(
            edit.span.start as usize..edit.span.end as usize,
            &edit.replacement,
        );
    }
    Some(out)
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
}
