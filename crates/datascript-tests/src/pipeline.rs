//! The driver that runs cases through the real pipeline, as far as it exists.

use std::sync::Arc;

use datascript_compiler::source::format_of_path;
use datascript_compiler::{Capabilities, SourceRef};
use datascript_datafusion::{DataFusionConfig, DataFusionEngine};
use datascript_diagnostics::{
    Diagnostic, DiagnosticSink, Diagnostics, FileId, Renderer, SourceFile, SourceMap,
};
use datascript_engine::{FixedClock, Session, table::format_table};
use datascript_syntax::ast::{Operator, Stmt};

use crate::case::Mode;
use crate::driver::{CaseInput, Driver, DriverError, DriverResult};
use crate::fixes::{apply_suggestions, apply_suggestions_detailed};

/// Runs cases through the DataScript pipeline: syntax, then the compiler, then the
/// [`Session`] over [`DataFusionEngine`].
///
/// `mode: syntax` prints the CST dump ([`datascript_syntax::debug_tree`]), then the lexical and
/// syntax diagnostics after a blank line. If any diagnostic has a machine-applicable fix, it then
/// prints the query with every such fix applied ([`apply_suggestions`]) and the diagnostics of
/// parsing that text again, so a case shows that its fixes work.
///
/// `check` and `explain` cases with a `schema:` header use the
/// [`MockSchemaProvider`](crate::MockSchemaProvider) and read no file; without one they go through the session, which
/// reads the files. `run` cases always go through the session.
#[derive(Debug, Clone)]
pub struct PipelineDriver {
    engine: String,
}

impl PipelineDriver {
    pub fn new(engine: impl Into<String>) -> Self {
        Self {
            engine: engine.into(),
        }
    }
}

/// The query as file `query`, ready to compile and render diagnostics against.
struct Query {
    sources: SourceMap,
    src: SourceFile,
    file: FileId,
}

impl Query {
    fn new(input: &CaseInput<'_>) -> Self {
        Self::from_text("query", input.query)
    }

    fn from_text(name: &str, text: &str) -> Self {
        let mut sources = SourceMap::new();
        let file = sources.add(name, text);
        Query {
            sources,
            src: SourceFile::new(name, text),
            file,
        }
    }

    fn render(&self, diagnostics: &[Diagnostic]) -> String {
        Renderer::plain().render_all(diagnostics, &self.sources)
    }
}

/// A session over the DataFusion engine, with the case's `tables:` registered as host tables.
/// Their paths resolve against the test file's directory, like paths in the query.
fn session(input: &CaseInput<'_>) -> Result<Session, DriverError> {
    let mut engine = DataFusionEngine::new(DataFusionConfig {
        target_partitions: input.config.target_partitions,
        batch_size: input.config.batch_size,
    });
    for (name, path) in input.tables {
        if format_of_path(path).is_none() {
            return Err(DriverError::Failed(format!(
                "`tables`: cannot tell the format of `{path}` from its extension"
            )));
        }
        engine = engine.with_table(name, SourceRef::for_path(path.as_str()));
    }
    Ok(Session::new(
        Arc::new(engine),
        Arc::new(FixedClock(input.now_micros)),
    ))
}

fn block_on<F: std::future::Future>(future: F) -> Result<F::Output, DriverError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| DriverError::Failed(format!("cannot start the async runtime: {e}")))?;
    Ok(runtime.block_on(future))
}

impl Driver for PipelineDriver {
    fn engine(&self) -> &str {
        &self.engine
    }

    fn supports(&self, mode: Mode) -> bool {
        matches!(mode, Mode::Syntax | Mode::Check | Mode::Explain | Mode::Run)
    }

    fn syntax(&self, input: &CaseInput<'_>) -> DriverResult {
        let mut sources = SourceMap::new();
        let file = sources.add("query", input.query);
        let parse = datascript_syntax::parse(input.query, file);
        let mut out = parse.debug_tree();
        let mut diagnostics = Diagnostics::new();
        for d in parse.errors {
            diagnostics.emit(d);
        }
        let diagnostics = diagnostics.finish();
        if !diagnostics.is_empty() {
            out.push('\n');
            out.push_str(&Renderer::plain().render_all(&diagnostics, &sources));
        }
        if let Some(fixed) = apply_suggestions(input.query, file, &diagnostics) {
            out.push_str("\nwith the machine-applicable fixes applied:\n");
            for line in fixed.trim_end().lines() {
                if !line.is_empty() {
                    out.push_str("    ");
                }
                out.push_str(line);
                out.push('\n');
            }
            let mut fixed_sources = SourceMap::new();
            let fixed_file = fixed_sources.add("fixed", fixed.as_str());
            let reparse = datascript_syntax::parse(&fixed, fixed_file);
            if reparse.errors.is_empty() {
                out.push_str("which parses without errors\n");
            } else {
                out.push_str("which still has errors:\n\n");
                out.push_str(&Renderer::plain().render_all(&reparse.errors, &fixed_sources));
            }
        }
        Ok(out)
    }

