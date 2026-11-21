//! Collecting diagnostics from the compiler stages.

use std::collections::{BTreeMap, HashSet};

use crate::diagnostic::{Diagnostic, Severity};
use crate::span::FileId;

/// At most this many errors are reported per file (DIAGNOSTICS.md, "Severity").
pub const MAX_ERRORS_PER_FILE: usize = 50;

/// Something compiler stages report diagnostics to.
pub trait DiagnosticSink {
    /// Reports one diagnostic.
    fn emit(&mut self, diagnostic: Diagnostic);
}

impl DiagnosticSink for Vec<Diagnostic> {
    fn emit(&mut self, diagnostic: Diagnostic) {
        self.push(diagnostic);
    }
}

/// The diagnostics of one compilation.
///
/// Stages [`emit`](DiagnosticSink::emit) in any order; [`finish`](Diagnostics::finish) returns
/// them sorted, de-duplicated and capped.
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// An empty collection.
    pub fn new() -> Self {
        Diagnostics::default()
    }

    /// Number of diagnostics emitted so far, before de-duplication.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing was emitted.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether any error was emitted. Warnings and infos do not stop a program.
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    /// The diagnostics in the order they were emitted.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    /// Sorts, de-duplicates and caps the diagnostics.
    ///
    /// - **Order**: by file, then primary span start and end, then severity (errors first), code
    ///   and message. Diagnostics without a primary span come after those with one, in emission
    ///   order among themselves.
    /// - **De-duplication**: diagnostics with the same severity, code, message and primary label
    ///   are reported once (the first one emitted is kept).
    /// - **Cap**: after [`MAX_ERRORS_PER_FILE`] errors in one file, later errors are dropped and
    ///   the last error kept gets a note saying how many were omitted. Warnings and infos are
    ///   never dropped.
    pub fn finish(self) -> Vec<Diagnostic> {
        let mut items = self.items;
        items.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        let mut seen = HashSet::new();
        items.retain(|d| {
            seen.insert((
                d.severity,
                d.code,
                d.message.clone(),
                d.primary.clone(),
                d.file_id(),
            ))
        });

        let mut kept = Vec::with_capacity(items.len());
        let mut errors: BTreeMap<Option<FileId>, (usize, usize)> = BTreeMap::new();
        for diagnostic in items {
            if diagnostic.is_error() {
                let (count, last_kept) = errors.entry(diagnostic.file_id()).or_insert((0, 0));
                *count += 1;
                if *count > MAX_ERRORS_PER_FILE {
                    continue;
                }
                *last_kept = kept.len();
            }
            kept.push(diagnostic);
        }
        for (file, (count, last_kept)) in errors {
            if count > MAX_ERRORS_PER_FILE {
                let omitted = count - MAX_ERRORS_PER_FILE;
                let (noun, verb) = if omitted == 1 {
                    ("error", "was")
                } else {
                    ("errors", "were")
                };
                let place = if file.is_some() { " in this file" } else { "" };
                kept[last_kept]
                    .notes
                    .push(format!("{omitted} more {noun}{place} {verb} not shown"));
            }
        }
        kept
    }
}

type SortKey<'a> = (
    bool,
    Option<FileId>,
    u32,
    u32,
    Severity,
    crate::Code,
    &'a str,
);

fn sort_key(d: &Diagnostic) -> SortKey<'_> {
    match &d.primary {
        Some(label) => (
            false,
            Some(label.span.file),
            label.span.start,
            label.span.end,
            d.severity,
            d.code,
            &d.message,
        ),
        // Location-less diagnostics keep their emission order: the sort is stable.
        None => (true, d.file, 0, 0, Severity::Error, crate::Code::new(0), ""),
    }
}

impl DiagnosticSink for Diagnostics {
    fn emit(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<T: IntoIterator<Item = Diagnostic>>(&mut self, iter: T) {
        self.items.extend(iter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Code, Span};

    const F: FileId = FileId(0);

    fn err(start: u32, msg: &str) -> Diagnostic {
        Diagnostic::error(Code::new(1001), msg).primary(Span::new(F, start, start + 1), "")
    }

    #[test]
    fn sorted_by_position_then_severity() {
        let mut sink = Diagnostics::new();
        sink.emit(Diagnostic::warning(Code::new(4), "no location"));
        sink.emit(err(10, "b"));
        sink.emit(Diagnostic::warning(Code::new(2101), "w").primary(Span::new(F, 2, 3), ""));
        sink.emit(err(2, "a"));
        assert!(sink.has_errors());
        let messages: Vec<_> = sink.finish().into_iter().map(|d| d.message).collect();
        assert_eq!(messages, ["a", "w", "b", "no location"]);
    }

    #[test]
    fn duplicates_are_removed() {
        let mut sink = Diagnostics::new();
        sink.emit(err(1, "same"));
        sink.emit(err(5, "other"));
        sink.emit(err(1, "same"));
        sink.emit(Diagnostic::warning(Code::new(4), "none"));
        sink.emit(Diagnostic::warning(Code::new(4), "none"));
        assert_eq!(sink.len(), 5);
        assert_eq!(sink.finish().len(), 3);
    }

    #[test]
    fn errors_are_capped_per_file() {
        let mut sink = Diagnostics::new();
        for i in 0..53 {
            sink.emit(err(i * 2, &format!("e{i}")));
        }
        sink.emit(Diagnostic::warning(Code::new(2101), "w").primary(Span::new(F, 200, 201), ""));
        sink.emit(err(0, "other file").primary(Span::new(FileId(1), 0, 1), ""));
        let out = sink.finish();
        let file0_errors: Vec<_> = out
            .iter()
            .filter(|d| d.is_error() && d.file_id() == Some(F))
            .collect();
        assert_eq!(file0_errors.len(), MAX_ERRORS_PER_FILE);
        assert_eq!(
            file0_errors.last().unwrap().notes,
            ["3 more errors in this file were not shown"]
        );
        assert!(out.iter().any(|d| d.message == "w"));
        assert!(out.iter().any(|d| d.file_id() == Some(FileId(1))));
    }

    #[test]
    fn one_omitted_error_is_singular() {
        let mut sink = Diagnostics::new();
        for i in 0..=MAX_ERRORS_PER_FILE as u32 {
            sink.emit(err(i, "e"));
        }
        let out = sink.finish();
        assert_eq!(
            out.last().unwrap().notes,
            ["1 more error in this file was not shown"]
        );
    }
}
