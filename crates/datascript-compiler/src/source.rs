//! Sources and schema discovery (ARCHITECTURE.md § Semantics, D-022, D-023).
//!
//! The compiler never reads files. It asks a [`SchemaProvider`] for each source's schema. The
//! provider is a synchronous cache: on a miss it answers [`SchemaLookup::Pending`], the binder
//! lists the source in [`Analysis::pending_sources`](crate::Analysis::pending_sources), and the
//! session fetches it and binds again.
//!
//! The reader functions and their options (LANGUAGE.md §3.1) are defined here too: [`READERS`].

use std::fmt;

use arrow_schema::SchemaRef;

use crate::capabilities::SourceFormat;

/// A source as the program names it. Engines resolve it (relative paths against the base
/// directory of the run).
///
/// Every value in it is a constant, so the same text always names the same source (D-022).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum SourceRef {
    /// A file path, exactly as the string's value spells it. The format comes from the
    /// extension ([`format_of_path`]).
    Path(String),
    /// A path with glob characters (`*`, `?`, `[`): every matching file, sorted by path.
    Glob(String),
    /// A reader function call such as `read_csv('x.txt', delimiter = ';')`.
    Reader(ReaderCall),
    /// A table registered by the host (`--table orders=path`, web UI drag-drop).
    Table(String),
}

impl SourceRef {
    /// A path or glob source for a constant string: a glob if it has glob characters.
    pub fn for_path(path: impl Into<String>) -> SourceRef {
        let path = path.into();
        if is_glob(&path) {
            SourceRef::Glob(path)
        } else {
            SourceRef::Path(path)
        }
    }

    /// The format the source is read as, if the compiler can tell (a host table's format is
    /// the host's business).
    pub fn format(&self) -> Option<SourceFormat> {
        match self {
            SourceRef::Path(path) | SourceRef::Glob(path) => format_of_path(path),
            SourceRef::Reader(call) => Some(call.format()),
            SourceRef::Table(_) => None,
        }
    }

    /// The file path or glob, if the source has one.
    pub fn path(&self) -> Option<&str> {
        match self {
            SourceRef::Path(path) | SourceRef::Glob(path) => Some(path),
            SourceRef::Reader(call) => Some(&call.path),
            SourceRef::Table(_) => None,
        }
    }
}

impl fmt::Display for SourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceRef::Path(path) | SourceRef::Glob(path) => write!(f, "'{path}'"),
            SourceRef::Reader(call) => write!(f, "{call}"),
            SourceRef::Table(name) => write!(f, "table `{name}`"),
        }
    }
}

/// Whether a path has glob characters (LANGUAGE.md §3.1).
pub fn is_glob(path: &str) -> bool {
    path.contains(['*', '?', '['])
}

/// The compression suffixes CSV, TSV and JSON files may have (LANGUAGE.md §3.1).
pub const COMPRESSION_SUFFIXES: &[&str] = &[".gz", ".bz2", ".xz", ".zst"];

/// The format of a file from its extension, ignoring case (LANGUAGE.md §3.1). `None` if the
/// extension is not one DataScript knows.
pub fn format_of_path(path: &str) -> Option<SourceFormat> {
    let lower = path.to_ascii_lowercase();
    let (stem, compressed) = match COMPRESSION_SUFFIXES.iter().find(|s| lower.ends_with(*s)) {
        Some(suffix) => (&lower[..lower.len() - suffix.len()], true),
        None => (lower.as_str(), false),
    };
    let ext = stem.rsplit_once('.').map(|(_, ext)| ext)?;
    let format = match ext {
        "csv" => SourceFormat::Csv,
        "tsv" => SourceFormat::Tsv,
        "parquet" if !compressed => SourceFormat::Parquet,
        "json" => SourceFormat::Json,
        "ndjson" | "jsonl" => SourceFormat::Ndjson,
        _ => return None,
    };
    Some(format)
}

/// A reader function call with constant arguments.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ReaderCall {
    /// The function: `read_csv`, `read_parquet` or `read_json`.
    pub function: String,
    /// The path or glob.
    pub path: String,
    /// The options given, sorted by name.
    pub options: Vec<(String, OptionValue)>,
}

