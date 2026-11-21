//! Operator typing (LANGUAGE.md §4.1) as data, and the diagnostics for operators applied to the
//! wrong types (`DS2003`) and untyped `null`s (`DS2009`).

use datascript_diagnostics::{Diagnostic, Span, codes};
use datascript_syntax::ast::{BinaryOp, PrefixOp, StringOp};

use crate::types::{Type, TypeClass};

/// Operators grouped by how they are typed. `has`, `!contains_cs`, `matches regex`, … are one
/// group: they all take two strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpGroup {
    Or,
    And,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    EqCi,
    NotEqCi,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    StringOp,
}

impl OpGroup {
    /// The group of a binary operator.
    pub fn of(op: BinaryOp) -> OpGroup {
        match op {
            BinaryOp::Or => OpGroup::Or,
            BinaryOp::And => OpGroup::And,
            BinaryOp::Eq => OpGroup::Eq,
            BinaryOp::NotEq => OpGroup::NotEq,
            BinaryOp::Lt => OpGroup::Lt,
            BinaryOp::LtEq => OpGroup::LtEq,
            BinaryOp::Gt => OpGroup::Gt,
            BinaryOp::GtEq => OpGroup::GtEq,
            BinaryOp::EqCi => OpGroup::EqCi,
            BinaryOp::NotEqCi => OpGroup::NotEqCi,
            BinaryOp::Add => OpGroup::Add,
            BinaryOp::Sub => OpGroup::Sub,
            BinaryOp::Mul => OpGroup::Mul,
            BinaryOp::Div => OpGroup::Div,
            BinaryOp::Mod => OpGroup::Mod,
            BinaryOp::String { .. } | BinaryOp::MatchesRegex => OpGroup::StringOp,
        }
    }

    /// Whether the result is always `bool`, whatever the operands.
    pub fn is_predicate(self) -> bool {
        !matches!(
            self,
            OpGroup::Add | OpGroup::Sub | OpGroup::Mul | OpGroup::Div | OpGroup::Mod
        )
    }
}

/// How the operands of a matching rule are converted before the operator is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operands {
    /// Both operands keep their types.
    Keep,
    /// Both operands convert to this type.
    To(Type),
    /// Both operands convert to their common type ([`Type::common`]), which must exist.
    Common,
}

/// One row of the operator typing table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryRule {
    /// The operators the row applies to.
    pub ops: &'static [OpGroup],
    /// Types accepted on the left.
    pub left: TypeClass,
    /// Types accepted on the right.
    pub right: TypeClass,
    /// How the operands are converted.
    pub operands: Operands,
    /// The result type.
    pub result: Type,
}

const fn rule(
    ops: &'static [OpGroup],
    left: TypeClass,
    right: TypeClass,
    operands: Operands,
    result: Type,
) -> BinaryRule {
    BinaryRule {
        ops,
        left,
        right,
        operands,
        result,
    }
}

use OpGroup as G;
use TypeClass::{Any, Comparable, Exact, Integer, Numeric};

const TIMESPAN: TypeClass = Exact(Type::TimeSpan);
const DATETIME: TypeClass = Exact(Type::DateTime);
const STRING: TypeClass = Exact(Type::String);
const BOOL: TypeClass = Exact(Type::Bool);

