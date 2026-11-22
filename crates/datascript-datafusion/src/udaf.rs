//! Aggregates DataFusion has no exact equivalent for: checked `sum` over integers and
//! timespans (D-014: overflow is `DS4003`, DataFusion's `sum` wraps) and exact nearest-rank
//! `percentile` (LANGUAGE.md §7). Like the scalar UDFs, each carries the span of its call.

use std::sync::Arc;

use arrow_schema::{DataType, Field, FieldRef};
use datafusion::arrow::array::{Array, ArrayRef, AsArray};
use datafusion::arrow::compute::cast;
use datafusion::error::{DataFusionError, Result};
use datafusion::logical_expr::function::{AccumulatorArgs, StateFieldsArgs};
use datafusion::logical_expr::utils::format_state_name;
use datafusion::logical_expr::{
    Accumulator, AggregateUDF, AggregateUDFImpl, Signature, Volatility,
};
use datafusion::scalar::ScalarValue;
use datascript_diagnostics::Span;

use crate::errors::{Fail, RuntimeError};

fn located(span: Span, what: &str, ty: &'static str, fail: Fail) -> DataFusionError {
    RuntimeError {
        span,
        what: what.to_string(),
        ty,
        fail,
    }
    .into_datafusion()
}

/// The values of an `Int64`, `Timestamp` or `Duration` array as `i64`.
fn ints(array: &ArrayRef) -> Result<ArrayRef> {
    Ok(match array.data_type() {
        DataType::Int64 => array.clone(),
        _ => cast(array, &DataType::Int64)?,
    })
}

/// `sum` of integers or timespans, failing on overflow.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct CheckedSum {
    ret: DataType,
    ty: &'static str,
    span: Span,
    signature: Signature,
}

pub(crate) fn checked_sum(ret: DataType, ty: &'static str, span: Span) -> AggregateUDF {
    AggregateUDF::from(CheckedSum {
        ret,
        ty,
        span,
        signature: Signature::any(1, Volatility::Immutable),
    })
}

impl AggregateUDFImpl for CheckedSum {
    fn name(&self) -> &str {
        "ds_sum"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(self.ret.clone())
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(SumAccumulator {
            sum: None,
            ret: self.ret.clone(),
            ty: self.ty,
            span: self.span,
        }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(
            format_state_name(args.name, "sum"),
            DataType::Int64,
            true,
        ))])
    }
}

#[derive(Debug)]
struct SumAccumulator {
    sum: Option<i64>,
    ret: DataType,
    ty: &'static str,
    span: Span,
}

impl SumAccumulator {
    fn add_all(&mut self, array: &ArrayRef) -> Result<()> {
        let values = ints(array)?;
        for v in values
            .as_primitive::<datafusion::arrow::datatypes::Int64Type>()
            .iter()
            .flatten()
        {
            let sum = self
                .sum
                .unwrap_or(0)
                .checked_add(v)
                .ok_or_else(|| located(self.span, "`sum`", self.ty, Fail::Overflow))?;
            self.sum = Some(sum);
        }
        Ok(())
    }
}

impl Accumulator for SumAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        self.add_all(&values[0])
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        self.add_all(&states[0])
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Int64(self.sum)])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        ScalarValue::Int64(self.sum).cast_to(&self.ret)
    }

    fn size(&self) -> usize {
        size_of_val(self)
    }
}

/// `percentile(x, p)` by the nearest-rank method: the smallest value with at least `p` percent
/// of the values at or below it. `p` is taken from the first row; 0 gives the minimum.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct Percentile {
    ret: DataType,
    span: Span,
    signature: Signature,
}

pub(crate) fn percentile(ret: DataType, span: Span) -> AggregateUDF {
    AggregateUDF::from(Percentile {
        ret,
        span,
        signature: Signature::any(2, Volatility::Immutable),
    })
}

impl Percentile {
    fn real(&self) -> bool {
        self.ret == DataType::Float64
    }

