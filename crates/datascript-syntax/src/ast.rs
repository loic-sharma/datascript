//! The typed AST: thin wrappers over CST nodes with accessor methods.
//!
//! Every accessor returns `Option` (or an iterator), because the tree of a broken program can lack
//! any part. Wrappers are cheap to clone; they hold a [`SyntaxNode`] and nothing else. Use
//! [`AstNode::syntax`] to get back to the node, for example for its text range.

use datascript_diagnostics::{FileId, Span};

use crate::SyntaxKind::{self, *};
use crate::literal;
use crate::syntax_tree::{SyntaxNode, SyntaxToken};

/// A typed view of a CST node.
pub trait AstNode: Sized {
    /// Whether nodes of this kind can be viewed as `Self`.
    fn can_cast(kind: SyntaxKind) -> bool;
    /// Views `node` as `Self`, or returns `None` if it has another kind.
    fn cast(node: SyntaxNode) -> Option<Self>;
    /// The underlying node.
    fn syntax(&self) -> &SyntaxNode;

    /// The node's span in `file`, trivia excluded at both ends.
    fn span(&self, file: FileId) -> Span {
        let range = self.syntax().text_range();
        Span::new(file, range.start().into(), range.end().into())
    }
}

macro_rules! ast_node {
    ($(#[$meta:meta])* $name:ident, $kind:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name {
            syntax: SyntaxNode,
        }

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == $kind
            }

            fn cast(syntax: SyntaxNode) -> Option<Self> {
                Self::can_cast(syntax.kind()).then_some(Self { syntax })
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.syntax
            }
        }
    };
}

macro_rules! ast_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident($ty:ident),)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant($ty),)*
        }

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                $($ty::can_cast(kind))||*
            }

            fn cast(syntax: SyntaxNode) -> Option<Self> {
                let kind = syntax.kind();
                $(if $ty::can_cast(kind) {
                    return $ty::cast(syntax).map($name::$variant);
                })*
                None
            }

            fn syntax(&self) -> &SyntaxNode {
                match self {
                    $($name::$variant(node) => node.syntax(),)*
                }
            }
        }
    };
}

fn child<N: AstNode>(parent: &SyntaxNode) -> Option<N> {
    parent.children().find_map(N::cast)
}

fn children<N: AstNode>(parent: &SyntaxNode) -> impl Iterator<Item = N> + use<N> {
    parent.children().filter_map(N::cast)
}

