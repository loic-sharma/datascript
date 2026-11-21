//! The driver that runs cases through the real pipeline, as far as it exists.

use std::fmt::Write as _;

use datascript_diagnostics::{DiagnosticSink, Diagnostics, Renderer, SourceMap};

use crate::case::Mode;
use crate::driver::{CaseInput, Driver, DriverResult};

/// Runs cases through the DataScript pipeline. Modes whose stages do not exist yet are
/// unsupported, so their cases are ignored.
///
/// `mode: syntax` prints one line per token, `KIND@start..end "text"`, trivia included, then the
/// lexical diagnostics after a blank line. M1-02 replaces the token list with the CST dump.
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
        let lexed = datascript_syntax::lex(input.query, file);
        let mut out = String::new();
        for token in &lexed.tokens {
            let _ = writeln!(
                out,
                "{:?}@{}..{} {:?}",
                token.kind,
                token.start,
                token.end(),
                token.text(input.query)
            );
        }
        let mut diagnostics = Diagnostics::new();
        for d in lexed.diagnostics {
            diagnostics.emit(d);
        }
        let diagnostics = diagnostics.finish();
        if !diagnostics.is_empty() {
            out.push('\n');
            out.push_str(&Renderer::plain().render_all(&diagnostics, &sources));
        }
        Ok(out)
    }
}
