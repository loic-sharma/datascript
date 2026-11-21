//! What a backend supports. Engines return a [`Capabilities`]; the compiler's capability pass
//! (M1-17) checks a bound program against it and reports `DS4001` at the call site, so `check`
//! reports it in the LSP and WASM too, without executing anything (ARCHITECTURE.md).

use std::collections::BTreeSet;
use std::fmt;

use crate::catalog::FunctionCatalog;

/// A `join` kind (LANGUAGE.md §3.2).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum JoinKind {
    Inner,
    LeftOuter,
    RightOuter,
    FullOuter,
    LeftSemi,
    LeftAnti,
    RightSemi,
    RightAnti,
}

impl JoinKind {
    /// Every kind, in the order of LANGUAGE.md §3.2.
    pub const ALL: [JoinKind; 8] = [
        JoinKind::Inner,
        JoinKind::LeftOuter,
        JoinKind::RightOuter,
        JoinKind::FullOuter,
        JoinKind::LeftSemi,
        JoinKind::LeftAnti,
        JoinKind::RightSemi,
        JoinKind::RightAnti,
    ];

    /// The name written after `kind =`.
    pub fn name(self) -> &'static str {
        match self {
            JoinKind::Inner => "inner",
            JoinKind::LeftOuter => "leftouter",
            JoinKind::RightOuter => "rightouter",
            JoinKind::FullOuter => "fullouter",
            JoinKind::LeftSemi => "leftsemi",
            JoinKind::LeftAnti => "leftanti",
            JoinKind::RightSemi => "rightsemi",
            JoinKind::RightAnti => "rightanti",
        }
    }

    /// The kind with this name.
    pub fn from_name(name: &str) -> Option<JoinKind> {
        JoinKind::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Semi and anti joins return the columns of one side only.
    pub fn one_side(self) -> Option<crate::hir::JoinSide> {
        use crate::hir::JoinSide;
        match self {
            JoinKind::LeftSemi | JoinKind::LeftAnti => Some(JoinSide::Left),
            JoinKind::RightSemi | JoinKind::RightAnti => Some(JoinSide::Right),
            _ => None,
        }
    }
}

impl fmt::Display for JoinKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A file format a source can be read as (LANGUAGE.md §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceFormat {
    Csv,
    Tsv,
    Parquet,
    /// A JSON array or newline-delimited JSON, detected.
    Json,
    /// Newline-delimited JSON (`.ndjson`, `.jsonl`).
    Ndjson,
}

impl SourceFormat {
    /// Every format.
    pub const ALL: [SourceFormat; 5] = [
        SourceFormat::Csv,
        SourceFormat::Tsv,
        SourceFormat::Parquet,
        SourceFormat::Json,
        SourceFormat::Ndjson,
    ];

    /// A short lowercase name.
    pub fn name(self) -> &'static str {
        match self {
            SourceFormat::Csv => "csv",
            SourceFormat::Tsv => "tsv",
            SourceFormat::Parquet => "parquet",
            SourceFormat::Json => "json",
            SourceFormat::Ndjson => "ndjson",
        }
    }
}

/// A rule of LANGUAGE.md that a backend knowingly does not follow (D-014). Golden cases record
/// the difference in an `---- expected[<engine>]` section.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Deviation {
    /// Integer overflow wraps or saturates instead of failing with `DS4003`.
    IntegerOverflowUnchecked,
    /// Integer division or modulo by zero does not fail with `DS4003`.
    IntegerDivisionByZeroUnchecked,
    /// Anything else, described in words.
    Other {
        /// A stable kebab-case identifier.
        id: String,
        /// What differs, in plain words.
        description: String,
    },
}

impl Deviation {
    /// What differs, in plain words.
    pub fn description(&self) -> &str {
        match self {
            Deviation::IntegerOverflowUnchecked => "integer overflow is not reported as an error",
            Deviation::IntegerDivisionByZeroUnchecked => {
                "integer division or modulo by zero is not reported as an error"
            }
            Deviation::Other { description, .. } => description,
        }
    }
}

/// Which functions a backend implements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionSupport {
    /// Every function in the catalog.
    All,
    /// Only these.
    Only(BTreeSet<String>),
}

/// What a backend supports. A plain data type, defined in the compiler so that `check` can use
/// it without depending on an engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// Implemented functions.
    pub functions: FunctionSupport,
    /// Implemented `join` kinds.
    pub join_kinds: BTreeSet<JoinKind>,
    /// Readable source formats.
    pub formats: BTreeSet<SourceFormat>,
    /// Rules of LANGUAGE.md the backend does not follow.
    pub deviations: BTreeSet<Deviation>,
}

impl Capabilities {
    /// Everything LANGUAGE.md describes, with no deviations. Useful for `check` without an
    /// engine, and in tests.
    pub fn full() -> Self {
        Capabilities {
            functions: FunctionSupport::All,
            join_kinds: JoinKind::ALL.into_iter().collect(),
            formats: SourceFormat::ALL.into_iter().collect(),
            deviations: BTreeSet::new(),
        }
    }

    /// Whether the function called `name` is implemented. Names not in the catalog are never
    /// supported.
    pub fn supports_function(&self, catalog: &FunctionCatalog, name: &str) -> bool {
        catalog.lookup(name).is_ok()
            && match &self.functions {
                FunctionSupport::All => true,
                FunctionSupport::Only(names) => names.contains(name),
            }
    }

    /// Whether this `join` kind is implemented.
    pub fn supports_join(&self, kind: JoinKind) -> bool {
        self.join_kinds.contains(&kind)
    }

    /// Whether this format can be read.
    pub fn supports_format(&self, format: SourceFormat) -> bool {
        self.formats.contains(&format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_kind_names_round_trip() {
        for kind in JoinKind::ALL {
            assert_eq!(JoinKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(JoinKind::from_name("innerunique"), None);
    }

    #[test]
    fn full_supports_everything() {
        let caps = Capabilities::full();
        let catalog = FunctionCatalog::builtin();
        assert!(
            catalog
                .names()
                .all(|name| caps.supports_function(catalog, name))
        );
        assert!(!caps.supports_function(catalog, "no_such_function"));
        assert!(JoinKind::ALL.into_iter().all(|k| caps.supports_join(k)));
        assert!(
            SourceFormat::ALL
                .into_iter()
                .all(|f| caps.supports_format(f))
        );
        assert!(caps.deviations.is_empty());
    }

    #[test]
    fn restricted() {
        let catalog = FunctionCatalog::builtin();
        let caps = Capabilities {
            functions: FunctionSupport::Only(["strlen".to_string()].into()),
            join_kinds: [JoinKind::Inner].into(),
            formats: [SourceFormat::Csv].into(),
            deviations: [Deviation::IntegerOverflowUnchecked].into(),
        };
        assert!(caps.supports_function(catalog, "strlen"));
        assert!(!caps.supports_function(catalog, "tolower"));
        assert!(!caps.supports_join(JoinKind::FullOuter));
        assert!(!caps.supports_format(SourceFormat::Parquet));
        let deviation = caps.deviations.iter().next().unwrap();
        assert_eq!(
            deviation.description(),
            "integer overflow is not reported as an error"
        );
    }
}
