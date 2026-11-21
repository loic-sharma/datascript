//! Values of literal tokens.
//!
//! Each function takes the text of one token of the matching [`SyntaxKind`](crate::SyntaxKind)
//! and the token's span, and returns the value or the diagnostics explaining why the text is not
//! a valid literal. The spans in those diagnostics point inside the token (at the bad unit, the
//! bad escape, the month that is out of range).
//!
//! The lexer calls these functions to report bad literals, so a token whose value fails to parse
//! has **already been reported**. Later stages that need the value call them again and, on error,
//! give the expression the error type silently (DIAGNOSTICS.md rule 8).

use std::ops::Range;

use datascript_diagnostics::{Applicability, Diagnostic, Span, Suggestion, TextEdit, codes};

/// The result of reading a literal: its value, or the diagnostics for it (at least one).
pub type LiteralResult<T> = Result<T, Vec<Diagnostic>>;

/// The largest number a [`LONG`](crate::SyntaxKind::LONG) token may hold: 2^63. It does not fit
/// in a `long` on its own, but `-9223372036854775808` (unary minus applied to it) does, so the
/// binder checks the upper bound once it knows the sign (LANGUAGE.md §1).
pub const MAX_LONG_LITERAL: u64 = 1 << 63;

/// The timespan units, in the order they are listed in messages.
const TIMESPAN_UNITS: &[(&str, i128)] = &[
    ("d", 86_400_000_000_000),
    ("h", 3_600_000_000_000),
    ("m", 60_000_000_000),
    ("s", 1_000_000_000),
    ("ms", 1_000_000),
    ("us", 1_000),
    ("µs", 1_000),
    // Greek small mu (U+03BC) looks the same as the micro sign (U+00B5) and is what many
    // keyboards type.
    ("μs", 1_000),
    ("tick", 100),
];

const UNITS_HELP: &str =
    "valid units are `d`, `h`, `m` (minutes), `s`, `ms`, `us` (or `µs`) and `tick` (100 ns)";

fn sub(span: Span, range: Range<usize>) -> Span {
    Span::new(
        span.file,
        span.start + range.start as u32,
        span.start + range.end as u32,
    )
}

fn invalid(message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(codes::DS0003, message)
}

/// The value of a [`LONG`](crate::SyntaxKind::LONG) token such as `42` or `1_000_000`.
///
/// Values up to [`MAX_LONG_LITERAL`] are accepted.
pub fn long(text: &str, span: Span) -> LiteralResult<u64> {
    if let Some(error) = check_underscores(text, 0, span) {
        return Err(vec![error]);
    }
    let digits: String = text.chars().filter(|c| *c != '_').collect();
    match digits.parse::<u64>() {
        Ok(value) if value <= MAX_LONG_LITERAL => Ok(value),
        _ => Err(vec![
            invalid("number is too large for a `long`")
                .primary(span, "the largest `long` is 9223372036854775807")
                .help(format!(
                    "for numbers this large, use a `real` by adding a fraction: `{digits}.0`"
                )),
        ]),
    }
}

/// The value of a [`REAL`](crate::SyntaxKind::REAL) token such as `3.14`, `1e-3` or `.5`.
pub fn real(text: &str, span: Span) -> LiteralResult<f64> {
    if let Some(error) = check_underscores(text, 0, span) {
        return Err(vec![error]);
    }
    let cleaned: String = text.chars().filter(|c| *c != '_').collect();
    match cleaned.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(value),
        Ok(_) => Err(vec![
            invalid("number is too large for a `real`")
                .primary(span, "the largest `real` is about 1.8e308"),
        ]),
        Err(_) => Err(vec![
            invalid(format!("invalid number `{text}`")).primary(span, "not a valid number"),
        ]),
    }
}

/// Underscores in numbers separate digits: each must sit between two digits.
fn check_underscores(text: &str, offset: usize, span: Span) -> Option<Diagnostic> {
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'_' {
            continue;
        }
        let before = i > 0 && bytes[i - 1].is_ascii_digit();
        let after = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
        if !before || !after {
            let at = sub(span, offset + i..offset + i + 1);
            return Some(
                invalid(format!("invalid number `{text}`"))
                    .primary(at, "`_` must be between two digits")
                    .help(format!("remove it: `{}`", text.replace('_', ""))),
            );
        }
    }
    None
}

/// The value of a [`TIMESPAN`](crate::SyntaxKind::TIMESPAN) token, in microseconds. `None` for
/// `timespan(null)`.
///
/// Accepts the short form (`1d`, `1.5h`, `250ms`) and `timespan(...)` with a body that is either a
/// short form or `[-][d.]hh:mm[:ss[.fffffff]]`. Fractions below a microsecond are truncated.
pub fn timespan(text: &str, span: Span) -> LiteralResult<Option<i64>> {
    if let Some(rest) = text.strip_prefix("timespan(") {
        let body_start = "timespan(".len();
        let (body, body_range) = call_body("timespan", rest, body_start, text, span)?;
        if body == "null" {
            return Ok(None);
        }
        let unsigned = body.strip_prefix('-');
        let result = if body.contains(':') {
            clock_timespan(body, body_range.start, span)
        } else if let Some(short) = unsigned.filter(|b| b.starts_with(is_number_start)) {
            short_timespan(short, body_range.start + 1, span).map(|v| -v)
        } else if body.starts_with(is_number_start) {
            short_timespan(body, body_range.start, span)
        } else {
            Err(vec![bad_timespan_body(body, body_range, span)])
        };
        return result.map(Some);
    }
    short_timespan(text, 0, span).map(Some)
}