/// LANGUAGE.md §4.1, in order. The first matching row wins, so the integer rows come before the
/// general numeric ones. Integer arithmetic yields `long` (D-021).
pub const BINARY_RULES: &[BinaryRule] = &[
    rule(
        &[G::Add, G::Sub, G::Mul],
        Integer,
        Integer,
        Operands::To(Type::Long),
        Type::Long,
    ),
    rule(
        &[G::Add, G::Sub, G::Mul],
        Numeric,
        Numeric,
        Operands::To(Type::Real),
        Type::Real,
    ),
    rule(
        &[G::Div],
        Integer,
        Integer,
        Operands::To(Type::Long),
        Type::Long,
    ),
    rule(
        &[G::Div],
        Numeric,
        Numeric,
        Operands::To(Type::Real),
        Type::Real,
    ),
    rule(
        &[G::Mod],
        Integer,
        Integer,
        Operands::To(Type::Long),
        Type::Long,
    ),
    rule(
        &[G::Add, G::Sub],
        TIMESPAN,
        TIMESPAN,
        Operands::Keep,
        Type::TimeSpan,
    ),
    rule(
        &[G::Add, G::Sub],
        DATETIME,
        TIMESPAN,
        Operands::Keep,
        Type::DateTime,
    ),
    rule(
        &[G::Add],
        TIMESPAN,
        DATETIME,
        Operands::Keep,
        Type::DateTime,
    ),
    rule(
        &[G::Sub],
        DATETIME,
        DATETIME,
        Operands::Keep,
        Type::TimeSpan,
    ),
    rule(
        &[G::Mul, G::Div],
        TIMESPAN,
        Numeric,
        Operands::Keep,
        Type::TimeSpan,
    ),
    rule(&[G::Mul], Numeric, TIMESPAN, Operands::Keep, Type::TimeSpan),
    rule(&[G::Div], TIMESPAN, TIMESPAN, Operands::Keep, Type::Real),
    rule(&[G::Eq, G::NotEq], Any, Any, Operands::Common, Type::Bool),
    rule(
        &[G::Lt, G::LtEq, G::Gt, G::GtEq],
        Comparable,
        Comparable,
        Operands::Common,
        Type::Bool,
    ),
    rule(
        &[G::EqCi, G::NotEqCi, G::StringOp],
        STRING,
        STRING,
        Operands::Keep,
        Type::Bool,
    ),
    rule(&[G::And, G::Or], BOOL, BOOL, Operands::Keep, Type::Bool),
];

/// The types of a well-typed binary expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryTyping {
    /// The type the left operand converts to before the operator is applied.
    pub left: Type,
    /// The type the right operand converts to.
    pub right: Type,
    /// The type of the expression.
    pub result: Type,
}

/// A binary operator applied to types it does not accept (`DS2003`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryTypeError {
    /// The operator.
    pub op: BinaryOp,
    /// The left operand's type.
    pub left: Type,
    /// The right operand's type.
    pub right: Type,
}

/// Types `left op right` (LANGUAGE.md §4.1).
///
/// - An operand of type [`Type::Error`] short-circuits: the result is `bool` for predicates and
///   `Error` otherwise, and nothing is reported.
/// - An untyped `null` takes the type the other operand's row needs: `null + 1` is `long`,
///   `dt - null` is `datetime` (the first matching row decides). With two `null`s, predicates
///   give `bool` and arithmetic gives [`Type::Null`], which the binder reports as `DS2009` if
///   nothing else gives it a type.
pub fn binary(op: BinaryOp, left: Type, right: Type) -> Result<BinaryTyping, BinaryTypeError> {
    let group = OpGroup::of(op);
    if left.is_error() || right.is_error() {
        let result = if group.is_predicate() {
            Type::Bool
        } else {
            Type::Error
        };
        return Ok(BinaryTyping {
            left,
            right,
            result,
        });
    }
    if left.is_null() && right.is_null() {
        let result = if group.is_predicate() {
            Type::Bool
        } else {
            Type::Null
        };
        return Ok(BinaryTyping {
            left,
            right,
            result,
        });
    }
    for rule in BINARY_RULES.iter().filter(|rule| rule.ops.contains(&group)) {
        if !rule.left.accepts(left) || !rule.right.accepts(right) {
            continue;
        }
        let typing = match rule.operands {
            Operands::To(ty) => BinaryTyping {
                left: ty,
                right: ty,
                result: rule.result,
            },
            Operands::Common => match left.common(right) {
                Some(ty) => BinaryTyping {
                    left: ty,
                    right: ty,
                    result: rule.result,
                },
                None => continue,
            },
            Operands::Keep => BinaryTyping {
                left: resolve_null(left, rule.left, right),
                right: resolve_null(right, rule.right, left),
                result: rule.result,
            },
        };
        return Ok(typing);
    }
    Err(BinaryTypeError { op, left, right })
}

