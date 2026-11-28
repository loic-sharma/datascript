//! Semantic tokens over the syntax tree (with optional [`Analysis`] for column-shadowed `let`s).
//!
//! Designed to work in syntax-only mode (D-053) and to align with the TextMate scopes in
//! `editors/vscode/syntaxes/datascript.tmLanguage.json` while fixing what a regex grammar cannot
//! know — in particular, contextual keywords (`by`, `on`, `has`, `in`, `kind`, `from`, …) used
//! as column or `let` names.

use datascript_diagnostics::{FileId, SourceFile};
use datascript_syntax::ast::{self, AstNode};
use datascript_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use super::lets::LetIndex;
use super::{Doc, is_operator, keyword_tokens, name_text, range};
use crate::Analysis;
use crate::binder::{property_takes_columns, property_words};
use crate::catalog::FunctionCatalog;

/// The semantic category of a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticTokenKind {
    /// Line or block comment.
    Comment,
    /// Contextual keyword (`from`, `by`, `asc`, `desc`, `nulls`, `first`, `last`, `on`, `with`,
    /// `kind` in `join`), `let` (with `declaration` modifier), or pipeline operator name (with
    /// `default_library` modifier).
    Keyword,
    /// Symbolic or word operator (`|`, `+`, `==`, `and`, `or`, `not`, `has`, `in`, `between`,
    /// `matches`, `regex`, `!in`, …).
    Operator,
    /// Boolean/null literal (`true`, `false`, `null`), chart kind (`barchart`), join kind
    /// (`inner`), or render option word (`stacked`, `hidden`).
    EnumMember,
    /// Function call callee (`sum`, `strlen`, `read_csv`, …).
    Function,
    /// Type name in a typed null (`long(null)`, `string(null)`, …).
    Type,
    /// Column name, `let` name, or `$left` / `$right`.
    Variable,
    /// Render property name (`title`, `xcolumn`, …) or reader option name (`header`, …).
    Property,
    /// Number, timespan, or datetime literal.
    Number,
    /// String literal.
    String,
}

/// Bitset of semantic token modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct SemanticTokenModifiers(u32);

impl SemanticTokenModifiers {
    pub const NONE: Self = Self(0);
    /// Defining occurrence (`let` name, `let` keyword, output column name, rename target).
    pub const DECLARATION: Self = Self(1 << 0);
    /// Immutable binding (`let` definitions and `let` references).
    pub const READONLY: Self = Self(1 << 1);
    /// Built-in symbol (pipeline operator keywords, catalog functions, `$left`/`$right`).
    pub const DEFAULT_LIBRARY: Self = Self(1 << 2);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// One semantic token with a byte range in the source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticToken {
    pub start: usize,
    pub end: usize,
    pub kind: SemanticTokenKind,
    pub modifiers: SemanticTokenModifiers,
}

/// Semantic tokens for `src`, in ascending non-overlapping source order.
///
/// Works from the parse alone when `analysis` is `None` (syntax-only mode, D-053).
pub fn semantic_tokens(src: &SourceFile, analysis: Option<&Analysis>) -> Vec<SemanticToken> {
    let text = src.text();
    let doc = analysis.map(|a| Doc::new(text, a));
    let root = match &doc {
        Some(d) => d.root.clone(),
        None => datascript_syntax::parse(text, FileId(0)).syntax(),
    };
    let lets = LetIndex::build(&root, doc.as_ref());
    let catalog = FunctionCatalog::builtin();

    let mut out = Vec::new();
    // Byte end of a multi-token span (such as `project-away`) already emitted.
    let mut skip_until = 0usize;

    for token in root
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
        let r = range(&token);
        if r.start < skip_until || r.is_empty() {
            continue;
        }
        if let Some((end, kind, modifiers)) = classify_token(&token, &lets, catalog) {
            out.push(SemanticToken {
                start: r.start,
                end,
                kind,
                modifiers,
            });
            skip_until = end;
        }
    }

    out
}

