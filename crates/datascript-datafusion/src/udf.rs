//! The scalar functions and operators DataFusion has no exact equivalent for, as UDFs.
//!
//! Each [`Udf`] is one kernel plus the span of the expression it computes, so a failure becomes
//! a diagnostic at that expression ([`RuntimeError`]). Kernels take Arrow arrays whose types the
//! lowering chose (`Int64`, `Float64`, `Utf8`, `Boolean`, `Timestamp(µs, UTC)`,
//! `Duration(µs)`), propagate nulls, and return an array of the UDF's return type. This crate
//! builds DataFusion without its string, math, regex and datetime function sets, so every
//! scalar function of the catalog is a kernel here: the semantics are ours (LANGUAGE.md §7,
//! D-014), not whatever a DataFusion builtin happens to do.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_schema::DataType;
use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, Float64Array, Int64Array, StringArray,
};
use datafusion::arrow::compute::cast;
use datafusion::error::{DataFusionError, Result};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datascript_compiler::arg_values;
use datascript_diagnostics::Span;
use regex::Regex;

use crate::calendar::{self, MICROS_PER_DAY, Part, Period};
use crate::errors::{Fail, RuntimeError};

/// A kernel: arguments (all of one length) and the return type, to the result.
pub(crate) type Kernel = fn(&[ArrayRef], &DataType) -> Result<ArrayRef>;

/// A scalar UDF computing one expression. Equality and hashing use the name (unique per
/// kernel), the return type and the span; the kernel follows from the name.
pub(crate) struct Udf {
    name: &'static str,
    kernel: Kernel,
    ret: DataType,
    span: Span,
    what: String,
    ty: &'static str,
    signature: Signature,
}

impl fmt::Debug for Udf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Udf")
            .field("name", &self.name)
            .field("ret", &self.ret)
            .field("span", &self.span)
            .finish()
    }
}

impl PartialEq for Udf {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.ret == other.ret && self.span == other.span
    }
}

impl Eq for Udf {}

impl Hash for Udf {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.ret.hash(state);
        self.span.hash(state);
    }
}

/// Builds a UDF. `what` names the computation in messages (`` `+` ``, `` `percentile` ``) and
/// `ty` is the DataScript name of the result type.
pub(crate) fn udf(
    name: &'static str,
    kernel: Kernel,
    ret: DataType,
    span: Span,
    what: String,
    ty: &'static str,
) -> ScalarUDF {
    ScalarUDF::from(Udf {
        name,
        kernel,
        ret,
        span,
        what,
        ty,
        signature: Signature::variadic_any(Volatility::Immutable),
    })
}

impl ScalarUDFImpl for Udf {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(self.ret.clone())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let arrays = args
            .args
            .iter()
            .map(|a| a.to_array(rows))
            .collect::<Result<Vec<_>>>()?;
        let out = (self.kernel)(&arrays, &self.ret).map_err(|e| self.locate(e))?;
        Ok(ColumnarValue::Array(out))
    }
}

impl Udf {
    /// Gives a [`Fail`] raised by the kernel this UDF's span.
    fn locate(&self, error: DataFusionError) -> DataFusionError {
        match error {
            DataFusionError::External(inner) if inner.is::<Fail>() => {
                match inner.downcast::<Fail>() {
                    Ok(fail) => RuntimeError {
                        span: self.span,
                        what: self.what.clone(),
                        ty: self.ty,
                        fail: *fail,
                    }
                    .into_datafusion(),
                    Err(inner) => DataFusionError::External(inner),
                }
            }
            other => other,
        }
    }
}

/// A [`Fail`] as a DataFusion error; [`Udf`] gives it a span.
pub(crate) fn fail(f: Fail) -> DataFusionError {
    DataFusionError::External(Box::new(f))
}

fn internal(message: impl Into<String>) -> DataFusionError {
    DataFusionError::Internal(message.into())
}

// --- Reading arguments ------------------------------------------------------------------------

/// One argument, read as the representation kernels compute with.
enum Col {
    Str(StringArray),
    /// Integers, datetimes and timespans, as their `i64` value (µs for the temporal types).
    Int(Int64Array),
    Real(Float64Array),
    Bool(BooleanArray),
}

