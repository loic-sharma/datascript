//! `let` navigation and rename: [`goto_definition`], [`references`], [`prepare_rename`] and
//! [`rename`].
//!
//! `let` bindings are resolved from the syntax tree with optional [`Analysis`] schema information.
//! In positions where columns are never in scope (scalar `let` RHS, `from`, `join` right, `union`,
//! `take`, `top` count, scalar `render` properties), every matching name resolves directly from the
//! syntax tree—including in syntax-only mode (D-053). In row-expression positions where columns win
//! over scalar `let`s (D-038), a name resolves to a `let` only when the operator's input columns
//! are known and do not contain it: when schemas are unavailable, those ambiguous sites are
//! excluded from navigation/references/semantic tokens, and [`prepare_rename`] / [`rename`] refuse
//! rather than risk rewriting a column reference.

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
    /// Inclusive start byte offset of the identifier token under the cursor.
    pub start: usize,
    /// Exclusive end byte offset of the identifier token under the cursor.
    pub end: usize,
    /// Unquoted name shown in the editor's rename input box.
    pub placeholder: String,
}

/// One text replacement produced by [`rename`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameEdit {
    /// Inclusive start byte offset of the range to replace.
    pub start: usize,
    /// Exclusive end byte offset of the range to replace.
    pub end: usize,
    /// Replacement source text (quoted with `['...']` when needed).
    pub replacement: String,
}

/// Why [`prepare_rename`] or [`rename`] refused to rename the symbol at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    /// The cursor is not on a renamable `let` definition or reference.
    NotRenamable,
    /// The requested new name is empty, contains line breaks/control characters, or is invalid.
    InvalidName(String),
    /// Another `let` in the same document already defines this name (`DS1003`).
    AlreadyDefined(String),
    /// Renaming would be shadowed by an in-scope column at a row-expression reference (D-038).
    ShadowedByColumn {
        /// The colliding column name.
        name: String,
        /// 1-based line number of the reference where the column would hide the `let`.
        line: usize,
    },
    /// At least one row-expression site using this name has an unknown input schema, so we cannot
    /// tell whether it refers to a column or to this `let` (D-038, D-053).
    UnknownSchema {
        /// Whether the request ran without an [`Analysis`] (syntax-only mode).
        syntax_only: bool,
    },
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameError::NotRenamable => write!(f, "only `let` names can be renamed here"),
            RenameError::InvalidName(msg) => write!(f, "{msg}"),
            RenameError::AlreadyDefined(name) => {
                write!(f, "`{name}` is already defined by another `let`")
            }
            RenameError::ShadowedByColumn { name, line } => write!(
                f,
                "`{name}` is a column at line {line}, so the renamed `let` would be hidden there"
            ),
            RenameError::UnknownSchema { syntax_only: true } => write!(
                f,
                "rename needs the input schema; trust the workspace to load schemas"
            ),
            RenameError::UnknownSchema { syntax_only: false } => write!(
                f,
                "rename needs the input schema, which could not be loaded for a source"
            ),
        }
    }
}

impl std::error::Error for RenameError {}

/// One resolved reference to a `let`.
#[derive(Debug, Clone)]
struct LetRef {
    range: Range<usize>,
    /// The enclosing row-expression operator when columns are in scope at this site (D-038),
    /// or `None` in no-column positions (`let` RHS, `from`, `join` right, `union`, `take`,
    /// `top` count, scalar `render` properties).
    row_op: Option<SyntaxNode>,
}

/// A top-level `let` definition and every reference that resolves to it.
#[derive(Debug, Clone)]
pub(crate) struct LetDef {
    pub name: String,
    pub name_range: Range<usize>,
    pub stmt_end: usize,
    references: Vec<LetRef>,
    /// Row-expression sites with this `let`'s name whose operator's input columns were unknown.
    unknown_row_refs: Vec<Range<usize>>,
}