fn classify_token(
    token: &SyntaxToken,
    lets: &LetIndex,
    catalog: &FunctionCatalog,
) -> Option<(usize, SemanticTokenKind, SemanticTokenModifiers)> {
    let r = range(token);
    let end = r.end;
    match token.kind() {
        SyntaxKind::WHITESPACE
        | SyntaxKind::EOF
        | SyntaxKind::ERROR
        | SyntaxKind::SEMICOLON
        | SyntaxKind::COMMA
        | SyntaxKind::COLON
        | SyntaxKind::DOT
        | SyntaxKind::L_PAREN
        | SyntaxKind::R_PAREN
        | SyntaxKind::L_BRACK
        | SyntaxKind::R_BRACK
        | SyntaxKind::L_BRACE
        | SyntaxKind::R_BRACE
        // Invalid habit tokens stay unhighlighted here so TextMate's `invalid.illegal` shows.
        | SyntaxKind::LTGT
        | SyntaxKind::AMP2
        | SyntaxKind::PIPE2
        | SyntaxKind::BANG => None,

        SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT => {
            Some((end, SemanticTokenKind::Comment, SemanticTokenModifiers::NONE))
        }
        SyntaxKind::STRING => Some((end, SemanticTokenKind::String, SemanticTokenModifiers::NONE)),
        SyntaxKind::LONG | SyntaxKind::REAL | SyntaxKind::TIMESPAN | SyntaxKind::DATETIME => {
            Some((end, SemanticTokenKind::Number, SemanticTokenModifiers::NONE))
        }
        SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW | SyntaxKind::NULL_KW => Some((
            end,
            SemanticTokenKind::EnumMember,
            SemanticTokenModifiers::NONE,
        )),
        SyntaxKind::LET_KW => Some((
            end,
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::DECLARATION,
        )),
        SyntaxKind::AND_KW
        | SyntaxKind::OR_KW
        | SyntaxKind::NOT_KW
        | SyntaxKind::NEGATED_OP
        | SyntaxKind::IN_TILDE
        | SyntaxKind::PIPE
        | SyntaxKind::EQ
        | SyntaxKind::EQ2
        | SyntaxKind::NEQ
        | SyntaxKind::LT
        | SyntaxKind::LTEQ
        | SyntaxKind::GT
        | SyntaxKind::GTEQ
        | SyntaxKind::EQ_TILDE
        | SyntaxKind::BANG_TILDE
        | SyntaxKind::PLUS
        | SyntaxKind::MINUS
        | SyntaxKind::STAR
        | SyntaxKind::SLASH
        | SyntaxKind::PERCENT
        | SyntaxKind::TILDE
        | SyntaxKind::DOT2 => Some((
            end,
            SemanticTokenKind::Operator,
            SemanticTokenModifiers::NONE,
        )),
        SyntaxKind::DOLLAR_LEFT | SyntaxKind::DOLLAR_RIGHT => Some((
            end,
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::DEFAULT_LIBRARY,
        )),
        SyntaxKind::IDENT | SyntaxKind::QUOTED_IDENT => {
            classify_word(token, &r, lets, catalog)
        }
        _ => None,
    }
}

fn classify_word(
    token: &SyntaxToken,
    r: &std::ops::Range<usize>,
    lets: &LetIndex,
    catalog: &FunctionCatalog,
) -> Option<(usize, SemanticTokenKind, SemanticTokenModifiers)> {
    let parent = token.parent()?;
    let end = r.end;

    match parent.kind() {
        SyntaxKind::PIPELINE => {
            let is_from = ast::Pipeline::cast(parent)
                .and_then(|p| p.from_token())
                .is_some_and(|t| range(&t) == *r);
            if is_from {
                Some((
                    end,
                    SemanticTokenKind::Keyword,
                    SemanticTokenModifiers::NONE,
                ))
            } else {
                None
            }
        }
        kind if is_operator(kind) => classify_operator_word(token, &parent, r),
        SyntaxKind::BY_CLAUSE | SyntaxKind::SORT_KEY | SyntaxKind::RENDER_PROPS => Some((
            end,
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::NONE,
        )),
        SyntaxKind::JOIN_KIND => {
            let is_value = ast::JoinKind::cast(parent)
                .and_then(|j| j.value_token())
                .is_some_and(|t| range(&t) == *r);
            if is_value {
                Some((
                    end,
                    SemanticTokenKind::EnumMember,
                    SemanticTokenModifiers::NONE,
                ))
            } else {
                Some((
                    end,
                    SemanticTokenKind::Keyword,
                    SemanticTokenModifiers::NONE,
                ))
            }
        }
        SyntaxKind::BIN_EXPR | SyntaxKind::IN_EXPR | SyntaxKind::BETWEEN_EXPR => Some((
            end,
            SemanticTokenKind::Operator,
            SemanticTokenModifiers::NONE,
        )),
        SyntaxKind::TYPED_NULL => {
            Some((end, SemanticTokenKind::Type, SemanticTokenModifiers::NONE))
        }
        SyntaxKind::COLUMN_PATTERN => Some((
            end,
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::NONE,
        )),
        SyntaxKind::NAME => classify_name_def(&parent, end),
        SyntaxKind::NAME_REF => classify_name_ref(token, &parent, r, lets, catalog),
        _ => None,
    }
}

