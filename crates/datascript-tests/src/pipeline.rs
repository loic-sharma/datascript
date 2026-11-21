//! The driver that runs cases through the real pipeline, as far as it exists.

use std::sync::Arc;

use datascript_compiler::Capabilities;
use datascript_datafusion::{DataFusionConfig, DataFusionEngine};
use datascript_diagnostics::{
    Diagnostic, DiagnosticSink, Diagnostics, FileId, Renderer, SourceFile, SourceMap,
};
use datascript_engine::{FixedClock, Session, table::format_table};
use datascript_syntax::ast::{Operator, Stmt};

use crate::case::Mode;
use crate::driver::{CaseInput, Driver, DriverError, DriverResult};
use crate::fixes::apply_suggestions;

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
        let mut sources = SourceMap::new();
        let file = sources.add("query", input.query);
        Query {
            sources,
            src: SourceFile::new("query", input.query),
            file,
        }
    }

    fn render(&self, diagnostics: &[Diagnostic]) -> String {
        Renderer::plain().render_all(diagnostics, &self.sources)
    }
}

fn session(input: &CaseInput<'_>) -> Session {
    let engine = DataFusionEngine::new(DataFusionConfig {
        target_partitions: input.config.target_partitions,
        batch_size: input.config.batch_size,
    });
    Session::new(Arc::new(engine), Arc::new(FixedClock(input.now_micros)))
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
        let analysis = match input.schema {
            Some(mock) => datascript_compiler::check(&query.src, query.file, mock, &caps),
            None => {
                let session = session(input);
                let ctx = session.exec_context(input.base_dir());
                block_on(session.check(&query.src, query.file, &ctx))?
            }
        };
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
                let session = session(input);
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
        let session = session(input);
        let ctx = session.exec_context(input.base_dir());
        let output = block_on(session.run(&query.src, query.file, &ctx))?;
        let mut out = query.render(&output.diagnostics);
        for (i, result) in output.results.iter().enumerate() {
            if i > 0 || !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format_table(&result.schema, &result.batches));
        }
        Ok(out)
    }

    fn output_order_defined(&self, query: &str) -> Option<bool> {
        Some(output_order_defined(query))
    }
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
    use super::output_order_defined;

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
}
