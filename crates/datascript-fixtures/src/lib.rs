//! Generator for the shared fixture datasets in `tests/data/`.
//!
//! Every file is built in memory from a fixed seed, so regenerating gives byte-identical output.
//! `cargo run -p datascript-fixtures` writes the files; the crate's tests check that the committed
//! files match what the generator produces and that the properties documented in
//! `tests/data/README.md` hold.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    ArrayRef, BinaryArray, Date32Array, Decimal128Array, DictionaryArray, Float32Array,
    Float64Array, Int16Array, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
    TimestampNanosecondArray, UInt32Array, UInt64Array,
};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

/// The seed for every random choice the generator makes.
pub const SEED: u64 = 0x5EED_DA7A_2024;

/// One generated file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixture {
    /// Path relative to `tests/data/`, with `/` separators.
    pub path: &'static str,
    /// The file contents.
    pub bytes: Vec<u8>,
}

/// The `tests/data/` directory of this repository.
pub fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate lives in <repo>/crates/<name>")
        .join("tests/data")
}

/// Builds every fixture file, in a fixed order.
pub fn generate() -> Vec<Fixture> {
    let mut rng = Rng(SEED);
    let orders = orders(&mut rng);
    let mut files = vec![
        fixture("orders.csv", orders_csv(&orders)),
        fixture("orders.parquet", orders_parquet(&orders)),
        fixture("customers.csv", customers_csv()),
    ];
    for month in 1..=3 {
        let path = match month {
            1 => "events/2024-01.ndjson",
            2 => "events/2024-02.ndjson",
            _ => "events/2024-03.ndjson",
        };
        files.push(fixture(path, events_ndjson(&mut rng, month)));
    }
    files.extend(edge_files());
    files.extend(reordered_files());
    files.push(fixture("types.parquet", types_parquet()));
    files.extend(source_files(&orders_csv(&orders)));
    files
}

fn fixture(path: &'static str, bytes: impl Into<Vec<u8>>) -> Fixture {
    Fixture {
        path,
        bytes: bytes.into(),
    }
}

// ---------------------------------------------------------------------------------------------
// orders + customers
// ---------------------------------------------------------------------------------------------

/// One row of `orders.csv`. Timestamps are Unix seconds, UTC.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub order_id: String,
    pub customer_id: &'static str,
    pub amount: Option<f64>,
    pub timestamp: i64,
}

