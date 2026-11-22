//! Open documents and incremental text changes.

use tower_lsp_server::ls_types::{Position, TextDocumentContentChangeEvent};

/// The byte offset of an LSP position in `text`.
///
/// Lines end at `\n`, `\r\n` or a lone `\r` (as in `SourceFile`); characters are UTF-16 code
/// units. Following the LSP, a character past the end of its line means the end of the line, and
/// a line past the end of the text means the end of the text. A position inside a surrogate pair
/// moves back to the start of the character.
pub(crate) fn offset_at(text: &str, position: Position) -> usize {
    let bytes = text.as_bytes();
    let mut line_start = 0;
    for _ in 0..position.line {
        let Some(rel) = bytes[line_start..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
        else {
            return text.len();
        };
        let end = line_start + rel;
        line_start = if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
            end + 2
        } else {
            end + 1
        };
    }
    let mut units = 0u32;
    for (i, c) in text[line_start..].char_indices() {
        if c == '\n' || c == '\r' {
            return line_start + i;
        }
        units += c.len_utf16() as u32;
        if units > position.character {
            return line_start + i;
        }
    }
    text.len()
}

/// Applies the changes of one `textDocument/didChange` notification, in order.
///
/// A change without a range replaces the whole text.
pub(crate) fn apply_changes(text: &mut String, changes: Vec<TextDocumentContentChangeEvent>) {
    for change in changes {
        match change.range {
            Some(range) => {
                let start = offset_at(text, range.start);
                let end = offset_at(text, range.end).max(start);
                text.replace_range(start..end, &change.text);
            }
            None => *text = change.text,
        }
    }
}

#[cfg(test)]
mod tests {
    use tower_lsp_server::ls_types::Range;

    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn offsets_count_utf16_and_every_line_break() {
        let text = "a😀b\r\ncd\ref\n";
        assert_eq!(offset_at(text, pos(0, 0)), 0);
        assert_eq!(offset_at(text, pos(0, 1)), 1);
        assert_eq!(offset_at(text, pos(0, 2)), 1); // inside the surrogate pair
        assert_eq!(offset_at(text, pos(0, 3)), 5);
        assert_eq!(offset_at(text, pos(0, 9)), 6); // past the end of the line
        assert_eq!(offset_at(text, pos(1, 0)), 8);
        assert_eq!(offset_at(text, pos(2, 1)), 12); // after a lone `\r`
        assert_eq!(offset_at(text, pos(3, 0)), text.len());
        assert_eq!(offset_at(text, pos(7, 3)), text.len());
    }

    #[test]
    fn incremental_and_full_changes() {
        let mut text = String::from("from t\n| take 5\n");
        let edit = |start, end, new: &str| TextDocumentContentChangeEvent {
            range: Some(Range { start, end }),
            range_length: None,
            text: new.into(),
        };
        apply_changes(
            &mut text,
            vec![
                edit(pos(1, 7), pos(1, 8), "10"),
                edit(pos(0, 5), pos(0, 6), "orders"),
            ],
        );
        assert_eq!(text, "from orders\n| take 10\n");
        apply_changes(&mut text, vec![edit(pos(2, 0), pos(2, 0), "| count")]);
        assert_eq!(text, "from orders\n| take 10\n| count");
        apply_changes(
            &mut text,
            vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "x".into(),
            }],
        );
        assert_eq!(text, "x");
    }
}
