//! Editor features over an [`Analysis`] and syntax tree: [`complete`], [`hover`],
//! [`signature_help`], [`goto_definition`], [`references`], [`prepare_rename`], [`rename`],
//! [`document_symbols`] and [`semantic_tokens`].
//!
//! They live in the compiler so the language server, the `ds ui` server and WASM give the same
//! answers for the same text (ARCHITECTURE.md § Semantics). Features that need schemas
//! ([`complete`], [`hover`], [`signature_help`]) take the checked text and its [`Analysis`].
//! Features that work in syntax-only mode (D-053: `let` navigation and rename, document symbols,
//! semantic tokens) take an optional [`Analysis`]: with `None` they work from the parse alone;
//! with `Some` they also use bound types and know when a column shadows a `let` (D-038).
//! Offsets and ranges are byte offsets into the text; frontends convert them.
//!
//! None of them reads a file. File names inside path strings are the frontend's job:
//! [`Completions::files`] says when and what to list.

use std::ops::Range;

use arrow_schema::SchemaRef;
use datascript_diagnostics::FileId;
use datascript_syntax::ast::{self, AstNode};
use datascript_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::Analysis;
use crate::arrow::from_arrow;
use crate::hir::{self, ExprKind, LetValue, Statement};
use crate::ir::Value;

mod complete;
mod hover;
mod lets;
mod semantic_tokens;
mod signature;
mod symbols;

pub use complete::{Completion, CompletionKind, Completions, FileCompletion, complete};
pub use hover::{Hover, hover};
pub use lets::{
    RenameEdit, RenameError, RenameTarget, goto_definition, prepare_rename, references, rename,
};
pub use semantic_tokens::{
    SemanticToken, SemanticTokenKind, SemanticTokenModifiers, semantic_tokens,
};
pub use signature::{SignatureHelp, SignatureInfo, signature_help};
pub use symbols::{DocumentSymbol, SymbolKind, document_symbols};

/// What each operator does, in one line (completion details and hover).
pub fn operator_doc(name: &str) -> Option<&'static str> {
    Some(match name {
        "where" => "Keeps the rows where the predicate is true.",
        "project" => "Picks, renames and computes columns, in the order written.",
        "project-away" => "Removes columns; `*` wildcards are allowed.",
        "project-rename" => "Renames columns, keeping their position.",
        "extend" => "Adds computed columns, or replaces columns in place.",
        "summarize" => "Groups rows by keys and computes aggregates for each group.",
        "sort" | "order" => "Sorts rows by keys, ascending unless `desc`.",
        "top" => "The first n rows by a key, descending unless `asc`.",
        "take" | "limit" => "At most n rows, in no particular order unless sorted.",
        "count" => "One row with the number of rows, in column `Count`.",
        "distinct" => "The distinct combinations of the given columns.",
        "join" => "Joins the rows with another table on key columns.",
        "union" => "Appends the rows of other tables, matching columns by name.",
        "render" => "Draws the result as a chart. It must be the last operator.",
        "getschema" => "One row per input column: its name, position and type.",
        _ => return None,
    })
}

/// A column as editors show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Col {
    pub name: String,
    /// The DataScript type, or the Arrow type of a column that has none.
    pub ty: String,
}

/// A `let` defined before the cursor.
#[derive(Debug, Clone)]
pub(crate) struct LetInfo {
    pub name: String,
    pub kind: LetKind,
}

#[derive(Debug, Clone)]
pub(crate) enum LetKind {
    /// A scalar `let`, its type, and its value when it is a string literal (a path, D-020).
    Scalar { ty: String, string: Option<String> },
    /// A tabular `let`, and its columns when known.
    Table(Option<Vec<Col>>),
    /// A `let` whose value has an error.
    Unknown,
}

/// The parsed text and its analysis, with the non-trivia tokens in order.
pub(crate) struct Doc<'a> {
    pub analysis: &'a Analysis,
    pub root: SyntaxNode,
    pub tokens: Vec<SyntaxToken>,
}