/// The type an untyped `null` operand takes in a row that keeps operand types.
fn resolve_null(ty: Type, class: TypeClass, other: Type) -> Type {
    match (ty, class) {
        (Type::Null, Exact(target)) => target,
        (Type::Null, _) => other,
        _ => ty,
    }
}

/// The types of a well-typed prefix expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnaryTyping {
    /// The type the operand converts to before the operator is applied.
    pub operand: Type,
    /// The type of the expression.
    pub result: Type,
}

/// A prefix operator applied to a type it does not accept (`DS2003`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnaryTypeError {
    /// The operator.
    pub op: PrefixOp,
    /// The operand's type.
    pub operand: Type,
}

/// Types `-x` and `not x` (LANGUAGE.md §4.1). `-` of an integer is `long`; `-` of `real` or
/// `timespan` keeps the type. `Error` short-circuits as in [`binary`]. `-null` stays
/// [`Type::Null`]; `not null` is `bool`.
pub fn unary(op: PrefixOp, operand: Type) -> Result<UnaryTyping, UnaryTypeError> {
    let typing = |operand, result| Ok(UnaryTyping { operand, result });
    match (op, operand) {
        (PrefixOp::Not, Type::Bool | Type::Null | Type::Error) => typing(operand, Type::Bool),
        (PrefixOp::Neg, Type::Int | Type::Long) => typing(Type::Long, Type::Long),
        (PrefixOp::Neg, Type::Real | Type::TimeSpan | Type::Null | Type::Error) => {
            typing(operand, operand)
        }
        _ => Err(UnaryTypeError { op, operand }),
    }
}

/// An operand as written, for building diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct Operand<'a> {
    /// Where the operand is.
    pub span: Span,
    /// Its source text, used in suggested rewrites.
    pub text: &'a str,
}

/// How a binary operator is spelled.
pub fn op_symbol(op: BinaryOp) -> String {
    let symbol = match op {
        BinaryOp::Or => "or",
        BinaryOp::And => "and",
        BinaryOp::Eq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::LtEq => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::GtEq => ">=",
        BinaryOp::EqCi => "=~",
        BinaryOp::NotEqCi => "!~",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::MatchesRegex => "matches regex",
        BinaryOp::String { op, negated } => {
            let name = match op {
                StringOp::Has => "has",
                StringOp::HasCs => "has_cs",
                StringOp::Contains => "contains",
                StringOp::ContainsCs => "contains_cs",
                StringOp::StartsWith => "startswith",
                StringOp::StartsWithCs => "startswith_cs",
                StringOp::EndsWith => "endswith",
                StringOp::EndsWithCs => "endswith_cs",
            };
            return if negated {
                format!("!{name}")
            } else {
                name.to_string()
            };
        }
    };
    symbol.to_string()
}

/// The conversion function that turns a string into `ty`, if there is one.
fn conversion_to(ty: Type) -> Option<&'static str> {
    Some(match ty {
        Type::Bool => "tobool",
        Type::Int | Type::Long => "tolong",
        Type::Real => "toreal",
        Type::DateTime => "todatetime",
        Type::TimeSpan => "totimespan",
        _ => return None,
    })
}

fn is_string_literal(text: &str) -> bool {
    text.starts_with(['"', '\'', '@'])
}

impl BinaryTypeError {
    /// Whether some row of the table accepts `ty` on this side, so the other side is the one to
    /// blame.
    fn side_ok(&self, ty: Type, left_side: bool) -> bool {
        let group = OpGroup::of(self.op);
        BINARY_RULES
            .iter()
            .filter(|rule| rule.ops.contains(&group))
            .any(|rule| if left_side { rule.left } else { rule.right }.accepts(ty))
    }

