//! The interface between the harness and the code under test.
//!
//! A [`Driver`] runs one case on one engine in one [`Mode`] and returns the text to compare with
//! the expected section. Modes a driver does not support yet make the case **ignored**, never
//! passed.

use std::fmt;
use std::path::Path;

use crate::case::Mode;
use crate::schema::MockSchemaProvider;

/// Engine settings that make output deterministic (TESTING.md § Determinism, D-017).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverConfig {
    /// Backends must run with this many partitions (DataFusion: `target_partitions`).
    pub target_partitions: usize,
    /// Backends must use this batch size.
    pub batch_size: usize,
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            target_partitions: 1,
            batch_size: 8192,
        }
    }
}

/// Everything a driver needs to run one case.
#[derive(Debug)]
pub struct CaseInput<'a> {
    /// The `.dstest` file. Relative paths in the query resolve against its directory.
    pub path: &'a Path,
    /// The query text (see [`crate::TestCase::query`]).
    pub query: &'a str,
    /// The fixed clock for `now()`/`ago()`, in microseconds since the Unix epoch.
    pub now_micros: i64,
    /// Host-registered tables from `tables:`, as `(name, path)` with paths relative to the file.
    pub tables: &'a [(String, String)],
    /// Schemas from `schema:`. When present, the driver must not read files.
    pub schema: Option<&'a MockSchemaProvider>,
    /// `show: schema`: `check` also prints each query's output columns.
    pub show_schema: bool,
    pub config: &'a DriverConfig,
}

impl CaseInput<'_> {
    /// The directory relative paths resolve against.
    pub fn base_dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
}

/// Why a driver produced no output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverError {
    /// The mode is not implemented yet.
    NotImplemented,
    /// A harness-level failure, not a program diagnostic (those are output). The case fails.
    Failed(String),
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriverError::NotImplemented => f.write_str("not implemented"),
            DriverError::Failed(message) => f.write_str(message),
        }
    }
}

/// Result of running a case: the text to compare with the expected section.
pub type DriverResult = Result<String, DriverError>;

/// Runs cases against one engine. The runner calls the method for the case's mode.
///
/// Output follows TESTING.md "Expected output rules": diagnostics from the plain renderer, result
/// tables from `datascript_engine::table::format_table`. The runner rewrites absolute paths and
/// sorts rows for `order: any`, so drivers need not.
pub trait Driver: Send + Sync {
    /// The engine name used in `engine:` headers and `---- expected[<engine>]` sections.
    fn engine(&self) -> &str;

    /// Whether the mode is implemented. Cases in unsupported modes are reported as ignored.
    fn supports(&self, mode: Mode) -> bool;

    /// Dumps the CST (`mode: syntax`).
    fn syntax(&self, _input: &CaseInput<'_>) -> DriverResult {
        Err(DriverError::NotImplemented)
    }

    /// Prints diagnostics (`mode: check`).
    fn check(&self, _input: &CaseInput<'_>) -> DriverResult {
        Err(DriverError::NotImplemented)
    }

    /// Prints the logical plan, or diagnostics (`mode: explain`).
    fn explain(&self, _input: &CaseInput<'_>) -> DriverResult {
        Err(DriverError::NotImplemented)
    }

    /// Prints warnings and result tables, or diagnostics (`mode: run`).
    fn run(&self, _input: &CaseInput<'_>) -> DriverResult {
        Err(DriverError::NotImplemented)
    }

    /// Whether every query's output order is defined (LANGUAGE.md §3.4), which makes
    /// `order: strict` the default. `None` until the parser can tell; the default is then `any`.
    fn output_order_defined(&self, _query: &str) -> Option<bool> {
        None
    }
}

/// Calls the driver method for `mode`.
pub fn dispatch(driver: &dyn Driver, mode: Mode, input: &CaseInput<'_>) -> DriverResult {
    match mode {
        Mode::Syntax => driver.syntax(input),
        Mode::Check => driver.check(input),
        Mode::Explain => driver.explain(input),
        Mode::Run => driver.run(input),
    }
}

/// A placeholder for an engine whose pipeline does not exist yet: every mode is unsupported.
#[derive(Debug, Clone)]
pub struct StubDriver {
    engine: String,
}

impl StubDriver {
    pub fn new(engine: impl Into<String>) -> Self {
        Self {
            engine: engine.into(),
        }
    }
}

impl Driver for StubDriver {
    fn engine(&self) -> &str {
        &self.engine
    }

    fn supports(&self, _mode: Mode) -> bool {
        false
    }
}
