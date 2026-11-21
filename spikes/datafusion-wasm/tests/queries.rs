//! Runs natively (`cargo test`) and on wasm32 (`wasm-pack test --node` / `--headless --chrome`).

use datafusion_wasm_spike::*;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test_attr;

#[cfg(target_arch = "wasm32")]
macro_rules! spike_test {
    ($name:ident, $body:block) => {
        #[test_attr]
        async fn $name() {
            console_error_panic_hook::set_once();
            $body
        }
    };
}
#[cfg(not(target_arch = "wasm32"))]
macro_rules! spike_test {
    ($name:ident, $body:block) => {
        #[tokio::test(flavor = "current_thread")]
        async fn $name() $body
    };
}

// tests/data/orders.csv: 50 rows; expected sum computed from the file below.
fn expected_sum() -> f64 {
    let text = std::str::from_utf8(ORDERS_CSV).unwrap();
    text.lines()
        .skip(1)
        .filter_map(|l| l.split(',').nth(2))
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<f64>().unwrap())
        .sum()
}

spike_test!(csv_count_and_sum, {
    let t = csv_totals().await.expect("csv query");
    assert_eq!(t.rows, 50);
    assert!((t.sum - expected_sum()).abs() < 1e-9, "{t:?}");
});

#[cfg(feature = "parquet")]
spike_test!(parquet_count_and_sum, {
    let t = parquet_totals().await.expect("parquet query");
    assert_eq!(t.rows, 50);
    assert!((t.sum - expected_sum()).abs() < 1e-9, "{t:?}");
});

#[cfg(feature = "parquet")]
spike_test!(join_group_sort, {
    let rows = revenue_by_country().await.expect("join query");
    assert!(!rows.is_empty());
    assert!(rows.windows(2).all(|w| w[0].1 >= w[1].1), "{rows:?}");
});

#[cfg(feature = "parquet")]
spike_test!(parquet_codecs, {
    for codec in ["snappy", "gzip", "lz4", "zstd", "brotli"] {
        let t = compressed_parquet_totals(codec).await.expect(codec);
        assert_eq!(t.rows, 50, "{codec}");
        assert!((t.sum - expected_sum()).abs() < 1e-9, "{codec}: {t:?}");
    }
});

#[cfg(feature = "sql")]
spike_test!(sql_count_and_sum, {
    let t = sql_totals().await.expect("sql query");
    assert_eq!(t.rows, 50);
});
