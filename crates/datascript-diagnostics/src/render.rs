//! Terminal and plain text renderers.
//!
//! Both produce the rustc-style snippet layout shown in DIAGNOSTICS.md and share all layout code;
//! they differ only in the characters used and in color. The plain renderer is ASCII-only and is
//! the format of golden tests (TESTING.md), so its output must stay byte-stable.
//!
//! - Columns in the location line count characters; a tab counts as one.
//! - In the snippet, a tab is shown as four spaces and markers line up with that. Help and notes
//!   show tabs the same way.
//! - A source line wider than [`MAX_LINE_WIDTH`] columns is cut to a window around its labels,
//!   with an ellipsis where text was left out.
//! - Output lines never end with whitespace.
//! - Help is printed before notes. Only the help text is printed: a [`Suggestion`] is for tools
//!   (JSON, LSP), so the help message itself must spell out the rewrite (style rule 4).
//!
//! [`Suggestion`]: crate::Suggestion

use std::fmt::Write as _;

use crate::diagnostic::{Diagnostic, Label, Severity};
use crate::source::{SourceFile, SourceMap};
use crate::span::FileId;

const TAB: &str = "    ";

/// Source lines up to this many columns are shown whole.
const MAX_LINE_WIDTH: usize = 120;
/// How many columns a cut line shows, at least.
const WINDOW: usize = 100;
/// How many columns of context a cut line keeps before its first label.
const CONTEXT: usize = 40;

/// Renders diagnostics as text, for terminals and golden tests.
#[derive(Debug, Clone, Copy)]
pub struct Renderer {
    chars: &'static Chars,
    color: bool,
}

#[derive(Debug)]
struct Chars {
    location: &'static str,
    border: char,
    connector: char,
    ellipsis: &'static str,
    /// Marks text left out of a long source line.
    cut: &'static str,
    primary: char,
    secondary: char,
}

const UNICODE: Chars = Chars {
    location: "┌─",
    border: '│',
    connector: '│',
    ellipsis: "·",
    cut: "…",
    primary: '^',
    secondary: '-',
};

const ASCII: Chars = Chars {
    location: "-->",
    border: '|',
    connector: '|',
    ellipsis: "...",
    cut: "...",
    primary: '^',
    secondary: '-',
};

impl Renderer {
    /// ASCII, no color. Used by golden tests and `--color never`.
    pub fn plain() -> Self {
        Renderer {
            chars: &ASCII,
            color: false,
        }
    }

    /// Unicode box drawing, with ANSI colors if `color` is true. Used by the CLI and REPL.
    pub fn terminal(color: bool) -> Self {
        Renderer {
            chars: &UNICODE,
            color,
        }
    }

    /// Renders one diagnostic. The result ends with a newline.
    pub fn render(&self, diagnostic: &Diagnostic, sources: &SourceMap) -> String {
        let mut out = Output::new(diagnostic.severity);
        self.render_into(&mut out, diagnostic, sources);
        out.finish(self.color)
    }

    /// Renders several diagnostics, separated by blank lines.
    pub fn render_all(&self, diagnostics: &[Diagnostic], sources: &SourceMap) -> String {
        let mut out = String::new();
        for (i, diagnostic) in diagnostics.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&self.render(diagnostic, sources));
        }
        out
    }

    fn render_into(&self, out: &mut Output, d: &Diagnostic, sources: &SourceMap) {
        let severity = Style::Severity(d.severity);
        let header = out.line();
        header.push(&format!("{}[{}]", d.severity, d.code), severity);
        header.push(": ", Style::Bold);
        header.push(&d.message, Style::Bold);

        let groups = group_labels(d);
        let snippets: Vec<Snippet<'_>> = groups
            .iter()
            .filter_map(|(file, labels)| {
                Some(Snippet::new(sources.get(*file)?, labels, self.chars))
            })
            .collect();
        let gutter = snippets
            .iter()
            .filter_map(Snippet::max_line_number)
            .max()
            .map_or(1, |n| n.to_string().len());
        let pad = " ".repeat(gutter + 1);

        let mut has_location = false;
        if snippets.is_empty() {
            // No primary span: show the file name only (style rule 11).
            if let Some(file) = d.file_id() {
                let name = sources.get(file).map_or("<unknown file>", SourceFile::name);
                let line = out.line();
                line.push(&pad, Style::Plain);
                line.push(self.chars.location, Style::Border);
                line.push(" ", Style::Plain);
                line.push(name, Style::Plain);
                has_location = true;
            }
        }
        for snippet in &snippets {
            snippet.render(out, &pad, gutter);
            has_location = true;
        }

        if has_location && (!d.help.is_empty() || !d.notes.is_empty()) {
            self.border_line(out, &pad);
        }
        let helps = d.help.iter().map(|h| ("help", h.message.as_str()));
        let notes = d.notes.iter().map(|n| ("note", n.as_str()));
        for (kind, message) in helps.chain(notes) {
            let indent = " ".repeat(pad.len() + kind.len() + 4);
            for (i, text) in message.lines().enumerate() {
                let line = out.line();
                if i == 0 {
                    line.push(&pad, Style::Plain);
                    line.push("= ", Style::Border);
                    line.push(kind, Style::Bold);
                    line.push(": ", Style::Plain);
                } else {
                    line.push(&indent, Style::Plain);
                }
                line.push(&text.replace('\t', TAB), Style::Plain);
            }
        }
    }

    fn border_line(&self, out: &mut Output, pad: &str) {
        let line = out.line();
        line.push(pad, Style::Plain);
        line.push_char(self.chars.border, Style::Border);
    }
}

