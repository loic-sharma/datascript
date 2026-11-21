//! The checked `UInt64` → `long` conversion at the scan (LANGUAGE.md §5.1).
//!
//! A plain cast fails with Arrow's generic cast error, which names neither the column nor the
//! value. This UDF knows its column, and fails with a [`ValueOutOfRange`] that
//! `engine_error` turns into `DS4003`.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use arrow_schema::DataType;
use datafusion::arrow::array::{Array, Int64Array, UInt64Array};
use datafusion::error::{DataFusionError, Result};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datafusion::scalar::ScalarValue;

/// A `UInt64` value above `i64::MAX` in a source column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValueOutOfRange {
    pub column: String,
    pub value: u64,
}

impl fmt::Display for ValueOutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "value {} in column `{}` does not fit in a long",
            self.value, self.column
        )
    }
}

impl Error for ValueOutOfRange {}

/// The [`ValueOutOfRange`] that caused `error`, if any. DataFusion and Arrow wrap external
/// errors at several levels, so this walks the whole chain.
pub(crate) fn find_out_of_range(error: &DataFusionError) -> Option<&ValueOutOfRange> {
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(e) = current {
        if let Some(found) = e.downcast_ref::<ValueOutOfRange>() {
            return Some(found);
        }
        if let Some(DataFusionError::External(inner)) = e.downcast_ref::<DataFusionError>()
            && let Some(found) = inner.downcast_ref::<ValueOutOfRange>()
        {
            return Some(found);
        }
        current = e.source();
    }
    None
}

/// `ds_uint64_to_long(column)` for the source column `column`.
pub(crate) fn uint64_to_long(column: &str) -> ScalarUDF {
    ScalarUDF::from(UInt64ToLong {
        column: column.to_string(),
        signature: Signature::exact(vec![DataType::UInt64], Volatility::Immutable),
    })
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct UInt64ToLong {
    column: String,
    signature: Signature,
}

impl UInt64ToLong {
    fn convert(&self, value: u64) -> Result<i64> {
        i64::try_from(value).map_err(|_| {
            DataFusionError::External(Box::new(ValueOutOfRange {
                column: self.column.clone(),
                value,
            }))
        })
    }
}

impl ScalarUDFImpl for UInt64ToLong {
    fn name(&self) -> &str {
        "ds_uint64_to_long"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Int64)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let [arg] = args.args.as_slice() else {
            return Err(DataFusionError::Internal(
                "ds_uint64_to_long takes one argument".into(),
            ));
        };
        match arg {
            ColumnarValue::Scalar(ScalarValue::UInt64(value)) => Ok(ColumnarValue::Scalar(
                ScalarValue::Int64(value.map(|v| self.convert(v)).transpose()?),
            )),
            ColumnarValue::Scalar(other) => Err(DataFusionError::Internal(format!(
                "ds_uint64_to_long got {}",
                other.data_type()
            ))),
            ColumnarValue::Array(array) => {
                let input = array
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .ok_or_else(|| {
                        DataFusionError::Internal("ds_uint64_to_long: not UInt64".into())
                    })?;
                let output = input
                    .iter()
                    .map(|v| v.map(|v| self.convert(v)).transpose())
                    .collect::<Result<Int64Array>>()?;
                Ok(ColumnarValue::Array(Arc::new(output) as Arc<dyn Array>))
            }
        }
    }
}
