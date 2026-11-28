//! Output parity (M2-29): `printer::stream_results` prints the same values in every `Format`.
//!
//! One edge-type dataset (odd reals, far-away datetimes, negative timespans, strings with wide,
//! control and CSV-special characters in data and headers, valid and invalid `dynamic` JSON, and
//! an Arrow-only `Binary` column) runs through a [`Session`] over a small in-memory engine, then
//! through the printer in each format. Every format is parsed back and compared, cell by cell,
//! with one table of expected value texts.

use std::collections::HashMap;
use std::io::{self, Cursor, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use arrow_array::{
    ArrayRef, BinaryArray, DurationMicrosecondArray, Float64Array, Int64Array, RecordBatch,
    StringArray, TimestampMicrosecondArray,
};
use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use datascript_compiler::ir::QueryPlan;
use datascript_compiler::{Capabilities, SourceError, SourceRef};
use datascript_diagnostics::codes::DS4002;
use datascript_diagnostics::{Diagnostic, FileId, SourceFile};
use datascript_engine::printer::{Format, Frontend, Layout, Sink, Streamed, stream_results};
use datascript_engine::table::format_table;
use datascript_engine::writers::{write_csv, write_csv_header, write_csv_rows};
use datascript_engine::{Engine, ExecContext, FixedClock, ResultStream, Session};
use futures::executor::block_on;
use serde_json::Value;

// ---- an engine over fixed batches ----

/// Answers `data.csv` with `schema`, and every query with `batches`, followed by an error item
/// when `fail_at_end` is set.
struct FixedEngine {
    caps: Capabilities,
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
    fail_at_end: bool,
}

impl FixedEngine {
    fn new(schema: SchemaRef, batches: Vec<RecordBatch>) -> Self {
        FixedEngine {
            caps: Capabilities::full(),
            schema,
            batches,
            fail_at_end: false,
        }
    }
}

#[async_trait::async_trait]
impl Engine for FixedEngine {
    fn name(&self) -> &str {
        "fixed"
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    async fn schema_of(
        &self,
        source: &SourceRef,
        _ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        match source {
            SourceRef::Path(p) if p == "data.csv" => Ok(self.schema.clone()),
            _ => Err(SourceError::NotFound { nearby: Vec::new() }),
        }
    }

    async fn execute(
        &self,
        plan: &QueryPlan,
        _ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        let schema = plan.output_schema.clone();
        let mut items: Vec<Result<RecordBatch, Diagnostic>> = self
            .batches
            .iter()
            .map(|b| Ok(RecordBatch::try_new(schema.clone(), b.columns().to_vec()).unwrap()))
            .collect();
        if self.fail_at_end {
            items.push(Err(Diagnostic::error(
                DS4002,
                "the query failed while running",
            )));
        }
        Ok(ResultStream {
            schema,
            batches: Box::pin(futures::stream::iter(items)),
        })
    }
}

// ---- printing ----

/// A writer whose bytes the test reads afterwards.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Runs `from 'data.csv'` over `engine` and prints it in `format`.
fn print(engine: FixedEngine, format: Format, max_rows: usize) -> (Vec<u8>, Streamed) {
    let session = Session::new(Arc::new(engine), Arc::new(FixedClock(0)));
    let ctx = session.exec_context(Path::new("."));
    let mut run = block_on(session.start(
        &SourceFile::new("query", "from 'data.csv'"),
        FileId(0),
        &ctx,
    ));
    assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
    let out = Shared::default();
    let mut sink = Sink::writer(Box::new(out.clone()));
    let layout = Layout {
        format,
        max_rows,
        frontend: Frontend::Run,
    };
    let streamed = block_on(stream_results(&mut sink, layout, &mut run, &|_| Some(1))).unwrap();
    sink.finish().unwrap();
    let bytes = out.0.lock().unwrap().clone();
    (bytes, streamed)
}

fn print_ok(engine: FixedEngine, format: Format) -> Vec<u8> {
    let (bytes, streamed) = print(engine, format, 0);
    assert!(streamed.error.is_none(), "{:?}", streamed.error);
    bytes
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}

// ---- the dataset ----

const UTC: &str = "UTC";

fn nullable(name: &str, ty: DataType) -> Field {
    Field::new(name, ty, true)
}

fn edge_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        nullable("id", DataType::Int64),
        nullable("x", DataType::Float64),
        nullable(
            "when",
            DataType::Timestamp(TimeUnit::Microsecond, Some(UTC.into())),
        ),
        nullable("span", DataType::Duration(TimeUnit::Microsecond)),
        // Not last: an empty value in the last column would be trimmed away with the line.
        nullable("日本 bytes", DataType::Binary),
        nullable("text, \"quoted\"", DataType::Utf8),
        nullable("j", DataType::Utf8).with_metadata(HashMap::from([(
            EXTENSION_TYPE_NAME_KEY.to_string(),
            "arrow.json".to_string(),
        )])),
    ]))
}

