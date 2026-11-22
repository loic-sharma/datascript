//! Printing results as `ds run` and the REPL print them: a table, CSV, JSON, NDJSON or Arrow IPC.
//!
//! The CLI, the REPL and the language server's `datascript/run` (M3-05, D-054) all print through
//! [`stream_results`], so the editor shows exactly what `ds run` prints.
//!
//! Results stream (M2-11, D-055): each query's batches are written as the engine produces them,
//! so output starts at once and memory stays flat. A table needs its rows before it can size its
//! columns, so it buffers up to `max_rows` rows, prints them, and then stops reading, which
//! cancels the rest of the query.

use std::io::{self, Write};

use arrow_array::RecordBatch;
use arrow_schema::{ArrowError, Schema};
use datascript_compiler::ir::{ChartKind, QueryPlan, RenderSpec};
use datascript_diagnostics::codes::DS5001;
use datascript_diagnostics::{Diagnostic, Span};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::table::{datascript_type, format_table};
use crate::writers::{json_rows, write_csv_header, write_csv_rows, write_ndjson};
use crate::{BatchStream, QueryStream, RunStream};

/// After a table's last shown row, how many more rows are read just to count them. Past this the
/// footer says "at least" instead of reading the whole result.
const COUNT_BUDGET: usize = 10_000;

/// How results are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// ASCII tables, one per query.
    Table,
    Csv,
    /// One JSON array with an element per query.
    Json,
    Ndjson,
    /// An Arrow IPC file.
    Arrow,
}

impl Format {
    /// Whether the format holds exactly one result.
    pub fn single_result(self) -> bool {
        matches!(self, Format::Csv | Format::Ndjson | Format::Arrow)
    }
}

/// Opens the writer of a [`Sink`].
type Opener = Box<dyn FnMut() -> io::Result<Box<dyn Write + Send>> + Send>;

/// Where results go, opened when the first result has something to write: a run that fails
/// before then (or has no query) writes nothing, so `ds run --output` creates no file.
pub struct Sink {
    open: Option<Opener>,
    out: Option<Box<dyn Write + Send>>,
}

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sink")
            .field("open", &self.out.is_some())
            .finish_non_exhaustive()
    }
}

impl Sink {
    /// A sink that calls `open` for its writer when it first writes.
    pub fn lazy(open: impl FnMut() -> io::Result<Box<dyn Write + Send>> + Send + 'static) -> Self {
        Sink {
            open: Some(Box::new(open)),
            out: None,
        }
    }

    /// A sink over a writer that is already open (the REPL's standard output, a buffer).
    pub fn writer(out: Box<dyn Write + Send>) -> Self {
        Sink {
            open: None,
            out: Some(out),
        }
    }

    fn get(&mut self) -> io::Result<&mut (dyn Write + Send)> {
        if self.out.is_none() {
            let open = self
                .open
                .as_mut()
                .expect("a sink has a writer or a way to open one");
            self.out = Some(open()?);
        }
        Ok(self.out.as_deref_mut().expect("opened above"))
    }

    /// Flushes what was written, if anything was.
    pub fn finish(&mut self) -> io::Result<()> {
        match &mut self.out {
            Some(out) => out.flush(),
            None => Ok(()),
        }
    }
}

/// Which frontend prints: they differ in how a cut table says to show the rest, and in `csv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frontend {
    /// `ds run`: the `--max-rows` flag. Single-result formats print the first query only; the
    /// caller checks there is one.
    Run,
    /// The REPL: the `.max-rows` command. `csv` prints every result, each under its
    /// `== query <n> (line <l>) ==` header when there are several: an interactive user can read
    /// them apart (M1-12).
    Repl,
}

/// How to print results.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub format: Format,
    /// Rows per table; 0 shows every row.
    pub max_rows: usize,
    pub frontend: Frontend,
}

/// What [`stream_results`] did.
#[derive(Debug, Default)]
pub struct Streamed {
    /// The statements of the queries that started, in order.
    pub statements: Vec<Span>,
    /// The runtime error that stopped the program, if one did. What was written before it
    /// stays written.
    pub error: Option<Diagnostic>,
}

