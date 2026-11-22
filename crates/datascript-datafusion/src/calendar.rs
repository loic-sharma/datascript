//! Calendar arithmetic on microseconds since the Unix epoch, in UTC (LANGUAGE.md §7).
//!
//! Hand-written rather than through chrono so the rules are visible: weeks start on Sunday
//! (D-014), `week_of_year` is the ISO 8601 week, and every result is checked for overflow.

use datascript_compiler::arg_values::unknown_format_specifier;

use crate::errors::Fail;

pub(crate) const MICROS_PER_SECOND: i64 = 1_000_000;
pub(crate) const MICROS_PER_MINUTE: i64 = 60 * MICROS_PER_SECOND;
pub(crate) const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MINUTE;
pub(crate) const MICROS_PER_DAY: i64 = 24 * MICROS_PER_HOUR;

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number: (year, month 1–12, day 1–31).
pub(crate) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The fields of an instant.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Parts {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    /// Days since the epoch.
    pub days: i64,
    /// Microseconds since midnight.
    pub time: i64,
}

pub(crate) fn parts(us: i64) -> Parts {
    let days = us.div_euclid(MICROS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    Parts {
        year,
        month,
        day,
        days,
        time: us.rem_euclid(MICROS_PER_DAY),
    }
}

/// 0 for Sunday … 6 for Saturday. 1970-01-01 was a Thursday.
pub(crate) fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

fn at_midnight(days: i64) -> Result<i64, Fail> {
    days.checked_mul(MICROS_PER_DAY).ok_or(Fail::Overflow)
}

/// A period that `startof*`/`endof*` round to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Period {
    Day,
    Week,
    Month,
    Year,
}

/// The start of the period containing `us`, moved by `offset` periods.
pub(crate) fn start_of(period: Period, us: i64, offset: i64) -> Result<i64, Fail> {
    let p = parts(us);
    let days = match period {
        Period::Day => p.days.checked_add(offset),
        Period::Week => offset
            .checked_mul(7)
            .and_then(|o| (p.days - weekday(p.days)).checked_add(o)),
        Period::Month => {
            let month0 = p.month - 1;
            let total = p.year.checked_mul(12).and_then(|y| y.checked_add(month0));
            let total = total
                .and_then(|t| t.checked_add(offset))
                .ok_or(Fail::Overflow)?;
            year_range(total.div_euclid(12))?;
            Some(days_from_civil(
                total.div_euclid(12),
                total.rem_euclid(12) + 1,
                1,
            ))
        }
        Period::Year => {
            let year = p.year.checked_add(offset).ok_or(Fail::Overflow)?;
            year_range(year)?;
            Some(days_from_civil(year, 1, 1))
        }
    };
    at_midnight(days.ok_or(Fail::Overflow)?)
}

/// The last microsecond of the period containing `us`, moved by `offset` periods.
pub(crate) fn end_of(period: Period, us: i64, offset: i64) -> Result<i64, Fail> {
    let next = offset.checked_add(1).ok_or(Fail::Overflow)?;
    Ok(start_of(period, us, next)? - 1)
}

/// Years far outside the range of a `datetime`; checked before day arithmetic so it cannot
/// overflow.
fn year_range(year: i64) -> Result<(), Fail> {
    if year.abs() > 400_000 {
        Err(Fail::Overflow)
    } else {
        Ok(())
    }
}

/// The parts `datetime_part` and `datetime_diff` accept.
/// Part names are a language rule, shared with the binder's check of constant parts.
pub(crate) use datascript_compiler::arg_values::DatetimePart as Part;

/// `datetime_part(part, us)`.
pub(crate) fn part_of(part: Part, us: i64) -> i64 {
    let p = parts(us);
    match part {
        Part::Year => p.year,
        Part::Quarter => (p.month - 1) / 3 + 1,
        Part::Month => p.month,
        Part::WeekOfYear => iso_week(p.days),
        Part::Day => p.day,
        Part::DayOfYear => p.days - days_from_civil(p.year, 1, 1) + 1,
        Part::Hour => p.time / MICROS_PER_HOUR,
        Part::Minute => p.time / MICROS_PER_MINUTE % 60,
        Part::Second => p.time / MICROS_PER_SECOND % 60,
        Part::Millisecond => p.time / 1000 % 1000,
        Part::Microsecond => p.time % MICROS_PER_SECOND,
        Part::Nanosecond => p.time % MICROS_PER_SECOND * 1000,
    }
}

/// The ISO 8601 week number (weeks start on Monday; week 1 holds the year's first Thursday).
fn iso_week(days: i64) -> i64 {
    // ISO weekday: Monday 0 … Sunday 6.
    let iso_wd = (weekday(days) + 6) % 7;
    let thursday = days - iso_wd + 3;
    let (year, _, _) = civil_from_days(thursday);
    (thursday - days_from_civil(year, 1, 1)) / 7 + 1
}

