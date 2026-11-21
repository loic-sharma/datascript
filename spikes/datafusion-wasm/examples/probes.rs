//! Native probes for M1-09: timespan as Duration(µs), and checked Int64 arithmetic (D-014).
//! `cargo run --example probes` prints a markdown table: expression, result type, value or error.

use std::sync::Arc;

use datafusion::arrow::array::{
    ArrayRef, DurationMicrosecondArray, Int64Array, RecordBatch, TimestampMicrosecondArray,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::ScalarValue;
use datafusion::functions_aggregate::expr_fn::{avg, max, min, sum};
use datafusion::logical_expr::{ColumnarValue, ScalarUDF, ScalarUDFImpl, Signature, Volatility};
use datafusion::prelude::*;

const HOUR: i64 = 3_600_000_000;
const DAY: i64 = 24 * HOUR;
// 2024-02-29T00:00:00Z and 2024-03-01T12:00:00Z in µs.
const T1: i64 = 1_709_164_800_000_000;
const T2: i64 = T1 + DAY + 12 * HOUR;

fn batch() -> RecordBatch {
    let utc: Arc<str> = "UTC".into();
    let ts = DataType::Timestamp(TimeUnit::Microsecond, Some(utc.clone()));
    let dur = DataType::Duration(TimeUnit::Microsecond);
    let schema = Schema::new(vec![
        Field::new("t1", ts.clone(), false),
        Field::new("t2", ts, false),
        Field::new("d", dur.clone(), true),
        Field::new("h", dur, false),
        Field::new("n", DataType::Int64, false),
        Field::new("zero", DataType::Int64, false),
        Field::new("big", DataType::Int64, false),
        Field::new("small", DataType::Int64, false),
    ]);
    let cols: Vec<ArrayRef> = vec![
        Arc::new(TimestampMicrosecondArray::from(vec![T1, T1, T1]).with_timezone(utc.clone())),
        Arc::new(TimestampMicrosecondArray::from(vec![T2, T2, T2]).with_timezone(utc)),
        Arc::new(DurationMicrosecondArray::from(vec![
            Some(DAY),
            Some(2 * HOUR),
            None,
        ])),
        Arc::new(DurationMicrosecondArray::from(vec![HOUR, HOUR, HOUR])),
        Arc::new(Int64Array::from(vec![3, 3, 3])),
        Arc::new(Int64Array::from(vec![0, 0, 0])),
        Arc::new(Int64Array::from(vec![i64::MAX, i64::MAX, i64::MAX])),
        Arc::new(Int64Array::from(vec![i64::MIN, i64::MIN, i64::MIN])),
    ];
    RecordBatch::try_new(Arc::new(schema), cols).unwrap()
}

/// A checked Int64 add, as M1-09 could lower `long + long` when overflow must be an error.
#[derive(Debug, PartialEq, Eq, Hash)]
struct CheckedAdd {
    signature: Signature,
}

impl ScalarUDFImpl for CheckedAdd {
    fn name(&self) -> &str {
        "ds_checked_add"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> datafusion::error::Result<DataType> {
        Ok(DataType::Int64)
    }
    fn invoke_with_args(
        &self,
        args: datafusion::logical_expr::ScalarFunctionArgs,
    ) -> datafusion::error::Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        // arrow's checked kernel: errors with "Arithmetic overflow" instead of wrapping.
        let out = datafusion::arrow::compute::kernels::numeric::add(&arrays[0], &arrays[1])?;
        Ok(ColumnarValue::Array(out))
    }
}

async fn probe(ctx: &SessionContext, label: &str, e: Expr, agg: bool) {
    let df = ctx.read_batch(batch()).unwrap();
    let df = if agg {
        df.aggregate(vec![], vec![e.alias("r")])
    } else {
        df.select(vec![e.alias("r")])
    };
    let result = match df {
        Err(err) => Err(err),
        Ok(df) => df.collect().await,
    };
    let (ty, val) = match result {
        Ok(batches) => {
            let col = batches[0].column(0);
            (
                format!("{}", col.data_type()),
                array_value_to_string(col, 0).unwrap(),
            )
        }
        Err(err) => (
            "—".to_string(),
            format!("ERROR: {}", err.to_string().replace('\n', " ")),
        ),
    };
    println!("| `{label}` | {ty} | {val} |");
}

fn dur(us: i64) -> Expr {
    lit(ScalarValue::DurationMicrosecond(Some(us)))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(1));
    println!("| Expression | Result type | Value (row 0) |\n|---|---|---|");
    // Durations
    probe(&ctx, "t2 - t1", col("t2") - col("t1"), false).await;
    probe(&ctx, "t1 + d", col("t1") + col("d"), false).await;
    probe(&ctx, "d + t1", col("d") + col("t1"), false).await;
    probe(&ctx, "t1 - d", col("t1") - col("d"), false).await;
    probe(&ctx, "t1 + 1h (lit)", col("t1") + dur(HOUR), false).await;
    probe(&ctx, "d + h", col("d") + col("h"), false).await;
    probe(&ctx, "d - h", col("d") - col("h"), false).await;
    probe(&ctx, "-d", -col("d"), false).await;
    probe(&ctx, "d * n", col("d") * col("n"), false).await;
    probe(&ctx, "n * d", col("n") * col("d"), false).await;
    probe(&ctx, "d / n", col("d") / col("n"), false).await;
    probe(&ctx, "d * 1.5", col("d") * lit(1.5), false).await;
    probe(&ctx, "d / h", col("d") / col("h"), false).await;
    probe(&ctx, "d < h", col("d").lt(col("h")), false).await;
    probe(
        &ctx,
        "(t2 - t1) / h",
        (col("t2") - col("t1")) / col("h"),
        false,
    )
    .await;
    probe(
        &ctx,
        "(t2 - t1) < d",
        (col("t2") - col("t1")).lt(col("d")),
        false,
    )
    .await;
    probe(&ctx, "sum(d)", sum(col("d")), true).await;
    probe(&ctx, "avg(d)", avg(col("d")), true).await;
    probe(&ctx, "min(d)", min(col("d")), true).await;
    probe(&ctx, "max(d)", max(col("d")), true).await;
    probe(&ctx, "sum(t2 - t1)", sum(col("t2") - col("t1")), true).await;
    probe(
        &ctx,
        "cast(d as Int64)",
        cast(col("d"), DataType::Int64),
        false,
    )
    .await;
    probe(
        &ctx,
        "cast(n as Duration(µs))",
        cast(col("n"), DataType::Duration(TimeUnit::Microsecond)),
        false,
    )
    .await;
    probe(
        &ctx,
        "date_bin(1h, t1)",
        date_bin(
            dur(HOUR),
            col("t1"),
            lit(ScalarValue::TimestampMicrosecond(
                Some(0),
                Some("UTC".into()),
            )),
        ),
        false,
    )
    .await;
    probe(
        &ctx,
        "date_bin(1h as Interval, t1)",
        date_bin(
            lit(ScalarValue::new_interval_mdn(0, 0, 3_600_000_000_000)),
            col("t1"),
            lit(ScalarValue::TimestampMicrosecond(
                Some(0),
                Some("UTC".into()),
            )),
        ),
        false,
    )
    .await;
    // Checked integer arithmetic (D-014)
    probe(&ctx, "big + 1", col("big") + lit(1i64), false).await;
    probe(&ctx, "small - 1", col("small") - lit(1i64), false).await;
    probe(&ctx, "big * n", col("big") * col("n"), false).await;
    probe(&ctx, "-small", -col("small"), false).await;
    probe(&ctx, "n / zero", col("n") / col("zero"), false).await;
    probe(&ctx, "n % zero", col("n") % col("zero"), false).await;
    probe(&ctx, "small / -1", col("small") / lit(-1i64), false).await;
    probe(
        &ctx,
        "n / 0 (lit, const-folded)",
        lit(3i64) / lit(0i64),
        false,
    )
    .await;
    probe(
        &ctx,
        "big + 1 (lit, const-folded)",
        lit(i64::MAX) + lit(1i64),
        false,
    )
    .await;
    probe(
        &ctx,
        "-7 / 2",
        lit(-7i64) / col("n").div(col("n")).mul(lit(2i64)),
        false,
    )
    .await;
    probe(
        &ctx,
        "-7 % 2",
        lit(-7i64) % (col("n") / col("n") * lit(2i64)),
        false,
    )
    .await;
    probe(&ctx, "abs(small)", abs(col("small")), false).await;
    probe(&ctx, "sum(big)", sum(col("big")), true).await;
    probe(&ctx, "d * big", col("d") * col("big"), false).await;
    probe(&ctx, "t1 + d*big", col("t1") + col("h") * col("big"), false).await;
    let checked = ScalarUDF::from(CheckedAdd {
        signature: Signature::exact(
            vec![DataType::Int64, DataType::Int64],
            Volatility::Immutable,
        ),
    });
    probe(
        &ctx,
        "ds_checked_add(big, 1) [UDF]",
        checked.call(vec![col("big"), lit(1i64)]),
        false,
    )
    .await;
    probe(
        &ctx,
        "ds_checked_add(n, 1) [UDF]",
        checked.call(vec![col("n"), lit(1i64)]),
        false,
    )
    .await;
    probe(
        &ctx,
        "ds_checked_add(big, 1) [UDF, lit]",
        checked.call(vec![lit(i64::MAX), lit(1i64)]),
        false,
    )
    .await;
}
