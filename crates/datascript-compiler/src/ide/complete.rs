//! Completion: what may be written at the cursor.
//!
//! The context comes from the token before the word being typed, not from where the parser put
//! that word: in incomplete code (`orders | where am`) the word is often inside an `ERROR` node,
//! while the tokens before it parsed as they should.

use datascript_syntax::{OPERATOR_NAMES, SyntaxKind, SyntaxNode, SyntaxToken, display_name};

use super::{
    Col, Doc, LetInfo, LetKind, keyword_tokens, operator_doc, operator_of, range, reader_of,
};
use crate::Analysis;
use crate::binder::{property_names, property_takes_columns, property_words};
use crate::capabilities::JoinKind;
use crate::catalog::{FunctionCatalog, FunctionDef, FunctionKind};
use crate::ir::ChartKind;
use crate::source::OptionKind;
use datascript_diagnostics::SourceFile;

/// The items for one cursor position.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Completions {
    /// The byte range an item replaces: the word being typed, or an empty range at the cursor.
    pub start: usize,
    pub end: usize,
    pub items: Vec<Completion>,
    /// Set when file names fit here: the frontend lists them, since the compiler reads no files.
    pub files: Option<FileCompletion>,
}

/// File names to offer, listed by the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCompletion {
    /// The directory part of the path typed so far (up to and including the last `/`), to list
    /// relative to the base directory. Empty for the base directory itself.
    pub dir: String,
    /// The quote the path is written in, when the cursor is inside a string: insert names as
    /// they are. `None` outside a string: insert each name as a string, `'orders.csv'`.
    pub quote: Option<char>,
}

/// One completion item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// What the list shows and filters on.
    pub label: String,
    pub kind: CompletionKind,
    /// A short note shown next to the label: a type, a signature.
    pub detail: Option<String>,
    /// Longer documentation, in Markdown.
    pub documentation: Option<String>,
    /// The text to insert.
    pub insert: String,
    /// Whether [`Completion::insert`] is an LSP snippet (`$0`, `${1:name}`). A frontend that
    /// cannot expand snippets inserts [`Completion::label`] instead.
    pub snippet: bool,
}

/// What a completion item is. The order is the order items are listed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompletionKind {
    /// A column of the table at the cursor.
    Column,
    /// A render property or reader option name.
    Property,
    /// A value from a fixed list: chart kinds, join kinds, render and reader option words.
    Value,
    /// A tabular `let`.
    Table,
    /// A scalar `let`.
    Variable,
    /// A function from the catalog.
    Function,
    /// A pipeline operator.
    Operator,
    /// Any other keyword.
    Keyword,
}

