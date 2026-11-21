//! Golden-file test harness for DataScript (TESTING.md).
//!
//! Every `tests/cases/**/*.dstest` file is one libtest trial, named by its path:
//!
//! ```bash
//! cargo test -p datascript-tests                       # all cases
//! cargo test -p datascript-tests -- operators/where/   # cases under a directory
//! cargo test -p datascript-tests -- run                # by mode (the trial kind)
//! DS_BLESS=1 cargo test -p datascript-tests            # rewrite expected sections
//! ```
//!
//! - [`TestCase`] parses `.dstest` files and reports malformed ones with a line number.
//! - [`Driver`] runs a case in one mode on one engine. Unsupported modes make a case ignored.
//! - [`MockSchemaProvider`] serves the `schema:` header to `check`/`explain` cases.
//! - [`runner::main`] is the entry point used by `tests/golden.rs`.

pub mod bless;
pub mod case;
pub mod driver;
pub mod output;
pub mod runner;
pub mod schema;

pub use case::{CaseError, ExpectedSection, Mode, Order, TestCase};
pub use driver::{CaseInput, Driver, DriverConfig, DriverError, DriverResult, StubDriver};
pub use runner::Drivers;
pub use schema::MockSchemaProvider;
