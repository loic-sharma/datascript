//! The diagnostic model.

use std::fmt;
use std::str::FromStr;

use crate::span::{FileId, Span};

/// A stable diagnostic code such as `DS1001`. Constants live in [`codes`](crate::codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code(u16);

impl Code {
    /// Creates a code from its number: `Code::new(1001)` is `DS1001`.
    ///
    /// # Panics
    ///
    /// If `number` has more than four digits.
    pub const fn new(number: u16) -> Self {
        assert!(number <= 9999, "diagnostic codes have four digits");
        Code(number)
    }

    /// The number without the `DS` prefix.
    pub const fn number(self) -> u16 {
        self.0
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DS{:04}", self.0)
    }
}

/// The text was not of the form `DS` followed by four digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidCode;

impl fmt::Display for InvalidCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a diagnostic code is `DS` followed by four digits")
    }
}

impl std::error::Error for InvalidCode {}

impl FromStr for Code {
    type Err = InvalidCode;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let digits = s.strip_prefix("DS").ok_or(InvalidCode)?;
        if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(InvalidCode);
        }
        digits.parse().map(Code).map_err(|_| InvalidCode)
    }
}

/// How serious a diagnostic is. See DIAGNOSTICS.md, "Severity".
///
/// Ordered from most to least severe, so sorting puts errors first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The program cannot run.
    Error,
    /// The program runs, but probably not as intended.
    Warning,
    /// A neutral fact.
    Info,
}

impl Severity {
    /// The lowercase name used in rendered output: `error`, `warning` or `info`.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A span with a short message, shown under the source line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label {
    /// What the label points at.
    pub span: Span,
    /// Short text that adds information; may be empty.
    pub message: String,
}

impl Label {
    /// Creates a label.
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Label {
            span,
            message: message.into(),
        }
    }
}

/// How safe it is to apply a [`Suggestion`] without a human looking at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Applicability {
    /// Certainly what the user meant; tools may apply it automatically.
    MachineApplicable,
    /// Probably right, but the user should check.
    MaybeIncorrect,
    /// Contains placeholders the user must fill in.
    HasPlaceholders,
}

impl Applicability {
    /// The kebab-case name used in the JSON form.
    pub fn as_str(self) -> &'static str {
        match self {
            Applicability::MachineApplicable => "machine-applicable",
            Applicability::MaybeIncorrect => "maybe-incorrect",
            Applicability::HasPlaceholders => "has-placeholders",
        }
    }
}

/// Replace the text in `span` with `replacement`. An empty span inserts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextEdit {
    /// The text to replace.
    pub span: Span,
    /// The new text.
    pub replacement: String,
}

impl TextEdit {
    /// Creates an edit.
    pub fn new(span: Span, replacement: impl Into<String>) -> Self {
        TextEdit {
            span,
            replacement: replacement.into(),
        }
    }
}

/// The edits of a suggestion overlap, so applying them is ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlappingEdits {
    /// The first of the two overlapping edits, in source order.
    pub first: Span,
    /// The second one.
    pub second: Span,
}

impl fmt::Display for OverlappingEdits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "suggestion edits overlap: {}..{} and {}..{}",
            self.first.start, self.first.end, self.second.start, self.second.end
        )
    }
}

impl std::error::Error for OverlappingEdits {}

/// A fix that tools (the LSP, `ds fix`) can apply: a set of non-overlapping edits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Suggestion {
    /// The edits, sorted by position and non-overlapping.
    pub edits: Vec<TextEdit>,
    /// How safe it is to apply.
    pub applicability: Applicability,
}

impl Suggestion {
    /// Creates a suggestion, sorting the edits by position.
    ///
    /// Two insertions at the same offset, or an insertion at the boundary of a replacement, do
    /// not overlap. Edits in different files never overlap.
    pub fn try_new(
        mut edits: Vec<TextEdit>,
        applicability: Applicability,
    ) -> Result<Self, OverlappingEdits> {
        edits.sort_by_key(|edit| (edit.span.file, edit.span.start, edit.span.end));
        for pair in edits.windows(2) {
            let (a, b) = (pair[0].span, pair[1].span);
            if a.file == b.file && b.start < a.end {
                return Err(OverlappingEdits {
                    first: a,
                    second: b,
                });
            }
        }
        Ok(Suggestion {
            edits,
            applicability,
        })
    }

    /// Like [`try_new`](Self::try_new).
    ///
    /// # Panics
    ///
    /// If edits overlap. That is a bug in the code building the suggestion.
    pub fn new(edits: Vec<TextEdit>, applicability: Applicability) -> Self {
        match Suggestion::try_new(edits, applicability) {
            Ok(suggestion) => suggestion,
            Err(err) => panic!("{err}"),
        }
    }

    /// A single machine-applicable replacement.
    pub fn replace(span: Span, replacement: impl Into<String>) -> Self {
        Suggestion::new(
            vec![TextEdit::new(span, replacement)],
            Applicability::MachineApplicable,
        )
    }
}

/// A "help:" line telling the user what to do, optionally with a fix tools can apply.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Help {
    /// What to do, in words. Spell out the rewrite here: text renderers show only this.
    pub message: String,
    /// The fix, if it can be expressed as edits.
    pub suggestion: Option<Suggestion>,
}