/// Resolved `let` definitions and uses across one document.
pub(crate) struct LetIndex {
    pub defs: Vec<LetDef>,
    /// All `let` names declared in the document (including duplicate `let`s).
    all_names: HashSet<String>,
    /// Every unambiguous reference byte range, for O(1) membership checks in semantic tokens.
    ref_ranges: HashSet<Range<usize>>,
}

impl LetIndex {
    /// Builds the index over `root`. Pass `doc` when an [`Analysis`] is available so row-expression
    /// references can be checked against in-scope columns (D-038).
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
                        unknown_row_refs: Vec::new(),
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

    /// Whether the token spanning `r` unambiguously resolves to a `let` reference.
    pub(crate) fn is_let_ref(&self, r: &Range<usize>) -> bool {
        self.ref_ranges.contains(r)
    }

    /// Finds the `LetDef` (index) and matched token range at byte `offset`, preferring a range
    /// containing `offset` and falling back to one ending at `offset` (so a cursor right before
    /// punctuation such as `cutoff;` or `abs(cutoff)` still resolves `cutoff`).
    fn resolve_at_offset(&self, offset: usize) -> Option<(usize, Range<usize>)> {
        let mut ending = None;
        for (idx, def) in self.defs.iter().enumerate() {
            for r in
                std::iter::once(&def.name_range).chain(def.references.iter().map(|lr| &lr.range))
            {
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

    /// Whether `offset` lands on a row-expression site matching some in-scope `let` whose input
    /// schema was unknown.
    fn has_unknown_ref_at(&self, offset: usize) -> bool {
        self.defs.iter().any(|def| {
            def.unknown_row_refs
                .iter()
                .any(|r| (r.start <= offset && offset < r.end) || r.end == offset)
        })
    }
}

/// Three-state classification of a `NAME_REF` whose text matches an in-scope `let`.
enum RefStatus {
    /// Definitely not a `let` reference (callee, field, column-only op, or shadowed by a column).
    NotLet,
    /// Unambiguously a `let` reference; `row_op` is `Some(op)` in row-expression positions.
    Let { row_op: Option<SyntaxNode> },
    /// In a row-expression position whose input columns are unknown (syntax-only or failed schema).
    UnknownRow,
}

/// Walks `node` in preorder, recording every `NAME_REF` that resolves to (or is ambiguously
/// shadowed at) a visible `let`.
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
        match classify_let_ref(&desc, &name, tok_range.start, doc) {
            RefStatus::Let { row_op } => {
                defs[def_idx].references.push(LetRef {
                    range: tok_range.clone(),
                    row_op,
                });
                ref_ranges.insert(tok_range);
            }
            RefStatus::UnknownRow => {
                defs[def_idx].unknown_row_refs.push(tok_range);
            }
            RefStatus::NotLet => {}
        }
    }
}

/// Classifies whether `name_ref` refers to a `let` named `name`, is definitely not a `let`, or
/// sits in a row expression whose input columns are unknown.
fn classify_let_ref(
    name_ref: &SyntaxNode,
    name: &str,
    offset: usize,
    doc: Option<&Doc<'_>>,
) -> RefStatus {
    let Some(parent) = name_ref.parent() else {
        return RefStatus::NotLet;
    };
    match parent.kind() {
        // Function call callee (`f(...)`): never a `let` (until UDFs in M2-05).
        SyntaxKind::CALL_EXPR if parent.children().next().as_ref() == Some(name_ref) => {
            return RefStatus::NotLet;
        }
        // Member field (`$left.col`, `a.b`): never a `let`.
        SyntaxKind::FIELD_EXPR => {
            if ast::FieldExpr::cast(parent.clone())
                .and_then(|f| f.field())
                .is_some_and(|f| f.syntax() == name_ref)
            {
                return RefStatus::NotLet;
            }
        }
        // `project-rename new = old`: `old` is always an input column.
        SyntaxKind::RENAME_ITEM => return RefStatus::NotLet,
        // `distinct a, b`: columns only (DS1032).
        SyntaxKind::DISTINCT_OP => return RefStatus::NotLet,
        _ => {}
    }

    // Find the nearest enclosing pipeline or operator to determine whether columns are in scope.
    let scope = name_ref
        .ancestors()
        .skip(1)
        .find(|n| n.kind() == SyntaxKind::PIPELINE || is_operator(n.kind()));

    let Some(scope) = scope else {
        // Scalar `let` RHS outside any pipeline: no columns in scope.
        return RefStatus::Let { row_op: None };
    };

    match scope.kind() {
        // Pipeline source (e.g. `from t`, `from read_csv(p)`): no columns in scope.
        SyntaxKind::PIPELINE => RefStatus::Let { row_op: None },
        // `union t1, t2`: table position, no columns in scope.
        SyntaxKind::UNION_OP => RefStatus::Let { row_op: None },
        // `join [kind = k] right on conditions`: table position before `on`, columns only after.
        SyntaxKind::JOIN_OP => {
            let on = scope
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| t.kind() == SyntaxKind::IDENT && t.text() == "on");
            if on.is_none_or(|t| offset < range(&t).start) {
                RefStatus::Let { row_op: None }
            } else {
                RefStatus::NotLet
            }
        }
        // `project-away`, `project-rename`, `distinct`: column names only.
        SyntaxKind::PROJECT_AWAY_OP | SyntaxKind::PROJECT_RENAME_OP | SyntaxKind::DISTINCT_OP => {
            RefStatus::NotLet
        }
        // `take n`: constant count, no columns in scope (D-049).
        SyntaxKind::TAKE_OP => RefStatus::Let { row_op: None },
        // `top n by key`: `n` is a constant (no columns), `key` is a row expression.
        SyntaxKind::TOP_OP => {
            let in_key = name_ref
                .ancestors()
                .take_while(|n| n != &scope)
                .any(|n| n.kind() == SyntaxKind::SORT_KEY);
            if !in_key {
                RefStatus::Let { row_op: None }
            } else {
                row_ref_status(&scope, name, offset, doc)
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
                return RefStatus::NotLet;
            };
            if !property_takes_columns(&prop) && property_words(&prop).is_empty() {
                RefStatus::Let { row_op: None }
            } else {
                RefStatus::NotLet
            }
        }
        // Row-expression operators (`where`, `project`, `extend`, `summarize`, `sort`):
        // columns win over `let`s (D-038), so schemas must be known.
        _ => row_ref_status(&scope, name, offset, doc),
    }
}