fn tokens(parent: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> + use<> {
    parent
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
}

fn token(parent: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    tokens(parent).find(|t| t.kind() == kind)
}

/// The direct child identifier token with this text: a contextual keyword such as `by`.
fn keyword(parent: &SyntaxNode, kw: &str) -> Option<SyntaxToken> {
    tokens(parent).find(|t| t.kind() == IDENT && t.text() == kw)
}

/// The text of a name token, with `['...']` quoting removed.
fn name_text(token: &SyntaxToken) -> Option<String> {
    match token.kind() {
        IDENT => Some(token.text().to_string()),
        QUOTED_IDENT => {
            let range = token.text_range();
            let span = Span::new(FileId(0), range.start().into(), range.end().into());
            literal::quoted_ident(token.text(), span).ok()
        }
        DOLLAR_LEFT | DOLLAR_RIGHT => Some(token.text().to_string()),
        _ => None,
    }
}

// --- Statements ---

ast_node!(
    /// The whole file: a sequence of statements.
    Program,
    PROGRAM
);

impl Program {
    /// The statements, in order.
    pub fn statements(&self) -> impl Iterator<Item = Stmt> + use<> {
        children(&self.syntax)
    }
}

ast_node!(
    /// `let name = value`. The value is a scalar or a table; the binder decides (D-020).
    LetStmt,
    LET_STMT
);

impl LetStmt {
    /// The name being defined.
    pub fn name(&self) -> Option<Name> {
        child(&self.syntax)
    }

    /// The right side, which may be a [`Pipeline`].
    pub fn value(&self) -> Option<Expr> {
        child(&self.syntax)
    }
}

ast_enum!(
    /// A statement.
    Stmt {
        /// `let name = value`
        Let(LetStmt),
        /// A query: a pipeline whose result is shown.
        Query(Pipeline),
    }
);

ast_node!(
    /// `source | op | op ...`. A query statement is always a pipeline, even with no operators.
    Pipeline,
    PIPELINE
);

impl Pipeline {
    /// The source: a path string, a name, a reader call, a parenthesized pipeline, or (in a broken
    /// program) any other expression, which the binder rejects.
    pub fn source(&self) -> Option<Expr> {
        child(&self.syntax)
    }

    /// The operators, in order. Operators that did not parse are left out.
    pub fn operators(&self) -> impl Iterator<Item = Operator> + use<> {
        children(&self.syntax)
    }
}

// --- Operators ---

ast_enum!(
    /// A tabular operator after `|`.
    Operator {
        Where(WhereOp),
        Project(ProjectOp),
        ProjectAway(ProjectAwayOp),
        ProjectRename(ProjectRenameOp),
        Extend(ExtendOp),
        Summarize(SummarizeOp),
        Sort(SortOp),
        Top(TopOp),
        Take(TakeOp),
        Count(CountOp),
        Distinct(DistinctOp),
        Join(JoinOp),
        Union(UnionOp),
        Render(RenderOp),
        GetSchema(GetSchemaOp),
    }
);

impl Operator {
    /// The operator's name: `where`, `project-away`, `order`, ….
    pub fn name(&self) -> String {
        let mut toks = tokens(self.syntax());
        let Some(first) = toks.next() else {
            return String::new();
        };
        // A hyphenated name is three tokens; the parser puts no other `-` directly in an
        // operator node. They are adjacent unless the parser reported `project - away` (DS0104).
        if let (Some(minus), Some(second)) = (toks.next(), toks.next())
            && minus.kind() == MINUS
            && second.kind() == IDENT
        {
            return format!("{}-{}", first.text(), second.text());
        }
        first.text().to_string()
    }
}

ast_node!(
    /// `where predicate`
    WhereOp,
    WHERE_OP
);

impl WhereOp {
    /// The condition rows must meet.
    pub fn predicate(&self) -> Option<Expr> {
        child(&self.syntax)
    }
}

ast_node!(
    /// `project a, b = expr`
    ProjectOp,
    PROJECT_OP
);

impl ProjectOp {
    /// The output columns, each an expression with an optional name.
    pub fn items(&self) -> impl Iterator<Item = NamedExpr> + use<> {
        children(&self.syntax)
    }
}

ast_node!(
    /// `project-away a, b*`
    ProjectAwayOp,
    PROJECT_AWAY_OP
);

impl ProjectAwayOp {
    /// The columns to remove, as names or `*` patterns.
    pub fn patterns(&self) -> impl Iterator<Item = ColumnPattern> + use<> {
        children(&self.syntax)
    }
}

ast_node!(
    /// `project-rename new = old`
    ProjectRenameOp,
    PROJECT_RENAME_OP
);

impl ProjectRenameOp {
    /// The `new = old` renames.
    pub fn items(&self) -> impl Iterator<Item = RenameItem> + use<> {
        children(&self.syntax)
    }
}

ast_node!(
    /// `extend c = expr`
    ExtendOp,
    EXTEND_OP
);

impl ExtendOp {
    /// The columns to add or replace, each an expression with an optional name.
    pub fn items(&self) -> impl Iterator<Item = NamedExpr> + use<> {
        children(&self.syntax)
    }
}

ast_node!(
    /// `summarize [aggregates] [by keys]`
    SummarizeOp,
    SUMMARIZE_OP
);

impl SummarizeOp {
    /// The aggregates before `by`.
    pub fn aggregates(&self) -> impl Iterator<Item = NamedExpr> + use<> {
        children(&self.syntax)
    }

    /// The `by` clause, if any.
    pub fn by(&self) -> Option<ByClause> {
        child(&self.syntax)
    }
}

ast_node!(
    /// `by key, ...` in `summarize`.
    ByClause,
    BY_CLAUSE
);

impl ByClause {
    /// The grouping keys after `by`.
    pub fn keys(&self) -> impl Iterator<Item = NamedExpr> + use<> {
        children(&self.syntax)
    }
}

ast_node!(
    /// `sort by keys` or `order by keys`
    SortOp,
    SORT_OP
);

impl SortOp {
    /// The sort keys, in order of priority.
    pub fn keys(&self) -> impl Iterator<Item = SortKey> + use<> {
        children(&self.syntax)
    }

    /// The `by` keyword, or `None` if it is missing.
    pub fn by_token(&self) -> Option<SyntaxToken> {
        keyword(&self.syntax, "by")
    }
}

ast_node!(
    /// `top n by key`
    TopOp,
    TOP_OP
);

impl TopOp {
    /// The number of rows.
    pub fn count(&self) -> Option<Expr> {
        child(&self.syntax)
    }

    /// The sort key after `by`.
    pub fn key(&self) -> Option<SortKey> {
        child(&self.syntax)
    }
}

ast_node!(
    /// `take n` or `limit n`
    TakeOp,
    TAKE_OP
);

impl TakeOp {
    /// The number of rows.
    pub fn count(&self) -> Option<Expr> {
        child(&self.syntax)
    }
}

ast_node!(
    /// `count`
    CountOp,
    COUNT_OP
);

ast_node!(
    /// `getschema`
    GetSchemaOp,
    GETSCHEMA_OP
);

ast_node!(
    /// `distinct a, b` or `distinct *`
    DistinctOp,
    DISTINCT_OP
);

impl DistinctOp {
    /// The `*` of `distinct *`.
    pub fn star_token(&self) -> Option<SyntaxToken> {
        token(&self.syntax, STAR)
    }

    /// The columns to deduplicate on; empty for `distinct *`.
    pub fn columns(&self) -> impl Iterator<Item = Expr> + use<> {
        children(&self.syntax)
    }
}

ast_node!(
    /// `join [kind = k] right on conditions`
    JoinOp,
    JOIN_OP
);

impl JoinOp {
    /// The join kind from `kind = ...`, if written.
    pub fn kind(&self) -> Option<JoinKind> {
        child(&self.syntax)
    }

    /// The right table: a name, a path string, or a parenthesized pipeline.
    pub fn right(&self) -> Option<Expr> {
        let on = keyword(&self.syntax, "on").map(|t| t.text_range().start());
        child::<Expr>(&self.syntax)
            .filter(|e| on.is_none_or(|on| e.syntax().text_range().start() < on))
    }

    /// The conditions after `on`: column names, or `$left.a == $right.b`.
    pub fn conditions(&self) -> impl Iterator<Item = Expr> + use<> {
        let on = keyword(&self.syntax, "on").map(|t| t.text_range().end());
        children::<Expr>(&self.syntax)
            .filter(move |e| on.is_some_and(|on| e.syntax().text_range().start() >= on))
    }
}

ast_node!(
    /// `kind = inner` in `join`.
    JoinKind,
    JOIN_KIND
);

impl JoinKind {
    /// The kind after `=`, such as `inner`.
    pub fn value_token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).filter(|t| t.kind() == IDENT).nth(1)
    }
}

