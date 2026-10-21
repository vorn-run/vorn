//! Feed dates in RFC 3339 or RFC 822 form, as ISO 8601 in UTC, as the
//! connector's `dates.ts` reads them; and the clock arithmetic `Date` does there.

use std::sync::LazyLock;

use regex::Regex;

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

static RFC822: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[A-Za-z]{2,},?\s*)?([0-9]{1,2})[\s-]+([A-Za-z]{3,})\.?[\s-]+([0-9]{4}|[0-9]{2})\s+([0-9]{1,2}):([0-9]{2})(?::([0-9]{2}))?(?:\s*([A-Za-z]+|[+-][0-9]{4}|[+-][0-9]{2}:[0-9]{2}))?")
        .expect("a valid pattern")
});
static RFC3339: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([0-9]{4})-([0-9]{2})-([0-9]{2})(?:[Tt\s]([0-9]{2}):([0-9]{2})(?::([0-9]{2})(?:[.,]([0-9]+))?)?\s*([Zz]|[+-][0-9]{2}:?[0-9]{2})?)?$")
        .expect("a valid pattern")
});
static NUMERIC_ZONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([+-])([0-9]{2}):?([0-9]{2})$").expect("a valid pattern"));

/// RFC 822 §5.1 zone names, in hours from UT.
fn named_zone(upper: &str) -> Option<i64> {
    Some(match upper {
        "UT" | "GMT" | "Z" => 0,
        "EST" | "CDT" => -5,
        "EDT" => -4,
        "CST" => -6,
        "MST" => -7,
        "MDT" => -6,
        "PST" => -8,
        "PDT" => -7,
        _ => return None,
    })
}

/// Minutes east of UT; an unknown or missing zone reads as UT.
fn offset_minutes(zone: Option<&str>) -> i64 {
    let Some(zone) = zone else { return 0 };
    if let Some(m) = NUMERIC_ZONE.captures(zone) {
        let sign = if &m[1] == "-" { -1 } else { 1 };
        return sign * (num(&m[2]) * 60 + num(&m[3]));
    }
    let upper = zone.to_uppercase();
    if let Some(hours) = named_zone(&upper) {
        return hours * 60;
    }
    // RFC 822 §5.2 military zones: A to M (no J) are -1 to -12, N to Y are +1 to +12.
    match upper.as_bytes() {
        [c @ (b'A'..=b'I' | b'K'..=b'M')] => {
            -(i64::from(*c) - 64 - i64::from(*c > b'J')) * 60
        }
        [c @ b'N'..=b'Y'] => (i64::from(*c) - 77) * 60,
        _ => 0,
    }
}

fn num(s: &str) -> i64 {
    s.parse().unwrap_or(0)
}

/// Days from 1970-01-01 to a proleptic Gregorian date, `month` from 1.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Milliseconds since the epoch as `Date.toISOString` writes them.
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    )
}

/// An ISO string this module wrote, back in milliseconds since the epoch.
pub fn iso_ms(value: &str) -> Option<i64> {
    let m = RFC3339.captures(value)?;
    let ms = m.get(7).map_or(0, |f| num(&format!("{:0<3}", &f.as_str()[..f.len().min(3)])));
    Some(
        days_from_civil(num(&m[1]), num(&m[2]), num(&m[3])) * 86_400_000
            + m.get(4).map_or(0, |h| num(h.as_str())) * 3_600_000
            + m.get(5).map_or(0, |v| num(v.as_str())) * 60_000
            + m.get(6).map_or(0, |s| num(s.as_str())) * 1000
            + ms,
    )
}

#[allow(clippy::too_many_arguments)]
fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64, ms: i64, offset: i64) -> String {
    if !(0..=11).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return String::new();
    }
    let (_, m, _) = civil_from_days(days_from_civil(year, month + 1, day));
    if m != month + 1 {
        return String::new();
    }
    let at = days_from_civil(year, month + 1, day) * 86_400_000
        + hour * 3_600_000
        + minute * 60_000
        + second * 1000
        + ms
        - offset * 60_000;
    iso(at)
}

fn from_rfc3339(value: &str) -> Option<String> {
    let m = RFC3339.captures(value)?;
    let ms = m.get(7).map_or(0, |f| num(&format!("{:0<3}", &f.as_str()[..f.len().min(3)])));
    let zone = m.get(8).map(|z| z.as_str()).filter(|z| !z.eq_ignore_ascii_case("z"));
    let part = |i: usize| m.get(i).map_or(0, |v| num(v.as_str()));
    Some(utc(
        num(&m[1]),
        num(&m[2]) - 1,
        num(&m[3]),
        part(4),
        part(5),
        part(6),
        ms,
        offset_minutes(zone),
    ))
}

fn from_rfc822(value: &str) -> Option<String> {
    let m = RFC822.captures(value)?;
    let month_name = m[2][..3].to_lowercase();
    let Some(month) = MONTHS.iter().position(|n| *n == month_name) else {
        return Some(String::new());
    };
    let year = if m[3].len() == 2 {
        let short = num(&m[3]);
        if short < 50 { 2000 + short } else { 1900 + short }
    } else {
        num(&m[3])
    };
    Some(utc(
        year,
        i64::try_from(month).unwrap_or(0),
        num(&m[1]),
        num(&m[4]),
        num(&m[5]),
        m.get(6).map_or(0, |s| num(s.as_str())),
        0,
        offset_minutes(m.get(7).map(|z| z.as_str())),
    ))
}

/// A feed date in RFC 3339 or RFC 822 form as ISO 8601 in UTC, or "" when it is neither.
pub fn parse_date(value: &str) -> String {
    let trimmed = crate::text::js_trim(value);
    if trimmed.is_empty() {
        return String::new();
    }
    from_rfc3339(trimmed)
        .or_else(|| from_rfc822(trimmed))
        .unwrap_or_default()
}

/// Now, in milliseconds since the epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_forms_into_utc() {
        assert_eq!(parse_date("Fri, 21 Jul 2023 09:04 EDT"), "2023-07-21T13:04:00.000Z");
        assert_eq!(parse_date("2000-08-09T09:30-05:00"), "2000-08-09T14:30:00.000Z");
        assert_eq!(parse_date("2026-09-09T16:03:05.12Z"), "2026-09-09T16:03:05.120Z");
        assert_eq!(parse_date("Sat, 12 Sep 26 10:00:00 A"), "2026-09-12T11:00:00.000Z");
        assert_eq!(parse_date("31 Feb 2024 10:00"), "");
        assert_eq!(parse_date("yesterday"), "");
    }

    #[test]
    fn iso_round_trips() {
        let at = "1999-12-31T23:59:59.999Z";
        assert_eq!(iso_ms(at).map(iso).as_deref(), Some(at));
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
    }
}