/// `datetime_diff(part, a, b)`: how many `part` boundaries lie between `b` and `a`.
pub(crate) fn diff(part: Part, a: i64, b: i64) -> Result<i64, Fail> {
    let (pa, pb) = (parts(a), parts(b));
    let floor_diff = |unit: i64| a.div_euclid(unit) - b.div_euclid(unit);
    Ok(match part {
        Part::Year => pa.year - pb.year,
        Part::Quarter => (pa.year * 4 + (pa.month - 1) / 3) - (pb.year * 4 + (pb.month - 1) / 3),
        Part::Month => (pa.year * 12 + pa.month) - (pb.year * 12 + pb.month),
        // Weeks start on Sunday, like `startofweek`.
        Part::WeekOfYear => ((pa.days - weekday(pa.days)) - (pb.days - weekday(pb.days))) / 7,
        Part::Day | Part::DayOfYear => pa.days - pb.days,
        Part::Hour => floor_diff(MICROS_PER_HOUR),
        Part::Minute => floor_diff(MICROS_PER_MINUTE),
        Part::Second => floor_diff(MICROS_PER_SECOND),
        Part::Millisecond => floor_diff(1000),
        Part::Microsecond => a.checked_sub(b).ok_or(Fail::Overflow)?,
        Part::Nanosecond => a
            .checked_sub(b)
            .and_then(|d| d.checked_mul(1000))
            .ok_or(Fail::Overflow)?,
    })
}

/// `format_datetime(us, format)` with KQL's specifiers. An unknown letter run is an error.
pub(crate) fn format(us: i64, format: &str) -> Result<String, Fail> {
    let p = parts(us);
    let hour = p.time / MICROS_PER_HOUR;
    let minute = p.time / MICROS_PER_MINUTE % 60;
    let second = p.time / MICROS_PER_SECOND % 60;
    let micros = p.time % MICROS_PER_SECOND;
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        if !c.is_ascii_alphabetic() {
            out.extend(std::iter::repeat_n(c, n));
            i += n;
            continue;
        }
        let hour12 = match hour % 12 {
            0 => 12,
            h => h,
        };
        let text = match (c, n) {
            ('y', 1) => (p.year % 100).to_string(),
            ('y', 2) => format!("{:02}", p.year.rem_euclid(100)),
            ('y', 4) => format!("{:04}", p.year),
            ('M', 1) => p.month.to_string(),
            ('M', 2) => format!("{:02}", p.month),
            ('d', 1) => p.day.to_string(),
            ('d', 2) => format!("{:02}", p.day),
            ('H', 1) => hour.to_string(),
            ('H', 2) => format!("{hour:02}"),
            ('h', 1) => hour12.to_string(),
            ('h', 2) => format!("{hour12:02}"),
            ('m', 1) => minute.to_string(),
            ('m', 2) => format!("{minute:02}"),
            ('s', 1) => second.to_string(),
            ('s', 2) => format!("{second:02}"),
            ('f', 1..=7) => fraction(micros, n),
            ('F', 1..=7) => fraction(micros, n).trim_end_matches('0').to_string(),
            ('t', 2) => if hour < 12 { "AM" } else { "PM" }.to_string(),
            _ => {
                // `arg_values::is_format_specifier` lists the same specifiers as this match.
                let run: String = std::iter::repeat_n(c, n).collect();
                let e = unknown_format_specifier(&run);
                return Err(Fail::invalid(e.message, e.note));
            }
        };
        out.push_str(&text);
        i += n;
    }
    Ok(out)
}

/// The first `n` digits of a 7-digit (100 ns) fraction.
fn fraction(micros: i64, n: usize) -> String {
    let ticks = format!("{:07}", micros * 10);
    ticks[..n].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use datascript_compiler::arg_values::is_format_specifier;

    /// `format` and the binder's check (`arg_values`) accept exactly the same letter runs.
    #[test]
    fn format_specifiers_match_the_shared_rule() {
        for c in ('a'..='z').chain('A'..='Z') {
            for n in 1..=8 {
                let run = c.to_string().repeat(n);
                assert_eq!(
                    format(0, &run).is_ok(),
                    is_format_specifier(c, n),
                    "`{run}`"
                );
            }
        }
    }

    fn us(y: i64, m: i64, d: i64) -> i64 {
        days_from_civil(y, m, d) * MICROS_PER_DAY
    }

    #[test]
    fn civil_round_trips() {
        for days in [-800_000, -1, 0, 1, 19_782, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2024, 2, 29), 19_782);
    }

    #[test]
    fn weeks_start_on_sunday() {
        // 2024-02-29 is a Thursday; its week starts on Sunday 2024-02-25.
        let t = us(2024, 2, 29) + 5 * MICROS_PER_HOUR;
        assert_eq!(start_of(Period::Week, t, 0), Ok(us(2024, 2, 25)));
        assert_eq!(end_of(Period::Week, t, 0), Ok(us(2024, 3, 3) - 1));
        assert_eq!(start_of(Period::Month, t, -2), Ok(us(2023, 12, 1)));
    }

    #[test]
    fn iso_weeks() {
        assert_eq!(part_of(Part::WeekOfYear, us(2021, 1, 3)), 53);
        assert_eq!(part_of(Part::WeekOfYear, us(2021, 1, 4)), 1);
        assert_eq!(part_of(Part::WeekOfYear, us(2024, 12, 30)), 1);
    }

    #[test]
    fn formats() {
        let t = us(2024, 3, 5) + 13 * MICROS_PER_HOUR + 7 * MICROS_PER_MINUTE + 1_250_000;
        assert_eq!(
            format(t, "yyyy-MM-dd HH:mm:ss.fff").unwrap(),
            "2024-03-05 13:07:01.250"
        );
        assert_eq!(format(t, "M/d/yy h:mm tt").unwrap(), "3/5/24 1:07 PM");
        assert_eq!(format(t, "s.FFFF").unwrap(), "1.25");
        assert!(format(t, "yyyy-QQ").is_err());
    }
}