ast_node!(
    /// `union t1, t2`
    UnionOp,
    UNION_OP
);

impl UnionOp {
    /// The tables to append, in order.
    pub fn tables(&self) -> impl Iterator<Item = Expr> + use<> {
        children(&self.syntax)
    }
}

ast_node!(
    /// `render kind [with (props)]`
    RenderOp,
    RENDER_OP
);

impl RenderOp {
    /// The chart kind, such as `timechart`.
    pub fn kind_token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).filter(|t| t.kind() == IDENT).nth(1)
    }

    /// The properties inside `with (...)`.
    pub fn properties(&self) -> impl Iterator<Item = RenderProp> + use<> {
        child::<RenderProps>(&self.syntax)
            .into_iter()
            .flat_map(|props| children(&props.syntax))
    }
}

ast_node!(
    /// `with (prop = value, ...)` in `render`.
    RenderProps,
    RENDER_PROPS
);

ast_node!(
    /// `prop = value [, value ...]` in `render ... with (...)`.
    RenderProp,
    RENDER_PROP
);

impl RenderProp {
    /// The property name before `=`.
    pub fn name(&self) -> Option<Name> {
        child(&self.syntax)
    }

    /// The values: one, or several for list properties such as `ycolumns`.
    pub fn values(&self) -> impl Iterator<Item = Expr> + use<> {
        children(&self.syntax)
    }
}

// --- Operator parts ---

ast_node!(
    /// `[name =] expr`: a projection, aggregate, `by` key, or named call argument.
    NamedExpr,
    NAMED_EXPR
);

impl NamedExpr {
    /// The explicit name, if any.
    pub fn name(&self) -> Option<Name> {
        child(&self.syntax)
    }

    /// The value expression.
    pub fn expr(&self) -> Option<Expr> {
        child(&self.syntax)
    }
}

ast_node!(
    /// A column name with optional `*` wildcards: `a`, `col*`, `*_id`.
    ColumnPattern,
    COLUMN_PATTERN
);

