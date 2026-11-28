//! Runtime errors that know where they come from, and the mapping from DataFusion errors to
//! diagnostics (DIAGNOSTICS.md rule 10).
//!
//! Every UDF and UDAF this backend lowers to carries the span of the expression it computes. When
//! it fails, it returns a [`RuntimeError`] with that span inside a `DataFusionError::External`, so
//! the diagnostic points at the expression that failed, whether DataFusion ran it while folding
//! constants or while executing.

use std::error::Error;
use std::fmt;

use datafusion::arrow::error::ArrowError;
use datafusion::error::DataFusionError;
use datascript_diagnostics::codes::{DS3008, DS4002, DS4003, DS4004};
use datascript_diagnostics::{Diagnostic, Span};

use crate::checked::ValueOutOfRange;
use crate::json::{JsonReadError, JsonReadErrorKind};

/// Why a computation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fail {
    /// The result does not fit its type.
    Overflow,
    /// Integer division or modulo by zero.
    DivisionByZero,
    /// An argument value the function cannot use: `message` is the diagnostic message,
    /// `note` the detail (the regex error, the valid values).
    Invalid {
        message: String,
        note: Option<String>,
    },
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::Overflow => f.write_str("overflow"),
            Fail::DivisionByZero => f.write_str("division by zero"),
            Fail::Invalid { message, .. } => f.write_str(message),
        }
    }
}

impl Error for Fail {}

impl Fail {
    pub(crate) fn invalid(message: impl Into<String>, note: impl Into<String>) -> Fail {
        Fail::Invalid {
            message: message.into(),
            note: Some(note.into()),
        }
    }
}

/// A [`Fail`] at the expression `span`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeError {
    pub span: Span,
    /// What was computed, for messages: an operator symbol in backticks or a function name.
    pub what: String,
    /// The DataScript type of the result, for overflow messages.
    pub ty: &'static str,
    pub fail: Fail,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed: {:?}", self.what, self.fail)
    }
}

impl Error for RuntimeError {}

impl RuntimeError {
    pub(crate) fn into_datafusion(self) -> DataFusionError {
        DataFusionError::External(Box::new(self))
    }

    fn diagnostic(&self) -> Diagnostic {
        match &self.fail {
            Fail::Overflow => {
                let d = Diagnostic::error(
                    DS4003,
                    format!(
                        "the result of {} does not fit in a `{}`",
                        self.what, self.ty
                    ),
                )
                .primary(self.span, "this overflowed");
                match self.ty {
                    "long" => d.note(format!(
                        "a `long` holds whole numbers from {} to {}",
                        i64::MIN,
                        i64::MAX
                    )),
                    "datetime" | "timespan" => d.note(
                        "datetimes and timespans are counted in microseconds and hold about \
                         ±292,000 years",
                    ),
                    _ => d,
                }
            }
            Fail::DivisionByZero => {
                let op = if self.what.contains('%') {
                    "modulo"
                } else {
                    "division"
                };
                let subject = if self.ty == "timespan" {
                    "timespan"
                } else {
                    "integer"
                };
                let d = Diagnostic::error(DS4003, format!("{subject} {op} by zero"))
                    .primary(self.span, "the divisor is zero for at least one row");
                if self.ty == "timespan" {
                    // A timespan has no `inf`, so converting is no way out.
                    d.help("filter out the zero divisors with `where`")
                } else {
                    d.help(
                        "convert to `real` to get `inf` or `NaN` instead, or filter out the zero \
                         divisors with `where`",
                    )
                }
            }
            Fail::Invalid { message, note } => {
                let d = Diagnostic::error(DS4004, message.clone())
                    .primary(self.span, format!("in this call of {}", self.what));
                match note {
                    Some(note) => d.note(note.clone()),
                    None => d,
                }
            }
        }
    }
}

/// Where errors that carry no span are reported.
#[derive(Debug, Clone)]
pub(crate) struct ErrorContext {
    /// Each scan: its span and the source as written (a path, glob or table name).
    pub scans: Vec<(Span, String)>,
}

