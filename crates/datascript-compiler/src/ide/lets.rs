//! `let` navigation and rename: [`goto_definition`], [`references`], [`prepare_rename`] and
//! [`rename`].
//!
//! `let` bindings are resolved directly from the syntax tree so every feature works in
//! syntax-only mode (D-053) as well as full mode. When an [`Analysis`] is provided, row
//! expressions check the columns in scope at the operator so a column shadowing a `let`
//! (D-038) is not mistaken for a reference to that `let`.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use datascript_diagnostics::{FileId, SourceFile, Span};
use datascript_syntax::ast::{self, AstNode, Stmt};
use datascript_syntax::{SyntaxKind, SyntaxNode, display_name, literal};

use super::{Doc, floor_char_boundary, is_operator, name_text, range, range_of};
use crate::Analysis;
use crate::binder::{property_takes_columns, property_words};

/// The target of a rename request at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameTarget {
    /// Byte range of the identifier token under the cursor.
    pub start: usize,
    pub end: usize,
    /// Unquoted name shown in the editor's rename input box.
    pub placeholder: String,
}

/// One text replacement produced by [`rename`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

/// Why [`rename`] refused to rename the symbol at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    /// The cursor is not on a renamable `let` definition or reference.
    NotRenamable,
    /// The requested new name is empty, contains line breaks/control characters, or is invalid.
    InvalidName(String),
    /// Another `let` in the same document already defines this name (`DS1003`).
    AlreadyDefined(String),
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameError::NotRenamable => write!(f, "only `let` names can be renamed here"),
            RenameError::InvalidName(msg) => write!(f, "{msg}"),
            RenameError::AlreadyDefined(name) => {
                write!(f, "`{name}` is already defined by another `let`")
            }
        }
    }
}

impl std::error::Error for RenameError {}

/// A top-level `let` definition and every reference that resolves to it.
#[derive(Debug, Clone)]
pub(crate) struct LetDef {
    pub name: String,
    pub name_range: Range<usize>,
    pub stmt_end: usize,
    pub references: Vec<Range<usize>>,
}

/// Resolved `let` definitions and uses across one document.
pub(crate) struct LetIndex {
    pub defs: Vec<LetDef>,
    /// All `let` names declared in the document (including duplicate `let`s).
    all_names: HashSet<String>,
    /// Every reference byte range, for O(1) membership checks in semantic tokens.
    ref_ranges: HashSet<Range<usize>>,
}

impl LetIndex {
    /// Builds the index over `root`. Pass `doc` when an [`Analysis`] is available so columns
    /// that shadow a `let` (D-038) are excluded.
    pub(crate) fn build(root: &SyntaxNode, doc: Option<&Doc<'_>>) -> Self {
        let Some(program) = ast::Program::cast(root.clone()) else {
            return LetIndex {
                defs: Vec::new(),
                all_names: HashSet::new(),
                ref_ranges: HashSet::new(),
            };
        };

        let mut defs: Vec<LetDef> = Vec::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        let mut all_names: HashSet<String> = HashSet::new();

        for stmt in program.statements() {
            if let Stmt::Let(let_stmt) = stmt
                && let Some(name_node) = let_stmt.name()
                && let Some(token) = name_node.token()
                && let Some(name) = name_text(&token)
            {
                all_names.insert(name.clone());
                if !by_name.contains_key(&name) {
                    let idx = defs.len();
                    by_name.insert(name.clone(), idx);
                    defs.push(LetDef {
                        name,
                        name_range: range(&token),
                        stmt_end: range_of(let_stmt.syntax()).end,
                        references: Vec::new(),
                    });
                }
            }
        }

        let mut ref_ranges = HashSet::new();
        for stmt in program.statements() {
            let stmt_start = range_of(stmt.syntax()).start;
            match stmt {
                Stmt::Let(let_stmt) => {
                    if let Some(value) = let_stmt.value() {
                        collect_refs(
                            value.syntax(),
                            stmt_start,
                            &by_name,
                            &mut defs,
                            &mut ref_ranges,
                            doc,
                        );
                    }
                }
                Stmt::Query(pipeline) => {
                    collect_refs(
                        pipeline.syntax(),
                        stmt_start,
                        &by_name,
                        &mut defs,
                        &mut ref_ranges,
                        doc,
                    );
                }
            }
        }

        LetIndex {
            defs,
            all_names,
            ref_ranges,
        }
    }

    /// Whether the token spanning `r` resolves to a `let` reference.
    pub(crate) fn is_let_ref(&self, r: &Range<usize>) -> bool {
        self.ref_ranges.contains(r)
    }