impl ColumnPattern {
    /// The pattern as written.
    pub fn text(&self) -> String {
        self.syntax.text().to_string()
    }

    /// Whether the pattern has a `*`.
    pub fn has_wildcard(&self) -> bool {
        token(&self.syntax, STAR).is_some()
    }
}

ast_node!(
    /// `new = old` in `project-rename`.
    RenameItem,
    RENAME_ITEM
);

impl RenameItem {
    /// The new column name, before `=`.
    pub fn new_name(&self) -> Option<Name> {
        child(&self.syntax)
    }

    /// The column being renamed, after `=`.
    pub fn old_name(&self) -> Option<NameRef> {
        child(&self.syntax)
    }
}

ast_node!(
    /// `expr [asc | desc] [nulls first | nulls last]`
    SortKey,
    SORT_KEY
);

/// `asc` or `desc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// `nulls first` or `nulls last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NullsPlacement {
    First,
    Last,
}

impl SortKey {
    /// The expression to sort by.
    pub fn expr(&self) -> Option<Expr> {
        child(&self.syntax)
    }

    /// The `asc` or `desc` token, if written.
    pub fn direction_token(&self) -> Option<SyntaxToken> {
        // Any case: `DESC` is reported (DS0105) but still means descending.
        tokens(&self.syntax).find(|t| {
            t.kind() == IDENT
                && (t.text().eq_ignore_ascii_case("asc") || t.text().eq_ignore_ascii_case("desc"))
        })
    }

    /// The direction, if written. The default depends on the operator.
    pub fn direction(&self) -> Option<SortDirection> {
        self.direction_token().map(|t| {
            if t.text().eq_ignore_ascii_case("asc") {
                SortDirection::Asc
            } else {
                SortDirection::Desc
            }
        })
    }

    /// `nulls first` or `nulls last`, if written.
    pub fn nulls(&self) -> Option<NullsPlacement> {
        let mut after_nulls = false;
        for t in tokens(&self.syntax) {
            match t.text() {
                "nulls" => after_nulls = true,
                "first" if after_nulls => return Some(NullsPlacement::First),
                "last" if after_nulls => return Some(NullsPlacement::Last),
                _ => {}
            }
        }
        None
    }
}

ast_node!(
    /// `(arg, name = arg, ...)`: call arguments, or the list after `in`.
    ArgList,
    ARG_LIST
);

/// An argument in an [`ArgList`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Arg {
    Positional(Expr),
    Named(NamedExpr),
}

impl ArgList {
    /// The arguments in order, positional or named.
    pub fn args(&self) -> impl Iterator<Item = Arg> + use<> {
        self.syntax.children().filter_map(|node| {
            if let Some(named) = NamedExpr::cast(node.clone()) {
                Some(Arg::Named(named))
            } else {
                Expr::cast(node).map(Arg::Positional)
            }
        })
    }
}

ast_node!(
    /// A name being defined: a `let` name, an output column name, a property name.
    Name,
    NAME
);

impl Name {
    /// The name's token (an identifier, or a quoted `['...']` name).
    pub fn token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).next()
    }

    /// The name, with `['...']` quoting removed.
    pub fn text(&self) -> Option<String> {
        name_text(&self.token()?)
    }
}

// --- Expressions ---

ast_enum!(
    /// A scalar or tabular expression.
    Expr {
        Literal(Literal),
        TypedNull(TypedNull),
        NameRef(NameRef),
        Paren(ParenExpr),
        Call(CallExpr),
        Field(FieldExpr),
        Index(IndexExpr),
        Prefix(PrefixExpr),
        Bin(BinExpr),
        In(InExpr),
        Between(BetweenExpr),
        /// A pipeline in expression position: the right side of a `let`, or inside parentheses.
        Pipeline(Pipeline),
    }
);

ast_node!(
    /// A literal: number, string, timespan, datetime, `true`, `false` or `null`.
    Literal,
    LITERAL
);

/// What kind of literal a [`Literal`] is. Read the value with the functions in
/// [`crate::literal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiteralKind {
    Long,
    Real,
    String,
    Timespan,
    Datetime,
    Bool,
    Null,
}

impl Literal {
    /// The literal's token.
    pub fn token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).next()
    }

    /// What kind of literal this is, from its token.
    pub fn kind(&self) -> Option<LiteralKind> {
        Some(match self.token()?.kind() {
            LONG => LiteralKind::Long,
            REAL => LiteralKind::Real,
            STRING => LiteralKind::String,
            TIMESPAN => LiteralKind::Timespan,
            DATETIME => LiteralKind::Datetime,
            TRUE_KW | FALSE_KW => LiteralKind::Bool,
            NULL_KW => LiteralKind::Null,
            _ => return None,
        })
    }
}

