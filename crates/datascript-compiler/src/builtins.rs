//! The M1 built-in functions (LANGUAGE.md §7). Semantics follow KQL unless noted (D-014); the
//! docs here are what hover and the reference show.

use crate::catalog::{FunctionDef, FunctionKind, Param, ParamType, Repeat, ReturnRule, Signature};
use crate::types::{Type, TypeClass};

// Parameter shorthands.

fn p(name: &'static str, ty: ParamType) -> Param {
    Param { name, ty }
}

const STRING: ParamType = ParamType::Class(TypeClass::Exact(Type::String));
const BOOL: ParamType = ParamType::Class(TypeClass::Exact(Type::Bool));
const DATETIME: ParamType = ParamType::Class(TypeClass::Exact(Type::DateTime));
const TIMESPAN: ParamType = ParamType::Class(TypeClass::Exact(Type::TimeSpan));
const INTEGER: ParamType = ParamType::Class(TypeClass::Integer);
const NUMBER: ParamType = ParamType::Class(TypeClass::Numeric);
const ANY: ParamType = ParamType::Class(TypeClass::Any);
/// `T`, any type.
const T_ANY: ParamType = ParamType::Var(TypeClass::Any);
/// `T`, a number.
const T_NUM: ParamType = ParamType::Var(TypeClass::Numeric);
/// `T`, a type with an order.
const T_CMP: ParamType = ParamType::Var(TypeClass::Comparable);

fn sig(required: Vec<Param>, returns: ReturnRule) -> Signature {
    Signature {
        required,
        optional: Vec::new(),
        repeat: None,
        trailing: Vec::new(),
        returns,
    }
}

impl Signature {
    fn optional(mut self, optional: Vec<Param>) -> Self {
        self.optional = optional;
        self
    }

    fn repeat(mut self, params: Vec<Param>, min: usize) -> Self {
        self.repeat = Some(Repeat { params, min });
        self
    }

    fn trailing(mut self, trailing: Vec<Param>) -> Self {
        self.trailing = trailing;
        self
    }
}

fn fixed(ty: Type) -> ReturnRule {
    ReturnRule::Fixed(ty)
}

const SAME_AS_FIRST: ReturnRule = ReturnRule::SameAsArg(0);

/// `sum` of integers is `long` (D-021); of reals, `real`.
fn sum_type(args: &[Type]) -> Type {
    match args[0] {
        Type::Int | Type::Long => Type::Long,
        Type::Null => Type::Null,
        other => other,
    }
}

const SUM_RETURN: ReturnRule = ReturnRule::Custom {
    rule: sum_type,
    display: "long or real",
};

struct Def {
    name: &'static str,
    kind: FunctionKind,
    doc: &'static str,
    examples: Vec<&'static str>,
    signatures: Vec<Signature>,
}

fn scalar(name: &'static str, doc: &'static str) -> Def {
    Def {
        name,
        kind: FunctionKind::Scalar,
        doc,
        examples: Vec::new(),
        signatures: Vec::new(),
    }
}

fn aggregate(name: &'static str, doc: &'static str) -> Def {
    Def {
        kind: FunctionKind::Aggregate,
        ..scalar(name, doc)
    }
}

impl Def {
    fn sig(mut self, signature: Signature) -> Self {
        self.signatures.push(signature);
        self
    }

