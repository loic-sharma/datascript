//! Parser for `.dstest` files (TESTING.md, "The `.dstest` format").
//!
//! The parser records the byte range of every section so bless mode can rewrite expected
//! sections without touching anything else in the file.

use std::fmt;
use std::ops::Range;

use datascript_diagnostics::did_you_mean;

use crate::schema::{self, MockSchemaProvider};

/// The fixed clock used when a case has no `now:` header.
pub const DEFAULT_NOW: &str = "2024-06-01T00:00:00Z";

/// The engine used when a case has no `engine:` header.
pub const DEFAULT_ENGINE: &str = "datafusion";

const HEADER_KEYS: &[&str] = &[
    "mode", "order", "now", "engine", "tables", "schema", "skip", "eol",
];
const SECTION_NAMES: &[&str] = &["query", "expected"];

/// How far the pipeline goes for a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Parse only and dump the CST.
    Syntax,
    /// Parse and bind; print diagnostics.
    Check,
    /// Print the logical plan.
    Explain,
    /// Execute and print results (the default).
    Run,
}

impl Mode {
    /// Every mode, in pipeline order.
    pub const ALL: [Mode; 4] = [Mode::Syntax, Mode::Check, Mode::Explain, Mode::Run];

    /// The name used in the `mode:` header.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Syntax => "syntax",
            Mode::Check => "check",
            Mode::Explain => "explain",
            Mode::Run => "run",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether result rows are compared in order (D-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// Rows must appear in the expected order.
    Strict,
    /// Rows are sorted before comparing.
    Any,
}

/// A malformed `.dstest` file. `line` is 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for CaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for CaseError {}

/// A header value together with the line it came from, for error messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located<T> {
    pub value: T,
    pub line: usize,
}

/// One `---- expected` or `---- expected[<engine>]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSection {
    /// `None` for the shared `---- expected` section.
    pub engine: Option<String>,
    /// Body with `\r\n` turned into `\n` and trailing blank lines removed.
    pub body: String,
    /// Byte range of the section, from the start of its `----` line to the end of its body.
    pub range: Range<usize>,
    /// Byte range of the body alone.
    pub body_range: Range<usize>,
    /// Line of the `----` header.
    pub line: usize,
}

/// A parsed `.dstest` file.
#[derive(Debug, Clone)]
pub struct TestCase {
    /// The free-form `#` lines before the first section, without the `#`.
    pub description: Vec<String>,
    pub mode: Mode,
    /// `None` when the header has no `order:` key; the runner then picks the default.
    pub order: Option<Order>,
    /// The `now:` clock as written, and as microseconds since the Unix epoch.
    pub now: String,
    pub now_micros: i64,
    /// Engines to run the case on, in header order. Never empty.
    pub engines: Located<Vec<String>>,
    /// Host-registered tables from `tables:`, as `(name, path)`.
    pub tables: Vec<(String, String)>,
    /// Schemas from `schema:`.
    pub schema: Option<MockSchemaProvider>,
    /// Reason from `skip:`.
    pub skip: Option<String>,
    /// The query text: `\r\n` normalized, trailing blank lines removed, ending with one `\n`.
    /// With `eol: crlf`, every line then ends with `\r\n` instead.
    pub query: String,
    /// Line of the first query line.
    pub query_line: usize,
    pub expected: Vec<ExpectedSection>,
}

impl TestCase {
    /// The expected section that applies to `engine`: its own section, else the shared one.
    pub fn expected_for(&self, engine: &str) -> Option<&ExpectedSection> {
        self.expected
            .iter()
            .find(|s| s.engine.as_deref() == Some(engine))
            .or_else(|| self.expected.iter().find(|s| s.engine.is_none()))
    }

    /// Parses a `.dstest` file.
    pub fn parse(text: &str) -> Result<TestCase, CaseError> {
        Parser::default().parse(text)
    }
}

struct Line<'a> {
    /// 1-based.
    number: usize,
    /// Text without the line terminator.
    text: &'a str,
    /// Byte range including the terminator.
    range: Range<usize>,
}