fn col(array: &ArrayRef) -> Result<Col> {
    Ok(match array.data_type() {
        DataType::Utf8 => Col::Str(array.as_string::<i32>().clone()),
        DataType::LargeUtf8 | DataType::Utf8View | DataType::Null => {
            Col::Str(cast(array, &DataType::Utf8)?.as_string::<i32>().clone())
        }
        DataType::Float64 => Col::Real(array.as_primitive().clone()),
        DataType::Float32 | DataType::Float16 => {
            Col::Real(cast(array, &DataType::Float64)?.as_primitive().clone())
        }
        DataType::Boolean => Col::Bool(array.as_boolean().clone()),
        DataType::Int64 => Col::Int(array.as_primitive().clone()),
        DataType::Timestamp(_, _) | DataType::Duration(_) => {
            // A reinterpreting cast: the value in its own unit, which is µs for canonical types.
            Col::Int(cast(array, &DataType::Int64)?.as_primitive().clone())
        }
        t if t.is_integer() => Col::Int(cast(array, &DataType::Int64)?.as_primitive().clone()),
        other => {
            return Err(internal(format!(
                "a kernel got an argument of type {other}"
            )));
        }
    })
}

impl Col {
    fn str(&self, i: usize) -> Option<&str> {
        match self {
            Col::Str(a) if a.is_valid(i) => Some(a.value(i)),
            _ => None,
        }
    }

    fn int(&self, i: usize) -> Option<i64> {
        match self {
            Col::Int(a) if a.is_valid(i) => Some(a.value(i)),
            _ => None,
        }
    }

    fn real(&self, i: usize) -> Option<f64> {
        match self {
            Col::Real(a) if a.is_valid(i) => Some(a.value(i)),
            Col::Int(a) if a.is_valid(i) => Some(a.value(i) as f64),
            _ => None,
        }
    }
}

fn cols(arrays: &[ArrayRef]) -> Result<Vec<Col>> {
    arrays.iter().map(col).collect()
}

fn rows(arrays: &[ArrayRef]) -> usize {
    arrays.first().map_or(0, |a| a.len())
}

/// An optional trailing argument: its value at row `i`, `default` when it was not given, `None`
/// when it was given but is null.
fn opt_int(args: &[Col], index: usize, i: usize, default: i64) -> Option<i64> {
    match args.get(index) {
        None => Some(default),
        Some(c) => c.int(i),
    }
}

// --- Building results -------------------------------------------------------------------------

/// `values` as an array of `ret`, which is `Int64` or a type stored as `i64`.
fn out_int(values: Vec<Option<i64>>, ret: &DataType) -> Result<ArrayRef> {
    let array: ArrayRef = Arc::new(Int64Array::from(values));
    Ok(match ret {
        DataType::Int64 => array,
        _ => cast(&array, ret)?,
    })
}

fn out_real(values: Vec<Option<f64>>) -> ArrayRef {
    Arc::new(Float64Array::from(values))
}

fn out_str(values: Vec<Option<String>>) -> ArrayRef {
    Arc::new(StringArray::from(values))
}

fn out_bool(values: Vec<Option<bool>>) -> ArrayRef {
    Arc::new(BooleanArray::from(values))
}

/// Rounds a real to the nearest `i64`, failing when it does not fit.
fn real_to_int(x: f64) -> Result<i64> {
    to_int(x.round())
}

/// Truncates a real toward zero to an `i64`, failing when it does not fit.
fn real_trunc_to_int(x: f64) -> Result<i64> {
    to_int(x.trunc())
}

/// A real with no fraction as an `i64`, failing when it does not fit.
fn to_int(r: f64) -> Result<i64> {
    // 2^63 is exactly representable; every whole f64 below it in magnitude fits.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if !r.is_finite() || r >= LIMIT || r < -LIMIT {
        return Err(fail(Fail::Overflow));
    }
    Ok(r as i64)
}

// --- Arithmetic (D-014: checked) --------------------------------------------------------------

fn overflow() -> DataFusionError {
    fail(Fail::Overflow)
}

/// Applies `int` when both operands are stored as `i64`, `mixed` when one is a real (a
/// timespan scaled by a real).
fn binary(
    arrays: &[ArrayRef],
    ret: &DataType,
    int: impl Fn(i64, i64) -> Result<i64>,
    mixed: impl Fn(f64, f64) -> Result<i64>,
) -> Result<ArrayRef> {
    let [a, b] = arrays else {
        return Err(internal("arithmetic takes two operands"));
    };
    let (a, b) = (col(a)?, col(b)?);
    let n = rows(arrays);
    let both_int = matches!((&a, &b), (Col::Int(_), Col::Int(_)));
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(if both_int {
            match (a.int(i), b.int(i)) {
                (Some(x), Some(y)) => Some(int(x, y)?),
                _ => None,
            }
        } else {
            match (a.real(i), b.real(i)) {
                (Some(x), Some(y)) => Some(mixed(x, y)?),
                _ => None,
            }
        });
    }
    out_int(out, ret)
}

