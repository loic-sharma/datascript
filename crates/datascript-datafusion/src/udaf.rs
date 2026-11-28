//! Aggregates DataFusion has no exact equivalent for: checked `sum` over integers and
//! timespans (D-014: overflow is `DS4003`, DataFusion's `sum` wraps) and exact nearest-rank
//! `percentile` (LANGUAGE.md §7). Like the scalar UDFs, each carries the span of its call.

use std::sync::Arc;

use arrow_schema::{DataType, Field, FieldRef};
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array};
use datafusion::arrow::compute::cast;
use datafusion::error::{DataFusionError, Result};
use datafusion::functions_aggregate::approx_percentile_cont::ApproxPercentileAccumulator;
use datafusion::logical_expr::function::{AccumulatorArgs, StateFieldsArgs};
use datafusion::logical_expr::utils::format_state_name;
use datafusion::logical_expr::{
    Accumulator, AggregateUDF, AggregateUDFImpl, Signature, Volatility,
};
use datafusion::scalar::ScalarValue;
use datascript_compiler::arg_values;
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

/// `sum` or `sumif` of integers or timespans, failing on overflow.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct CheckedSum {
    name: String,
    what: String,
    ret: DataType,
    ty: &'static str,
    span: Span,
    signature: Signature,
}

pub(crate) fn checked_sum(
    function: &str,
    ret: DataType,
    ty: &'static str,
    span: Span,
) -> AggregateUDF {
    AggregateUDF::from(CheckedSum {
        name: format!("ds_{function}"),
        what: format!("`{function}`"),
        ret,
        ty,
        span,
        signature: Signature::any(1, Volatility::Immutable),
    })
}

impl AggregateUDFImpl for CheckedSum {
    fn name(&self) -> &str {
        &self.name
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
            what: self.what.clone(),
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
    what: String,
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
                .ok_or_else(|| located(self.span, &self.what, self.ty, Fail::Overflow))?;
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
        size_of_val(self) + self.what.capacity()
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
        if let Err(e) = arg_values::check_percentage(p) {
            return Err(located(
                self.span,
                "`percentile`",
                "",
                Fail::invalid(e.message, e.note),
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

/// Maximum number of non-null `f64` values kept in an exact buffer before spilling to
/// DataFusion's t-digest (16 KiB per accumulator, matching one HyperLogLog sketch).
///
/// DataFusion 55's `TDigest::estimate_quantile` overshoots and clamps on small, uneven value
/// sets (where each value is its own weight-1 centroid). Keeping an exact buffer up to this
/// threshold gives exact continuous linear-interpolated quantiles on small inputs while keeping
/// memory strictly bounded on large files.
const DESCRIBE_EXACT_LIMIT: usize = 2048;

/// Hybrid continuous quantile for `describe`: exact linear interpolation (SQL `PERCENTILE_CONT`)
/// up to [`DESCRIBE_EXACT_LIMIT`] non-null values, spilling to DataFusion's t-digest above it.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct DescribeQuantile {
    name: String,
    /// Quantile in basis points (0..=10_000), e.g. `2500` for `q25`.
    percentile_bps: u16,
    signature: Signature,
}

pub(crate) fn describe_quantile(percentile_bps: u16) -> AggregateUDF {
    AggregateUDF::from(DescribeQuantile {
        name: format!("ds_describe_q{percentile_bps}"),
        percentile_bps,
        signature: Signature::exact(vec![DataType::Float64], Volatility::Immutable),
    })
}

impl DescribeQuantile {
    fn percentile(&self) -> f64 {
        f64::from(self.percentile_bps) / 10_000.0
    }
}

impl AggregateUDFImpl for DescribeQuantile {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Float64)
    }

    fn accumulator(&self, _args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(DescribeQuantileAccumulator::new(
            self.percentile(),
        )))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![
            Arc::new(Field::new_list(
                format_state_name(args.name, "exact"),
                Field::new_list_field(DataType::Float64, true),
                true,
            )),
            Arc::new(Field::new(
                format_state_name(args.name, "max_size"),
                DataType::UInt64,
                false,
            )),
            Arc::new(Field::new(
                format_state_name(args.name, "sum"),
                DataType::Float64,
                false,
            )),
            Arc::new(Field::new(
                format_state_name(args.name, "count"),
                DataType::Float64,
                false,
            )),
            Arc::new(Field::new(
                format_state_name(args.name, "max"),
                DataType::Float64,
                false,
            )),
            Arc::new(Field::new(
                format_state_name(args.name, "min"),
                DataType::Float64,
                false,
            )),
            Arc::new(Field::new_list(
                format_state_name(args.name, "centroids"),
                Field::new_list_field(DataType::Float64, true),
                false,
            )),
        ])
    }
}