fn lines(text: &str) -> impl Iterator<Item = Line<'_>> {
    let mut offset = 0;
    text.split_inclusive('\n').enumerate().map(move |(i, raw)| {
        let start = offset;
        offset += raw.len();
        let content = raw.strip_suffix('\n').unwrap_or(raw);
        let content = content.strip_suffix('\r').unwrap_or(content);
        Line {
            number: i + 1,
            text: content,
            range: start..offset,
        }
    })
}

/// A `----` line: four dashes, one space, then a name. Table separators never contain a space,
/// so they are not mistaken for section headers.
fn section_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("---- ")?;
    let name = rest.trim_end();
    (!name.is_empty() && !name.starts_with(' ')).then_some(name)
}

fn err(line: usize, message: impl Into<String>) -> CaseError {
    CaseError {
        line,
        message: message.into(),
    }
}

fn hint(unknown: &str, known: &[&str]) -> String {
    match did_you_mean(unknown, known.iter().copied()).single() {
        Some(name) => format!(" (did you mean `{name}`?)"),
        None => format!(
            " (expected one of: {})",
            known
                .iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[derive(Default)]
struct Parser {
    description: Vec<String>,
    headers: Vec<(String, Located<String>)>,
}

struct RawSection {
    name: String,
    engine: Option<String>,
    line: usize,
    start: usize,
    body_start: usize,
    body_end: usize,
    first_body_line: usize,
}

impl Parser {
    fn parse(mut self, text: &str) -> Result<TestCase, CaseError> {
        let mut sections: Vec<RawSection> = Vec::new();
        for line in lines(text) {
            if let Some(name) = section_name(line.text) {
                let (name, engine) = parse_section_name(name, line.number)?;
                if sections
                    .iter()
                    .any(|s| s.name == name && s.engine == engine)
                {
                    let shown = match &engine {
                        Some(e) => format!("{name}[{e}]"),
                        None => name.clone(),
                    };
                    return Err(err(
                        line.number,
                        format!("duplicate `---- {shown}` section"),
                    ));
                }
                sections.push(RawSection {
                    name,
                    engine,
                    line: line.number,
                    start: line.range.start,
                    body_start: line.range.end,
                    body_end: line.range.end,
                    first_body_line: line.number + 1,
                });
            } else if let Some(section) = sections.last_mut() {
                section.body_end = line.range.end;
            } else if line.text.starts_with("----") {
                return Err(err(
                    line.number,
                    format!(
                        "malformed section header `{}`; write `---- query` or `---- expected`",
                        line.text
                    ),
                ));
            } else {
                self.header_line(line.text, line.number)?;
            }
        }

        let header = |key: &str| self.headers.iter().find(|(k, _)| k == key).map(|(_, v)| v);

        let mode = match header("mode") {
            None => Mode::Run,
            Some(v) => *Mode::ALL
                .iter()
                .find(|m| m.as_str() == v.value)
                .ok_or_else(|| {
                    let names: Vec<_> = Mode::ALL.iter().map(|m| m.as_str()).collect();
                    err(
                        v.line,
                        format!("unknown mode `{}`{}", v.value, hint(&v.value, &names)),
                    )
                })?,
        };
        let order = match header("order") {
            None => None,
            Some(v) => Some(match v.value.as_str() {
                "strict" => Order::Strict,
                "any" => Order::Any,
                other => {
                    return Err(err(
                        v.line,
                        format!("unknown order `{other}`{}", hint(other, &["strict", "any"])),
                    ));
                }
            }),
        };
        let (now, now_line) =
            header("now").map_or((DEFAULT_NOW.to_string(), 0), |v| (v.value.clone(), v.line));
        let now_micros = parse_utc_datetime(&now).ok_or_else(|| {
            err(
                now_line,
                format!(
                    "`now` must be an ISO 8601 UTC datetime such as `{DEFAULT_NOW}`, found `{now}`"
                ),
            )
        })?;
        let engines = match header("engine") {
            None => Located {
                value: vec![DEFAULT_ENGINE.to_string()],
                line: 0,
            },
            Some(v) => {
                let list = split_list(&v.value, ',');
                if list.is_empty() {
                    return Err(err(v.line, "`engine` needs at least one engine name"));
                }
                for (i, e) in list.iter().enumerate() {
                    if list[..i].contains(e) {
                        return Err(err(v.line, format!("engine `{e}` is listed twice")));
                    }
                }
                Located {
                    value: list,
                    line: v.line,
                }
            }
        };
        let tables = match header("tables") {
            None => Vec::new(),
            Some(v) => parse_tables(&v.value).map_err(|m| err(v.line, m))?,
        };
        let schema = match header("schema") {
            None => None,
            Some(v) => Some(schema::parse(&v.value).map_err(|m| err(v.line, m))?),
        };
        let skip = header("skip").map(|v| v.value.clone());
        // `.gitattributes` keeps `.dstest` files LF, so CRLF input is asked for explicitly.
        let crlf = match header("eol") {
            None => false,
            Some(v) => match v.value.as_str() {
                "lf" => false,
                "crlf" => true,
                other => {
                    return Err(err(
                        v.line,
                        format!("unknown eol `{other}`{}", hint(other, &["lf", "crlf"])),
                    ));
                }
            },
        };

        let mut query = None;
        let mut expected = Vec::new();
        for s in &sections {
            let body = normalize_body(&text[s.body_start..s.body_end]);
            if s.name == "query" {
                query = Some((body, s.first_body_line));
            } else {
                if let Some(engine) = &s.engine
                    && !engines.value.contains(engine)
                {
                    return Err(err(
                        s.line,
                        format!(
                            "section is for engine `{engine}`, which the `engine` header does not list"
                        ),
                    ));
                }
                expected.push(ExpectedSection {
                    engine: s.engine.clone(),
                    body,
                    range: s.start..s.body_end,
                    body_range: s.body_start..s.body_end,
                    line: s.line,
                });
            }
        }
        let Some((query, query_line)) = query else {
            let line = sections
                .first()
                .map_or(lines(text).count().max(1), |s| s.line);
            return Err(err(line, "missing `---- query` section"));
        };
        if query.is_empty() {
            return Err(err(query_line - 1, "the `---- query` section is empty"));
        }

        Ok(TestCase {
            description: self.description,
            mode,
            order,
            now,
            now_micros,
            engines,
            tables,
            schema,
            skip,
            query: if crlf {
                format!("{}\r\n", query.replace('\n', "\r\n"))
            } else {
                format!("{query}\n")
            },
            query_line,
            expected,
        })
    }

    fn header_line(&mut self, text: &str, line: usize) -> Result<(), CaseError> {
        if text.trim().is_empty() {
            return Ok(());
        }
        if let Some(desc) = text.strip_prefix('#') {
            self.description
                .push(desc.strip_prefix(' ').unwrap_or(desc).to_string());
            return Ok(());
        }
        let Some((key, value)) = text.split_once(':') else {
            return Err(err(
                line,
                format!(
                    "expected a `key: value` header, a `#` description or a `---- query` section, found `{text}`"
                ),
            ));
        };
        let key = key.trim();
        if !HEADER_KEYS.contains(&key) {
            return Err(err(
                line,
                format!("unknown header key `{key}`{}", hint(key, HEADER_KEYS)),
            ));
        }
        if self.headers.iter().any(|(k, _)| k == key) {
            return Err(err(line, format!("header key `{key}` is set twice")));
        }
        let value = value.trim();
        if value.is_empty() {
            return Err(err(line, format!("header key `{key}` has no value")));
        }
        self.headers.push((
            key.to_string(),
            Located {
                value: value.to_string(),
                line,
            },
        ));
        Ok(())
    }
}

fn parse_section_name(name: &str, line: usize) -> Result<(String, Option<String>), CaseError> {
    let (base, engine) = match name.split_once('[') {
        Some((base, rest)) => {
            let engine = rest.strip_suffix(']').filter(|e| {
                !e.is_empty()
                    && e.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            });
            match engine {
                Some(e) => (base, Some(e.to_string())),
                None => {
                    return Err(err(
                        line,
                        format!(
                            "malformed section header `---- {name}`; write `---- expected[<engine>]`"
                        ),
                    ));
                }
            }
        }
        None => (name, None),
    };
    if !SECTION_NAMES.contains(&base) {
        return Err(err(
            line,
            format!("unknown section `{base}`{}", hint(base, SECTION_NAMES)),
        ));
    }
    if base == "query" && engine.is_some() {
        return Err(err(line, "the `---- query` section cannot name an engine"));
    }
    Ok((base.to_string(), engine))
}

/// Normalizes a section body for comparison: `\r\n` becomes `\n` and trailing blank lines go.
/// Trailing spaces inside lines are kept (`.editorconfig` keeps them in `.dstest` files).
pub fn normalize_body(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn split_list(value: &str, sep: char) -> Vec<String> {
    value
        .split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_tables(value: &str) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in split_list(value, ',') {
        let Some((name, path)) = entry.split_once('=') else {
            return Err(format!(
                "`tables` entry `{entry}` must look like `name=path/to/file.csv`"
            ));
        };
        let (name, path) = (name.trim(), path.trim());
        if name.is_empty() || path.is_empty() {
            return Err(format!(
                "`tables` entry `{entry}` must look like `name=path/to/file.csv`"
            ));
        }
        if out.iter().any(|(n, _)| n == name) {
            return Err(format!("table `{name}` is listed twice in `tables`"));
        }
        out.push((name.to_string(), path.to_string()));
    }
    if out.is_empty() {
        return Err("`tables` needs at least one `name=path` entry".to_string());
    }
    Ok(out)
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.f{1,6}]Z` to microseconds since the Unix epoch.
pub fn parse_utc_datetime(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut date = date.split('-');
    let (y, m, d) = (date.next()?, date.next()?, date.next()?);
    if date.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    let (hms, frac) = match time.split_once('.') {
        Some((hms, frac)) => (hms, frac),
        None => (time, ""),
    };
    let mut hms = hms.split(':');
    let (hh, mm, ss) = (hms.next()?, hms.next()?, hms.next()?);
    if hms.next().is_some() || [hh, mm, ss].iter().any(|p| p.len() != 2) {
        return None;
    }
    let (hh, mm, ss): (i64, i64, i64) = (hh.parse().ok()?, mm.parse().ok()?, ss.parse().ok()?);
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) || hh > 23 || mm > 59 || ss > 59
    {
        return None;
    }
    let micros = if frac.is_empty() {
        0
    } else {
        if frac.len() > 6 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        format!("{frac:0<6}").parse::<i64>().ok()?
    };
    let secs = days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss;
    Some(secs * 1_000_000 + micros)
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Howard Hinnant's `days_from_civil`.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_err(text: &str) -> String {
        TestCase::parse(text).unwrap_err().to_string()
    }

    #[test]
    fn full_case() {
        let text = "# Line one.\n#Line two.\nmode: check\norder: any\nnow: 2024-01-02T03:04:05.5Z\n\
                    engine: datafusion, sql-duckdb\ntables: t=../data/a.csv, u = b.csv\nskip: M1-02\n\
                    ---- query\nt | take 1\n\n\n---- expected\nout\n---- expected[sql-duckdb]\nother\n";
        let case = TestCase::parse(text).unwrap();
        assert_eq!(case.description, ["Line one.", "Line two."]);
        assert_eq!(case.mode, Mode::Check);
        assert_eq!(case.order, Some(Order::Any));
        assert_eq!(case.now_micros, 1_704_164_645_500_000);
        assert_eq!(case.engines.value, ["datafusion", "sql-duckdb"]);
        assert_eq!(
            case.tables,
            [
                ("t".to_string(), "../data/a.csv".to_string()),
                ("u".to_string(), "b.csv".to_string())
            ]
        );
        assert_eq!(case.skip.as_deref(), Some("M1-02"));
        assert_eq!(case.query, "t | take 1\n");
        assert_eq!(case.query_line, 10);
        assert_eq!(case.expected_for("datafusion").unwrap().body, "out");
        assert_eq!(case.expected_for("sql-duckdb").unwrap().body, "other");
        let shared = &case.expected[0];
        assert_eq!(&text[shared.range.clone()], "---- expected\nout\n");
    }

    #[test]
    fn defaults_and_crlf() {
        let case = TestCase::parse("---- query\r\nx\r\n---- expected\r\na\r\nb\r\n").unwrap();
        assert_eq!(case.mode, Mode::Run);
        assert_eq!(case.order, None);
        assert_eq!(case.now, DEFAULT_NOW);
        assert_eq!(case.now_micros, 1_717_200_000_000_000);
        assert_eq!(case.engines.value, [DEFAULT_ENGINE]);
        assert_eq!(case.query, "x\n");
        assert_eq!(case.expected[0].body, "a\nb");
    }

    #[test]
    fn eol_crlf_converts_the_query() {
        let case = TestCase::parse("eol: crlf\n---- query\na\nb\n\n---- expected\nx\n").unwrap();
        assert_eq!(case.query, "a\r\nb\r\n");
        let error = TestCase::parse("eol: cr\n---- query\na\n").unwrap_err();
        assert!(error.to_string().contains("unknown eol `cr`"), "{error}");
    }

    #[test]
    fn table_separators_are_not_sections() {
        let text = "---- query\nx\n---- expected\nabc\nstring\n----\n---------+---\nv\n";
        let case = TestCase::parse(text).unwrap();
        assert_eq!(case.expected[0].body, "abc\nstring\n----\n---------+---\nv");
    }

    #[test]
    fn errors_name_the_line() {
        assert_eq!(
            parse_err("mode: chek\n---- query\nx\n"),
            "line 1: unknown mode `chek` (did you mean `check`?)"
        );
        assert_eq!(
            parse_err("# d\nmdoe: run\n---- query\nx\n"),
            "line 2: unknown header key `mdoe` (did you mean `mode`?)"
        );
        assert_eq!(
            parse_err("mode run\n---- query\nx\n"),
            "line 1: expected a `key: value` header, a `#` description or a `---- query` section, found `mode run`"
        );
        assert_eq!(
            parse_err("mode: run\nmode: check\n---- query\nx\n"),
            "line 2: header key `mode` is set twice"
        );
        assert_eq!(
            parse_err("---- query\nx\n---- expectd\ny\n"),
            "line 3: unknown section `expectd` (did you mean `expected`?)"
        );
        assert_eq!(
            parse_err("---- query\nx\n---- expected\ny\n---- expected\nz\n"),
            "line 5: duplicate `---- expected` section"
        );
        assert_eq!(
            parse_err("----query\nx\n"),
            "line 1: malformed section header `----query`; write `---- query` or `---- expected`"
        );
        assert_eq!(
            parse_err("mode: run\n---- expected\ny\n"),
            "line 2: missing `---- query` section"
        );
        assert_eq!(
            parse_err("mode: run\n"),
            "line 1: missing `---- query` section"
        );
        assert_eq!(
            parse_err("---- query\n\n---- expected\n"),
            "line 1: the `---- query` section is empty"
        );
        assert_eq!(
            parse_err("---- query\nx\n---- expected[duckdb]\ny\n"),
            "line 3: section is for engine `duckdb`, which the `engine` header does not list"
        );
        assert_eq!(
            parse_err("---- query\nx\n---- expected[]\ny\n"),
            "line 3: malformed section header `---- expected[]`; write `---- expected[<engine>]`"
        );
        assert_eq!(
            parse_err("now: 2024-02-30T00:00:00Z\n---- query\nx\n"),
            "line 1: `now` must be an ISO 8601 UTC datetime such as `2024-06-01T00:00:00Z`, found `2024-02-30T00:00:00Z`"
        );
        assert_eq!(
            parse_err("tables: t\n---- query\nx\n"),
            "line 1: `tables` entry `t` must look like `name=path/to/file.csv`"
        );
        assert_eq!(
            parse_err("order: sorted\n---- query\nx\n"),
            "line 1: unknown order `sorted` (expected one of: `strict`, `any`)"
        );
        assert_eq!(
            parse_err("schema: t(a: lng)\n---- query\nx\n"),
            "line 1: `schema`: unknown type `lng` for column `a` (did you mean `long`?)"
        );
    }

    #[test]
    fn datetimes() {
        assert_eq!(parse_utc_datetime("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_utc_datetime("2024-02-29T23:59:59.000001Z"),
            Some(1_709_251_199_000_001)
        );
        assert_eq!(parse_utc_datetime("2024-01-01T00:00:00"), None);
        assert_eq!(parse_utc_datetime("2024-01-01 00:00:00Z"), None);
        assert_eq!(parse_utc_datetime("2024-01-01T24:00:00Z"), None);
        assert_eq!(parse_utc_datetime("2024-01-01T00:00:00.1234567Z"), None);
    }
}