fn row_ref_status(op: &SyntaxNode, name: &str, offset: usize, doc: Option<&Doc<'_>>) -> RefStatus {
    let Some(cols) = doc.and_then(|d| d.columns_at(op, offset)) else {
        return RefStatus::UnknownRow;
    };
    if cols.iter().any(|c| c.name == name) {
        RefStatus::NotLet
    } else {
        RefStatus::Let {
            row_op: Some(op.clone()),
        }
    }
}

/// Parses `text` (reusing `analysis` when present) and builds its [`LetIndex`].
fn build_index<'a>(text: &'a str, analysis: Option<&'a Analysis>) -> (Option<Doc<'a>>, LetIndex) {
    match analysis {
        Some(analysis) => {
            let doc = Doc::new(text, analysis);
            let index = LetIndex::build(&doc.root, Some(&doc));
            (Some(doc), index)
        }
        None => {
            let root = datascript_syntax::parse(text, FileId(0)).syntax();
            let index = LetIndex::build(&root, None);
            (None, index)
        }
    }
}

/// The byte range of the `let` definition for the name at byte `offset`, if it unambiguously
/// refers to a `let`.
///
/// Works from the syntax tree alone when `analysis` is `None` (syntax-only mode, D-053) for
/// no-column positions; row-expression positions require a known schema (D-038).
pub fn goto_definition(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
) -> Option<Range<usize>> {
    let text = src.text();
    let offset = floor_char_boundary(text, offset);
    let (_, index) = build_index(text, analysis);
    let (def_idx, _) = index.resolve_at_offset(offset)?;
    Some(index.defs[def_idx].name_range.clone())
}

