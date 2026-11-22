//! Rules for argument values that the language defines (LANGUAGE.md §7): datetime part names,
//! `format_datetime` specifiers, regular expressions and percentages.
//!
//! The binder applies them to constant arguments, so `ds check` reports a bad one (`DS4004`)
//! before anything runs. Engines apply the same rules to computed values at run time, so the two
//! never disagree on what is valid or on the message.

/// A bad argument value: the diagnostic's message and a note listing what is allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidValue {
    /// One line, in the style of DIAGNOSTICS.md.
    pub message: String,
    /// What the valid values are.
    pub note: String,
}

impl InvalidValue {
    fn new(message: impl Into<String>, note: impl Into<String>) -> Self {
        InvalidValue {
            message: message.into(),
            note: note.into(),
        }
    }
}

/// A part of a datetime, for `datetime_part` and `datetime_diff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatetimePart {
    /// `year`
    Year,
    /// `quarter`
    Quarter,
    /// `month`
    Month,
    /// `week_of_year` or `week`
    WeekOfYear,
    /// `day`
    Day,
    /// `dayofyear`
    DayOfYear,
    /// `hour`
    Hour,
    /// `minute`
    Minute,
    /// `second`
    Second,
    /// `millisecond`
    Millisecond,
    /// `microsecond`
    Microsecond,
    /// `nanosecond`
    Nanosecond,
}

/// The part names, in the order messages list them.
pub const DATETIME_PARTS: &[&str] = &[
    "year",
    "quarter",
    "month",
    "week_of_year",
    "week",
    "day",
    "dayofyear",
    "hour",
    "minute",
    "second",
    "millisecond",
    "microsecond",
    "nanosecond",
];

/// The part names, as listed in messages.
pub const DATETIME_PART_NAMES: &str = "\"year\", \"quarter\", \"month\", \"week_of_year\", \
     \"week\", \"day\", \"dayofyear\", \"hour\", \"minute\", \"second\", \"millisecond\", \
     \"microsecond\", \"nanosecond\"";

impl DatetimePart {
    /// The part with this name, case-insensitively.
    pub fn parse(name: &str) -> Result<DatetimePart, InvalidValue> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "year" => DatetimePart::Year,
            "quarter" => DatetimePart::Quarter,
            "month" => DatetimePart::Month,
            "week_of_year" | "week" => DatetimePart::WeekOfYear,
            "day" => DatetimePart::Day,
            "dayofyear" => DatetimePart::DayOfYear,
            "hour" => DatetimePart::Hour,
            "minute" => DatetimePart::Minute,
            "second" => DatetimePart::Second,
            "millisecond" => DatetimePart::Millisecond,
            "microsecond" => DatetimePart::Microsecond,
            "nanosecond" => DatetimePart::Nanosecond,
            _ => {
                return Err(InvalidValue::new(
                    format!("`{name}` is not a datetime part"),
                    format!("the parts are {DATETIME_PART_NAMES}"),
                ));
            }
        })
    }
}

/// Whether a run of `n` copies of the letter `c` is a `format_datetime` specifier (`yyyy`,
/// `MM`, `fff`, …).
pub fn is_format_specifier(c: char, n: usize) -> bool {
    matches!(
        (c, n),
        ('y', 1 | 2 | 4)
            | ('M' | 'd' | 'H' | 'h' | 'm' | 's', 1 | 2)
            | ('f' | 'F', 1..=7)
            | ('t', 2)
    )
}

/// The error for a letter run that is not a specifier.
pub fn unknown_format_specifier(run: &str) -> InvalidValue {
    InvalidValue::new(
        format!("`{run}` is not a datetime format specifier"),
        "the specifiers are yyyy, yy, y, MM, M, dd, d, HH, H, hh, h, mm, m, ss, s, f to fffffff, \
         F to FFFFFFF and tt; other characters that are not letters are copied",
    )
}

/// Checks a `format_datetime` format: every run of one repeated letter must be a specifier.
pub fn check_datetime_format(format: &str) -> Result<(), InvalidValue> {
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let n = chars[i..].iter().take_while(|&&x| x == c).count();
        if c.is_ascii_alphabetic() && !is_format_specifier(c, n) {
            return Err(unknown_format_specifier(&c.to_string().repeat(n)));
        }
        i += n;
    }
    Ok(())
}

/// Checks the percentage of `percentile`.
pub fn check_percentage(p: f64) -> Result<(), InvalidValue> {
    if (0.0..=100.0).contains(&p) {
        Ok(())
    } else {
        Err(InvalidValue::new(
            format!("percentile {p} is outside 0 to 100"),
            "the second argument of `percentile` is a percentage from 0 to 100",
        ))
    }
}

/// The error for a regular expression that does not compile, from the regex engine's message.
/// The note is the message's last line, which says what is wrong ("unclosed group").
pub fn invalid_regex(pattern: &str, error: &str) -> InvalidValue {
    let line = error
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(error)
        .trim();
    let line = line.strip_prefix("error: ").unwrap_or(line);
    InvalidValue::new(
        format!("`{pattern}` is not a valid regular expression"),
        format!("{line} (regexes use Rust `regex` syntax)"),
    )
}

/// Checks the syntax of a regular expression (D-014: Rust `regex` syntax). An engine may still
/// reject a pattern that parses, for example one too large to compile.
pub fn check_regex(pattern: &str) -> Result<(), InvalidValue> {
    regex_syntax::Parser::new()
        .parse(pattern)
        .map(|_| ())
        .map_err(|e| invalid_regex(pattern, &e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_lists_agree() {
        let quoted: Vec<String> = DATETIME_PARTS.iter().map(|p| format!("\"{p}\"")).collect();
        assert_eq!(quoted.join(", "), DATETIME_PART_NAMES);
        assert!(
            DATETIME_PARTS
                .iter()
                .all(|p| DatetimePart::parse(p).is_ok())
        );
    }

    #[test]
    fn parts_are_case_insensitive() {
        assert_eq!(DatetimePart::parse("Week"), Ok(DatetimePart::WeekOfYear));
        let e = DatetimePart::parse("fortnight").unwrap_err();
        assert_eq!(e.message, "`fortnight` is not a datetime part");
    }

    #[test]
    fn formats() {
        assert!(check_datetime_format("yyyy-MM-dd HH:mm:ss.fffffff tt").is_ok());
        assert!(check_datetime_format("").is_ok());
        let e = check_datetime_format("yyyy-QQ").unwrap_err();
        assert_eq!(e.message, "`QQ` is not a datetime format specifier");
        assert!(check_datetime_format("yyy").is_err());
        assert!(check_datetime_format("ffffffff").is_err());
    }

    #[test]
    fn percentages() {
        assert!(check_percentage(0.0).is_ok() && check_percentage(100.0).is_ok());
        let e = check_percentage(150.0).unwrap_err();
        assert_eq!(e.message, "percentile 150 is outside 0 to 100");
        assert!(check_percentage(-0.5).is_err() && check_percentage(f64::NAN).is_err());
    }

    #[test]
    fn regexes() {
        assert!(check_regex(r"(\w)-(\d+)").is_ok());
        let e = check_regex("(o-").unwrap_err();
        assert_eq!(e.message, "`(o-` is not a valid regular expression");
        assert_eq!(e.note, "unclosed group (regexes use Rust `regex` syntax)");
    }
}