pub(crate) fn k_add(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    binary(
        arrays,
        ret,
        |x, y| x.checked_add(y).ok_or_else(overflow),
        |_, _| Err(internal("`+` with a real operand")),
    )
}

pub(crate) fn k_sub(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    binary(
        arrays,
        ret,
        |x, y| x.checked_sub(y).ok_or_else(overflow),
        |_, _| Err(internal("`-` with a real operand")),
    )
}

pub(crate) fn k_mul(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    binary(
        arrays,
        ret,
        |x, y| x.checked_mul(y).ok_or_else(overflow),
        |x, y| real_to_int(x * y),
    )
}

/// `long / long` truncates toward zero; a timespan divided by a number keeps whole µs. A
/// timespan divided by a timespan is a real: see [`k_div_real`].
pub(crate) fn k_div(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    binary(
        arrays,
        ret,
        |x, y| {
            if y == 0 {
                Err(fail(Fail::DivisionByZero))
            } else {
                x.checked_div(y).ok_or_else(overflow)
            }
        },
        // LANGUAGE.md §4.1: a timespan divided by a number truncates toward zero, whatever the
        // divisor's type (`5us / 2.0` is `2us`, like `5us / 2`).
        |x, y| {
            if y == 0.0 {
                Err(fail(Fail::DivisionByZero))
            } else {
                real_trunc_to_int(x / y)
            }
        },
    )
}

/// `timespan / timespan`: a real, IEEE 754 on division by zero.
pub(crate) fn k_div_real(arrays: &[ArrayRef], _ret: &DataType) -> Result<ArrayRef> {
    let [a, b] = arrays else {
        return Err(internal("`/` takes two operands"));
    };
    let (a, b) = (col(a)?, col(b)?);
    let out = (0..rows(arrays))
        .map(|i| Some(a.real(i)? / b.real(i)?))
        .collect();
    Ok(out_real(out))
}

pub(crate) fn k_mod(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    binary(
        arrays,
        ret,
        // `i64::MIN % -1` is 0, which fits, so only a zero divisor fails.
        |x, y| {
            if y == 0 {
                Err(fail(Fail::DivisionByZero))
            } else {
                Ok(x.wrapping_rem(y))
            }
        },
        |_, _| Err(internal("`%` with a real operand")),
    )
}

fn unary_int(
    arrays: &[ArrayRef],
    ret: &DataType,
    int: impl Fn(i64) -> Option<i64>,
    real: impl Fn(f64) -> f64,
) -> Result<ArrayRef> {
    let [a] = arrays else {
        return Err(internal("a unary kernel takes one operand"));
    };
    match col(a)? {
        Col::Real(a) => Ok(out_real(a.iter().map(|v| v.map(&real)).collect())),
        c => {
            let out = (0..a.len())
                .map(|i| c.int(i).map(|x| int(x).ok_or_else(overflow)).transpose())
                .collect::<Result<Vec<_>>>()?;
            out_int(out, ret)
        }
    }
}

pub(crate) fn k_neg(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    unary_int(arrays, ret, i64::checked_neg, |x| -x)
}

pub(crate) fn k_abs(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    unary_int(arrays, ret, i64::checked_abs, f64::abs)
}

/// `bin(value, size)`: rounds down to a multiple of `size`. A size that is not positive gives
/// null.
pub(crate) fn k_bin(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let [a, b] = arrays else {
        return Err(internal("bin takes two arguments"));
    };
    let (value, size) = (col(a)?, col(b)?);
    let n = rows(arrays);
    if let DataType::Float64 = ret {
        let out = (0..n)
            .map(|i| {
                let (x, s) = (value.real(i)?, size.real(i)?);
                (s > 0.0).then(|| (x / s).floor() * s)
            })
            .collect();
        return Ok(out_real(out));
    }
    let out = (0..n)
        .map(|i| match (value.int(i), size.int(i)) {
            (Some(x), Some(s)) if s > 0 => x
                .div_euclid(s)
                .checked_mul(s)
                .map(Some)
                .ok_or_else(overflow),
            _ => Ok(None),
        })
        .collect::<Result<Vec<_>>>()?;
    out_int(out, ret)
}