    /// The `DS2003` diagnostic. The primary label goes on the operand that no row of the table
    /// accepts (the left one if both or neither are acceptable), the secondary label on the
    /// other; both say the operand's type. Callers may extend the labels with where a type came
    /// from.
    pub fn diagnostic(&self, left: Operand<'_>, right: Operand<'_>) -> Diagnostic {
        let (l, r) = (self.left, self.right);
        let group = OpGroup::of(self.op);
        let symbol = op_symbol(self.op);
        let message = match group {
            G::Add => format!("cannot add `{l}` and `{r}`"),
            G::Sub => format!("cannot subtract `{r}` from `{l}`"),
            G::Mul => format!("cannot multiply `{l}` by `{r}`"),
            G::Div => format!("cannot divide `{l}` by `{r}`"),
            G::Mod => format!("cannot compute the remainder of `{l}` divided by `{r}`"),
            G::Lt | G::LtEq | G::Gt | G::GtEq if l == r => {
                format!("values of type `{l}` cannot be ordered with `{symbol}`")
            }
            G::Eq | G::NotEq | G::Lt | G::LtEq | G::Gt | G::GtEq => {
                format!("cannot compare `{l}` with `{r}`")
            }
            G::EqCi | G::NotEqCi | G::StringOp => {
                format!("`{symbol}` needs two strings, found `{l}` and `{r}`")
            }
            G::And | G::Or => format!("`{symbol}` needs two `bool` values, found `{l}` and `{r}`"),
        };

        let blame_right = self.side_ok(l, true) && !self.side_ok(r, false);
        let (primary, primary_ty, secondary, secondary_ty) = if blame_right {
            (right, r, left, l)
        } else {
            (left, l, right, r)
        };
        let mut diagnostic = Diagnostic::error(codes::DS2003, message)
            .primary(primary.span, format!("this is `{primary_ty}`"))
            .secondary(secondary.span, format!("this is `{secondary_ty}`"));

        if let Some(help) = self.help(left, right, &symbol) {
            diagnostic = diagnostic.help(help);
        }
        diagnostic
    }

    fn help(&self, left: Operand<'_>, right: Operand<'_>, symbol: &str) -> Option<String> {
        let (l, r) = (self.left, self.right);
        let group = OpGroup::of(self.op);
        let arithmetic = !group.is_predicate();
        let comparison = matches!(group, G::Eq | G::NotEq | G::Lt | G::LtEq | G::Gt | G::GtEq);

        if group == G::Add && l == Type::String && r == Type::String {
            return Some(format!(
                "use `strcat` to join strings: `strcat({}, {})`",
                left.text, right.text
            ));
        }
        if group == G::Add && l == Type::DateTime && r == Type::DateTime {
            return Some(
                "add a `timespan` to a `datetime`, or subtract two datetimes to get a `timespan`"
                    .to_string(),
            );
        }
        // One side is a string and the other a type a string converts to.
        let string_side = match (l, r) {
            (Type::String, other) if other != Type::String => Some((true, other)),
            (other, Type::String) if other != Type::String => Some((false, other)),
            _ => None,
        };
        if let Some((string_is_left, other)) = string_side
            && (arithmetic && other.is_numeric() || comparison)
        {
            let conversion = if arithmetic {
                if group == G::Mod { "tolong" } else { "toreal" }
            } else {
                conversion_to(other)?
            };
            let (text, rewrite) = if string_is_left {
                let rewrite = format!("{conversion}({}) {symbol} {}", left.text, right.text);
                (left.text, rewrite)
            } else {
                let rewrite = format!("{} {symbol} {conversion}({})", left.text, right.text);
                (right.text, rewrite)
            };
            if is_string_literal(text) {
                return Some(format!(
                    "convert the string first, e.g. `{rewrite}`, or convert the other side with \
                     `tostring()`"
                ));
            }
            let holds = if other.is_numeric() && arithmetic {
                "numbers".to_string()
            } else {
                format!("`{other}` values")
            };
            return Some(format!(
                "if `{text}` holds {holds}, convert it first: `{rewrite}`"
            ));
        }
        if matches!(group, G::EqCi | G::NotEqCi | G::StringOp) {
            let (text, ty) = if l != Type::String {
                (left.text, l)
            } else {
                (right.text, r)
            };
            if ty != Type::String {
                return Some(format!("convert it to a string first: `tostring({text})`"));
            }
        }
        if matches!(group, G::And | G::Or) {
            return Some(format!(
                "`{symbol}` combines conditions; write a comparison such as `x > 0` on each side"
            ));
        }
        None
    }
}

