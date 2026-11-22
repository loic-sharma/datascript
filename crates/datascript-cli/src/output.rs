//! Printing results: `ds run --format table|csv|json|ndjson|arrow`.

use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};

use arrow_array::RecordBatch;
use arrow_schema::{ArrowError, Schema};
use datascript_diagnostics::SourceFile;
use datascript_engine::QueryResult;
use datascript_engine::table::{datascript_type, format_table};
use datascript_engine::writers::{json_rows, write_csv, write_ndjson};
use serde_json::{Value, json};

use crate::args::RunFormat;

/// Where results go: standard output or `--output <path>`.
#[derive(Debug, Clone)]
pub enum Destination {
    Stdout,
    File(PathBuf),
}

impl Destination {
    pub fn new(path: Option<&Path>) -> Self {
        match path {
            Some(path) => Destination::File(path.to_path_buf()),
            None => Destination::Stdout,
        }
    }

    /// Whether results would land on a terminal.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Destination::Stdout) && io::stdout().is_terminal()
    }

    fn open(&self) -> Result<Box<dyn Write>, OutputError> {
        match self {
            Destination::Stdout => Ok(Box::new(BufWriter::new(io::stdout().lock()))),
            Destination::File(path) => match File::create(path) {
                Ok(file) => Ok(Box::new(BufWriter::new(file))),
                Err(error) => Err(OutputError::Io {
                    target: self.to_string(),
                    error,
                }),
            },
        }
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Destination::Stdout => f.write_str("standard output"),
            Destination::File(path) => write!(f, "`{}`", path.display()),
        }
    }
}

/// Writing results failed.
#[derive(Debug)]
pub enum OutputError {
    /// The reader closed the pipe (`ds run … | head`).
    BrokenPipe,
    Io {
        target: String,
        error: io::Error,
    },
}

impl OutputError {
    fn new(destination: &Destination, error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::BrokenPipe {
            OutputError::BrokenPipe
        } else {
            OutputError::Io {
                target: destination.to_string(),
                error,
            }
        }
    }
}

/// Writes `text` to standard output.
pub fn write_stdout(text: &[u8]) -> Result<(), OutputError> {
    let mut out = io::stdout().lock();
    out.write_all(text)
        .and_then(|()| out.flush())
        .map_err(|e| OutputError::new(&Destination::Stdout, e))
}

/// Writes the results of a run. Writes nothing (and creates no file) when there are none.
///
/// `src` is the script, for naming each result by the line its statement starts on.
/// `max_rows` limits each table (0: no limit). Single-result formats get exactly one result; the
/// caller checks that first.
pub fn write_results(
    destination: &Destination,
    format: RunFormat,
    results: &[QueryResult],
    src: &SourceFile,
    max_rows: usize,
) -> Result<(), OutputError> {
    if results.is_empty() {
        return Ok(());
    }
    let mut out = destination.open()?;
    let line = |result: &QueryResult| result.statement.map(|s| src.line_col(s.start).line);
    let written = match format {
        RunFormat::Table => write_tables(&mut out, results, line, max_rows),
        RunFormat::Json => {
            let array: Vec<Value> = results
                .iter()
                .map(|result| {
                    json!({
                        "statement": line(result),
                        "columns": columns(&result.schema),
                        "rows": json_rows(&result.schema, &result.batches),
                    })
                })
                .collect();
            serde_json::to_writer(&mut out, &array)
                .map_err(io::Error::from)
                .and_then(|()| out.write_all(b"\n"))
        }
        RunFormat::Csv => write_csv(&mut out, &results[0].schema, &results[0].batches),
        RunFormat::Ndjson => write_ndjson(&mut out, &results[0].schema, &results[0].batches),
        RunFormat::Arrow => write_arrow(&mut out, &results[0].schema, &results[0].batches),
    };
    written
        .and_then(|()| out.flush())
        .map_err(|e| OutputError::new(destination, e))
}

/// One table per result. With several, each gets a header naming its statement.
fn write_tables(
    out: &mut dyn Write,
    results: &[QueryResult],
    line: impl Fn(&QueryResult) -> Option<u32>,
    max_rows: usize,
) -> io::Result<()> {
    for (i, result) in results.iter().enumerate() {
        if results.len() > 1 {
            if i > 0 {
                writeln!(out)?;
            }
            match line(result) {
                Some(line) => writeln!(out, "== query {} (line {line}) ==", i + 1)?,
                None => writeln!(out, "== query {} ==", i + 1)?,
            }
        }
        let total: usize = result.batches.iter().map(RecordBatch::num_rows).sum();
        let limit = if max_rows == 0 { total } else { max_rows };
        if total <= limit {
            out.write_all(format_table(&result.schema, &result.batches).as_bytes())?;
        } else {
            let shown = first_rows(&result.batches, limit);
            out.write_all(format_table(&result.schema, &shown).as_bytes())?;
            let more = total - limit;
            writeln!(
                out,
                "… {} (show them with --max-rows {total})",
                count(more, "more row", "more rows")
            )?;
        }
    }
    Ok(())
}

/// The first `limit` rows of `batches`.
fn first_rows(batches: &[RecordBatch], limit: usize) -> Vec<RecordBatch> {
    let mut shown = Vec::new();
    let mut room = limit;
    for batch in batches {
        if room == 0 {
            break;
        }
        let take = batch.num_rows().min(room);
        shown.push(batch.slice(0, take));
        room -= take;
    }
    shown
}

/// `[{"name": "customerId", "type": "string"}, …]`, with the type as the table prints it.
fn columns(schema: &Schema) -> Value {
    schema
        .fields()
        .iter()
        .map(|field| {
            let ty = match datascript_type(field) {
                Some(ty) => ty.to_string(),
                None => format!("arrow:{}", field.data_type()),
            };
            json!({ "name": field.name(), "type": ty })
        })
        .collect()
}

/// An Arrow IPC file (the format `.arrow` and Feather v2 files use).
fn write_arrow(out: &mut dyn Write, schema: &Schema, batches: &[RecordBatch]) -> io::Result<()> {
    let mut writer = arrow_ipc::writer::FileWriter::try_new(out, schema).map_err(arrow_io)?;
    for batch in batches {
        writer.write(batch).map_err(arrow_io)?;
    }
    writer.finish().map_err(arrow_io)
}

fn arrow_io(error: ArrowError) -> io::Error {
    match error {
        ArrowError::IoError(_, error) => error,
        other => io::Error::other(other),
    }
}

/// "1 more row", "3 more rows".
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::Int64Array;
    use arrow_schema::{DataType, Field};

    use super::*;

    fn batch(values: &[i64]) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values.to_vec()))]).unwrap()
    }

    #[test]
    fn first_rows_cuts_across_batches() {
        let batches = [batch(&[1, 2]), batch(&[3, 4]), batch(&[5])];
        let shown = first_rows(&batches, 3);
        let rows: Vec<usize> = shown.iter().map(RecordBatch::num_rows).collect();
        assert_eq!(rows, [2, 1]);
        assert!(first_rows(&batches, 0).is_empty());
    }

    #[test]
    fn counts_are_singular_for_one() {
        assert_eq!(count(1, "more row", "more rows"), "1 more row");
        assert_eq!(count(0, "more row", "more rows"), "0 more rows");
    }
}