ast_node!(
    /// `long(null)`, `string(null)`, …
    TypedNull,
    TYPED_NULL
);

impl TypedNull {
    /// The type name token, such as `long`.
    pub fn type_token(&self) -> Option<SyntaxToken> {
        token(&self.syntax, IDENT)
    }
}

ast_node!(
    /// A reference to a column, `let`, table, `$left` or `$right`.
    NameRef,
    NAME_REF
);

impl NameRef {
    /// The referenced name's token.
    pub fn token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).next()
    }

    /// The name, with `['...']` quoting removed.
    pub fn text(&self) -> Option<String> {
        name_text(&self.token()?)
    }
}

ast_node!(
    /// `( inner )`
    ParenExpr,
    PAREN_EXPR
);

impl ParenExpr {
    /// The expression inside, which may be a [`Pipeline`].
    pub fn inner(&self) -> Option<Expr> {
        child(&self.syntax)
    }
}

ast_node!(
    /// `f(args)`
    CallExpr,
    CALL_EXPR
);

impl CallExpr {
    /// The function's name. `None` for a method-style call `a.f(x)` (DS0113), whose first child is
    /// the member access.
    pub fn callee(&self) -> Option<NameRef> {
        child(&self.syntax)
    }

    /// The parenthesized argument list.
    pub fn arg_list(&self) -> Option<ArgList> {
        child(&self.syntax)
    }

    /// The arguments, or nothing if the list is missing.
    pub fn args(&self) -> impl Iterator<Item = Arg> + use<> {
        self.arg_list().into_iter().flat_map(|list| list.args())
    }
}

ast_node!(
    /// `base.field`
    FieldExpr,
    FIELD_EXPR
);

impl FieldExpr {
    /// The expression before `.`.
    pub fn base(&self) -> Option<Expr> {
        child(&self.syntax)
    }

    /// The name after `.`.
    pub fn field(&self) -> Option<NameRef> {
        let dot = token(&self.syntax, DOT)?.text_range().end();
        children::<NameRef>(&self.syntax).find(|n| n.syntax().text_range().start() >= dot)
    }
}

ast_node!(
    /// `base[index]`
    IndexExpr,
    INDEX_EXPR
);

impl IndexExpr {
    /// The expression before `[`.
    pub fn base(&self) -> Option<Expr> {
        child(&self.syntax)
    }

    /// The expression inside the brackets.
    pub fn index(&self) -> Option<Expr> {
        children(&self.syntax).nth(1)
    }
}

ast_node!(
    /// `-x` or `not x`
    PrefixExpr,
    PREFIX_EXPR
);

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrefixOp {
    /// `-x`
    Neg,
    /// `not x` or `not(x)`
    Not,
}

impl PrefixExpr {
    /// The operator token: `not`, `-` or `!` (reported as DS0111).
    pub fn op_token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).next()
    }

    /// The operator, read from its token.
    pub fn op(&self) -> Option<PrefixOp> {
        match self.op_token()?.kind() {
            MINUS => Some(PrefixOp::Neg),
            // `!x` is reported by the parser (DS0111) and read as `not(x)`.
            NOT_KW | BANG => Some(PrefixOp::Not),
            _ => None,
        }
    }

    /// The expression the operator applies to.
    pub fn operand(&self) -> Option<Expr> {
        child(&self.syntax)
    }
}

ast_node!(
    /// `lhs op rhs`
    BinExpr,
    BIN_EXPR
);

/// A string operator. Case-insensitive unless the name ends in `_cs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringOp {
    Has,
    HasCs,
    Contains,
    ContainsCs,
    StartsWith,
    StartsWithCs,
    EndsWith,
    EndsWithCs,
}

impl StringOp {
    fn from_name(name: &str) -> Option<StringOp> {
        Some(match name {
            "has" => StringOp::Has,
            "has_cs" => StringOp::HasCs,
            "contains" => StringOp::Contains,
            "contains_cs" => StringOp::ContainsCs,
            "startswith" => StringOp::StartsWith,
            "startswith_cs" => StringOp::StartsWithCs,
            "endswith" => StringOp::EndsWith,
            "endswith_cs" => StringOp::EndsWithCs,
            _ => return None,
        })
    }
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Or,
    And,
    /// `==`
    Eq,
    /// `!=`
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    /// `=~`, case-insensitive equality.
    EqCi,
    /// `!~`
    NotEqCi,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    /// `has`, `!contains_cs`, …
    String {
        op: StringOp,
        negated: bool,
    },
    /// `matches regex`
    MatchesRegex,
}