/// Labels grouped by file: the primary label's file first, then the others by id.
fn group_labels(d: &Diagnostic) -> Vec<(FileId, Vec<(&Label, bool)>)> {
    let mut groups: Vec<(FileId, Vec<(&Label, bool)>)> = Vec::new();
    let labels = d
        .primary
        .iter()
        .map(|label| (label, true))
        .chain(d.secondary.iter().map(|label| (label, false)));
    for (label, primary) in labels {
        match groups.iter_mut().find(|(file, _)| *file == label.span.file) {
            Some((_, group)) => group.push((label, primary)),
            None => groups.push((label.span.file, vec![(label, primary)])),
        }
    }
    if let Some(((_, first), rest)) = groups.split_first_mut() {
        // Secondary-only groups (no primary label) are ordered by file id.
        if !first.iter().any(|(_, primary)| *primary) {
            groups.sort_by_key(|(file, _)| *file);
        } else {
            rest.sort_by_key(|(file, _)| *file);
        }
    }
    groups
}

/// The underline of one label on one line, in display columns (0-based, tabs expanded).
#[derive(Debug)]
struct Segment<'a> {
    start: usize,
    end: usize,
    primary: bool,
    message: Option<&'a str>,
}

struct Snippet<'a> {
    file: &'a SourceFile,
    chars: &'static Chars,
    /// Where the location line points: (line, column), 1-based, columns in characters.
    location: (u32, u32),
    /// 0-based line index → segments on that line.
    lines: Vec<(usize, Vec<Segment<'a>>)>,
}