/// How a customer's order amounts are chosen. All amounts are multiples of 0.25, so sums are
/// exact in `f64` whatever order they are added in.
#[derive(Debug)]
enum Amounts {
    /// Exactly these amounts.
    Fixed(&'static [f64]),
    /// `n` random amounts that add up to exactly this total.
    Total(f64, usize),
    /// `n` random amounts in `[5, 70]`, so the total stays below 300.
    Random(usize),
}

/// Who orders what. `c-200` is in `customers.csv` but has no orders; `c-999` has orders but is
/// not in `customers.csv`.
const ORDER_PLAN: &[(&str, Amounts)] = &[
    ("c-042", Amounts::Total(1250.5, 7)),
    ("c-007", Amounts::Total(980.0, 6)),
    ("c-113", Amounts::Total(402.25, 4)),
    ("c-001", Amounts::Fixed(&[75.0, 75.0, 75.0, 75.0])),
    ("c-019", Amounts::Fixed(&[100.0, 100.0, 100.0])),
    ("c-002", Amounts::Random(4)),
    ("c-003", Amounts::Random(3)),
    ("c-011", Amounts::Random(4)),
    ("c-023", Amounts::Random(3)),
    ("c-056", Amounts::Random(3)),
    ("c-077", Amounts::Random(3)),
    ("c-128", Amounts::Random(3)),
    ("c-999", Amounts::Random(2)),
];

/// Customer whose extra order has a null amount.
pub const NULL_AMOUNT_CUSTOMER: &str = "c-011";

/// One row of `customers.csv`: `customerId, name, country, signupDate (year, month, day)`.
/// An empty country is written as an empty field (null).
pub type Customer = (&'static str, &'static str, &'static str, (i32, u32, u32));

/// The rows of `customers.csv`.
pub const CUSTOMERS: &[Customer] = &[
    ("c-001", "Ana Silva", "BR", (2023, 1, 9)),
    ("c-002", "Ben Okafor", "NG", (2023, 2, 14)),
    ("c-003", "Chen Wei", "CN", (2023, 2, 28)),
    ("c-007", "Dana Novak", "CZ", (2023, 3, 3)),
    ("c-011", "Eli Cohen", "IL", (2023, 4, 17)),
    ("c-019", "Fatima Zahra", "MA", (2023, 5, 5)),
    ("c-023", "Gus Larsen", "DK", (2023, 6, 21)),
    ("c-042", "Hana Sato", "JP", (2023, 7, 1)),
    ("c-056", "Ivan Petrov", "BG", (2023, 8, 12)),
    ("c-077", "Jo Martin", "", (2023, 9, 30)),
    ("c-113", "Kai Mueller", "DE", (2023, 10, 8)),
    ("c-128", "Lena Fischer", "DE", (2023, 11, 19)),
    ("c-200", "Mo Haddad", "JO", (2024, 5, 20)),
];

fn orders(rng: &mut Rng) -> Vec<Order> {
    let start = unix_seconds(2023, 12, 1, 0, 0, 0);
    let end = unix_seconds(2024, 6, 1, 0, 0, 0);
    let mut rows: Vec<(&'static str, Option<f64>)> = Vec::new();
    for (customer, amounts) in ORDER_PLAN {
        let amounts = match *amounts {
            Amounts::Fixed(fixed) => fixed.to_vec(),
            Amounts::Total(total, n) => split_total(rng, total, n),
            Amounts::Random(n) => (0..n).map(|_| rng.quarters(5.0, 70.0)).collect(),
        };
        rows.extend(amounts.into_iter().map(|a| (*customer, Some(a))));
    }
    rows.push((NULL_AMOUNT_CUSTOMER, None));

    let mut timestamps: Vec<i64> = rows
        .iter()
        .map(|_| start + rng.below((end - start) as u64) as i64)
        .collect();
    // Edge timestamps that tests rely on: a leap day, a month boundary, and a tie.
    timestamps[5] = unix_seconds(2024, 2, 29, 23, 59, 59);
    timestamps[17] = unix_seconds(2024, 3, 1, 0, 0, 0);
    timestamps[31] = timestamps[30];

    let mut orders: Vec<Order> = rows
        .into_iter()
        .zip(timestamps)
        .map(|((customer_id, amount), timestamp)| Order {
            order_id: String::new(),
            customer_id,
            amount,
            timestamp,
        })
        .collect();
    // Stable, so the two tied rows keep their plan order.
    orders.sort_by_key(|o| o.timestamp);
    for (i, order) in orders.iter_mut().enumerate() {
        order.order_id = format!("o-{:04}", i + 1);
    }
    orders
}

/// `n` multiples of 0.25 that add up to exactly `total`, each roughly `total / n`.
fn split_total(rng: &mut Rng, total: f64, n: usize) -> Vec<f64> {
    let weights: Vec<f64> = (0..n)
        .map(|_| 1.0 + rng.below(200) as f64 / 100.0)
        .collect();
    let sum: f64 = weights.iter().sum();
    let mut parts: Vec<f64> = weights[..n - 1]
        .iter()
        .map(|w| (total * w / sum * 4.0).round() / 4.0)
        .collect();
    let rest = total - parts.iter().sum::<f64>();
    parts.push(rest);
    parts
}

fn orders_csv(orders: &[Order]) -> String {
    let mut out = String::from("orderId,customerId,amount,timestamp\n");
    for o in orders {
        out.push_str(&format!(
            "{},{},{},{}\n",
            o.order_id,
            o.customer_id,
            o.amount.map(format_real).unwrap_or_default(),
            format_timestamp(o.timestamp)
        ));
    }
    out
}

fn orders_parquet(orders: &[Order]) -> Vec<u8> {
    let columns: Vec<(&str, ArrayRef)> = vec![
        (
            "orderId",
            Arc::new(StringArray::from_iter_values(
                orders.iter().map(|o| o.order_id.as_str()),
            )),
        ),
        (
            "customerId",
            Arc::new(StringArray::from_iter_values(
                orders.iter().map(|o| o.customer_id),
            )),
        ),
        (
            "amount",
            Arc::new(Float64Array::from_iter(orders.iter().map(|o| o.amount))),
        ),
        (
            "timestamp",
            Arc::new(
                TimestampMicrosecondArray::from_iter_values(
                    orders.iter().map(|o| o.timestamp * 1_000_000),
                )
                .with_timezone("UTC"),
            ),
        ),
    ];
    parquet_bytes(columns)
}

fn customers_csv() -> String {
    let mut out = String::from("customerId,name,country,signupDate\n");
    for (id, name, country, (y, m, d)) in CUSTOMERS {
        out.push_str(&format!("{id},{name},{country},{y:04}-{m:02}-{d:02}\n"));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// events
// ---------------------------------------------------------------------------------------------

const EVENT_ROWS_PER_FILE: usize = 8;

fn events_ndjson(rng: &mut Rng, month: u32) -> String {
    const USERS: &[&str] = &["u-1", "u-2", "u-3", "u-4", "u-5"];
    const EVENTS: &[&str] = &["view", "click", "purchase"];
    const PAGES: &[&str] = &["/home", "/pricing", "/docs"];
    const COUNTRIES: &[&str] = &["DE", "JP", "US"];

    let start = unix_seconds(2024, month, 1, 0, 0, 0);
    let end = unix_seconds(2024, month + 1, 1, 0, 0, 0);
    let mut times: Vec<i64> = (0..EVENT_ROWS_PER_FILE)
        .map(|_| start + rng.below((end - start) as u64) as i64)
        .collect();
    times.sort_unstable();

    let mut out = String::new();
    for (i, ts) in times.into_iter().enumerate() {
        let user = rng.pick(USERS);
        let event = rng.pick(EVENTS);
        let duration = 10 + rng.below(4990);
        let page = rng.pick(PAGES);
        let referrer = if rng.below(2) == 0 {
            "null".to_string()
        } else {
            json_string("newsletter")
        };
        // March starts with a null duration, so the column has a null in one file only.
        let duration = if month == 3 && i == 0 {
            "null".to_string()
        } else {
            duration.to_string()
        };
        out.push_str(&format!(
            "{{\"ts\":{},\"user\":{},\"event\":{},\"duration_ms\":{duration},\"props\":{{\"page\":{},\"ref\":{referrer}}}",
            json_string(&format_timestamp(ts)),
            json_string(user),
            json_string(event),
            json_string(page),
        ));
        // Only February has a `country` field.
        if month == 2 {
            out.push_str(&format!(
                ",\"country\":{}",
                json_string(rng.pick(COUNTRIES))
            ));
        }
        out.push_str("}\n");
    }
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------------------------
// edge cases and reordered headers (hand-written; no randomness)
// ---------------------------------------------------------------------------------------------

fn edge_files() -> Vec<Fixture> {
    vec![
        fixture("edge/empty.csv", ""),
        fixture("edge/header_only.csv", "id,name,value\n"),
        fixture(
            "edge/nulls.csv",
            "id,name,value,when,always_null\n\
             1,alpha,1.5,2024-01-01T00:00:00Z,\n\
             2,,2.5,2024-01-02T00:00:00Z,\n\
             3,gamma,,2024-01-03T00:00:00Z,\n\
             4,delta,4.5,,\n\
             5,,,,\n",
        ),
        fixture(
            "edge/unicode.csv",
            "id,name,città,unit price\n\
             1,Zoë,Zürich,12.5\n\
             2,José,São Paulo,7.25\n\
             3,李雷,北京,30.0\n\
             4,Даша,Москва,9.75\n\
             5,سارة,القاهرة,4.0\n\
             6,e\u{301}mile,Montre\u{301}al,1.0\n\
             7,😀 smile,🌍,0.5\n",
        ),
        fixture(
            "edge/quoted.csv",
            "id,text,amount\n\
             1,\"a, b\",1.5\n\
             2,\"line one\nline two\",2.5\n\
             3,\"she said \"\"hi\"\"\",3.5\n\
             4,\"\",4.5\n\
             5,plain,5.5\n\
             6,\" padded \",6.5\n",
        ),
    ]
}

fn reordered_files() -> Vec<Fixture> {
    vec![
        fixture(
            "reordered/a.csv",
            "id,name,score\n1,ant,10\n2,bee,20\n3,cat,30\n",
        ),
        fixture(
            "reordered/b.csv",
            "score,name,id\n40,dog,4\n50,eel,5\n60,fox,6\n",
        ),
        fixture(
            "reordered/c.csv",
            "name,id,score,extra\ngnu,7,70,x\nhen,8,80,y\n",
        ),
    ]
}

/// Files for source tests (M1-08): glob widening and conflicts, compression, JSON arrays, reader
/// options, unreadable files, and `UInt64` values above `i64::MAX`.
fn source_files(orders_csv: &str) -> Vec<Fixture> {
    vec![
        fixture("widen/a.csv", "id,score\n1,10\n2,20\n"),
        fixture("widen/b.csv", "id,score\n3,30.5\n4,40.25\n"),
        fixture(
            "conflict/a.csv",
            "id,when\n1,2024-01-01T00:00:00Z\n2,2024-01-02T00:00:00Z\n",
        ),
        fixture("conflict/b.csv", "id,when\n3,soon\n4,later\n"),
        fixture("compressed/orders.csv.gz", gzip(orders_csv.as_bytes())),
        fixture(
            "compressed/orders.tsv.zst",
            zstd(orders_csv.replace(',', "\t").as_bytes()),
        ),
        fixture("compressed/orders.dat", gzip(orders_csv.as_bytes())),
        fixture(
            "json/array.json",
            "[\n  {\"id\": 1, \"name\": \"ant\", \"score\": 1.5},\n  \
             {\"id\": 2, \"name\": \"bee\", \"score\": null},\n  \
             {\"id\": 3, \"name\": \"cat\", \"score\": 3.25}\n]\n",
        ),
        fixture(
            "json/lines.json",
            "{\"id\": 1, \"name\": \"ant\"}\n{\"id\": 2, \"name\": \"bee\"}\n",
        ),
        fixture(
            "options/semicolon.txt",
            "1;'a;b';1.5\n2;NA;2.5\n3;plain;NA\n",
        ),
        fixture("bad/broken.json", "{\"id\": 1}\n{\"id\": \n"),
        fixture("bad/not_parquet.parquet", "this is not a parquet file\n"),
        fixture("uint64.parquet", uint64_parquet()),
    ]
}

/// A gzip file holding `bytes` in one stored (uncompressed) deflate block.
///
/// Built by hand because compressed output depends on flate2's backend, which Cargo's feature
/// unification picks differently when the whole workspace is built (DataFusion's `compression`
/// feature changes it). A stored block is the same everywhere. No file name, zero mtime.
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let len = u16::try_from(bytes.len()).expect("a stored block holds at most 65535 bytes");
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    out.push(1); // BFINAL = 1, BTYPE = 00 (stored)
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(&crc.sum().to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out
}

fn zstd(bytes: &[u8]) -> Vec<u8> {
    zstd::encode_all(bytes, 3).expect("compress in memory")
}

fn uint64_parquet() -> Vec<u8> {
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("id", Arc::new(Int64Array::from(vec![1, 2, 3]))),
        (
            "big",
            Arc::new(UInt64Array::from(vec![
                Some(1),
                Some(i64::MAX as u64),
                Some(u64::MAX),
            ])),
        ),
    ];
    parquet_bytes(columns)
}

// ---------------------------------------------------------------------------------------------
// types.parquet
// ---------------------------------------------------------------------------------------------

fn types_parquet() -> Vec<u8> {
    let s = unix_seconds;
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("id", Arc::new(Int64Array::from(vec![1, 2, 3, 4]))),
        (
            "ts_ns",
            Arc::new(TimestampNanosecondArray::from(vec![
                Some(s(2024, 1, 15, 9, 30, 0) * 1_000_000_000 + 123_456_789),
                Some(s(1969, 12, 31, 23, 59, 59) * 1_000_000_000),
                Some(s(2024, 2, 29, 23, 59, 59) * 1_000_000_000 + 999_999_999),
                None,
            ])),
        ),
        (
            "ts_tz",
            Arc::new(
                TimestampMicrosecondArray::from(vec![
                    Some(s(2024, 1, 15, 9, 30, 0) * 1_000_000),
                    Some(s(2024, 6, 30, 22, 30, 0) * 1_000_000),
                    Some(0),
                    None,
                ])
                .with_timezone("+02:00"),
            ),
        ),
        (
            "date",
            Arc::new(Date32Array::from(vec![
                Some(days_from_civil(2024, 1, 15) as i32),
                Some(-1),
                Some(days_from_civil(2024, 2, 29) as i32),
                None,
            ])),
        ),
        (
            "i16",
            Arc::new(Int16Array::from(vec![
                Some(i16::MIN),
                Some(0),
                Some(i16::MAX),
                None,
            ])),
        ),
        (
            "u32",
            Arc::new(UInt32Array::from(vec![
                Some(0),
                Some(1),
                Some(u32::MAX),
                None,
            ])),
        ),
        (
            "f32",
            Arc::new(Float32Array::from(vec![
                Some(0.1),
                Some(-1.5),
                Some(f32::MAX),
                None,
            ])),
        ),
        (
            "dict",
            Arc::new(
                [Some("red"), Some("green"), Some("red"), None]
                    .into_iter()
                    .collect::<DictionaryArray<Int32Type>>(),
            ),
        ),
        (
            "dec",
            Arc::new(
                Decimal128Array::from(vec![Some(1_234_567), Some(-1), Some(0), None])
                    .with_precision_and_scale(10, 2)
                    .expect("valid decimal precision and scale"),
            ),
        ),
        (
            "bin",
            Arc::new(BinaryArray::from(vec![
                Some(&b"\x00\x01"[..]),
                Some(&b""[..]),
                Some(&b"\xff"[..]),
                None,
            ])),
        ),
    ];
    parquet_bytes(columns)
}

// ---------------------------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------------------------

fn parquet_bytes(columns: Vec<(&str, ArrayRef)>) -> Vec<u8> {
    let batch = RecordBatch::try_from_iter(columns).expect("columns have equal lengths");
    // A fixed `created_by`, so upgrading the parquet crate does not change the bytes.
    let props = WriterProperties::builder()
        .set_created_by("datascript-fixtures".to_string())
        .build();
    let mut buf = Vec::new();
    let mut writer =
        ArrowWriter::try_new(&mut buf, batch.schema(), Some(props)).expect("valid schema");
    writer.write(&batch).expect("write to memory");
    writer.close().expect("finish parquet file");
    buf
}

/// Formats a real the way the CSV fixtures spell it: shortest round-trip form, always with a
/// decimal point (`75.0`, `402.25`), so CSV readers infer a float column.
pub fn format_real(v: f64) -> String {
    format!("{v:?}")
}

/// `YYYY-MM-DDTHH:MM:SSZ` for Unix seconds.
pub fn format_timestamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// Unix seconds for a UTC date and time. `month` may be 13 (January of the next year).
pub fn unix_seconds(year: i32, month: u32, day: u32, h: i64, m: i64, s: i64) -> i64 {
    let (year, month) = if month > 12 {
        (year + 1, month - 12)
    } else {
        (year, month)
    };
    days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s
}

// Howard Hinnant's civil calendar algorithms (proleptic Gregorian).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// SplitMix64: tiny, fast, and stable across platforms and Rust versions.
#[derive(Debug)]
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// A random multiple of 0.25 in `[lo, hi]`.
    fn quarters(&mut self, lo: f64, hi: f64) -> f64 {
        let steps = ((hi - lo) * 4.0) as u64;
        lo + self.below(steps + 1) as f64 / 4.0
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

/// The rows of `orders.csv`, for tests that check its documented properties.
pub fn order_rows() -> Vec<Order> {
    orders(&mut Rng(SEED))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_round_trips() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(
            format_timestamp(unix_seconds(2024, 2, 29, 23, 59, 59)),
            "2024-02-29T23:59:59Z"
        );
        assert_eq!(
            unix_seconds(2024, 13, 1, 0, 0, 0),
            unix_seconds(2025, 1, 1, 0, 0, 0)
        );
        for day in -800_000..800_000 {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y as i32, m as u32, d as u32), day);
        }
    }

    #[test]
    fn split_total_is_exact_and_positive() {
        let mut rng = Rng(1);
        for _ in 0..1000 {
            let parts = split_total(&mut rng, 1250.5, 7);
            assert_eq!(parts.iter().sum::<f64>(), 1250.5);
            assert!(parts.iter().all(|p| *p > 0.0 && (p * 4.0).fract() == 0.0));
        }
    }

    #[test]
    fn real_formatting_keeps_a_decimal_point() {
        assert_eq!(format_real(75.0), "75.0");
        assert_eq!(format_real(402.25), "402.25");
    }
}
