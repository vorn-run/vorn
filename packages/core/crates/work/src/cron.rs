//! Cron expressions as node-cron 4.2 reads them, so a schedule fires in the
//! same minutes whichever side arms it.
//!
//! node-cron is not Vixie cron, and the differences are kept:
//!
//! - five fields get a seconds field of `0` in front; six are taken as they
//!   are, and fields past the sixth are ignored;
//! - month and weekday names are replaced anywhere in their field, case
//!   insensitive, and only the first `7` in the weekday field means Sunday
//!   (`1-7` is Sunday and Monday, a range swapped into `0-1`);
//! - only the first `*` of a field is its full range;
//! - a range is `a-b` or `a-b/s`, its ends swapped when `a > b`; a value is
//!   whatever `parseInt` makes of it, so `5/10` is `5` and `0x1f` is 31;
//! - a time matches when every field does: day of month and day of week are
//!   both required, never either.
//!
//! Times are matched on the wall clock of the schedule's time zone, at whole
//! seconds, as the runner's heartbeat does.

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

/// Each field's bounds, and the range its first `*` stands for. The weekday
/// accepts 7 but its `*` is `0-6`.
const FIELDS: [(i64, i64, &str); 6] = [
    (0, 59, "0-59"),
    (0, 59, "0-59"),
    (0, 23, "0-23"),
    (1, 31, "1-31"),
    (1, 12, "1-12"),
    (0, 7, "0-6"),
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
const SHORT_MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];
const SHORT_WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// The most values one range may produce. node-cron would build any list,
/// and refuse it afterwards when a value is out of bounds; a bound here
/// keeps a typo from allocating without end.
const MAX_RANGE_VALUES: i64 = 10_000;

/// An expression node-cron's `validate` refuses: such a schedule is never armed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidCron(pub String);