impl<'a> Snippet<'a> {
    fn new(file: &'a SourceFile, labels: &[(&'a Label, bool)], chars: &'static Chars) -> Self {
        let mut lines: Vec<(usize, Vec<Segment<'a>>)> = Vec::new();
        let mut add =
            |line: usize, segment: Segment<'a>| match lines.iter_mut().find(|(l, _)| *l == line) {
                Some((_, segments)) => segments.push(segment),
                None => lines.push((line, vec![segment])),
            };

        for &(label, primary) in labels {
            let span = label.span;
            let start_line = file.line_index(span.start);
            let end_line = if span.is_empty() {
                start_line
            } else {
                file.line_index(span.end - 1)
            };
            let message = Some(label.message.as_str()).filter(|m| !m.is_empty());
            let start = display_column(file, start_line, span.start);
            if start_line == end_line {
                let end = display_column(file, start_line, span.end);
                add(
                    start_line,
                    Segment {
                        start,
                        end: end.max(start + 1),
                        primary,
                        message,
                    },
                );
            } else {
                // A multi-line span: underline the rest of the first line, and the last line up
                // to the end of the span, starting at its first non-blank character.
                let first_end = display_width(file.line_text(start_line));
                add(
                    start_line,
                    Segment {
                        start,
                        end: first_end.max(start + 1),
                        primary,
                        message: None,
                    },
                );
                let last_text = file.line_text(end_line);
                let indent =
                    display_width(&last_text[..last_text.len() - last_text.trim_start().len()]);
                let end = display_column(file, end_line, span.end);
                add(
                    end_line,
                    Segment {
                        start: indent.min(end),
                        end: end.max(indent.min(end) + 1),
                        primary,
                        message,
                    },
                );
            }
        }
        lines.sort_by_key(|(line, _)| *line);
        for (_, segments) in &mut lines {
            segments.sort_by_key(|s| (s.start, s.end));
        }

        let (first, _) = labels
            .iter()
            .find(|(_, primary)| *primary)
            .or_else(|| labels.iter().min_by_key(|(label, _)| label.span.start))
            .expect("a snippet has at least one label");
        let location = file.line_col(first.span.start);
        Snippet {
            file,
            chars,
            location: (location.line, location.column),
            lines,
        }
    }

    fn max_line_number(&self) -> Option<usize> {
        self.lines.last().map(|(line, _)| line + 1)
    }

    fn render(&self, out: &mut Output, pad: &str, gutter: usize) {
        let chars = self.chars;
        let line = out.line();
        line.push(pad, Style::Plain);
        line.push(chars.location, Style::Border);
        line.push(
            &format!(
                " {}:{}:{}",
                self.file.name(),
                self.location.0,
                self.location.1
            ),
            Style::Plain,
        );
        border(out, pad, chars);

        let mut previous: Option<usize> = None;
        for (index, segments) in &self.lines {
            if let Some(previous) = previous {
                match index - previous {
                    1 => {}
                    2 => {
                        let text = self.file.line_text(previous + 1).replace('\t', TAB);
                        let width = text.chars().count();
                        let window = Window::new(width, 0, 0, chars.cut);
                        self.source_line(out, &text, previous + 1, gutter, &window);
                    }
                    _ => {
                        let line = out.line();
                        line.push(pad, Style::Plain);
                        line.push(chars.ellipsis, Style::Border);
                    }
                }
            }
            previous = Some(*index);
            let text = self.file.line_text(*index).replace('\t', TAB);
            let width = text.chars().count();
            let lo = segments.iter().map(|s| s.start).min().unwrap_or(0);
            let hi = segments.iter().map(|s| s.end).max().unwrap_or(0);
            let window = Window::new(width, lo, hi, chars.cut);
            self.source_line(out, &text, *index, gutter, &window);
            let shifted: Vec<Segment<'_>> = segments
                .iter()
                .map(|s| Segment {
                    start: window.column(s.start),
                    end: window.column(s.end),
                    primary: s.primary,
                    message: s.message,
                })
                .collect();
            self.markers(out, pad, &shifted);
        }
    }

    /// One source line, `text`, with tabs already expanded, cut to `window`.
    fn source_line(
        &self,
        out: &mut Output,
        text: &str,
        index: usize,
        gutter: usize,
        window: &Window,
    ) {
        let line = out.line();
        let mut number = String::new();
        let _ = write!(number, "{:>gutter$} ", index + 1);
        line.push(&number, Style::Border);
        line.push_char(self.chars.border, Style::Border);
        line.push(" ", Style::Plain);
        if window.start > 0 {
            line.push(self.chars.cut, Style::Border);
        }
        let shown: String = text
            .chars()
            .skip(window.start)
            .take(window.end - window.start)
            .collect();
        line.push(&shown, Style::Plain);
        if window.end < window.width {
            line.push(self.chars.cut, Style::Border);
        }
    }

    fn markers(&self, out: &mut Output, pad: &str, segments: &[Segment<'_>]) {
        let chars = self.chars;
        let style = |s: &Segment<'_>| {
            if s.primary {
                Style::Primary
            } else {
                Style::Secondary
            }
        };
        let mut rows: Vec<Row> = vec![Row::default()];

        // Secondary marks first, so primary marks win where they overlap.
        for segment in segments.iter().filter(|s| !s.primary) {
            rows[0].put_repeat(
                segment.start,
                chars.secondary,
                segment.end - segment.start,
                style(segment),
            );
        }
        for segment in segments.iter().filter(|s| s.primary) {
            rows[0].put_repeat(
                segment.start,
                chars.primary,
                segment.end - segment.start,
                style(segment),
            );
        }

        // The rightmost label puts its message on the marker line, if nothing extends past it.
        let max_end = segments.iter().map(|s| s.end).max().unwrap_or(0);
        let inline = segments
            .last()
            .filter(|s| s.message.is_some() && s.end == max_end);
        if let Some(segment) = inline {
            rows[0].put_str(
                segment.end + 1,
                segment.message.unwrap_or(""),
                style(segment),
            );
        }

        // The others hang below, right to left, each joined to its marks by a connector.
        let stacked: Vec<&Segment<'_>> = segments
            .iter()
            .filter(|s| s.message.is_some())
            .filter(|s| !inline.is_some_and(|i| std::ptr::eq(*s, i)))
            .collect();
        for k in (0..stacked.len()).rev() {
            let mut connectors = Row::default();
            for s in &stacked[..=k] {
                connectors.put_char(s.start, chars.connector, style(s));
            }
            let mut message = Row::default();
            for s in &stacked[..k] {
                message.put_char(s.start, chars.connector, style(s));
            }
            message.put_str(
                stacked[k].start,
                stacked[k].message.unwrap_or(""),
                style(stacked[k]),
            );
            rows.push(connectors);
            rows.push(message);
        }

        for row in rows {
            let line = out.line();
            line.push(pad, Style::Plain);
            line.push_char(chars.border, Style::Border);
            line.push(" ", Style::Plain);
            line.cells.extend(row.cells);
        }
    }
}

