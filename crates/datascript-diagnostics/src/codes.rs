//! The code registry: one constant per code in DIAGNOSTICS.md, and [`REGISTRY`] with each code's
//! default severity and summary.
//!
//! This file and the table in DIAGNOSTICS.md must agree; a test checks it. Codes are never reused
//! or renumbered. Allocate new codes only from the block your task owns (D-018), and add them to
//! both places in the same change.

use crate::diagnostic::{Code, Severity};

/// Whether a code is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeStatus {
    /// Implemented, or allocated to a task that implements it.
    Active,
    /// Reserved by a design decision; the task named in the summary implements it.
    Planned,
    /// Never to be used again. Retired codes have no constant.
    Retired,
}

/// One row of the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeInfo {
    /// The code.
    pub code: Code,
    /// The severity diagnostics with this code use. `None` for retired codes.
    pub severity: Option<Severity>,
    /// One-line summary, as in DIAGNOSTICS.md.
    pub summary: &'static str,
    /// Whether the code is in use.
    pub status: CodeStatus,
}

/// Looks up a code in [`REGISTRY`].
pub fn lookup(code: Code) -> Option<&'static CodeInfo> {
    REGISTRY
        .binary_search_by_key(&code, |info| info.code)
        .ok()
        .map(|index| &REGISTRY[index])
}