impl ReaderCall {
    /// The format the call reads. `read_json(..., format = "ndjson")` reads NDJSON.
    pub fn format(&self) -> SourceFormat {
        match self.function.as_str() {
            "read_parquet" => SourceFormat::Parquet,
            "read_json" => match self.option("format") {
                Some(OptionValue::String(f)) if f == "ndjson" => SourceFormat::Ndjson,
                _ => SourceFormat::Json,
            },
            _ => SourceFormat::Csv,
        }
    }

    /// The value of option `name`, if given.
    pub fn option(&self, name: &str) -> Option<&OptionValue> {
        self.options.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

impl fmt::Display for ReaderCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}('{}'", self.function, self.path)?;
        for (name, value) in &self.options {
            write!(f, ", {name} = {value}")?;
        }
        f.write_str(")")
    }
}

/// The constant value of a reader option.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum OptionValue {
    Bool(bool),
    Long(i64),
    String(String),
}

impl fmt::Display for OptionValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OptionValue::Bool(b) => write!(f, "{b}"),
            OptionValue::Long(n) => write!(f, "{n}"),
            OptionValue::String(s) => write!(f, "'{s}'"),
        }
    }
}

/// What values a reader option takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Bool,
    /// A `long` of at least 1.
    PositiveLong,
    /// A string of exactly one character.
    Char,
    /// Any string.
    String,
    /// One of these strings.
    OneOf(&'static [&'static str]),
}

/// One option of a reader function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderOption {
    pub name: &'static str,
    pub kind: OptionKind,
    /// The milestone that adds it, if it is not available yet.
    pub planned: Option<&'static str>,
}

/// A reader function (LANGUAGE.md §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderDef {
    pub name: &'static str,
    pub options: &'static [ReaderOption],
}

impl ReaderDef {
    /// The option called `name`.
    pub fn option(&self, name: &str) -> Option<&ReaderOption> {
        self.options.iter().find(|o| o.name == name)
    }
}

/// The values of the `compression` option.
pub const COMPRESSIONS: &[&str] = &["gzip", "bzip2", "xz", "zstd"];

const fn option(name: &'static str, kind: OptionKind) -> ReaderOption {
    ReaderOption {
        name,
        kind,
        planned: None,
    }
}

/// The reader functions, sorted by name, with their M1 options (LANGUAGE.md §3.1). The parser's
/// `datascript_syntax::READERS` lists the same names.
pub const READERS: &[ReaderDef] = &[
    ReaderDef {
        name: "read_csv",
        options: &[
            option("delimiter", OptionKind::Char),
            option("header", OptionKind::Bool),
            option("quote", OptionKind::Char),
            option("escape", OptionKind::Char),
            option("null_value", OptionKind::String),
            ReaderOption {
                name: "schema",
                kind: OptionKind::String,
                planned: Some("M2"),
            },
            option("infer_rows", OptionKind::PositiveLong),
            option("compression", OptionKind::OneOf(COMPRESSIONS)),
        ],
    },
    ReaderDef {
        name: "read_json",
        options: &[
            option("format", OptionKind::OneOf(&["ndjson", "array"])),
            option("infer_rows", OptionKind::PositiveLong),
            option("compression", OptionKind::OneOf(COMPRESSIONS)),
        ],
    },
    ReaderDef {
        name: "read_parquet",
        options: &[],
    },
];

/// The reader function called `name`.
pub fn reader(name: &str) -> Option<&'static ReaderDef> {
    READERS.iter().find(|r| r.name == name)
}