/// The columns of a source line that are shown: all of a short line, and a window around the
/// labels (columns `lo..hi`) of a line wider than [`MAX_LINE_WIDTH`].
#[derive(Debug)]
struct Window {
    /// Width of the whole line, in columns.
    width: usize,
    start: usize,
    end: usize,
    /// Columns taken by the mark of left-out text before the window (0 if nothing is cut).
    lead: usize,
}

impl Window {
    fn new(width: usize, lo: usize, hi: usize, cut: &str) -> Self {
        if width <= MAX_LINE_WIDTH {
            return Window {
                width,
                start: 0,
                end: width,
                lead: 0,
            };
        }
        let mut start = lo.saturating_sub(CONTEXT);
        let end = (hi + CONTEXT).max(start + WINDOW).min(width);
        if end - start < WINDOW {
            start = end.saturating_sub(WINDOW).min(lo);
        }
        let lead = if start > 0 { cut.chars().count() } else { 0 };
        Window {
            width,
            start,
            end,
            lead,
        }
    }

    /// Where line column `column` is shown.
    fn column(&self, column: usize) -> usize {
        column.saturating_sub(self.start) + self.lead
    }
}

fn border(out: &mut Output, pad: &str, chars: &Chars) {
    let line = out.line();
    line.push(pad, Style::Plain);
    line.push_char(chars.border, Style::Border);
}

/// Display width of `text`: one column per character, four per tab.
fn display_width(text: &str) -> usize {
    text.chars()
        .map(|c| if c == '\t' { TAB.len() } else { 1 })
        .sum()
}

/// Display column (0-based) of byte `offset` on the 0-based line `index`.
fn display_column(file: &SourceFile, index: usize, offset: u32) -> usize {
    let text = file.line_text(index);
    let start = file.line_start(index);
    let mut within = (offset.saturating_sub(start) as usize).min(text.len());
    while !text.is_char_boundary(within) {
        within -= 1;
    }
    display_width(&text[..within])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Bold,
    Border,
    Severity(Severity),
    Primary,
    Secondary,
}

/// One output line as styled cells. Marker rows are drawn on a grid, so each cell is one char.
#[derive(Debug, Default)]
struct Row {
    cells: Vec<(char, Style)>,
}

impl Row {
    fn push(&mut self, text: &str, style: Style) {
        self.cells.extend(text.chars().map(|c| (c, style)));
    }

    fn push_char(&mut self, c: char, style: Style) {
        self.cells.push((c, style));
    }

    fn put_char(&mut self, column: usize, c: char, style: Style) {
        if self.cells.len() <= column {
            self.cells.resize(column + 1, (' ', Style::Plain));
        }
        self.cells[column] = (c, style);
    }

    fn put_repeat(&mut self, column: usize, c: char, count: usize, style: Style) {
        for i in 0..count {
            self.put_char(column + i, c, style);
        }
    }

    fn put_str(&mut self, column: usize, text: &str, style: Style) {
        for (i, c) in text.chars().enumerate() {
            self.put_char(column + i, c, style);
        }
    }
}