    /// Finds the `LetDef` (index) and matched token range at byte `offset`, preferring a range
    /// containing `offset` and falling back to one ending at `offset` (so a cursor right before
    /// punctuation such as `cutoff;` or `abs(cutoff)` still resolves `cutoff`).
    fn resolve_at_offset(&self, offset: usize) -> Option<(usize, Range<usize>)> {
        let mut ending = None;
        for (idx, def) in self.defs.iter().enumerate() {
            for r in std::iter::once(&def.name_range).chain(&def.references) {
                if r.start <= offset && offset < r.end {
                    return Some((idx, r.clone()));
                }
                if ending.is_none() && r.end == offset {
                    ending = Some((idx, r.clone()));
                }
            }
        }
        ending
    }
}

/// Walks `node` in preorder, recording every `NAME_REF` that resolves to a visible `let`.
fn collect_refs(
    node: &SyntaxNode,
    stmt_start: usize,
    by_name: &HashMap<String, usize>,
    defs: &mut [LetDef],
    ref_ranges: &mut HashSet<Range<usize>>,
    doc: Option<&Doc<'_>>,
) {
    for desc in node.descendants() {
        if desc.kind() != SyntaxKind::NAME_REF {
            continue;
        }
        let Some(token) = desc
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::QUOTED_IDENT))
        else {
            continue;
        };
        let Some(name) = name_text(&token) else {
            continue;
        };
        let Some(&def_idx) = by_name.get(&name) else {
            continue;
        };
        // Only `let`s whose statement ended before the enclosing statement are in scope.
        if defs[def_idx].stmt_end > stmt_start {
            continue;
        }
        let tok_range = range(&token);
        if is_let_reference(&desc, &name, tok_range.start, doc) {
            defs[def_idx].references.push(tok_range.clone());
            ref_ranges.insert(tok_range);
        }
    }
}

/// Whether `name_ref` can refer to a `let` named `name` (not a function call, field access,
/// column-only clause, or shadowed by a known column).
fn is_let_reference(
    name_ref: &SyntaxNode,
    name: &str,
    offset: usize,
    doc: Option<&Doc<'_>>,
) -> bool {
    let Some(parent) = name_ref.parent() else {
        return false;
    };
    match parent.kind() {
        // Function call callee (`f(...)`): never a `let` (until UDFs in M2-05).
        SyntaxKind::CALL_EXPR if parent.children().next().as_ref() == Some(name_ref) => {
            return false;
        }
        // Member field (`$left.col`, `a.b`): never a `let`.
        SyntaxKind::FIELD_EXPR => {
            if ast::FieldExpr::cast(parent.clone())
                .and_then(|f| f.field())
                .is_some_and(|f| f.syntax() == name_ref)
            {
                return false;
            }
        }
        // `project-rename new = old`: `old` is always an input column.
        SyntaxKind::RENAME_ITEM => return false,
        // `distinct a, b`: columns only (DS1032).
        SyntaxKind::DISTINCT_OP => return false,
        _ => {}
    }

    // Find the nearest enclosing pipeline or operator to determine whether columns are in scope.
    let scope = name_ref
        .ancestors()
        .skip(1)
        .find(|n| n.kind() == SyntaxKind::PIPELINE || is_operator(n.kind()));

    let Some(scope) = scope else {
        // Scalar `let` RHS outside any pipeline: no columns in scope.
        return true;
    };

    match scope.kind() {
        // Pipeline source (e.g. `from t`, `from read_csv(p)`): no columns in scope.
        SyntaxKind::PIPELINE => true,
        // `union t1, t2`: table position, no columns in scope.
        SyntaxKind::UNION_OP => true,
        // `join [kind = k] right on conditions`: table position before `on`, columns only after.
        SyntaxKind::JOIN_OP => {
            let on = scope
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| t.kind() == SyntaxKind::IDENT && t.text() == "on");
            on.is_none_or(|t| offset < range(&t).start)
        }
        // `project-away`, `project-rename`, `distinct`: column names only.
        SyntaxKind::PROJECT_AWAY_OP | SyntaxKind::PROJECT_RENAME_OP | SyntaxKind::DISTINCT_OP => {
            false
        }
        // `take n`: constant count, no columns in scope (D-049).
        SyntaxKind::TAKE_OP => true,
        // `top n by key`: `n` is a constant (no columns), `key` is a row expression.
        SyntaxKind::TOP_OP => {
            let in_key = name_ref
                .ancestors()
                .take_while(|n| n != &scope)
                .any(|n| n.kind() == SyntaxKind::SORT_KEY);
            if !in_key {
                true
            } else {
                !shadowed_by_column(&scope, name, offset, doc)
            }
        }
        // `render`: column and keyword properties are never `let`s; scalar props (`title`, `ymin`,
        // ...) are constants with no columns in scope.
        SyntaxKind::RENDER_OP => {
            let Some(prop) = name_ref
                .ancestors()
                .find(|n| n.kind() == SyntaxKind::RENDER_PROP)
                .and_then(ast::RenderProp::cast)
                .and_then(|p| p.name())
                .and_then(|n| n.text())
            else {
                return false;
            };
            !property_takes_columns(&prop) && property_words(&prop).is_empty()
        }
        // Row-expression operators (`where`, `project`, `extend`, `summarize`, `sort`):
        // columns win over `let`s when known (D-038).
        _ => !shadowed_by_column(&scope, name, offset, doc),
    }
}