fn is_number_start(c: char) -> bool {
    c.is_ascii_digit() || c == '.'
}

fn bad_timespan_body(body: &str, range: Range<usize>, span: Span) -> Diagnostic {
    let at = sub(span, range.clone());
    let d = invalid(format!("invalid timespan `{body}`"));
    let d = if body.is_empty() {
        d.primary(at, "expected a timespan here")
    } else {
        d.primary(at, "not a timespan")
    };
    d.help("write a timespan like `timespan(1.02:03:04)` (days.hours:minutes:seconds) or `90m`")
        .help("to convert a value to a timespan, use `totimespan(...)`")
}

/// Splits `datetime(` / `timespan(` bodies: everything up to the closing `)`, trimmed.
fn call_body<'a>(
    name: &str,
    rest: &'a str,
    body_start: usize,
    text: &str,
    span: Span,
) -> LiteralResult<(&'a str, Range<usize>)> {
    let Some(inner) = rest.strip_suffix(')') else {
        return Err(vec![
            invalid(format!("`{name}(` is never closed"))
                .primary(
                    sub(span, 0..name.len() + 1),
                    "this literal has no closing `)` on this line",
                )
                .help(format!("close it with `)`: `{text})`")),
        ]);
    };
    let leading = inner.len() - inner.trim_start().len();
    let body = inner.trim();
    let start = body_start + leading;
    Ok((body, start..start + body.len()))
}

fn short_timespan(text: &str, offset: usize, span: Span) -> LiteralResult<i64> {
    let number_end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '_' || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(number_end);
    let unit_range = offset + number_end..offset + text.len();
    if number.is_empty() || number == "." {
        return Err(vec![
            invalid(format!("invalid timespan `{text}`"))
                .primary(sub(span, offset..offset + text.len()), "not a timespan")
                .help(
                    "write a timespan like `timespan(1.02:03:04)` (days.hours:minutes:seconds) or `90m`",
                ),
        ]);
    }
    if let Some(error) = check_underscores(number, offset, span) {
        return Err(vec![error]);
    }
    if unit.is_empty() {
        return Err(vec![
            invalid(format!("timespan `{text}` has no unit"))
                .primary(
                    sub(span, offset..offset + text.len()),
                    "add a unit after the number",
                )
                .help(UNITS_HELP),
        ]);
    }
    let exponent = unit.starts_with(['e', 'E'])
        && unit[1..].starts_with(|c: char| c.is_ascii_digit() || c == '+' || c == '-');
    if exponent {
        return Err(vec![
            invalid("a timespan cannot use an exponent")
                .primary(
                    sub(span, offset..offset + text.len()),
                    "exponent used in a timespan",
                )
                .help("write the number out in full, or use a larger unit"),
        ]);
    }
    let Some(&(_, unit_ns)) = TIMESPAN_UNITS.iter().find(|(name, _)| *name == unit) else {
        if let Some(d) = hex_number(number, unit, sub(span, offset..offset + text.len())) {
            return Err(vec![d]);
        }
        let mut d = invalid(format!("unknown timespan unit `{unit}`"))
            .primary(sub(span, unit_range), "not a timespan unit")
            .help(UNITS_HELP);
        if let Some(hint) = unit_hint(unit) {
            d = d.help(hint);
        }
        return Err(vec![d]);
    };
    let (int_part, frac_part) = number.split_once('.').unwrap_or((number, ""));
    // Digits past nanosecond precision cannot change the value (it is truncated to µs), and
    // keeping them would overflow `10^scale`.
    let frac_digits: String = frac_part.chars().filter(|c| *c != '_').take(9).collect();
    let digits: String = int_part
        .chars()
        .filter(|c| *c != '_')
        .chain(frac_digits.chars())
        .collect();
    let scale = frac_digits.len() as u32;
    let too_large = || {
        vec![invalid("timespan is too large").primary(
            sub(span, offset..offset + text.len()),
            "timespans range over about ±292,000 years",
        )]
    };
    let mantissa: i128 = digits.parse().map_err(|_| too_large())?;
    let ns = mantissa
        .checked_mul(unit_ns)
        .and_then(|v| v.checked_div(10i128.checked_pow(scale)?))
        .ok_or_else(too_large)?;
    i64::try_from(ns / 1_000).map_err(|_| too_large())
}

/// `0x1F` lexes as the number `0` with the unit `x1F`: say that hexadecimal is not supported.
fn hex_number(number: &str, unit: &str, at: Span) -> Option<Diagnostic> {
    let hex = unit.strip_prefix(['x', 'X'])?;
    if number != "0" || hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let d = invalid("hexadecimal numbers are not supported").primary(at, "hexadecimal number");
    Some(match u64::from_str_radix(hex, 16) {
        Ok(value) if value <= i64::MAX as u64 => {
            d.help(format!("write the number in decimal: `{value}`"))
        }
        _ => d.help("write the number in decimal"),
    })
}

/// Common spellings of units people try first.
fn unit_hint(unit: &str) -> Option<&'static str> {
    Some(match unit {
        "min" | "mins" | "minute" | "minutes" => "for minutes, write `m`: `30m`",
        "sec" | "secs" | "second" | "seconds" => "for seconds, write `s`: `10s`",
        "hr" | "hrs" | "hour" | "hours" => "for hours, write `h`: `2h`",
        "day" | "days" => "for days, write `d`: `7d`",
        "ticks" => "write `tick`: `10tick`",
        "w" | "wk" | "week" | "weeks" => "there is no week unit; use days: `7d`",
        "ns" => "timespans have microsecond precision; the smallest unit is `tick` (100 ns)",
        _ => return None,
    })
}