#[derive(Debug)]
struct Output {
    lines: Vec<Row>,
    /// Colors primary marks.
    severity: Severity,
}

impl Output {
    fn new(severity: Severity) -> Self {
        Output {
            lines: Vec::new(),
            severity,
        }
    }

    fn line(&mut self) -> &mut Row {
        self.lines.push(Row::default());
        self.lines.last_mut().expect("just pushed")
    }

    fn finish(self, color: bool) -> String {
        let severity = self.severity;
        let mut out = String::new();
        for row in self.lines {
            let mut cells = row.cells;
            while cells.last().is_some_and(|(c, _)| c.is_whitespace()) {
                cells.pop();
            }
            let mut current = Style::Plain;
            for (c, style) in cells {
                if color && style != current {
                    if current != Style::Plain {
                        out.push_str(RESET);
                    }
                    out.push_str(ansi(style, severity));
                    current = style;
                }
                out.push(c);
            }
            if color && current != Style::Plain {
                out.push_str(RESET);
            }
            out.push('\n');
        }
        out
    }
}

const RESET: &str = "\x1b[0m";

fn ansi(style: Style, severity: Severity) -> &'static str {
    let severity_color = |s: Severity| match s {
        Severity::Error => "\x1b[1;31m",
        Severity::Warning => "\x1b[1;33m",
        Severity::Info => "\x1b[1;36m",
    };
    match style {
        Style::Plain => "",
        Style::Bold => "\x1b[1m",
        Style::Border | Style::Secondary => "\x1b[1;34m",
        Style::Severity(s) => severity_color(s),
        Style::Primary => severity_color(severity),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{Code, Span};

    fn shown_lines(snippet: &Snippet<'_>) -> BTreeSet<usize> {
        snippet.lines.iter().map(|(line, _)| *line).collect()
    }

    #[test]
    fn segments_cover_every_labelled_line() {
        let file = SourceFile::new("q", "a\nb\nc\n");
        let label = Label::new(Span::new(FileId(0), 0, 5), "x");
        let snippet = Snippet::new(&file, &[(&label, true)], &ASCII);
        assert_eq!(shown_lines(&snippet), BTreeSet::from([0, 2]));
    }

    #[test]
    fn no_trailing_whitespace() {
        let mut sources = SourceMap::new();
        let f = sources.add("q", "x   \n");
        let d = Diagnostic::error(Code::new(1), "m").primary(Span::new(f, 0, 1), "");
        let out = Renderer::plain().render(&d, &sources);
        assert!(out.lines().all(|l| l == l.trim_end()), "{out:?}");
    }

    #[test]
    fn long_lines_are_cut_around_the_label() {
        let mut sources = SourceMap::new();
        let text = format!("{}xyz{}\n", "a".repeat(300), "b".repeat(300));
        let f = sources.add("q", text.as_str());
        let d = Diagnostic::error(Code::new(1), "m").primary(Span::new(f, 300, 303), "here");
        let out = Renderer::plain().render(&d, &sources);
        let lines: Vec<&str> = out.lines().collect();
        let source = lines[3];
        let marker = lines[4];
        assert!(
            source.starts_with("1 | ...") && source.ends_with("..."),
            "{out}"
        );
        assert!(source.chars().count() < 120, "{out}");
        let column = source.find("xyz").unwrap();
        assert_eq!(marker.find('^'), Some(column), "{out}");
        assert!(marker.ends_with("^^^ here"), "{out}");
    }

    #[test]
    fn short_lines_are_not_cut() {
        let mut sources = SourceMap::new();
        let text = "a".repeat(120);
        let f = sources.add("q", text.as_str());
        let d = Diagnostic::error(Code::new(1), "m").primary(Span::new(f, 119, 120), "");
        let out = Renderer::plain().render(&d, &sources);
        assert!(!out.contains("..."), "{out}");
    }

    #[test]
    fn tabs_in_help_are_expanded() {
        let mut sources = SourceMap::new();
        let f = sources.add("q", "x\n");
        let d = Diagnostic::error(Code::new(1), "m")
            .primary(Span::new(f, 0, 1), "")
            .help("write `a\tb`")
            .note("c\td");
        let out = Renderer::plain().render(&d, &sources);
        assert!(!out.contains('\t'), "{out:?}");
        assert!(
            out.contains("write `a    b`") && out.contains("c    d"),
            "{out}"
        );
    }
}
