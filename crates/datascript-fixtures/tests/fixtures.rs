//! Checks the committed files in `tests/data/` against the generator, and the properties that
//! `tests/data/README.md` promises.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::Path;

use arrow_array::Array;
use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, TimestampMicrosecondType};
use datascript_fixtures::{
    CUSTOMERS, NULL_AMOUNT_CUSTOMER, data_dir, format_real, format_timestamp, generate, order_rows,
    unix_seconds,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

const REGENERATE: &str = "run `cargo run -p datascript-fixtures` and commit the result";

#[test]
fn committed_files_match_the_generator() {
    let dir = data_dir();
    let mut stale = Vec::new();
    for fixture in generate() {
        match std::fs::read(dir.join(fixture.path)) {
            Ok(bytes) if bytes == fixture.bytes => {}
            Ok(_) => stale.push(format!("{} differs", fixture.path)),
            Err(err) => stale.push(format!("{} could not be read: {err}", fixture.path)),
        }
    }
    assert!(stale.is_empty(), "{}; {REGENERATE}", stale.join(", "));
}

#[test]
fn every_data_file_is_generated_or_documented() {
    let generated: BTreeSet<String> = generate().iter().map(|f| f.path.to_string()).collect();
    let mut on_disk = BTreeSet::new();
    list_files(&data_dir(), "", &mut on_disk);
    on_disk.remove("README.md");
    let extra: Vec<_> = on_disk.difference(&generated).collect();
    assert!(
        extra.is_empty(),
        "files in tests/data that the generator does not produce: {extra:?}. Add them to the \
         generator or delete them"
    );

    let readme = std::fs::read_to_string(data_dir().join("README.md")).expect("README.md");
    let undocumented: Vec<_> = generated
        .iter()
        .filter(|p| !readme.contains(&format!("`{p}`")))
        .collect();
    assert!(
        undocumented.is_empty(),
        "tests/data/README.md does not mention {undocumented:?}"
    );
}

#[test]
fn total_size_is_under_one_megabyte() {
    let total: usize = generate().iter().map(|f| f.bytes.len()).sum();
    assert!(total < 1_000_000, "fixtures total {total} bytes");
}

#[test]
fn orders_parquet_has_the_same_rows_as_orders_csv() {
    let file = File::open(data_dir().join("orders.parquet")).expect("orders.parquet");
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("valid parquet")
        .build()
        .expect("reader");
    let mut csv = String::from("orderId,customerId,amount,timestamp\n");
    for batch in reader {
        let batch = batch.expect("batch");
        let ids = batch.column(0).as_string::<i32>();
        let customers = batch.column(1).as_string::<i32>();
        let amounts = batch.column(2).as_primitive::<Float64Type>();
        let times = batch.column(3).as_primitive::<TimestampMicrosecondType>();
        for i in 0..batch.num_rows() {
            let amount = if amounts.is_null(i) {
                String::new()
            } else {
                format_real(amounts.value(i))
            };
            csv.push_str(&format!(
                "{},{},{amount},{}\n",
                ids.value(i),
                customers.value(i),
                format_timestamp(times.value(i) / 1_000_000)
            ));
        }
    }
    let expected = std::fs::read_to_string(data_dir().join("orders.csv")).expect("orders.csv");
    assert_eq!(csv, expected);
}

#[test]
fn orders_have_the_documented_properties() {
    let orders = order_rows();
    assert_eq!(orders.len(), 50);

    let mut revenue: BTreeMap<&str, f64> = BTreeMap::new();
    for o in &orders {
        *revenue.entry(o.customer_id).or_default() += o.amount.unwrap_or(0.0);
    }
    let mut ranked: Vec<_> = revenue.iter().map(|(c, r)| (*c, *r)).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    assert_eq!(
        ranked[..5],
        [
            ("c-042", 1250.5),
            ("c-007", 980.0),
            ("c-113", 402.25),
            ("c-001", 300.0),
            ("c-019", 300.0),
        ],
        "top customers by revenue"
    );
    assert!(ranked[5..].iter().all(|(_, r)| *r < 300.0));

    let known: BTreeSet<&str> = CUSTOMERS.iter().map(|c| c.0).collect();
    let ordering: BTreeSet<&str> = orders.iter().map(|o| o.customer_id).collect();
    assert_eq!(
        known.difference(&ordering).collect::<Vec<_>>(),
        [&"c-200"],
        "customers without orders"
    );
    assert_eq!(
        ordering.difference(&known).collect::<Vec<_>>(),
        [&"c-999"],
        "orders without a customer"
    );

    let nulls: Vec<_> = orders.iter().filter(|o| o.amount.is_none()).collect();
    assert_eq!(nulls.len(), 1);
    assert_eq!(nulls[0].customer_id, NULL_AMOUNT_CUSTOMER);

    let times: Vec<i64> = orders.iter().map(|o| o.timestamp).collect();
    assert!(times.is_sorted(), "orders are in timestamp order");
    assert!(times.windows(2).any(|w| w[0] == w[1]), "a timestamp tie");
    assert!(times.contains(&unix_seconds(2024, 2, 29, 23, 59, 59)));
    assert!(times.contains(&unix_seconds(2024, 3, 1, 0, 0, 0)));
    assert!(times[0] >= unix_seconds(2023, 12, 1, 0, 0, 0));
    assert!(times[49] < unix_seconds(2024, 6, 1, 0, 0, 0));
}

fn list_files(dir: &Path, prefix: &str, out: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).expect("read tests/data") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".DS_Store" {
            continue;
        }
        let path = format!("{prefix}{name}");
        if entry.file_type().expect("file type").is_dir() {
            list_files(&entry.path(), &format!("{path}/"), out);
        } else {
            out.insert(path);
        }
    }
}