    fn storage(&self) -> DataType {
        if self.real() {
            DataType::Float64
        } else {
            DataType::Int64
        }
    }
}

impl AggregateUDFImpl for Percentile {
    fn name(&self) -> &str {
        "ds_percentile"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(self.ret.clone())
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(PercentileAccumulator {
            ints: Vec::new(),
            reals: Vec::new(),
            p: None,
            real: self.real(),
            ret: self.ret.clone(),
            span: self.span,
        }))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![
            Arc::new(Field::new_list(
                format_state_name(args.name, "values"),
                Field::new_list_field(self.storage(), true),
                true,
            )),
            Arc::new(Field::new(
                format_state_name(args.name, "p"),
                DataType::Float64,
                true,
            )),
        ])
    }
}

#[derive(Debug)]
struct PercentileAccumulator {
    ints: Vec<i64>,
    reals: Vec<f64>,
    p: Option<f64>,
    real: bool,
    ret: DataType,
    span: Span,
}

impl PercentileAccumulator {
    fn add_values(&mut self, array: &ArrayRef) -> Result<()> {
        if self.real {
            let values = cast(array, &DataType::Float64)?;
            self.reals.extend(
                values
                    .as_primitive::<datafusion::arrow::datatypes::Float64Type>()
                    .iter()
                    .flatten(),
            );
        } else {
            let values = ints(array)?;
            self.ints.extend(
                values
                    .as_primitive::<datafusion::arrow::datatypes::Int64Type>()
                    .iter()
                    .flatten(),
            );
        }
        Ok(())
    }

    fn add_p(&mut self, array: &ArrayRef) -> Result<()> {
        if self.p.is_none() {
            let ps = cast(array, &DataType::Float64)?;
            self.p = ps
                .as_primitive::<datafusion::arrow::datatypes::Float64Type>()
                .iter()
                .flatten()
                .next();
        }
        Ok(())
    }

    fn len(&self) -> usize {
        if self.real {
            self.reals.len()
        } else {
            self.ints.len()
        }
    }
}

impl Accumulator for PercentileAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        self.add_values(&values[0])?;
        self.add_p(&values[1])
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let lists = states[0].as_list::<i32>();
        for i in 0..lists.len() {
            if lists.is_valid(i) {
                self.add_values(&lists.value(i))?;
            }
        }
        self.add_p(&states[1])
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        let values: Vec<ScalarValue> = if self.real {
            self.reals
                .iter()
                .map(|v| ScalarValue::Float64(Some(*v)))
                .collect()
        } else {
            self.ints
                .iter()
                .map(|v| ScalarValue::Int64(Some(*v)))
                .collect()
        };
        let storage = if self.real {
            DataType::Float64
        } else {
            DataType::Int64
        };
        Ok(vec![
            ScalarValue::List(ScalarValue::new_list_nullable(&values, &storage)),
            ScalarValue::Float64(self.p),
        ])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        let n = self.len();
        let Some(p) = self.p.filter(|_| n > 0) else {
            return ScalarValue::try_from(&self.ret);
        };
        if !(0.0..=100.0).contains(&p) {
            return Err(located(
                self.span,
                "`percentile`",
                "",
                Fail::invalid(
                    format!("percentile {p} is outside 0 to 100"),
                    "the second argument of `percentile` is a percentage from 0 to 100",
                ),
            ));
        }
        let rank = ((p / 100.0) * n as f64).ceil() as usize;
        let index = rank.clamp(1, n) - 1;
        if self.real {
            self.reals.sort_by(f64::total_cmp);
            Ok(ScalarValue::Float64(Some(self.reals[index])))
        } else {
            self.ints.sort_unstable();
            ScalarValue::Int64(Some(self.ints[index])).cast_to(&self.ret)
        }
    }

    fn size(&self) -> usize {
        size_of_val(self) + self.ints.capacity() * 8 + self.reals.capacity() * 8
    }
}
