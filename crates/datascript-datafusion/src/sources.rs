//! Reading sources: from a [`SourceRef`] to the files it names, each file's own schema, and the
//! merged, widened schema the compiler sees (LANGUAGE.md §3.1, §5.1, D-012, D-023).
//!
//! A source resolves to a [`Resolved`]: its files, sorted by path, with the schema DataFusion
//! infers for each one. Files are merged **by name**, never by position, so CSV files whose
//! headers list the same columns in another order read correctly. A column missing from a file is
//! null there; one whose types differ is widened by `Type::common` (LANGUAGE.md §4.2), or the
//! source fails with [`SourceError::Conflict`].
//!
//! The scan (in `lib.rs`) reads each file with its own schema and casts every column to the
//! merged type, so it never depends on DataFusion applying one table schema to every file.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, SchemaRef};
use datafusion::datasource::file_format::FileFormat;
use datafusion::datasource::file_format::csv::CsvFormat;
use datafusion::datasource::file_format::file_compression_type::FileCompressionType;
use datafusion::datasource::file_format::json::JsonFormat;
use datafusion::datasource::file_format::parquet::ParquetFormat;
use datafusion::datasource::listing::{ListingOptions, ListingTableUrl};
use datafusion::error::DataFusionError;
use datafusion::prelude::SessionContext;
use datascript_compiler::arrow::{from_arrow_source, to_arrow};
use datascript_compiler::capabilities::SourceFormat;
use datascript_compiler::source::{COMPRESSION_SUFFIXES, format_of_path};
use datascript_compiler::{FileType, OptionValue, ReadProblem, SourceError, SourceRef, Type};
use datascript_engine::ExecContext;

/// The default of `read_csv`'s `infer_rows` (LANGUAGE.md §3.1).
const DEFAULT_INFER_ROWS: usize = 1000;

/// A source with its files and schemas.
#[derive(Debug)]
pub(crate) struct Resolved {
    pub reader: Reader,
    /// The matching files, sorted by path. Never empty.
    pub files: Vec<SourceFile>,
    /// The merged schema, widened to DataScript types (what `schema_of` returns). Columns with
    /// no DataScript type keep their Arrow type.
    pub widened: SchemaRef,
}

impl Resolved {
    /// Whether every file has the same schema, so one DataFusion table can read them all.
    pub fn uniform(&self) -> bool {
        self.files.windows(2).all(|w| w[0].raw == w[1].raw)
    }
}

/// One file of a source.
#[derive(Debug)]
pub(crate) struct SourceFile {
    pub url: ListingTableUrl,
    /// The schema DataFusion infers for this file alone.
    pub raw: SchemaRef,
}

/// How to read the files of a source: the format and its options.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Reader {
    Csv(CsvReader),
    Json {
        newline_delimited: bool,
        compression: FileCompressionType,
    },
    Parquet,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CsvReader {
    delimiter: u8,
    header: bool,
    quote: u8,
    escape: Option<u8>,
    /// Read by the scan: DataFusion 55 applies `null_regex` when it infers the schema but not
    /// when it reads rows, so the scan reads text and turns this value into null itself.
    pub null_value: Option<String>,
    infer_rows: usize,
    compression: FileCompressionType,
}

impl Reader {
    /// The text that means null, if `read_csv`'s `null_value` gave one.
    pub fn null_value(&self) -> Option<&str> {
        match self {
            Reader::Csv(csv) => csv.null_value.as_deref(),
            _ => None,
        }
    }