/// Byte ranges of all unambiguous references to the `let` at byte `offset`, in source order.
///
/// When `include_declaration` is true, the defining `let` name is included first.
pub fn references(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
    include_declaration: bool,
) -> Vec<Range<usize>> {
    let text = src.text();
    let offset = floor_char_boundary(text, offset);
    let (_, index) = build_index(text, analysis);
    let Some((def_idx, _)) = index.resolve_at_offset(offset) else {
        return Vec::new();
    };
    let def = &index.defs[def_idx];
    let mut out = Vec::with_capacity(def.references.len() + usize::from(include_declaration));
    if include_declaration {
        out.push(def.name_range.clone());
    }
    out.extend(def.references.iter().map(|r| r.range.clone()));
    out
}

/// Checks whether the token at byte `offset` is a `let` that can be safely renamed, returning its
/// range and unquoted name.
///
/// Returns `Ok(None)` when the cursor is not on a `let`, and `Err(RenameError::UnknownSchema)`
/// when a row-expression use of the `let` sits in an operator with an unknown input schema.
pub fn prepare_rename(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
) -> Result<Option<RenameTarget>, RenameError> {
    let text = src.text();
    let offset = floor_char_boundary(text, offset);
    let (_, index) = build_index(text, analysis);
    let Some((def_idx, tok_range)) = index.resolve_at_offset(offset) else {
        if index.has_unknown_ref_at(offset) {
            return Err(RenameError::UnknownSchema {
                syntax_only: analysis.is_none(),
            });
        }
        return Ok(None);
    };
    let def = &index.defs[def_idx];
    if !def.unknown_row_refs.is_empty() {
        return Err(RenameError::UnknownSchema {
            syntax_only: analysis.is_none(),
        });
    }
    Ok(Some(RenameTarget {
        start: tok_range.start,
        end: tok_range.end,
        placeholder: def.name.clone(),
    }))
}

/// Renames the `let` at byte `offset` to `new_name`, returning edits for its definition and all
/// its references in source order.
///
/// Names that are not plain identifiers (or are reserved words) are quoted with `['...']`
/// automatically; an already-quoted `['...']` name is accepted too. Refuses when any
/// row-expression site has an unknown schema or when `new_name` would be shadowed by an in-scope
/// column at a reference (D-038).
pub fn rename(
    src: &SourceFile,
    analysis: Option<&Analysis>,
    offset: usize,
    new_name: &str,
) -> Result<Vec<RenameEdit>, RenameError> {
    let text = src.text();
    let offset = floor_char_boundary(text, offset);
    let (doc, index) = build_index(text, analysis);
    let Some((def_idx, _)) = index.resolve_at_offset(offset) else {
        if index.has_unknown_ref_at(offset) {
            return Err(RenameError::UnknownSchema {
                syntax_only: analysis.is_none(),
            });
        }
        return Err(RenameError::NotRenamable);
    };
    let def = &index.defs[def_idx];
    if !def.unknown_row_refs.is_empty() {
        return Err(RenameError::UnknownSchema {
            syntax_only: analysis.is_none(),
        });
    }
    let (raw, replacement) = parse_new_name(new_name)?;
    if raw != def.name && index.all_names.contains(&raw) {
        return Err(RenameError::AlreadyDefined(raw));
    }
    if raw != def.name
        && let Some(doc) = doc.as_ref()
    {
        for r in &def.references {
            if let Some(op) = &r.row_op
                && doc
                    .columns_at(op, r.range.start)
                    .is_some_and(|cols| cols.iter().any(|c| c.name == raw))
            {
                let line = text[..r.range.start]
                    .bytes()
                    .filter(|&b| b == b'\n')
                    .count()
                    + 1;
                return Err(RenameError::ShadowedByColumn { name: raw, line });
            }
        }
    }
    let mut edits = Vec::with_capacity(1 + def.references.len());
    edits.push(RenameEdit {
        start: def.name_range.start,
        end: def.name_range.end,
        replacement: replacement.clone(),
    });
    for r in &def.references {
        edits.push(RenameEdit {
            start: r.range.start,
            end: r.range.end,
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