impl std::fmt::Display for InvalidCron {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` is not a cron expression node-cron accepts", self.0)
    }
}

impl std::error::Error for InvalidCron {}

/// A parsed expression: one bit per allowed value, for second, minute, hour,
/// day of month, month and day of week (Sunday 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cron {
    fields: [u64; 6],
}

impl Cron {
    /// Reads `expression` as node-cron's `validate` and `schedule` do.
    pub fn parse(expression: &str) -> Result<Cron, InvalidCron> {
        let invalid = || InvalidCron(expression.to_owned());
        let normalized = collapse_spaces(expression);
        let mut parts: Vec<String> = normalized.split(' ').map(str::to_owned).collect();
        if parts.len() == 5 {
            parts.insert(0, "0".to_owned());
        }
        if parts.len() < 6 {
            return Err(invalid());
        }
        parts[4] = month_names(&parts[4]);
        parts[5] = weekday_names(&parts[5]);
        let mut fields = [0u64; 6];
        for (i, part) in parts.iter().take(6).enumerate() {
            let (min, max, all) = FIELDS[i];
            let expanded = expand_ranges(&part.replacen('*', all, 1)).ok_or_else(invalid)?;
            for item in expanded.split(',') {
                let n = parse_int(item)
                    .filter(|n| (min..=max).contains(n))
                    .ok_or_else(invalid)?;
                fields[i] |= 1 << n;
            }
        }
        Ok(Cron { fields })
    }

    /// Whether node-cron fires at the wall-clock time `at`.
    pub fn matches(&self, at: &Zoned) -> bool {
        let allows = |field: usize, value: i8| {
            u32::try_from(value).is_ok_and(|v| v < 64 && self.fields[field] & (1 << v) != 0)
        };
        allows(0, at.second())
            && allows(1, at.minute())
            && allows(2, at.hour())
            && allows(3, at.day())
            && allows(4, at.month())
            && allows(5, at.weekday().to_sunday_zero_offset())
    }

    /// The first second, in Unix milliseconds, at which node-cron fires in
    /// the minute `minute` (Unix minutes, as the scheduler's tick locks are
    /// keyed), in time zone `tz`; `None` when it does not fire in it.
    pub fn fires_in(&self, minute: i64, tz: &TimeZone) -> Option<i64> {
        let start = minute.checked_mul(60)?;
        let at = |second: i64| {
            Timestamp::from_second(start + second)
                .ok()
                .map(|t| t.to_zoned(tz.clone()))
        };
        let first = at(0)?;
        if first.offset().seconds() % 60 == 0 {
            // A whole-minute offset keeps the minute's wall clock but its seconds.
            let second = self.fields[0].trailing_zeros();
            let probe = at(i64::from(second))?;
            return self
                .matches(&probe)
                .then(|| probe.timestamp().as_millisecond());
        }
        (0..60)
            .filter_map(at)
            .find(|z| self.matches(z))
            .map(|z| z.timestamp().as_millisecond())
    }
}

/// `str.replace(/\s{2,}/g, ' ').trim()`.
fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    for c in text.chars() {
        if is_js_space(c) {
            run.push(c);
            continue;
        }
        if run.chars().count() >= 2 {
            out.push(' ');
        } else {
            out.push_str(&run);
        }
        run.clear();
        out.push(c);
    }
    out.push_str(&run);
    out.trim_matches(is_js_space).to_owned()
}

/// What `\s` and `trim` count as space.
fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn month_names(field: &str) -> String {
    let field = replace_names(field, &MONTHS, 1);
    replace_names(&field, &SHORT_MONTHS, 1)
}

fn weekday_names(field: &str) -> String {
    let field = field.replacen('7', "0", 1);
    let field = replace_names(&field, &WEEKDAYS, 0);
    replace_names(&field, &SHORT_WEEKDAYS, 0)
}

/// Replaces every case-insensitive occurrence of each name, in order, by its
/// index plus `first`.
fn replace_names(field: &str, names: &[&str], first: usize) -> String {
    let mut out = field.to_owned();
    for (i, name) in names.iter().enumerate() {
        out = replace_ignoring_case(&out, name, &(i + first).to_string());
    }
    out
}

fn replace_ignoring_case(haystack: &str, needle: &str, with: &str) -> String {
    let lower = haystack.to_ascii_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut rest = 0;
    while let Some(found) = lower[rest..].find(needle) {
        out.push_str(&haystack[rest..rest + found]);
        out.push_str(with);
        rest += found + needle.len();
    }
    out.push_str(&haystack[rest..]);
    out
}

/// Replaces each `a-b(/s)?`, leftmost first, with the values it stands for,
/// until none is left. `None` for a step of zero (node-cron never returns)
/// or a range too long to list.
fn expand_ranges(field: &str) -> Option<String> {
    let mut field = field.to_owned();
    while let Some(found) = find_range(&field) {
        let (mut first, mut last) = (found.from, found.to);
        if first > last {
            std::mem::swap(&mut first, &mut last);
        }
        let step = found.step.unwrap_or(1);
        if step == 0 || (last - first) / step >= MAX_RANGE_VALUES {
            return None;
        }
        let values: Vec<String> = (first..=last)
            .step_by(usize::try_from(step).ok()?)
            .map(|n| n.to_string())
            .collect();
        field.replace_range(found.span, &values.join(","));
    }
    Some(field)
}

struct Range {
    span: std::ops::Range<usize>,
    from: i64,
    to: i64,
    step: Option<i64>,
}

/// The leftmost match of `/(\d+)-(\d+)(\/(\d+)|)/`.
fn find_range(text: &str) -> Option<Range> {
    let bytes = text.as_bytes();
    let digits = |at: usize| {
        bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let number = |from: usize, len: usize| {
        text[from..from + len]
            .parse::<i64>()
            .unwrap_or(i64::MAX / 2)
    };
    (0..bytes.len()).find_map(|start| {
        let a = digits(start);
        if a == 0 || bytes.get(start + a) != Some(&b'-') {
            return None;
        }
        let b_at = start + a + 1;
        let b = digits(b_at);
        if b == 0 {
            return None;
        }
        let mut end = b_at + b;
        let mut step = None;
        if bytes.get(end) == Some(&b'/') {
            let s = digits(end + 1);
            if s > 0 {
                step = Some(number(end + 1, s));
                end += 1 + s;
            }
        }
        Some(Range {
            span: start..end,
            from: number(start, a),
            to: number(b_at, b),
            step,
        })
    })
}

/// `parseInt(text)` without a radix, for a value that fits a field: leading
/// space, a sign, `0x` for hexadecimal, then as many digits as there are.
/// `None` where it would be `NaN`.
fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim_start_matches(is_js_space);
    let (negative, text) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let (radix, text) = match text.get(..2) {
        Some("0x" | "0X") => (16, &text[2..]),
        _ => (10, text),
    };
    let digits: String = text.chars().take_while(|c| c.is_digit(radix)).collect();
    if digits.is_empty() {
        return None;
    }
    // Too long for a field is out of bounds either way.
    let n = i64::from_str_radix(&digits, radix).unwrap_or(i64::MAX);
    Some(if negative { -n } else { n })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(cron: &Cron, field: usize) -> Vec<u32> {
        (0..64)
            .filter(|v| cron.fields[field] & (1 << v) != 0)
            .collect()
    }

    fn parse(expression: &str) -> Cron {
        Cron::parse(expression).unwrap_or_else(|e| panic!("{e}"))
    }

    fn utc() -> TimeZone {
        TimeZone::UTC
    }

    fn minute_of(iso: &str) -> i64 {
        iso.parse::<Timestamp>().unwrap().as_second() / 60
    }

    #[test]
    fn five_fields_fire_at_second_zero_and_six_at_their_own() {
        assert_eq!(values(&parse("* * * * *"), 0), [0]);
        assert_eq!(values(&parse("*/20 * * * * *"), 0), [0, 20, 40]);
        assert_eq!(values(&parse("  1   2 * * *  "), 1), [1]);
        assert_eq!(values(&parse("0 0 1 1 * 0 extra"), 3), [1]);
    }

    #[test]
    fn refuses_what_validate_refuses() {
        for bad in [
            "",
            "* * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "a * * * *",
            "-1 * * * *",
            "*/0 * * * *",
            "0-99999999 * * * *",
            "1,,2 * * * *",
        ] {
            assert!(Cron::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn keeps_node_crons_leniencies() {
        // parseInt reads the leading number and stops.
        assert_eq!(values(&parse("5/10 * * * *"), 1), [5]);
        assert_eq!(values(&parse("0x1f * * * *"), 1), [31]);
        assert_eq!(values(&parse("+5 * * * *"), 1), [5]);
        // Only the first `*` is a range.
        assert!(Cron::parse("*,* * * * *").is_err());
        // Swapped ends, and a step past the end.
        assert_eq!(values(&parse("10-8 * * * *"), 1), [8, 9, 10]);
        assert_eq!(values(&parse("5-50/100 * * * *"), 1), [5]);
        assert_eq!(values(&parse("0-30/10,45 * * * *"), 1), [0, 10, 20, 30, 45]);
    }

    #[test]
    fn reads_month_and_weekday_names_and_the_first_seven() {
        assert_eq!(values(&parse("0 0 1 JAN,mar-May *"), 4), [1, 3, 4, 5]);
        assert_eq!(values(&parse("0 0 * * Mon-Fri"), 5), [1, 2, 3, 4, 5]);
        assert_eq!(values(&parse("0 0 * * sunday"), 5), [0]);
        assert_eq!(values(&parse("0 0 * * 7"), 5), [0]);
        // `1-7` becomes `1-0`, swapped into Sunday and Monday.
        assert_eq!(values(&parse("0 0 * * 1-7"), 5), [0, 1]);
        // A second 7 stays 7, which no day is.
        assert_eq!(values(&parse("0 0 * * 7,7"), 5), [0, 7]);
    }

    #[test]
    fn requires_day_of_month_and_day_of_week_together() {
        // The 13th, only when it is a Friday: 2026-02-13 is one, 2026-01-13 is not.
        let cron = parse("0 9 13 * 5");
        assert!(cron
            .fires_in(minute_of("2026-02-13T09:00:00Z"), &utc())
            .is_some());
        assert!(cron
            .fires_in(minute_of("2026-01-13T09:00:00Z"), &utc())
            .is_none());
    }

    #[test]
    fn fires_at_the_first_matching_second_of_a_minute() {
        let cron = parse("15,45 * * * * *");
        let minute = minute_of("2026-03-01T10:00:00Z");
        assert_eq!(
            cron.fires_in(minute, &utc()),
            Some(minute * 60_000 + 15_000)
        );
        assert_eq!(
            parse("30 10 * * *").fires_in(minute_of("2026-03-01T10:30:00Z"), &utc()),
            Some(minute_of("2026-03-01T10:30:00Z") * 60_000)
        );
        assert_eq!(parse("30 10 * * *").fires_in(minute, &utc()), None);
    }

    #[test]
    fn matches_the_wall_clock_of_its_zone_across_daylight_saving() {
        let ny = TimeZone::get("America/New_York").unwrap();
        let cron = parse("30 2 * * *");
        // 2026-03-08 springs from 02:00 to 03:00: 02:30 never happens.
        let day = minute_of("2026-03-08T05:00:00Z");
        assert!((day..day + 24 * 60).all(|m| cron.fires_in(m, &ny).is_none()));
        // 2026-11-01 falls back from 02:00 to 01:00: 01:30 happens twice.
        let cron = parse("30 1 * * *");
        let day = minute_of("2026-11-01T04:00:00Z");
        let fired: Vec<_> = (day..day + 24 * 60)
            .filter(|m| cron.fires_in(*m, &ny).is_some())
            .collect();
        assert_eq!(fired.len(), 2);
    }

    #[test]
    fn fires_in_a_zone_whose_offset_has_seconds() {
        // Monrovia kept -00:44:30 until 1972.
        let monrovia = TimeZone::get("Africa/Monrovia").unwrap();
        let cron = parse("0 0 12 * * *");
        let minute = minute_of("1970-06-01T12:44:00Z");
        let at = cron.fires_in(minute, &monrovia).unwrap();
        assert_eq!(at % 60_000, 30_000);
        let local = Timestamp::from_millisecond(at).unwrap().to_zoned(monrovia);
        assert_eq!((local.hour(), local.minute(), local.second()), (12, 0, 0));
    }

    #[test]
    fn reads_parse_int_as_javascript_does() {
        assert_eq!(parse_int(" 12abc"), Some(12));
        assert_eq!(parse_int("-3"), Some(-3));
        assert_eq!(parse_int("0X1A"), Some(26));
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int("0x"), None);
        assert_eq!(parse_int(""), None);
    }
}
