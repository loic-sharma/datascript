//! CSV and JSON forms of a query result, for `ds run --format` (M1-11), the web UI and WASM.
//!
//! Values print as in the [table](crate::table): `real` in the shortest form that round-trips,
//! `datetime` in ISO 8601 UTC, `timespan` as `[-][d.]hh:mm:ss[.ffffff]`. Strings are not escaped
//! the table's way, because both formats can carry any text.
//!
//! - CSV: a header row with the column names, then one line per row, quoted as RFC 4180 says
//!   (only fields with `,`, `"`, CR or LF are quoted). Null is an empty field. Lines end in `\n`.
//! - JSON: one object per row, keyed by column name, in column order. `bool` is a JSON boolean,
//!   `int`, `long` and finite `real` values are numbers, `dynamic` is the JSON value itself, null
//!   is `null`, and everything else is a string (`NaN`, `inf` and `-inf` too, which JSON numbers
//!   cannot hold). `long` values beyond ±2^53 lose precision in JavaScript readers.

use std::io::{self, Write};

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int32Type, Int64Type};
use arrow_array::{Array, RecordBatch};
use arrow_schema::Schema;
use datascript_compiler::Type;
use serde_json::{Map, Number, Value};

use crate::table::{datascript_type, value_text};

/// Writes the result as CSV with a header row.
pub fn write_csv(out: &mut dyn Write, schema: &Schema, batches: &[RecordBatch]) -> io::Result<()> {
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    write_csv_line(out, names.iter().map(|name| Some(*name)))?;
    let types: Vec<Option<Type>> = schema.fields().iter().map(|f| datascript_type(f)).collect();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let cells: Vec<Option<String>> = batch
                .columns()
                .iter()
                .zip(&types)
                .map(|(array, &ty)| value_text(array.as_ref(), ty, row))
                .collect();
            write_csv_line(out, cells.iter().map(Option::as_deref))?;
        }
    }
    Ok(())
}

fn write_csv_line<'a>(
    out: &mut dyn Write,
    cells: impl Iterator<Item = Option<&'a str>>,
) -> io::Result<()> {
    let mut line = String::new();
    for (i, cell) in cells.enumerate() {
        if i > 0 {
            line.push(',');
        }
        let cell = cell.unwrap_or("");
        if cell.contains([',', '"', '\r', '\n']) {
            line.push('"');
            line.push_str(&cell.replace('"', "\"\""));
            line.push('"');
        } else {
            line.push_str(cell);
        }
    }
    line.push('\n');
    out.write_all(line.as_bytes())
}

/// The rows of the result as JSON objects.
pub fn json_rows(schema: &Schema, batches: &[RecordBatch]) -> Vec<Map<String, Value>> {
    let types: Vec<Option<Type>> = schema.fields().iter().map(|f| datascript_type(f)).collect();
    let mut rows = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let object = schema
                .fields()
                .iter()
                .zip(batch.columns())
                .zip(&types)
                .map(|((field, array), &ty)| {
                    (field.name().clone(), json_value(array.as_ref(), ty, row))
                })
                .collect();
            rows.push(object);
        }
    }
    rows
}

/// Writes the result as one JSON array of row objects, followed by a newline.
pub fn write_json(out: &mut dyn Write, schema: &Schema, batches: &[RecordBatch]) -> io::Result<()> {
    let rows: Vec<Value> = json_rows(schema, batches)
        .into_iter()
        .map(Value::Object)
        .collect();
    serde_json::to_writer(&mut *out, &rows)?;
    out.write_all(b"\n")
}