/// `[-][d.]hh:mm[:ss[.fffffff]]`
fn clock_timespan(body: &str, offset: usize, span: Span) -> LiteralResult<i64> {
    let bad = |range: Range<usize>, label: &str| {
        vec![
            invalid(format!("invalid timespan `{body}`"))
                .primary(sub(span, range), label.to_string())
                .help("write `[days.]hours:minutes[:seconds]`, e.g. `timespan(1.02:03:04)`"),
        ]
    };
    let (negative, rest, mut pos) = match body.strip_prefix('-') {
        Some(rest) => (true, rest, offset + 1),
        None => (false, body, offset),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() > 3 {
        return Err(bad(offset..offset + body.len(), "too many `:` parts"));
    }
    let mut total_us: i128 = 0;

    // First part: `hh` or `d.hh`.
    let first = parts[0];
    let (days, hours) = match first.split_once('.') {
        Some((d, h)) => (Some(d), h),
        None => (None, first),
    };
    if let Some(d) = days {
        let value = digits_value(d).ok_or_else(|| bad(pos..pos + d.len(), "expected days"))?;
        total_us += value * 86_400_000_000;
        pos += d.len() + 1;
    }
    let h = digits_value(hours).ok_or_else(|| bad(pos..pos + hours.len(), "expected hours"))?;
    if h > 23 {
        return Err(bad(pos..pos + hours.len(), "hours must be 0 to 23"));
    }
    total_us += h * 3_600_000_000;
    pos += hours.len() + 1;

    let minutes = parts.get(1).copied().unwrap_or("");
    let m =
        digits_value(minutes).ok_or_else(|| bad(pos..pos + minutes.len(), "expected minutes"))?;
    if m > 59 {
        return Err(bad(pos..pos + minutes.len(), "minutes must be 0 to 59"));
    }
    total_us += m * 60_000_000;
    pos += minutes.len() + 1;

    if let Some(seconds) = parts.get(2) {
        let (whole, frac) = match seconds.split_once('.') {
            Some((w, f)) => (w, Some(f)),
            None => (*seconds, None),
        };
        let s =
            digits_value(whole).ok_or_else(|| bad(pos..pos + whole.len(), "expected seconds"))?;
        if s > 59 {
            return Err(bad(pos..pos + whole.len(), "seconds must be 0 to 59"));
        }
        total_us += s * 1_000_000;
        if let Some(frac) = frac {
            let start = pos + whole.len() + 1;
            if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad(start..start + frac.len(), "expected digits after `.`"));
            }
            total_us += fraction_micros(frac);
        }
    }
    let total = if negative { -total_us } else { total_us };
    i64::try_from(total).map_err(|_| bad(offset..offset + body.len(), "timespan is too large"))
}

/// The value of 1 to 18 ASCII digits.
fn digits_value(text: &str) -> Option<i128> {
    if text.is_empty() || text.len() > 18 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Microseconds in a decimal fraction of a second, truncated.
fn fraction_micros(frac: &str) -> i128 {
    let mut padded: String = frac.chars().take(6).collect();
    while padded.len() < 6 {
        padded.push('0');
    }
    padded.parse().unwrap_or(0)
}

/// The value of a [`DATETIME`](crate::SyntaxKind::DATETIME) token, in microseconds since the Unix
/// epoch (UTC). `None` for `datetime(null)`.
///
/// The body is ISO 8601: `YYYY-MM-DD`, optionally followed by `T` (or a space), `hh:mm`,
/// optional `:ss` and fraction, and an optional `Z` or `±hh:mm` offset. No offset means UTC.
pub fn datetime(text: &str, span: Span) -> LiteralResult<Option<i64>> {
    let rest = text.strip_prefix("datetime(").unwrap_or(text);
    let body_start = text.len() - rest.len();
    let (body, range) = call_body("datetime", rest, body_start, text, span)?;
    if body == "null" {
        return Ok(None);
    }
    if !body.starts_with(|c: char| c.is_ascii_digit()) {
        let at = sub(span, range.clone());
        let d = invalid(format!("invalid datetime `{body}`"));
        let d = if body.is_empty() {
            d.primary(at, "expected a date here")
        } else {
            d.primary(at, "not a date")
        };
        return Err(vec![
            d.help("write an ISO 8601 date: `datetime(2024-01-31)` or `datetime(2024-01-31T12:00:00Z)`")
                .help("to convert a value to a datetime, use `todatetime(...)`"),
        ]);
    }
    DateTimeParser {
        body,
        offset: range.start,
        pos: 0,
        span,
    }
    .parse()
    .map(Some)
}

struct DateTimeParser<'a> {
    body: &'a str,
    offset: usize,
    pos: usize,
    span: Span,
}