/// unexpected character
pub const DS0001: Code = Code::new(1);
/// unterminated string literal
pub const DS0002: Code = Code::new(2);
/// invalid number/timespan/datetime literal
pub const DS0003: Code = Code::new(3);
/// program has no query to run (no span)
pub const DS0004: Code = Code::new(4);
/// unknown escape sequence in string (help: `@"..."` or `/` for paths)
pub const DS0005: Code = Code::new(5);
/// unterminated block comment
pub const DS0006: Code = Code::new(6);
/// empty quoted name `['']`
pub const DS0007: Code = Code::new(7);
/// expression nested too deeply for the parser
pub const DS0100: Code = Code::new(100);
/// expected X, found Y (generic parse error, used as a last resort)
pub const DS0101: Code = Code::new(101);
/// missing `|` between operators
pub const DS0102: Code = Code::new(102);
/// pipe with no operator after it
pub const DS0103: Code = Code::new(103);
/// unknown operator (with did-you-mean; also `project - away` with spaces, D-026, and a row generator such as `range` before M2-01)
pub const DS0104: Code = Code::new(104);
/// SQL keyword used (`SELECT ... FROM`, rewritten as a `from` pipeline; `GROUP BY`, `WHERE` and `AND`/`OR`/`NOT` in caps, `DESC`, `IS NULL`, `LIKE`, `AS`, …), with the DataScript equivalent
pub const DS0105: Code = Code::new(105);
/// missing `;` between statements (machine-applicable fix, except after an operator that takes a list, where `,` is offered too and neither is certain, and before a `from` on the same line; a `from` first on its line always makes `;` certain, D-037)
pub const DS0106: Code = Code::new(106);
/// `<>` used instead of `!=`
pub const DS0107: Code = Code::new(107);
/// `=` used for comparison (also `===` and `= =`, reported once with a fix to `==`)
pub const DS0110: Code = Code::new(110);
/// `&&`/`||`/`!` used instead of `and`/`or`/`not`
pub const DS0111: Code = Code::new(111);
/// unbalanced parenthesis or bracket (points at the opening one)
pub const DS0112: Code = Code::new(112);
/// method-style call `a.f(x)` (DataScript has no methods)
pub const DS0113: Code = Code::new(113);
/// prefix `not` without parentheses as the operand of a tighter operator (`1 + not x`)
pub const DS0114: Code = Code::new(114);
/// `where` with conditions separated by commas
pub const DS0115: Code = Code::new(115);
/// `distinct *` with other columns
pub const DS0116: Code = Code::new(116);
/// SQL form with no DataScript equivalent: `count(*)`, `project *`, `''` inside a string
pub const DS0117: Code = Code::new(117);
/// `render` is not the last operator
pub const DS0120: Code = Code::new(120);
/// missing `by` in `sort`/`top`
pub const DS0121: Code = Code::new(121);
/// missing `on` in `join`/`lookup`
pub const DS0122: Code = Code::new(122);
/// unknown join kind (did-you-mean)
pub const DS0123: Code = Code::new(123);
/// `innerunique` is not supported (explains D-004)
pub const DS0124: Code = Code::new(124);
/// query starts with an operator or a pipe instead of `from` and a table (`where a > 1`, `| take 5`)
pub const DS0125: Code = Code::new(125);
/// reserved word used as a name (help: quote it, `['null']`)
pub const DS0126: Code = Code::new(126);
/// trailing comma at the end of a list
pub const DS0127: Code = Code::new(127);
/// list after `in` written in square brackets
pub const DS0128: Code = Code::new(128);
/// `in ~` / `!in ~` written with a space (machine-applicable fix to `in~`)
pub const DS0129: Code = Code::new(129);
/// query or pipeline does not start with `from` (machine-applicable fix inserts it when the head is clearly a table, D-036)
pub const DS0150: Code = Code::new(150);
/// `from` with no table after it
pub const DS0151: Code = Code::new(151);
/// `from` where it is not written: after `|`, or on the right of `join` or `union` (fix removes it, or turns the `|` into `;` when a new query follows, D-037)
pub const DS0152: Code = Code::new(152);
/// SQL clause order: a SQL keyword after the table without `|` (`from t select a`), or `FROM` after the column list (`| project a from t`)
pub const DS0153: Code = Code::new(153);
/// unknown column
pub const DS1001: Code = Code::new(1001);
/// unknown name (let/table)
pub const DS1002: Code = Code::new(1002);
/// name already defined
pub const DS1003: Code = Code::new(1003);
/// unknown function
pub const DS1004: Code = Code::new(1004);
/// duplicate output column name
pub const DS1005: Code = Code::new(1005);
/// a table name used where a value is expected (`let alias = orders;`)
pub const DS1006: Code = Code::new(1006);
/// `join` renamed colliding non-key columns (says how to rename explicitly; `on key` columns are merged, D-019)
pub const DS1007: Code = Code::new(1007);
/// `join` condition that is not a column name or `$left.a == $right.b`
pub const DS1030: Code = Code::new(1030);
/// `$left`/`$right` used outside a `join` condition
pub const DS1031: Code = Code::new(1031);
/// `distinct` given an expression instead of a column name
pub const DS1032: Code = Code::new(1032);
/// predicate is not `bool`
pub const DS2001: Code = Code::new(2001);
/// wrong number of arguments
pub const DS2002: Code = Code::new(2002);
/// operator not defined for these types
pub const DS2003: Code = Code::new(2003);
/// aggregate function used outside `summarize`
pub const DS2004: Code = Code::new(2004);
/// non-aggregate expression in `summarize` aggregate list
pub const DS2005: Code = Code::new(2005);
/// function argument has the wrong type (points at the argument, lists signatures)
pub const DS2006: Code = Code::new(2006);
/// column type has no DataScript equivalent and can only be projected
pub const DS2008: Code = Code::new(2008);
/// untyped `null` with no context (suggest `long(null)`, …)
pub const DS2009: Code = Code::new(2009);
/// a pipeline (a table) used where a value is expected
pub const DS2010: Code = Code::new(2010);
/// named argument in a call to a function that takes none
pub const DS2011: Code = Code::new(2011);
/// field access or indexing on a value that is not `dynamic` (`dynamic` is M2)
pub const DS2012: Code = Code::new(2012);
/// `summarize` with no aggregate and no `by` key
pub const DS2030: Code = Code::new(2030);
/// a value that is not a table or a path used as a source (`from 5`, a `let` bound to a number)
pub const DS2040: Code = Code::new(2040);
/// reader option value has the wrong type or is not allowed
pub const DS2041: Code = Code::new(2041);
/// reader call without exactly one path argument
pub const DS2042: Code = Code::new(2042);
/// reader option given twice
pub const DS2043: Code = Code::new(2043);
/// `take`/`top` row count that is not a constant whole number of 0 or more
pub const DS2050: Code = Code::new(2050);
/// predicate is always true or always false (planned, M1-14)
pub const DS2102: Code = Code::new(2102);
/// `sort` key without `asc`/`desc` (default is asc, ≠ KQL); machine-applicable fix inserts `asc` (planned, M1-14, D-005)
pub const DS2103: Code = Code::new(2103);
/// file not found (with did-you-mean over nearby files)
pub const DS3001: Code = Code::new(3001);
/// unknown file format
pub const DS3002: Code = Code::new(3002);
/// unknown reader option
pub const DS3003: Code = Code::new(3003);
/// glob matched no files
pub const DS3004: Code = Code::new(3004);
/// incompatible schemas across glob matches
pub const DS3005: Code = Code::new(3005);
/// source path or reader option is not a constant
pub const DS3006: Code = Code::new(3006);
/// path is outside the UI root directory (planned, M4-01, D-016)
pub const DS3007: Code = Code::new(3007);
/// file could not be read (permission denied, malformed CSV/JSON row at runtime)
pub const DS3008: Code = Code::new(3008);
/// invalid glob pattern
pub const DS3009: Code = Code::new(3009);
/// function, `join` kind or file format not supported by this engine (reported by `check`, from the engine's capabilities)
pub const DS4001: Code = Code::new(4001);
/// engine runtime error
pub const DS4002: Code = Code::new(4002);
/// arithmetic error: integer division by zero, overflow, or out-of-range value (D-014)
pub const DS4003: Code = Code::new(4003);
/// invalid argument value: a regular expression that does not compile, an unknown datetime part or format, a percentile outside 0–100 (reported by `check` when the argument is a constant, D-049)
pub const DS4004: Code = Code::new(4004);
/// chart not drawn in this runtime (once per program run, not for `render table`)
pub const DS5001: Code = Code::new(5001);
/// chart role cannot be filled: no column of the type the kind needs, or an explicit column of the wrong type (names the role and lists the result's columns)
pub const DS5002: Code = Code::new(5002);
/// unknown render kind (did-you-mean; a KQL kind DataScript does not support yet says so instead)
pub const DS5003: Code = Code::new(5003);
/// unknown or repeated render property (did-you-mean; a KQL property DataScript does not support yet says so instead)
pub const DS5004: Code = Code::new(5004);
/// bad value for a render property: wrong form or count, unknown `legend`/`kind` word (did-you-mean), `ymin` not below `ymax`
pub const DS5005: Code = Code::new(5005);
/// render property does not apply to this chart kind (says which properties the kind accepts; `kind = map` says map charts are not supported yet)
pub const DS5006: Code = Code::new(5006);
/// one column used in two chart roles (x, y, series)
pub const DS5007: Code = Code::new(5007);
/// `render` outside a query statement (in a tabular `let`, a parenthesized pipeline, or the right side of `join`/`union`)
pub const DS5008: Code = Code::new(5008);
/// internal compiler error (also used for caught panics)
pub const DS9001: Code = Code::new(9001);

