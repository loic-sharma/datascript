//! Sources (M1-17): what a table position names, and the schema the provider gives for it
//! (LANGUAGE.md §3.1, ARCHITECTURE.md § Source resolution, D-020, D-022, D-023).
//!
//! A table position (the operand of `from`, the right side of `join` and `union`) is:
//!
//! - a tabular `let` or a pipeline in parentheses;
//! - a reader call, `read_csv('x.txt', delimiter = ';')`, whose path and options are constants;
//! - a constant string, a literal or a scalar `let` bound to one, which is a path or a glob
//!   (D-020); a string that is not constant is `DS3006`, and any other value `DS2040`;
//! - any other name, which is a host table.
//!
//! The format of a path comes from its extension (`DS3002` if it has none we know); a format the
//! engine cannot read is `DS4001`, reported before the provider is asked. A `SourceError` from
//! the provider becomes a diagnostic at the span where the source appears, so a source used in
//! two places is reported at both.

use std::collections::HashSet;

use datascript_diagnostics::codes::{
    DS1002, DS2006, DS2040, DS2041, DS2042, DS2043, DS3001, DS3002, DS3003, DS3006, DS4001, DS9001,
};
use datascript_diagnostics::{Applicability, Diagnostic, Span, Suggestion, TextEdit, did_you_mean};
use datascript_syntax::ast::{self, AstNode};

use super::{Agg, Binder, Cols, Cx, LetEntry, StageSchema, schema_of};
use crate::Type;
use crate::arrow::from_arrow;
use crate::capabilities::SourceFormat;
use crate::hir::{Column, Expr, ExprKind, Source};
use crate::ir::Value;
use crate::source::{
    self, OptionKind, OptionValue, ReaderCall, ReaderDef, SchemaLookup, SourceError, SourceRef,
    format_of_path,
};

/// The constant value of a bound expression: a literal, or a `let` bound to one.
fn const_value(expr: &Expr) -> Option<&Value> {
    match &expr.kind {
        ExprKind::Literal(value) => Some(value),
        ExprKind::Let { value, .. } => const_value(value),
        _ => None,
    }
}