    fn examples(mut self, examples: &[&'static str]) -> FunctionDef {
        self.examples.extend_from_slice(examples);
        FunctionDef {
            name: self.name,
            kind: self.kind,
            signatures: self.signatures,
            doc: self.doc,
            examples: self.examples,
        }
    }
}

/// One-argument `datetime` → `datetime` functions with KQL's optional offset in units of the
/// period (`startofmonth(dt, -1)` is the start of the previous month).
fn period(name: &'static str, doc: &'static str, example: &'static str) -> FunctionDef {
    scalar(name, doc)
        .sig(
            sig(vec![p("date", DATETIME)], fixed(Type::DateTime))
                .optional(vec![p("offset", INTEGER)]),
        )
        .examples(&[example])
}

fn conversion(
    name: &'static str,
    to: Type,
    doc: &'static str,
    example: &'static str,
) -> FunctionDef {
    scalar(name, doc)
        .sig(sig(vec![p("value", ANY)], fixed(to)))
        .examples(&[example])
}

fn real_math(name: &'static str, doc: &'static str, example: &'static str) -> FunctionDef {
    scalar(name, doc)
        .sig(sig(vec![p("x", NUMBER)], fixed(Type::Real)))
        .examples(&[example])
}

/// The functions of LANGUAGE.md §7 available in M1.
pub(crate) fn functions() -> Vec<FunctionDef> {
    vec![
        // Aggregates. They ignore nulls; `count()` counts rows.
        aggregate("count", "Counts the rows in each group.")
            .sig(sig(vec![], fixed(Type::Long)))
            .examples(&["count()"]),
        aggregate("countif", "Counts the rows in each group for which `predicate` is true.")
            .sig(sig(vec![p("predicate", BOOL)], fixed(Type::Long)))
            .examples(&["countif(amount > 100)"]),
        aggregate(
            "sum",
            "Adds up the non-null values in each group. The sum of integers is `long`; of reals, \
             `real`. With no non-null values the result is null.",
        )
        .sig(sig(vec![p("x", T_NUM)], SUM_RETURN))
        .examples(&["sum(amount)"]),
        aggregate(
            "sumif",
            "Adds up the non-null values in each group for the rows where `predicate` is true.",
        )
        .sig(sig(vec![p("x", T_NUM), p("predicate", BOOL)], SUM_RETURN))
        .examples(&["sumif(amount, status == \"shipped\")"]),
        aggregate("avg", "The mean of the non-null values in each group, as a `real`.")
            .sig(sig(vec![p("x", NUMBER)], fixed(Type::Real)))
            .examples(&["avg(amount)"]),
        aggregate("min", "The smallest non-null value in each group.")
            .sig(sig(vec![p("x", T_CMP)], SAME_AS_FIRST))
            .examples(&["min(timestamp)"]),
        aggregate("max", "The largest non-null value in each group.")
            .sig(sig(vec![p("x", T_CMP)], SAME_AS_FIRST))
            .examples(&["max(amount)"]),
        aggregate(
            "dcount",
            "The number of distinct non-null values in each group. Exact (≠ KQL, which estimates).",
        )
        .sig(sig(vec![p("x", ANY)], fixed(Type::Long)))
        .examples(&["dcount(customerId)"]),
        aggregate(
            "percentile",
            "The value below which `percentage` percent of the values in each group fall, by the \
             exact nearest-rank method (≠ KQL, which estimates).",
        )
        .sig(sig(
            vec![p("x", T_NUM), p("percentage", NUMBER)],
            SAME_AS_FIRST,
        ))
        .sig(sig(
            vec![p("x", DATETIME), p("percentage", NUMBER)],
            fixed(Type::DateTime),
        ))
        .sig(sig(
            vec![p("x", TIMESPAN), p("percentage", NUMBER)],
            fixed(Type::TimeSpan),
        ))
        .examples(&["percentile(amount, 95)"]),
        aggregate("stdev", "The sample standard deviation of the values in each group.")
            .sig(sig(vec![p("x", NUMBER)], fixed(Type::Real)))
            .examples(&["stdev(amount)"]),
        aggregate("variance", "The sample variance of the values in each group.")
            .sig(sig(vec![p("x", NUMBER)], fixed(Type::Real)))
            .examples(&["variance(amount)"]),
        aggregate("take_any", "Any one non-null value from each group.")
            .sig(sig(vec![p("x", T_ANY)], SAME_AS_FIRST))
            .examples(&["take_any(customerName)"]),
        // Conditional.
        scalar("iff", "`then` if `condition` is true, otherwise `else` (also when it is null).")
            .sig(sig(
                vec![p("condition", BOOL), p("then", T_ANY), p("else", T_ANY)],
                ReturnRule::SameAsArg(1),
            ))
            .examples(&["iff(amount > 100, \"big\", \"small\")"]),
        scalar(
            "case",
            "The value after the first condition that is true, or the last argument if none is.",
        )
        .sig(
            sig(vec![], ReturnRule::SameAsArg(1))
                .repeat(vec![p("condition", BOOL), p("value", T_ANY)], 1)
                .trailing(vec![p("else", T_ANY)]),
        )
        .examples(&["case(amount > 1000, \"large\", amount > 100, \"medium\", \"small\")"]),
        scalar("coalesce", "The first argument that is not null.")
            .sig(
                sig(vec![p("value", T_ANY), p("value", T_ANY)], SAME_AS_FIRST)
                    .repeat(vec![p("value", T_ANY)], 0),
            )
            .examples(&["coalesce(nickname, name, \"anonymous\")"]),
        scalar("isnull", "Whether `value` is null.")
            .sig(sig(vec![p("value", ANY)], fixed(Type::Bool)))
            .examples(&["isnull(shippedAt)"]),
        scalar("isnotnull", "Whether `value` is not null.")
            .sig(sig(vec![p("value", ANY)], fixed(Type::Bool)))
            .examples(&["isnotnull(shippedAt)"]),
        scalar("isempty", "Whether `value` is null or an empty string.")
            .sig(sig(vec![p("value", ANY)], fixed(Type::Bool)))
            .examples(&["isempty(comment)"]),
        scalar("isnotempty", "Whether `value` is neither null nor an empty string.")
            .sig(sig(vec![p("value", ANY)], fixed(Type::Bool)))
            .examples(&["isnotempty(comment)"]),
        // Conversion. A value that does not convert becomes null.
        conversion(
            "tostring",
            Type::String,
            "Converts a value to its string form.",
            "tostring(orderId)",
        ),
        conversion(
            "toint",
            Type::Int,
            "Converts a value to a 32-bit `int`, or null if it does not convert.",
            "toint(\"42\")",
        ),
        conversion(
            "tolong",
            Type::Long,
            "Converts a value to a `long`, or null if it does not convert.",
            "tolong(\"42\")",
        ),
        conversion(
            "toreal",
            Type::Real,
            "Converts a value to a `real`, or null if it does not convert.",
            "toreal(price)",
        ),
        conversion(
            "tobool",
            Type::Bool,
            "Converts a value to a `bool` (`\"true\"`, `\"false\"`, or a number: 0 is false), or \
             null if it does not convert.",
            "tobool(\"true\")",
        ),
        conversion(
            "todatetime",
            Type::DateTime,
            "Converts a value to a `datetime` (ISO 8601 strings, UTC unless an offset is given), \
             or null if it does not convert.",
            "todatetime(\"2024-01-31T12:00:00Z\")",
        ),
        conversion(
            "totimespan",
            Type::TimeSpan,
            "Converts a value to a `timespan` (`\"1.02:03:04\"` or `\"90m\"`), or null if it does \
             not convert.",
            "totimespan(\"00:30:00\")",
        ),
        // String.
        scalar("strlen", "The length of a string in Unicode scalar values.")
            .sig(sig(vec![p("source", STRING)], fixed(Type::Long)))
            .examples(&["strlen(name)"]),
        scalar("tolower", "The string in lower case.")
            .sig(sig(vec![p("source", STRING)], fixed(Type::String)))
            .examples(&["tolower(email)"]),
        scalar("toupper", "The string in upper case.")
            .sig(sig(vec![p("source", STRING)], fixed(Type::String)))
            .examples(&["toupper(country)"]),
        scalar(
            "trim",
            "Removes leading and trailing matches of `regex` from `source` (as in KQL).",
        )
        .sig(sig(
            vec![p("regex", STRING), p("source", STRING)],
            fixed(Type::String),
        ))
        .examples(&["trim(@\"\\s+\", name)"]),
        scalar(
            "substring",
            "Part of a string: from position `start` (0-based), `length` characters or to the end.",
        )
        .sig(
            sig(
                vec![p("source", STRING), p("start", INTEGER)],
                fixed(Type::String),
            )
            .optional(vec![p("length", INTEGER)]),
        )
        .examples(&["substring(orderId, 0, 3)", "substring(name, 2)"]),
        scalar("strcat", "Joins the string forms of its arguments.")
            .sig(sig(vec![], fixed(Type::String)).repeat(vec![p("value", ANY)], 1))
            .examples(&["strcat(firstName, \" \", lastName)"]),
        scalar(
            "replace_string",
            "Replaces every occurrence of `lookup` in `source` with `rewrite`.",
        )
        .sig(sig(
            vec![
                p("source", STRING),
                p("lookup", STRING),
                p("rewrite", STRING),
            ],
            fixed(Type::String),
        ))
        .examples(&["replace_string(path, \"\\\\\", \"/\")"]),
        scalar(
            "indexof",
            "The 0-based position of the `occurrence`-th match of `lookup` in `source`, searching \
             `length` characters from `start`, or -1 if it is absent.",
        )
        .sig(
            sig(
                vec![p("source", STRING), p("lookup", STRING)],
                fixed(Type::Long),
            )
            .optional(vec![
                p("start", INTEGER),
                p("length", INTEGER),
                p("occurrence", INTEGER),
            ]),
        )
        .examples(&["indexof(email, \"@\")"]),
        scalar(
            "extract",
            "The text matched by capture group `group` of `regex` in `source` (0 is the whole \
             match), or null if there is no match. Regexes use Rust `regex` syntax (D-014).",
        )
        .sig(sig(
            vec![p("regex", STRING), p("group", INTEGER), p("source", STRING)],
            fixed(Type::String),
        ))
        .examples(&["extract(@\"id=(\\d+)\", 1, message)"]),
        // Datetime. All in UTC.
        scalar(
            "now",
            "The current time, plus `offset` if given. One value for the whole program run (D-015).",
        )
        .sig(sig(vec![], fixed(Type::DateTime)).optional(vec![p("offset", TIMESPAN)]))
        .examples(&["now()", "now(-1h)"]),
        scalar("ago", "The time `span` before `now()`.")
            .sig(sig(vec![p("span", TIMESPAN)], fixed(Type::DateTime)))
            .examples(&["ago(1d)"]),
        scalar(
            "bin",
            "Rounds `value` down to a multiple of `size`. Datetimes are binned from the Unix epoch.",
        )
        .sig(sig(vec![p("value", T_NUM), p("size", T_NUM)], SAME_AS_FIRST))
        .sig(sig(
            vec![p("value", DATETIME), p("size", TIMESPAN)],
            fixed(Type::DateTime),
        ))
        .sig(sig(
            vec![p("value", TIMESPAN), p("size", TIMESPAN)],
            fixed(Type::TimeSpan),
        ))
        .examples(&["bin(timestamp, 1h)", "bin(amount, 100)"]),
        period("startofday", "The start of the day.", "startofday(timestamp)"),
        period(
            "startofweek",
            "The start of the week (Sunday, as in KQL).",
            "startofweek(timestamp)",
        ),
        period("startofmonth", "The start of the month.", "startofmonth(timestamp)"),
        period("startofyear", "The start of the year.", "startofyear(timestamp)"),
        period(
            "endofday",
            "The last moment (1 µs before midnight) of the day.",
            "endofday(timestamp)",
        ),
        period(
            "endofweek",
            "The last moment of the week (Saturday, as in KQL).",
            "endofweek(timestamp)",
        ),
        period("endofmonth", "The last moment of the month.", "endofmonth(timestamp)"),
        period("endofyear", "The last moment of the year.", "endofyear(timestamp)"),
        scalar(
            "datetime_part",
            "One part of a datetime as a number: `\"year\"`, `\"quarter\"`, `\"month\"`, \
             `\"week_of_year\"`, `\"day\"`, `\"dayofyear\"`, `\"hour\"`, `\"minute\"`, \
             `\"second\"`, `\"millisecond\"`, `\"microsecond\"`.",
        )
        .sig(sig(
            vec![p("part", STRING), p("date", DATETIME)],
            fixed(Type::Long),
        ))
        .examples(&["datetime_part(\"hour\", timestamp)"]),
        scalar(
            "dayofweek",
            "The time since the start of the week (Sunday): `0d` for Sunday, `1d` for Monday, … (as \
             in KQL).",
        )
        .sig(sig(vec![p("date", DATETIME)], fixed(Type::TimeSpan)))
        .examples(&["dayofweek(timestamp)"]),
        scalar(
            "format_datetime",
            "Formats a datetime with KQL format specifiers such as `yyyy-MM-dd HH:mm:ss`.",
        )
        .sig(sig(
            vec![p("date", DATETIME), p("format", STRING)],
            fixed(Type::String),
        ))
        .examples(&["format_datetime(timestamp, \"yyyy-MM-dd\")"]),
        scalar(
            "datetime_diff",
            "The number of `part` boundaries (`\"day\"`, `\"hour\"`, …, as in `datetime_part`) \
             between `date2` and `date1`.",
        )
        .sig(sig(
            vec![p("part", STRING), p("date1", DATETIME), p("date2", DATETIME)],
            fixed(Type::Long),
        ))
        .examples(&["datetime_diff(\"day\", shippedAt, orderedAt)"]),
        // Math.
        scalar("abs", "The absolute value.")
            .sig(sig(vec![p("x", T_NUM)], SAME_AS_FIRST))
            .sig(sig(vec![p("x", TIMESPAN)], fixed(Type::TimeSpan)))
            .examples(&["abs(delta)"]),
        scalar(
            "round",
            "Rounds to `digits` decimal places (default 0), halves away from zero.",
        )
        .sig(sig(vec![p("x", T_NUM)], SAME_AS_FIRST).optional(vec![p("digits", INTEGER)]))
        .examples(&["round(amount, 2)"]),
        scalar("floor", "The largest whole number not greater than `x`.")
            .sig(sig(vec![p("x", T_NUM)], SAME_AS_FIRST))
            .examples(&["floor(amount)"]),
        scalar("ceiling", "The smallest whole number not less than `x`.")
            .sig(sig(vec![p("x", T_NUM)], SAME_AS_FIRST))
            .examples(&["ceiling(amount)"]),
        real_math("sqrt", "The square root.", "sqrt(area)"),
        scalar("pow", "`base` raised to the power `exponent`.")
            .sig(sig(
                vec![p("base", NUMBER), p("exponent", NUMBER)],
                fixed(Type::Real),
            ))
            .examples(&["pow(2, 10)"]),
        real_math("log", "The natural logarithm.", "log(amount)"),
        real_math("log10", "The base-10 logarithm.", "log10(amount)"),
        real_math("exp", "`e` raised to the power `x`.", "exp(rate)"),
    ]
}