    /// The DataFusion listing options for files read this way.
    pub fn listing_options(&self) -> ListingOptions {
        let format: Arc<dyn FileFormat> = match self {
            Reader::Csv(csv) => Arc::new(
                CsvFormat::default()
                    .with_has_header(csv.header)
                    .with_delimiter(csv.delimiter)
                    .with_quote(csv.quote)
                    .with_escape(csv.escape)
                    .with_null_regex(csv.null_value.as_deref().map(exact_regex))
                    .with_schema_infer_max_rec(csv.infer_rows)
                    .with_newlines_in_values(true)
                    .with_file_compression_type(csv.compression),
            ),
            Reader::Json {
                newline_delimited,
                compression,
            } => Arc::new(
                JsonFormat::default()
                    .with_newline_delimited(*newline_delimited)
                    .with_schema_infer_max_rec(DEFAULT_INFER_ROWS)
                    .with_file_compression_type(*compression),
            ),
            // Without view types, columns keep the types the file has (`Binary`, not
            // `BinaryView`), which is what DS2008 shows the user.
            Reader::Parquet => Arc::new(ParquetFormat::default().with_force_view_types(false)),
        };
        // Each table lists exactly the files resolved here, so no extension filter is needed
        // (and a filter would have to know about compression suffixes).
        ListingOptions::new(format).with_file_extension("")
    }
}

/// A regex that matches exactly `text`, for `null_value`.
fn exact_regex(text: &str) -> String {
    format!("^{}$", regex::escape(text))
}

/// What a `SourceRef` names once host tables are looked up.
struct Request<'a> {
    /// The path or glob as written.
    path: &'a str,
    glob: bool,
    format: SourceFormat,
    options: &'a [(String, OptionValue)],
}

/// Resolves `source` and reads the schema of every file it names.
///
/// `tables` are the host's tables. A host table's own failures are reported as the file's, since
/// [`SourceError::NotFound`] for a table means there is no table of that name.
pub(crate) async fn resolve(
    session: &SessionContext,
    source: &SourceRef,
    tables: &BTreeMap<String, SourceRef>,
    ctx: &ExecContext,
) -> Result<Resolved, SourceError> {
    let (source, host_table) = match source {
        SourceRef::Table(name) => match tables.get(name) {
            Some(inner) => (inner, true),
            None => {
                return Err(SourceError::NotFound {
                    nearby: tables.keys().cloned().collect(),
                });
            }
        },
        other => (other, false),
    };
    let request = match source {
        SourceRef::Path(path) | SourceRef::Glob(path) => Request {
            path,
            glob: matches!(source, SourceRef::Glob(_)),
            format: format_of_path(path).ok_or_else(|| unknown_format(path))?,
            options: &[],
        },
        SourceRef::Reader(call) => Request {
            path: &call.path,
            glob: datascript_compiler::source::is_glob(&call.path),
            format: call.format(),
            options: &call.options,
        },
        // Hosts register files, not other tables.
        SourceRef::Table(name) => {
            return Err(SourceError::Unreadable {
                file: name.clone(),
                problem: ReadProblem::Io("a host table cannot name another table".into()),
            });
        }
    };
    let result = read(session, &request, ctx).await;
    match result {
        Err(SourceError::NotFound { .. }) if host_table => Err(SourceError::Unreadable {
            file: request.path.to_string(),
            problem: ReadProblem::Io("the file registered for this table does not exist".into()),
        }),
        other => other,
    }
}

/// The error for a host table whose path has no known extension. The binder reports `DS3002`
/// for paths in the program, so this only happens for a host that registers such a table.
fn unknown_format(path: &str) -> SourceError {
    SourceError::Unreadable {
        file: path.to_string(),
        problem: ReadProblem::Io("the format cannot be told from the file name".into()),
    }
}

