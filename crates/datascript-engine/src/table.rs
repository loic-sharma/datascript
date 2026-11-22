//! Deterministic ASCII tables for Arrow results.
//!
//! This is the format golden tests compare against (TESTING.md, "Expected output rules") and the
//! CLI prints. The output never depends on the locale, the time zone or the machine:
//!
//! ```text
//! customerId | revenue
//! string     | real
//! -----------+--------
//! c-042      | 1250.5
//! c-007      |  980.0
//! c-113      |  402.25
//! ```
//!
//! - Row 1 holds the column names, row 2 the DataScript type names, row 3 a separator.
//! - `real` values use the shortest representation that round-trips and are aligned on the
//!   decimal point. `int`/`long` are right-aligned. Everything else is left-aligned.
//! - `datetime` values print in ISO 8601 UTC (`2024-01-05T10:00:00Z`, with a fraction only when
//!   it is not zero), `timespan` values as `[-][d.]hh:mm:ss[.ffffff]`, nulls as `null`.
//! - Control characters inside strings are escaped (`\n`, `\t`, `\u{1b}`) so a row stays one line.
//! - Trailing spaces are trimmed from every line, and the table ends with a newline.

use std::fmt::Write as _;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    DurationMicrosecondType, Float64Type, Int32Type, Int64Type, TimestampMicrosecondType,
};
use arrow_array::{Array, RecordBatch};
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_schema::{Field, Schema};
use datascript_compiler::Type;

/// Formats a query result as a deterministic ASCII table.
///
/// `batches` must all have `schema`'s columns. An empty result still prints the header, type and
/// separator rows.
pub fn format_table(schema: &Schema, batches: &[RecordBatch]) -> String {
    let columns: Vec<Column> = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, field)| Column::new(field, batches, i))
        .collect();

    let mut out = String::new();
    if columns.is_empty() {
        out.push_str("(no columns)\n");
        return out;
    }
    let widths: Vec<usize> = columns.iter().map(Column::width).collect();
    let last = columns.len() - 1;

    let push_row = |out: &mut String, cells: &mut dyn Iterator<Item = String>| {
        let mut line = String::new();
        for (i, cell) in cells.enumerate() {
            if i > 0 {
                line.push_str(" | ");
            }
            line.push_str(&cell);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    };

    push_row(
        &mut out,
        &mut columns
            .iter()
            .zip(&widths)
            .map(|(c, &w)| pad_left_aligned(&c.name, w)),
    );
    push_row(
        &mut out,
        &mut columns
            .iter()
            .zip(&widths)
            .map(|(c, &w)| pad_left_aligned(&c.type_name, w)),
    );
    for (i, &w) in widths.iter().enumerate() {
        let edge = usize::from(i > 0) + usize::from(i < last);
        if i > 0 {
            out.push('+');
        }
        out.push_str(&"-".repeat(w + edge));
    }
    out.push('\n');

    let rows = columns.first().map_or(0, |c| c.cells.len());
    for row in 0..rows {
        push_row(
            &mut out,
            &mut columns
                .iter()
                .zip(&widths)
                .map(|(c, &w)| c.render_cell(row, w)),
        );
    }
    out
}

/// The DataScript type of a result field, or `None` when its Arrow type has no DataScript type.
/// Result fields use the canonical Arrow types (LANGUAGE.md §5), so this is
/// [`datascript_compiler::arrow::from_arrow`].
pub fn datascript_type(field: &Field) -> Option<Type> {
    datascript_compiler::arrow::from_arrow(field).ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Decimal,
}

struct Column {
    name: String,
    type_name: String,
    align: Align,
    cells: Vec<String>,
    /// For `Align::Decimal`: widest integer part and widest fraction part (including the `.`).
    int_width: usize,
    frac_width: usize,
}

impl Column {
    fn new(field: &Field, batches: &[RecordBatch], index: usize) -> Self {
        let ds_type = datascript_type(field);
        let align = match ds_type {
            Some(Type::Real) => Align::Decimal,
            Some(Type::Int | Type::Long) => Align::Right,
            _ => Align::Left,
        };
        let type_name = match ds_type {
            Some(ty) => ty.to_string(),
            // Readable but untyped columns (LANGUAGE.md §5.1) show their Arrow type.
            None => format!("arrow:{}", field.data_type()),
        };

        let mut cells = Vec::new();
        for batch in batches {
            let array = batch.column(index).as_ref();
            cells.extend(
                (0..array.len()).map(|i| match value_text(array, ds_type, i) {
                    Some(text) => escape_string(&text),
                    None => "null".to_string(),
                }),
            );
        }

        let (mut int_width, mut frac_width) = (0, 0);
        if align == Align::Decimal {
            for cell in &cells {
                let (int, frac) = split_decimal(cell);
                int_width = int_width.max(char_len(int));
                frac_width = frac_width.max(char_len(frac));
            }
        }

        Self {
            name: field.name().clone(),
            type_name,
            align,
            cells,
            int_width,
            frac_width,
        }
    }

