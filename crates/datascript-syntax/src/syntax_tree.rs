//! The rowan tree types for DataScript and the CST dump used by `mode: syntax` golden tests.

use std::fmt::Write as _;

use crate::SyntaxKind;

/// The rowan `Language` for DataScript trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataScriptLanguage {}

impl rowan::Language for DataScriptLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        // Raw kinds only ever come from `kind_to_raw`, so this cannot fail on a tree we built.
        SyntaxKind::from_raw(raw.0).unwrap_or(SyntaxKind::ERROR)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind.into())
    }
}

/// A node in the lossless syntax tree.
pub type SyntaxNode = rowan::SyntaxNode<DataScriptLanguage>;
/// A token in the lossless syntax tree, trivia included.
pub type SyntaxToken = rowan::SyntaxToken<DataScriptLanguage>;
/// A node or a token.
pub type SyntaxElement = rowan::SyntaxElement<DataScriptLanguage>;

/// Dumps a tree, one element per line, children indented by two spaces:
///
/// ```text
/// PROGRAM@0..9
///   PIPELINE@0..9
///     NAME_REF@0..1
///       IDENT@0..1 "t"
///     WHITESPACE@1..2 " "
/// ```
///
/// Tokens show their text as a Rust string literal. This is the `mode: syntax` golden format.
pub fn debug_tree(node: &SyntaxNode) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for event in node.preorder_with_tokens() {
        match event {
            rowan::WalkEvent::Enter(element) => {
                let range = element.text_range();
                let indent = "  ".repeat(depth);
                let start = u32::from(range.start());
                let end = u32::from(range.end());
                match element {
                    rowan::NodeOrToken::Node(node) => {
                        let _ = writeln!(out, "{indent}{:?}@{start}..{end}", node.kind());
                        depth += 1;
                    }
                    rowan::NodeOrToken::Token(token) => {
                        let _ = writeln!(
                            out,
                            "{indent}{:?}@{start}..{end} {:?}",
                            token.kind(),
                            token.text()
                        );
                    }
                }
            }
            rowan::WalkEvent::Leave(rowan::NodeOrToken::Node(_)) => depth -= 1,
            rowan::WalkEvent::Leave(rowan::NodeOrToken::Token(_)) => {}
        }
    }
    out
}