/// Why a source's schema could not be found. It carries no span: the binder builds the DS3xxx
/// diagnostic at every place the source appears (D-023).
///
/// File names in it are as the engine shows them to the user: relative to the base directory
/// when they are inside it (DIAGNOSTICS.md rule 12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceError {
    /// No file at the path, or no host table with the name. `nearby` lists file names in the
    /// same directory (or the host's table names), sorted, for a did-you-mean (`DS3001`,
    /// `DS1002`).
    NotFound { nearby: Vec<String> },
    /// A glob that matched no file (`DS3004`).
    NoMatches,
    /// A glob that is not a valid pattern, such as an unclosed `[` (`DS3009`). `reason` is
    /// the pattern parser's message; `position` is the byte offset in the pattern it points at.
    BadPattern { reason: String, position: usize },
    /// Two files of a glob have a column whose types cannot be widened to one (`DS3005`,
    /// LANGUAGE.md §3.1).
    Conflict {
        column: String,
        /// The first file with the column, and its type there.
        first: FileType,
        /// The first file whose type cannot be merged with the types before it.
        second: FileType,
    },
    /// The path resolves outside the directory the host allows (`DS3007`, D-016).
    OutsideSandbox { root: String },
    /// The file exists but cannot be read (`DS3008`).
    Unreadable { file: String, problem: ReadProblem },
}

/// A file of a glob and a column's type in it, for [`SourceError::Conflict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileType {
    pub file: String,
    /// The DataScript type name, or the Arrow type for a column with no DataScript type.
    pub ty: String,
}

/// Why a file cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadProblem {
    PermissionDenied,
    /// The path is a directory.
    IsDirectory,
    /// The contents are not valid for the format. The text is the engine's message.
    Malformed(String),
    /// Any other I/O error. The text is the system's message.
    Io(String),
}

/// The answer of a [`SchemaProvider`].
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaLookup {
    /// The schema, already widened to DataScript types (LANGUAGE.md §5.1). Columns with no
    /// DataScript type keep their Arrow type; they can be projected but not used (DS2008).
    Ready(SchemaRef),
    /// The source cannot be read.
    Failed(SourceError),
    /// Not known yet. The binder gives the source the error type silently and lists it in
    /// `pending_sources`.
    Pending,
}

/// Answers schema lookups synchronously, from a cache (D-022).
pub trait SchemaProvider {
    /// The schema of `source`, or [`SchemaLookup::Pending`] if it has not been fetched yet.
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup;
}

/// A provider that knows nothing: every source is pending. A first binding pass with it finds
/// every source the program uses.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSchemas;

impl SchemaProvider for NoSchemas {
    fn schema_of(&self, _source: &SourceRef) -> SchemaLookup {
        SchemaLookup::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_by_extension() {
        let cases = [
            ("a.csv", Some(SourceFormat::Csv)),
            ("dir/A.CSV", Some(SourceFormat::Csv)),
            ("a.tsv.gz", Some(SourceFormat::Tsv)),
            ("logs/*.parquet", Some(SourceFormat::Parquet)),
            ("a.parquet.gz", None),
            ("a.json.zst", Some(SourceFormat::Json)),
            ("a.jsonl", Some(SourceFormat::Ndjson)),
            ("a.ndjson.xz", Some(SourceFormat::Ndjson)),
            ("a.txt", None),
            ("noext", None),
            ("a.gz", None),
        ];
        for (path, format) in cases {
            assert_eq!(format_of_path(path), format, "{path}");
        }
    }

    #[test]
    fn globs_and_display() {
        assert_eq!(
            SourceRef::for_path("a.csv"),
            SourceRef::Path("a.csv".into())
        );
        assert_eq!(
            SourceRef::for_path("logs/*.csv"),
            SourceRef::Glob("logs/*.csv".into())
        );
        let call = ReaderCall {
            function: "read_csv".into(),
            path: "x.txt".into(),
            options: vec![
                ("delimiter".into(), OptionValue::String(";".into())),
                ("header".into(), OptionValue::Bool(false)),
            ],
        };
        assert_eq!(
            SourceRef::Reader(call).to_string(),
            "read_csv('x.txt', delimiter = ';', header = false)"
        );
        assert_eq!(SourceRef::Table("t".into()).to_string(), "table `t`");
    }

    #[test]
    fn readers_are_sorted() {
        let names: Vec<_> = READERS.iter().map(|r| r.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }
}