    fn check(&self, input: &CaseInput<'_>) -> DriverResult {
        let query = Query::new(input);
        let caps = input
            .capabilities
            .cloned()
            .unwrap_or_else(Capabilities::full);
        let live_session = match input.schema {
            Some(_) => None,
            None => Some(session(input)?),
        };
        let run_check = |q: &Query| -> Result<datascript_compiler::Analysis, DriverError> {
            match input.schema {
                Some(mock) => Ok(datascript_compiler::check(&q.src, q.file, mock, &caps)),
                None => {
                    let session = live_session
                        .as_ref()
                        .expect("session built when schema is None");
                    let ctx = session.exec_context(input.base_dir());
                    block_on(session.check(&q.src, q.file, &ctx))
                }
            }
        };
        let analysis = run_check(&query)?;
        verify_check_fixes(input.query, query.file, &analysis.diagnostics, &run_check)?;
        let mut out = query.render(&analysis.diagnostics);
        if input.show_schema {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&query_schemas(&analysis.hir));
        }
        Ok(out)
    }

    fn explain(&self, input: &CaseInput<'_>) -> DriverResult {
        let query = Query::new(input);
        let caps = input
            .capabilities
            .cloned()
            .unwrap_or_else(Capabilities::full);
        let program = match input.schema {
            Some(mock) => datascript_compiler::compile(&query.src, query.file, mock, &caps),
            None => {
                let session = session(input)?;
                let ctx = session.exec_context(input.base_dir());
                block_on(session.compile(&query.src, query.file, &ctx))?
            }
        };
        Ok(match program {
            Ok(program) => program.explain(),
            Err(diagnostics) => query.render(&diagnostics),
        })
    }

    fn run(&self, input: &CaseInput<'_>) -> DriverResult {
        if input.schema.is_some() {
            return Err(DriverError::Failed(
                "`run` cases read real files and cannot use `schema:`".to_string(),
            ));
        }
        let query = Query::new(input);
        let session = session(input)?;
        let ctx = session.exec_context(input.base_dir());
        let output = block_on(session.run(&query.src, query.file, &ctx))?;
        let mut out = query.render(&output.diagnostics);
        for (i, result) in output.results.iter().enumerate() {
            if i > 0 || !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format_table(&result.schema, &result.batches));
            // DS5001 is not printed: this line already shows the chart (TESTING.md).
            if let Some(render) = &result.render {
                out.push_str(&render.summary());
                out.push('\n');
            }
        }
        Ok(out)
    }

    fn output_order_defined(&self, query: &str) -> Option<bool> {
        Some(output_order_defined(query))
    }

    fn query_renders(&self, query: &str) -> Option<Vec<bool>> {
        Some(query_renders(query))
    }
}