impl Binder<'_> {
    /// Binds a table position.
    pub(super) fn bind_source(&mut self, source: &ast::Expr) -> (Source, Option<Vec<Column>>) {
        let span = self.span(source);
        let error = (Source::Error { span }, None);
        if self.broken.contains(source.syntax()) {
            return error;
        }
        match source {
            ast::Expr::Paren(paren) => match paren.inner() {
                Some(ast::Expr::Pipeline(inner)) => {
                    let pipeline = self.bind_pipeline(&inner);
                    let columns = pipeline.columns.clone();
                    (Source::Subquery { span, pipeline }, columns)
                }
                Some(inner) => self.bind_source(&inner),
                None => error,
            },
            ast::Expr::NameRef(name_ref) => {
                let Some(name) = name_ref.text() else {
                    return error;
                };
                let entry = self.lets.get(&name).map(|(_, entry)| entry.clone());
                match entry {
                    Some(LetEntry::Table(pipeline)) => {
                        let columns = pipeline.columns.clone();
                        if let Some(columns) = &columns {
                            self.stage_schemas.push(StageSchema {
                                span,
                                schema: schema_of(columns),
                            });
                        }
                        (
                            Source::Let {
                                name,
                                span,
                                pipeline,
                            },
                            columns,
                        )
                    }
                    Some(LetEntry::Error) => error,
                    Some(LetEntry::Scalar(value)) => {
                        let expr = Expr {
                            ty: value.ty,
                            kind: ExprKind::Let {
                                name,
                                value: Box::new(value),
                            },
                            span,
                        };
                        self.path_source(&expr, span)
                    }
                    None => self.lookup(SourceRef::Table(name), span),
                }
            }
            ast::Expr::Call(call) => {
                let def = call
                    .callee()
                    .and_then(|c| c.text())
                    .and_then(|name| source::reader(&name));
                match def {
                    Some(def) => self.reader_source(call, def, span),
                    None => {
                        let expr = self.const_expr(source);
                        self.path_source(&expr, span)
                    }
                }
            }
            _ => {
                let expr = self.const_expr(source);
                self.path_source(&expr, span)
            }
        }
    }

    /// Binds an expression that must be a constant: a path or a reader option. There are no
    /// columns.
    fn const_expr(&mut self, expr: &ast::Expr) -> Expr {
        let cx = Cx {
            cols: Cols::NoRows,
            agg: Agg::Row("a source"),
        };
        self.expr(expr, &cx)
    }

    /// A value in a table position: a constant string is a path (D-020).
    fn path_source(&mut self, expr: &Expr, span: Span) -> (Source, Option<Vec<Column>>) {
        let error = (Source::Error { span }, None);
        if expr.ty.is_error() {
            return error;
        }
        match const_value(expr) {
            Some(Value::String(path)) => {
                let path = path.clone();
                match format_of_path(&path) {
                    None => {
                        self.unknown_format(span, &path);
                        error
                    }
                    Some(format) if !self.caps.supports_format(format) => {
                        self.format_unsupported(span, format);
                        error
                    }
                    Some(_) => self.lookup(SourceRef::for_path(path), span),
                }
            }
            _ if expr.ty == Type::String => {
                self.not_constant(span, expr, "the path of a source");
                error
            }
            _ => {
                let mut d = Diagnostic::error(DS2040, "this value is not a table")
                    .primary(
                        span,
                        format!("this is `{}`, not a table or a path", expr.ty),
                    )
                    .help(
                        "a source is a file path such as `'data.csv'`, a reader call such as \
                         `read_csv('x.txt')`, a table name, or a pipeline in parentheses",
                    );
                if let ExprKind::Let { name, .. } = &expr.kind {
                    d = d.note(format!("`{name}` is defined by a `let` as a value"));
                }
                self.diagnostics.push(d);
                error
            }
        }
    }

    /// `read_csv(path, option = value, ...)`.
    fn reader_source(
        &mut self,
        call: &ast::CallExpr,
        def: &'static ReaderDef,
        span: Span,
    ) -> (Source, Option<Vec<Column>>) {
        let error = (Source::Error { span }, None);
        let reader = def.name;
        let mut ok = true;
        let mut paths = Vec::new();
        let mut options: Vec<(String, OptionValue)> = Vec::new();
        let mut seen: Vec<(String, Span)> = Vec::new();
        for arg in call.args() {
            let named = match arg {
                ast::Arg::Positional(expr) => {
                    paths.push(expr);
                    continue;
                }
                ast::Arg::Named(named) => named,
            };
            let (Some(name_node), Some(value)) = (named.name(), named.expr()) else {
                ok = false;
                continue;
            };
            let Some(name) = name_node.text() else {
                ok = false;
                continue;
            };
            let name_span = self.span(&name_node);
            let Some(option) = def.option(&name) else {
                self.unknown_option(def, &name, name_span);
                ok = false;
                continue;
            };
            if let Some(milestone) = option.planned {
                self.diagnostics.push(
                    Diagnostic::error(
                        DS3003,
                        format!("reader option `{name}` is not available yet"),
                    )
                    .primary(name_span, format!("planned for {milestone}"))
                    .note(format!(
                        "it is part of `{reader}`, but this version does not have it"
                    )),
                );
                ok = false;
                continue;
            }
            if let Some((_, first)) = seen.iter().find(|(n, _)| *n == name) {
                let first = *first;
                self.diagnostics.push(
                    Diagnostic::error(DS2043, format!("option `{name}` is given twice"))
                        .primary(name_span, "given again here")
                        .secondary(first, "first given here")
                        .help("keep one of them"),
                );
                ok = false;
                continue;
            }
            seen.push((name.clone(), name_span));
            let expr = self.const_expr(&value);
            if expr.ty.is_error() {
                ok = false;
                continue;
            }
            let Some(constant) = const_value(&expr) else {
                self.not_constant(expr.span, &expr, &format!("reader option `{name}`"));
                ok = false;
                continue;
            };
            match self.option_value(&name, option.kind, constant, &expr) {
                Some(value) => options.push((name, value)),
                None => ok = false,
            }
        }

        let path = match paths.as_slice() {
            [] => {
                self.diagnostics.push(
                    Diagnostic::error(
                        DS2042,
                        format!("`{reader}` needs the path of the file to read"),
                    )
                    .primary(span, "no path given")
                    .help(format!("pass the path first: `{reader}('data/file')`")),
                );
                None
            }
            [path] => self.reader_path(path),
            [_, extra, ..] => {
                let extra_span = self.span(extra);
                self.diagnostics.push(
                    Diagnostic::error(DS2042, format!("`{reader}` reads one path"))
                        .primary(extra_span, "a second path")
                        .help(
                            "options are passed by name, such as `delimiter = ';'`; to read \
                             several files, use a glob such as `'logs/*.csv'`",
                        ),
                );
                None
            }
        };
        let (Some(path), true) = (path, ok) else {
            return error;
        };
        options.sort();
        let call = ReaderCall {
            function: reader.to_string(),
            path,
            options,
        };
        let format = call.format();
        if !self.caps.supports_format(format) {
            self.format_unsupported(span, format);
            return error;
        }
        self.lookup(SourceRef::Reader(call), span)
    }

    /// The path argument of a reader call: a constant string.
    fn reader_path(&mut self, path: &ast::Expr) -> Option<String> {
        let expr = self.const_expr(path);
        if expr.ty.is_error() {
            return None;
        }
        match const_value(&expr) {
            Some(Value::String(path)) => Some(path.clone()),
            _ if expr.ty == Type::String => {
                self.not_constant(expr.span, &expr, "the path of a source");
                None
            }
            _ => {
                self.diagnostics.push(
                    Diagnostic::error(DS2006, "the path must be a string")
                        .primary(expr.span, format!("this is `{}`", expr.ty))
                        .help("write the path as a string, such as `'data/file.csv'`"),
                );
                None
            }
        }
    }

    /// Checks a constant option value against what the option takes (`DS2041`).
    fn option_value(
        &mut self,
        name: &str,
        kind: OptionKind,
        value: &Value,
        expr: &Expr,
    ) -> Option<OptionValue> {
        let span = expr.span;
        let (message, label, fix) = match (kind, value) {
            (OptionKind::Bool, Value::Bool(b)) => return Some(OptionValue::Bool(*b)),
            (OptionKind::Bool, _) => (
                format!("`{name}` must be `true` or `false`"),
                format!("this is `{}`", expr.ty),
                None,
            ),
            (OptionKind::PositiveLong, Value::Long(n)) if *n >= 1 => {
                return Some(OptionValue::Long(*n));
            }
            (OptionKind::PositiveLong, _) => (
                format!("`{name}` must be a whole number of at least 1"),
                match value {
                    Value::Long(n) => format!("this is {n}"),
                    _ => format!("this is `{}`", expr.ty),
                },
                None,
            ),
            (OptionKind::Char, Value::String(s)) if s.chars().count() == 1 => {
                return Some(OptionValue::String(s.clone()));
            }
            (OptionKind::Char, Value::String(s)) => (
                format!("`{name}` must be a single character"),
                format!("this has {} characters", s.chars().count()),
                None,
            ),
            (OptionKind::String, Value::String(s)) => return Some(OptionValue::String(s.clone())),
            (OptionKind::OneOf(allowed), Value::String(s)) if allowed.contains(&s.as_str()) => {
                return Some(OptionValue::String(s.clone()));
            }
            (OptionKind::OneOf(allowed), Value::String(s)) => {
                let list: Vec<String> = allowed.iter().map(|a| format!("'{a}'")).collect();
                let fix = did_you_mean(s, allowed.iter().copied())
                    .single()
                    .map(|best| best.to_string());
                (
                    format!("`{name}` must be one of {}", list.join(", ")),
                    format!("'{s}' is not one of them"),
                    fix,
                )
            }
            (OptionKind::Char | OptionKind::String | OptionKind::OneOf(_), _) => (
                format!("`{name}` must be a string"),
                format!("this is `{}`", expr.ty),
                None,
            ),
        };
        let mut d = Diagnostic::error(DS2041, message).primary(span, label);
        if let Some(best) = fix {
            d = d.help_with(
                format!("did you mean '{best}'?"),
                Suggestion::replace(span, format!("'{best}'")),
            );
        }
        self.diagnostics.push(d);
        None
    }

    /// Asks the provider for the schema of `source`, which appears at `span`.
    fn lookup(&mut self, source: SourceRef, span: Span) -> (Source, Option<Vec<Column>>) {
        match self.schemas.schema_of(&source) {
            SchemaLookup::Ready(schema) => {
                let read_from = match &source {
                    SourceRef::Table(name) => name.clone(),
                    other => other.path().unwrap_or_default().to_string(),
                };
                let columns: Vec<Column> = schema
                    .fields()
                    .iter()
                    .map(|f| match from_arrow(f) {
                        Ok(ty) => Column {
                            name: f.name().clone(),
                            ty,
                            read_from: Some(read_from.clone()),
                            arrow_only: None,
                        },
                        Err(_) => Column {
                            name: f.name().clone(),
                            ty: Type::Error,
                            read_from: Some(read_from.clone()),
                            arrow_only: Some(f.data_type().clone()),
                        },
                    })
                    .collect();
                self.stage_schemas.push(StageSchema {
                    span,
                    schema: schema_of(&columns),
                });
                let source = Source::Scan {
                    source,
                    span,
                    columns: columns.clone(),
                };
                (source, Some(columns))
            }
            SchemaLookup::Failed(failure) => {
                self.source_error(span, &source, &failure);
                (Source::Error { span }, None)
            }
            SchemaLookup::Pending => {
                self.pending.insert(source);
                (Source::Error { span }, None)
            }
        }
    }

    // --- Diagnostics ---

    /// `DS3006`: a path or reader option that is not a constant.
    fn not_constant(&mut self, span: Span, expr: &Expr, what: &str) {
        let mut d = Diagnostic::error(DS3006, format!("{what} must be a constant"))
            .primary(span, "this is computed")
            .help(
                "write the value itself, such as `'data/orders.csv'`, or a `let` bound to it: \
                 `let path = 'data/orders.csv';`",
            )
            .note("schemas are read before the program runs, so sources cannot depend on values");
        if let ExprKind::Let { name, .. } = &expr.kind {
            d = d.note(format!(
                "`{name}` is defined by a `let` whose value is not a literal"
            ));
        }
        self.diagnostics.push(d);
    }

    /// `DS3002`: a path whose extension is not a known format.
    fn unknown_format(&mut self, span: Span, path: &str) {
        let text = &self.text[span.range()];
        let rewrite = format!("read_csv({text})");
        self.diagnostics.push(
            Diagnostic::error(DS3002, format!("cannot tell the format of `{path}`"))
                .primary(span, "the file name has no known extension")
                .help_with(
                    format!("to read it as CSV, use a reader function: `{rewrite}`"),
                    Suggestion::new(
                        vec![TextEdit::new(span, rewrite.clone())],
                        Applicability::MaybeIncorrect,
                    ),
                )
                .note(
                    "formats come from the extension: .csv, .tsv, .parquet, .json, .ndjson and \
                     .jsonl, and CSV, TSV and JSON may end in .gz, .bz2, .xz or .zst; \
                     `read_parquet` and `read_json` read other names too",
                ),
        );
    }

    /// `DS4001`: a format the engine cannot read.
    fn format_unsupported(&mut self, span: Span, format: SourceFormat) {
        let mut readable: Vec<&str> = self.caps.formats.iter().map(|f| f.name()).collect();
        readable.sort_unstable();
        let mut d = Diagnostic::error(
            DS4001,
            format!("this engine cannot read {} files", format.name()),
        )
        .primary(span, format!("a {} source", format.name()));
        d = if readable.is_empty() {
            d.note("this engine reads no files")
        } else {
            d.note(format!("this engine reads: {}", readable.join(", ")))
        };
        self.diagnostics.push(d);
    }

    /// `DS3003`: an option the reader does not have.
    fn unknown_option(&mut self, def: &ReaderDef, name: &str, span: Span) {
        let reader = def.name;
        let names: Vec<&str> = def
            .options
            .iter()
            .filter(|o| o.planned.is_none())
            .map(|o| o.name)
            .collect();
        let mut d = Diagnostic::error(DS3003, format!("`{reader}` has no option `{name}`"))
            .primary(span, "unknown option");
        if let Some(best) = did_you_mean(name, names.iter().copied()).single() {
            d = d.help_with(
                format!("an option with a similar name exists: `{best}`"),
                Suggestion::replace(span, best),
            );
        }
        d = if names.is_empty() {
            d.note(format!("`{reader}` takes no options"))
        } else {
            d.note(format!("`{reader}` takes: {}", names.join(", ")))
        };
        self.diagnostics.push(d);
    }

    /// The diagnostic for a source the provider could not read, at one place it appears.
    fn source_error(&mut self, span: Span, source: &SourceRef, error: &SourceError) {
        match (source, error) {
            (SourceRef::Table(name), SourceError::NotFound { nearby }) => {
                self.unknown_table(span, name, nearby)
            }
            (_, SourceError::NotFound { nearby }) => {
                let path = source.path().unwrap_or_default();
                self.file_not_found(span, path, nearby)
            }
            (_, SourceError::Unreadable { reason }) => {
                self.diagnostics.push(
                    Diagnostic::error(DS9001, format!("cannot read the schema of {source}"))
                        .primary(span, "this source could not be read")
                        .note(format!("the engine said: {reason}")),
                );
            }
        }
    }

    fn file_not_found(&mut self, span: Span, path: &str, nearby: &[String]) {
        let (dir, file_name) = match path.rfind(['/', '\\']) {
            Some(i) => (&path[..=i], &path[i + 1..]),
            None => ("", path),
        };
        let mut d = Diagnostic::error(DS3001, format!("file `{path}` not found"))
            .primary(span, "no file at this path");
        let suggestions = did_you_mean(file_name, nearby.iter().map(String::as_str));
        if let Some(best) = suggestions.single() {
            let text = &self.text[span.range()];
            let help = format!("a file with a similar name exists: `{dir}{best}`");
            // Replace the file name inside the literal. A literal with escapes that spell the
            // name differently, or a `let` name, gets the help without an edit.
            let quoted = ["'", "\""]
                .iter()
                .find_map(|q| text.find(&format!("{file_name}{q}")));
            match quoted {
                Some(i) => {
                    let start = span.start + i as u32;
                    let edit = TextEdit::new(
                        Span::new(self.file, start, start + file_name.len() as u32),
                        best,
                    );
                    d = d.help_with(
                        help,
                        Suggestion::new(vec![edit], Applicability::MaybeIncorrect),
                    );
                }
                None => d = d.help(help),
            }
        }
        if path.starts_with('~') {
            d = d.note("`~` is not expanded; write the full path");
        }
        self.diagnostics.push(d);
    }

    /// `DS1002` for a name in a table position that is neither a `let` nor a host table.
    fn unknown_table(&mut self, span: Span, name: &str, nearby: &[String]) {
        let mut candidates: HashSet<String> = nearby.iter().cloned().collect();
        candidates.extend(
            self.lets
                .iter()
                .filter(|(_, (_, entry))| matches!(entry, LetEntry::Table(_)))
                .map(|(n, _)| n.clone()),
        );
        let mut candidates: Vec<String> = candidates.into_iter().collect();
        candidates.sort();
        let mut d = Diagnostic::error(DS1002, format!("unknown table `{name}`"))
            .primary(span, "no table with this name");
        let suggestions = did_you_mean(name, candidates.iter().map(String::as_str));
        if let Some(best) = suggestions.single() {
            d = d.help_with(
                format!("a table with a similar name exists: `{best}`"),
                Suggestion::replace(span, best),
            );
        } else if !suggestions.is_empty() {
            let list: Vec<String> = suggestions
                .names()
                .iter()
                .map(|n| format!("`{n}`"))
                .collect();
            d = d.help(format!("similar names exist: {}", list.join(", ")));
        }
        self.diagnostics.push(d.note(
            "a table name is a tabular `let` or a table registered by the host; to read a \
             file, write its path as a string: `from 'orders.csv'`",
        ));
    }
}
