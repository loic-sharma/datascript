//! When Enter submits: the REPL's submit rule (D-048), judged with the lexer.

use datascript_diagnostics::FileId;
use datascript_diagnostics::codes::{DS0002, DS0006};
use datascript_syntax::{SyntaxKind, lex};

/// Whether pressing Enter at the end of `buffer` submits it, rather than adding a line.
///
/// It submits when:
/// - the input is empty or blank (the loop then ignores it),
/// - the input is a meta command (it starts with `.`),
/// - the current (last) line is blank, even if the input is incomplete, so the user gets the
///   parser's diagnostic instead of being stuck, or
/// - the input is [complete](is_complete) and its last token is `;`.
pub fn submits(buffer: &str) -> bool {
    let trimmed = buffer.trim();
    if trimmed.is_empty() || is_meta(buffer) {
        return true;
    }
    let current_line = buffer.rsplit('\n').next().unwrap_or_default();
    if current_line.trim().is_empty() {
        return true;
    }
    is_complete(buffer) && ends_with_semicolon(buffer)
}

/// Whether the input is a meta command: its first non-blank character is `.`.
pub fn is_meta(buffer: &str) -> bool {
    buffer.trim_start().starts_with('.')
}

/// Whether the input could stand as it is: its brackets are balanced and it has no unterminated
/// string or block comment. Brackets inside strings and comments don't count: the lexer has
/// already made them part of those tokens. More closing than opening brackets is complete too:
/// another line cannot fix it, so the parser should report it.
pub fn is_complete(buffer: &str) -> bool {
    let lexed = lex(buffer, FileId(0));
    if lexed
        .diagnostics
        .iter()
        .any(|d| d.code == DS0002 || d.code == DS0006)
    {
        return false;
    }
    let mut depth: i64 = 0;
    for token in &lexed.tokens {
        match token.kind {
            SyntaxKind::L_PAREN | SyntaxKind::L_BRACK | SyntaxKind::L_BRACE => depth += 1,
            SyntaxKind::R_PAREN | SyntaxKind::R_BRACK | SyntaxKind::R_BRACE => depth -= 1,
            _ => {}
        }
    }
    depth <= 0
}

/// Whether the last token that is not white space or a comment is `;`.
fn ends_with_semicolon(buffer: &str) -> bool {
    lex(buffer, FileId(0))
        .tokens
        .iter()
        .rev()
        .find(|t| !t.kind.is_trivia())
        .is_some_and(|t| t.kind == SyntaxKind::SEMICOLON)
}

/// Splits piped input into the inputs the REPL would have submitted, line by line, with the same
/// rule as Enter. Blank inputs are dropped; what is left at the end of the input is submitted.
pub fn split_inputs(text: &str) -> Vec<String> {
    let mut inputs = Vec::new();
    let mut buffer = String::new();
    for line in text.lines() {
        if !buffer.is_empty() {
            buffer.push('\n');
        }
        buffer.push_str(line);
        if submits(&buffer) {
            if !buffer.trim().is_empty() {
                inputs.push(std::mem::take(&mut buffer));
            }
            buffer.clear();
        }
    }
    if !buffer.trim().is_empty() {
        inputs.push(buffer);
    }
    inputs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_query_waits_for_a_semicolon_or_a_blank_line() {
        // The usual style puts `| where` at the start of the next line.
        assert!(!submits("from t | take 5"));
        assert!(submits("from t | take 5;"));
        assert!(submits("from t | take 5; // done"));
        assert!(!submits("from t\n| where x > 1"));
        assert!(submits("from t\n| where x > 1\n"));
        assert!(submits("from t\n| where x > 1\n   "));
    }

    #[test]
    fn semicolons_in_strings_and_comments_do_not_submit() {
        assert!(!submits("from t | where s == ';"));
        assert!(!submits("from t | where s == 'a;b'"));
        assert!(!submits("from t // the end;"));
        assert!(!submits("from t /* ; */"));
        assert!(!submits("from t /* ;"));
        assert!(!submits("from t /* still open;\n*/ | take 1"));
        assert!(submits("from t /* closed */ | take 1;"));
    }

    #[test]
    fn unbalanced_brackets_wait_for_more() {
        assert!(!submits("from t | where x in (1, 2;"));
        assert!(!submits("from t | extend y = f(x;"));
        assert!(!submits("from t | where s == '(' and x in [1;"));
        assert!(submits("from t | where s == '(' and x == 1;"));
        // Too many closing brackets: another line cannot help, so the parser reports it.
        assert!(submits("from t | where x == 1);"));
    }

    #[test]
    fn a_blank_line_submits_incomplete_input() {
        assert!(submits("from t | where x in (1,\n"));
        assert!(submits("from t | where s == 'open\n"));
    }

    #[test]
    fn meta_commands_and_empty_input_submit_at_once() {
        assert!(submits(".help"));
        assert!(submits("  .schema 'orders.csv'"));
        assert!(submits(""));
        assert!(submits("   "));
    }

    #[test]
    fn piped_input_splits_like_the_editor() {
        let text = "let a = 1;\nfrom t\n| take a\n\n.tables\nfrom u; from v;\nfrom w";
        assert_eq!(
            split_inputs(text),
            [
                "let a = 1;",
                "from t\n| take a\n",
                ".tables",
                "from u; from v;",
                "from w"
            ]
        );
        assert!(split_inputs("\n\n  \n").is_empty());
    }
}