/// Runs the queries of `run` one by one, writing each result to `sink` as its batches arrive.
/// `line_of` gives the line a statement starts on, for headers and JSON.
///
/// The caller reports `run.diagnostics` first (with [`chart_not_drawn`]). A write error stops the
/// run (dropping a query cancels it); the caller says where the write went.
pub async fn stream_results(
    sink: &mut Sink,
    layout: Layout,
    run: &mut RunStream,
    line_of: &(dyn Fn(Span) -> Option<u32> + Sync),
) -> io::Result<Streamed> {
    let mut streamed = Streamed::default();
    stream_inner(sink, layout, run, line_of, &mut streamed).await?;
    Ok(streamed)
}

async fn stream_inner(
    sink: &mut Sink,
    layout: Layout,
    run: &mut RunStream,
    line_of: &(dyn Fn(Span) -> Option<u32> + Sync),
    streamed: &mut Streamed,
) -> io::Result<()> {
    let count = run.queries().len();
    while let Some(started) = run.next_query().await {
        let mut query = match started {
            Ok(query) => query,
            Err(diagnostic) => {
                streamed.error = Some(diagnostic);
                break;
            }
        };
        let index = streamed.statements.len();
        streamed.statements.push(query.statement);
        let place = Place {
            index,
            count,
            line: line_of(query.statement),
        };
        let error = match layout.format {
            Format::Table => write_table(sink, &mut query, place, layout).await?,
            Format::Json => write_json(sink, &mut query, place).await?,
            Format::Csv | Format::Ndjson | Format::Arrow => {
                write_flat(sink, &mut query, place, layout).await?
            }
        };
        if let Some(error) = error {
            streamed.error = Some(error);
            break;
        }
        if layout.frontend == Frontend::Run && layout.format.single_result() {
            break;
        }
    }
    if layout.format == Format::Json && !streamed.statements.is_empty() && sink.out.is_some() {
        let out = sink.get()?;
        if streamed.error.is_none() {
            out.write_all(b"]\n")?;
        }
        out.flush()?;
    }
    Ok(())
}

/// Where a result sits among the run's results.
#[derive(Debug, Clone, Copy)]
struct Place {
    index: usize,
    count: usize,
    line: Option<u32>,
}

/// `== query <n> (line <l>) ==`, after a blank line unless it is the first.
fn write_header(out: &mut dyn Write, place: Place) -> io::Result<()> {
    if place.index > 0 {
        writeln!(out)?;
    }
    match place.line {
        Some(line) => writeln!(out, "== query {} (line {line}) ==", place.index + 1),
        None => writeln!(out, "== query {} ==", place.index + 1),
    }
}

/// What a table holds back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rest {
    /// This many more rows, counted to the end.
    Counted(usize),
    /// At least this many more rows: counting stopped at [`COUNT_BUDGET`].
    AtLeast(usize),
}

/// Reads up to `max_rows` rows (0: every row). Returns them, and the batch that went past the
/// limit with how many of its rows are shown, so the caller can count the rest. `Err` is the
/// query's error.
async fn read_rows(
    batches: &mut BatchStream,
    max_rows: usize,
) -> Result<(Vec<RecordBatch>, Option<usize>), Diagnostic> {
    let mut shown = Vec::new();
    let mut rows = 0;
    while let Some(batch) = batches.next().await {
        let batch = batch?;
        if max_rows > 0 && rows + batch.num_rows() > max_rows {
            let room = max_rows - rows;
            if room > 0 {
                shown.push(batch.slice(0, room));
            }
            return Ok((shown, Some(batch.num_rows() - room)));
        }
        rows += batch.num_rows();
        shown.push(batch);
    }
    Ok((shown, None))
}

/// Counts the rows left in `batches`, starting from `extra` already read, while that is cheap:
/// once [`COUNT_BUDGET`] rows are counted it stops, and the caller drops (cancels) the stream.
async fn count_rest(batches: &mut BatchStream, mut extra: usize) -> Result<Rest, Diagnostic> {
    while extra < COUNT_BUDGET {
        match batches.next().await {
            Some(batch) => extra += batch?.num_rows(),
            None => return Ok(Rest::Counted(extra)),
        }
    }
    Ok(Rest::AtLeast(extra))
}