#[derive(Debug)]
struct DescribeQuantileAccumulator {
    /// Exact non-null values while count <= `DESCRIBE_EXACT_LIMIT`; `None` once spilled to `digest`.
    exact: Option<Vec<f64>>,
    digest: ApproxPercentileAccumulator,
    percentile: f64,
}

impl DescribeQuantileAccumulator {
    fn new(percentile: f64) -> Self {
        Self {
            exact: Some(Vec::new()),
            digest: ApproxPercentileAccumulator::new(percentile, DataType::Float64),
            percentile,
        }
    }

    /// Moves any buffered exact values into `self.digest` and switches to t-digest mode.
    fn spill(&mut self) -> Result<()> {
        if let Some(buf) = self.exact.take()
            && !buf.is_empty()
        {
            let arr: ArrayRef = Arc::new(Float64Array::from(buf));
            self.digest.update_batch(&[arr])?;
        }
        Ok(())
    }

    fn add_array(&mut self, array: &ArrayRef) -> Result<()> {
        let non_null = array.len() - array.null_count();
        if non_null == 0 {
            return Ok(());
        }
        if let Some(buf) = &mut self.exact
            && buf.len() + non_null <= DESCRIBE_EXACT_LIMIT
        {
            let floats = array.as_primitive::<datafusion::arrow::datatypes::Float64Type>();
            buf.extend(floats.iter().flatten());
            return Ok(());
        }
        self.spill()?;
        self.digest.update_batch(std::slice::from_ref(array))
    }
}

impl Accumulator for DescribeQuantileAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        self.add_array(&values[0])
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let exact_lists = states[0].as_list::<i32>();
        for i in 0..exact_lists.len() {
            if exact_lists.is_valid(i) {
                let values = exact_lists.value(i);
                self.add_array(&values)?;
            } else {
                self.spill()?;
                let digest_row: Vec<ArrayRef> =
                    states[1..].iter().map(|col| col.slice(i, 1)).collect();
                self.digest.merge_batch(&digest_row)?;
            }
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        let exact = match &self.exact {
            Some(buf) => {
                let scalars: Vec<ScalarValue> =
                    buf.iter().map(|&v| ScalarValue::Float64(Some(v))).collect();
                ScalarValue::List(ScalarValue::new_list_nullable(&scalars, &DataType::Float64))
            }
            None => ScalarValue::try_from(&DataType::List(Arc::new(Field::new_list_field(
                DataType::Float64,
                true,
            ))))?,
        };
        let mut out = Vec::with_capacity(7);
        out.push(exact);
        out.extend(self.digest.state()?);
        Ok(out)
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        let Some(buf) = &mut self.exact else {
            return self.digest.evaluate();
        };
        if buf.is_empty() {
            return Ok(ScalarValue::Float64(None));
        }
        buf.sort_by(f64::total_cmp);
        let n = buf.len();
        let pos = self.percentile * (n - 1) as f64;
        let lo = (pos.floor() as usize).min(n - 1);
        let hi = (pos.ceil() as usize).min(n - 1);
        let frac = pos - lo as f64;
        let val = if lo == hi || buf[lo] == buf[hi] {
            buf[lo]
        } else {
            let diff = buf[hi] - buf[lo];
            if diff.is_finite() {
                buf[lo] + frac * diff
            } else {
                (1.0 - frac) * buf[lo] + frac * buf[hi]
            }
        };
        Ok(ScalarValue::Float64(Some(val)))
    }

    fn size(&self) -> usize {
        let exact_bytes = self
            .exact
            .as_ref()
            .map_or(0, |v| v.capacity() * size_of::<f64>());
        size_of_val(self) + exact_bytes + self.digest.size()
    }
}

#[cfg(test)]
mod tests {
    use std::hash::{DefaultHasher, Hash, Hasher};

    use datascript_diagnostics::FileId;

    use super::*;

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn checked_sum_distinguishes_sum_and_sumif() {
        let span = Span::new(FileId(0), 0, 5);
        let sum = checked_sum("sum", DataType::Int64, "long", span);
        let sumif = checked_sum("sumif", DataType::Int64, "long", span);

        assert_eq!(sum.name(), "ds_sum");
        assert_eq!(sumif.name(), "ds_sumif");
        assert_ne!(sum, sumif);
        assert_ne!(hash_of(&sum), hash_of(&sumif));
    }

