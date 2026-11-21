//! The driver that runs cases through the real pipeline, as far as it exists.

use datascript_diagnostics::{DiagnosticSink, Diagnostics, FileId, Renderer, SourceMap};
use datascript_syntax::ast::{Operator, Stmt};

use crate::case::Mode;
use crate::driver::{CaseInput, Driver, DriverResult};

/// Runs cases through the DataScript pipeline. Modes whose stages do not exist yet are
/// unsupported, so their cases are ignored.
///
/// `mode: syntax` prints the CST dump ([`datascript_syntax::debug_tree`]), then the lexical and
/// syntax diagnostics after a blank line.
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

impl Driver for PipelineDriver {
    fn engine(&self) -> &str {
        &self.engine
    }

    fn supports(&self, mode: Mode) -> bool {
        mode == Mode::Syntax
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
        Ok(out)
    }

    fn output_order_defined(&self, query: &str) -> Option<bool> {
        Some(output_order_defined(query))
    }
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