/// One table. With several results, each gets a header naming its statement.
async fn write_table(
    sink: &mut Sink,
    query: &mut QueryStream,
    place: Place,
    layout: Layout,
) -> io::Result<Option<Diagnostic>> {
    let (shown, over) = match read_rows(&mut query.batches, layout.max_rows).await {
        Ok(read) => read,
        Err(error) => return Ok(Some(error)),
    };
    let out = sink.get()?;
    if place.count > 1 {
        write_header(out, place)?;
    }
    out.write_all(format_table(&query.schema, &shown).as_bytes())?;
    out.flush()?;
    let Some(extra) = over else {
        return Ok(None);
    };
    // The table is out; counting what it left out may take a moment.
    let rest = match count_rest(&mut query.batches, extra).await {
        Ok(rest) => rest,
        Err(error) => return Ok(Some(error)),
    };
    let how = match layout.frontend {
        Frontend::Run => "--max-rows",
        Frontend::Repl => ".max-rows",
    };
    let out = sink.get()?;
    match rest {
        Rest::Counted(more) => writeln!(
            out,
            "… {} (show them with {how} {})",
            count(more, "more row", "more rows"),
            layout.max_rows + more
        )?,
        Rest::AtLeast(more) => writeln!(
            out,
            "… at least {more} more rows (show them all with {how} 0)"
        )?,
    }
    out.flush()?;
    Ok(None)
}

/// Reads the first item of a query: `Err` is its error, before anything is written.
async fn first_batch(query: &mut QueryStream) -> Result<Option<RecordBatch>, Diagnostic> {
    query.batches.next().await.transpose()
}

/// CSV, NDJSON or Arrow, batch by batch.
async fn write_flat(
    sink: &mut Sink,
    query: &mut QueryStream,
    place: Place,
    layout: Layout,
) -> io::Result<Option<Diagnostic>> {
    let mut next = match first_batch(query).await {
        Ok(first) => first,
        Err(error) => return Ok(Some(error)),
    };
    let out = sink.get()?;
    let schema = query.schema.clone();
    if layout.format == Format::Arrow {
        // An Arrow IPC file (the format `.arrow` and Feather v2 files use).
        let mut writer =
            arrow_ipc::writer::FileWriter::try_new(&mut *out, &schema).map_err(arrow_io)?;
        while let Some(batch) = &next {
            writer.write(batch).map_err(arrow_io)?;
            writer.flush().map_err(arrow_io)?;
            next = match query.batches.next().await.transpose() {
                Ok(batch) => batch,
                Err(error) => return Ok(Some(error)),
            };
        }
        writer.finish().map_err(arrow_io)?;
        return Ok(None);
    }
    if layout.format == Format::Csv {
        if layout.frontend == Frontend::Repl && place.count > 1 {
            write_header(out, place)?;
        }
        write_csv_header(out, &schema)?;
    }
    while let Some(batch) = &next {
        let batch = std::slice::from_ref(batch);
        if layout.format == Format::Csv {
            write_csv_rows(out, &schema, batch)?;
        } else {
            write_ndjson(out, &schema, batch)?;
        }
        out.flush()?;
        next = match query.batches.next().await.transpose() {
            Ok(batch) => batch,
            Err(error) => return Ok(Some(error)),
        };
    }
    out.flush()?;
    Ok(None)
}