impl DateTimeParser<'_> {
    fn error(&self, range: Range<usize>, label: impl Into<String>) -> Vec<Diagnostic> {
        let range = self.offset + range.start..self.offset + range.end;
        vec![
            invalid(format!("invalid datetime `{}`", self.body))
                .primary(sub(self.span, range), label)
                .help(
                    "write an ISO 8601 date: `datetime(2024-01-31)` or `datetime(2024-01-31T12:00:00Z)`",
                ),
        ]
    }

    fn peek(&self) -> Option<u8> {
        self.body.as_bytes().get(self.pos).copied()
    }

    /// Reads `min..=max` digits; `what` names the field for the error message.
    fn number(&mut self, min: usize, max: usize, what: &str) -> LiteralResult<(i64, Range<usize>)> {
        let start = self.pos;
        while self.pos - start < max && self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        let len = self.pos - start;
        if len < min || self.peek().is_some_and(|b| b.is_ascii_digit()) {
            let end = self.body[start..]
                .find(|c: char| !c.is_ascii_digit())
                .map_or(self.body.len(), |i| start + i)
                .max(start + 1)
                .min(self.body.len());
            let end = self.body.ceil_char_boundary_compat(end);
            let digits = if min == max {
                format!("{min} digits")
            } else {
                format!("{min} or {max} digits")
            };
            return Err(self.error(start..end.max(start), format!("expected {what} ({digits})")));
        }
        let value = self.body[start..self.pos].parse().unwrap_or(0);
        Ok((value, start..self.pos))
    }

    fn expect(&mut self, byte: u8, what: &str) -> LiteralResult<()> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            let end = (self.pos + 1).min(self.body.len());
            let end = self.body.ceil_char_boundary_compat(end);
            Err(self.error(self.pos..end, format!("expected {what}")))
        }
    }

    /// Reads the whole body first and checks field ranges only afterwards, so that text that
    /// does not belong (`2024-01-0é`) is blamed rather than a field it happens to cut short.
    fn parse(mut self) -> LiteralResult<i64> {
        let year = self.number(4, 4, "a year")?;
        self.expect(b'-', "`-` after the year")?;
        let month = self.number(1, 2, "a month")?;
        self.expect(b'-', "`-` after the month")?;
        let day = self.number(1, 2, "a day")?;
        let mut time = None;
        let mut second = None;
        let mut fraction = 0;
        let mut offset = None;

        if matches!(self.peek(), Some(b'T' | b't' | b' ')) {
            self.pos += 1;
            let hour = self.number(1, 2, "an hour")?;
            self.expect(b':', "`:` after the hour")?;
            let minute = self.number(1, 2, "minutes")?;
            time = Some((hour, minute));
            if self.peek() == Some(b':') {
                self.pos += 1;
                second = Some(self.number(1, 2, "seconds")?);
                if self.peek() == Some(b'.') {
                    self.pos += 1;
                    let start = self.pos;
                    while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                        self.pos += 1;
                    }
                    if self.pos == start {
                        return Err(self.error(start - 1..start, "expected digits after `.`"));
                    }
                    fraction = fraction_micros(&self.body[start..self.pos]) as i64;
                }
            }
            match self.peek() {
                Some(b'Z' | b'z') => self.pos += 1,
                Some(sign @ (b'+' | b'-')) => {
                    self.pos += 1;
                    let oh = self.number(2, 2, "an offset hour")?;
                    self.expect(b':', "`:` in the offset")?;
                    let om = self.number(2, 2, "offset minutes")?;
                    offset = Some((sign, oh, om));
                }
                _ => {}
            }
        }
        if self.pos < self.body.len() {
            let range = self.pos..self.body.len();
            return Err(self.error(range, "unexpected text after the date"));
        }

        let check = |(value, range): &(i64, Range<usize>), max: i64, label: &str| {
            if *value > max {
                Err(self.error(range.clone(), label.to_string()))
            } else {
                Ok(*value)
            }
        };
        if year.0 == 0 {
            return Err(self.error(year.1, "years start at 0001"));
        }
        let year = year.0;
        if !(1..=12).contains(&month.0) {
            return Err(self.error(month.1, "months are 01 to 12"));
        }
        let month = month.0;
        let last_day = days_in_month(year, month);
        if day.0 < 1 || day.0 > last_day {
            return Err(self.error(
                day.1,
                format!("{} has {last_day} days", month_name(year, month)),
            ));
        }
        let mut micros = days_from_civil(year, month, day.0) * 86_400_000_000;
        if let Some((hour, minute)) = &time {
            micros += check(hour, 23, "hours are 00 to 23")? * 3_600_000_000;
            micros += check(minute, 59, "minutes are 00 to 59")? * 60_000_000;
        }
        if let Some(second) = &second {
            micros += check(second, 59, "seconds are 00 to 59")? * 1_000_000 + fraction;
        }
        if let Some((sign, oh, om)) = &offset {
            let oh = check(oh, 23, "offset hours are 00 to 23")?;
            let om = check(om, 59, "offset minutes are 00 to 59")?;
            let offset = oh * 3_600_000_000 + om * 60_000_000;
            if *sign == b'+' {
                micros -= offset;
            } else {
                micros += offset;
            }
        }
        Ok(micros)
    }
}

trait CeilCharBoundary {
    fn ceil_char_boundary_compat(&self, index: usize) -> usize;
}