impl BinExpr {
    /// The left operand.
    pub fn lhs(&self) -> Option<Expr> {
        child(&self.syntax)
    }

    /// The right operand.
    pub fn rhs(&self) -> Option<Expr> {
        children(&self.syntax).nth(1)
    }

    /// The operator token (the first one, `matches`, for `matches regex`).
    pub fn op_token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).next()
    }

    /// The operator, read from its token. Habits (`&&`, `||`, `=`, `<>`) read as the operator meant.
    pub fn op(&self) -> Option<BinaryOp> {
        let token = self.op_token()?;
        Some(match token.kind() {
            // `||`, `&&`, `=` and `<>` are reported by the parser (DS0111, DS0110, DS0107) and
            // read as the operator the user meant.
            OR_KW | PIPE2 => BinaryOp::Or,
            AND_KW | AMP2 => BinaryOp::And,
            EQ2 | EQ => BinaryOp::Eq,
            NEQ | LTGT => BinaryOp::NotEq,
            LT => BinaryOp::Lt,
            LTEQ => BinaryOp::LtEq,
            GT => BinaryOp::Gt,
            GTEQ => BinaryOp::GtEq,
            EQ_TILDE => BinaryOp::EqCi,
            BANG_TILDE => BinaryOp::NotEqCi,
            PLUS => BinaryOp::Add,
            MINUS => BinaryOp::Sub,
            STAR => BinaryOp::Mul,
            SLASH => BinaryOp::Div,
            PERCENT => BinaryOp::Mod,
            NEGATED_OP => BinaryOp::String {
                op: StringOp::from_name(token.text().strip_prefix('!')?)?,
                negated: true,
            },
            IDENT if token.text() == "matches" => BinaryOp::MatchesRegex,
            IDENT => BinaryOp::String {
                op: StringOp::from_name(token.text())?,
                negated: false,
            },
            _ => return None,
        })
    }
}

ast_node!(
    /// `x in (a, b)` and its negated and case-insensitive forms.
    InExpr,
    IN_EXPR
);

impl InExpr {
    /// The value tested for membership.
    pub fn lhs(&self) -> Option<Expr> {
        child(&self.syntax)
    }

    /// The operator token: `in`, `!in`, `in~` or `!in~`.
    pub fn op_token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).next()
    }

    /// Whether this is `!in` or `!in~`.
    pub fn negated(&self) -> bool {
        self.op_token().is_some_and(|t| t.text().starts_with('!'))
    }

    /// Whether this is `in~` or `!in~`.
    pub fn case_insensitive(&self) -> bool {
        self.op_token().is_some_and(|t| t.text().ends_with('~'))
    }

    /// The list of values.
    pub fn list(&self) -> Option<ArgList> {
        child(&self.syntax)
    }
}

ast_node!(
    /// `x between (lo .. hi)` or `x !between (lo .. hi)`
    BetweenExpr,
    BETWEEN_EXPR
);

impl BetweenExpr {
    /// The value tested against the range.
    pub fn lhs(&self) -> Option<Expr> {
        child(&self.syntax)
    }

    /// The operator token: `between` or `!between`.
    pub fn op_token(&self) -> Option<SyntaxToken> {
        tokens(&self.syntax).next()
    }

    /// Whether this is `!between`.
    pub fn negated(&self) -> bool {
        self.op_token().is_some_and(|t| t.text().starts_with('!'))
    }

    /// The lower bound: the expression between `(` and `..`.
    pub fn low(&self) -> Option<Expr> {
        let open = token(&self.syntax, L_PAREN)?.text_range().end();
        let dots = token(&self.syntax, DOT2).map(|t| t.text_range().start());
        children::<Expr>(&self.syntax).find(|e| {
            let start = e.syntax().text_range().start();
            start >= open && dots.is_none_or(|d| start < d)
        })
    }

    /// The upper bound: the expression after `..`.
    pub fn high(&self) -> Option<Expr> {
        let dots = token(&self.syntax, DOT2)?.text_range().end();
        children::<Expr>(&self.syntax).find(|e| e.syntax().text_range().start() >= dots)
    }
}