    fn eval_quantile(percentile: f64, values: &[Option<f64>]) -> Option<f64> {
        let mut acc = DescribeQuantileAccumulator::new(percentile);
        let arr: ArrayRef = Arc::new(Float64Array::from(values.to_vec()));
        acc.update_batch(&[arr]).unwrap();
        match acc.evaluate().unwrap() {
            ScalarValue::Float64(v) => v,
            other => panic!("expected Float64, got {other:?}"),
        }
    }

    fn merge_states(percentile: f64, parts: &[Vec<f64>]) -> Option<f64> {
        let mut states: Vec<Vec<ScalarValue>> = Vec::with_capacity(parts.len());
        for part in parts {
            let mut acc = DescribeQuantileAccumulator::new(percentile);
            let arr: ArrayRef = Arc::new(Float64Array::from(part.clone()));
            acc.update_batch(&[arr]).unwrap();
            states.push(acc.state().unwrap());
        }
        let field_count = states[0].len();
        let state_arrays: Vec<ArrayRef> = (0..field_count)
            .map(|col| {
                let col_vals = states.iter().map(|row| row[col].clone());
                ScalarValue::iter_to_array(col_vals).unwrap()
            })
            .collect();
        let mut merged = DescribeQuantileAccumulator::new(percentile);
        merged.merge_batch(&state_arrays).unwrap();
        match merged.evaluate().unwrap() {
            ScalarValue::Float64(v) => v,
            other => panic!("expected Float64, got {other:?}"),
        }
    }

    #[test]
    fn describe_quantile_exact_on_small_uneven_input() {
        // Matches `span` in `all_types.parquet`: [-5s, 0, 90m, 1d] in microseconds.
        let span = [
            Some(-5_000_000.0),
            Some(0.0),
            None,
            Some(5_400_000_000.0),
            Some(86_400_000_000.0),
        ];
        assert_eq!(eval_quantile(0.25, &span), Some(-1_250_000.0));
        assert_eq!(eval_quantile(0.50, &span), Some(2_700_000_000.0));
        assert_eq!(eval_quantile(0.75, &span), Some(25_650_000_000.0));

        // Matches `id` in `nulls.csv`: [1, 2, 3, 4, 5].
        let ids = [Some(5.0), Some(1.0), None, Some(4.0), Some(2.0), Some(3.0)];
        assert_eq!(eval_quantile(0.25, &ids), Some(2.0));
        assert_eq!(eval_quantile(0.50, &ids), Some(3.0));
        assert_eq!(eval_quantile(0.75, &ids), Some(4.0));
    }

    #[test]
    fn describe_quantile_empty_and_single() {
        assert_eq!(eval_quantile(0.50, &[]), None);
        assert_eq!(eval_quantile(0.50, &[None, None]), None);
        assert_eq!(eval_quantile(0.25, &[Some(42.0)]), Some(42.0));
        assert_eq!(eval_quantile(0.50, &[Some(42.0)]), Some(42.0));
        assert_eq!(eval_quantile(0.75, &[Some(42.0)]), Some(42.0));
    }

    #[test]
    fn describe_quantile_spills_past_limit_and_merges_across_modes() {
        // Stay exact when merged total <= DESCRIBE_EXACT_LIMIT.
        let p1: Vec<f64> = (0..1000).map(|x| x as f64).collect();
        let p2: Vec<f64> = (1000..2000).map(|x| x as f64).collect();
        assert_eq!(merge_states(0.50, &[p1, p2]), Some(999.5));

        // Single-partition spill (> DESCRIBE_EXACT_LIMIT).
        let big: Vec<Option<f64>> = (0..5000).map(|x| Some(x as f64)).collect();
        let q50 = eval_quantile(0.50, &big).unwrap();
        assert!((q50 - 2500.0).abs() < 25.0, "q50={q50}");

        // Exact + exact spilling past DESCRIBE_EXACT_LIMIT during merge.
        let a: Vec<f64> = (0..1500).map(|x| x as f64).collect();
        let b: Vec<f64> = (1500..3000).map(|x| x as f64).collect();
        let q50 = merge_states(0.50, &[a, b]).unwrap();
        assert!((q50 - 1500.0).abs() < 20.0, "q50={q50}");

        // Exact + digest and digest + digest merges.
        let small: Vec<f64> = (0..100).map(|x| x as f64).collect();
        let large1: Vec<f64> = (100..3000).map(|x| x as f64).collect();
        let large2: Vec<f64> = (3000..6000).map(|x| x as f64).collect();
        let q50 = merge_states(0.50, &[small, large1, large2]).unwrap();
        assert!((q50 - 3000.0).abs() < 30.0, "q50={q50}");
    }
}