    fn width(&self) -> usize {
        let values = match self.align {
            Align::Decimal => self.int_width + self.frac_width,
            _ => self.cells.iter().map(|c| char_len(c)).max().unwrap_or(0),
        };
        values
            .max(char_len(&self.name))
            .max(char_len(&self.type_name))
    }

    fn render_cell(&self, row: usize, width: usize) -> String {
        let cell = &self.cells[row];
        match self.align {
            Align::Left => pad_left_aligned(cell, width),
            Align::Right => pad_right_aligned(cell, width),
            Align::Decimal => {
                // Align on the decimal point, then right-align the block in the column.
                let (int, frac) = split_decimal(cell);
                let mut s = " ".repeat(self.int_width - char_len(int));
                s.push_str(int);
                s.push_str(frac);
                s.push_str(&" ".repeat(self.frac_width - char_len(frac)));
                pad_right_aligned(&s, width)
            }
        }
    }
}

/// Splits a formatted real at its decimal point. Values without one (`NaN`, `inf`, `1e20`) are
/// all integer part.
fn split_decimal(cell: &str) -> (&str, &str) {
    match cell.find('.') {
        Some(dot) => cell.split_at(dot),
        None => (cell, ""),
    }
}

/// Terminal columns taken by a cell, so wide (CJK) characters and combining marks line up.
fn char_len(s: &str) -> usize {
    datascript_diagnostics::text_width(s)
}

fn pad_left_aligned(s: &str, width: usize) -> String {
    let mut out = s.to_string();
    out.push_str(&" ".repeat(width.saturating_sub(char_len(s))));
    out
}

fn pad_right_aligned(s: &str, width: usize) -> String {
    let mut out = " ".repeat(width.saturating_sub(char_len(s)));
    out.push_str(s);
    out
}

/// The text of value `i` of `array`, whose DataScript type is `ty`, or `None` for null. Strings
/// are not escaped. The table, CSV and JSON writers all print values this way.
pub(crate) fn value_text(array: &dyn Array, ty: Option<Type>, i: usize) -> Option<String> {
    if array.is_null(i) {
        return None;
    }
    Some(match ty {
        Some(Type::Bool) => array.as_boolean().value(i).to_string(),
        Some(Type::Int) => array.as_primitive::<Int32Type>().value(i).to_string(),
        Some(Type::Long) => array.as_primitive::<Int64Type>().value(i).to_string(),
        Some(Type::Real) => format_real(array.as_primitive::<Float64Type>().value(i)),
        Some(Type::String | Type::Dynamic) => array.as_string::<i32>().value(i).to_string(),
        Some(Type::DateTime) => {
            format_datetime(array.as_primitive::<TimestampMicrosecondType>().value(i))
        }
        Some(Type::TimeSpan) => {
            format_timespan(array.as_primitive::<DurationMicrosecondType>().value(i))
        }
        _ => fallback(array, i),
    })
}

/// Formats values of types the table has no dedicated rule for, with Arrow's own display.
fn fallback(array: &dyn Array, i: usize) -> String {
    let options = FormatOptions::default().with_null("null");
    match ArrayFormatter::try_new(array, &options) {
        Ok(formatter) => formatter.value(i).to_string(),
        Err(_) => format!("<{}>", array.data_type()),
    }
}

/// Shortest round-trip representation, always with a decimal point for finite values in plain
/// notation (`980.0`, `0.1`, `1e20`).
pub fn format_real(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    // Rust's `Debug` for f64 is the shortest representation that round-trips.
    format!("{v:?}")
}