impl<'a> Doc<'a> {
    pub fn new(text: &'a str, analysis: &'a Analysis) -> Self {
        let parse = datascript_syntax::parse(text, FileId(0));
        let root = parse.syntax();
        let tokens = root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia())
            .collect();
        Doc {
            analysis,
            root,
            tokens,
        }
    }

    /// Whether `offset` is inside a comment, or at the end of a line comment or of an
    /// unterminated block comment.
    pub fn in_comment(&self, offset: usize) -> bool {
        self.root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| {
                let r = range(&t);
                let comment = matches!(
                    t.kind(),
                    SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT
                );
                let open_end = t.kind() == SyntaxKind::LINE_COMMENT
                    || (t.kind() == SyntaxKind::BLOCK_COMMENT && !t.text().ends_with("*/"));
                comment && r.start < offset && (offset < r.end || (open_end && offset == r.end))
            })
    }

    /// The index of the last non-trivia token that starts before `offset`.
    pub fn left_index(&self, offset: usize) -> Option<usize> {
        let n = self
            .tokens
            .partition_point(|t| usize::from(t.text_range().start()) < offset);
        n.checked_sub(1)
    }

    /// The token at `offset` for hover: the one containing it, or else the one ending at it.
    pub fn token_at(&self, offset: usize) -> Option<&SyntaxToken> {
        token_at(&self.tokens, offset)
    }

    /// The columns after `node`, a pipeline stage (the source or an operator), a pipeline, or a
    /// parenthesized pipeline.
    pub fn output_of(&self, node: &SyntaxNode) -> Option<Vec<Col>> {
        if let Some(schema) = self.schema_at(range_of(node)) {
            return Some(columns(&schema));
        }
        match node.kind() {
            SyntaxKind::PAREN_EXPR => {
                let inner = node.children().find(|c| c.kind() != SyntaxKind::ERROR)?;
                self.output_of(&inner)
            }
            SyntaxKind::PIPELINE => {
                let last = node
                    .children()
                    .filter(|c| c.kind() != SyntaxKind::ERROR)
                    .last()?;
                self.output_of(&last)
            }
            _ => None,
        }
    }

    /// The columns an operator reads: the output of the stage before it.
    pub fn input_of(&self, op: &SyntaxNode) -> Option<Vec<Col>> {
        let mut prev = op.prev_sibling();
        while let Some(p) = &prev {
            if p.kind() != SyntaxKind::ERROR {
                break;
            }
            prev = p.prev_sibling();
        }
        self.output_of(&prev?)
    }

    /// The columns in scope at byte `offset` inside `op`. For `extend`, this includes columns
    /// made or replaced by items before the comma preceding `offset` (D-040).
    pub fn columns_at(&self, op: &SyntaxNode, offset: usize) -> Option<Vec<Col>> {
        let cols = self.input_of(op)?;
        if op.kind() != SyntaxKind::EXTEND_OP {
            return Some(cols);
        }
        // Only items before the last `,` preceding `offset` are complete prior items.
        let Some(last_comma) = op
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::COMMA && range(t).start < offset)
            .map(|t| range(&t).start)
            .max()
        else {
            return Some(cols);
        };
        let Some(extend) = ast::ExtendOp::cast(op.clone()) else {
            return Some(cols);
        };
        let last_prior = extend
            .items()
            .take_while(|item| range_of(item.syntax()).end <= last_comma)
            .filter_map(|item| self.schema_at(range_of(item.syntax())))
            .last();
        Some(last_prior.map_or(cols, |schema| columns(&schema)))
    }

    /// The columns of the right table of a `join` operator.
    pub fn join_right_of(&self, op: &SyntaxNode) -> Option<Vec<Col>> {
        let join = ast::JoinOp::cast(op.clone())?;
        let right = join.right()?;
        self.output_of(right.syntax())
    }

    fn schema_at(&self, r: Range<usize>) -> Option<SchemaRef> {
        self.analysis
            .stage_schemas
            .iter()
            .find(|s| s.span.start as usize == r.start && s.span.end as usize == r.end)
            .map(|s| s.schema.clone())
    }

    /// The `let`s defined by statements that end before `offset`, in order.
    pub fn lets_before(&self, offset: usize) -> Vec<LetInfo> {
        let names: Vec<String> = self
            .root
            .children()
            .filter(|c| c.kind() == SyntaxKind::LET_STMT && range_of(c).end <= offset)
            .filter_map(|c| {
                c.children()
                    .find(|n| n.kind() == SyntaxKind::NAME)
                    .and_then(|n| name_of(&n))
            })
            .collect();
        names
            .into_iter()
            .map(|name| {
                let kind = self.let_kind(&name);
                LetInfo { name, kind }
            })
            .collect()
    }

    /// What the analysis knows about `let` `name`.
    pub fn let_kind(&self, name: &str) -> LetKind {
        self.let_kind_matching(|b| b.name == name)
    }

    /// What the analysis knows about the `let` binding whose defining name spans `name_range`.
    /// Duplicate `let` definitions are omitted from the HIR and therefore return [`LetKind::Unknown`].
    pub fn let_kind_at(&self, name: &str, name_range: &Range<usize>) -> LetKind {
        self.let_kind_matching(|b| b.name == name && b.name_span.range() == *name_range)
    }

    fn let_kind_matching(&self, pred: impl Fn(&hir::LetBinding) -> bool) -> LetKind {
        let binding = self.analysis.hir.statements.iter().find_map(|s| match s {
            Statement::Let(b) if pred(b) => Some(b),
            _ => None,
        });
        match binding.map(|b| &b.value) {
            Some(LetValue::Scalar(expr)) => LetKind::Scalar {
                ty: expr.ty.name().to_owned(),
                string: match &expr.kind {
                    ExprKind::Literal(Value::String(s)) => Some(s.clone()),
                    _ => None,
                },
            },
            Some(LetValue::Table(pipeline)) => {
                LetKind::Table(pipeline.columns.as_ref().map(|cs| {
                    cs.iter()
                        .map(|c| Col {
                            name: c.name.clone(),
                            ty: c
                                .arrow_only
                                .as_ref()
                                .map_or_else(|| c.ty.name().to_owned(), |t| t.to_string()),
                        })
                        .collect()
                }))
            }
            Some(LetValue::Error) | None => LetKind::Unknown,
        }
    }
}