impl CeilCharBoundary for str {
    fn ceil_char_boundary_compat(&self, mut index: usize) -> usize {
        while index < self.len() && !self.is_char_boundary(index) {
            index += 1;
        }
        index.min(self.len())
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn month_name(year: i64, month: i64) -> String {
    const NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let name = NAMES[(month - 1) as usize];
    if month == 2 {
        format!("February {year}")
    } else {
        name.to_string()
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The value of a [`STRING`](crate::SyntaxKind::STRING) token: `"..."` or `'...'` with escapes, or
/// verbatim `@"..."` / `@'...'` without.
///
/// Escapes: `\\` `\"` `\'` `\n` `\r` `\t` `\0` `\u{1F600}`. Any other escape is DS0005; a string
/// with no closing quote is DS0002. Text that does not start with a quote (or `@` and a quote)
/// is not a string token; it gets a DS0002 rather than a panic.
pub fn string(text: &str, span: Span) -> LiteralResult<String> {
    let verbatim = text.starts_with('@');
    let open_len = if verbatim { 2 } else { 1 };
    let Some(quote @ ('"' | '\'')) = text.get(open_len - 1..).and_then(|t| t.chars().next()) else {
        return Err(vec![
            Diagnostic::error(codes::DS0002, "expected a string")
                .primary(span, "a string starts with `\"` or `'`"),
        ]);
    };
    let content = &text[open_len..];
    let mut errors = Vec::new();

    if verbatim {
        return match content.find(quote) {
            Some(i) if open_len + i + 1 == text.len() => Ok(content[..i].to_string()),
            _ => Err(vec![unterminated(span, quote, false)]),
        };
    }

    let mut value = String::new();
    let mut bad_escapes: Vec<(Range<usize>, String)> = Vec::new();
    let mut chars = content.char_indices().peekable();
    let mut closed = false;
    let mut trailing_backslash = false;
    while let Some((i, c)) = chars.next() {
        let at = open_len + i;
        if c == quote {
            closed = at + 1 == text.len();
            break;
        }
        if c != '\\' {
            value.push(c);
            continue;
        }
        let Some(&(_, e)) = chars.peek() else {
            trailing_backslash = true;
            break;
        };
        chars.next();
        let simple = match e {
            '\\' => Some('\\'),
            '"' => Some('"'),
            '\'' => Some('\''),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            '0' => Some('\0'),
            _ => None,
        };
        if let Some(ch) = simple {
            value.push(ch);
            continue;
        }
        if e == 'u' {
            match unicode_escape(content, i + 2) {
                Ok((ch, len)) => {
                    value.push(ch);
                    for _ in 0..len {
                        chars.next();
                    }
                }
                Err(len) => {
                    for _ in 0..len {
                        chars.next();
                    }
                    let end = at
                        + 2
                        + content[i + 2..]
                            .chars()
                            .take(len)
                            .map(char::len_utf8)
                            .sum::<usize>();
                    errors.push(bad_unicode_escape(&text[at..end], sub(span, at..end)));
                }
            }
            continue;
        }
        let end = at + 1 + e.len_utf8();
        bad_escapes.push((at..end, text[at..end].to_string()));
    }

    if !closed {
        errors.push(unterminated(span, quote, trailing_backslash));
    }
    if !bad_escapes.is_empty() {
        errors.push(unknown_escapes(text, span, &bad_escapes, closed));
    }
    // Source order (the promise on `Lexed`): the unterminated string covers the whole token, so
    // it sorts first; the escapes follow by position.
    errors.sort_by_key(|d| d.primary.as_ref().map(|l| l.span.start));
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(errors)
    }
}

/// Reads `{XXXX}` after `\u` (at `start` in `content`). Returns the character and the number of
/// characters consumed, or the number of characters to skip on error.
fn unicode_escape(content: &str, start: usize) -> Result<(char, usize), usize> {
    let rest = &content[start..];
    if !rest.starts_with('{') {
        return Err(0);
    }
    let Some(close) = rest.find('}') else {
        let len = rest
            .chars()
            .take_while(|c| *c == '{' || c.is_ascii_hexdigit())
            .count();
        return Err(len);
    };
    let hex = &rest[1..close];
    let consumed = rest[..=close].chars().count();
    if hex.is_empty() || hex.len() > 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(consumed);
    }
    let value = u32::from_str_radix(hex, 16).map_err(|_| consumed)?;
    char::from_u32(value).map(|c| (c, consumed)).ok_or(consumed)
}

fn bad_unicode_escape(escape: &str, at: Span) -> Diagnostic {
    Diagnostic::error(codes::DS0005, format!("invalid unicode escape `{escape}`"))
        .primary(at, "not a valid unicode escape")
        .help("write `\\u{` followed by 1 to 6 hex digits and `}`, e.g. `\\u{1F600}`")
}

fn unterminated(span: Span, quote: char, trailing_backslash: bool) -> Diagnostic {
    let end = Span::empty(span.file, span.end);
    let d = Diagnostic::error(codes::DS0002, "unterminated string")
        .primary(span, "this string is never closed");
    // `'abc\` + `'` would be `'abc\'`, still open: the `\` escapes the new quote.
    let d = if trailing_backslash {
        d.help_with(
            format!(
                "the `\\` at the end escapes a quote; to end the string with a backslash, \
                 double it and add a closing `{quote}`"
            ),
            Suggestion::new(
                vec![TextEdit::new(end, format!("\\{quote}"))],
                Applicability::MaybeIncorrect,
            ),
        )
    } else {
        d.help_with(
            format!("add a closing `{quote}` at the end of the string"),
            Suggestion::new(
                vec![TextEdit::new(end, quote.to_string())],
                Applicability::MaybeIncorrect,
            ),
        )
    };
    d.note("a string ends at the end of its line")
}

/// Whether a string reads like a Windows path: a drive (`C:\`) or a UNC path (`\\server`). A
/// rooted path (`\data\x.csv`) has several bad escapes, which counts too (see the caller).
fn looks_like_path(text: &str) -> bool {
    let unc = text[1..]
        .strip_prefix("\\\\")
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_alphanumeric()));
    text.contains(":\\") || unc
}