/// Verifies that applying all `MachineApplicable` suggestions from `check` produces a query with
/// no new parse errors and resolves each fixed diagnostic at its fixed location (M2-30).
///
/// Pre-existing parse errors whose fix was not applied (because they had no `MachineApplicable`
/// suggestion or because their suggestion overlapped an earlier applied fix) may remain at their
/// mapped locations.
fn verify_check_fixes(
    text: &str,
    file: FileId,
    diagnostics: &[Diagnostic],
    run_check: &dyn Fn(&Query) -> Result<datascript_compiler::Analysis, DriverError>,
) -> Result<(), DriverError> {
    let Some(applied) = apply_suggestions_detailed(text, file, diagnostics) else {
        return Ok(());
    };
    let mut applied_diags: Vec<&Diagnostic> = applied
        .applied_indices
        .iter()
        .map(|&i| &diagnostics[i])
        .collect();
    let orig_parse = datascript_syntax::parse(text, file);
    let pre_existing_unfixed: Vec<(datascript_diagnostics::Code, Option<std::ops::Range<u32>>)> =
        orig_parse
            .errors
            .iter()
            .filter(|orig| {
                // If this parse error had a suggestion actually applied, it must not remain.
                if let Some(pos) = applied_diags.iter().position(|d| {
                    d.code == orig.code && d.primary == orig.primary && d.message == orig.message
                }) {
                    applied_diags.swap_remove(pos);
                    false
                } else {
                    true
                }
            })
            .map(|d| {
                let span = d
                    .primary
                    .as_ref()
                    .filter(|p| p.span.file == file)
                    .map(|p| applied.map_range(p.span.start..p.span.end));
                (d.code, span)
            })
            .collect();

    let fixed_query = Query::from_text("fixed", &applied.text);
    let is_pre_existing = |d: &Diagnostic| {
        let span = d
            .primary
            .as_ref()
            .filter(|p| p.span.file == fixed_query.file)
            .map(|p| p.span.start..p.span.end);
        pre_existing_unfixed.iter().any(|(code, mapped)| {
            *code == d.code && pre_existing_spans_match(mapped.as_ref(), span.as_ref())
        })
    };
    let reparse = datascript_syntax::parse(&applied.text, fixed_query.file);
    let new_parse_errors: Vec<Diagnostic> = reparse
        .errors
        .into_iter()
        .filter(|d| !is_pre_existing(d))
        .collect();
    if !new_parse_errors.is_empty() {
        return Err(DriverError::Failed(format!(
            "applying machine-applicable fixes produced parse errors:\n\
             fixed query:\n{}\
             diagnostics:\n{}",
            indent_lines(&applied.text),
            fixed_query.render(&new_parse_errors),
        )));
    }
    let rechecked = run_check(&fixed_query)?;
    if let Some(remaining) = rechecked
        .diagnostics
        .iter()
        .find(|d| !is_pre_existing(d) && applied.matches_remaining(fixed_query.file, d))
    {
        return Err(DriverError::Failed(format!(
            "machine-applicable fix did not resolve {} at the fixed location:\n\
             fixed query:\n{}\
             diagnostic:\n{}",
            remaining.code,
            indent_lines(&applied.text),
            fixed_query.render(std::slice::from_ref(remaining)),
        )));
    }
    Ok(())
}

/// Whether a re-parsed error span matches a pre-existing unfixed error's mapped span (by overlap
/// or containment, so recovery spans that grow or shrink slightly when nearby tokens change are
/// still recognized as the same pre-existing error).
///
/// Unlike [`crate::fixes::ranges_overlap`] — which rejects touching edges so an empty deletion
/// range does not falsely blame an adjacent diagnostic — this accepts touching edges for empty
/// spans so a pre-existing zero-width error right at an edit boundary is still recognized.
fn pre_existing_spans_match(
    mapped: Option<&std::ops::Range<u32>>,
    current: Option<&std::ops::Range<u32>>,
) -> bool {
    match (mapped, current) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if a.start == a.end {
                b.start <= a.start && a.start <= b.end
            } else if b.start == b.end {
                a.start <= b.start && b.start <= a.end
            } else {
                a.start < b.end && b.start < a.end
            }
        }
        _ => false,
    }
}

