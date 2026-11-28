//! The JSON form of diagnostics, for the web UI, the WASM API and `ds check --format json`.
//!
//! # Schema (version 1)
//!
//! ```text
//! Report = {
//!   "version": 1,                    // bumped on any breaking change; consumers check it
//!   "diagnostics": [Diagnostic]      // in the order given (normally `Diagnostics::finish` order)
//! }
//! Diagnostic = {
//!   "code": "DS1001",                // `DS` + four digits
//!   "severity": "error" | "warning" | "info",
//!   "message": string,
//!   "file": string | null,           // file name; set even when "primary" is null if known
//!   "primary": Label | null,         // null only for location-less diagnostics (DS0004)
//!   "secondary": [Label],
//!   "help": [Help],
//!   "notes": [string]
//! }
//! Label = { "span": Span, "message": string }   // message may be ""
//! Help = { "message": string, "suggestion": Suggestion | null }
//! Suggestion = {
//!   "applicability": "machine-applicable" | "maybe-incorrect" | "has-placeholders",
//!   "edits": [{ "span": Span, "replacement": string }]   // sorted, non-overlapping
//! }
//! Span = {
//!   "file": string,
//!   "start": number, "end": number,          // UTF-16 offsets from the start of the file
//!   "line": number, "column": number,        // 1-based start position
//!   "endLine": number, "endColumn": number   // 1-based end position (exclusive)
//! }
//! ```
//!
//! All positions count **UTF-16 code units** (D-025), so JavaScript can index its strings with
//! them directly. Byte offsets are not part of the JSON form. A line ends at `\n`, `\r\n` or a lone
//! `\r`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::diagnostic::{
    Applicability, Code, Diagnostic, Help, Label, OverlappingEdits, Severity, Suggestion, TextEdit,
};
use crate::source::{LineCol, SourceMap};
use crate::span::Span;

/// The version of the JSON form this crate writes and reads.
pub const JSON_VERSION: u32 = 1;

/// Name used for a span whose file is missing from the source map. That is a bug in the caller.
const UNKNOWN_FILE: &str = "<unknown file>";

/// A list of diagnostics in the JSON form. See the [module docs](self) for the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonReport {
    /// Always [`JSON_VERSION`] when written by this crate.
    pub version: u32,
    /// The diagnostics.
    pub diagnostics: Vec<JsonDiagnostic>,
}

/// One diagnostic in the JSON form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonDiagnostic {
    /// E.g. `DS1001`.
    pub code: String,
    /// How serious it is.
    pub severity: Severity,
    /// The message.
    pub message: String,
    /// The file the diagnostic is about.
    pub file: Option<String>,
    /// The primary label.
    pub primary: Option<JsonLabel>,
    /// Secondary labels.
    pub secondary: Vec<JsonLabel>,
    /// Help lines.
    pub help: Vec<JsonHelp>,
    /// Notes.
    pub notes: Vec<String>,
}

/// A label in the JSON form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonLabel {
    /// Where it points.
    pub span: JsonSpan,
    /// Its text, possibly empty.
    pub message: String,
}

/// A help line in the JSON form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonHelp {
    /// What to do.
    pub message: String,
    /// A fix tools can apply.
    pub suggestion: Option<JsonSuggestion>,
}

/// A suggestion in the JSON form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonSuggestion {
    /// How safe it is to apply.
    pub applicability: Applicability,
    /// The edits, sorted and non-overlapping.
    pub edits: Vec<JsonTextEdit>,
}

/// A text edit in the JSON form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonTextEdit {
    /// The text to replace.
    pub span: JsonSpan,
    /// The new text.
    pub replacement: String,
}

/// A span in the JSON form. Every number counts UTF-16 code units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonSpan {
    /// File name.
    pub file: String,
    /// Start offset from the start of the file.
    pub start: u32,
    /// End offset (exclusive).
    pub end: u32,
    /// 1-based line of `start`.
    pub line: u32,
    /// 1-based column of `start`.
    pub column: u32,
    /// 1-based line of `end`.
    pub end_line: u32,
    /// 1-based column of `end`.
    pub end_column: u32,
}

/// Why a [`JsonReport`] could not be turned back into [`Diagnostic`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// The report has a version this crate does not read.
    UnsupportedVersion(u32),
    /// A code is not `DS` followed by four digits.
    InvalidCode(String),
    /// A span names a file that is not in the source map.
    UnknownFile(String),
    /// A UTF-16 offset is past the end of its file or inside a surrogate pair.
    InvalidOffset {
        /// The file name.
        file: String,
        /// The offset.
        offset: u32,
    },
    /// The edits of a suggestion overlap.
    OverlappingEdits(OverlappingEdits),
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonError::UnsupportedVersion(v) => write!(
                f,
                "diagnostics JSON version {v} is not supported (expected {JSON_VERSION})"
            ),
            JsonError::InvalidCode(code) => write!(f, "`{code}` is not a diagnostic code"),
            JsonError::UnknownFile(file) => write!(f, "no source file named `{file}`"),
            JsonError::InvalidOffset { file, offset } => {
                write!(
                    f,
                    "UTF-16 offset {offset} is not a valid position in `{file}`"
                )
            }
            JsonError::OverlappingEdits(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for JsonError {}

impl JsonReport {
    /// Converts diagnostics to the JSON form, computing UTF-16 positions from `sources`.
    pub fn new(diagnostics: &[Diagnostic], sources: &SourceMap) -> Self {
        JsonReport {
            version: JSON_VERSION,
            diagnostics: diagnostics
                .iter()
                .map(|d| JsonDiagnostic::new(d, sources))
                .collect(),
        }
    }

    /// Converts back to diagnostics, mapping UTF-16 positions to bytes with `sources`.
    ///
    /// Files are matched by name. Line and column fields are not read; offsets are authoritative.
    pub fn to_diagnostics(&self, sources: &SourceMap) -> Result<Vec<Diagnostic>, JsonError> {
        if self.version != JSON_VERSION {
            return Err(JsonError::UnsupportedVersion(self.version));
        }
        self.diagnostics
            .iter()
            .map(|d| d.to_diagnostic(sources))
            .collect()
    }

    /// Pretty-printed JSON. Key order and formatting are stable.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("diagnostics always serialize")
    }
}

