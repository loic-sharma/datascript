//! Mapping between DataScript types and Arrow types.
//!
//! - [`to_arrow`] gives the canonical Arrow field of LANGUAGE.md §5, with Arrow's canonical
//!   extension type where the storage type is ambiguous (D-024).
//! - [`from_arrow`] is its exact inverse, for results and schemas that engines already produced
//!   in canonical form.
//! - [`from_arrow_source`] is the lenient read mapping of LANGUAGE.md §5.1, for columns as they
//!   appear in files (D-012, D-021).

use std::fmt;
use std::sync::Arc;

use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use arrow_schema::{DataType, Field, TimeUnit};

use crate::types::Type;

/// Extension type name of `dynamic` values stored as JSON text.
pub const JSON_EXTENSION: &str = "arrow.json";
/// Extension type name of `guid` values stored as 16 bytes.
pub const UUID_EXTENSION: &str = "arrow.uuid";

/// Timezone of every canonical `datetime` column.
pub const UTC: &str = "UTC";

/// An Arrow type that has no DataScript equivalent. Such columns can be projected but not used
/// in expressions (`DS2008`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedArrowType {
    /// The Arrow type as found.
    pub data_type: DataType,
    /// The field's extension type name, if any.
    pub extension: Option<String>,
}

impl fmt::Display for UnsupportedArrowType {
    /// Names the Arrow type (DIAGNOSTICS.md rule 6 allows it: it comes from the user's file).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.extension {
            Some(ext) => write!(f, "{} (extension type `{ext}`)", self.data_type),
            None => write!(f, "{}", self.data_type),
        }
    }
}

impl std::error::Error for UnsupportedArrowType {}

/// The canonical Arrow storage type of `ty` (LANGUAGE.md §5).
///
/// [`Type::Null`] maps to Arrow `Null`. [`Type::Error`] never reaches a plan (the program has an
/// error), and also maps to `Null` so callers need no special case.
pub fn to_arrow_type(ty: Type) -> DataType {
    match ty {
        Type::Bool => DataType::Boolean,
        Type::Int => DataType::Int32,
        Type::Long => DataType::Int64,
        Type::Real => DataType::Float64,
        Type::Decimal => DataType::Decimal128(38, 18),
        Type::String | Type::Dynamic => DataType::Utf8,
        Type::DateTime => DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from(UTC))),
        Type::TimeSpan => DataType::Duration(TimeUnit::Microsecond),
        Type::Guid => DataType::FixedSizeBinary(16),
        Type::Null | Type::Error => DataType::Null,
    }
}

/// The extension type name `ty` carries in Arrow field metadata, if its storage type alone is
/// ambiguous (D-024).
pub fn extension_name(ty: Type) -> Option<&'static str> {
    match ty {
        Type::Dynamic => Some(JSON_EXTENSION),
        Type::Guid => Some(UUID_EXTENSION),
        _ => None,
    }
}

/// The canonical, nullable Arrow field for a column `name` of type `ty`, with extension type
/// metadata for `dynamic` and `guid` (D-024).
pub fn to_arrow(name: &str, ty: Type) -> Field {
    let field = Field::new(name, to_arrow_type(ty), true);
    match extension_name(ty) {
        Some(ext) => field.with_metadata(
            [(EXTENSION_TYPE_NAME_KEY.to_string(), ext.to_string())]
                .into_iter()
                .collect(),
        ),
        None => field,
    }
}

/// The DataScript type of a field in canonical form (as produced by [`to_arrow`]).
///
/// Anything that [`to_arrow`] does not produce is an error; use [`from_arrow_source`] for
/// columns read from files.
pub fn from_arrow(field: &Field) -> Result<Type, UnsupportedArrowType> {
    let ext = field.extension_type_name();
    let found = Type::NAMED
        .into_iter()
        .find(|&ty| to_arrow_type(ty) == *field.data_type() && extension_name(ty) == ext);
    // `string` and `dynamic` share Utf8: the loop finds `string` first unless the extension
    // matches, and `extension_name(String)` is `None`, so a Utf8 field with an unknown extension
    // is rejected here. That is intended: canonical results carry only our extensions.
    found.ok_or_else(|| unsupported(field))
}