fn classify_operator_word(
    token: &SyntaxToken,
    op: &SyntaxNode,
    r: &std::ops::Range<usize>,
) -> Option<(usize, SemanticTokenKind, SemanticTokenModifiers)> {
    let kws = keyword_tokens(op);
    if kws.first().is_some_and(|t| range(t) == *r) {
        // Merge adjacent hyphenated operator tokens (`project-away`, `project-rename`) into one.
        let end = if kws.len() == 3
            && range(&kws[0]).end == range(&kws[1]).start
            && range(&kws[1]).end == range(&kws[2]).start
        {
            range(&kws[2]).end
        } else {
            r.end
        };
        return Some((
            end,
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::DEFAULT_LIBRARY,
        ));
    }
    if kws.iter().any(|t| range(t) == *r) {
        return Some((
            r.end,
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::DEFAULT_LIBRARY,
        ));
    }
    if op.kind() == SyntaxKind::RENDER_OP
        && ast::RenderOp::cast(op.clone())
            .and_then(|r_op| r_op.kind_token())
            .is_some_and(|t| range(&t) == *r)
    {
        return Some((
            r.end,
            SemanticTokenKind::EnumMember,
            SemanticTokenModifiers::NONE,
        ));
    }
    // `by` in `sort`/`top`, `on` in `join`.
    let _ = token;
    Some((
        r.end,
        SemanticTokenKind::Keyword,
        SemanticTokenModifiers::NONE,
    ))
}

fn classify_name_def(
    name_node: &SyntaxNode,
    end: usize,
) -> Option<(usize, SemanticTokenKind, SemanticTokenModifiers)> {
    let grand = name_node.parent()?;
    match grand.kind() {
        SyntaxKind::LET_STMT => Some((
            end,
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::DECLARATION.union(SemanticTokenModifiers::READONLY),
        )),
        SyntaxKind::RENDER_PROP => Some((
            end,
            SemanticTokenKind::Property,
            SemanticTokenModifiers::NONE,
        )),
        SyntaxKind::NAMED_EXPR => {
            if grand
                .parent()
                .is_some_and(|gg| gg.kind() == SyntaxKind::ARG_LIST)
            {
                Some((
                    end,
                    SemanticTokenKind::Property,
                    SemanticTokenModifiers::NONE,
                ))
            } else {
                Some((
                    end,
                    SemanticTokenKind::Variable,
                    SemanticTokenModifiers::DECLARATION,
                ))
            }
        }
        SyntaxKind::RENAME_ITEM => Some((
            end,
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::DECLARATION,
        )),
        _ => Some((
            end,
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::DECLARATION,
        )),
    }
}

fn classify_name_ref(
    token: &SyntaxToken,
    name_ref: &SyntaxNode,
    r: &std::ops::Range<usize>,
    lets: &LetIndex,
    catalog: &FunctionCatalog,
) -> Option<(usize, SemanticTokenKind, SemanticTokenModifiers)> {
    let end = r.end;
    let grand = name_ref.parent();
    if let Some(call) = grand.as_ref().filter(|g| g.kind() == SyntaxKind::CALL_EXPR)
        && call.children().next().as_ref() == Some(name_ref)
    {
        let modifiers = if name_text(token).is_some_and(|n| catalog.lookup(&n).is_ok()) {
            SemanticTokenModifiers::DEFAULT_LIBRARY
        } else {
            SemanticTokenModifiers::NONE
        };
        return Some((end, SemanticTokenKind::Function, modifiers));
    }
    if let Some(prop) = grand
        .as_ref()
        .filter(|g| g.kind() == SyntaxKind::RENDER_PROP)
        .and_then(|g| ast::RenderProp::cast(g.clone()))
        .and_then(|p| p.name())
        .and_then(|n| n.text())
    {
        if !property_words(&prop).is_empty() {
            return Some((
                end,
                SemanticTokenKind::EnumMember,
                SemanticTokenModifiers::NONE,
            ));
        }
        if property_takes_columns(&prop) {
            return Some((
                end,
                SemanticTokenKind::Variable,
                SemanticTokenModifiers::NONE,
            ));
        }
    }
    let modifiers = if lets.is_let_ref(r) {
        SemanticTokenModifiers::READONLY
    } else {
        SemanticTokenModifiers::NONE
    };
    Some((end, SemanticTokenKind::Variable, modifiers))
}