async fn read(
    session: &SessionContext,
    request: &Request<'_>,
    ctx: &ExecContext,
) -> Result<Resolved, SourceError> {
    let paths = if request.glob {
        glob_files(request.path, ctx)?
    } else {
        vec![single_file(request.path, ctx)?]
    };
    let compression = compression(request)?;
    let csv = |delimiter: u8| -> Result<Reader, SourceError> {
        let char_option = |name: &str| match option(request, name) {
            Some(OptionValue::String(s)) => s.bytes().next(),
            _ => None,
        };
        Ok(Reader::Csv(CsvReader {
            delimiter: char_option("delimiter").unwrap_or(delimiter),
            header: !matches!(option(request, "header"), Some(OptionValue::Bool(false))),
            quote: char_option("quote").unwrap_or(b'"'),
            escape: char_option("escape"),
            null_value: match option(request, "null_value") {
                Some(OptionValue::String(s)) => Some(s.clone()),
                _ => None,
            },
            infer_rows: match option(request, "infer_rows") {
                Some(OptionValue::Long(n)) => usize::try_from(*n).unwrap_or(usize::MAX),
                _ => DEFAULT_INFER_ROWS,
            },
            compression,
        }))
    };
    let reader = match request.format {
        SourceFormat::Csv => csv(b',')?,
        SourceFormat::Tsv => csv(b'\t')?,
        SourceFormat::Parquet => Reader::Parquet,
        SourceFormat::Ndjson => Reader::Json {
            newline_delimited: true,
            compression,
        },
        SourceFormat::Json => {
            let newline_delimited = match option(request, "format") {
                Some(OptionValue::String(f)) => f != "array",
                _ => !is_json_array(&paths[0].0, compression)?,
            };
            Reader::Json {
                newline_delimited,
                compression,
            }
        }
    };

    let options = reader.listing_options();
    let state = session.state();
    let mut files = Vec::with_capacity(paths.len());
    for (path, shown) in paths {
        // ASCII-only characters with a meaning in URLs are escaped by `ListingTableUrl` itself;
        // a path it cannot turn into a URL is unreadable.
        let url = ListingTableUrl::parse(strip_verbatim(&path).to_string_lossy()).map_err(|e| {
            SourceError::Unreadable {
                file: shown.clone(),
                problem: ReadProblem::Io(e.strip_backtrace()),
            }
        })?;
        let raw =
            options
                .infer_schema(&state, &url)
                .await
                .map_err(|e| SourceError::Unreadable {
                    file: shown.clone(),
                    problem: read_problem(&e),
                })?;
        files.push((shown, SourceFile { url, raw }));
    }
    let widened = merge(&files)?;
    Ok(Resolved {
        reader,
        files: files.into_iter().map(|(_, file)| file).collect(),
        widened,
    })
}

fn option<'a>(request: &'a Request<'_>, name: &str) -> Option<&'a OptionValue> {
    request
        .options
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
}

/// The compression from the `compression` option, else from the path's suffix.
fn compression(request: &Request<'_>) -> Result<FileCompressionType, SourceError> {
    let name = match option(request, "compression") {
        Some(OptionValue::String(name)) => name.as_str(),
        _ => {
            let lower = request.path.to_ascii_lowercase();
            match COMPRESSION_SUFFIXES.iter().find(|s| lower.ends_with(*s)) {
                Some(&".gz") => "gzip",
                Some(&".bz2") => "bzip2",
                Some(&".xz") => "xz",
                Some(&".zst") => "zstd",
                _ => return Ok(FileCompressionType::UNCOMPRESSED),
            }
        }
    };
    let compression = match name {
        "gzip" => FileCompressionType::GZIP,
        "bzip2" => FileCompressionType::BZIP2,
        "xz" => FileCompressionType::XZ,
        _ => FileCompressionType::ZSTD,
    };
    if cfg!(feature = "compression") {
        Ok(compression)
    } else {
        Err(SourceError::Unreadable {
            file: request.path.to_string(),
            problem: ReadProblem::Io("this build cannot read compressed files".into()),
        })
    }
}