/// The DataScript type of a column read from a file (LANGUAGE.md §5.1). Engines cast the column
/// to the canonical type of the result at the scan.
///
/// Columns that would be `dynamic` or `guid` (JSON text with `arrow.json`, nested types,
/// `arrow.uuid`) are unsupported until those types land in M2 (D-039), so they can be projected
/// but not used in expressions.
pub fn from_arrow_source(field: &Field) -> Result<Type, UnsupportedArrowType> {
    let ext = field.extension_type_name();
    let ty = match field.data_type() {
        DataType::Boolean => Type::Bool,
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => Type::Long,
        DataType::Float16 | DataType::Float32 | DataType::Float64 => Type::Real,
        // `real` in M1; `decimal` from M2 (LANGUAGE.md §5.1).
        DataType::Decimal32(..)
        | DataType::Decimal64(..)
        | DataType::Decimal128(..)
        | DataType::Decimal256(..) => Type::Real,
        // `dynamic` from M2 (M2-03).
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
            if ext == Some(JSON_EXTENSION) =>
        {
            return Err(unsupported(field));
        }
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Type::String,
        DataType::Dictionary(_, value)
            if matches!(
                value.as_ref(),
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
            ) =>
        {
            Type::String
        }
        DataType::Timestamp(_, _) | DataType::Date32 | DataType::Date64 => Type::DateTime,
        DataType::Duration(_) => Type::TimeSpan,
        // `guid` (M2-10) and `dynamic` (M2-03) are not in M1; nested columns and
        // `FixedSizeBinary(16)` with `arrow.uuid` fall through to unsupported.
        _ => return Err(unsupported(field)),
    };
    Ok(ty)
}