fn shadowed_by_column(op: &SyntaxNode, name: &str, offset: usize, doc: Option<&Doc<'_>>) -> bool {
    doc.and_then(|d| d.columns_at(op, offset))
        .is_some_and(|cols| cols.iter().any(|c| c.name == name))
}

/// Parses `src` (reusing `analysis` when present) and resolves the `let` under `offset`.
fn resolve_at(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
) -> Option<(LetIndex, usize, Range<usize>)> {
    let text = src.text();
    let offset = floor_char_boundary(text, offset);
    let index = match analysis {
        Some(analysis) => {
            let doc = Doc::new(text, analysis);
            LetIndex::build(&doc.root, Some(&doc))
        }
        None => {
            let root = datascript_syntax::parse(text, FileId(0)).syntax();
            LetIndex::build(&root, None)
        }
    };
    let (def_idx, tok_range) = index.resolve_at_offset(offset)?;
    Some((index, def_idx, tok_range))
}

/// The byte range of the `let` definition for the name at byte `offset`, if it refers to a `let`.
///
/// Works from the syntax tree alone when `analysis` is `None` (syntax-only mode, D-053).
pub fn goto_definition(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
) -> Option<Range<usize>> {
    let (index, def_idx, _) = resolve_at(src, analysis, offset)?;
    Some(index.defs[def_idx].name_range.clone())
}

/// Byte ranges of all references to the `let` at byte `offset`, in source order.
///
/// When `include_declaration` is true, the defining `let` name is included first.
pub fn references(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
    include_declaration: bool,
) -> Vec<Range<usize>> {
    let Some((index, def_idx, _)) = resolve_at(src, analysis, offset) else {
        return Vec::new();
    };
    let def = &index.defs[def_idx];
    let mut out = Vec::with_capacity(def.references.len() + usize::from(include_declaration));
    if include_declaration {
        out.push(def.name_range.clone());
    }
    out.extend(def.references.iter().cloned());
    out
}

/// Checks whether the token at byte `offset` is a `let` that can be renamed, returning its range
/// and unquoted name.
pub fn prepare_rename(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
) -> Option<RenameTarget> {
    let (index, def_idx, tok_range) = resolve_at(src, analysis, offset)?;
    Some(RenameTarget {
        start: tok_range.start,
        end: tok_range.end,
        placeholder: index.defs[def_idx].name.clone(),
    })
}

/// Renames the `let` at byte `offset` to `new_name`, returning edits for its definition and all
/// its references in source order.
///
/// Names that are not plain identifiers (or are reserved words) are quoted with `['...']`
/// automatically; an already-quoted `['...']` name is accepted too.
pub fn rename(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
    new_name: &str,
) -> Result<Vec<RenameEdit>, RenameError> {
    let Some((index, def_idx, _)) = resolve_at(src, analysis, offset) else {
        return Err(RenameError::NotRenamable);
    };
    let (raw, replacement) = parse_new_name(new_name)?;
    let def = &index.defs[def_idx];
    if raw != def.name && index.all_names.contains(&raw) {
        return Err(RenameError::AlreadyDefined(raw));
    }
    let mut edits = Vec::with_capacity(1 + def.references.len());
    edits.push(RenameEdit {
        start: def.name_range.start,
        end: def.name_range.end,
        replacement: replacement.clone(),
    });
    for r in &def.references {
        edits.push(RenameEdit {
            start: r.start,
            end: r.end,
            replacement: replacement.clone(),
        });
    }
    Ok(edits)
}

/// Validates `new_name` and returns `(unquoted_name, source_text)`.
fn parse_new_name(new_name: &str) -> Result<(String, String), RenameError> {
    if new_name.is_empty() {
        return Err(RenameError::InvalidName("name cannot be empty".into()));
    }
    let is_quoted_ident = new_name.len() >= 4
        && ((new_name.starts_with("['") && new_name.ends_with("']"))
            || (new_name.starts_with("[\"") && new_name.ends_with("\"]")));
    let raw = if is_quoted_ident {
        let span = Span::new(FileId(0), 0, 0);
        literal::quoted_ident(new_name, span)
            .map_err(|_| RenameError::InvalidName(format!("`{new_name}` is not a valid name")))?
    } else {
        new_name.to_owned()
    };
    if raw.is_empty() || raw.chars().all(char::is_whitespace) {
        return Err(RenameError::InvalidName("name cannot be empty".into()));
    }
    if raw
        .chars()
        .any(|c| c == '\r' || c == '\n' || c.is_control())
    {
        return Err(RenameError::InvalidName(
            "name cannot contain line breaks or control characters".into(),
        ));
    }
    let replacement = display_name(&raw);
    Ok((raw, replacement))
}