/// Every registered code, sorted by code.
pub const REGISTRY: &[CodeInfo] = &[
    CodeInfo {
        code: DS0001,
        severity: Some(Severity::Error),
        summary: "unexpected character",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0002,
        severity: Some(Severity::Error),
        summary: "unterminated string literal",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0003,
        severity: Some(Severity::Error),
        summary: "invalid number/timespan/datetime literal",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0004,
        severity: Some(Severity::Warning),
        summary: "program has no query to run (no span)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0005,
        severity: Some(Severity::Error),
        summary: "unknown escape sequence in string (help: `@\"...\"` or `/` for paths)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0006,
        severity: Some(Severity::Error),
        summary: "unterminated block comment",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0007,
        severity: Some(Severity::Error),
        summary: "empty quoted name `['']`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0100,
        severity: Some(Severity::Error),
        summary: "expression nested too deeply, or an operator chain too long, for the parser",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0101,
        severity: Some(Severity::Error),
        summary: "expected X, found Y (generic parse error, used as a last resort)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0102,
        severity: Some(Severity::Error),
        summary: "missing `|` between operators",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0103,
        severity: Some(Severity::Error),
        summary: "pipe with no operator after it",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0104,
        severity: Some(Severity::Error),
        summary: "unknown operator (with did-you-mean; also `project - away` with spaces, D-026, and a row generator such as `range` before M2-01)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0105,
        severity: Some(Severity::Error),
        summary: "SQL keyword used (`SELECT ... FROM`, rewritten as a `from` pipeline; `GROUP BY`, `WHERE` and `AND`/`OR`/`NOT` in caps, `DESC`, `IS NULL`, `LIKE`, `AS`, …), with the DataScript equivalent",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0106,
        severity: Some(Severity::Error),
        summary: "missing `;` between statements (machine-applicable fix, except after an operator that takes a list, where `,` is offered too and neither is certain, and before a `from` on the same line; a `from` first on its line always makes `;` certain, D-037)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0107,
        severity: Some(Severity::Error),
        summary: "`<>` used instead of `!=`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0110,
        severity: Some(Severity::Error),
        summary: "`=` used for comparison (also `===` and `= =`, reported once with a fix to `==`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0111,
        severity: Some(Severity::Error),
        summary: "`&&`/`||`/`!` used instead of `and`/`or`/`not`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0112,
        severity: Some(Severity::Error),
        summary: "unbalanced parenthesis or bracket (points at the opening one)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0113,
        severity: Some(Severity::Error),
        summary: "method-style call `a.f(x)` (DataScript has no methods)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0114,
        severity: Some(Severity::Error),
        summary: "prefix `not` without parentheses as the operand of a tighter operator (`1 + not x`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0115,
        severity: Some(Severity::Error),
        summary: "`where` with conditions separated by commas",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0116,
        severity: Some(Severity::Error),
        summary: "`distinct *` with other columns",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0117,
        severity: Some(Severity::Error),
        summary: "SQL form with no DataScript equivalent: `count(*)`, `project *`, `''` inside a string",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0120,
        severity: Some(Severity::Error),
        summary: "`render` is not the last operator",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0121,
        severity: Some(Severity::Error),
        summary: "missing `by` in `sort`/`top`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0122,
        severity: Some(Severity::Error),
        summary: "missing `on` in `join`/`lookup`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0123,
        severity: Some(Severity::Error),
        summary: "unknown join kind (did-you-mean)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0124,
        severity: Some(Severity::Error),
        summary: "`innerunique` is not supported (explains D-004)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0125,
        severity: Some(Severity::Error),
        summary: "query starts with an operator or a pipe instead of `from` and a table (`where a > 1`, `| take 5`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0126,
        severity: Some(Severity::Error),
        summary: "reserved word used as a name (help: quote it, `['null']`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0127,
        severity: Some(Severity::Error),
        summary: "trailing comma at the end of a list",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0128,
        severity: Some(Severity::Error),
        summary: "list after `in` written in square brackets",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0129,
        severity: Some(Severity::Error),
        summary: "`in ~` / `!in ~` written with a space (machine-applicable fix to `in~`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0150,
        severity: Some(Severity::Error),
        summary: "query or pipeline does not start with `from` (machine-applicable fix inserts it when the head is clearly a table, D-036)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0151,
        severity: Some(Severity::Error),
        summary: "`from` with no table after it",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0152,
        severity: Some(Severity::Error),
        summary: "`from` where it is not written: after `|`, or on the right of `join` or `union` (fix removes it, or turns the `|` into `;` when a new query follows, D-037)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS0153,
        severity: Some(Severity::Error),
        summary: "SQL clause order: a SQL keyword after the table without `|` (`from t select a`), or `FROM` after the column list (`| project a from t`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1001,
        severity: Some(Severity::Error),
        summary: "unknown column",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1002,
        severity: Some(Severity::Error),
        summary: "unknown name (let/table)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1003,
        severity: Some(Severity::Error),
        summary: "name already defined",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1004,
        severity: Some(Severity::Error),
        summary: "unknown function",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1005,
        severity: Some(Severity::Error),
        summary: "duplicate output column name",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1006,
        severity: Some(Severity::Error),
        summary: "a table name used where a value is expected (`let alias = orders;`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1007,
        severity: Some(Severity::Info),
        summary: "`join` renamed colliding non-key columns (says how to rename explicitly; `on key` columns are merged, D-019)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1030,
        severity: Some(Severity::Error),
        summary: "`join` condition that is not a column name or `$left.a == $right.b`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1031,
        severity: Some(Severity::Error),
        summary: "`$left`/`$right` used outside a `join` condition",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS1032,
        severity: Some(Severity::Error),
        summary: "`distinct` given an expression instead of a column name",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2001,
        severity: Some(Severity::Error),
        summary: "predicate is not `bool`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2002,
        severity: Some(Severity::Error),
        summary: "wrong number of arguments",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2003,
        severity: Some(Severity::Error),
        summary: "operator not defined for these types",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2004,
        severity: Some(Severity::Error),
        summary: "aggregate function used outside `summarize`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2005,
        severity: Some(Severity::Error),
        summary: "non-aggregate expression in `summarize` aggregate list",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2006,
        severity: Some(Severity::Error),
        summary: "function argument has the wrong type (points at the argument, lists signatures)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: Code::new(2007),
        severity: None,
        summary: "*retired, never implemented* (was: name used both as a table and as a scalar, D-010; superseded by D-020)",
        status: CodeStatus::Retired,
    },
    CodeInfo {
        code: DS2008,
        severity: Some(Severity::Error),
        summary: "column type has no DataScript equivalent and can only be projected",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2009,
        severity: Some(Severity::Error),
        summary: "untyped `null` with no context (suggest `long(null)`, …)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2010,
        severity: Some(Severity::Error),
        summary: "a pipeline (a table) used where a value is expected",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2011,
        severity: Some(Severity::Error),
        summary: "named argument in a call to a function that takes none",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2012,
        severity: Some(Severity::Error),
        summary: "field access or indexing on a value that is not `dynamic` (`dynamic` is M2)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2030,
        severity: Some(Severity::Error),
        summary: "`summarize` with no aggregate and no `by` key",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2040,
        severity: Some(Severity::Error),
        summary: "a value that is not a table or a path used as a source (`from 5`, a `let` bound to a number)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2041,
        severity: Some(Severity::Error),
        summary: "reader option value has the wrong type or is not allowed",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2042,
        severity: Some(Severity::Error),
        summary: "reader call without exactly one path argument",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2043,
        severity: Some(Severity::Error),
        summary: "reader option given twice",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS2050,
        severity: Some(Severity::Error),
        summary: "`take`/`top` row count that is not a constant whole number of 0 or more",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: Code::new(2101),
        severity: None,
        summary: "*retired, never implemented* (was: `take`/`limit` before `sort`, D-050)",
        status: CodeStatus::Retired,
    },
    CodeInfo {
        code: DS2102,
        severity: Some(Severity::Warning),
        summary: "predicate is always true or always false *(planned, M1-14)*",
        status: CodeStatus::Planned,
    },
    CodeInfo {
        code: DS2103,
        severity: Some(Severity::Warning),
        summary: "`sort` key without `asc`/`desc` (default is asc, ≠ KQL); machine-applicable fix inserts `asc` *(planned, M1-14, D-005)*",
        status: CodeStatus::Planned,
    },
    CodeInfo {
        code: DS3001,
        severity: Some(Severity::Error),
        summary: "file not found (with did-you-mean over nearby files)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS3002,
        severity: Some(Severity::Error),
        summary: "unknown file format",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS3003,
        severity: Some(Severity::Error),
        summary: "unknown reader option",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS3004,
        severity: Some(Severity::Error),
        summary: "glob matched no files",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS3005,
        severity: Some(Severity::Error),
        summary: "incompatible schemas across glob matches",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS3006,
        severity: Some(Severity::Error),
        summary: "source path or reader option is not a constant",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS3007,
        severity: Some(Severity::Error),
        summary: "path is outside the UI root directory *(planned, M4-01, D-016)*",
        status: CodeStatus::Planned,
    },
    CodeInfo {
        code: DS3008,
        severity: Some(Severity::Error),
        summary: "file could not be read (permission denied, malformed CSV/JSON row at runtime)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS3009,
        severity: Some(Severity::Error),
        summary: "invalid glob pattern",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS4001,
        severity: Some(Severity::Error),
        summary: "function, `join` kind or file format not supported by this engine (reported by `check`, from the engine's capabilities)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS4002,
        severity: Some(Severity::Error),
        summary: "engine runtime error",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS4003,
        severity: Some(Severity::Error),
        summary: "arithmetic error: integer division by zero, overflow, or out-of-range value (D-014)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS4004,
        severity: Some(Severity::Error),
        summary: "invalid argument value: a regular expression that does not compile, an unknown datetime part or format, a percentile outside 0–100 (reported by `check` when the argument is a constant, D-049)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS5001,
        severity: Some(Severity::Info),
        summary: "chart not drawn in this runtime (once per program run, not for `render table`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS5002,
        severity: Some(Severity::Error),
        summary: "chart role cannot be filled: no column of the type the kind needs, or an explicit column of the wrong type (names the role and lists the result's columns)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS5003,
        severity: Some(Severity::Error),
        summary: "unknown render kind (did-you-mean; a KQL kind DataScript does not support yet says so instead)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS5004,
        severity: Some(Severity::Error),
        summary: "unknown or repeated render property (did-you-mean; a KQL property DataScript does not support yet says so instead)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS5005,
        severity: Some(Severity::Error),
        summary: "bad value for a render property: wrong form or count, unknown `legend`/`kind` word (did-you-mean), `ymin` not below `ymax`",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS5006,
        severity: Some(Severity::Error),
        summary: "render property does not apply to this chart kind (says which properties the kind accepts; `kind = map` says map charts are not supported yet)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS5007,
        severity: Some(Severity::Error),
        summary: "one column used in two chart roles (x, y, series)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS5008,
        severity: Some(Severity::Error),
        summary: "`render` outside a query statement (in a tabular `let`, a parenthesized pipeline, or the right side of `join`/`union`)",
        status: CodeStatus::Active,
    },
    CodeInfo {
        code: DS9001,
        severity: Some(Severity::Error),
        summary: "internal compiler error (also used for caught panics)",
        status: CodeStatus::Active,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of the "Code registry" table in DIAGNOSTICS.md: (code, severity, summary).
    fn documented_rows() -> Vec<(String, String, String)> {
        let doc = include_str!("../../../docs/DIAGNOSTICS.md").replace('\r', "");
        let registry = doc
            .split("## Code registry")
            .nth(1)
            .expect("DIAGNOSTICS.md has a `## Code registry` section");
        registry
            .lines()
            .filter_map(|line| {
                let cells: Vec<&str> = line
                    .strip_prefix('|')?
                    .strip_suffix('|')?
                    .split(" | ")
                    .map(str::trim)
                    .collect();
                match cells.as_slice() {
                    [code, severity, summary] if code.starts_with("DS") => {
                        Some((code.to_string(), severity.to_string(), summary.to_string()))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    #[test]
    fn registry_matches_diagnostics_md() {
        let documented = documented_rows();
        assert!(!documented.is_empty());
        let registered: Vec<(String, String, String)> = REGISTRY
            .iter()
            .map(|info| {
                (
                    info.code.to_string(),
                    info.severity.map_or("—", Severity::as_str).to_string(),
                    info.summary.to_string(),
                )
            })
            .collect();
        assert_eq!(
            registered, documented,
            "codes.rs and the registry table in DIAGNOSTICS.md disagree"
        );
    }

    #[test]
    fn registry_is_sorted_and_statuses_are_consistent() {
        assert!(REGISTRY.windows(2).all(|w| w[0].code < w[1].code));
        for info in REGISTRY {
            let planned = info.summary.contains("*(planned");
            let expected = match info.severity {
                None => CodeStatus::Retired,
                Some(_) if planned => CodeStatus::Planned,
                Some(_) => CodeStatus::Active,
            };
            assert_eq!(info.status, expected, "{}", info.code);
        }
    }

    #[test]
    fn this_tasks_codes() {
        let no_query = lookup(DS0004).unwrap();
        assert_eq!(no_query.severity, Some(Severity::Warning));
        let ice = lookup(DS9001).unwrap();
        assert_eq!(ice.severity, Some(Severity::Error));
        assert_eq!(lookup(Code::new(2007)).unwrap().status, CodeStatus::Retired);
        assert!(lookup(Code::new(42)).is_none());
    }
}