/// Whether a `.json` file holds one array (rather than one object per line): its first
/// character that is not white space is `[`.
fn is_json_array(path: &Path, compression: FileCompressionType) -> Result<bool, SourceError> {
    use std::io::Read;
    let io = |e: &dyn std::fmt::Display| SourceError::Unreadable {
        file: path.display().to_string(),
        problem: ReadProblem::Io(e.to_string()),
    };
    let file = std::fs::File::open(path).map_err(|e| io(&e))?;
    let mut reader = compression.convert_read(file).map_err(|e| io(&e))?;
    let mut buf = [0u8; 256];
    loop {
        let n = reader.read(&mut buf).map_err(|e| io(&e))?;
        if n == 0 {
            return Ok(false);
        }
        if let Some(&c) = buf[..n].iter().find(|c| !c.is_ascii_whitespace()) {
            return Ok(c == b'[');
        }
    }
}

/// Classifies an error from schema inference.
fn read_problem(error: &DataFusionError) -> ReadProblem {
    if let DataFusionError::IoError(io) = error.find_root() {
        return io_problem(io);
    }
    ReadProblem::Malformed(error.strip_backtrace())
}

fn io_problem(error: &std::io::Error) -> ReadProblem {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => ReadProblem::PermissionDenied,
        std::io::ErrorKind::IsADirectory => ReadProblem::IsDirectory,
        _ => ReadProblem::Io(error.to_string()),
    }
}

/// A merged column's type so far.
#[derive(Debug, Clone, PartialEq)]
enum Merged {
    /// Only nulls so far: an all-empty CSV column, or JSON values that are all `null`. It takes
    /// the type of the other files, and is `string` if none has one.
    Null,
    Typed(Type),
    /// No DataScript type: merges only with the same Arrow type.
    Arrow(DataType),
}

impl Merged {
    fn of(field: &Field) -> Merged {
        match from_arrow_source(field) {
            Ok(ty) => Merged::Typed(ty),
            Err(_) if field.data_type() == &DataType::Null => Merged::Null,
            Err(_) => Merged::Arrow(field.data_type().clone()),
        }
    }

    fn name(&self) -> String {
        match self {
            Merged::Null => "null".to_string(),
            Merged::Typed(ty) => ty.to_string(),
            Merged::Arrow(dt) => dt.to_string(),
        }
    }

    fn merge(&self, other: &Merged) -> Option<Merged> {
        match (self, other) {
            (Merged::Null, x) | (x, Merged::Null) => Some(x.clone()),
            (Merged::Typed(a), Merged::Typed(b)) => a.common(*b).map(Merged::Typed),
            (Merged::Arrow(a), Merged::Arrow(b)) if a == b => Some(self.clone()),
            _ => None,
        }
    }

    fn field(&self, name: &str) -> Field {
        match self {
            Merged::Null => to_arrow(name, Type::String),
            Merged::Typed(ty) => to_arrow(name, *ty),
            Merged::Arrow(dt) => Field::new(name, dt.clone(), true),
        }
    }
}

/// Merges the files' schemas by name: columns in order of first appearance, each widened to its
/// DataScript type and to a common type across files.
fn merge(files: &[(String, SourceFile)]) -> Result<SchemaRef, SourceError> {
    struct Column {
        name: String,
        ty: Merged,
        /// The first file whose type for the column is not null.
        first: FileType,
    }
    let mut columns: Vec<Column> = Vec::new();
    for (shown, file) in files {
        for field in file.raw.fields() {
            let ty = Merged::of(field);
            let this = FileType {
                file: shown.clone(),
                ty: ty.name(),
            };
            let Some(column) = columns.iter_mut().find(|c| &c.name == field.name()) else {
                columns.push(Column {
                    name: field.name().clone(),
                    ty,
                    first: this,
                });
                continue;
            };
            let merged = column.ty.merge(&ty).ok_or_else(|| SourceError::Conflict {
                column: field.name().clone(),
                first: column.first.clone(),
                second: this.clone(),
            })?;
            if column.ty == Merged::Null {
                column.first = this;
            }
            column.ty = merged;
        }
    }
    Ok(Arc::new(Schema::new(
        columns
            .iter()
            .map(|c| c.ty.field(&c.name))
            .collect::<Vec<_>>(),
    )))
}