impl UnaryTypeError {
    /// The `DS2003` diagnostic, pointing at the operand.
    pub fn diagnostic(&self, operand: Operand<'_>) -> Diagnostic {
        let ty = self.operand;
        let message = match self.op {
            PrefixOp::Neg => format!("cannot negate `{ty}`"),
            PrefixOp::Not => format!("`not` needs a `bool`, found `{ty}`"),
        };
        let mut diagnostic = Diagnostic::error(codes::DS2003, message)
            .primary(operand.span, format!("this is `{ty}`"));
        if self.op == PrefixOp::Neg && ty == Type::String && !is_string_literal(operand.text) {
            diagnostic = diagnostic.help(format!(
                "if `{}` holds numbers, convert it first: `-toreal({})`",
                operand.text, operand.text
            ));
        }
        diagnostic
    }
}

/// The `DS2009` diagnostic for an untyped `null` whose type nothing determines
/// (`extend x = null`).
pub fn untyped_null(span: Span) -> Diagnostic {
    Diagnostic::error(codes::DS2009, "cannot tell what type this `null` has")
        .primary(span, "nothing here gives this `null` a type")
        .help(
            "write a typed null instead, such as `long(null)`, `string(null)` or `datetime(null)`",
        )
}

#[cfg(test)]
mod tests {
    use super::Type::*;
    use super::*;
    use datascript_diagnostics::FileId;

    fn ok(op: BinaryOp, l: Type, r: Type) -> Type {
        match binary(op, l, r) {
            Ok(typing) => typing.result,
            Err(_) => panic!("{l} {} {r} should be well-typed", op_symbol(op)),
        }
    }

    fn err(op: BinaryOp, l: Type, r: Type) {
        assert!(
            binary(op, l, r).is_err(),
            "{l} {} {r} should be a type error",
            op_symbol(op)
        );
    }

    const INTS: [Type; 2] = [Int, Long];
    const NUMS: [Type; 3] = [Int, Long, Real];
    const HAS: BinaryOp = BinaryOp::String {
        op: StringOp::Has,
        negated: false,
    };

    // One test per row of LANGUAGE.md §4.1.

    #[test]
    fn row_integer_add_sub_mul_is_long() {
        for op in [BinaryOp::Add, BinaryOp::Sub, BinaryOp::Mul] {
            for l in INTS {
                for r in INTS {
                    let typing = binary(op, l, r).unwrap();
                    assert_eq!(typing.result, Long);
                    assert_eq!((typing.left, typing.right), (Long, Long));
                }
            }
        }
        // D-021: even `int * int`.
        assert_eq!(ok(BinaryOp::Mul, Int, Int), Long);
    }

    #[test]
    fn row_real_add_sub_mul_is_real() {
        for op in [BinaryOp::Add, BinaryOp::Sub, BinaryOp::Mul] {
            for other in NUMS {
                assert_eq!(ok(op, Real, other), Real);
                assert_eq!(ok(op, other, Real), Real);
            }
        }
        let typing = binary(BinaryOp::Add, Long, Real).unwrap();
        assert_eq!((typing.left, typing.right), (Real, Real));
    }

    #[test]
    fn row_integer_division_is_long() {
        for l in INTS {
            for r in INTS {
                assert_eq!(ok(BinaryOp::Div, l, r), Long);
            }
        }
    }