fn unknown_escapes(
    text: &str,
    span: Span,
    bad: &[(Range<usize>, String)],
    closed: bool,
) -> Diagnostic {
    let (first_range, first) = &bad[0];
    let mut d = Diagnostic::error(codes::DS0005, format!("unknown escape sequence `{first}`"))
        .primary(sub(span, first_range.clone()), "not a valid escape");
    for (range, escape) in &bad[1..] {
        d = d.secondary(
            sub(span, range.clone()),
            format!("`{escape}` is not valid either"),
        );
    }
    // A path is checked before anything else: doubling only the bad escapes of
    // `'C:\data\new.csv'` would leave `\n`, a newline.
    if looks_like_path(text) || bad.len() > 1 {
        let slashed = text.replace('\\', "/");
        let help = format!(
            "if this is a Windows path, use a verbatim string `@{text}` or forward slashes `{slashed}`"
        );
        d = if closed {
            d.help_with(
                help,
                Suggestion::new(
                    vec![TextEdit::new(Span::empty(span.file, span.start), "@")],
                    Applicability::MaybeIncorrect,
                ),
            )
        } else {
            d.help(help)
        };
    } else {
        let mut doubled = String::new();
        let mut last = 0;
        for (range, _) in bad {
            doubled.push_str(&text[last..range.start]);
            doubled.push('\\');
            last = range.start;
        }
        doubled.push_str(&text[last..]);
        let edits = bad
            .iter()
            .map(|(range, _)| {
                TextEdit::new(
                    Span::empty(span.file, span.start + range.start as u32),
                    "\\",
                )
            })
            .collect();
        d = d.help_with(
            format!("to write a backslash, double it: `{doubled}`"),
            Suggestion::new(edits, Applicability::MaybeIncorrect),
        );
    }
    d.note("valid escapes are `\\\\`, `\\\"`, `\\'`, `\\n`, `\\r`, `\\t`, `\\0` and `\\u{...}`")
}

