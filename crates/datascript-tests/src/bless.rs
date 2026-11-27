//! `DS_BLESS=1`: rewrite expected sections in place.
//!
//! Only expected sections whose content no longer matches are rewritten; everything else in the
//! file (header, description, query, matching sections, line endings) stays byte for byte, so
//! blessing twice changes nothing the second time.

use crate::case::{Order, TestCase};
use crate::output::canonicalize_with_renders;

/// Returns the new file text after blessing `outputs` (one per engine, in the order of the
/// case's `engine:` header), or `None` when every expected section already matches.
///
/// The shared `---- expected` section holds the first engine's output. Another engine gets its
/// own `---- expected[<engine>]` section only when its output differs (TESTING.md, "Several
/// engines").
pub fn bless(
    text: &str,
    case: &TestCase,
    outputs: &[(String, String)],
    order: Order,
) -> Option<String> {
    bless_with_renders(text, case, outputs, order, None)
}

/// Like [`bless`], passing per-table `render` flags so `Order::Any` distinguishes chart footers
/// from data rows structurally.
pub fn bless_with_renders(
    text: &str,
    case: &TestCase,
    outputs: &[(String, String)],
    order: Order,
    table_renders: Option<&[bool]>,
) -> Option<String> {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let same = |a: &str, b: &str| {
        canonicalize_with_renders(a, order, table_renders)
            == canonicalize_with_renders(b, order, table_renders)
    };
    let shared = case.expected.iter().find(|s| s.engine.is_none());

    // The shared section: keep it if it matches the first engine, else replace it.
    let (_, first_output) = outputs.first()?;
    let new_shared: Option<&str> = match shared {
        Some(s) if same(&s.body, first_output) => None,
        _ => Some(first_output.as_str()),
    };
    let shared_text: &str = match (new_shared, shared) {
        (Some(new), _) => new,
        (None, Some(s)) => &s.body,
        (None, None) => unreachable!("a missing shared section is always rewritten"),
    };

    // Per-engine sections: `Keep`, `Remove`, or `Write(output)`.
    enum Action<'a> {
        Keep,
        Remove,
        Write(&'a str),
    }
    let mut engine_actions: Vec<(&str, Action<'_>)> = Vec::new();
    for (engine, output) in &outputs[1..] {
        let existing = case
            .expected
            .iter()
            .find(|s| s.engine.as_deref() == Some(engine.as_str()));
        let action = match existing {
            _ if same(shared_text, output) => match existing {
                Some(_) => Action::Remove,
                None => Action::Keep,
            },
            Some(s) if same(&s.body, output) => Action::Keep,
            _ => Action::Write(output),
        };
        engine_actions.push((engine, action));
    }
    // Sections for engines no longer in the header cannot exist: the parser rejects them.

    let changed = new_shared.is_some()
        || engine_actions
            .iter()
            .any(|(_, a)| !matches!(a, Action::Keep));
    if !changed {
        return None;
    }

    let body = |content: &str| -> String {
        let mut s = content
            .replace("\r\n", "\n")
            .trim_end_matches('\n')
            .replace('\n', eol);
        s.push_str(eol);
        s
    };

    // Rewrite in file order, then append sections that did not exist.
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut sections: Vec<_> = case.expected.iter().collect();
    sections.sort_by_key(|s| s.range.start);
    for section in sections {
        match &section.engine {
            None => {
                if let Some(new) = new_shared {
                    out.push_str(&text[cursor..section.body_range.start]);
                    out.push_str(&body(new));
                    cursor = section.body_range.end;
                }
            }
            Some(engine) => {
                let action = engine_actions
                    .iter()
                    .find(|(e, _)| e == engine)
                    .map(|(_, a)| a);
                match action {
                    Some(Action::Remove) => {
                        out.push_str(&text[cursor..section.range.start]);
                        cursor = section.range.end;
                    }
                    Some(Action::Write(new)) => {
                        out.push_str(&text[cursor..section.body_range.start]);
                        out.push_str(&body(new));
                        cursor = section.body_range.end;
                    }
                    Some(Action::Keep) | None => {}
                }
            }
        }
    }
    out.push_str(&text[cursor..]);

    let mut append = |header: &str, content: &str| {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(eol);
        }
        out.push_str(header);
        out.push_str(eol);
        out.push_str(&body(content));
    };
    if shared.is_none() {
        append("---- expected", shared_text);
    }
    for (engine, action) in &engine_actions {
        let exists = case
            .expected
            .iter()
            .any(|s| s.engine.as_deref() == Some(*engine));
        if let (Action::Write(new), false) = (action, exists) {
            append(&format!("---- expected[{engine}]"), new);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, outputs: &[(&str, &str)]) -> Option<String> {
        let case = TestCase::parse(text).unwrap();
        let outputs: Vec<(String, String)> = outputs
            .iter()
            .map(|(e, o)| (e.to_string(), o.to_string()))
            .collect();
        bless(text, &case, &outputs, Order::Strict)
    }

    /// Blessing the result again must change nothing.
    fn assert_idempotent(text: &str, outputs: &[(&str, &str)]) {
        assert_eq!(run(text, outputs), None, "second bless changed:\n{text}");
    }

    #[test]
    fn matching_output_leaves_file_untouched() {
        let text = "# d\nmode: run\n---- query\nq\n---- expected\nsame\n\n\n";
        assert_eq!(run(text, &[("datafusion", "same\n")]), None);
    }

    #[test]
    fn rewrites_only_the_expected_body() {
        let text = "# keep  \nmode: check\n\n---- query\n  q  \n\n---- expected\nold\n";
        let new = run(text, &[("datafusion", "new 1\nnew 2\n")]).unwrap();
        assert_eq!(
            new,
            "# keep  \nmode: check\n\n---- query\n  q  \n\n---- expected\nnew 1\nnew 2\n"
        );
        assert_idempotent(&new, &[("datafusion", "new 1\nnew 2\n")]);
    }

    #[test]
    fn keeps_crlf_and_following_sections() {
        let text = "engine: a, b\r\n---- query\r\nq\r\n---- expected\r\nold\r\n---- expected[b]\r\nbee\r\n";
        let new = run(text, &[("a", "new"), ("b", "bee")]).unwrap();
        assert_eq!(
            new,
            "engine: a, b\r\n---- query\r\nq\r\n---- expected\r\nnew\r\n---- expected[b]\r\nbee\r\n"
        );
        assert_idempotent(&new, &[("a", "new"), ("b", "bee")]);
    }

    #[test]
    fn adds_missing_sections() {
        let text = "engine: a, b\n---- query\nq";
        let new = run(text, &[("a", "x"), ("b", "y")]).unwrap();
        assert_eq!(
            new,
            "engine: a, b\n---- query\nq\n---- expected\nx\n---- expected[b]\ny\n"
        );
        assert_idempotent(&new, &[("a", "x"), ("b", "y")]);
    }

    #[test]
    fn removes_engine_section_once_outputs_agree() {
        let text = "engine: a, b\n---- query\nq\n---- expected[b]\ny\n---- expected\nx\n";
        let new = run(text, &[("a", "x"), ("b", "x")]).unwrap();
        assert_eq!(new, "engine: a, b\n---- query\nq\n---- expected\nx\n");
        assert_idempotent(&new, &[("a", "x"), ("b", "x")]);
    }

    #[test]
    fn any_order_does_not_rewrite_reordered_rows() {
        let text = "---- query\nq\n---- expected\nx\nlong\n-\n2\n1\n";
        let case = TestCase::parse(text).unwrap();
        let outputs = [("datafusion".to_string(), "x\nlong\n-\n1\n2\n".to_string())];
        assert_eq!(bless(text, &case, &outputs, Order::Any), None);
        assert!(bless(text, &case, &outputs, Order::Strict).is_some());
    }
}