/// Shorthand for `JsonReport::new(diagnostics, sources).to_json_pretty()`.
pub fn render_json(diagnostics: &[Diagnostic], sources: &SourceMap) -> String {
    JsonReport::new(diagnostics, sources).to_json_pretty()
}

impl JsonDiagnostic {
    fn new(d: &Diagnostic, sources: &SourceMap) -> Self {
        let label = |label: &Label| JsonLabel {
            span: JsonSpan::new(label.span, sources),
            message: label.message.clone(),
        };
        JsonDiagnostic {
            code: d.code.to_string(),
            severity: d.severity,
            message: d.message.clone(),
            file: d.file_id().map(|file| {
                sources
                    .get(file)
                    .map_or(UNKNOWN_FILE, |f| f.name())
                    .to_string()
            }),
            primary: d.primary.as_ref().map(label),
            secondary: d.secondary.iter().map(label).collect(),
            help: d
                .help
                .iter()
                .map(|help| JsonHelp {
                    message: help.message.clone(),
                    suggestion: help.suggestion.as_ref().map(|s| JsonSuggestion {
                        applicability: s.applicability,
                        edits: s
                            .edits
                            .iter()
                            .map(|edit| JsonTextEdit {
                                span: JsonSpan::new(edit.span, sources),
                                replacement: edit.replacement.clone(),
                            })
                            .collect(),
                    }),
                })
                .collect(),
            notes: d.notes.clone(),
        }
    }

    fn to_diagnostic(&self, sources: &SourceMap) -> Result<Diagnostic, JsonError> {
        let code: Code = self
            .code
            .parse()
            .map_err(|_| JsonError::InvalidCode(self.code.clone()))?;
        let label = |label: &JsonLabel| -> Result<Label, JsonError> {
            Ok(Label::new(
                label.span.to_span(sources)?,
                label.message.clone(),
            ))
        };
        let mut help = Vec::with_capacity(self.help.len());
        for h in &self.help {
            let suggestion = match &h.suggestion {
                None => None,
                Some(s) => {
                    let edits = s
                        .edits
                        .iter()
                        .map(|e| {
                            Ok(TextEdit::new(
                                e.span.to_span(sources)?,
                                e.replacement.clone(),
                            ))
                        })
                        .collect::<Result<Vec<_>, JsonError>>()?;
                    Some(
                        Suggestion::try_new(edits, s.applicability)
                            .map_err(JsonError::OverlappingEdits)?,
                    )
                }
            };
            help.push(Help {
                message: h.message.clone(),
                suggestion,
            });
        }
        let primary = self.primary.as_ref().map(label).transpose()?;
        let file = match (&primary, &self.file) {
            (None, Some(name)) => Some(
                sources
                    .find(name)
                    .ok_or_else(|| JsonError::UnknownFile(name.clone()))?,
            ),
            _ => None,
        };
        Ok(Diagnostic {
            code,
            severity: self.severity,
            message: self.message.clone(),
            primary,
            secondary: self.secondary.iter().map(label).collect::<Result<_, _>>()?,
            file,
            notes: self.notes.clone(),
            help,
        })
    }
}

impl JsonSpan {
    /// Converts a byte [`Span`] to a UTF-16 [`JsonSpan`] using `sources` (D-025).
    pub fn new(span: Span, sources: &SourceMap) -> Self {
        let Some(file) = sources.get(span.file) else {
            return JsonSpan {
                file: UNKNOWN_FILE.to_string(),
                start: 0,
                end: 0,
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            };
        };
        let LineCol { line, column } = file.utf16_line_col(span.start);
        let LineCol {
            line: end_line,
            column: end_column,
        } = file.utf16_line_col(span.end);
        JsonSpan {
            file: file.name().to_string(),
            start: file.utf16_offset(span.start),
            end: file.utf16_offset(span.end),
            line,
            column,
            end_line,
            end_column,
        }
    }

    fn to_span(&self, sources: &SourceMap) -> Result<Span, JsonError> {
        let id = sources
            .find(&self.file)
            .ok_or_else(|| JsonError::UnknownFile(self.file.clone()))?;
        let file = sources.get(id).expect("found by name");
        let byte = |offset: u32| {
            file.byte_offset_from_utf16(offset)
                .ok_or_else(|| JsonError::InvalidOffset {
                    file: self.file.clone(),
                    offset,
                })
        };
        Ok(Span::new(id, byte(self.start)?, byte(self.end)?))
    }
}