impl Completion {
    fn plain(label: impl Into<String>, kind: CompletionKind) -> Self {
        let label = label.into();
        Completion {
            insert: label.clone(),
            label,
            kind,
            detail: None,
            documentation: None,
            snippet: false,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn snippet(mut self, insert: impl Into<String>) -> Self {
        self.insert = insert.into();
        self.snippet = true;
        self
    }
}

/// What can be written at byte `offset` of `src`, whose analysis is `analysis`.
///
/// `analysis` must be the [`check`](crate::check) of exactly this text: its stage schemas give
/// the columns and its HIR the `let` types. Items are not filtered by the word being typed;
/// editors filter and rank them as the user types.
pub fn complete(src: &SourceFile, analysis: &Analysis, offset: usize) -> Completions {
    let text = src.text();
    let offset = floor_char_boundary(text, offset.min(text.len()));
    let doc = Doc::new(text, analysis);
    let mut out = Completions {
        start: offset,
        end: offset,
        ..Completions::default()
    };
    if doc.in_comment(offset) {
        return out;
    }
    let Some(cursor) = Cursor::find(&doc, offset) else {
        statement_start(&mut out);
        return out;
    };
    out.start = cursor.start;
    out.end = cursor.end;
    if let Some(string) = &cursor.string {
        if is_path_position(string) {
            in_path(&mut out, text, string, offset);
        }
        return out;
    }
    let lets = doc.lets_before(offset);
    let context = classify(&doc, &cursor);
    fill(&mut out, &lets, context);
    // Columns keep their table order; everything else is alphabetical within its kind.
    out.items.sort_by(|a, b| {
        a.kind.cmp(&b.kind).then_with(|| {
            if a.kind == CompletionKind::Column {
                std::cmp::Ordering::Equal
            } else {
                a.label.cmp(&b.label)
            }
        })
    });
    out.items
        .dedup_by(|a, b| a.label == b.label && a.kind == b.kind);
    out
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Where the cursor is: the word being typed, if any, and the token before it.
struct Cursor {
    /// The replaced range.
    start: usize,
    end: usize,
    /// The last token before the word (or before the cursor, when no word is typed).
    prev: Option<SyntaxToken>,
    /// The string token the cursor is inside.
    string: Option<SyntaxToken>,
    /// Whether the word is joined to `-`s and words before it right after a `|` (`project-aw`).
    hyphenated_operator: bool,
}

impl Cursor {
    /// `None` when there is no token before the cursor at all.
    fn find(doc: &Doc<'_>, offset: usize) -> Option<Cursor> {
        let i = doc.left_index(offset)?;
        let token = &doc.tokens[i];
        let r = range(token);
        if token.kind() == SyntaxKind::STRING && is_inside_string(token, offset) {
            return Some(Cursor {
                start: offset,
                end: offset,
                prev: None,
                string: Some(token.clone()),
                hyphenated_operator: false,
            });
        }
        let is_word =
            |t: &SyntaxToken| t.kind() == SyntaxKind::IDENT || t.kind().is_reserved_word();
        if is_word(token) && offset <= r.end {
            // The word being typed, and any `ident -` joined before it right after a `|`.
            let mut first = i;
            while first >= 2
                && doc.tokens[first - 1].kind() == SyntaxKind::MINUS
                && is_word(&doc.tokens[first - 2])
                && joined(&doc.tokens[first - 2], &doc.tokens[first - 1])
                && joined(&doc.tokens[first - 1], &doc.tokens[first])
            {
                first -= 2;
            }
            let after_pipe = first
                .checked_sub(1)
                .is_some_and(|p| doc.tokens[p].kind() == SyntaxKind::PIPE);
            if first < i && after_pipe {
                return Some(Cursor {
                    start: range(&doc.tokens[first]).start,
                    end: r.end,
                    prev: Some(doc.tokens[first - 1].clone()),
                    string: None,
                    hyphenated_operator: true,
                });
            }
            return Some(Cursor {
                start: r.start,
                end: r.end,
                prev: i.checked_sub(1).map(|p| doc.tokens[p].clone()),
                string: None,
                hyphenated_operator: false,
            });
        }
        // `project-` with the cursor after the `-`.
        if token.kind() == SyntaxKind::MINUS
            && r.end == offset
            && i >= 2
            && is_word(&doc.tokens[i - 1])
            && joined(&doc.tokens[i - 1], token)
            && doc.tokens[i - 2].kind() == SyntaxKind::PIPE
        {
            return Some(Cursor {
                start: range(&doc.tokens[i - 1]).start,
                end: offset,
                prev: Some(doc.tokens[i - 2].clone()),
                string: None,
                hyphenated_operator: true,
            });
        }
        // A quoted name being typed: replace it whole.
        if token.kind() == SyntaxKind::QUOTED_IDENT && offset <= r.end {
            return Some(Cursor {
                start: r.start,
                end: r.end,
                prev: i.checked_sub(1).map(|p| doc.tokens[p].clone()),
                string: None,
                hyphenated_operator: false,
            });
        }
        Some(Cursor {
            start: offset,
            end: offset,
            prev: Some(token.clone()),
            string: None,
            hyphenated_operator: false,
        })
    }
}

fn joined(a: &SyntaxToken, b: &SyntaxToken) -> bool {
    range(a).end == range(b).start
}

/// Whether `offset` is after the opening quote of `string` and not after its closing quote.
fn is_inside_string(string: &SyntaxToken, offset: usize) -> bool {
    let r = range(string);
    let (open, quote) = string_open(string.text());
    if offset < r.start + open {
        return false;
    }
    let text = string.text();
    let closed = text.len() > open && text.ends_with(quote);
    !closed || offset < r.end
}

/// The length of a string's opening (`'`, `"`, `@"`) and its quote character.
fn string_open(text: &str) -> (usize, char) {
    let verbatim = text.starts_with('@');
    let quote = text[usize::from(verbatim)..].chars().next().unwrap_or('\'');
    (usize::from(verbatim) + 1, quote)
}

/// Whether a string literal is in a place where it names a file: a pipeline's source, a table
/// of `join` or `union`, or the first argument of a reader function.
fn is_path_position(string: &SyntaxToken) -> bool {
    let Some(literal) = string.parent().filter(|p| p.kind() == SyntaxKind::LITERAL) else {
        return false;
    };
    let Some(parent) = literal.parent() else {
        return false;
    };
    match parent.kind() {
        SyntaxKind::PIPELINE | SyntaxKind::JOIN_OP | SyntaxKind::UNION_OP => true,
        SyntaxKind::ARG_LIST => {
            parent.parent().is_some_and(|call| {
                call.kind() == SyntaxKind::CALL_EXPR && reader_of(&call).is_some()
            }) && parent.children().next().as_ref() == Some(&literal)
        }
        _ => false,
    }
}

/// The cursor is inside a path string: list files from the directory typed so far.
fn in_path(out: &mut Completions, text: &str, string: &SyntaxToken, offset: usize) {
    let r = range(string);
    let (open, quote) = string_open(string.text());
    let content_start = r.start + open;
    let typed = &text[content_start..offset];
    let dir_len = typed.rfind('/').map_or(0, |i| i + 1);
    let closed = string.text().len() > open && string.text().ends_with(quote);
    out.start = content_start + dir_len;
    out.end = if closed { r.end - 1 } else { r.end };
    out.files = Some(FileCompletion {
        dir: typed[..dir_len].to_owned(),
        quote: Some(quote),
    });
}

/// What fits at the cursor.
#[derive(Debug, Clone, PartialEq)]
enum Context {
    Nothing,
    /// The start of a statement: `from`, `let`.
    StatementStart,
    /// The value of a `let`: `from`, or a scalar expression.
    LetValue,
    /// An operator name, after `|`.
    Operator,
    /// A table: after `from` (`with_from`), or the right side of `join` and `union`.
    Source {
        with_from: bool,
    },
    /// A scalar expression over these columns; aggregates allowed or not.
    Expression {
        columns: Option<Vec<Col>>,
        aggregates: bool,
    },
    /// Column names only (`project-away`, `distinct`, render properties); `star` adds `*`.
    Columns {
        columns: Option<Vec<Col>>,
        star: bool,
    },
    /// Keywords that may follow what is written.
    Keywords(Vec<&'static str>),
    /// After a complete expression: these keywords, then the infix operators when `infix`.
    AfterExpression {
        keywords: Vec<&'static str>,
        infix: bool,
    },
    /// The chart kinds, after `render`.
    ChartKinds,
    /// Render property names for this kind (all if unknown).
    RenderProps(Option<ChartKind>),
    /// The words a render property takes, or its columns.
    RenderValue {
        prop: String,
        columns: Option<Vec<Col>>,
    },
    /// `join` kinds, after `kind =`.
    JoinKinds,
    /// Option names of a reader function.
    ReaderOptions(&'static crate::source::ReaderDef),
    /// The words a reader option takes.
    ReaderValues(OptionKind),
    /// The first argument of a reader: a path.
    ReaderPath,
    /// `join`: the `kind =` clause or the right table.
    JoinStart,
    /// Each of these.
    Both(Vec<Context>),
}

fn classify(doc: &Doc<'_>, cursor: &Cursor) -> Context {
    let Some(prev) = &cursor.prev else {
        return Context::StatementStart;
    };
    let parent = prev.parent();
    let parent_kind = parent.as_ref().map(SyntaxNode::kind);
    match prev.kind() {
        SyntaxKind::SEMICOLON if parent_kind == Some(SyntaxKind::PROGRAM) => {
            return Context::StatementStart;
        }
        SyntaxKind::PIPE => return Context::Operator,
        _ => {}
    }
    if cursor.hyphenated_operator {
        return Context::Operator;
    }
    if parent_kind == Some(SyntaxKind::PIPELINE)
        && prev.kind() == SyntaxKind::IDENT
        && prev.text() == "from"
        && parent
            .as_ref()
            .and_then(|p| p.first_token_non_trivia())
            .as_ref()
            == Some(prev)
    {
        return Context::Source { with_from: true };
    }
    // `(from ` with nothing after it parses `from` as a name.
    if parent_kind == Some(SyntaxKind::NAME_REF)
        && prev.text() == "from"
        && prev
            .prev_token()
            .and_then(skip_trivia_back)
            .is_some_and(|t| {
                t.kind() == SyntaxKind::L_PAREN
                    && t.parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::PAREN_EXPR)
            })
    {
        return Context::Source { with_from: true };
    }
    if parent_kind == Some(SyntaxKind::LET_STMT) && prev.kind() == SyntaxKind::EQ {
        return Context::LetValue;
    }
    // Inside a reader call's arguments.
    if let Some(context) = in_reader_call(prev) {
        return context;
    }
    let Some(op) = operator_of(prev) else {
        // A `let` value after something: an expression with no columns.
        if prev
            .parent_ancestors()
            .any(|n| n.kind() == SyntaxKind::LET_STMT)
            && !ends_expression(prev)
            && prev.kind() != SyntaxKind::LET_KW
            && parent_kind != Some(SyntaxKind::NAME)
        {
            return Context::Expression {
                columns: None,
                aggregates: false,
            };
        }
        return Context::Nothing;
    };
    let keywords = keyword_tokens(&op);
    let after_keyword = keywords.last() == Some(prev);
    let input = || doc.input_of(&op);
    match op.kind() {
        SyntaxKind::RENDER_OP => render_context(doc, &op, prev, after_keyword),
        SyntaxKind::JOIN_OP => join_context(doc, &op, prev, after_keyword),
        SyntaxKind::UNION_OP => {
            if after_keyword
                || (prev.kind() == SyntaxKind::COMMA && parent_kind == Some(SyntaxKind::UNION_OP))
            {
                Context::Source { with_from: false }
            } else {
                Context::Nothing
            }
        }
        SyntaxKind::COUNT_OP | SyntaxKind::GETSCHEMA_OP => Context::Nothing,
        SyntaxKind::PROJECT_AWAY_OP => {
            if after_keyword || prev.kind() == SyntaxKind::COMMA {
                Context::Columns {
                    columns: input(),
                    star: false,
                }
            } else {
                Context::Nothing
            }
        }
        SyntaxKind::DISTINCT_OP => {
            if after_keyword || prev.kind() == SyntaxKind::COMMA {
                Context::Columns {
                    columns: input(),
                    star: after_keyword,
                }
            } else {
                Context::Nothing
            }
        }
        SyntaxKind::PROJECT_RENAME_OP => {
            if prev.kind() == SyntaxKind::EQ {
                Context::Columns {
                    columns: input(),
                    star: false,
                }
            } else {
                Context::Nothing
            }
        }
        SyntaxKind::SORT_OP => {
            if after_keyword {
                Context::Keywords(vec!["by"])
            } else if ends_expression(prev) {
                Context::AfterExpression {
                    keywords: vec!["asc", "desc", "nulls first", "nulls last"],
                    infix: false,
                }
            } else {
                expression(input(), false)
            }
        }
        SyntaxKind::TOP_OP => {
            let has_by = op
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .any(|t| {
                    t.kind() == SyntaxKind::IDENT
                        && t.text() == "by"
                        && range(&t).end <= range(prev).end
                });
            if after_keyword {
                expression(input(), false)
            } else if ends_expression(prev) {
                Context::AfterExpression {
                    keywords: if has_by {
                        vec!["asc", "desc", "nulls first", "nulls last"]
                    } else {
                        vec!["by"]
                    },
                    infix: false,
                }
            } else {
                expression(input(), false)
            }
        }
        SyntaxKind::SUMMARIZE_OP => {
            let in_by = prev
                .parent_ancestors()
                .any(|n| n.kind() == SyntaxKind::BY_CLAUSE);
            if ends_expression(prev) {
                Context::AfterExpression {
                    keywords: if in_by { vec![] } else { vec!["by"] },
                    infix: true,
                }
            } else {
                expression(input(), !in_by)
            }
        }
        _ => {
            if ends_expression(prev) {
                Context::AfterExpression {
                    keywords: vec![],
                    infix: true,
                }
            } else {
                expression(input(), false)
            }
        }
    }
}

fn skip_trivia_back(mut token: SyntaxToken) -> Option<SyntaxToken> {
    while token.kind().is_trivia() {
        token = token.prev_token()?;
    }
    Some(token)
}

fn expression(columns: Option<Vec<Col>>, aggregates: bool) -> Context {
    Context::Expression {
        columns,
        aggregates,
    }
}

trait FirstToken {
    fn first_token_non_trivia(&self) -> Option<SyntaxToken>;
}

impl FirstToken for SyntaxNode {
    fn first_token_non_trivia(&self) -> Option<SyntaxToken> {
        self.descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.kind().is_trivia())
    }
}

/// Whether `token` can end a complete expression: a name, a literal or a closing bracket.
fn ends_expression(token: &SyntaxToken) -> bool {
    match token.kind() {
        SyntaxKind::IDENT => token
            .parent()
            .is_some_and(|p| matches!(p.kind(), SyntaxKind::NAME_REF | SyntaxKind::COLUMN_PATTERN)),
        SyntaxKind::QUOTED_IDENT
        | SyntaxKind::R_PAREN
        | SyntaxKind::R_BRACK
        | SyntaxKind::TRUE_KW
        | SyntaxKind::FALSE_KW
        | SyntaxKind::NULL_KW => true,
        kind => kind.is_literal(),
    }
}

fn render_context(
    doc: &Doc<'_>,
    op: &SyntaxNode,
    prev: &SyntaxToken,
    after_keyword: bool,
) -> Context {
    if after_keyword {
        return Context::ChartKinds;
    }
    let kind_token = op
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::IDENT)
        .nth(1);
    let kind = kind_token
        .as_ref()
        .and_then(|t| ChartKind::from_name(t.text()));
    if kind_token.as_ref() == Some(prev) {
        return Context::Keywords(vec!["with"]);
    }
    let parent = prev.parent();
    let parent_kind = parent.as_ref().map(SyntaxNode::kind);
    let prop_of = |node: &SyntaxNode| {
        node.ancestors()
            .find(|n| n.kind() == SyntaxKind::RENDER_PROP)
            .and_then(|p| p.children().find(|c| c.kind() == SyntaxKind::NAME))
            .and_then(|n| super::name_of(&n))
    };
    match prev.kind() {
        SyntaxKind::L_PAREN if parent_kind == Some(SyntaxKind::RENDER_PROPS) => {
            Context::RenderProps(kind)
        }
        SyntaxKind::COMMA => {
            // `ycolumns = a, ` may go on with columns, or start the next property. Before `)` or
            // the next `name =`, the comma sits in `RENDER_PROPS` right after that `RENDER_PROP`.
            let prop = parent.as_ref().and_then(prop_of).or_else(|| {
                let mut sibling = prev.prev_sibling_or_token();
                while let Some(el) = sibling {
                    if !el.kind().is_trivia() {
                        return el.into_node().and_then(|n| prop_of(&n));
                    }
                    sibling = el.prev_sibling_or_token();
                }
                None
            });
            match prop {
                Some(prop) if property_takes_columns(&prop) => Context::Both(vec![
                    Context::RenderValue {
                        prop,
                        columns: doc.input_of(op),
                    },
                    Context::RenderProps(kind),
                ]),
                _ => Context::RenderProps(kind),
            }
        }
        SyntaxKind::EQ if parent_kind == Some(SyntaxKind::RENDER_PROP) => {
            match parent.as_ref().and_then(prop_of) {
                Some(prop) => Context::RenderValue {
                    prop,
                    columns: doc.input_of(op),
                },
                None => Context::Nothing,
            }
        }
        _ => Context::Nothing,
    }
}

fn join_context(
    doc: &Doc<'_>,
    op: &SyntaxNode,
    prev: &SyntaxToken,
    after_keyword: bool,
) -> Context {
    if after_keyword {
        return Context::JoinStart;
    }
    let parent = prev.parent();
    if parent
        .as_ref()
        .is_some_and(|p| p.kind() == SyntaxKind::JOIN_KIND)
    {
        return match prev.kind() {
            SyntaxKind::EQ => Context::JoinKinds,
            SyntaxKind::IDENT if prev.text() == "kind" => Context::Keywords(vec!["="]),
            _ => Context::Source { with_from: false },
        };
    }
    let on = op
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::IDENT && t.text() == "on");
    match on {
        Some(on) if range(&on).start <= range(prev).start => {
            if ends_expression(prev) {
                Context::AfterExpression {
                    keywords: vec![],
                    infix: false,
                }
            } else {
                expression(doc.input_of(op), false)
            }
        }
        // After the right table.
        _ if ends_expression(prev) => Context::Keywords(vec!["on"]),
        _ => Context::Nothing,
    }
}

/// The context inside a reader call's argument list, if the cursor is in one.
fn in_reader_call(prev: &SyntaxToken) -> Option<Context> {
    let args = prev
        .parent_ancestors()
        .find(|n| n.kind() == SyntaxKind::ARG_LIST)?;
    let call = args
        .parent()
        .filter(|c| c.kind() == SyntaxKind::CALL_EXPR)?;
    // A closed argument list before the cursor is behind us.
    if prev.kind() == SyntaxKind::R_PAREN && prev.parent().as_ref() == Some(&args) {
        return None;
    }
    let reader = reader_of(&call)?;
    let parent = prev.parent()?;
    Some(match prev.kind() {
        SyntaxKind::L_PAREN if parent == args => Context::ReaderPath,
        SyntaxKind::COMMA if parent == args => Context::ReaderOptions(reader),
        SyntaxKind::EQ if parent.kind() == SyntaxKind::NAMED_EXPR => {
            let name = parent
                .children()
                .find(|c| c.kind() == SyntaxKind::NAME)
                .and_then(|n| super::name_of(&n));
            match name.and_then(|n| reader.option(&n)) {
                Some(option) => Context::ReaderValues(option.kind),
                None => Context::Nothing,
            }
        }
        _ => Context::Nothing,
    })
}

fn fill(out: &mut Completions, lets: &[LetInfo], context: Context) {
    let items = &mut out.items;
    match context {
        Context::Nothing => {}
        Context::Both(contexts) => {
            for context in contexts {
                fill(out, lets, context);
            }
        }
        Context::StatementStart => statement_start(out),
        Context::LetValue => {
            items.push(keyword("from"));
            expression_items(items, None, false, lets);
        }
        Context::Operator => {
            for name in OPERATOR_NAMES {
                let mut item = Completion::plain(*name, CompletionKind::Operator);
                if let Some(doc) = operator_doc(name) {
                    item = item.detail(doc);
                }
                items.push(item);
            }
        }
        Context::Source { with_from } => {
            source_items(items, lets, with_from);
            out.files = Some(FileCompletion {
                dir: String::new(),
                quote: None,
            });
        }
        Context::JoinStart => {
            items.push(
                Completion::plain("kind =", CompletionKind::Keyword).snippet("kind = ${1:inner} "),
            );
            source_items(items, lets, false);
            out.files = Some(FileCompletion {
                dir: String::new(),
                quote: None,
            });
        }
        Context::Expression {
            columns,
            aggregates,
        } => expression_items(items, columns.as_deref(), aggregates, lets),
        Context::Columns { columns, star } => {
            column_items(items, columns.as_deref());
            if star {
                items.push(Completion::plain("*", CompletionKind::Keyword).detail("every column"));
            }
        }
        Context::Keywords(words) => items.extend(words.into_iter().map(keyword)),
        Context::AfterExpression { keywords, infix } => {
            items.extend(keywords.into_iter().map(keyword));
            if infix {
                for word in INFIX {
                    items.push(keyword(word));
                }
            }
        }
        Context::ChartKinds => {
            for kind in ChartKind::ALL {
                items.push(
                    Completion::plain(kind.name(), CompletionKind::Value).detail("chart kind"),
                );
            }
        }
        Context::RenderProps(kind) => {
            for name in property_names(kind) {
                items.push(
                    Completion::plain(name, CompletionKind::Property)
                        .detail("render property")
                        .snippet(format!("{name} = $0")),
                );
            }
        }
        Context::RenderValue { prop, columns } => {
            if property_takes_columns(&prop) {
                column_items(items, columns.as_deref());
            }
            for word in property_words(&prop) {
                items.push(Completion::plain(*word, CompletionKind::Value));
            }
        }
        Context::JoinKinds => {
            for kind in JoinKind::ALL {
                items.push(
                    Completion::plain(kind.name(), CompletionKind::Value).detail("join kind"),
                );
            }
        }
        Context::ReaderOptions(reader) => {
            for option in reader.options.iter().filter(|o| o.planned.is_none()) {
                items.push(
                    Completion::plain(option.name, CompletionKind::Property)
                        .detail(option_detail(option.kind))
                        .snippet(format!("{} = $0", option.name)),
                );
            }
        }
        Context::ReaderValues(kind) => match kind {
            OptionKind::Bool => {
                items.push(keyword("true"));
                items.push(keyword("false"));
            }
            OptionKind::OneOf(words) => {
                for word in words {
                    items.push(Completion::plain(
                        format!("'{word}'"),
                        CompletionKind::Value,
                    ));
                }
            }
            _ => {}
        },
        Context::ReaderPath => {
            string_let_items(items, lets);
            out.files = Some(FileCompletion {
                dir: String::new(),
                quote: None,
            });
        }
    }
}

/// Infix operators offered after a complete expression.
const INFIX: &[&str] = &[
    "and",
    "or",
    "in",
    "between",
    "has",
    "contains",
    "startswith",
    "endswith",
    "matches regex",
];

fn statement_start(out: &mut Completions) {
    out.items.push(keyword("from"));
    out.items
        .push(Completion::plain("let", CompletionKind::Keyword).snippet("let ${1:name} = $0"));
}

fn keyword(word: &str) -> Completion {
    Completion::plain(word, CompletionKind::Keyword)
}

fn option_detail(kind: OptionKind) -> String {
    match kind {
        OptionKind::Bool => "reader option: true or false".into(),
        OptionKind::PositiveLong => "reader option: a whole number of 1 or more".into(),
        OptionKind::Char => "reader option: one character".into(),
        OptionKind::String => "reader option: a string".into(),
        OptionKind::OneOf(words) => format!("reader option: {}", words.join(", ")),
    }
}

fn column_items(items: &mut Vec<Completion>, columns: Option<&[Col]>) {
    for col in columns.unwrap_or_default() {
        items.push(
            Completion::plain(display_name(&col.name), CompletionKind::Column)
                .detail(col.ty.clone()),
        );
    }
}

fn expression_items(
    items: &mut Vec<Completion>,
    columns: Option<&[Col]>,
    aggregates: bool,
    lets: &[LetInfo],
) {
    column_items(items, columns);
    for l in lets {
        if let LetKind::Scalar { ty, .. } = &l.kind {
            items.push(
                Completion::plain(display_name(&l.name), CompletionKind::Variable)
                    .detail(ty.clone()),
            );
        }
    }
    let catalog = FunctionCatalog::builtin();
    for f in catalog.iter() {
        if crate::source::reader(f.name).is_some() {
            continue;
        }
        if f.kind == FunctionKind::Aggregate && !aggregates {
            continue;
        }
        items.push(function_item(f));
    }
    for word in ["true", "false", "null", "not"] {
        items.push(keyword(word));
    }
}

fn function_item(f: &FunctionDef) -> Completion {
    let no_args = f.signatures.iter().all(|s| s.max_args() == Some(0));
    let mut item = Completion::plain(f.name, CompletionKind::Function).snippet(if no_args {
        format!("{}()", f.name)
    } else {
        format!("{}($0)", f.name)
    });
    item.detail = f.signatures.first().map(|s| s.display(f.name));
    item.documentation = Some(f.doc.to_owned());
    item
}

fn string_let_items(items: &mut Vec<Completion>, lets: &[LetInfo]) {
    for l in lets {
        if let LetKind::Scalar {
            string: Some(path), ..
        } = &l.kind
        {
            items.push(
                Completion::plain(display_name(&l.name), CompletionKind::Variable)
                    .detail(format!("'{path}'")),
            );
        }
    }
}

/// Tables: tabular `let`s, scalar `let`s bound to strings (paths), and reader functions after
/// `from`. Files are listed by the frontend.
fn source_items(items: &mut Vec<Completion>, lets: &[LetInfo], with_from: bool) {
    for l in lets {
        if let LetKind::Table(columns) = &l.kind {
            let detail = match columns {
                Some(cs) => format!("table ({} columns)", cs.len()),
                None => "table".to_owned(),
            };
            items.push(
                Completion::plain(display_name(&l.name), CompletionKind::Table).detail(detail),
            );
        }
    }
    string_let_items(items, lets);
    if with_from {
        let catalog = FunctionCatalog::builtin();
        for reader in crate::source::READERS {
            let mut item = Completion::plain(reader.name, CompletionKind::Function)
                .snippet(format!("{}('$1')", reader.name));
            if let Ok(f) = catalog.lookup(reader.name) {
                item.detail = f.signatures.first().map(|s| s.display(f.name));
                item.documentation = Some(f.doc.to_owned());
            }
            items.push(item);
        }
    }
}
