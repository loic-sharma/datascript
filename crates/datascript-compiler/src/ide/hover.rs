//! Hover: what the name under the cursor is.

use datascript_diagnostics::SourceFile;
use datascript_syntax::ast::{self, AstNode};
use datascript_syntax::{SyntaxKind, SyntaxNode, display_name};

use super::{
    Col, Doc, LetKind, keyword_text, keyword_tokens, name_text, operator_doc, operator_of, range,
    range_of,
};
use crate::Analysis;
use crate::catalog::{FunctionCatalog, FunctionDef};

/// What hover shows for one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// The byte range of the name.
    pub start: usize,
    pub end: usize,
    /// The text, in Markdown.
    pub markdown: String,
}

/// What to show for the name at byte `offset` of `src`, whose analysis is `analysis`: a
/// column's type, a `let`'s type or columns, a function's signatures and docs, a source's
/// columns, or what an operator does.
pub fn hover(src: &SourceFile, analysis: &Analysis, offset: usize) -> Option<Hover> {
    let doc = Doc::new(src.text(), analysis);
    let token = doc.token_at(offset)?.clone();
    let parent = token.parent()?;

    // An operator's keyword.
    if let Some(op) = operator_of(&token)
        && keyword_tokens(&op).contains(&token)
    {
        let name = keyword_text(&op);
        let keywords = keyword_tokens(&op);
        let start = range(keywords.first()?).start;
        let end = range(keywords.last()?).end;
        return Some(Hover {
            start,
            end,
            markdown: format!("```ds\n{name}\n```\n{}", operator_doc(&name)?),
        });
    }

    // A source string: its columns.
    if token.kind() == SyntaxKind::STRING {
        let literal = parent.clone();
        let columns = doc.output_of(&literal)?;
        let r = range_of(&literal);
        return Some(Hover {
            start: r.start,
            end: r.end,
            markdown: format!("table `{}`\n\n{}", token.text(), columns_list(&columns)),
        });
    }

    if !matches!(token.kind(), SyntaxKind::IDENT | SyntaxKind::QUOTED_IDENT) {
        return None;
    }
    let name = name_text(&token)?;
    let r = range(&token);
    let at = |markdown: String| Hover {
        start: r.start,
        end: r.end,
        markdown,
    };
    match parent.kind() {
        SyntaxKind::NAME_REF => {
            let grand = parent.parent();
            // A function being called.
            if let Some(call) = grand.as_ref().filter(|g| g.kind() == SyntaxKind::CALL_EXPR)
                && call.children().next().as_ref() == Some(&parent)
            {
                let f = FunctionCatalog::builtin().lookup(&name).ok()?;
                return Some(at(function_markdown(f)));
            }
            // A pipeline's source: a tabular `let`, or a scalar `let` naming a file.
            if grand.as_ref().is_some_and(|g| {
                matches!(
                    g.kind(),
                    SyntaxKind::PIPELINE | SyntaxKind::JOIN_OP | SyntaxKind::UNION_OP
                )
            }) {
                return let_hover(&doc, &name, Some(&parent)).map(at);
            }
            // `$left.col` or `$right.col` in `join ... on`: resolve against that side only.
            if let Some(op) = operator_of(&token)
                && let Some(side_cols) = join_side_columns(&doc, &op, &parent)
            {
                let col = side_cols?.into_iter().find(|c| c.name == name)?;
                return Some(at(format!(
                    "```ds\n{}: {}\n```\ncolumn",
                    display_name(&col.name),
                    col.ty
                )));
            }
            // A column, or else a scalar `let`.
            if let Some(op) = operator_of(&token)
                && let Some(col) = doc
                    .input_of(&op)
                    .and_then(|cols| cols.into_iter().find(|c| c.name == name))
            {
                return Some(at(format!(
                    "```ds\n{}: {}\n```\ncolumn",
                    display_name(&col.name),
                    col.ty
                )));
            }
            let_hover(&doc, &name, None).map(at)
        }
        SyntaxKind::COLUMN_PATTERN | SyntaxKind::RENDER_PROP => {
            let op = operator_of(&token)?;
            let col = doc.input_of(&op)?.into_iter().find(|c| c.name == name)?;
            Some(at(format!(
                "```ds\n{}: {}\n```\ncolumn",
                display_name(&col.name),
                col.ty
            )))
        }
        SyntaxKind::NAME => {
            // A `let` being defined.
            let stmt = parent
                .parent()
                .filter(|p| p.kind() == SyntaxKind::LET_STMT)?;
            let _ = stmt;
            let_hover(&doc, &name, None).map(at)
        }
        _ => None,
    }
}

/// If `name_ref` is the field of `$left.<field>` or `$right.<field>` inside `op` (a `JOIN_OP`),
/// returns `Some(columns)` for that side; otherwise `None`.
fn join_side_columns(
    doc: &Doc<'_>,
    op: &SyntaxNode,
    name_ref: &SyntaxNode,
) -> Option<Option<Vec<Col>>> {
    if op.kind() != SyntaxKind::JOIN_OP {
        return None;
    }
    let field = ast::FieldExpr::cast(name_ref.parent()?)?;
    if field.field()?.syntax() != name_ref {
        return None;
    }
    let ast::Expr::NameRef(base) = field.base()? else {
        return None;
    };
    match base.token()?.kind() {
        SyntaxKind::DOLLAR_LEFT => Some(doc.input_of(op)),
        SyntaxKind::DOLLAR_RIGHT => Some(doc.join_right_of(op)),
        _ => None,
    }
}

/// A `let`'s type, or its columns when it is a table. `source` is the name used as a source,
/// whose stage schema gives the columns of a file named by a scalar `let`.
fn let_hover(doc: &Doc<'_>, name: &str, source: Option<&SyntaxNode>) -> Option<String> {
    let shown = display_name(name);
    match doc.let_kind(name) {
        LetKind::Scalar { ty, string } => {
            let mut out = format!("```ds\nlet {shown}: {ty}\n```");
            if let Some(path) = string {
                out.push_str(&format!("\n`'{path}'`"));
                if let Some(cols) = source.and_then(|s| doc.output_of(s)) {
                    out.push_str("\n\n");
                    out.push_str(&columns_list(&cols));
                }
            }
            Some(out)
        }
        LetKind::Table(columns) => {
            let mut out = format!("```ds\nlet {shown}: table\n```");
            if let Some(cols) = columns {
                out.push_str("\n\n");
                out.push_str(&columns_list(&cols));
            }
            Some(out)
        }
        LetKind::Unknown => None,
    }
}

/// The columns as a Markdown list, at most 50.
fn columns_list(columns: &[Col]) -> String {
    const MAX: usize = 50;
    let mut lines: Vec<String> = columns
        .iter()
        .take(MAX)
        .map(|c| format!("- `{}`: {}", display_name(&c.name), c.ty))
        .collect();
    if columns.len() > MAX {
        lines.push(format!("- … and {} more", columns.len() - MAX));
    }
    if lines.is_empty() {
        return "no columns".to_owned();
    }
    lines.join("\n")
}

/// A function's signatures, docs and examples.
pub(crate) fn function_markdown(f: &FunctionDef) -> String {
    let mut out = format!("```ds\n{}\n```\n{}", f.signature_lines().join("\n"), f.doc);
    if !f.examples.is_empty() {
        out.push_str("\n\nExamples:\n```ds\n");
        out.push_str(&f.examples.join("\n"));
        out.push_str("\n```");
    }
    out
}