/// Writes the result as newline-delimited JSON: one row object per line.
pub fn write_ndjson(
    out: &mut dyn Write,
    schema: &Schema,
    batches: &[RecordBatch],
) -> io::Result<()> {
    for row in json_rows(schema, batches) {
        serde_json::to_writer(&mut *out, &row)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn json_value(array: &dyn Array, ty: Option<Type>, i: usize) -> Value {
    if array.is_null(i) {
        return Value::Null;
    }
    match ty {
        Some(Type::Bool) => Value::Bool(array.as_boolean().value(i)),
        Some(Type::Int) => Value::from(array.as_primitive::<Int32Type>().value(i)),
        Some(Type::Long) => Value::from(array.as_primitive::<Int64Type>().value(i)),
        Some(Type::Real) => {
            let v = array.as_primitive::<Float64Type>().value(i);
            Number::from_f64(v).map_or_else(|| text(array, ty, i), Value::Number)
        }
        Some(Type::Dynamic) => {
            let raw = array.as_string::<i32>().value(i);
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
        }
        _ => text(array, ty, i),
    }
}

fn text(array: &dyn Array, ty: Option<Type>, i: usize) -> Value {
    value_text(array, ty, i).map_or(Value::Null, Value::String)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_array::{
        ArrayRef, BooleanArray, DurationMicrosecondArray, Float64Array, Int32Array, Int64Array,
        StringArray, TimestampMicrosecondArray,
    };
    use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
    use arrow_schema::{DataType, Field, TimeUnit};

    use super::*;

    fn sample() -> (Schema, RecordBatch) {
        let schema = Schema::new(vec![
            Field::new("name", DataType::Utf8, true),
            Field::new("n", DataType::Int32, true),
            Field::new("big", DataType::Int64, true),
            Field::new("x", DataType::Float64, true),
            Field::new("ok", DataType::Boolean, true),
            Field::new(
                "t",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                true,
            ),
            Field::new("d", DataType::Duration(TimeUnit::Microsecond), true),
            Field::new("j", DataType::Utf8, true).with_metadata(HashMap::from([(
                EXTENSION_TYPE_NAME_KEY.to_string(),
                "arrow.json".to_string(),
            )])),
        ]);
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(vec![
                Some("plain"),
                Some("a,\"b\"\nc"),
                None,
            ])),
            Arc::new(Int32Array::from(vec![Some(1), None, Some(-3)])),
            Arc::new(Int64Array::from(vec![
                Some(9_007_199_254_740_993),
                None,
                None,
            ])),
            Arc::new(Float64Array::from(vec![Some(980.0), Some(f64::NAN), None])),
            Arc::new(BooleanArray::from(vec![Some(true), Some(false), None])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![Some(1_704_448_800_000_000), None, None])
                    .with_timezone("UTC"),
            ),
            Arc::new(DurationMicrosecondArray::from(vec![
                Some(93_784_500_000),
                None,
                None,
            ])),
            Arc::new(StringArray::from(vec![Some("{\"a\":[1,2]}"), None, None])),
        ];
        let batch = RecordBatch::try_new(Arc::new(schema.clone()), arrays).unwrap();
        (schema, batch)
    }

    fn written(
        write: fn(&mut dyn Write, &Schema, &[RecordBatch]) -> io::Result<()>,
        schema: &Schema,
        batches: &[RecordBatch],
    ) -> String {
        let mut out = Vec::new();
        write(&mut out, schema, batches).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn csv_quotes_only_when_needed_and_prints_nulls_empty() {
        let (schema, batch) = sample();
        assert_eq!(
            written(write_csv, &schema, &[batch]),
            "name,n,big,x,ok,t,d,j\n\
             plain,1,9007199254740993,980.0,true,2024-01-05T10:00:00Z,1.02:03:04.5,\"{\"\"a\"\":[1,2]}\"\n\
             \"a,\"\"b\"\"\nc\",,,NaN,false,,,\n\
             ,-3,,,,,,\n"
        );
    }

    #[test]
    fn csv_of_an_empty_result_is_the_header() {
        let (schema, _) = sample();
        assert_eq!(written(write_csv, &schema, &[]), "name,n,big,x,ok,t,d,j\n");
    }

    #[test]
    fn json_uses_native_types_where_json_has_them() {
        let (schema, batch) = sample();
        assert_eq!(
            written(write_ndjson, &schema, std::slice::from_ref(&batch)),
            "{\"name\":\"plain\",\"n\":1,\"big\":9007199254740993,\"x\":980.0,\"ok\":true,\
             \"t\":\"2024-01-05T10:00:00Z\",\"d\":\"1.02:03:04.5\",\"j\":{\"a\":[1,2]}}\n\
             {\"name\":\"a,\\\"b\\\"\\nc\",\"n\":null,\"big\":null,\"x\":\"NaN\",\"ok\":false,\
             \"t\":null,\"d\":null,\"j\":null}\n\
             {\"name\":null,\"n\":-3,\"big\":null,\"x\":null,\"ok\":null,\"t\":null,\"d\":null,\
             \"j\":null}\n"
        );
        let array = written(write_json, &schema, &[batch]);
        assert!(array.starts_with("[{\"name\":\"plain\""), "{array}");
        assert!(array.ends_with("}]\n"), "{array}");
        assert_eq!(written(write_json, &schema, &[]), "[]\n");
    }
}
