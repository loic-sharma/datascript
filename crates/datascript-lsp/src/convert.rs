//! `datascript_diagnostics::Diagnostic` → `ls_types::Diagnostic`.

use datascript_diagnostics::{Diagnostic, Severity, SourceFile, Span, codes};
use tower_lsp_server::ls_types::{
    self as lsp, CodeDescription, DiagnosticRelatedInformation, DiagnosticSeverity, Location,
    NumberOrString, Position, Range, Uri,
};

/// The `source` of every diagnostic the server publishes.
pub const SOURCE: &str = "datascript";

/// Converts one diagnostic of `file`, the text of the document at `uri`.
///
/// - Positions count UTF-16 code units (D-025), and lines end where the LSP ends them.
/// - The message is the diagnostic's message followed by its help and note lines, worded as the
///   terminal shows them, because editors show only the message in their hover.
/// - The primary label (when it says something) and the secondary labels become related
///   information, so their text and location are shown and can be clicked.
/// - The code links to its documentation ([`codes::docs_url`]).
/// - A diagnostic without a location (`DS0004`) sits at the start of the document.
pub fn to_lsp(diagnostic: &Diagnostic, file: &SourceFile, uri: &Uri) -> lsp::Diagnostic {
    let range = diagnostic
        .primary
        .as_ref()
        .map_or_else(Range::default, |label| range_of(file, label.span));
    let labels = diagnostic
        .primary
        .iter()
        .filter(|label| !label.message.is_empty())
        .chain(&diagnostic.secondary);
    let related: Vec<_> = labels
        .map(|label| DiagnosticRelatedInformation {
            location: Location::new(uri.clone(), range_of(file, label.span)),
            message: if label.message.is_empty() {
                "related to this".into()
            } else {
                label.message.clone()
            },
        })
        .collect();
    let mut message = diagnostic.message.clone();
    for help in &diagnostic.help {
        message.push_str("\nhelp: ");
        message.push_str(&help.message);
    }
    for note in &diagnostic.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    let href = codes::docs_url(diagnostic.code).parse().ok();
    lsp::Diagnostic {
        range,
        severity: Some(severity(diagnostic.severity)),
        code: Some(NumberOrString::String(diagnostic.code.to_string())),
        code_description: href.map(|href| CodeDescription { href }),
        source: Some(SOURCE.into()),
        message,
        related_information: (!related.is_empty()).then_some(related),
        tags: None,
        data: None,
    }
}

/// Converts every diagnostic of `file`.
pub fn to_lsp_all(
    diagnostics: &[Diagnostic],
    file: &SourceFile,
    uri: &Uri,
) -> Vec<lsp::Diagnostic> {
    diagnostics.iter().map(|d| to_lsp(d, file, uri)).collect()
}

fn severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
    }
}

/// The LSP range of a span. The span's file is not checked: the server checks one document at a
/// time, so every span points into it.
pub fn range_of(file: &SourceFile, span: Span) -> Range {
    Range {
        start: position_of(file, span.start),
        end: position_of(file, span.end),
    }
}

fn position_of(file: &SourceFile, offset: u32) -> Position {
    let at = file.utf16_line_col(offset);
    Position {
        line: at.line - 1,
        character: at.column - 1,
    }
}

#[cfg(test)]
mod tests {
    use datascript_diagnostics::{FileId, Suggestion, codes};

    use super::*;

    fn uri() -> Uri {
        "file:///w/q.ds".parse().unwrap()
    }

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn full_conversion() {
        let text = "from 'é😀.csv'\r\n| where price + 1";
        let file = SourceFile::new("q.ds", text);
        let f = FileId(0);
        let price = text.find("price").unwrap() as u32;
        let one = text.rfind('1').unwrap() as u32;
        let d = Diagnostic::error(codes::DS2003, "cannot add `string` and `long`")
            .primary(Span::new(f, price, price + 5), "this is `string`")
            .secondary(Span::new(f, one, one + 1), "this is `long`")
            .help_with(
                "convert it first: `toreal(price) + 1`",
                Suggestion::replace(Span::new(f, price, price + 5), "toreal(price)"),
            )
            .note("columns available at this point: price");
        let lsp = to_lsp(&d, &file, &uri());
        assert_eq!(lsp.range.start, pos(1, 8));
        assert_eq!(lsp.range.end, pos(1, 13));
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(lsp.code, Some(NumberOrString::String("DS2003".into())));
        assert_eq!(lsp.source.as_deref(), Some("datascript"));
        assert_eq!(
            lsp.code_description.unwrap().href.as_str(),
            "https://github.com/loic-sharma/datascript/blob/main/docs/DIAGNOSTICS.md#code-registry"
        );
        assert_eq!(
            lsp.message,
            "cannot add `string` and `long`\n\
             help: convert it first: `toreal(price) + 1`\n\
             note: columns available at this point: price"
        );
        let related = lsp.related_information.unwrap();
        assert_eq!(related.len(), 2);
        assert_eq!(related[0].message, "this is `string`");
        assert_eq!(related[1].message, "this is `long`");
        assert_eq!(related[1].location.uri, uri());
        assert_eq!(related[1].location.range.start, pos(1, 16));
    }

    #[test]
    fn utf16_columns_on_the_first_line() {
        let text = "from 'é😀.csv' x";
        let file = SourceFile::new("q.ds", text);
        let x = text.rfind('x').unwrap() as u32;
        let d = Diagnostic::error(codes::DS0101, "expected `|`, found `x`")
            .primary(Span::new(FileId(0), x, x + 1), "");
        let lsp = to_lsp(&d, &file, &uri());
        // `é` is one UTF-16 unit and `😀` two: `x` is at character 15, not 14 or 18.
        assert_eq!(lsp.range.start, pos(0, 15));
        assert_eq!(lsp.related_information, None);
    }

    #[test]
    fn location_less_and_info_diagnostics() {
        let file = SourceFile::new("q.ds", "let a = 1;");
        let d = Diagnostic::warning(codes::DS0004, "this program has no query to run")
            .in_file(FileId(0));
        let lsp = to_lsp(&d, &file, &uri());
        assert_eq!(lsp.range, Range::default());
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::WARNING));
        let info = Diagnostic::info(codes::DS1007, "renamed");
        assert_eq!(
            to_lsp(&info, &file, &uri()).severity,
            Some(DiagnosticSeverity::INFORMATION)
        );
    }
}