/// `round(x [, digits])`, halves away from zero. Integers round to tens, hundreds, … with
/// negative `digits` and are unchanged otherwise.
pub(crate) fn k_round(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let args = cols(arrays)?;
    let n = rows(arrays);
    if let DataType::Float64 = ret {
        let out = (0..n)
            .map(|i| {
                let x = args[0].real(i)?;
                let digits = opt_int(&args, 1, i, 0)?;
                let digits = i32::try_from(digits.clamp(-400, 400)).unwrap_or(0);
                let factor = 10f64.powi(digits);
                let scaled = x * factor;
                Some(if scaled.is_finite() && factor != 0.0 {
                    let r = scaled.round() / factor;
                    if r.is_finite() { r } else { x }
                } else if factor == 0.0 {
                    0.0 * x.signum()
                } else {
                    x
                })
            })
            .collect();
        return Ok(out_real(out));
    }
    let out = (0..n)
        .map(|i| {
            let (Some(x), Some(digits)) = (args[0].int(i), opt_int(&args, 1, i, 0)) else {
                return Ok(None);
            };
            if digits >= 0 {
                return Ok(Some(x));
            }
            let Some(p) = u32::try_from(-digits)
                .ok()
                .and_then(|e| 10i64.checked_pow(e))
            else {
                return Ok(Some(0));
            };
            let (q, r) = (x / p, x % p);
            let q = if r.unsigned_abs() * 2 >= p.unsigned_abs() {
                q + x.signum()
            } else {
                q
            };
            q.checked_mul(p).map(Some).ok_or_else(overflow)
        })
        .collect::<Result<Vec<_>>>()?;
    out_int(out, ret)
}

/// A one-argument real function.
fn real_fn(arrays: &[ArrayRef], f: impl Fn(f64) -> f64) -> Result<ArrayRef> {
    let [a] = arrays else {
        return Err(internal("a math function takes one argument"));
    };
    let a = col(a)?;
    Ok(out_real(
        (0..rows(arrays)).map(|i| a.real(i).map(&f)).collect(),
    ))
}

pub(crate) fn k_floor(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    real_fn(arrays, f64::floor)
}

pub(crate) fn k_ceiling(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    real_fn(arrays, f64::ceil)
}

pub(crate) fn k_sqrt(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    real_fn(arrays, f64::sqrt)
}

pub(crate) fn k_ln(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    real_fn(arrays, f64::ln)
}

pub(crate) fn k_log10(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    real_fn(arrays, f64::log10)
}

pub(crate) fn k_exp(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    real_fn(arrays, f64::exp)
}

pub(crate) fn k_pow(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    let args = cols(arrays)?;
    let out = (0..rows(arrays))
        .map(|i| Some(args[0].real(i)?.powf(args[1].real(i)?)))
        .collect();
    Ok(out_real(out))
}

// --- Strings ----------------------------------------------------------------------------------

fn str_fn<T>(
    arrays: &[ArrayRef],
    f: impl Fn(&[Col], usize) -> Result<Option<T>>,
) -> Result<Vec<Option<T>>> {
    let args = cols(arrays)?;
    (0..rows(arrays)).map(|i| f(&args, i)).collect()
}

pub(crate) fn k_lower(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    Ok(out_str(str_fn(arrays, |a, i| {
        Ok(a[0].str(i).map(str::to_lowercase))
    })?))
}

pub(crate) fn k_upper(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    Ok(out_str(str_fn(arrays, |a, i| {
        Ok(a[0].str(i).map(str::to_uppercase))
    })?))
}

pub(crate) fn k_strlen(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let out = str_fn(arrays, |a, i| {
        Ok(a[0].str(i).map(|s| s.chars().count() as i64))
    })?;
    out_int(out, ret)
}