fn unsupported(field: &Field) -> UnsupportedArrowType {
    UnsupportedArrowType {
        data_type: field.data_type().clone(),
        extension: field.extension_type_name().map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::{Fields, IntervalUnit};

    fn with_ext(field: Field, ext: &str) -> Field {
        field.with_metadata(
            [(EXTENSION_TYPE_NAME_KEY.to_string(), ext.to_string())]
                .into_iter()
                .collect(),
        )
    }

    fn source(dt: DataType) -> Result<Type, UnsupportedArrowType> {
        from_arrow_source(&Field::new("c", dt, true))
    }

    // LANGUAGE.md §5, one assertion per row.
    #[test]
    fn canonical_types() {
        let utc = Some(Arc::from(UTC));
        let cases = [
            (Type::Bool, DataType::Boolean, None),
            (Type::Int, DataType::Int32, None),
            (Type::Long, DataType::Int64, None),
            (Type::Real, DataType::Float64, None),
            (Type::Decimal, DataType::Decimal128(38, 18), None),
            (Type::String, DataType::Utf8, None),
            (
                Type::DateTime,
                DataType::Timestamp(TimeUnit::Microsecond, utc),
                None,
            ),
            (
                Type::TimeSpan,
                DataType::Duration(TimeUnit::Microsecond),
                None,
            ),
            (
                Type::Guid,
                DataType::FixedSizeBinary(16),
                Some(UUID_EXTENSION),
            ),
            (Type::Dynamic, DataType::Utf8, Some(JSON_EXTENSION)),
        ];
        assert_eq!(cases.len(), Type::NAMED.len());
        for (ty, dt, ext) in cases {
            let field = to_arrow("col", ty);
            assert_eq!(field.name(), "col");
            assert!(field.is_nullable());
            assert_eq!(field.data_type(), &dt, "{ty}");
            assert_eq!(field.extension_type_name(), ext, "{ty}");
        }
    }

    #[test]
    fn canonical_round_trip() {
        for ty in Type::NAMED {
            let field = to_arrow("c", ty);
            assert_eq!(from_arrow(&field), Ok(ty), "{ty}");
            // D-021: `int` reads as `long`; decimals read as `real` in M1; `guid` and
            // `dynamic` are unsupported until M2 (D-039).
            let expected = match ty {
                Type::Int => Ok(Type::Long),
                Type::Decimal => Ok(Type::Real),
                Type::Guid | Type::Dynamic => Err(unsupported(&field)),
                other => Ok(other),
            };
            assert_eq!(from_arrow_source(&field), expected, "{ty}");
        }
    }

    #[test]
    fn from_arrow_is_strict() {
        let not_canonical = [
            DataType::Int16,
            DataType::Utf8View,
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            DataType::Timestamp(TimeUnit::Microsecond, None),
            DataType::FixedSizeBinary(16), // no `arrow.uuid`
        ];
        for dt in &not_canonical {
            let field = Field::new("c", dt.clone(), true);
            assert!(from_arrow(&field).is_err(), "{dt}");
        }
        let field = with_ext(Field::new("c", DataType::Utf8, true), "my.ext");
        assert!(from_arrow(&field).is_err());
    }

    // LANGUAGE.md §5.1, one test per row.
    #[test]
    fn read_boolean() {
        assert_eq!(source(DataType::Boolean), Ok(Type::Bool));
    }

    #[test]
    fn read_integers_as_long() {
        for dt in [
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
        ] {
            assert_eq!(source(dt), Ok(Type::Long));
        }
    }

    #[test]
    fn read_uint64_as_long() {
        assert_eq!(source(DataType::UInt64), Ok(Type::Long));
    }

    #[test]
    fn read_floats_as_real() {
        for dt in [DataType::Float16, DataType::Float32, DataType::Float64] {
            assert_eq!(source(dt), Ok(Type::Real));
        }
    }

    #[test]
    fn read_decimals_as_real_in_m1() {
        for dt in [
            DataType::Decimal32(9, 2),
            DataType::Decimal64(18, 2),
            DataType::Decimal128(38, 10),
            DataType::Decimal256(76, 10),
        ] {
            assert_eq!(source(dt), Ok(Type::Real));
        }
    }

    #[test]
    fn read_strings() {
        for dt in [DataType::Utf8, DataType::LargeUtf8, DataType::Utf8View] {
            assert_eq!(source(dt.clone()), Ok(Type::String));
            let dict = DataType::Dictionary(Box::new(DataType::Int32), Box::new(dt));
            assert_eq!(source(dict), Ok(Type::String));
        }
        // An unknown extension on Utf8 is still a string.
        let field = with_ext(Field::new("c", DataType::Utf8, true), "my.ext");
        assert_eq!(from_arrow_source(&field), Ok(Type::String));
        // A dictionary of something else is not.
        let dict = DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Int64));
        assert!(source(dict).is_err());
    }

    #[test]
    fn read_timestamps_as_datetime() {
        for unit in [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
        ] {
            assert_eq!(source(DataType::Timestamp(unit, None)), Ok(Type::DateTime));
            assert_eq!(
                source(DataType::Timestamp(unit, Some(Arc::from("+02:00")))),
                Ok(Type::DateTime)
            );
        }
    }

    #[test]
    fn read_dates_as_datetime() {
        assert_eq!(source(DataType::Date32), Ok(Type::DateTime));
        assert_eq!(source(DataType::Date64), Ok(Type::DateTime));
    }

    #[test]
    fn read_durations_as_timespan() {
        for unit in [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
        ] {
            assert_eq!(source(DataType::Duration(unit)), Ok(Type::TimeSpan));
        }
    }

    #[test]
    fn uuid_is_unsupported_until_m2() {
        let field = with_ext(
            Field::new("c", DataType::FixedSizeBinary(16), true),
            UUID_EXTENSION,
        );
        assert_eq!(
            from_arrow_source(&field).unwrap_err().to_string(),
            "FixedSizeBinary(16) (extension type `arrow.uuid`)"
        );
        let err = source(DataType::FixedSizeBinary(16)).unwrap_err();
        assert_eq!(err.to_string(), "FixedSizeBinary(16)");
    }

    #[test]
    fn json_text_is_unsupported_until_m2() {
        for dt in [DataType::Utf8, DataType::LargeUtf8, DataType::Utf8View] {
            let field = with_ext(Field::new("c", dt.clone(), true), JSON_EXTENSION);
            let err = from_arrow_source(&field).unwrap_err();
            assert_eq!(err.data_type, dt);
            assert_eq!(err.extension.as_deref(), Some(JSON_EXTENSION));
        }
    }

    #[test]
    fn nested_is_unsupported_until_m2() {
        let item = Arc::new(Field::new("item", DataType::Int64, true));
        let entries = Arc::new(Field::new(
            "entries",
            DataType::Struct(Fields::from(vec![
                Field::new("key", DataType::Utf8, false),
                Field::new("value", DataType::Int64, true),
            ])),
            false,
        ));
        for dt in [
            DataType::Struct(Fields::from(vec![Field::new("a", DataType::Int64, true)])),
            DataType::List(item.clone()),
            DataType::LargeList(item),
            DataType::Map(entries, false),
        ] {
            assert_eq!(source(dt.clone()).unwrap_err().data_type, dt);
        }
    }

    #[test]
    fn read_anything_else_is_unsupported() {
        for dt in [
            DataType::Binary,
            DataType::LargeBinary,
            DataType::Time32(TimeUnit::Second),
            DataType::Time64(TimeUnit::Microsecond),
            DataType::Interval(IntervalUnit::MonthDayNano),
            DataType::Null,
        ] {
            let err = source(dt.clone()).unwrap_err();
            assert_eq!(err.data_type, dt);
        }
        let field = with_ext(Field::new("c", DataType::Binary, true), "my.ext");
        assert_eq!(
            from_arrow_source(&field).unwrap_err().to_string(),
            "Binary (extension type `my.ext`)"
        );
    }
}