// --- Paths ---

/// The file at `path`, resolved against the base directory, with the name to show for it.
fn single_file(path: &str, ctx: &ExecContext) -> Result<(PathBuf, String), SourceError> {
    let resolved = resolve_path(path, &ctx.base_dir);
    check_sandbox(&resolved, ctx)?;
    let shown = path.to_string();
    match std::fs::metadata(&resolved) {
        Ok(meta) if meta.is_dir() => Err(SourceError::Unreadable {
            file: shown,
            problem: ReadProblem::IsDirectory,
        }),
        Ok(_) => {
            // Opening the file finds permission problems before DataFusion's less clear errors.
            std::fs::File::open(&resolved).map_err(|e| SourceError::Unreadable {
                file: shown.clone(),
                problem: io_problem(&e),
            })?;
            Ok((resolved, shown))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SourceError::NotFound {
            nearby: nearby_files(&resolved),
        }),
        Err(e) => Err(SourceError::Unreadable {
            file: shown,
            problem: io_problem(&e),
        }),
    }
}

/// The files matching `pattern`, sorted by path, with the names to show for them: as the
/// pattern spells them (relative to the base directory when the pattern is relative), with `/`
/// as the separator.
fn glob_files(pattern: &str, ctx: &ExecContext) -> Result<Vec<(PathBuf, String)>, SourceError> {
    let relative = !Path::new(pattern).is_absolute();
    let full = if relative {
        let base = glob::Pattern::escape(&ctx.base_dir.to_string_lossy());
        format!("{base}/{pattern}")
    } else {
        pattern.to_string()
    };
    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };
    // Check the pattern as written, so the position the error names is in the user's text.
    if let Err(e) = glob::Pattern::new(pattern) {
        return Err(SourceError::BadPattern {
            reason: e.msg.to_string(),
            position: e.pos,
        });
    }
    let matches = glob::glob_with(&full, options).map_err(|e| SourceError::BadPattern {
        reason: e.msg.to_string(),
        position: e.pos,
    })?;
    let mut files = Vec::new();
    for entry in matches {
        // A directory that cannot be listed is skipped, as a shell would.
        let Ok(path) = entry else { continue };
        if !path.is_file() {
            continue;
        }
        check_sandbox(&path, ctx)?;
        let shown = match path.strip_prefix(&ctx.base_dir) {
            Ok(rest) if relative => rest.to_string_lossy().into_owned(),
            _ => path.to_string_lossy().into_owned(),
        };
        files.push((path, shown.replace('\\', "/")));
    }
    if files.is_empty() {
        return Err(SourceError::NoMatches);
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(files)
}

pub(crate) fn resolve_path(path: &str, base_dir: &Path) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

/// `OutsideSandbox` if `ctx` has a sandbox and `path` is not inside it (D-016). Paths are
/// compared after resolving symbolic links where the file exists, and `..` otherwise.
fn check_sandbox(path: &Path, ctx: &ExecContext) -> Result<(), SourceError> {
    let Some(root) = &ctx.sandbox else {
        return Ok(());
    };
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| normalize(p));
    if real(path).starts_with(real(root)) {
        Ok(())
    } else {
        Err(SourceError::OutsideSandbox {
            root: root.display().to_string(),
        })
    }
}

/// `path` with `.` and `..` components removed lexically.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` without a Windows verbatim prefix: `\\?\C:\x` becomes `C:\x` and
/// `\\?\UNC\server\share\x` becomes `\\server\share\x`.
///
/// `canonicalize` returns verbatim paths on Windows, and `ListingTableUrl` (through the `url`
/// crate) cannot turn one into a `file://` URL that maps back to an absolute path. Other paths,
/// and every path on other platforms, come back unchanged.
pub(crate) fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// File names in the directory of `path`, sorted, for a did-you-mean.
fn nearby_files(path: &Path) -> Vec<String> {
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}
