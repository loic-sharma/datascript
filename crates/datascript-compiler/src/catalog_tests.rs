//! Tests for the function catalog and overload resolution.

use datascript_diagnostics::{FileId, Span, codes};

use crate::catalog::*;
use crate::operators::Operand;
use crate::types::Type::{self, *};
use crate::types::TypeClass;

fn catalog() -> &'static FunctionCatalog {
    FunctionCatalog::builtin()
}

fn def(name: &str) -> &'static FunctionDef {
    catalog().lookup(name).unwrap()
}

fn returns(name: &str, args: &[Type]) -> Type {
    match def(name).resolve(args) {
        Ok(resolved) => resolved.return_type,
        Err(e) => panic!("{name}{args:?} should resolve: {e:?}"),
    }
}

fn arg_error(name: &str, args: &[Type]) -> ArgTypeError {
    match def(name).resolve(args) {
        Err(ResolveError::ArgType(e)) => e,
        other => panic!("{name}{args:?} should be an argument type error: {other:?}"),
    }
}

fn arity_error(name: &str, args: &[Type]) -> ArityError {
    match def(name).resolve(args) {
        Err(ResolveError::Arity(e)) => e,
        other => panic!("{name}{args:?} should be an arity error: {other:?}"),
    }
}

/// Fake operands at offsets 100, 110, 120, … with the given texts.
fn operands<'a>(texts: &[&'a str]) -> Vec<Operand<'a>> {
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let start = 100 + 10 * i as u32;
            Operand {
                span: Span::new(FileId(0), start, start + text.len() as u32),
                text,
            }
        })
        .collect()
}