fn escape_string(s: &str) -> String {
    if !s.chars().any(char::is_control) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

const MICROS_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// Formats microseconds since the Unix epoch as ISO 8601 UTC.
pub fn format_datetime(micros: i64) -> String {
    let secs = micros.div_euclid(MICROS_PER_SECOND);
    let frac = micros.rem_euclid(MICROS_PER_SECOND);
    let days = secs.div_euclid(SECONDS_PER_DAY);
    let sod = secs.rem_euclid(SECONDS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    let mut out = String::new();
    if (0..=9999).contains(&y) {
        let _ = write!(out, "{y:04}");
    } else {
        let _ = write!(out, "{y:+05}");
    }
    let _ = write!(
        out,
        "-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        sod / 3600,
        sod / 60 % 60,
        sod % 60
    );
    push_fraction(&mut out, frac);
    out.push('Z');
    out
}

/// Formats a duration in microseconds as `[-][d.]hh:mm:ss[.ffffff]`, the form `timespan(...)`
/// literals use (LANGUAGE.md §1).
pub fn format_timespan(micros: i64) -> String {
    let mut out = String::new();
    if micros < 0 {
        out.push('-');
    }
    let abs = micros.unsigned_abs();
    let micros_per_second = MICROS_PER_SECOND as u64;
    let secs = abs / micros_per_second;
    let frac = (abs % micros_per_second) as i64;
    let days = secs / SECONDS_PER_DAY as u64;
    let sod = secs % SECONDS_PER_DAY as u64;
    if days > 0 {
        let _ = write!(out, "{days}.");
    }
    let _ = write!(
        out,
        "{:02}:{:02}:{:02}",
        sod / 3600,
        sod / 60 % 60,
        sod % 60
    );
    push_fraction(&mut out, frac);
    out
}

/// Appends `.ffffff` without trailing zeros, or nothing for a whole second.
fn push_fraction(out: &mut String, micros: i64) {
    if micros != 0 {
        let digits = format!("{micros:06}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
    use arrow_schema::{DataType, TimeUnit};

    use arrow_array::{
        ArrayRef, BinaryArray, BooleanArray, DurationMicrosecondArray, Float64Array, Int32Array,
        Int64Array, StringArray, TimestampMicrosecondArray,
    };

    use super::*;

    fn batch(fields: Vec<Field>, arrays: Vec<ArrayRef>) -> (Schema, RecordBatch) {
        let schema = Schema::new(fields);
        let batch = RecordBatch::try_new(Arc::new(schema.clone()), arrays).unwrap();
        (schema, batch)
    }

    fn ts_field(name: &str) -> Field {
        Field::new(
            name,
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            true,
        )
    }

    #[test]
    fn testing_md_example() {
        let (schema, b) = batch(
            vec![
                Field::new("customerId", DataType::Utf8, true),
                Field::new("revenue", DataType::Float64, true),
            ],
            vec![
                Arc::new(StringArray::from(vec!["c-042", "c-007", "c-113"])),
                Arc::new(Float64Array::from(vec![1250.5, 980.0, 402.25])),
            ],
        );
        assert_eq!(
            format_table(&schema, &[b]),
            "customerId | revenue\n\
             string     | real\n\
             -----------+--------\n\
             c-042      | 1250.5\n\
             c-007      |  980.0\n\
             c-113      |  402.25\n"
        );
    }

    #[test]
    fn all_types_nulls_and_alignment() {
        let (schema, b) = batch(
            vec![
                Field::new("b", DataType::Boolean, true),
                Field::new("i", DataType::Int32, true),
                Field::new("n", DataType::Int64, true),
                ts_field("t"),
                Field::new("d", DataType::Duration(TimeUnit::Microsecond), true),
                Field::new("j", DataType::Utf8, true).with_metadata(HashMap::from([(
                    EXTENSION_TYPE_NAME_KEY.to_string(),
                    "arrow.json".to_string(),
                )])),
                Field::new("raw", DataType::Binary, true),
            ],
            vec![
                Arc::new(BooleanArray::from(vec![Some(true), None])),
                Arc::new(Int32Array::from(vec![Some(7), None])),
                Arc::new(Int64Array::from(vec![Some(-12345), Some(3)])),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![Some(1_704_448_800_000_000), None])
                        .with_timezone("UTC"),
                ),
                Arc::new(DurationMicrosecondArray::from(vec![
                    Some(93_784_500_000),
                    Some(-1_000),
                ])),
                Arc::new(StringArray::from(vec![Some("{\"a\":1}"), None])),
                Arc::new(BinaryArray::from(vec![Some(&b"\x01\xff"[..]), None])),
            ],
        );
        assert_eq!(
            format_table(&schema, &[b]),
            "b    | i    | n      | t                    | d             | j       | raw\n\
             bool | int  | long   | datetime             | timespan      | dynamic | arrow:Binary\n\
             -----+------+--------+----------------------+---------------+---------+-------------\n\
             true |    7 | -12345 | 2024-01-05T10:00:00Z | 1.02:03:04.5  | {\"a\":1} | 01ff\n\
             null | null |      3 | null                 | -00:00:00.001 | null    | null\n"
        );
    }

    #[test]
    fn empty_result_and_multiple_batches() {
        let (schema, b1) = batch(
            vec![Field::new("x", DataType::Utf8, false)],
            vec![Arc::new(StringArray::from(vec!["a\tb"]))],
        );
        let (_, b2) = batch(
            vec![Field::new("x", DataType::Utf8, false)],
            vec![Arc::new(StringArray::from(vec!["line\nbreak"]))],
        );
        assert_eq!(format_table(&schema, &[]), "x\nstring\n------\n");
        assert_eq!(
            format_table(&schema, &[b1, b2]),
            "x\nstring\n-----------\na\\tb\nline\\nbreak\n"
        );
    }

    #[test]
    fn reals() {
        assert_eq!(format_real(980.0), "980.0");
        assert_eq!(format_real(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(format_real(1e20), "1e20");
        assert_eq!(format_real(-0.0), "-0.0");
        assert_eq!(format_real(f64::NAN), "NaN");
        assert_eq!(format_real(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    fn datetimes() {
        assert_eq!(format_datetime(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_datetime(-1), "1969-12-31T23:59:59.999999Z");
        assert_eq!(
            format_datetime(1_709_210_096_120_000),
            "2024-02-29T12:34:56.12Z"
        );
    }

    #[test]
    fn timespans() {
        assert_eq!(format_timespan(0), "00:00:00");
        assert_eq!(format_timespan(250_000), "00:00:00.25");
        assert_eq!(format_timespan(i64::MIN), "-106751991.04:00:54.775808");
    }
}
