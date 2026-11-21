//! DataScript scalar types (LANGUAGE.md §5) and implicit conversions (LANGUAGE.md §4.2).

use std::fmt;

/// A DataScript scalar type.
///
/// Besides the types of LANGUAGE.md §5 there are two internal types that users never write:
///
/// - [`Type::Null`] is the type of an untyped `null` literal. It converts to any type, and the
///   binder replaces it with the type its context needs (or reports `DS2009` when there is none).
/// - [`Type::Error`] is given to an expression that already has a reported error. It is
///   compatible with every type, so one mistake produces one diagnostic (DIAGNOSTICS.md rule 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Type {
    /// `bool`
    Bool,
    /// `int`: 32-bit integer. Only from `toint()` and `int(null)` (D-021).
    Int,
    /// `long`: 64-bit integer, the default integer type.
    Long,
    /// `real`: 64-bit float.
    Real,
    /// `decimal` (M2).
    Decimal,
    /// `string`
    String,
    /// `datetime`: UTC instant with microsecond precision.
    DateTime,
    /// `timespan`: duration with microsecond precision.
    TimeSpan,
    /// `guid` (M2).
    Guid,
    /// `dynamic` (M2).
    Dynamic,
    /// The type of an untyped `null` literal (internal).
    Null,
    /// The type of an expression that already has an error (internal).
    Error,
}

impl Type {
    /// Every type a user can name, in the order of LANGUAGE.md §5.
    pub const NAMED: [Type; 10] = [
        Type::Bool,
        Type::Int,
        Type::Long,
        Type::Real,
        Type::Decimal,
        Type::String,
        Type::DateTime,
        Type::TimeSpan,
        Type::Guid,
        Type::Dynamic,
    ];