fn call<'a>(args: &'a [Operand<'a>]) -> CallSite<'a> {
    CallSite {
        span: Span::new(FileId(0), 90, 200),
        args,
    }
}

#[test]
fn every_entry_has_docs_and_examples() {
    for f in catalog().iter() {
        assert!(!f.doc.trim().is_empty(), "`{}` has no doc", f.name);
        assert!(!f.examples.is_empty(), "`{}` has no example", f.name);
        assert!(!f.signatures.is_empty(), "`{}` has no signature", f.name);
        for example in &f.examples {
            assert!(
                example.starts_with(&format!("{}(", f.name)),
                "example `{example}` of `{}` does not call it",
                f.name
            );
            let program = match f.kind {
                FunctionKind::Scalar => format!("from t | extend x = {example}"),
                FunctionKind::Aggregate => format!("from t | summarize x = {example}"),
            };
            let parse = datascript_syntax::parse(&program, FileId(0));
            assert!(
                parse.errors.is_empty(),
                "example `{example}` does not parse: {:?}",
                parse.errors
            );
        }
    }
}

#[test]
fn signatures_are_well_formed() {
    for f in catalog().iter() {
        for sig in &f.signatures {
            assert!(
                sig.optional.is_empty() || sig.repeat.is_none(),
                "`{}` mixes optional and repeated parameters",
                f.name
            );
            assert!(
                sig.repeat.as_ref().is_none_or(|r| !r.params.is_empty()),
                "`{}` has an empty repeated group",
                f.name
            );
            if let ReturnRule::SameAsArg(n) | ReturnRule::IntegerAsLong(n) = sig.returns {
                assert!(
                    n < sig.min_args(),
                    "`{}` returns a missing argument",
                    f.name
                );
            }
        }
    }
}

#[test]
fn m1_function_set_is_registered() {
    // LANGUAGE.md §7, without the functions marked M2.
    let m1 = [
        "count",
        "countif",
        "sum",
        "sumif",
        "avg",
        "min",
        "max",
        "dcount",
        "percentile",
        "stdev",
        "variance",
        "take_any",
        "iff",
        "case",
        "coalesce",
        "isnull",
        "isnotnull",
        "isempty",
        "isnotempty",
        "tostring",
        "toint",
        "tolong",
        "toreal",
        "tobool",
        "todatetime",
        "totimespan",
        "strlen",
        "tolower",
        "toupper",
        "trim",
        "substring",
        "strcat",
        "replace_string",
        "indexof",
        "extract",
        "now",
        "ago",
        "bin",
        "startofday",
        "startofweek",
        "startofmonth",
        "startofyear",
        "endofday",
        "endofweek",
        "endofmonth",
        "endofyear",
        "datetime_part",
        "dayofweek",
        "format_datetime",
        "datetime_diff",
        "abs",
        "round",
        "floor",
        "ceiling",
        "sqrt",
        "pow",
        "log",
        "log10",
        "exp",
    ];
    for name in m1 {
        assert!(catalog().lookup(name).is_ok(), "`{name}` is missing");
    }
    assert_eq!(catalog().iter().count(), m1.len());
    for m2 in ["make_list", "make_set", "arg_max", "arg_min", "split"] {
        assert!(catalog().lookup(m2).is_err(), "`{m2}` is M2");
    }
}

#[test]
fn kinds() {
    assert_eq!(def("sum").kind, FunctionKind::Aggregate);
    assert_eq!(def("count").kind, FunctionKind::Aggregate);
    assert_eq!(def("strlen").kind, FunctionKind::Scalar);
    assert_eq!(def("iff").kind, FunctionKind::Scalar);
}

#[test]
fn lookup_suggests_close_names() {
    let unknown = catalog().lookup("strln").unwrap_err();
    assert_eq!(unknown.suggestions.single(), Some("strlen"));
    assert_eq!(unknown.equivalent, None);
    let unknown = catalog().lookup("Strlen").unwrap_err();
    assert_eq!(unknown.suggestions.single(), Some("strlen"));
    let unknown = catalog().lookup("frobnicate").unwrap_err();
    assert!(unknown.suggestions.is_empty());
}

#[test]
fn lookup_knows_names_from_other_languages() {
    for (foreign, ours) in [
        ("len", "strlen"),
        ("LOWER", "tolower"),
        ("concat", "strcat"),
        ("count_distinct", "dcount"),
        ("if", "iff"),
        ("ifnull", "coalesce"),
        ("todouble", "toreal"),
        ("ceil", "ceiling"),
    ] {
        assert_eq!(
            catalog().lookup(foreign).unwrap_err().equivalent,
            Some(ours),
            "{foreign}"
        );
    }
}

#[test]
fn signature_display() {
    let lines = |name| def(name).signature_lines();
    assert_eq!(lines("strlen"), ["strlen(source: string) -> long"]);
    assert_eq!(
        lines("substring"),
        ["substring(source: string, start: integer[, length: integer]) -> string"]
    );
    assert_eq!(lines("now"), ["now([offset: timespan]) -> datetime"]);
    assert_eq!(lines("count"), ["count() -> long"]);
    assert_eq!(
        lines("iff"),
        ["iff(condition: bool, then: T, else: T) -> T"]
    );
    assert_eq!(
        lines("case"),
        ["case(condition: bool, value: T, ..., else: T) -> T"]
    );
    assert_eq!(
        lines("coalesce"),
        ["coalesce(value: T, value: T, ...) -> T"]
    );
    assert_eq!(lines("strcat"), ["strcat(value: any, ...) -> string"]);
    assert_eq!(
        lines("sum"),
        [
            "sum(x: T) -> long or real, where T is a number",
            "sum(x: timespan) -> timespan",
        ]
    );
    assert_eq!(
        lines("bin"),
        [
            "bin(value: T, size: T) -> long or real, where T is a number",
            "bin(value: datetime, size: timespan) -> datetime",
            "bin(value: timespan, size: timespan) -> timespan",
        ]
    );
    assert_eq!(
        lines("indexof"),
        [
            "indexof(source: string, lookup: string[, start: integer[, length: integer[, \
             occurrence: integer]]]) -> long"
        ]
    );
}

#[test]
fn fixed_and_same_as_arg_returns() {
    assert_eq!(returns("strlen", &[String]), Long);
    assert_eq!(returns("count", &[]), Long);
    assert_eq!(returns("now", &[]), DateTime);
    assert_eq!(returns("now", &[TimeSpan]), DateTime);
    assert_eq!(returns("min", &[String]), String);
    assert_eq!(returns("max", &[DateTime]), DateTime);
    assert_eq!(returns("take_any", &[Bool]), Bool);
    assert_eq!(returns("toint", &[String]), Int);
}

#[test]
fn sum_of_integers_is_long() {
    // D-021.
    assert_eq!(returns("sum", &[Int]), Long);
    assert_eq!(returns("sum", &[Long]), Long);
    assert_eq!(returns("sum", &[Real]), Real);
    assert_eq!(returns("sumif", &[Int, Bool]), Long);
    assert_eq!(returns("sumif", &[Real, Bool]), Real);
    assert_eq!(returns("avg", &[Long]), Real);
    assert_eq!(returns("stdev", &[Int]), Real);
}

#[test]
fn integer_math_is_long() {
    // D-021: like `-x`, these give `long` for an `int`, so `abs(toint(-2147483648))` fits.
    for name in ["abs", "round", "floor", "ceiling"] {
        assert_eq!(returns(name, &[Int]), Long, "{name}");
        assert_eq!(returns(name, &[Long]), Long, "{name}");
        assert_eq!(returns(name, &[Real]), Real, "{name}");
    }
    assert_eq!(returns("round", &[Int, Long]), Long);
    assert_eq!(returns("bin", &[Int, Int]), Long);
    assert_eq!(returns("bin", &[Int, Long]), Long);
    assert_eq!(returns("bin", &[Int, Real]), Real);
    // Only the first argument decides: an error elsewhere keeps the type.
    assert_eq!(returns("round", &[Int, Error]), Long);
    assert_eq!(returns("abs", &[Error]), Error);
    let line = |name: &str| def(name).signature_lines()[0].clone();
    assert_eq!(
        line("abs"),
        "abs(x: T) -> long or real, where T is a number"
    );
    assert_eq!(
        line("bin"),
        "bin(value: T, size: T) -> long or real, where T is a number"
    );
    assert_eq!(
        line("sum"),
        "sum(x: T) -> long or real, where T is a number"
    );
}

#[test]
fn functions_with_null_arguments() {
    // Only untyped `null`s decide the result: it stays `Null` for the binder to report (D-038).
    let cases: &[(&str, &[Type])] = &[
        ("abs", &[Null]),
        ("round", &[Null]),
        ("sum", &[Null]),
        ("sumif", &[Null, Bool]),
        ("min", &[Null]),
        ("max", &[Null]),
        ("take_any", &[Null]),
        ("iff", &[Bool, Null, Null]),
        ("coalesce", &[Null, Null]),
        ("case", &[Bool, Null, Null]),
        ("percentile", &[Null, Long]),
        ("bin", &[Null, Null]),
    ];
    for (name, args) in cases {
        assert_eq!(returns(name, args), Null, "{name}{args:?}");
    }
    // A typed argument, or a fixed result, decides it.
    assert_eq!(returns("bin", &[Null, TimeSpan]), DateTime);
    assert_eq!(returns("bin", &[Null, Int]), Long);
    assert_eq!(returns("abs", &[Null]), Null);
    assert_eq!(returns("avg", &[Null]), Real);
    assert_eq!(returns("strlen", &[Null]), Long);
    assert_eq!(returns("tostring", &[Null]), String);
    assert_eq!(returns("iff", &[Null, Null, String]), String);
    assert_eq!(returns("coalesce", &[Null, Null, Real]), Real);
    let resolved = def("ago").resolve(&[Null]).unwrap();
    assert_eq!(resolved.arg_types, [TimeSpan]);
    // `null` for a number parameter keeps its type: the backend casts it.
    let resolved = def("pow").resolve(&[Null, Null]).unwrap();
    assert_eq!(
        (resolved.arg_types.as_slice(), resolved.return_type),
        (&[Null, Null][..], Real)
    );
}

#[test]
fn decimal_arguments() {
    // D-039: `decimal` is M2 and mixes with no other type, like in the operator table.
    arg_error("avg", &[Decimal]);
    arg_error("sum", &[Decimal]);
    arg_error("min", &[Decimal]);
    arg_error("abs", &[Decimal]);
    arg_error("pow", &[Decimal, Real]);
    let e = arg_error("coalesce", &[Decimal, Real]);
    assert_eq!(
        e.expected,
        Expected::SameAs {
            index: 0,
            ty: Decimal
        }
    );
    assert_eq!(returns("iff", &[Bool, Decimal, Decimal]), Decimal);
    assert_eq!(returns("iff", &[Bool, Decimal, Null]), Decimal);
    assert_eq!(returns("isnull", &[Decimal]), Bool);
    assert_eq!(returns("tostring", &[Decimal]), String);
    assert_eq!(returns("take_any", &[Decimal]), Decimal);
    assert_eq!(returns("dcount", &[Decimal]), Long);
}

#[test]
fn type_variable_widens() {
    let resolved = def("iff").resolve(&[Bool, Long, Real]).unwrap();
    assert_eq!(resolved.return_type, Real);
    assert_eq!(resolved.arg_types, [Bool, Real, Real]);
    assert_eq!(returns("iff", &[Bool, Int, Long]), Long);
    assert_eq!(returns("coalesce", &[Int, Long, Real]), Real);
    assert_eq!(returns("case", &[Bool, Long, Bool, Int, Real]), Real);
    assert_eq!(returns("bin", &[Long, Real]), Real);
    assert_eq!(returns("round", &[Real, Long]), Real);
}

#[test]
fn overloads_pick_the_first_match() {
    assert_eq!(returns("bin", &[Long, Long]), Long);
    assert_eq!(returns("bin", &[DateTime, TimeSpan]), DateTime);
    assert_eq!(returns("bin", &[TimeSpan, TimeSpan]), TimeSpan);
    assert_eq!(returns("abs", &[TimeSpan]), TimeSpan);
    assert_eq!(returns("percentile", &[Long, Long]), Long);
    assert_eq!(returns("percentile", &[DateTime, Real]), DateTime);
}

#[test]
fn widening_converts_arguments() {
    // `int` passed for a `real` parameter is converted.
    let resolved = def("sqrt").resolve(&[Int]).unwrap();
    assert_eq!(resolved.arg_types, [Int]);
    let resolved = def("ago").resolve(&[TimeSpan]).unwrap();
    assert_eq!(resolved.arg_types, [TimeSpan]);
    let resolved = def("pow").resolve(&[Int, Real]).unwrap();
    assert_eq!(resolved.return_type, Real);
}

#[test]
fn null_arguments_take_the_parameter_type() {
    let resolved = def("iff").resolve(&[Bool, Null, Long]).unwrap();
    assert_eq!(resolved.return_type, Long);
    assert_eq!(resolved.arg_types, [Bool, Long, Long]);
    let resolved = def("strlen").resolve(&[Null]).unwrap();
    assert_eq!(resolved.arg_types, [String]);
    assert_eq!(returns("coalesce", &[Null, String]), String);
    // Nothing gives `T` a type: the binder reports DS2009 if the context needs one.
    assert_eq!(returns("iff", &[Bool, Null, Null]), Null);
    assert_eq!(returns("sum", &[Null]), Null);
    assert_eq!(returns("isnull", &[Null]), Bool);
}

#[test]
fn error_arguments_short_circuit() {
    // An argument with an error never causes a second error...
    assert_eq!(returns("strlen", &[Error]), Long);
    assert_eq!(returns("iff", &[Error, Long, Long]), Long);
    assert_eq!(returns("iff", &[Bool, Error, String]), Error);
    assert_eq!(returns("sum", &[Error]), Error);
    assert_eq!(returns("min", &[Error]), Error);
    assert_eq!(returns("bin", &[Error, TimeSpan]), DateTime);
    assert_eq!(returns("bin", &[Error, Error]), Error);
    // ...but arity does not depend on types.
    arity_error("strlen", &[Error, Error]);
}

#[test]
fn arity_errors() {
    assert_eq!(
        arity_error("substring", &[String]),
        ArityError {
            function: "substring",
            found: 1
        }
    );
    arity_error("substring", &[String, Long, Long, Long]);
    arity_error("now", &[TimeSpan, TimeSpan]);
    arity_error("count", &[Long]);
    arity_error("case", &[Bool, Long]);
    arity_error("case", &[Bool, Long, Bool, Long]);
    arity_error("coalesce", &[Long]);
    arity_error("strcat", &[]);
    assert_eq!(returns("strcat", &[Long, String, Bool]), String);
    assert_eq!(returns("case", &[Bool, Long, Long]), Long);
}

#[test]
fn describe_accepted() {
    let d = |name| ArityError::describe_accepted(def(name));
    assert_eq!(d("count"), "no arguments");
    assert_eq!(d("strlen"), "1 argument");
    assert_eq!(d("iff"), "3 arguments");
    assert_eq!(d("now"), "0 or 1 arguments");
    assert_eq!(d("substring"), "2 or 3 arguments");
    assert_eq!(d("indexof"), "2 to 5 arguments");
    assert_eq!(d("strcat"), "at least 1 argument");
    assert_eq!(d("coalesce"), "at least 2 arguments");
    assert_eq!(d("case"), "an odd number of arguments, at least 3");
}

#[test]
fn arity_diagnostic_missing_argument() {
    let args = operands(&["name"]);
    let d = arity_error("substring", &[String]).diagnostic(def("substring"), call(&args));
    assert_eq!(d.code, codes::DS2002);
    assert_eq!(d.message, "`substring` takes 2 or 3 arguments, found 1");
    let primary = d.primary.unwrap();
    assert_eq!(primary.span.start, 90);
    assert_eq!(primary.message, "missing `start`");
    assert_eq!(
        d.notes,
        ["signature: `substring(source: string, start: integer[, length: integer]) -> string`"]
    );
}

#[test]
fn swapped_arguments_are_suggested() {
    let e = arg_error("datetime_part", &[DateTime, String]);
    assert_eq!(e.swap_with, Some(1));
    let args = operands(&["timestamp", "'year'"]);
    let d = e.diagnostic(def("datetime_part"), call(&args));
    assert_eq!(
        d.help[0].message,
        "the arguments may be in the wrong order: `datetime_part('year', timestamp)`"
    );
    assert!(d.help[0].suggestion.is_some());
    // No swap makes `strlen(long)` valid.
    assert_eq!(arg_error("strlen", &[Long]).swap_with, None);
}

#[test]
fn conversion_help_only_when_the_conversion_gives_a_value() {
    let args = operands(&["timestamp"]);
    let d = arg_error("sqrt", &[DateTime]).diagnostic(def("sqrt"), call(&args));
    assert!(d.help.is_empty(), "{:?}", d.help);
    let args = operands(&["qty"]);
    let d = arg_error("iff", &[Long, Long, Long]).diagnostic(def("iff"), call(&args));
    assert_eq!(
        d.help[0].message,
        "to test whether it is not zero, compare it: `qty != 0`"
    );
}

#[test]
fn arity_diagnostic_extra_arguments() {
    let args = operands(&["a", "b", "c"]);
    let d = arity_error("strlen", &[String, String, String]).diagnostic(def("strlen"), call(&args));
    assert_eq!(d.message, "`strlen` takes 1 argument, found 3");
    let primary = d.primary.unwrap();
    assert_eq!((primary.span.start, primary.span.end), (110, 121));
    assert_eq!(primary.message, "these 2 arguments are not expected");

    let args = operands(&["1h"]);
    let d = arity_error("count", &[TimeSpan]).diagnostic(def("count"), call(&args));
    assert_eq!(d.message, "`count` takes no arguments, found 1");
    assert_eq!(
        d.primary.as_ref().unwrap().message,
        "this argument is not expected"
    );
    assert_eq!(
        d.help[0].message,
        "`count()` counts rows; to count the rows where `1h` is not null, use \
         `countif(isnotnull(1h))`"
    );

    let args = operands(&["s", "0", "1", "2"]);
    let d = arity_error("substring", &[String, Long, Long, Long])
        .diagnostic(def("substring"), call(&args));
    assert_eq!(d.primary.unwrap().message, "this argument is not expected");
}

#[test]
fn arity_diagnostic_repeated_group() {
    let args = operands(&["p", "1"]);
    let d = arity_error("case", &[Bool, Long]).diagnostic(def("case"), call(&args));
    assert_eq!(
        d.message,
        "`case` takes an odd number of arguments, at least 3, found 2"
    );
    assert_eq!(d.primary.unwrap().message, "called with 2 arguments");
}

#[test]
fn arg_type_error_points_at_the_argument() {
    let e = arg_error("strlen", &[Long]);
    assert_eq!(
        e,
        ArgTypeError {
            function: "strlen",
            index: 0,
            found: Long,
            expected: Expected::OneOf(vec![TypeClass::Exact(String)]),
            swap_with: None,
        }
    );
    let args = operands(&["orderId"]);
    let d = e.diagnostic(def("strlen"), call(&args));
    assert_eq!(d.code, codes::DS2006);
    assert_eq!(
        d.message,
        "argument `source` of `strlen` must be `string`, found `long`"
    );
    let primary = d.primary.unwrap();
    assert_eq!(primary.span.start, 100);
    assert_eq!(primary.message, "this is `long`");
    assert_eq!(
        d.help[0].message,
        "convert it first if that is what you meant: `tostring(orderId)`"
    );
    assert_eq!(d.notes, ["signature: `strlen(source: string) -> long`"]);
}

#[test]
fn arg_type_error_reports_the_furthest_argument() {
    // `substring(s, "x")`: the first argument is fine, the second is not.
    let e = arg_error("substring", &[String, String]);
    assert_eq!(e.index, 1);
    let args = operands(&["s", "\"x\""]);
    let d = e.diagnostic(def("substring"), call(&args));
    assert_eq!(
        d.message,
        "argument `start` of `substring` must be an integer (`int` or `long`), found `string`"
    );
    assert_eq!(d.primary.unwrap().span.start, 110);
}

#[test]
fn arg_type_error_merges_overloads() {
    let e = arg_error("bin", &[String, TimeSpan]);
    assert_eq!(e.index, 0);
    assert_eq!(
        e.expected,
        Expected::OneOf(vec![
            TypeClass::Numeric,
            TypeClass::Exact(DateTime),
            TypeClass::Exact(TimeSpan)
        ])
    );
    let args = operands(&["ts", "1h"]);
    let d = e.diagnostic(def("bin"), call(&args));
    assert_eq!(
        d.message,
        "argument `value` of `bin` must be a number, `datetime` or `timespan`, found `string`"
    );
    assert!(d.help.is_empty());
    assert!(
        d.notes[0].starts_with("`bin` has these forms: `bin(value: T, size: T) -> long or real")
    );

    // `bin(ts, 5)` with `ts: datetime`: the datetime form gets furthest.
    let e = arg_error("bin", &[DateTime, Long]);
    assert_eq!(e.index, 1);
    assert_eq!(
        e.expected,
        Expected::OneOf(vec![TypeClass::Exact(TimeSpan)])
    );
}

/// A function with made-up forms, for resolution rules the built-ins do not exercise.
fn made_up(signatures: Vec<Signature>) -> FunctionDef {
    FunctionDef {
        name: "f",
        kind: FunctionKind::Scalar,
        signatures,
        doc: "",
        examples: Vec::new(),
    }
}

fn form(params: &[(&'static str, Type)]) -> Signature {
    Signature {
        required: params
            .iter()
            .map(|&(name, ty)| Param {
                name,
                ty: ParamType::Class(TypeClass::Exact(ty)),
            })
            .collect(),
        optional: Vec::new(),
        repeat: None,
        trailing: Vec::new(),
        returns: ReturnRule::Fixed(Bool),
    }
}

fn made_up_error(f: &FunctionDef, args: &[Type]) -> ArgTypeError {
    match f.resolve(args) {
        Err(ResolveError::ArgType(e)) => e,
        other => panic!("{args:?} should be an argument type error: {other:?}"),
    }
}

#[test]
fn arg_type_error_prefers_exact_matches() {
    // `f(1, 2, 3)`: the first form only gets to `c` by widening `a` and `b`; the second matched `a`
    // exactly and fails at `b`. Before M1-23 the furthest argument, `c`, was blamed.
    let f = made_up(vec![
        form(&[("a", Real), ("b", Real), ("c", String)]),
        form(&[("a", Long), ("b", String), ("c", Long)]),
    ]);
    let e = made_up_error(&f, &[Long, Long, Long]);
    assert_eq!(e.index, 1);
    assert_eq!(e.expected, Expected::OneOf(vec![TypeClass::Exact(String)]));

    // Equally good forms failing at different arguments: the first form decides.
    let f = made_up(vec![
        form(&[("a", Long), ("b", String), ("c", Long)]),
        form(&[("a", Long), ("b", Long), ("c", String)]),
    ]);
    let e = made_up_error(&f, &[Long, Bool, Bool]);
    assert_eq!(e.index, 1);
    // `null` matches but is not an exact match: it does not make a form better.
    let e = made_up_error(&f, &[Null, Long, Long]);
    assert_eq!((e.index, e.found), (2, Long));
    let e = made_up_error(&f, &[Null, Bool, Long]);
    assert_eq!(e.index, 1);
    // Forms failing at the same argument still merge.
    let e = made_up_error(&f, &[Bool, Long, Long]);
    assert_eq!(
        (e.index, e.expected),
        (0, Expected::OneOf(vec![TypeClass::Exact(Long)]))
    );
}

#[test]
fn describe_accepted_with_several_kinds_of_form() {
    let repeated = |min| Signature {
        repeat: Some(Repeat {
            params: vec![Param {
                name: "v",
                ty: ParamType::Class(TypeClass::Any),
            }],
            min,
        }),
        ..form(&[])
    };
    let f = made_up(vec![form(&[("a", Long)]), repeated(3)]);
    assert_eq!(
        ArityError::describe_accepted(&f),
        "1 argument, or at least 3 arguments"
    );
    let mut pairs = repeated(1);
    pairs.repeat.as_mut().unwrap().params.push(Param {
        name: "w",
        ty: ParamType::Class(TypeClass::Any),
    });
    let f = made_up(vec![repeated(4), pairs]);
    assert_eq!(
        ArityError::describe_accepted(&f),
        "at least 4 arguments, or an even number of arguments, at least 2"
    );
    assert_eq!(
        ArityError::describe_accepted(&made_up(vec![form(&[])])),
        "no arguments"
    );
}

#[test]
fn arg_type_error_for_mismatched_type_variable() {
    let e = arg_error("iff", &[Bool, Long, String]);
    assert_eq!(
        e,
        ArgTypeError {
            function: "iff",
            index: 2,
            found: String,
            expected: Expected::SameAs { index: 1, ty: Long },
            swap_with: None,
        }
    );
    let args = operands(&["p", "1", "\"x\""]);
    let d = e.diagnostic(def("iff"), call(&args));
    assert_eq!(
        d.message,
        "argument `else` of `iff` must have the same type as argument `then` (`long`), found \
         `string`"
    );
    assert_eq!(d.primary.unwrap().span.start, 120);
    assert_eq!(d.secondary[0].span.start, 110);
    assert_eq!(d.secondary[0].message, "this is `long`");

    // The first non-null argument fixes `T`.
    let e = arg_error("coalesce", &[Null, Long, String]);
    assert_eq!(e.expected, Expected::SameAs { index: 1, ty: Long });
    let args = operands(&["null", "a", "b"]);
    let d = e.diagnostic(def("coalesce"), call(&args));
    assert_eq!(
        d.message,
        "the 3rd argument of `coalesce` must have the same type as the 2nd argument (`long`), \
         found `string`"
    );
}

#[test]
fn arg_type_error_for_class_constraint() {
    let e = arg_error("stdev", &[String]);
    assert_eq!(e.expected, Expected::OneOf(vec![TypeClass::Numeric]));
    let args = operands(&["price"]);
    let d = e.diagnostic(def("stdev"), call(&args));
    assert_eq!(
        d.message,
        "argument `x` of `stdev` must be a number, found `string`"
    );
    assert_eq!(
        d.help[0].message,
        "convert it first if that is what you meant: `toreal(price)`"
    );
    let e = arg_error("countif", &[Long]);
    assert_eq!(e.expected, Expected::OneOf(vec![TypeClass::Exact(Bool)]));
    arg_error("min", &[Dynamic]);
}

/// The parser's list of reader functions (`datascript_syntax::READERS`, for DS0150) must be the
/// compiler's reader definitions (`source::READERS`), and no reader is also a scalar function.
#[test]
fn syntax_readers_match_the_compiler() {
    let ours: Vec<&str> = crate::source::READERS.iter().map(|r| r.name).collect();
    assert_eq!(datascript_syntax::READERS, ours.as_slice());
    for reader in datascript_syntax::READERS {
        assert!(
            catalog().lookup(reader).is_err(),
            "`{reader}` is also a scalar function"
        );
    }
}

#[test]
fn planned_functions_are_known_and_not_in_the_catalog() {
    for (name, milestone) in crate::catalog::PLANNED {
        let unknown = catalog().lookup(name).unwrap_err();
        assert_eq!(unknown.planned, Some(*milestone), "{name}");
    }
    assert_eq!(catalog().lookup("strln").unwrap_err().planned, None);
}

/// Every catalog example binds without diagnostics. The columns an example uses, and their
/// types, are inferred: unknown names become columns, then the first assignment of types to them
/// under which the example checks cleanly is kept.
#[test]
fn every_example_resolves() {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema, TimeUnit};
    use datascript_diagnostics::SourceFile;

    use crate::{Capabilities, SchemaLookup, SchemaProvider, SourceRef, check};

    struct Table(Vec<(std::string::String, DataType)>);
    impl SchemaProvider for Table {
        fn schema_of(&self, _: &SourceRef) -> SchemaLookup {
            let fields: Vec<Field> = self
                .0
                .iter()
                .map(|(name, dt)| Field::new(name, dt.clone(), true))
                .collect();
            SchemaLookup::Ready(Arc::new(Schema::new(fields)))
        }
    }
    let candidates = [
        (Long, DataType::Int64),
        (Real, DataType::Float64),
        (String, DataType::Utf8),
        (Bool, DataType::Boolean),
        (
            DateTime,
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        ),
        (TimeSpan, DataType::Duration(TimeUnit::Microsecond)),
    ];

    for f in catalog().iter() {
        for example in &f.examples {
            let program = match f.kind {
                FunctionKind::Scalar => format!("from 't.csv' | extend x = {example}"),
                FunctionKind::Aggregate => format!("from 't.csv' | summarize x = {example}"),
            };
            // `extend` and `summarize` check their expressions, then report DS9001 "not
            // implemented yet" until M1-16; that one is not about the example.
            let run = |columns: &[(std::string::String, DataType)]| {
                let mut diagnostics = check(
                    &SourceFile::new("example.ds", &program),
                    FileId(0),
                    &Table(columns.to_vec()),
                    &Capabilities::full(),
                )
                .diagnostics;
                diagnostics.retain(|d| {
                    !(d.code == codes::DS9001 && d.message.ends_with("is not implemented yet"))
                });
                diagnostics
            };
            // The names the example uses as columns.
            let names: Vec<std::string::String> = run(&[])
                .iter()
                .filter(|d| d.code == codes::DS1001)
                .filter_map(|d| d.primary.as_ref())
                .map(|label| program[label.span.start as usize..label.span.end as usize].into())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            let combinations = candidates.len().pow(names.len() as u32);
            let found = (0..combinations).find_map(|mut n| {
                let mut columns = Vec::new();
                let mut types = BTreeMap::new();
                for name in &names {
                    let (ty, dt) = &candidates[n % candidates.len()];
                    n /= candidates.len();
                    columns.push((name.clone(), dt.clone()));
                    types.insert(name.clone(), *ty);
                }
                run(&columns).is_empty().then_some(types)
            });
            assert!(
                found.is_some(),
                "example `{example}` of `{}` does not resolve for any column types; with none: \
                 {:?}",
                f.name,
                run(&[]).iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
    }
}