    #[test]
    fn row_real_division_is_real() {
        for other in NUMS {
            assert_eq!(ok(BinaryOp::Div, Real, other), Real);
            assert_eq!(ok(BinaryOp::Div, other, Real), Real);
        }
    }

    #[test]
    fn row_integer_modulo_is_long() {
        for l in INTS {
            for r in INTS {
                assert_eq!(ok(BinaryOp::Mod, l, r), Long);
            }
        }
        err(BinaryOp::Mod, Real, Long);
        err(BinaryOp::Mod, Long, Real);
    }

    #[test]
    fn row_timespan_add_sub_timespan() {
        assert_eq!(ok(BinaryOp::Add, TimeSpan, TimeSpan), TimeSpan);
        assert_eq!(ok(BinaryOp::Sub, TimeSpan, TimeSpan), TimeSpan);
    }

    #[test]
    fn row_datetime_plus_timespan_either_order() {
        assert_eq!(ok(BinaryOp::Add, DateTime, TimeSpan), DateTime);
        assert_eq!(ok(BinaryOp::Add, TimeSpan, DateTime), DateTime);
    }

    #[test]
    fn row_datetime_minus_timespan() {
        assert_eq!(ok(BinaryOp::Sub, DateTime, TimeSpan), DateTime);
        err(BinaryOp::Sub, TimeSpan, DateTime);
    }

    #[test]
    fn row_datetime_minus_datetime() {
        assert_eq!(ok(BinaryOp::Sub, DateTime, DateTime), TimeSpan);
        err(BinaryOp::Add, DateTime, DateTime);
    }

    #[test]
    fn row_timespan_scaled_by_number() {
        for n in NUMS {
            assert_eq!(ok(BinaryOp::Mul, TimeSpan, n), TimeSpan);
            assert_eq!(ok(BinaryOp::Mul, n, TimeSpan), TimeSpan);
            assert_eq!(ok(BinaryOp::Div, TimeSpan, n), TimeSpan);
            err(BinaryOp::Div, n, TimeSpan);
        }
    }

    #[test]
    fn row_timespan_divided_by_timespan() {
        assert_eq!(ok(BinaryOp::Div, TimeSpan, TimeSpan), Real);
    }

    #[test]
    fn row_negate_integer_is_long() {
        for ty in INTS {
            assert_eq!(
                unary(PrefixOp::Neg, ty),
                Ok(UnaryTyping {
                    operand: Long,
                    result: Long
                })
            );
        }
    }

    #[test]
    fn row_negate_real_or_timespan_keeps_type() {
        for ty in [Real, TimeSpan] {
            assert_eq!(unary(PrefixOp::Neg, ty).unwrap().result, ty);
        }
        for ty in [String, Bool, DateTime] {
            assert!(unary(PrefixOp::Neg, ty).is_err());
        }
    }

    #[test]
    fn row_equality_same_kind() {
        for op in [BinaryOp::Eq, BinaryOp::NotEq] {
            for ty in Type::NAMED {
                assert_eq!(ok(op, ty, ty), Bool);
            }
            assert_eq!(ok(op, Int, Real), Bool);
            let typing = binary(op, Int, Real).unwrap();
            assert_eq!((typing.left, typing.right), (Real, Real));
            err(op, String, Long);
            err(op, DateTime, TimeSpan);
            err(op, Bool, Long);
        }
    }

    #[test]
    fn row_ordering() {
        for op in [BinaryOp::Lt, BinaryOp::LtEq, BinaryOp::Gt, BinaryOp::GtEq] {
            for ty in [Int, Long, Real, String, DateTime, TimeSpan, Bool] {
                assert_eq!(ok(op, ty, ty), Bool);
            }
            assert_eq!(ok(op, Long, Real), Bool);
            err(op, String, Long);
            err(op, DateTime, Long);
            err(op, Dynamic, Dynamic);
            err(op, Guid, Guid);
        }
    }

    #[test]
    fn row_case_insensitive_equality() {
        for op in [BinaryOp::EqCi, BinaryOp::NotEqCi] {
            assert_eq!(ok(op, String, String), Bool);
            err(op, String, Long);
        }
    }

