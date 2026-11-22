//! Binder, type system, function catalog and logical plan IR for DataScript.
//!
//! - [`types`]: the scalar [`Type`]s and implicit conversions (LANGUAGE.md §4.2, §5).
//! - [`arrow`]: mapping between DataScript and Arrow types (LANGUAGE.md §5, §5.1).
//! - [`operators`]: operator typing (LANGUAGE.md §4.1), `DS2003` and `DS2009`.
//! - [`catalog`]: the [`FunctionCatalog`], overload resolution, `DS2002` and `DS2006`.
//! - [`capabilities`]: what a backend supports ([`Capabilities`]).
//! - [`source`]: [`SourceRef`], [`SchemaProvider`] and [`SourceError`] (D-022, D-023).
//! - [`hir`]: the typed HIR the binder produces.
//! - [`ir`]: the logical plan IR and its `explain` printout.
//! - [`check`] and [`compile`]: the entry points that parse, bind and plan a program.
//!
//! This crate does not depend on DataFusion, tokio or the filesystem, and builds for
//! `wasm32-unknown-unknown`.

pub mod arrow;
mod binder;
mod builtins;
pub mod capabilities;
pub mod catalog;
pub mod hir;
pub mod ir;
pub mod operators;
mod plan;
mod render;
pub mod source;
pub mod types;

pub use binder::StageSchema;
pub use capabilities::Capabilities;
pub use catalog::{FunctionCatalog, FunctionDef, FunctionKind};
pub use ir::Program;
pub use source::{
    FileType, NoSchemas, OptionValue, ReadProblem, ReaderCall, SchemaLookup, SchemaProvider,
    SourceError, SourceRef,
};
pub use types::Type;

use datascript_diagnostics::codes::DS9001;
use datascript_diagnostics::{Diagnostic, DiagnosticSink, Diagnostics, FileId, SourceFile};

/// What [`check`] found out about a program.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    /// The bound program: every name resolved, every expression typed.
    pub hir: hir::Hir,
    /// Syntax and semantic diagnostics, sorted and capped.
    pub diagnostics: Vec<Diagnostic>,
    /// The schema after each pipeline stage whose schema is known.
    pub stage_schemas: Vec<StageSchema>,
    /// Sources the provider answered [`SchemaLookup::Pending`] for, sorted. The session fetches
    /// them and checks again (D-022).
    pub pending_sources: Vec<SourceRef>,
    /// The rules of LANGUAGE.md the engine does not follow, from [`Capabilities::deviations`],
    /// for frontends to show. They produce no diagnostic.
    pub deviations: Vec<capabilities::Deviation>,
}

impl Analysis {
    /// Whether any diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Parses and binds `src`, the text of file `file` (diagnostic spans point into it).
///
/// Schemas come from `schemas`. A source it answers `Pending` for is listed in
/// [`Analysis::pending_sources`] without a diagnostic, and its columns cause no errors.
///
/// Whatever `caps` says the engine cannot do (a function, a `join` kind, a file format) is
/// `DS4001` at the place it is used; `caps.deviations` are copied to
/// [`Analysis::deviations`].
pub fn check(
    src: &SourceFile,
    file: FileId,
    schemas: &dyn SchemaProvider,
    caps: &Capabilities,
) -> Analysis {
    let parse = datascript_syntax::parse(src.text(), file);
    let program = parse.program();
    let mut binder = binder::Binder::new(file, src.text(), schemas, caps, &program, &parse.errors);
    let hir = binder.bind_program(&program);
    let mut diagnostics = Diagnostics::new();
    for d in parse.errors.into_iter().chain(binder.diagnostics) {
        diagnostics.emit(d);
    }
    Analysis {
        hir,
        diagnostics: diagnostics.finish(),
        stage_schemas: binder.stage_schemas,
        pending_sources: binder.pending.into_iter().collect(),
        deviations: caps.deviations.iter().cloned().collect(),
    }
}

/// Checks `src` and, if it has no errors, plans it.
///
/// Every source must be known to `schemas`: a pending source is an internal error (`DS9001`),
/// because the session resolves sources before compiling.
pub fn compile(
    src: &SourceFile,
    file: FileId,
    schemas: &dyn SchemaProvider,
    caps: &Capabilities,
) -> Result<Program, Vec<Diagnostic>> {
    let analysis = check(src, file, schemas, caps);
    program_of(analysis, file)
}

/// The program of an analysis without errors or pending sources; otherwise its diagnostics.
pub fn program_of(analysis: Analysis, file: FileId) -> Result<Program, Vec<Diagnostic>> {
    let mut diagnostics = analysis.diagnostics;
    if let Some(source) = analysis.pending_sources.first() {
        diagnostics.push(
            Diagnostic::error(
                DS9001,
                format!("internal error: the schema of {source} was never looked up"),
            )
            .in_file(file)
            .note("this is a bug in DataScript; please report it"),
        );
    }
    if diagnostics.iter().any(Diagnostic::is_error) {
        return Err(diagnostics);
    }
    let mut queries = Vec::new();
    let mut internal = Vec::new();
    for query in analysis.hir.queries() {
        match plan::plan_query(query) {
            Ok(plan) => queries.push(plan),
            Err(d) => internal.push(d),
        }
    }
    if !internal.is_empty() {
        return Err(internal);
    }
    Ok(Program { queries })
}

#[cfg(test)]
mod binder_tests;
#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod plan_tests;
