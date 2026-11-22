//! The lexer's tokens for the VS Code grammar's fixture files (M3-04).
//!
//! `editors/vscode/test/grammar/*.ds` are lexed here and their tokens written to `tokens.json`
//! next to them, one line per token: `[line, start, end, "KIND"]`, with 0-based lines and UTF-16
//! columns, the way the editor counts. Tokens that span lines are split per line. The extension's
//! grammar test (`npm run test:grammar`) checks that the TextMate grammar gives every token the
//! scope its kind calls for, so the grammar and the lexer cannot drift apart.
//!
//! This test fails when `tokens.json` is out of date (`DS_BLESS=1` rewrites it) or when the
//! fixtures miss a token kind, so a new kind cannot be added without a grammar test for it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use datascript_diagnostics::FileId;
use datascript_syntax::{SyntaxKind, lex};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../editors/vscode/test/grammar")
}

/// The fixture files, sorted by name.
fn fixtures() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(fixture_dir())
        .expect("the grammar fixtures exist")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ds"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no fixtures in {}",
        fixture_dir().display()
    );
    files
}

/// Each token of `text` as `(line, start, end, kind)` segments, one per line it touches.
/// Lines end at `\n`, `\r\n` or a lone `\r`, as in the LSP; line breaks belong to no segment.
fn segments(text: &str) -> Vec<(usize, usize, usize, SyntaxKind)> {
    let mut out = Vec::new();
    let mut line = 0;
    let mut column = 0; // UTF-16 units since the start of the line
    for token in lex(text, FileId(0)).tokens {
        let mut start = column;
        let mut chars = token.text(text).chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\n' || c == '\r' {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if column > start {
                    out.push((line, start, column, token.kind));
                }
                line += 1;
                column = 0;
                start = 0;
            } else {
                column += c.len_utf16();
            }
        }
        if column > start {
            out.push((line, start, column, token.kind));
        }
    }
    out
}

fn render(files: &[PathBuf]) -> (String, BTreeSet<SyntaxKind>) {
    let mut seen = BTreeSet::new();
    let mut json = String::from("{\n");
    for (i, path) in files.iter().enumerate() {
        let text = std::fs::read_to_string(path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy();
        writeln!(json, "  \"{name}\": [").unwrap();
        let segments = segments(&text);
        for (j, (line, start, end, kind)) in segments.iter().enumerate() {
            seen.insert(*kind);
            let comma = if j + 1 < segments.len() { "," } else { "" };
            writeln!(json, "    [{line}, {start}, {end}, \"{kind:?}\"]{comma}").unwrap();
        }
        let comma = if i + 1 < files.len() { "," } else { "" };
        writeln!(json, "  ]{comma}").unwrap();
    }
    json.push_str("}\n");
    (json, seen)
}

#[test]
fn grammar_fixture_tokens_are_up_to_date() {
    let (json, _) = render(&fixtures());
    let path = fixture_dir().join("tokens.json");
    if std::env::var_os("DS_BLESS").is_some() {
        std::fs::write(&path, &json).unwrap();
        return;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        current == json,
        "{} is out of date; rerun with DS_BLESS=1 and then `npm run test:grammar` in editors/vscode",
        path.display()
    );
}

#[test]
fn grammar_fixtures_cover_every_token_kind() {
    let (_, seen) = render(&fixtures());
    let missing: Vec<SyntaxKind> = SyntaxKind::ALL
        .iter()
        .copied()
        .take_while(|&kind| kind != SyntaxKind::PROGRAM)
        .filter(|&kind| kind != SyntaxKind::EOF && !seen.contains(&kind))
        .collect();
    assert!(
        missing.is_empty(),
        "no fixture in editors/vscode/test/grammar has a token of kind {missing:?}; add one, \
         with its scope in the grammar test's `EXPECTED` table"
    );
}

#[test]
fn segments_count_utf16_and_split_lines() {
    use SyntaxKind::*;
    assert_eq!(
        segments("😀/* a\r\nb */x"),
        [
            (0, 0, 2, ERROR),
            (0, 2, 6, BLOCK_COMMENT),
            (1, 0, 4, BLOCK_COMMENT),
            (1, 4, 5, IDENT)
        ]
    );
}