fn indent_lines(text: &str) -> String {
    let mut out = String::new();
    for line in text.trim_end().lines() {
        if !line.is_empty() {
            out.push_str("    ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// For `show: schema`: each query's output columns, one per line, or `unknown` when an error
/// or a pending source hides them, separated by blank lines.
fn query_schemas(hir: &datascript_compiler::hir::Hir) -> String {
    let mut out = String::new();
    for (i, query) in hir.queries().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match &query.pipeline.columns {
            Some(columns) => {
                out.push_str(&format!("query {} columns:\n", i + 1));
                for c in columns {
                    let ty = if c.ty.is_error() {
                        "(error)".to_string()
                    } else {
                        c.ty.to_string()
                    };
                    out.push_str(&format!("  {}: {ty}\n", c.name));
                }
            }
            None => out.push_str(&format!("query {} columns: unknown\n", i + 1)),
        }
    }
    out
}

fn query_renders(query: &str) -> Vec<bool> {
    let parse = datascript_syntax::parse(query, FileId(0));
    parse
        .program()
        .statements()
        .filter_map(|stmt| match stmt {
            Stmt::Let(_) => None,
            Stmt::Query(pipeline) => Some(matches!(
                pipeline.operators().last(),
                Some(Operator::Render(_))
            )),
        })
        .collect()
}

/// Whether every query statement's row order is defined (LANGUAGE.md §3.4): its last `sort` or
/// `top` is followed only by order-preserving operators. A statement without either has no
/// defined order. Lets are not followed: `s | take 5` with `let s = t | sort ...` counts as
/// undefined, which is the safe answer (`order: any`).
fn output_order_defined(query: &str) -> bool {
    let parse = datascript_syntax::parse(query, FileId(0));
    parse.program().statements().all(|stmt| match stmt {
        Stmt::Let(_) => true,
        Stmt::Query(pipeline) => {
            let mut defined = false;
            for op in pipeline.operators() {
                defined = match op {
                    Operator::Sort(_) | Operator::Top(_) => true,
                    Operator::Where(_)
                    | Operator::Project(_)
                    | Operator::ProjectAway(_)
                    | Operator::ProjectRename(_)
                    | Operator::Extend(_)
                    | Operator::Take(_)
                    | Operator::Render(_) => defined,
                    _ => false,
                };
            }
            defined
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use datascript_diagnostics::{Diagnostic, FileId, Span, Suggestion, codes};

    use super::{PipelineDriver, Query, output_order_defined, verify_check_fixes};
    use crate::driver::{CaseInput, Driver, DriverConfig, DriverError};
    use crate::schema;

    fn mock_input<'a>(
        query: &'a str,
        mock: &'a crate::MockSchemaProvider,
        config: &'a DriverConfig,
    ) -> CaseInput<'a> {
        CaseInput {
            path: Path::new("case.dstest"),
            query,
            now_micros: 0,
            tables: &[],
            schema: Some(mock),
            capabilities: None,
            show_schema: false,
            config,
        }
    }

    #[test]
    fn order_is_defined_by_a_final_sort_or_top() {
        assert!(output_order_defined(
            "t | sort by a asc | take 5 | project a"
        ));
        assert!(output_order_defined("t | top 3 by a | render barchart"));
        assert!(output_order_defined(
            "let x = 1; t | sort by a asc; u | top 1 by b"
        ));
        assert!(!output_order_defined("t | take 5"));
        assert!(!output_order_defined(
            "t | sort by a asc | summarize count() by a"
        ));
        assert!(!output_order_defined("t | sort by a asc; u | count"));
    }

    #[test]
    fn check_verifies_valid_compiler_fixes_and_allows_unrelated_remaining_errors() {
        let driver = PipelineDriver::new("datafusion");
        let mock = schema::parse("t(['total amount']: long, name: string)").unwrap();
        let config = DriverConfig::default();

        // Quoted column did-you-mean fix produces valid syntax and resolves DS1001.
        let out = driver
            .check(&mock_input(
                "from t | project ['total amout']\n",
                &mock,
                &config,
            ))
            .unwrap();
        assert!(out.contains("error[DS1001]"), "{out}");

        // Recovery case: `['total amout']` is fixed, while `zzzzz` stays DS1001 elsewhere.
        let out = driver
            .check(&mock_input(
                "from t | project ['total amout'], zzzzz\n",
                &mock,
                &config,
            ))
            .unwrap();
        assert!(out.contains("zzzzz"), "{out}");

        // Recovery case with an unfixable syntax error in another statement still passes.
        let out = driver
            .check(&mock_input(
                "from t | where 1 < ['total amount'] < 3;\nfrom t | project ['total amout']\n",
                &mock,
                &config,
            ))
            .unwrap();
        assert!(
            out.contains("error[DS0101]") && out.contains("error[DS1001]"),
            "{out}"
        );
    }

    #[test]
    fn verify_check_fixes_fails_on_parse_error_or_unresolved_code() {
        let mock = schema::parse("t(amount: long)").unwrap();
        let caps = datascript_compiler::Capabilities::full();
        let run_check = |q: &Query| Ok(datascript_compiler::check(&q.src, q.file, &mock, &caps));

        // A bad suggestion that produces a parse error must fail.
        let text = "from t | project amout\n";
        let file = FileId(0);
        let bad_syntax = Diagnostic::error(codes::DS1001, "unknown column `amout`")
            .primary(Span::new(file, 17, 22), "unknown")
            .help_with(
                "bad fix",
                Suggestion::replace(Span::new(file, 17, 22), "a b"),
            );
        let err = verify_check_fixes(text, file, &[bad_syntax], &run_check).unwrap_err();
        let DriverError::Failed(msg) = err else {
            panic!("expected DriverError::Failed, got {err:?}");
        };
        assert!(msg.contains("parse errors"), "{msg}");

        // A bad suggestion that parses but still emits the same code at the fixed location fails.
        let still_unknown = Diagnostic::error(codes::DS1001, "unknown column `amout`")
            .primary(Span::new(file, 17, 22), "unknown")
            .help_with(
                "wrong column",
                Suggestion::replace(Span::new(file, 17, 22), "still_missing"),
            );
        let err = verify_check_fixes(text, file, &[still_unknown], &run_check).unwrap_err();
        let DriverError::Failed(msg) = err else {
            panic!("expected DriverError::Failed, got {err:?}");
        };
        assert!(msg.contains("did not resolve DS1001"), "{msg}");
    }

    #[test]
    fn verify_check_fixes_allows_skipped_overlapping_parse_fix() {
        // `from t | where a = 1` produces a real parse error (DS0110) with a MachineApplicable
        // fix (`=` -> `==`). When an earlier applied suggestion overlaps its edit, DS0110's fix
        // is skipped, so DS0110 remains after reparsing and must not be reported as a *new*
        // parse error.
        let mock = schema::parse("t(a: long)").unwrap();
        let caps = datascript_compiler::Capabilities::full();
        let run_check = |q: &Query| Ok(datascript_compiler::check(&q.src, q.file, &mock, &caps));

        let text = "from t | where a = 1\n";
        let file = FileId(0);
        let orig_parse = datascript_syntax::parse(text, file);
        assert_eq!(orig_parse.errors.len(), 1);
        assert_eq!(orig_parse.errors[0].code, codes::DS0110);
        let eq_span = orig_parse.errors[0].primary.as_ref().unwrap().span;

        // First diagnostic overlaps `eq_span` with a no-op edit, causing DS0110's fix to be
        // skipped by `apply_suggestions_detailed`.
        let earlier = Diagnostic::warning(codes::DS2103, "earlier overlapping fix")
            .primary(Span::new(file, 0, 4), "earlier")
            .help_with("overlap", Suggestion::replace(eq_span, "="));
        verify_check_fixes(
            text,
            file,
            &[earlier, orig_parse.errors[0].clone()],
            &run_check,
        )
        .expect("skipped overlapping parse fix should not fail as a new parse error");
    }

    #[test]
    fn verify_check_fixes_allows_shifted_pre_existing_parse_error_span() {
        let mock = schema::parse("t(amount: long)").unwrap();
        let caps = datascript_compiler::Capabilities::full();
        let run_check = |q: &Query| Ok(datascript_compiler::check(&q.src, q.file, &mock, &caps));

        // Inserting `=` after `<` makes the pre-existing DS0101 span grow from 26..27 to 26..28.
        let text = "from t | where 1 < amount < 3\n";
        let file = FileId(0);
        let orig_parse = datascript_syntax::parse(text, file);
        assert_eq!(orig_parse.errors.len(), 1);
        let err_span = orig_parse.errors[0].primary.as_ref().unwrap().span;
        let boundary_fix = Diagnostic::warning(codes::DS2103, "boundary")
            .primary(Span::new(file, 0, 4), "boundary")
            .help_with(
                "insert =",
                Suggestion::replace(Span::empty(file, err_span.end), "="),
            );
        verify_check_fixes(text, file, &[boundary_fix], &run_check).expect(
            "pre-existing parse error whose span grew at boundary should still be recognized",
        );
    }

    #[test]
    fn verify_check_fixes_does_not_blame_pre_existing_error_with_same_code() {
        let mock = schema::parse("t(amount: long)").unwrap();
        let caps = datascript_compiler::Capabilities::full();
        let run_check = |q: &Query| Ok(datascript_compiler::check(&q.src, q.file, &mock, &caps));

        // `1 < amount < 3` has a pre-existing unfixed DS0101 on `<` (26..27).
        // A separate applied fix that also carries DS0101 and touches that region must not cause
        // the pre-existing unfixed DS0101 to be blamed as a new parse error.
        let text = "from t | where 1 < amount < 3\n";
        let file = FileId(0);
        let orig_parse = datascript_syntax::parse(text, file);
        assert_eq!(orig_parse.errors.len(), 1);
        assert_eq!(orig_parse.errors[0].code, codes::DS0101);
        let err_span = orig_parse.errors[0].primary.as_ref().unwrap().span;

        let same_code_fix = Diagnostic::error(codes::DS0101, "other DS0101")
            .primary(err_span, "other")
            .help_with(
                "insert =",
                Suggestion::replace(Span::empty(file, err_span.end), "="),
            );
        verify_check_fixes(text, file, &[same_code_fix], &run_check).expect(
            "pre-existing unfixed parse error must not be blamed on another fix with the same code",
        );
    }
}