/// 0000-01-01T00:00:00Z in microseconds since the Unix epoch.
const YEAR_ZERO: i64 = -62_167_219_200_000_000;
/// 10000-01-01T00:00:00Z.
const YEAR_10000: i64 = 253_402_300_800_000_000;
const DAY: i64 = 86_400_000_000;

/// The dataset, in two batches, and the text every format should give each cell (`None`: null).
#[allow(clippy::type_complexity)]
fn edge_data() -> (SchemaRef, Vec<RecordBatch>, Vec<Vec<Option<&'static str>>>) {
    let schema = edge_schema();
    let rows: Vec<(
        i64,
        Option<f64>,
        Option<i64>,
        Option<i64>,
        Option<&str>,
        Option<&str>,
        Option<&[u8]>,
    )> = vec![
        (
            1,
            Some(1.5),
            Some(-1),
            Some(-1_500_000),
            Some("日本語"),
            Some("{\"a\":[1,2]}"),
            Some(&[0x01, 0xff]),
        ),
        (
            2,
            Some(f64::NAN),
            Some(YEAR_ZERO - DAY),
            Some(-(DAY + 3_723_000_001)),
            Some("tab\there"),
            Some("not json"),
            Some(&[]),
        ),
        (
            3,
            Some(f64::INFINITY),
            Some(YEAR_10000),
            Some(0),
            Some("a,b \"c\""),
            Some("\"a string\""),
            None,
        ),
        (
            4,
            Some(f64::NEG_INFINITY),
            Some(-86_400_000_000 * 365),
            None,
            Some("line\nbreak\r\n"),
            Some("[1, {\"b\": null}]"),
            Some(b"x"),
        ),
        (
            5,
            Some(-0.0),
            Some(0),
            Some(1),
            Some("esc\u{1b}[0m 🎉"),
            Some("{bad"),
            Some(&[0]),
        ),
        (6, Some(1e20), None, Some(-1), None, None, None),
        (7, Some(0.25), Some(1), None, Some("é"), Some("12"), None),
        (8, None, None, None, Some("plain"), Some("null"), None),
    ];
    let mut expected = vec![
        vec![
            Some("1"),
            Some("1.5"),
            Some("1969-12-31T23:59:59.999999Z"),
            Some("-00:00:01.5"),
            Some("日本語"),
            Some("{\"a\":[1,2]}"),
            Some("01ff"),
        ],
        vec![
            Some("2"),
            Some("NaN"),
            Some("-0001-12-31T00:00:00Z"),
            Some("-1.01:02:03.000001"),
            Some("tab\there"),
            Some("not json"),
            Some(""),
        ],
        vec![
            Some("3"),
            Some("inf"),
            Some("+10000-01-01T00:00:00Z"),
            Some("00:00:00"),
            Some("a,b \"c\""),
            Some("\"a string\""),
            None,
        ],
        vec![
            Some("4"),
            Some("-inf"),
            Some("1969-01-01T00:00:00Z"),
            None,
            Some("line\nbreak\r\n"),
            Some("[1, {\"b\": null}]"),
            Some("78"),
        ],
        vec![
            Some("5"),
            Some("-0.0"),
            Some("1970-01-01T00:00:00Z"),
            Some("00:00:00.000001"),
            Some("esc\u{1b}[0m 🎉"),
            Some("{bad"),
            Some("00"),
        ],
        vec![
            Some("6"),
            Some("1e20"),
            None,
            Some("-00:00:00.000001"),
            None,
            None,
            None,
        ],
        vec![
            Some("7"),
            Some("0.25"),
            Some("1970-01-01T00:00:00.000001Z"),
            None,
            Some("é"),
            Some("12"),
            None,
        ],
        vec![
            Some("8"),
            None,
            None,
            None,
            Some("plain"),
            Some("null"),
            None,
        ],
    ];
    let batch = |rows: &[(
        i64,
        Option<f64>,
        Option<i64>,
        Option<i64>,
        Option<&str>,
        Option<&str>,
        Option<&[u8]>,
    )]| {
        let columns: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.0))),
            Arc::new(Float64Array::from_iter(rows.iter().map(|r| r.1))),
            Arc::new(
                TimestampMicrosecondArray::from_iter(rows.iter().map(|r| r.2)).with_timezone(UTC),
            ),
            Arc::new(DurationMicrosecondArray::from_iter(
                rows.iter().map(|r| r.3),
            )),
            Arc::new(BinaryArray::from_iter(rows.iter().map(|r| r.6))),
            Arc::new(StringArray::from_iter(rows.iter().map(|r| r.4))),
            Arc::new(StringArray::from_iter(rows.iter().map(|r| r.5))),
        ];
        RecordBatch::try_new(schema.clone(), columns).unwrap()
    };
    // The rows above list the binary column last; the schema has it fifth.
    for row in &mut expected {
        let bytes = row.remove(6);
        row.insert(4, bytes);
    }
    // Two batches, so every format writes across a batch boundary.
    let batches = vec![batch(&rows[..3]), batch(&rows[3..])];
    (schema, batches, expected)
}