/// One element of the JSON array: `{"statement": <line>, "columns": […], "rows": […]}`, written
/// row by row, byte for byte as `serde_json` writes the whole value.
async fn write_json(
    sink: &mut Sink,
    query: &mut QueryStream,
    place: Place,
) -> io::Result<Option<Diagnostic>> {
    let mut next = match first_batch(query).await {
        Ok(first) => first,
        Err(error) => return Ok(Some(error)),
    };
    let out = sink.get()?;
    out.write_all(if place.index == 0 { b"[" } else { b"," })?;
    // `preserve_order` keeps the keys in this order.
    let head = json!({ "statement": place.line, "columns": columns(&query.schema) });
    let head = serde_json::to_string(&head).map_err(io::Error::from)?;
    out.write_all(&head.as_bytes()[..head.len() - 1])?;
    out.write_all(b",\"rows\":[")?;
    let mut first_row = true;
    while let Some(batch) = &next {
        for row in json_rows(&query.schema, std::slice::from_ref(batch)) {
            if !first_row {
                out.write_all(b",")?;
            }
            first_row = false;
            serde_json::to_writer(&mut *out, &row).map_err(io::Error::from)?;
        }
        out.flush()?;
        next = match query.batches.next().await.transpose() {
            Ok(batch) => batch,
            Err(error) => return Ok(Some(error)),
        };
    }
    out.write_all(b"]}")?;
    Ok(None)
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

/// The `render` specs of `queries`, with their spans, for [`chart_not_drawn`].
pub fn charts_of(queries: &[QueryPlan]) -> impl Iterator<Item = (&RenderSpec, Option<Span>)> {
    queries
        .iter()
        .filter_map(|q| q.render.as_ref().map(|render| (render, q.render_span)))
}

/// `DS5001`, once per program run: text frontends print charts as tables (LANGUAGE.md §6.3). It
/// points at the first `render` that is not `render table`, with a note counting the others. It
/// is given before anything runs, from the plans.
pub fn chart_not_drawn<'a>(
    renders: impl IntoIterator<Item = (&'a RenderSpec, Option<Span>)>,
) -> Option<Diagnostic> {
    let charts: Vec<(&RenderSpec, Option<Span>)> = renders
        .into_iter()
        .filter(|(spec, _)| spec.kind != ChartKind::Table)
        .collect();
    let &(first, render_span) = charts.first()?;
    let kind = first.kind;
    let mut d = Diagnostic::info(DS5001, "charts are not drawn on the command line");
    d = match render_span {
        Some(span) => d.primary(span, format!("this `{kind}` is printed as a table")),
        None => d.note(format!("the `{kind}` is printed as a table")),
    };
    if charts.len() > 1 {
        let others = charts.len() - 1;
        d = d.note(format!(
            "{} not drawn either",
            count(others, "other chart is", "other charts are")
        ));
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::Int64Array;
    use arrow_schema::{DataType, Field};
    use futures::executor::block_on;

    use super::*;

    fn batch(values: &[i64]) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values.to_vec()))]).unwrap()
    }

    fn stream(batches: Vec<RecordBatch>) -> BatchStream {
        Box::pin(futures::stream::iter(batches.into_iter().map(Ok)))
    }

    fn row_counts(batches: &[RecordBatch]) -> Vec<usize> {
        batches.iter().map(RecordBatch::num_rows).collect()
    }

    #[test]
    fn read_rows_cuts_across_batches() {
        let mut batches = stream(vec![batch(&[1, 2]), batch(&[3, 4]), batch(&[5])]);
        let (shown, over) = block_on(read_rows(&mut batches, 3)).unwrap();
        assert_eq!(row_counts(&shown), [2, 1]);
        // One row of the second batch is left over, and the third batch is not read.
        assert_eq!(over, Some(1));
        assert_eq!(block_on(count_rest(&mut batches, 1)), Ok(Rest::Counted(2)));
    }

    #[test]
    fn read_rows_with_room_to_spare_or_no_limit_reads_everything() {
        let (shown, over) =
            block_on(read_rows(&mut stream(vec![batch(&[1, 2]), batch(&[3])]), 3)).unwrap();
        assert_eq!((row_counts(&shown), over), (vec![2, 1], None));
        let (shown, over) =
            block_on(read_rows(&mut stream(vec![batch(&[1, 2]), batch(&[3])]), 0)).unwrap();
        assert_eq!((row_counts(&shown), over), (vec![2, 1], None));
    }

    #[test]
    fn counting_stops_at_the_budget() {
        let big = batch(&vec![0; 6_000]);
        let mut batches = stream(vec![big.clone(), big.clone(), big]);
        assert_eq!(
            block_on(count_rest(&mut batches, 10)),
            Ok(Rest::AtLeast(12_010))
        );
        // The third batch was never read.
        assert_eq!(block_on(batches.next()).map(|b| b.is_ok()), Some(true));
    }

    #[test]
    fn counts_are_singular_for_one() {
        assert_eq!(count(1, "more row", "more rows"), "1 more row");
        assert_eq!(count(0, "more row", "more rows"), "0 more rows");
    }
}
