//! Document symbols: top-level `let` bindings and query statements.

use datascript_diagnostics::{FileId, SourceFile};
use datascript_syntax::ast::{self, AstNode, Stmt};

use super::{Doc, LetKind, range_of};
use crate::Analysis;

/// The kind of a document symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A scalar `let` binding.
    Variable,
    /// A tabular `let` binding.
    Table,
    /// A query statement.
    Query,
}

/// One top-level symbol in a document (outline / go to symbol).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSymbol {
    /// Display name of the symbol (the `let` name, or `query 1`, `query 2`, …).
    pub name: String,
    /// Optional secondary detail (inferred type, column count, or short source preview).
    pub detail: Option<String>,
    /// Whether this symbol is a scalar `let`, tabular `let`, or query statement.
    pub kind: SymbolKind,
    /// Inclusive start byte offset of the whole statement.
    pub start: usize,
    /// Exclusive end byte offset of the whole statement.
    pub end: usize,
    /// Inclusive start byte offset of the name or statement head, contained within `start..end`.
    pub selection_start: usize,
    /// Exclusive end byte offset of the name or statement head, contained within `start..end`.
    pub selection_end: usize,
}

/// The top-level `let` bindings and query statements in `src`, in source order.
///
/// Works from the parse alone when `analysis` is `None` (syntax-only mode, D-053). When
/// `analysis` is `Some`, `let` details include their inferred types or column counts.
pub fn document_symbols(src: &SourceFile, analysis: Option<&Analysis>) -> Vec<DocumentSymbol> {
    let text = src.text();
    let doc = analysis.map(|a| Doc::new(text, a));
    let root = match &doc {
        Some(d) => d.root.clone(),
        None => datascript_syntax::parse(text, FileId(0)).syntax(),
    };
    let Some(program) = ast::Program::cast(root) else {
        return Vec::new();
    };

    let mut symbols = Vec::new();
    let mut query_index = 0usize;

    for stmt in program.statements() {
        match stmt {
            Stmt::Let(let_stmt) => {
                let Some(name_node) = let_stmt.name() else {
                    continue;
                };
                let Some(name) = name_node.text() else {
                    continue;
                };
                let is_tabular = let_stmt.is_tabular();
                let kind = if is_tabular {
                    SymbolKind::Table
                } else {
                    SymbolKind::Variable
                };
                let full = range_of(let_stmt.syntax());
                let sel = range_of(name_node.syntax());
                let detail = let_detail(&name, &sel, is_tabular, doc.as_ref());
                symbols.push(DocumentSymbol {
                    name,
                    detail: Some(detail),
                    kind,
                    start: full.start,
                    end: full.end,
                    selection_start: sel.start.clamp(full.start, full.end),
                    selection_end: sel.end.clamp(full.start, full.end),
                });
            }
            Stmt::Query(pipeline) => {
                query_index += 1;
                let full = range_of(pipeline.syntax());
                let sel = pipeline
                    .from_token()
                    .map(|t| super::range(&t))
                    .or_else(|| pipeline.source().map(|s| range_of(s.syntax())))
                    .unwrap_or_else(|| full.clone());
                let detail = query_detail(&pipeline, text);
                symbols.push(DocumentSymbol {
                    name: format!("query {query_index}"),
                    detail,
                    kind: SymbolKind::Query,
                    start: full.start,
                    end: full.end,
                    selection_start: sel.start.clamp(full.start, full.end),
                    selection_end: sel.end.clamp(full.start, full.end),
                });
            }
        }
    }

    symbols
}

fn let_detail(
    name: &str,
    name_range: &std::ops::Range<usize>,
    is_tabular: bool,
    doc: Option<&Doc<'_>>,
) -> String {
    if let Some(doc) = doc {
        match doc.let_kind_at(name, name_range) {
            LetKind::Scalar { ty, .. } => return ty,
            LetKind::Table(Some(cols)) => {
                let word = if cols.len() == 1 { "column" } else { "columns" };
                return format!("table ({} {word})", cols.len());
            }
            LetKind::Table(None) => return "table".to_owned(),
            LetKind::Unknown => {}
        }
    }
    if is_tabular {
        "table".to_owned()
    } else {
        "scalar".to_owned()
    }
}

/// Short single-line summary of a query's source, e.g. `from 'orders.csv'`.
fn query_detail(pipeline: &ast::Pipeline, text: &str) -> Option<String> {
    const MAX_CHARS: usize = 60;
    let source = pipeline.source()?;
    let src_range = range_of(source.syntax());
    let start = pipeline
        .from_token()
        .map_or(src_range.start, |t| super::range(&t).start);
    let raw = text.get(start..src_range.end)?;
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= MAX_CHARS {
        Some(collapsed)
    } else {
        let mut cut: String = collapsed.chars().take(MAX_CHARS - 1).collect();
        cut.push('…');
        Some(cut)
    }
}
