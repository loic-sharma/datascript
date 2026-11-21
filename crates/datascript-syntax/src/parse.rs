//! The entry point: [`parse`] and its result, [`Parse`].

use datascript_diagnostics::{Diagnostic, FileId};
use rowan::GreenNode;

use crate::ast::{AstNode, Program};
use crate::syntax_tree::{SyntaxNode, debug_tree};
use crate::{lex, parser};

/// The result of [`parse`]: a lossless syntax tree, always, plus every lexical and syntax error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse {
    /// The immutable tree. Its text is exactly the parsed text.
    pub green: GreenNode,
    /// Lexical (DS0001–DS0019) and syntax (DS0100–DS0149) errors, in source order.
    pub errors: Vec<Diagnostic>,
}

impl Parse {
    /// The root of the tree, a `PROGRAM` node.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// The typed view of the root.
    pub fn program(&self) -> Program {
        Program::cast(self.syntax()).expect("the root is always a PROGRAM node")
    }

    /// The tree dump used by `mode: syntax` golden tests (see [`debug_tree`]).
    pub fn debug_tree(&self) -> String {
        debug_tree(&self.syntax())
    }
}

/// Parses `text`, the contents of file `file`. Diagnostic spans point into that file.
///
/// Parsing never fails: the tree covers the whole input, with `ERROR` nodes around what did not
/// fit the grammar, and `errors` says what went wrong.
///
/// ```
/// use datascript_diagnostics::FileId;
/// use datascript_syntax::ast::{Operator, Stmt};
/// use datascript_syntax::parse;
///
/// let parse = parse("from 'orders.csv' | where amount > 10 | take 5", FileId(0));
/// assert!(parse.errors.is_empty());
/// let Some(Stmt::Query(query)) = parse.program().statements().next() else { panic!() };
/// let ops: Vec<Operator> = query.operators().collect();
/// assert!(matches!(ops[..], [Operator::Where(_), Operator::Take(_)]));
/// ```
pub fn parse(text: &str, file: FileId) -> Parse {
    let lexed = lex(text, file);
    let (green, parse_errors) = parser::parse_tokens(text, &lexed.tokens, file);
    let mut errors = lexed.diagnostics;
    errors.extend(parse_errors);
    errors.sort_by_key(|d| d.primary.as_ref().map(|l| (l.span.start, l.span.end)));
    Parse { green, errors }
}