    #[test]
    fn row_string_operators() {
        for op in [
            HAS,
            BinaryOp::String {
                op: StringOp::ContainsCs,
                negated: true,
            },
            BinaryOp::String {
                op: StringOp::EndsWith,
                negated: false,
            },
            BinaryOp::MatchesRegex,
        ] {
            assert_eq!(ok(op, String, String), Bool);
            err(op, Long, String);
            err(op, String, Dynamic);
        }
    }

    #[test]
    fn row_logical() {
        for op in [BinaryOp::And, BinaryOp::Or] {
            assert_eq!(ok(op, Bool, Bool), Bool);
            err(op, Long, Bool);
        }
        assert_eq!(unary(PrefixOp::Not, Bool).unwrap().result, Bool);
        assert!(unary(PrefixOp::Not, Long).is_err());
    }

    #[test]
    fn anything_else_is_an_error() {
        err(BinaryOp::Add, String, Long);
        err(BinaryOp::Add, String, String);
        err(BinaryOp::Add, Bool, Bool);
        err(BinaryOp::Mul, TimeSpan, TimeSpan);
        err(BinaryOp::Add, Decimal, Long); // decimal arithmetic is M2
    }

    #[test]
    fn error_type_short_circuits() {
        for ty in Type::NAMED.into_iter().chain([Null, Error]) {
            assert_eq!(ok(BinaryOp::Add, Error, ty), Error);
            assert_eq!(ok(BinaryOp::Add, ty, Error), Error);
            assert_eq!(ok(BinaryOp::Eq, Error, ty), Bool);
            assert_eq!(ok(HAS, ty, Error), Bool);
            assert_eq!(ok(BinaryOp::And, Error, ty), Bool);
        }
        assert_eq!(unary(PrefixOp::Neg, Error).unwrap().result, Error);
        assert_eq!(unary(PrefixOp::Not, Error).unwrap().result, Bool);
    }

    #[test]
    fn null_takes_its_type_from_the_other_side() {
        let typing = binary(BinaryOp::Add, Null, Long).unwrap();
        assert_eq!(
            typing,
            BinaryTyping {
                left: Long,
                right: Long,
                result: Long
            }
        );
        assert_eq!(ok(BinaryOp::Add, Real, Null), Real);
        let typing = binary(BinaryOp::Sub, DateTime, Null).unwrap();
        assert_eq!(typing.result, DateTime);
        assert_eq!(typing.right, TimeSpan);
        let typing = binary(BinaryOp::Sub, Null, DateTime).unwrap();
        assert_eq!((typing.left, typing.result), (DateTime, TimeSpan));
        let typing = binary(BinaryOp::Eq, String, Null).unwrap();
        assert_eq!((typing.left, typing.right), (String, String));
        assert_eq!(ok(BinaryOp::And, Null, Bool), Bool);
        assert_eq!(ok(HAS, Null, String), Bool);
        // Two nulls.
        assert_eq!(ok(BinaryOp::Add, Null, Null), Null);
        assert_eq!(ok(BinaryOp::Eq, Null, Null), Bool);
        assert_eq!(unary(PrefixOp::Neg, Null).unwrap().result, Null);
        assert_eq!(unary(PrefixOp::Not, Null).unwrap().result, Bool);
    }