/// Whether `needle` occurs in `hay` as whole terms: where the needle starts (ends) with a
/// letter or digit, the text before (after) the match must not be one (LANGUAGE.md §4).
pub(crate) fn has_term(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let check_before = needle.chars().next().is_some_and(char::is_alphanumeric);
    let check_after = needle
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric);
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before = !check_before
            || hay[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric());
        let after = !check_after
            || hay[end..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// A string predicate; `fold` lowercases both sides first.
fn string_test(arrays: &[ArrayRef], fold: bool, test: fn(&str, &str) -> bool) -> Result<ArrayRef> {
    Ok(out_bool(str_fn(arrays, |a, i| {
        let (Some(x), Some(y)) = (a[0].str(i), a[1].str(i)) else {
            return Ok(None);
        };
        Ok(Some(if fold {
            test(&x.to_lowercase(), &y.to_lowercase())
        } else {
            test(x, y)
        }))
    })?))
}

pub(crate) fn k_has(a: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    string_test(a, true, has_term)
}
pub(crate) fn k_has_cs(a: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    string_test(a, false, has_term)
}
pub(crate) fn k_contains(a: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    string_test(a, true, |x, y| x.contains(y))
}
pub(crate) fn k_contains_cs(a: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    string_test(a, false, |x, y| x.contains(y))
}
pub(crate) fn k_startswith(a: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    string_test(a, true, |x, y| x.starts_with(y))
}
pub(crate) fn k_startswith_cs(a: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    string_test(a, false, |x, y| x.starts_with(y))
}
pub(crate) fn k_endswith(a: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    string_test(a, true, |x, y| x.ends_with(y))
}
pub(crate) fn k_endswith_cs(a: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    string_test(a, false, |x, y| x.ends_with(y))
}

/// Compiled regexes for one kernel call; most patterns are constants.
#[derive(Default)]
struct Regexes(HashMap<String, Regex>);

impl Regexes {
    fn get(&mut self, pattern: &str) -> Result<&Regex> {
        self.wrapped(pattern, pattern)
    }

    /// Compiles `source` (built from the user's `pattern`), reporting errors against `pattern`.
    fn wrapped(&mut self, pattern: &str, source: &str) -> Result<&Regex> {
        if !self.0.contains_key(source) {
            let regex = Regex::new(source).map_err(|e| {
                let e = arg_values::invalid_regex(pattern, &e.to_string());
                fail(Fail::invalid(e.message, e.note))
            })?;
            self.0.insert(source.to_string(), regex);
        }
        Ok(&self.0[source])
    }
}

pub(crate) fn k_matches_regex(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    let args = cols(arrays)?;
    let mut regexes = Regexes::default();
    let mut out = Vec::with_capacity(rows(arrays));
    for i in 0..rows(arrays) {
        out.push(match (args[0].str(i), args[1].str(i)) {
            (Some(s), Some(p)) => Some(regexes.get(p)?.is_match(s)),
            _ => None,
        });
    }
    Ok(out_bool(out))
}

/// `trim(regex, source)`: removes leading and trailing runs of matches.
pub(crate) fn k_trim(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    let args = cols(arrays)?;
    let mut regexes = Regexes::default();
    let mut out = Vec::with_capacity(rows(arrays));
    for i in 0..rows(arrays) {
        out.push(match (args[0].str(i), args[1].str(i)) {
            (Some(p), Some(s)) => {
                let start = regexes.wrapped(p, &format!("^(?:{p})+"))?;
                let s = match start.find(s) {
                    Some(m) => &s[m.end()..],
                    None => s,
                };
                let end = regexes.wrapped(p, &format!("(?:{p})+$"))?;
                let s = match end.find(s) {
                    Some(m) => &s[..m.start()],
                    None => s,
                };
                Some(s.to_string())
            }
            _ => None,
        });
    }
    Ok(out_str(out))
}

/// `substring(source, start [, length])`, in characters, 0-based. A negative `start` counts
/// from the end; a negative `length` gives an empty string.
pub(crate) fn k_substring(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    Ok(out_str(str_fn(arrays, |a, i| {
        let (Some(s), Some(start)) = (a[0].str(i), a[1].int(i)) else {
            return Ok(None);
        };
        let Some(length) = opt_int(a, 2, i, i64::MAX) else {
            return Ok(None);
        };
        let n = s.chars().count() as i64;
        let start = if start < 0 {
            (n + start).max(0)
        } else {
            start.min(n)
        };
        let take = length.max(0).min(n - start);
        Ok(Some(
            s.chars().skip(start as usize).take(take as usize).collect(),
        ))
    })?))
}

/// `strcat(...)`: the string forms of the arguments; a null argument adds nothing (as in KQL).
pub(crate) fn k_strcat(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    let texts = arrays.iter().map(to_text).collect::<Result<Vec<_>>>()?;
    let out = (0..rows(arrays))
        .map(|i| {
            let mut s = String::new();
            for t in &texts {
                if let Some(part) = &t[i] {
                    s.push_str(part);
                }
            }
            Some(s)
        })
        .collect();
    Ok(out_str(out))
}

pub(crate) fn k_replace_string(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    Ok(out_str(str_fn(arrays, |a, i| {
        let (Some(s), Some(find), Some(repl)) = (a[0].str(i), a[1].str(i), a[2].str(i)) else {
            return Ok(None);
        };
        Ok(Some(if find.is_empty() {
            s.to_string()
        } else {
            s.replace(find, repl)
        }))
    })?))
}

/// `indexof(source, lookup [, start [, length [, occurrence]]])`, in characters, 0-based; -1 if
/// absent. A negative `start` counts from the end; `length` -1 means to the end. A `length`
/// below -1 or an `occurrence` below 1 gives null.
pub(crate) fn k_indexof(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let out = str_fn(arrays, |a, i| {
        let (Some(s), Some(lookup)) = (a[0].str(i), a[1].str(i)) else {
            return Ok(None);
        };
        let (Some(start), Some(length), Some(occurrence)) = (
            opt_int(a, 2, i, 0),
            opt_int(a, 3, i, -1),
            opt_int(a, 4, i, 1),
        ) else {
            return Ok(None);
        };
        if length < -1 || occurrence < 1 {
            return Ok(None);
        }
        let chars: Vec<char> = s.chars().collect();
        let needle: Vec<char> = lookup.chars().collect();
        let n = chars.len() as i64;
        let start = if start < 0 { (n + start).max(0) } else { start };
        if start > n {
            return Ok(Some(-1));
        }
        let end = if length == -1 {
            n
        } else {
            (start + length).min(n)
        } as usize;
        let mut seen = 0;
        let mut p = start as usize;
        while p + needle.len() <= end {
            if chars[p..p + needle.len()] == needle[..] {
                seen += 1;
                if seen == occurrence {
                    return Ok(Some(p as i64));
                }
            }
            p += 1;
        }
        Ok(Some(-1))
    })?;
    out_int(out, ret)
}

/// `extract(regex, group, source)`: the text of capture group `group`, or null.
pub(crate) fn k_extract(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    let args = cols(arrays)?;
    let mut regexes = Regexes::default();
    let mut out = Vec::with_capacity(rows(arrays));
    for i in 0..rows(arrays) {
        out.push(match (args[0].str(i), args[1].int(i), args[2].str(i)) {
            (Some(p), Some(group), Some(s)) => {
                let regex = regexes.get(p)?;
                usize::try_from(group).ok().and_then(|g| {
                    regex
                        .captures(s)
                        .and_then(|c| c.get(g))
                        .map(|m| m.as_str().to_string())
                })
            }
            _ => None,
        });
    }
    Ok(out_str(out))
}

// --- Conversions (a value that does not convert is null) --------------------------------------

/// The string form of each value, as `tostring` gives it; null stays null.
fn to_text(array: &ArrayRef) -> Result<Vec<Option<String>>> {
    let dt = array.data_type().clone();
    let c = col(array)?;
    Ok((0..array.len())
        .map(|i| match (&dt, &c) {
            (_, Col::Str(_)) => c.str(i).map(str::to_string),
            (_, Col::Bool(a)) => a.is_valid(i).then(|| a.value(i).to_string()),
            (_, Col::Real(_)) => c.real(i).map(|x| x.to_string()),
            (DataType::Timestamp(_, _), _) => {
                c.int(i).map(datascript_engine::table::format_datetime)
            }
            (DataType::Duration(_), _) => c.int(i).map(datascript_engine::table::format_timespan),
            _ => c.int(i).map(|x| x.to_string()),
        })
        .collect())
}

pub(crate) fn k_tostring(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    Ok(out_str(to_text(&arrays[0])?))
}

/// An integer from any value: strings in integer syntax, reals truncated toward zero, bools as
/// 1 and 0. Datetimes and timespans do not convert.
fn to_integer(array: &ArrayRef, i: usize, c: &Col) -> Option<i64> {
    match (array.data_type(), c) {
        (_, Col::Str(_)) => c.str(i)?.trim().parse().ok(),
        (_, Col::Bool(a)) => a.is_valid(i).then(|| i64::from(a.value(i))),
        (_, Col::Real(_)) => {
            let x = c.real(i)?.trunc();
            const LIMIT: f64 = 9_223_372_036_854_775_808.0;
            (x.is_finite() && (-LIMIT..LIMIT).contains(&x)).then_some(x as i64)
        }
        (DataType::Timestamp(_, _) | DataType::Duration(_), _) => None,
        _ => c.int(i),
    }
}

pub(crate) fn k_tolong(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let c = col(&arrays[0])?;
    let out = (0..rows(arrays))
        .map(|i| to_integer(&arrays[0], i, &c))
        .collect();
    out_int(out, ret)
}

pub(crate) fn k_toint(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let c = col(&arrays[0])?;
    let out = (0..rows(arrays))
        .map(|i| to_integer(&arrays[0], i, &c).filter(|v| i32::try_from(*v).is_ok()))
        .collect();
    out_int(out, ret)
}

pub(crate) fn k_toreal(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    let a = &arrays[0];
    let c = col(a)?;
    let out = (0..a.len())
        .map(|i| match (a.data_type(), &c) {
            (_, Col::Str(_)) => c.str(i)?.trim().parse().ok(),
            (_, Col::Bool(b)) => b.is_valid(i).then(|| f64::from(u8::from(b.value(i)))),
            (DataType::Timestamp(_, _) | DataType::Duration(_), _) => None,
            _ => c.real(i),
        })
        .collect();
    Ok(out_real(out))
}

pub(crate) fn k_tobool(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    let a = &arrays[0];
    let c = col(a)?;
    let out = (0..a.len())
        .map(|i| match (a.data_type(), &c) {
            (_, Col::Str(_)) => {
                let s = c.str(i)?.trim();
                if s.eq_ignore_ascii_case("true") {
                    Some(true)
                } else if s.eq_ignore_ascii_case("false") {
                    Some(false)
                } else {
                    None
                }
            }
            (_, Col::Bool(b)) => b.is_valid(i).then(|| b.value(i)),
            (_, Col::Real(_)) => c.real(i).filter(|x| !x.is_nan()).map(|x| x != 0.0),
            (DataType::Timestamp(_, _) | DataType::Duration(_), _) => None,
            _ => c.int(i).map(|x| x != 0),
        })
        .collect();
    Ok(out_bool(out))
}

/// A datetime from an ISO 8601 string (the `datetime(...)` literal syntax) or a datetime.
pub(crate) fn k_todatetime(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let a = &arrays[0];
    let c = col(a)?;
    let out = (0..a.len())
        .map(|i| match (a.data_type(), &c) {
            (_, Col::Str(_)) => parse_literal(c.str(i)?, "datetime"),
            (DataType::Timestamp(_, _), _) => c.int(i),
            _ => None,
        })
        .collect();
    out_int(out, ret)
}

/// A timespan from a string (`"1.02:03:04"`, `"90m"`) or a timespan.
pub(crate) fn k_totimespan(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let a = &arrays[0];
    let c = col(a)?;
    let out = (0..a.len())
        .map(|i| match (a.data_type(), &c) {
            (_, Col::Str(_)) => parse_literal(c.str(i)?, "timespan"),
            (DataType::Duration(_), _) => c.int(i),
            _ => None,
        })
        .collect();
    out_int(out, ret)
}

/// The value of `kind(text)` as a literal, or `None`.
fn parse_literal(text: &str, kind: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() || text.contains([')', '\n', '\r']) || text == "null" {
        return None;
    }
    let literal = format!("{kind}({text})");
    let span = Span::new(datascript_diagnostics::FileId(0), 0, literal.len() as u32);
    let value = match kind {
        "datetime" => datascript_syntax::literal::datetime(&literal, span),
        _ => datascript_syntax::literal::timespan(&literal, span),
    };
    value.ok().flatten()
}

// --- Datetimes --------------------------------------------------------------------------------

fn period(arrays: &[ArrayRef], ret: &DataType, p: Period, end: bool) -> Result<ArrayRef> {
    let args = cols(arrays)?;
    let out = (0..rows(arrays))
        .map(|i| {
            let (Some(us), Some(offset)) = (args[0].int(i), opt_int(&args, 1, i, 0)) else {
                return Ok(None);
            };
            let r = if end {
                calendar::end_of(p, us, offset)
            } else {
                calendar::start_of(p, us, offset)
            };
            r.map(Some).map_err(fail)
        })
        .collect::<Result<Vec<_>>>()?;
    out_int(out, ret)
}

pub(crate) fn k_startofday(a: &[ArrayRef], r: &DataType) -> Result<ArrayRef> {
    period(a, r, Period::Day, false)
}
pub(crate) fn k_startofweek(a: &[ArrayRef], r: &DataType) -> Result<ArrayRef> {
    period(a, r, Period::Week, false)
}
pub(crate) fn k_startofmonth(a: &[ArrayRef], r: &DataType) -> Result<ArrayRef> {
    period(a, r, Period::Month, false)
}
pub(crate) fn k_startofyear(a: &[ArrayRef], r: &DataType) -> Result<ArrayRef> {
    period(a, r, Period::Year, false)
}
pub(crate) fn k_endofday(a: &[ArrayRef], r: &DataType) -> Result<ArrayRef> {
    period(a, r, Period::Day, true)
}
pub(crate) fn k_endofweek(a: &[ArrayRef], r: &DataType) -> Result<ArrayRef> {
    period(a, r, Period::Week, true)
}
pub(crate) fn k_endofmonth(a: &[ArrayRef], r: &DataType) -> Result<ArrayRef> {
    period(a, r, Period::Month, true)
}
pub(crate) fn k_endofyear(a: &[ArrayRef], r: &DataType) -> Result<ArrayRef> {
    period(a, r, Period::Year, true)
}

fn part(name: &str) -> Result<Part> {
    Part::parse(name).map_err(|e| fail(Fail::invalid(e.message, e.note)))
}

pub(crate) fn k_datetime_part(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let out = str_fn(arrays, |a, i| {
        let (Some(name), Some(us)) = (a[0].str(i), a[1].int(i)) else {
            return Ok(None);
        };
        Ok(Some(calendar::part_of(part(name)?, us)))
    })?;
    out_int(out, ret)
}

pub(crate) fn k_datetime_diff(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let out = str_fn(arrays, |a, i| {
        let (Some(name), Some(x), Some(y)) = (a[0].str(i), a[1].int(i), a[2].int(i)) else {
            return Ok(None);
        };
        calendar::diff(part(name)?, x, y).map(Some).map_err(fail)
    })?;
    out_int(out, ret)
}

/// `dayofweek(dt)`: the timespan since the start of its week (Sunday).
pub(crate) fn k_dayofweek(arrays: &[ArrayRef], ret: &DataType) -> Result<ArrayRef> {
    let out = str_fn(arrays, |a, i| {
        Ok(a[0]
            .int(i)
            .map(|us| calendar::weekday(us.div_euclid(MICROS_PER_DAY)) * MICROS_PER_DAY))
    })?;
    out_int(out, ret)
}

pub(crate) fn k_format_datetime(arrays: &[ArrayRef], _: &DataType) -> Result<ArrayRef> {
    Ok(out_str(str_fn(arrays, |a, i| {
        let (Some(us), Some(format)) = (a[0].int(i), a[1].str(i)) else {
            return Ok(None);
        };
        calendar::format(us, format).map(Some).map_err(fail)
    })?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_matches_whole_terms() {
        assert!(has_term("the quick fox", "quick"));
        assert!(!has_term("the quickest fox", "quick"));
        assert!(has_term("error-42 happened", "42"));
        assert!(has_term("user=bob id=7", "bob id"));
        assert!(!has_term("abc", "b"));
        assert!(has_term("a.b", ".b"));
        assert!(has_term("über alles", "über"));
        assert!(!has_term("x1", "x"));
    }

    fn longs(values: &[Option<i64>]) -> ArrayRef {
        Arc::new(Int64Array::from(values.to_vec()))
    }

    #[test]
    fn integer_arithmetic_is_checked() {
        let a = longs(&[Some(i64::MAX), Some(-7), None]);
        let b = longs(&[Some(1), Some(2), Some(1)]);
        assert!(k_add(&[a.clone(), b.clone()], &DataType::Int64).is_err());
        let d = k_div(&[a.slice(1, 2), b.slice(1, 2)], &DataType::Int64).unwrap();
        assert_eq!(
            d.as_primitive::<datafusion::arrow::datatypes::Int64Type>()
                .value(0),
            -3
        );
        let m = k_mod(
            &[longs(&[Some(i64::MIN)]), longs(&[Some(-1)])],
            &DataType::Int64,
        )
        .unwrap();
        assert_eq!(
            m.as_primitive::<datafusion::arrow::datatypes::Int64Type>()
                .value(0),
            0
        );
        assert!(k_div(&[longs(&[Some(1)]), longs(&[Some(0)])], &DataType::Int64).is_err());
        assert!(k_neg(&[longs(&[Some(i64::MIN)])], &DataType::Int64).is_err());
    }
}