/// One problem found in a program. See DIAGNOSTICS.md for the style rules.
///
/// Build one with [`Diagnostic::error`], [`Diagnostic::warning`] or [`Diagnostic::info`]:
///
/// ```
/// use datascript_diagnostics::{codes, Diagnostic, FileId, Span};
///
/// let span = Span::new(FileId(0), 26, 31);
/// let diagnostic = Diagnostic::error(codes::DS1001, "unknown column `amout`")
///     .primary(span, "no column named `amout` here")
///     .help("a column with a similar name exists: `amount`");
/// assert!(diagnostic.is_error());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    /// The stable code, e.g. `DS1001`.
    pub code: Code,
    /// How serious it is.
    pub severity: Severity,
    /// One line, lowercase start, no trailing period.
    pub message: String,
    /// The smallest span that explains the problem. `None` only when there is truly no location
    /// (`DS0004`, untraceable engine errors).
    pub primary: Option<Label>,
    /// Spans that show why, e.g. where a type came from.
    pub secondary: Vec<Label>,
    /// The file a location-less diagnostic belongs to. Ignored when `primary` is set.
    pub file: Option<FileId>,
    /// "note:" lines giving context.
    pub notes: Vec<String>,
    /// "help:" lines saying what to do.
    pub help: Vec<Help>,
}

impl Diagnostic {
    /// Creates a diagnostic with no labels, notes or help.
    pub fn new(severity: Severity, code: Code, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity,
            message: message.into(),
            primary: None,
            secondary: Vec::new(),
            file: None,
            notes: Vec::new(),
            help: Vec::new(),
        }
    }

    /// Creates an error.
    pub fn error(code: Code, message: impl Into<String>) -> Self {
        Diagnostic::new(Severity::Error, code, message)
    }

    /// Creates a warning.
    pub fn warning(code: Code, message: impl Into<String>) -> Self {
        Diagnostic::new(Severity::Warning, code, message)
    }

    /// Creates an info diagnostic.
    pub fn info(code: Code, message: impl Into<String>) -> Self {
        Diagnostic::new(Severity::Info, code, message)
    }

    /// Sets the primary label.
    pub fn primary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.primary = Some(Label::new(span, message));
        self
    }

    /// Adds a secondary label.
    pub fn secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.secondary.push(Label::new(span, message));
        self
    }

    /// Sets the file of a diagnostic that has no primary span, so renderers can name it.
    pub fn in_file(mut self, file: FileId) -> Self {
        self.file = Some(file);
        self
    }

    /// Adds a note.
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Adds a help line without a suggestion.
    pub fn help(mut self, message: impl Into<String>) -> Self {
        self.help.push(Help {
            message: message.into(),
            suggestion: None,
        });
        self
    }

    /// Adds a help line with a suggestion tools can apply.
    pub fn help_with(mut self, message: impl Into<String>, suggestion: Suggestion) -> Self {
        self.help.push(Help {
            message: message.into(),
            suggestion: Some(suggestion),
        });
        self
    }

    /// Whether this is an error.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// The file this diagnostic is about: the primary span's file, else [`Diagnostic::file`].
    pub fn file_id(&self) -> Option<FileId> {
        self.primary
            .as_ref()
            .map(|label| label.span.file)
            .or(self.file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_display_and_parse() {
        assert_eq!(Code::new(4).to_string(), "DS0004");
        assert_eq!("DS1001".parse(), Ok(Code::new(1001)));
        assert_eq!("DS101".parse::<Code>(), Err(InvalidCode));
        assert_eq!("XX1001".parse::<Code>(), Err(InvalidCode));
        assert_eq!("DS+101".parse::<Code>(), Err(InvalidCode));
    }

    #[test]
    fn suggestion_edits_are_sorted_and_checked() {
        let f = FileId(0);
        let s = Suggestion::try_new(
            vec![
                TextEdit::new(Span::new(f, 5, 6), "b"),
                TextEdit::new(Span::new(f, 1, 2), "a"),
                TextEdit::new(Span::empty(f, 2), "insert"),
            ],
            Applicability::MaybeIncorrect,
        )
        .unwrap();
        let starts: Vec<u32> = s.edits.iter().map(|e| e.span.start).collect();
        assert_eq!(starts, [1, 2, 5]);

        let err = Suggestion::try_new(
            vec![
                TextEdit::new(Span::new(f, 1, 4), "a"),
                TextEdit::new(Span::new(f, 3, 6), "b"),
            ],
            Applicability::MachineApplicable,
        )
        .unwrap_err();
        assert_eq!(err.first, Span::new(f, 1, 4));
    }

    #[test]
    #[should_panic(expected = "suggestion edits overlap")]
    fn overlapping_suggestion_panics() {
        let f = FileId(0);
        Suggestion::new(
            vec![
                TextEdit::new(Span::new(f, 1, 4), "a"),
                TextEdit::new(Span::new(f, 2, 3), "b"),
            ],
            Applicability::MachineApplicable,
        );
    }

    #[test]
    fn file_id_prefers_primary_span() {
        let d = Diagnostic::warning(Code::new(4), "x").in_file(FileId(2));
        assert_eq!(d.file_id(), Some(FileId(2)));
        let d = d.primary(Span::empty(FileId(1), 0), "");
        assert_eq!(d.file_id(), Some(FileId(1)));
    }
}