/// The byte range of a token.
pub(crate) fn range(token: &SyntaxToken) -> Range<usize> {
    let r = token.text_range();
    usize::from(r.start())..usize::from(r.end())
}

/// The byte range of a node.
pub(crate) fn range_of(node: &SyntaxNode) -> Range<usize> {
    let r = node.text_range();
    usize::from(r.start())..usize::from(r.end())
}

/// The name a `NAME` or `NAME_REF` node holds, unquoted.
pub(crate) fn name_of(node: &SyntaxNode) -> Option<String> {
    let token = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::QUOTED_IDENT))?;
    name_text(&token)
}

/// The name an `IDENT` or `QUOTED_IDENT` token holds, unquoted.
pub(crate) fn name_text(token: &SyntaxToken) -> Option<String> {
    match token.kind() {
        SyntaxKind::IDENT => Some(token.text().to_owned()),
        SyntaxKind::QUOTED_IDENT => {
            let span = datascript_diagnostics::Span::new(FileId(0), 0, 0);
            datascript_syntax::literal::quoted_ident(token.text(), span).ok()
        }
        _ => None,
    }
}

/// Whether `kind` is an operator node (`WHERE_OP` … `GETSCHEMA_OP`).
pub(crate) fn is_operator(kind: SyntaxKind) -> bool {
    (SyntaxKind::WHERE_OP..=SyntaxKind::GETSCHEMA_OP).contains(&kind)
}

/// The operator node a token belongs to: its nearest ancestor that is an operator of a pipeline.
pub(crate) fn operator_of(token: &SyntaxToken) -> Option<SyntaxNode> {
    token.parent_ancestors().find(|n| {
        is_operator(n.kind()) && n.parent().is_some_and(|p| p.kind() == SyntaxKind::PIPELINE)
    })
}

/// The keyword tokens an operator starts with (`project`, `-`, `away`).
pub(crate) fn keyword_tokens(op: &SyntaxNode) -> Vec<SyntaxToken> {
    let mut out = Vec::new();
    for e in op.children_with_tokens() {
        match e.into_token() {
            Some(t) if t.kind().is_trivia() => {}
            Some(t) if t.kind() == SyntaxKind::IDENT && out.is_empty() => out.push(t),
            Some(t)
                if t.kind() == SyntaxKind::MINUS
                    && out.len() == 1
                    && matches!(
                        op.kind(),
                        SyntaxKind::PROJECT_AWAY_OP | SyntaxKind::PROJECT_RENAME_OP
                    ) =>
            {
                out.push(t)
            }
            Some(t) if t.kind() == SyntaxKind::IDENT && out.len() == 2 => {
                out.push(t);
                break;
            }
            _ => break,
        }
    }
    out
}

/// The operator's name as written, `project-away` for the hyphenated ones.
pub(crate) fn keyword_text(op: &SyntaxNode) -> String {
    keyword_tokens(op)
        .iter()
        .map(|t| t.text().to_owned())
        .collect()
}

/// The columns of an Arrow schema, with their DataScript types.
pub(crate) fn columns(schema: &SchemaRef) -> Vec<Col> {
    schema
        .fields()
        .iter()
        .map(|f| Col {
            name: f.name().clone(),
            ty: from_arrow(f).map_or_else(|_| f.data_type().to_string(), |t| t.name().to_owned()),
        })
        .collect()
}

/// Whether `call` (a `CALL_EXPR`) calls a reader function, and which.
pub(crate) fn reader_of(call: &SyntaxNode) -> Option<&'static crate::source::ReaderDef> {
    let callee = call.children().find(|c| c.kind() == SyntaxKind::NAME_REF)?;
    crate::source::reader(&name_of(&callee)?)
}

/// The callee name of a `CALL_EXPR`.
pub(crate) fn callee_of(call: &SyntaxNode) -> Option<(String, SyntaxNode)> {
    let callee = call.children().find(|c| c.kind() == SyntaxKind::NAME_REF)?;
    Some((name_of(&callee)?, callee))
}

/// Clamps `offset` to a UTF-8 char boundary at or before `offset.min(text.len())`.
pub(crate) fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The token at `offset` in `tokens`: the one containing it, or else the one ending at it.
pub(crate) fn token_at(tokens: &[SyntaxToken], offset: usize) -> Option<&SyntaxToken> {
    let containing = tokens.iter().find(|t| {
        let r = range(t);
        r.start <= offset && offset < r.end
    });
    containing.or_else(|| tokens.iter().find(|t| range(t).end == offset))
}

#[cfg(test)]
mod tests;
