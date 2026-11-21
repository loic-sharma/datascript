//! Binder, type system, function catalog and logical plan IR for DataScript.
//!
//! - [`types`]: the scalar [`Type`]s and implicit conversions (LANGUAGE.md §4.2, §5).
//! - [`arrow`]: mapping between DataScript and Arrow types (LANGUAGE.md §5, §5.1).
//! - [`operators`]: operator typing (LANGUAGE.md §4.1), `DS2003` and `DS2009`.
//! - [`catalog`]: the [`FunctionCatalog`], overload resolution, `DS2002` and `DS2006`.
//! - [`capabilities`]: what a backend supports ([`Capabilities`]).
//!
//! This crate does not depend on DataFusion, tokio or the filesystem, and builds for
//! `wasm32-unknown-unknown`.

pub mod arrow;
mod builtins;
pub mod capabilities;
pub mod catalog;
pub mod operators;
pub mod types;

pub use capabilities::Capabilities;
pub use catalog::{FunctionCatalog, FunctionDef, FunctionKind};
pub use types::Type;

#[cfg(test)]
mod catalog_tests;