/// The name in a [`QUOTED_IDENT`](crate::SyntaxKind::QUOTED_IDENT) token: `['order id']` is
/// `order id`. The quoted part follows the [`string`] rules.
///
/// An empty name (`['']`) is DS0007: no column, table or `let` can be called that.
pub fn quoted_ident(text: &str, span: Span) -> LiteralResult<String> {
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(text);
    let name = string(inner, sub(span, 1..1 + inner.len()))?;
    if name.is_empty() {
        return Err(vec![
            Diagnostic::error(codes::DS0007, "a name cannot be empty")
                .primary(span, "empty quoted name")
                .help("write the name between the quotes: `['order id']`"),
        ]);
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datascript_diagnostics::FileId;

    fn span(text: &str) -> Span {
        Span::new(FileId(0), 0, text.len() as u32)
    }

    fn ts(text: &str) -> Option<i64> {
        timespan(text, span(text)).unwrap()
    }

    fn ts_err(text: &str) -> Diagnostic {
        timespan(text, span(text)).unwrap_err().remove(0)
    }

    fn dt(text: &str) -> Option<i64> {
        datetime(text, span(text)).unwrap()
    }

    fn dt_err(text: &str) -> Diagnostic {
        datetime(text, span(text)).unwrap_err().remove(0)
    }

    fn primary(d: &Diagnostic) -> Range<usize> {
        d.primary.as_ref().unwrap().span.range()
    }

    #[test]
    fn longs() {
        assert_eq!(long("42", span("42")).unwrap(), 42);
        assert_eq!(long("1_000_000", span("1_000_000")).unwrap(), 1_000_000);
        assert_eq!(
            long("9223372036854775808", span("9223372036854775808")).unwrap(),
            MAX_LONG_LITERAL
        );
        let err = long("9223372036854775809", span("9223372036854775809")).unwrap_err();
        assert_eq!(err[0].code, codes::DS0003);
        let err = long("1_", span("1_")).unwrap_err();
        assert_eq!(primary(&err[0]), 1..2);
    }

    #[test]
    fn reals() {
        assert_eq!(real("2.5", span("2.5")).unwrap(), 2.5);
        assert_eq!(real("1e-3", span("1e-3")).unwrap(), 1e-3);
        assert_eq!(real(".5", span(".5")).unwrap(), 0.5);
        assert_eq!(real("2.5E+2", span("2.5E+2")).unwrap(), 250.0);
        assert!(real("1e999", span("1e999")).is_err());
    }

    #[test]
    fn short_timespans() {
        assert_eq!(ts("1d"), Some(86_400_000_000));
        assert_eq!(ts("2h"), Some(7_200_000_000));
        assert_eq!(ts("30m"), Some(1_800_000_000));
        assert_eq!(ts("10s"), Some(10_000_000));
        assert_eq!(ts("250ms"), Some(250_000));
        assert_eq!(ts("7us"), Some(7));
        assert_eq!(ts("7µs"), Some(7));
        assert_eq!(ts("15tick"), Some(1));
        assert_eq!(ts("1.5h"), Some(5_400_000_000));
        assert_eq!(ts("0.3s"), Some(300_000));
        assert_eq!(ts(".5s"), Some(500_000));
    }

    #[test]
    fn bad_timespan_unit_names_the_units() {
        let d = ts_err("1x");
        assert_eq!(d.code, codes::DS0003);
        assert_eq!(d.message, "unknown timespan unit `x`");
        assert_eq!(primary(&d), 1..2);
        assert!(d.help[0].message.contains("`ms`"));
        let d = ts_err("10min");
        assert!(d.help[1].message.contains("`m`"));
        let d = ts_err("1e3s");
        assert_eq!(d.message, "a timespan cannot use an exponent");
    }

    #[test]
    fn timespan_bodies() {
        assert_eq!(
            ts("timespan(1.02:03:04)"),
            Some(((24 + 2) * 3600 + 3 * 60 + 4) * 1_000_000)
        );
        assert_eq!(ts("timespan(02:03)"), Some((2 * 3600 + 3 * 60) * 1_000_000));
        assert_eq!(ts("timespan(00:00:01.5)"), Some(1_500_000));
        assert_eq!(ts("timespan(-01:00)"), Some(-3_600_000_000));
        assert_eq!(ts("timespan( 90m )"), Some(5_400_000_000));
        assert_eq!(ts("timespan(null)"), None);
        let d = ts_err("timespan(1.25:00:00)");
        assert_eq!(d.primary.unwrap().message, "hours must be 0 to 23");
        let d = ts_err("timespan(x)");
        assert!(d.help[1].message.contains("totimespan"));
        let d = ts_err("timespan(1:00");
        assert_eq!(d.message, "`timespan(` is never closed");
    }

    #[test]
    fn datetimes() {
        assert_eq!(dt("datetime(1970-01-01)"), Some(0));
        assert_eq!(dt("datetime(2024-01-31)"), Some(1_706_659_200_000_000));
        assert_eq!(
            dt("datetime(2024-01-31T12:00:00Z)"),
            Some(1_706_659_200_000_000 + 12 * 3_600_000_000)
        );
        assert_eq!(
            dt("datetime(2024-01-31 12:00)"),
            Some(1_706_659_200_000_000 + 12 * 3_600_000_000)
        );
        assert_eq!(
            dt("datetime(2024-01-31T12:00:00+02:00)"),
            Some(1_706_659_200_000_000 + 10 * 3_600_000_000)
        );
        assert_eq!(
            dt("datetime(2024-01-31T00:00:00.123456789)"),
            Some(1_706_659_200_000_000 + 123_456)
        );
        assert_eq!(dt("datetime(2024-02-29)"), Some(1_709_164_800_000_000));
        assert_eq!(dt("datetime(1969-12-31)"), Some(-86_400_000_000));
        assert_eq!(dt("datetime(null)"), None);
    }

    #[test]
    fn datetime_errors_point_at_the_field() {
        let text = "datetime(2023-02-29)";
        let d = dt_err(text);
        assert_eq!(&text[primary(&d)], "29");
        assert_eq!(d.primary.unwrap().message, "February 2023 has 28 days");
        let text = "datetime(2024-13-01)";
        assert_eq!(&text[primary(&dt_err(text))], "13");
        let text = "datetime(2024-01-31T25:00)";
        assert_eq!(&text[primary(&dt_err(text))], "25");
        let text = "datetime(2024-01-31x)";
        assert_eq!(&text[primary(&dt_err(text))], "x");
        let d = dt_err("datetime(now())");
        assert!(d.help[1].message.contains("todatetime"));
        let d = dt_err("datetime(2024-01");
        assert_eq!(d.message, "`datetime(` is never closed");
    }

    #[test]
    fn strings() {
        let s = |t: &str| string(t, span(t));
        assert_eq!(s(r#""text""#).unwrap(), "text");
        assert_eq!(s("'text'").unwrap(), "text");
        assert_eq!(s(r#"@"C:\raw""#).unwrap(), r"C:\raw");
        assert_eq!(s(r"@'C:\raw'").unwrap(), r"C:\raw");
        assert_eq!(
            s(r#"'\\ \" \' \n \r \t \0'"#).unwrap(),
            "\\ \" ' \n \r \t \0"
        );
        assert_eq!(s(r"'\u{1F600}'").unwrap(), "😀");
        assert_eq!(s(r#""it's""#).unwrap(), "it's");
        assert_eq!(s("''").unwrap(), "");
    }

    #[test]
    fn string_errors() {
        let s = |t: &str| string(t, span(t)).unwrap_err();
        let errors = s("'abc");
        assert_eq!(errors[0].code, codes::DS0002);
        assert_eq!(primary(&errors[0]), 0..4);

        let text = r"'C:\data\x.csv'";
        let errors = s(text);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, codes::DS0005);
        assert_eq!(&text[primary(&errors[0])], r"\d");
        assert_eq!(errors[0].secondary.len(), 1);
        assert!(errors[0].help[0].message.contains(r"@'C:\data\x.csv'"));
        assert!(errors[0].help[0].message.contains("'C:/data/x.csv'"));

        let errors = s(r"'\u{110000}'");
        assert_eq!(errors[0].message, r"invalid unicode escape `\u{110000}`");
        let errors = s(r"'\u12'");
        assert_eq!(errors[0].message, r"invalid unicode escape `\u`");
        assert_eq!(s(r"@'abc")[0].code, codes::DS0002);
    }

    #[test]
    fn quoted_idents() {
        let q = |t: &str| quoted_ident(t, span(t));
        assert_eq!(q("['order id']").unwrap(), "order id");
        assert_eq!(q(r#"["x"]"#).unwrap(), "x");
    }

    /// Applies a suggestion's edits to `text` (which starts at offset 0).
    fn apply(text: &str, d: &Diagnostic, help: usize) -> String {
        let mut out = text.to_string();
        let mut edits = d.help[help].suggestion.as_ref().unwrap().edits.clone();
        edits.sort_by_key(|e| std::cmp::Reverse(e.span.start));
        for e in edits {
            out.replace_range(e.span.range(), &e.replacement);
        }
        out
    }

    #[test]
    fn tiny_timespan_fractions_are_truncated() {
        let text = "0.0000000000000000000000000000000000000001s";
        assert_eq!(ts(text), Some(0));
        assert_eq!(ts("1.0000019999999999999999999999999s"), Some(1_000_001));
        assert_eq!(ts("0.000_000_5s"), Some(0));
    }

    #[test]
    fn greek_mu_and_negative_short_form() {
        assert_eq!(ts("7μs"), Some(7));
        assert_eq!(ts("timespan(-90m)"), Some(-5_400_000_000));
        assert_eq!(ts("timespan( -1.5h )"), Some(-5_400_000_000));
        let text = "timespan(-x)";
        assert_eq!(&text[primary(&ts_err(text))], "-x");
    }

    #[test]
    fn hex_numbers() {
        let d = ts_err("0x1F");
        assert_eq!(d.message, "hexadecimal numbers are not supported");
        assert_eq!(primary(&d), 0..4);
        assert!(d.help[0].message.contains("`31`"), "{}", d.help[0].message);
        let d = ts_err("0xFFFFFFFFFFFFFFFFFF");
        assert_eq!(d.help[0].message, "write the number in decimal");
        // Not hex: a bad unit.
        assert_eq!(ts_err("0xg").message, "unknown timespan unit `xg`");
        assert_eq!(ts_err("1x1F").message, "unknown timespan unit `x1F`");
    }

    #[test]
    fn datetime_spans_stay_on_char_boundaries() {
        let text = "datetime(2024-é1-01)";
        let d = dt_err(text);
        assert_eq!(&text[primary(&d)], "é");
        let text = "datetime(é)";
        assert!(text.is_char_boundary(primary(&dt_err(text)).end));
    }

    #[test]
    fn datetime_blames_the_stray_text() {
        let text = "datetime(2024-01-0é)";
        let d = dt_err(text);
        assert_eq!(&text[primary(&d)], "é");
        assert_eq!(d.primary.unwrap().message, "unexpected text after the date");
        let text = "datetime(2024-01-31T12:99x)";
        assert_eq!(&text[primary(&dt_err(text))], "x");
        // Ranges are still checked, in source order.
        let text = "datetime(2024-13-01T25:00)";
        assert_eq!(&text[primary(&dt_err(text))], "13");
        let text = "datetime(2024-01-01T10:00+24:00)";
        assert_eq!(&text[primary(&dt_err(text))], "24");
    }

    #[test]
    fn unterminated_string_with_trailing_backslash() {
        let text = r"'abc\";
        let d = string(text, span(text)).unwrap_err().remove(0);
        assert_eq!(d.code, codes::DS0002);
        let fixed = apply(text, &d, 0);
        assert_eq!(fixed, r"'abc\\'");
        assert_eq!(string(&fixed, span(&fixed)).unwrap(), r"abc\");
        // Two backslashes are an escaped backslash: just close the string.
        let text = r"'abc\\";
        let d = string(text, span(text)).unwrap_err().remove(0);
        assert_eq!(apply(text, &d, 0), r"'abc\\'");
        // Verbatim strings have no escapes.
        let text = r"@'abc\";
        let d = string(text, span(text)).unwrap_err().remove(0);
        assert_eq!(apply(text, &d, 0), r"@'abc\'");
    }

    #[test]
    fn windows_paths_with_a_valid_escape() {
        let text = r"'C:\data\new.csv'";
        let errors = string(text, span(text)).unwrap_err();
        assert_eq!(errors.len(), 1);
        let d = &errors[0];
        assert!(
            d.help[0].message.contains(r"@'C:\data\new.csv'"),
            "{}",
            d.help[0].message
        );
        assert!(d.help[0].message.contains("'C:/data/new.csv'"));
        let fixed = apply(text, d, 0);
        assert_eq!(string(&fixed, span(&fixed)).unwrap(), r"C:\data\new.csv");
        // A rooted or UNC path.
        let text = r"'\\server\share'";
        let d = &string(text, span(text)).unwrap_err()[0];
        assert!(d.help[0].message.contains("forward slashes"));
        // A string that merely starts with an escape is not a path.
        let text = r"'\qx'";
        let d = &string(text, span(text)).unwrap_err()[0];
        assert!(
            d.help[0].message.contains("double it"),
            "{}",
            d.help[0].message
        );
        // Not a path: one bad escape is doubled.
        let text = r"'a\qb'";
        let d = &string(text, span(text)).unwrap_err()[0];
        assert!(d.help[0].message.contains(r"double it: `'a\\qb'`"));
    }

    #[test]
    fn string_diagnostics_are_in_source_order() {
        let text = r"'\q\u{zz}'";
        let errors = string(text, span(text)).unwrap_err();
        let starts: Vec<_> = errors.iter().map(|d| primary(d).start).collect();
        assert_eq!(starts, [1, 3]);
        let text = r"'\u{zz}\q";
        let errors = string(text, span(text)).unwrap_err();
        assert_eq!(errors[0].code, codes::DS0002);
        let starts: Vec<_> = errors.iter().map(|d| primary(d).start).collect();
        assert!(starts.is_sorted(), "{starts:?}");
    }

    #[test]
    fn string_without_a_quote_does_not_panic() {
        for text in ["é", "", "@", "@é", "x"] {
            let d = &string(text, span(text)).unwrap_err()[0];
            assert_eq!(d.code, codes::DS0002, "{text:?}");
        }
    }

    #[test]
    fn empty_quoted_name() {
        let q = |t: &str| quoted_ident(t, span(t));
        let d = &q("['']").unwrap_err()[0];
        assert_eq!(d.code, codes::DS0007);
        assert_eq!(primary(d), 0..4);
        assert_eq!(q(r#"[""]"#).unwrap_err()[0].code, codes::DS0007);
        assert_eq!(q("[' ']").unwrap(), " ");
    }
}