impl ErrorContext {
    /// The diagnostic for a DataFusion error.
    ///
    /// A [`RuntimeError`] points at its expression and a `UInt64` out of range at the scan. A
    /// malformed row is `DS3008` at the scan when the query reads one source; otherwise, and for
    /// every other error, `DS4002` at `fallback`.
    pub(crate) fn diagnostic(&self, fallback: Span, error: &DataFusionError) -> Diagnostic {
        if let Some(e) = find_cause::<RuntimeError>(error) {
            return e.diagnostic();
        }
        if let Some(out_of_range) = find_cause::<ValueOutOfRange>(error) {
            let span = match self.scans.as_slice() {
                [(span, _)] => *span,
                _ => fallback,
            };
            return Diagnostic::error(
                DS4003,
                format!("a value in column `{}` is too large", out_of_range.column),
            )
            .primary(span, "while reading this source")
            .note(format!(
                "the file has {}, but the largest `long` is {}",
                out_of_range.value,
                i64::MAX
            ));
        }
        if let Some(problem) = read_problem(error) {
            let mut d = match self.scans.as_slice() {
                [(span, source)] => Diagnostic::error(DS3008, format!("cannot read `{source}`"))
                    .primary(*span, problem.label),
                // Several sources: DataFusion does not say which one failed.
                _ => Diagnostic::error(DS3008, "cannot read one of the query's sources").primary(
                    fallback,
                    "a file read by this query has data that is not valid",
                ),
            }
            .note(format!("DataFusion said: {}", strip(error)));
            if let Some(help) = problem.help {
                d = d.help(help);
            }
            return d;
        }
        Diagnostic::error(DS4002, "the query failed while running")
            .primary(fallback, "this step failed")
            .note(format!("DataFusion said: {}", strip(error)))
    }
}

fn strip(error: &DataFusionError) -> String {
    error.strip_backtrace()
}

/// A problem with the data in a file: the label for the scan and an optional help.
struct ReadProblem {
    label: &'static str,
    help: Option<&'static str>,
}

/// What is wrong with the data, if `error` is a problem reading a file.
fn read_problem(error: &DataFusionError) -> Option<ReadProblem> {
    let mut found = None;
    walk(error, &mut |e| {
        if found.is_some() {
            return;
        }
        if let Some(json_err) = e.downcast_ref::<JsonReadError>() {
            found = Some(match &json_err.kind {
                JsonReadErrorKind::Syntax { .. } | JsonReadErrorKind::NonObject { .. } => {
                    ReadProblem {
                        label: "a value in this source is not valid JSON",
                        help: None,
                    }
                }
                JsonReadErrorKind::TypeMismatch { .. } => ReadProblem {
                    label: "a value in this source does not match its column's type",
                    help: Some(
                        "column types are inferred from the first rows of each file; \
                         `read_json(..., infer_rows = n)` reads more rows to infer them",
                    ),
                },
            });
        }
        if let Some(arrow) = e.downcast_ref::<ArrowError>() {
            found = match arrow {
                ArrowError::CsvError(_) => Some(ReadProblem {
                    label: "a row of this source is not valid CSV",
                    help: None,
                }),
                ArrowError::JsonError(_) => Some(ReadProblem {
                    label: "a value in this source is not valid JSON",
                    help: None,
                }),
                ArrowError::ParseError(_) => Some(ReadProblem {
                    label: "a value in this source does not match its column's type",
                    help: Some(
                        "column types are inferred from the first rows of each file; \
                         `read_csv(..., infer_rows = n)` reads more rows to infer them",
                    ),
                }),
                _ => None,
            };
        }
        if let Some(DataFusionError::ParquetError(_)) = e.downcast_ref::<DataFusionError>() {
            found = Some(ReadProblem {
                label: "the Parquet data in this source could not be read",
                help: None,
            });
        }
    });
    found
}

/// The first `T` in `error`'s chain. DataFusion and Arrow wrap external errors at several levels
/// (`External`, `Shared`, `ArrowError::ExternalError`, `Context`), so this walks all of them.
pub(crate) fn find_cause<T: Error + 'static>(error: &DataFusionError) -> Option<&T> {
    let mut found: Option<&T> = None;
    walk(error, &mut |e| {
        if found.is_none() {
            found = e.downcast_ref::<T>();
        }
    });
    found
}

fn walk<'a>(error: &'a (dyn Error + 'static), visit: &mut dyn FnMut(&'a (dyn Error + 'static))) {
    visit(error);
    if let Some(df) = error.downcast_ref::<DataFusionError>() {
        match df {
            DataFusionError::External(inner) => return walk(inner.as_ref(), visit),
            DataFusionError::Shared(inner) => return walk(inner.as_ref(), visit),
            DataFusionError::ArrowError(inner, _) => return walk(inner.as_ref(), visit),
            DataFusionError::Context(_, inner) => return walk(inner.as_ref(), visit),
            DataFusionError::Collection(errors) => {
                for e in errors {
                    walk(e, visit);
                }
                return;
            }
            _ => {}
        }
    }
    if let Some(ArrowError::ExternalError(inner)) = error.downcast_ref::<ArrowError>() {
        return walk(inner.as_ref(), visit);
    }
    if let Some(source) = error.source() {
        walk(source, visit);
    }
}