fn edge_engine() -> FixedEngine {
    let (schema, batches, _) = edge_data();
    FixedEngine::new(schema, batches)
}

fn names(schema: &Schema) -> Vec<String> {
    schema.fields().iter().map(|f| f.name().clone()).collect()
}

/// Splits CSV text into records of fields, per RFC 4180.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') => quoted = true,
            (false, ',') => record.push(std::mem::take(&mut field)),
            (false, '\n') => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            (false, c) => field.push(c),
        }
    }
    assert!(field.is_empty() && record.is_empty(), "CSV ends mid-record");
    records
}

/// A string as a table cell shows it: control characters escaped.
fn escaped(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Whether a JSON value is what the writers give for a cell whose text is `expected`, in a
/// column of type `ty` (as the table's type row names it).
fn json_matches(value: &Value, expected: Option<&str>, ty: &str) -> bool {
    // A `dynamic` value is the JSON it holds; text that is not JSON is a string.
    if let (Some(text), "dynamic") = (expected, ty)
        && let Ok(parsed) = serde_json::from_str::<Value>(text)
    {
        return &parsed == value;
    }
    match (value, expected) {
        (Value::Null, None) => true,
        (Value::Number(n), Some(text)) => n.as_f64() == text.parse::<f64>().ok(),
        (Value::String(s), Some(text)) => s == text,
        _ => false,
    }
}

// ---- edge types in every format ----

#[test]
fn every_format_prints_the_same_values() {
    let (schema, batches, expected) = edge_data();
    let columns = names(&schema);

    // Table: names, then types, then one row per row with control characters escaped.
    let table = text(print_ok(edge_engine(), Format::Table));
    let lines: Vec<&str> = table.lines().collect();
    let cells =
        |line: &str| -> Vec<String> { line.split(" | ").map(|c| c.trim().to_string()).collect() };
    assert_eq!(cells(lines[0]), columns);
    let types = cells(lines[1]);
    assert_eq!(
        types,
        [
            "long",
            "real",
            "datetime",
            "timespan",
            "arrow:Binary",
            "string",
            "dynamic"
        ]
    );
    let table_rows: Vec<Vec<String>> = lines[3..].iter().map(|l| cells(l)).collect();
    let want: Vec<Vec<String>> = expected
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.map_or("null".to_string(), escaped))
                .collect()
        })
        .collect();
    assert_eq!(table_rows, want, "{table}");
    // Wide characters still line up: every ` | ` sits over a `+` of the separator, counting
    // terminal columns.
    assert_eq!(lines.len(), 3 + expected.len());
    let pluses: Vec<usize> = lines[2].match_indices('+').map(|(i, _)| i).collect();
    for line in lines.iter().filter(|l| !l.starts_with('-')) {
        let bars: Vec<usize> = line
            .match_indices(" | ")
            .map(|(i, _)| datascript_diagnostics::text_width(&line[..i]) + 1)
            .collect();
        assert_eq!(bars, pluses, "{line}");
    }

    // CSV: the header, then every value unescaped (null is empty).
    let csv = parse_csv(&text(print_ok(edge_engine(), Format::Csv)));
    assert_eq!(csv[0], columns);
    let want: Vec<Vec<String>> = expected
        .iter()
        .map(|row| row.iter().map(|c| c.unwrap_or("").to_string()).collect())
        .collect();
    assert_eq!(csv[1..], want);

    // NDJSON: one object per row, keyed by the column names in order.
    let ndjson = text(print_ok(edge_engine(), Format::Ndjson));
    let objects: Vec<serde_json::Map<String, Value>> = ndjson
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(objects.len(), expected.len());
    for (object, row) in objects.iter().zip(&expected) {
        assert_eq!(object.keys().cloned().collect::<Vec<_>>(), columns);
        for ((value, cell), ty) in object.values().zip(row).zip(&types) {
            assert!(json_matches(value, *cell, ty), "{value} vs {cell:?} ({ty})");
        }
    }

    // JSON: the same rows inside the one result object, whose column types match the table's.
    let json: Value = serde_json::from_str(&text(print_ok(edge_engine(), Format::Json))).unwrap();
    let result = &json.as_array().unwrap()[0];
    assert_eq!(result["statement"], 1);
    let json_columns: Vec<(String, String)> = result["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_string(),
                c["type"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let table_columns: Vec<(String, String)> =
        columns.iter().cloned().zip(types.iter().cloned()).collect();
    assert_eq!(json_columns, table_columns);
    let json_rows: Vec<Value> = result["rows"].as_array().unwrap().clone();
    let ndjson_rows: Vec<Value> = objects.into_iter().map(Value::Object).collect();
    assert_eq!(json_rows, ndjson_rows);

    // Arrow: reading the file back gives the same schema (extension metadata included) and
    // values that print the same, `NaN` and `-0.0` included.
    let arrow = print_ok(edge_engine(), Format::Arrow);
    let reader = arrow_ipc::reader::FileReader::try_new(Cursor::new(arrow), None).unwrap();
    let read_schema = reader.schema();
    let read: Vec<RecordBatch> = reader.map(Result::unwrap).collect();
    assert_eq!(names(&read_schema), columns);
    for (read, field) in read_schema.fields().iter().zip(schema.fields()) {
        assert_eq!(read.data_type(), field.data_type());
        assert_eq!(read.metadata(), field.metadata());
    }
    assert_eq!(
        format_table(&read_schema, &read),
        format_table(&schema, &batches)
    );
    assert_eq!(format_table(&read_schema, &read), table);
}

// ---- empty results ----

#[test]
fn a_result_with_no_rows_prints_its_columns_in_every_format() {
    let empty = || FixedEngine::new(edge_schema(), Vec::new());
    let columns = names(&edge_schema());

    let table = text(print_ok(empty(), Format::Table));
    assert_eq!(table.lines().count(), 3, "{table}");
    assert!(table.starts_with("id   | x    | when "), "{table}");

    let csv = parse_csv(&text(print_ok(empty(), Format::Csv)));
    assert_eq!(csv, std::slice::from_ref(&columns));

    assert_eq!(text(print_ok(empty(), Format::Ndjson)), "");

    let json: Value = serde_json::from_str(&text(print_ok(empty(), Format::Json))).unwrap();
    assert_eq!(json[0]["rows"], Value::Array(Vec::new()));
    assert_eq!(json[0]["columns"].as_array().unwrap().len(), columns.len());

    // An Arrow file with no batches still has the schema.
    let arrow = print_ok(empty(), Format::Arrow);
    let reader = arrow_ipc::reader::FileReader::try_new(Cursor::new(arrow), None).unwrap();
    assert_eq!(names(&reader.schema()), columns);
    assert_eq!(reader.num_batches(), 0);

    // A batch with no rows prints the same as no batch at all.
    let (schema, ..) = edge_data();
    let zero = edge_data().1[0].slice(0, 0);
    for format in [Format::Table, Format::Csv, Format::Ndjson, Format::Json] {
        assert_eq!(
            print_ok(FixedEngine::new(schema.clone(), vec![zero.clone()]), format),
            print_ok(empty(), format),
            "{format:?}"
        );
    }
}

#[test]
fn a_result_with_no_columns_prints_in_every_format() {
    let none = || FixedEngine::new(Arc::new(Schema::empty()), Vec::new());
    assert_eq!(text(print_ok(none(), Format::Table)), "(no columns)\n");
    assert_eq!(text(print_ok(none(), Format::Csv)), "\n");
    assert_eq!(text(print_ok(none(), Format::Ndjson)), "");
    assert_eq!(
        text(print_ok(none(), Format::Json)),
        "[{\"statement\":1,\"columns\":[],\"rows\":[]}]\n"
    );
    let arrow = print_ok(none(), Format::Arrow);
    let reader = arrow_ipc::reader::FileReader::try_new(Cursor::new(arrow), None).unwrap();
    assert!(reader.schema().fields().is_empty());
    assert_eq!(reader.num_batches(), 0);
}

// ---- CSV in pieces ----

#[test]
fn csv_header_then_rows_batch_by_batch_is_write_csv() {
    let (schema, batches, _) = edge_data();
    let mut whole = Vec::new();
    write_csv(&mut whole, &schema, &batches).unwrap();
    let mut pieces = Vec::new();
    write_csv_header(&mut pieces, &schema).unwrap();
    for batch in &batches {
        write_csv_rows(&mut pieces, &schema, std::slice::from_ref(batch)).unwrap();
    }
    assert_eq!(pieces, whole);
    // And the printer writes the same bytes.
    assert_eq!(print_ok(edge_engine(), Format::Csv), whole);
}

// ---- table footers ----

/// `n` rows of one `long` column, in batches of two.
fn counting(n: i64) -> FixedEngine {
    let schema = Arc::new(Schema::new(vec![nullable("n", DataType::Int64)]));
    let batches = (1..=n)
        .collect::<Vec<_>>()
        .chunks(2)
        .map(|chunk| {
            let column: ArrayRef = Arc::new(Int64Array::from(chunk.to_vec()));
            RecordBatch::try_new(schema.clone(), vec![column]).unwrap()
        })
        .collect();
    FixedEngine::new(schema, batches)
}

#[test]
fn a_table_one_row_over_says_1_more_row() {
    let (out, streamed) = print(counting(3), Format::Table, 2);
    assert!(streamed.error.is_none());
    assert_eq!(
        text(out),
        "n\nlong\n----\n   1\n   2\n… 1 more row (show them with --max-rows 3)\n"
    );
}

#[test]
fn a_table_of_exactly_max_rows_has_no_footer() {
    let (out, streamed) = print(counting(4), Format::Table, 4);
    assert!(streamed.error.is_none());
    assert_eq!(text(out), "n\nlong\n----\n   1\n   2\n   3\n   4\n");
}

#[test]
fn a_runtime_error_while_counting_comes_after_the_table() {
    let mut engine = counting(4);
    engine.fail_at_end = true;
    let (out, streamed) = print(engine, Format::Table, 2);
    // The table is printed; the error that stopped the count is returned, with no footer.
    assert_eq!(text(out), "n\nlong\n----\n   1\n   2\n");
    assert_eq!(streamed.error.map(|d| d.code), Some(DS4002));
    assert_eq!(streamed.statements.len(), 1);
}