    fn operand(start: u32, text: &str) -> Operand<'_> {
        Operand {
            span: Span::new(FileId(0), start, start + text.len() as u32),
            text,
        }
    }

    #[test]
    fn diagnostic_matches_the_documented_example() {
        // DIAGNOSTICS.md: `price + 1` with `price: string`.
        let error = binary(BinaryOp::Add, String, Long).unwrap_err();
        let d = error.diagnostic(operand(17, "price"), operand(25, "1"));
        assert_eq!(d.code, codes::DS2003);
        assert_eq!(d.message, "cannot add `string` and `long`");
        let primary = d.primary.unwrap();
        assert_eq!(primary.span.start, 17);
        assert_eq!(primary.message, "this is `string`");
        assert_eq!(d.secondary[0].message, "this is `long`");
        assert_eq!(
            d.help[0].message,
            "if `price` holds numbers, convert it first: `toreal(price) + 1`"
        );
    }

    #[test]
    fn diagnostic_blames_the_unacceptable_side() {
        let error = binary(BinaryOp::Add, Long, String).unwrap_err();
        let d = error.diagnostic(operand(0, "1"), operand(4, "price"));
        assert_eq!(d.primary.unwrap().span.start, 4);
        assert_eq!(
            d.help[0].message,
            "if `price` holds numbers, convert it first: `1 + toreal(price)`"
        );
    }

    #[test]
    fn diagnostic_messages() {
        let cases = [
            (
                BinaryOp::Sub,
                Long,
                String,
                "cannot subtract `string` from `long`",
            ),
            (
                BinaryOp::Mul,
                Bool,
                Long,
                "cannot multiply `bool` by `long`",
            ),
            (
                BinaryOp::Div,
                String,
                Long,
                "cannot divide `string` by `long`",
            ),
            (
                BinaryOp::Mod,
                Real,
                Long,
                "cannot compute the remainder of `real` divided by `long`",
            ),
            (
                BinaryOp::Eq,
                String,
                Long,
                "cannot compare `string` with `long`",
            ),
            (
                BinaryOp::Lt,
                Dynamic,
                Dynamic,
                "values of type `dynamic` cannot be ordered with `<`",
            ),
            (
                HAS,
                Long,
                String,
                "`has` needs two strings, found `long` and `string`",
            ),
            (
                BinaryOp::And,
                Long,
                Bool,
                "`and` needs two `bool` values, found `long` and `bool`",
            ),
        ];
        for (op, l, r, message) in cases {
            let d = binary(op, l, r)
                .unwrap_err()
                .diagnostic(operand(0, "a"), operand(4, "b"));
            assert_eq!(d.message, message);
        }
    }

    #[test]
    fn diagnostic_helps() {
        let help = |op, l, r, lt, rt| {
            binary(op, l, r)
                .unwrap_err()
                .diagnostic(operand(0, lt), operand(10, rt))
                .help
                .first()
                .map(|h| h.message.clone())
        };
        assert_eq!(
            help(BinaryOp::Add, String, String, "a", "b").as_deref(),
            Some("use `strcat` to join strings: `strcat(a, b)`")
        );
        assert_eq!(
            help(BinaryOp::Gt, String, DateTime, "ts", "cutoff").as_deref(),
            Some("if `ts` holds `datetime` values, convert it first: `todatetime(ts) > cutoff`")
        );
        assert_eq!(
            help(BinaryOp::Add, String, Long, "\"1\"", "1").as_deref(),
            Some(
                "convert the string first, e.g. `toreal(\"1\") + 1`, or convert the other side \
                 with `tostring()`"
            )
        );
        assert_eq!(
            help(HAS, Long, String, "code", "\"x\"").as_deref(),
            Some("convert it to a string first: `tostring(code)`")
        );
        assert!(help(BinaryOp::Add, DateTime, DateTime, "a", "b").is_some());
    }

    #[test]
    fn unary_diagnostics() {
        let d = unary(PrefixOp::Neg, String)
            .unwrap_err()
            .diagnostic(operand(1, "price"));
        assert_eq!(d.message, "cannot negate `string`");
        assert_eq!(d.primary.unwrap().message, "this is `string`");
        let d = unary(PrefixOp::Not, Long)
            .unwrap_err()
            .diagnostic(operand(4, "x"));
        assert_eq!(d.message, "`not` needs a `bool`, found `long`");
    }

    #[test]
    fn untyped_null_diagnostic() {
        let d = untyped_null(Span::new(FileId(0), 11, 15));
        assert_eq!(d.code, codes::DS2009);
        assert!(d.help[0].message.contains("`long(null)`"));
    }
}
