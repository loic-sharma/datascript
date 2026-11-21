//! Binder, type system, function catalog and logical plan IR for DataScript.
//!
//! - [`types`]: the scalar [`Type`]s and implicit conversions (LANGUAGE.md §4.2, §5).
//! - [`arrow`]: mapping between DataScript and Arrow types (LANGUAGE.md §5, §5.1).
//! - [`operators`]: operator typing (LANGUAGE.md §4.1), `DS2003` and `DS2009`.
//! - [`catalog`]: the [`FunctionCatalog`], overload resolution, `DS2002` and `DS2006`.
//! - [`capabilities`]: what a backend supports ([`Capabilities`]).
//! - [`source`]: [`SourceRef`], [`SchemaProvider`] and [`SourceError`] (D-022, D-023).
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
pub mod ir;
pub mod operators;
pub mod source;
pub mod types;

pub use binder::{BoundQuery, StageSchema};
pub use capabilities::Capabilities;
pub use catalog::{FunctionCatalog, FunctionDef, FunctionKind};
pub use ir::Program;
pub use source::{NoSchemas, SchemaLookup, SchemaProvider, SourceError, SourceRef};
pub use types::Type;

use datascript_diagnostics::codes::DS9001;
use datascript_diagnostics::{Diagnostic, DiagnosticSink, Diagnostics, FileId, SourceFile};

/// What [`check`] found out about a program.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    /// One entry per query statement. For now this is the plan itself; M1-05 and M1-06 put a
    /// typed HIR between the binder and the planner.
    pub hir: Vec<BoundQuery>,
    /// Syntax and semantic diagnostics, sorted and capped.
    pub diagnostics: Vec<Diagnostic>,
    /// The schema after each pipeline stage whose schema is known.
    pub stage_schemas: Vec<StageSchema>,
    /// Sources the provider answered [`SchemaLookup::Pending`] for, sorted. The session fetches
    /// them and checks again (D-022).
    pub pending_sources: Vec<SourceRef>,
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
/// `caps` is not used yet; the capability pass is M1-17.
pub fn check(
    src: &SourceFile,
    file: FileId,
    schemas: &dyn SchemaProvider,
    _caps: &Capabilities,
) -> Analysis {
    let parse = datascript_syntax::parse(src.text(), file);
    let mut binder = binder::Binder::new(file, src.text(), schemas);
    let hir = binder.bind_program(&parse.program());
    let mut diagnostics = Diagnostics::new();
    for d in parse.errors.into_iter().chain(binder.diagnostics) {
        diagnostics.emit(d);
    }
    Analysis {
        hir,
        diagnostics: diagnostics.finish(),
        stage_schemas: binder.stage_schemas,
        pending_sources: binder.pending.into_iter().collect(),
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
    let queries = analysis
        .hir
        .into_iter()
        .filter_map(|query| query.plan)
        .map(|root| ir::QueryPlan {
            output_schema: root.schema.clone(),
            root,
        })
        .collect();
    Ok(Program { queries })
}

#[cfg(test)]
mod catalog_tests;
