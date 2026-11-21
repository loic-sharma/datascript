//! M0-05 spike: DataFusion over in-memory CSV and Parquet bytes, natively and on wasm32.
//!
//! No SQL: DataScript lowers to DataFusion plans directly, so the queries use the DataFrame API.
//! With `--features sql` the same query also runs through `SessionContext::sql` for comparison.

use std::sync::Arc;

use datafusion::arrow::array::{Array, Float64Array, Int64Array};
use datafusion::common::Column;
use datafusion::error::Result;
use datafusion::functions_aggregate::expr_fn::{count, sum};
use datafusion::prelude::*;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{ObjectStoreExt, PutPayload};
use url::Url;
use wasm_bindgen::prelude::*;

pub const ORDERS_CSV: &[u8] = include_bytes!("../../../tests/data/orders.csv");
pub const CUSTOMERS_CSV: &[u8] = include_bytes!("../../../tests/data/customers.csv");
pub const ORDERS_PARQUET: &[u8] = include_bytes!("../../../tests/data/orders.parquet");

/// `count(*)` and `sum(amount)` over one file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Totals {
    pub rows: i64,
    pub sum: f64,
}

/// A session with one partition (no threads on wasm) and an in-memory store at `memory://`.
async fn session_with_files() -> Result<SessionContext> {
    let config = SessionConfig::new().with_target_partitions(1);
    let ctx = SessionContext::new_with_config(config);
    let store = Arc::new(InMemory::new());
    store
        .put(
            &Path::from("orders.csv"),
            PutPayload::from_static(ORDERS_CSV),
        )
        .await?;
    store
        .put(
            &Path::from("orders.parquet"),
            PutPayload::from_static(ORDERS_PARQUET),
        )
        .await?;
    store
        .put(
            &Path::from("customers.csv"),
            PutPayload::from_static(CUSTOMERS_CSV),
        )
        .await?;
    let url = Url::parse("memory://").expect("valid url");
    ctx.register_object_store(&url, store);
    Ok(ctx)
}

async fn totals(df: DataFrame) -> Result<Totals> {
    let batches = df
        .aggregate(
            vec![],
            vec![count(lit(1)).alias("n"), sum(col("amount")).alias("s")],
        )?
        .collect()
        .await?;
    let batch = &batches[0];
    let rows = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0);
    let s = batch
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    let sum = if s.is_null(0) { f64::NAN } else { s.value(0) };
    Ok(Totals { rows, sum })
}

pub async fn csv_totals() -> Result<Totals> {
    let ctx = session_with_files().await?;
    totals(
        ctx.read_csv("memory:///orders.csv", CsvReadOptions::new())
            .await?,
    )
    .await
}

#[cfg(feature = "parquet")]
pub async fn parquet_totals() -> Result<Totals> {
    let ctx = session_with_files().await?;
    totals(
        ctx.read_parquet("memory:///orders.parquet", ParquetReadOptions::default())
            .await?,
    )
    .await
}

/// Join + group by + sort: operators that might spawn tokio tasks (they must not need a runtime).
/// Returns `(country, revenue)` rows, highest revenue first.
#[cfg(feature = "parquet")]
pub async fn revenue_by_country() -> Result<Vec<(Option<String>, f64)>> {
    use datafusion::arrow::array::StringArray;
    use datafusion::arrow::compute::cast;
    use datafusion::arrow::datatypes::DataType;
    let ctx = session_with_files().await?;
    let orders = ctx
        .read_parquet("memory:///orders.parquet", ParquetReadOptions::default())
        .await?
        .alias("o")?;
    let customers = ctx
        .read_csv("memory:///customers.csv", CsvReadOptions::new())
        .await?
        .alias("c")?;
    let batches = orders
        // Both readers name their table `?table?`; without aliases the join keys are ambiguous.
        .join_on(
            customers,
            JoinType::Inner,
            [Expr::Column(Column::new(Some("o"), "customerId"))
                .eq(Expr::Column(Column::new(Some("c"), "customerId")))],
        )?
        .aggregate(
            vec![col("country")],
            vec![sum(col("amount")).alias("revenue")],
        )?
        .sort(vec![col("revenue").sort(false, true)])?
        .collect()
        .await?;
    let mut out = Vec::new();
    for b in &batches {
        let country = cast(b.column(0), &DataType::Utf8)?;
        let country = country.as_any().downcast_ref::<StringArray>().unwrap();
        let revenue = b.column(1).as_any().downcast_ref::<Float64Array>().unwrap();
        for i in 0..b.num_rows() {
            let c = (!country.is_null(i)).then(|| country.value(i).to_string());
            out.push((c, revenue.value(i)));
        }
    }
    Ok(out)
}

/// Re-encode orders.parquet with `codec`, then read it back through DataFusion. Exercises the
/// codec on wasm32 (zstd is C compiled to wasm; snappy, gzip, lz4 and brotli are Rust).
#[cfg(feature = "parquet")]
pub async fn compressed_parquet_totals(codec: &str) -> Result<Totals> {
    use datafusion::parquet::arrow::ArrowWriter;
    use datafusion::parquet::basic::{BrotliLevel, Compression, GzipLevel, ZstdLevel};
    use datafusion::parquet::file::properties::WriterProperties;
    let compression = match codec {
        "snappy" => Compression::SNAPPY,
        "gzip" => Compression::GZIP(GzipLevel::default()),
        "lz4" => Compression::LZ4_RAW,
        "zstd" => Compression::ZSTD(ZstdLevel::default()),
        "brotli" => Compression::BROTLI(BrotliLevel::default()),
        other => panic!("unknown codec {other}"),
    };
    let ctx = session_with_files().await?;
    let batches = ctx
        .read_parquet("memory:///orders.parquet", ParquetReadOptions::default())
        .await?
        .collect()
        .await?;
    let props = WriterProperties::builder()
        .set_compression(compression)
        .build();
    let mut buf = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut buf, batches[0].schema(), Some(props))?;
    for b in &batches {
        writer.write(b)?;
    }
    writer.close()?;
    let store = ctx.runtime_env().object_store(
        datafusion::execution::object_store::ObjectStoreUrl::parse("memory://")?,
    )?;
    let path = format!("orders.{codec}.parquet");
    store
        .put(&Path::from(path.as_str()), PutPayload::from(buf))
        .await?;
    let df = ctx
        .read_parquet(format!("memory:///{path}"), ParquetReadOptions::default())
        .await?;
    totals(df).await
}

#[cfg(feature = "sql")]
pub async fn sql_totals() -> Result<Totals> {
    let ctx = session_with_files().await?;
    ctx.register_csv("orders", "memory:///orders.csv", CsvReadOptions::new())
        .await?;
    let df = ctx
        .sql("SELECT count(*) AS n, sum(amount) AS s FROM orders")
        .await?;
    let batches = df.collect().await?;
    let batch = &batches[0];
    let rows = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0);
    let sum = batch
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap()
        .value(0);
    Ok(Totals { rows, sum })
}

/// JS entry point so the cdylib keeps the engine (and the size measurement is honest).
#[wasm_bindgen]
pub async fn run_spike() -> std::result::Result<String, JsValue> {
    let c = csv_totals()
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    #[cfg(feature = "parquet")]
    {
        let p = parquet_totals()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let r = revenue_by_country()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        return Ok(format!("csv: {c:?}\nparquet: {p:?}\nby country: {r:?}"));
    }
    #[allow(unreachable_code)]
    Ok(format!("csv: {c:?}"))
}