    /// The name users see in messages and write in typed nulls (`long(null)`).
    pub fn name(self) -> &'static str {
        match self {
            Type::Bool => "bool",
            Type::Int => "int",
            Type::Long => "long",
            Type::Real => "real",
            Type::Decimal => "decimal",
            Type::String => "string",
            Type::DateTime => "datetime",
            Type::TimeSpan => "timespan",
            Type::Guid => "guid",
            Type::Dynamic => "dynamic",
            // Shown only if an internal type leaks into a message, which is a bug.
            Type::Null => "null",
            Type::Error => "unknown",
        }
    }

    /// The type with this name, as written in a typed null (`long(null)`). Internal types have
    /// no name.
    pub fn from_name(name: &str) -> Option<Type> {
        Type::NAMED.into_iter().find(|ty| ty.name() == name)
    }

    /// `int`, `long` or `real`: the `num` of LANGUAGE.md §4.1.
    pub fn is_numeric(self) -> bool {
        matches!(self, Type::Int | Type::Long | Type::Real)
    }

    /// `int` or `long`.
    pub fn is_integer(self) -> bool {
        matches!(self, Type::Int | Type::Long)
    }

    /// Whether this is [`Type::Error`].
    pub fn is_error(self) -> bool {
        self == Type::Error
    }

    /// Whether this is [`Type::Null`].
    pub fn is_null(self) -> bool {
        self == Type::Null
    }

    /// The smallest type both `self` and `other` convert to implicitly (LANGUAGE.md §4.2), used
    /// wherever two types meet: operator operands, `iff` branches, `union` columns, glob schemas.
    ///
    /// - Equal types meet at themselves.
    /// - [`Type::Error`] meets anything at `Error`, so callers stay silent.
    /// - [`Type::Null`] meets anything at the other type.
    /// - Numbers widen `int` → `long` → `real`; `decimal` widens to `real` when mixed with
    ///   `real`.
    /// - Nothing else converts implicitly: `None`.
    pub fn common(self, other: Type) -> Option<Type> {
        use Type::*;
        Some(match (self, other) {
            (a, b) if a == b => a,
            (Error, _) | (_, Error) => Error,
            (Null, t) | (t, Null) => t,
            (Int, Long) | (Long, Int) => Long,
            (Int | Long, Real) | (Real, Int | Long) => Real,
            (Decimal, Real) | (Real, Decimal) => Real,
            _ => return None,
        })
    }

    /// Whether a value of type `self` can be used where `target` is expected without an explicit
    /// conversion (LANGUAGE.md §4.2).
    pub fn coerces_to(self, target: Type) -> bool {
        self.common(target) == Some(target) || self.is_error() || target.is_error()
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of types a function parameter or operator operand accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeClass {
    /// Exactly this type (or one that widens to it).
    Exact(Type),
    /// `int` or `long`.
    Integer,
    /// `int`, `long` or `real`.
    Numeric,
    /// Types with an order: numbers, `string`, `datetime`, `timespan`, `bool`
    /// (LANGUAGE.md §4.1, `<`).
    Comparable,
    /// Any type a user can name.
    Any,
}

impl TypeClass {
    /// Whether a value of type `ty` is accepted. [`Type::Null`] and [`Type::Error`] are
    /// accepted everywhere.
    pub fn accepts(self, ty: Type) -> bool {
        match (self, ty) {
            (_, Type::Null | Type::Error) => true,
            (TypeClass::Exact(target), ty) => ty.coerces_to(target),
            (TypeClass::Integer, ty) => ty.is_integer(),
            (TypeClass::Numeric, ty) => ty.is_numeric(),
            (TypeClass::Comparable, ty) => {
                ty.is_numeric()
                    || matches!(
                        ty,
                        Type::String | Type::DateTime | Type::TimeSpan | Type::Bool
                    )
            }
            (TypeClass::Any, _) => true,
        }
    }

    /// How the class reads in a signature or a message: `long`, `number`, `any`.
    pub fn name(self) -> &'static str {
        match self {
            TypeClass::Exact(ty) => ty.name(),
            TypeClass::Integer => "integer",
            TypeClass::Numeric => "number",
            TypeClass::Comparable => "comparable",
            TypeClass::Any => "any",
        }
    }

    /// The class described for a message, with backticks: "`long`", "a number".
    pub fn describe(self) -> String {
        match self {
            TypeClass::Exact(ty) => format!("`{ty}`"),
            TypeClass::Integer => "an integer (`int` or `long`)".to_string(),
            TypeClass::Numeric => "a number".to_string(),
            TypeClass::Comparable => {
                "a number, `string`, `datetime`, `timespan` or `bool`".to_string()
            }
            TypeClass::Any => "any value".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Type::*;
    use super::*;

    #[test]
    fn names_round_trip() {
        for ty in Type::NAMED {
            assert_eq!(Type::from_name(ty.name()), Some(ty));
        }
        assert_eq!(Type::from_name("null"), None);
        assert_eq!(Type::from_name("unknown"), None);
        assert_eq!(Type::from_name("Long"), None); // case-sensitive
    }

    #[test]
    fn numeric_widening() {
        assert_eq!(Int.common(Long), Some(Long));
        assert_eq!(Long.common(Int), Some(Long));
        assert_eq!(Int.common(Real), Some(Real));
        assert_eq!(Long.common(Real), Some(Real));
        assert_eq!(Real.common(Int), Some(Real));
        assert_eq!(Decimal.common(Real), Some(Real));
        assert_eq!(Int.common(Int), Some(Int));
        // Widening goes one way only.
        assert!(Int.coerces_to(Long));
        assert!(Long.coerces_to(Real));
        assert!(Int.coerces_to(Real));
        assert!(!Long.coerces_to(Int));
        assert!(!Real.coerces_to(Long));
    }

    #[test]
    fn no_other_implicit_conversions() {
        assert_eq!(String.common(Long), None);
        assert_eq!(Bool.common(Long), None);
        assert_eq!(DateTime.common(TimeSpan), None);
        assert_eq!(Decimal.common(Long), None);
        assert_eq!(Dynamic.common(String), None);
        assert!(!Long.coerces_to(String));
        assert!(!String.coerces_to(Long));
    }

    #[test]
    fn null_takes_the_other_type() {
        for ty in Type::NAMED {
            assert_eq!(Null.common(ty), Some(ty));
            assert_eq!(ty.common(Null), Some(ty));
            assert!(Null.coerces_to(ty));
        }
        assert_eq!(Null.common(Null), Some(Null));
    }

    #[test]
    fn error_unifies_with_everything() {
        for ty in Type::NAMED.into_iter().chain([Null]) {
            assert_eq!(Error.common(ty), Some(Error));
            assert_eq!(ty.common(Error), Some(Error));
            assert!(Error.coerces_to(ty));
            assert!(ty.coerces_to(Error));
        }
    }

    #[test]
    fn classes() {
        assert!(TypeClass::Numeric.accepts(Int));
        assert!(TypeClass::Numeric.accepts(Real));
        assert!(!TypeClass::Numeric.accepts(Decimal));
        assert!(!TypeClass::Numeric.accepts(String));
        assert!(TypeClass::Integer.accepts(Long));
        assert!(!TypeClass::Integer.accepts(Real));
        assert!(TypeClass::Comparable.accepts(String));
        assert!(TypeClass::Comparable.accepts(Bool));
        assert!(!TypeClass::Comparable.accepts(Dynamic));
        assert!(TypeClass::Exact(Real).accepts(Int));
        assert!(!TypeClass::Exact(Int).accepts(Long));
        for class in [
            TypeClass::Exact(String),
            TypeClass::Integer,
            TypeClass::Numeric,
            TypeClass::Comparable,
            TypeClass::Any,
        ] {
            assert!(class.accepts(Null));
            assert!(class.accepts(Error));
        }
    }
}
