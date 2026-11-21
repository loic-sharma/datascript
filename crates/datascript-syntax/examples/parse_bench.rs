//! Parses a generated 10,000-line program and prints how long it took (M1-02 acceptance:
//! under 50 ms in release).
//!
//! ```bash
//! cargo run --release -p datascript-syntax --example parse_bench
//! ```

use std::time::Instant;

use datascript_diagnostics::FileId;

const BLOCK: &str = "\
let threshold = 100 * 2 + -3;
let big = 'data/orders.csv'
| where amount > threshold and customerId in ('c-1', 'c-2') and not (status has \"x\")
| extend total = amount * 1.2, day = bin(timestamp, 1d)
| project-away tmp*, other
| summarize revenue = sum(total), n = count() by customerId, day
| sort by revenue desc nulls last, customerId asc
| take 100;
big
| join kind = leftouter (read_csv('customers.csv', delimiter = ';') | project customerId, name) on customerId
| where day between (datetime(2024-01-01) .. datetime(2024-06-30))
| top 10 by revenue desc
| render timechart with (title = \"Revenue\", ycolumns = revenue, n);
";

fn main() {
    let lines_per_block = BLOCK.lines().count();
    let text = BLOCK.repeat(10_000 / lines_per_block + 1);
    let lines = text.lines().count();
    // Warm up, then take the best of a few runs.
    let mut best = f64::MAX;
    for _ in 0..5 {
        let start = Instant::now();
        let parse = datascript_syntax::parse(&text, FileId(0));
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        assert!(parse.errors.is_empty(), "{:?}", parse.errors.first());
        best = best.min(elapsed);
    }
    println!(
        "parsed {lines} lines ({} bytes) in {best:.1} ms",
        text.len()
    );
}
