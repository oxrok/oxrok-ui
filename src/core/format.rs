//! Shared formatting and parsing rules.

use crate::core::time::{CaptureTime, FS_PER_S, WallClock};
use chrono::{DateTime, Local, Utc};

/// SI prefixes from pico to tera, as (exponent, symbol).
const PREFIXES: [(i32, &str); 9] =
    [(-12, "p"), (-9, "n"), (-6, "µ"), (-3, "m"), (0, ""), (3, "k"), (6, "M"), (9, "G"), (12, "T")];
const MIN_EXP: i32 = -12;
const MAX_EXP: i32 = 12;

fn prefix_symbol(exp: i32) -> &'static str {
    PREFIXES.iter().find(|(e, _)| *e == exp).map(|(_, s)| *s).unwrap_or("")
}

/// Exponent (multiple of 3, clamped p…T) such that `1 ≤ |v| / 10^exp < 1000`.
fn prefix_exp(abs: f64) -> i32 {
    if abs == 0.0 || !abs.is_finite() {
        return 0;
    }
    let e = abs.log10().floor() as i32;
    (e.div_euclid(3) * 3).clamp(MIN_EXP, MAX_EXP)
}

fn pow10(exp: i32) -> f64 {
    10f64.powi(exp)
}

fn round_to(v: f64, decimals: usize) -> f64 {
    let m = pow10(decimals as i32);
    (v * m).round() / m
}

fn strip_zeros(mut s: String) -> String {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

fn join_unit(num: &str, exp: i32, unit: &str) -> String {
    format!("{num} {}{unit}", prefix_symbol(exp))
}

fn zero(unit: &str) -> String {
    format!("0 {unit}")
}

/// Number of integer digits of `|v|` for `1 ≤ |v| < 1000`.
fn int_digits(v: f64) -> usize {
    let a = v.abs();
    if a >= 100.0 {
        3
    } else if a >= 10.0 {
        2
    } else {
        1
    }
}

/// Shared SI formatter: `decimals(scaled)` picks the number of decimals,
/// trailing zeros are stripped, and a rounded value of 1000 steps up one prefix.
fn si_with(value: f64, unit: &str, decimals: impl Fn(f64) -> usize) -> String {
    if !value.is_finite() {
        return format!("{value} {unit}");
    }
    if value == 0.0 {
        return zero(unit);
    }
    let mut exp = prefix_exp(value.abs());
    loop {
        let scaled = value / pow10(exp);
        let d = decimals(scaled);
        let rounded = round_to(scaled, d);
        if rounded == 0.0 {
            return zero(unit);
        }
        if rounded.abs() >= 1000.0 && exp < MAX_EXP {
            exp += 3;
            continue;
        }
        return join_unit(&strip_zeros(format!("{rounded:.d$}")), exp, unit);
    }
}

/// SI value, e.g. `1.5625 MS/s`, `989.44 µs`.
///
/// With `precision = None` the value keeps 9 significant digits (enough to hide
/// float noise without losing information).
pub fn si(value: f64, unit: &str, precision: Option<usize>) -> String {
    si_with(value, unit, |scaled| precision.unwrap_or_else(|| 9 - int_digits(scaled)))
}

/// Helper: SI with at most `sig` significant digits, e.g. `2.55648 ms`.
pub fn si_sig(value: f64, unit: &str, sig: usize) -> String {
    si_with(value, unit, |scaled| sig.saturating_sub(int_digits(scaled)))
}

/// SI value whose last digit represents `resolution` (rounded to a
/// power of ten). Decimals are not stripped: the digit count shows the
/// resolution.
pub fn si_res(value: f64, unit: &str, resolution: f64) -> String {
    assert!(resolution > 0.0, "resolution must be > 0");
    let r_exp = resolution.log10().round() as i32;
    let snapped = (value / pow10(r_exp)).round() * pow10(r_exp);
    if snapped == 0.0 || !snapped.is_finite() {
        return zero(unit);
    }
    let mut exp = prefix_exp(snapped.abs());
    loop {
        let d = (exp - r_exp).max(0) as usize;
        let rounded = round_to(snapped / pow10(exp), d);
        if rounded.abs() >= 1000.0 && exp < MAX_EXP {
            exp += 3;
            continue;
        }
        return join_unit(&format!("{rounded:.d$}"), exp, unit);
    }
}

/// Integer with US thousands separators, e.g. `1,234,567`.
pub fn thousands(n: u128) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Groups a run of digits in threes separated by spaces: `744850851` → `744 850 851`.
fn group3(digits: &str) -> String {
    digits.as_bytes().chunks(3).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(" ")
}

/// Rounds femtoseconds to the nearest nanosecond (half away from zero).
fn fs_to_ns(fs: i128) -> i128 {
    let q = fs / 1_000_000;
    let r = fs % 1_000_000;
    if r.abs() >= 500_000 { q + fs.signum() } else { q }
}

/// full-precision time with 1 ns resolution.
///
/// `850.851 µs`, `3.744 850 851 s`, `1,234.000 000 001 s`.
pub fn time_full(t: CaptureTime) -> String {
    let ns = fs_to_ns(t.fs());
    let neg = ns < 0;
    let a = ns.unsigned_abs();
    let sign = if neg { "-" } else { "" };
    if a == 0 {
        return zero("s");
    }
    if a < 1_000_000_000 {
        let (div, decimals, prefix) = if a < 1_000 {
            (1u128, 0usize, "n")
        } else if a < 1_000_000 {
            (1_000, 3, "µ")
        } else {
            (1_000_000, 6, "m")
        };
        let whole = a / div;
        if decimals == 0 {
            return format!("{sign}{whole} {prefix}s");
        }
        let frac = a % div;
        return format!("{sign}{whole}.{frac:0decimals$} {prefix}s");
    }
    let secs = a / 1_000_000_000;
    let frac = a % 1_000_000_000;
    format!("{sign}{}.{} s", thousands(secs), group3(&format!("{frac:09}")))
}

/// For an `f64` number of seconds.
pub fn time_full_secs(t: f64) -> String {
    time_full(CaptureTime::from_secs_f64(t))
}

/// Short time with up to 6 significant digits, e.g. `4.8312 ms`.
pub fn time_short(seconds: f64) -> String {
    si_sig(seconds, "s", 6)
}

/// Sample rate, e.g. `24 MS/s`.
pub fn sample_rate(rate_hz: f64) -> String {
    si(rate_hz, "S/s", None)
}

/// Count with thousands separators.
pub fn count(n: u64) -> String {
    thousands(n as u128)
}

/// Capped count label used where specified.
pub fn count_capped(n: u64) -> String {
    if n > 99 { "99+".to_owned() } else { n.to_string() }
}

/// Options.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WallClockFormat {
    pub iso: bool,
    pub utc: bool,
}

/// wall-clock time of `start + offset`.
///
/// Default: `Mar 05, 2026 02:03:22.125 431 PM` (local, 12 h). ISO:
/// `2026-03-05 14:03:22.125431` (24 h). Zero sub-millisecond groups are dropped.
pub fn wall_clock(start: &WallClock, offset: CaptureTime, fmt: WallClockFormat) -> String {
    let ns = start.unix_ns_at(offset);
    let secs = ns.div_euclid(1_000_000_000) as i64;
    let sub = ns.rem_euclid(1_000_000_000) as u32;
    let utc: DateTime<Utc> = DateTime::from_timestamp(secs, sub).unwrap_or_default();

    let digits = format!("{sub:09}");
    let mut groups: Vec<&str> = vec![&digits[0..3], &digits[3..6], &digits[6..9]];
    while groups.len() > 1 && groups.last() == Some(&"000") {
        groups.pop();
    }
    let (date_fmt, frac, suffix_fmt) = if fmt.iso {
        ("%Y-%m-%d %H:%M:%S", groups.concat(), "")
    } else {
        ("%b %d, %Y %I:%M:%S", groups.join(" "), " %p")
    };
    let render = |date: String, suffix: String| format!("{date}.{frac}{suffix}");
    if fmt.utc {
        render(utc.format(date_fmt).to_string(), utc.format(suffix_fmt).to_string())
    } else {
        let local = utc.with_timezone(&Local);
        render(local.format(date_fmt).to_string(), local.format(suffix_fmt).to_string())
    }
}

/// Time units accepted by [`parse_time`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TimeUnit {
    Ps,
    Ns,
    Us,
    Ms,
    S,
    Min,
    H,
}

impl TimeUnit {
    /// Converts `value` in this unit to seconds. Sub-second units divide by
    /// an exact power of ten, so `7 ns` is the double closest to 7e-9.
    pub fn to_seconds(self, value: f64) -> f64 {
        match self {
            TimeUnit::Ps => value / 1e12,
            TimeUnit::Ns => value / 1e9,
            TimeUnit::Us => value / 1e6,
            TimeUnit::Ms => value / 1e3,
            TimeUnit::S => value,
            TimeUnit::Min => value * 60.0,
            TimeUnit::H => value * 3600.0,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        let lower = s.to_lowercase();
        Some(match lower.as_str() {
            "ps" => TimeUnit::Ps,
            "ns" => TimeUnit::Ns,
            // U+00B5 MICRO SIGN and U+03BC GREEK SMALL LETTER MU look identical.
            "us" | "µs" | "μs" => TimeUnit::Us,
            "ms" => TimeUnit::Ms,
            "s" => TimeUnit::S,
            "m" => TimeUnit::Min,
            "h" => TimeUnit::H,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeParseError {
    #[error("empty input")]
    Empty,
    #[error("invalid number")]
    InvalidNumber,
    #[error("unknown unit `{0}`")]
    UnknownUnit(String),
}

/// Splits `input` into its numeric prefix (sign, digits, fraction, exponent)
/// and the rest. Returns `None` if there is no valid number.
fn split_number(input: &str) -> Option<(&str, &str)> {
    let b = input.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut mantissa_digits = i - int_start;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        mantissa_digits += i - frac_start;
    }
    if mantissa_digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == exp_start {
            return None;
        }
        i = j;
    }
    Some(input.split_at(i))
}

/// Parses a time such as `1.5ms`, `250 µs`, `-3e-6` or `2h` into seconds.
pub fn parse_time(input: &str, default_unit: TimeUnit) -> Result<f64, TimeParseError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(TimeParseError::Empty);
    }
    let (num, rest) = split_number(s).ok_or(TimeParseError::InvalidNumber)?;
    let value: f64 = num.parse().map_err(|_| TimeParseError::InvalidNumber)?;
    let rest = rest.trim_start();
    let unit = if rest.is_empty() {
        default_unit
    } else {
        TimeUnit::parse(rest).ok_or_else(|| TimeParseError::UnknownUnit(rest.to_owned()))?
    };
    let secs = unit.to_seconds(value);
    if secs.is_finite() { Ok(secs) } else { Err(TimeParseError::InvalidNumber) }
}

/// With 1 ns resolution, as an exact [`CaptureTime`].
pub fn parse_capture_time(input: &str, default_unit: TimeUnit) -> Result<CaptureTime, TimeParseError> {
    let secs = parse_time(input, default_unit)?;
    let ns = (secs * 1e9).round() as i128;
    Ok(CaptureTime::from_fs(ns * (FS_PER_S / 1_000_000_000)))
}

/// Recent list: `Oct 19`, with the year when it isn't this
/// year (`Oct 19, 2025`), in local time.
pub fn short_date(t: std::time::SystemTime) -> String {
    use chrono::Datelike;
    let d: chrono::DateTime<chrono::Local> = t.into();
    if d.year() == chrono::Local::now().year() {
        d.format("%b %-d").to_string()
    } else {
        d.format("%b %-d, %Y").to_string()
    }
}

/// ISO-8601 export timestamps: UTC with nanoseconds, e.g.
/// `2026-10-20T14:03:22.123456789Z`.
pub fn iso8601_utc(start: &WallClock, offset: CaptureTime) -> String {
    let ns = start.unix_ns_at(offset);
    let secs = ns.div_euclid(1_000_000_000) as i64;
    let sub = ns.rem_euclid(1_000_000_000) as u32;
    let utc: DateTime<Utc> = DateTime::from_timestamp(secs, sub).unwrap_or_default();
    format!("{}.{sub:09}Z", utc.format("%Y-%m-%dT%H:%M:%S"))
}

/// Default export file names: local `YYYY-MM-DD HH-MM`.
pub fn file_stamp(t: std::time::SystemTime) -> String {
    let d: chrono::DateTime<chrono::Local> = t.into();
    d.format("%Y-%m-%d %H-%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn si_examples() {
        assert_eq!(si(1_562_500.0, "S/s", None), "1.5625 MS/s");
        assert_eq!(si(989.44e-6, "s", None), "989.44 µs");
        assert_eq!(si(110.765, "Hz", None), "110.765 Hz");
        assert_eq!(si(0.0, "V", None), "0 V");
        assert_eq!(si(-2.5e-3, "V", Some(1)), "-2.5 mV");
        assert_eq!(sample_rate(24e6), "24 MS/s");
    }

    #[test]
    fn si_rounding_steps_up_prefix() {
        assert_eq!(si(999.96, "Hz", Some(1)), "1 kHz");
        assert_eq!(si(0.999_999_9, "s", Some(2)), "1 s");
        assert_eq!(si(0.99999, "s", Some(2)), "999.99 ms");
    }

    #[test]
    fn si_clamps_prefix_range() {
        assert_eq!(si(1e-15, "s", None), "0.001 ps");
        assert_eq!(si(5e15, "Hz", None), "5000 THz");
    }

    #[test]
    fn si_res_examples() {
        assert_eq!(si_res(850.851e-6, "s", 1e-9), "850.851 µs");
        assert_eq!(si_res(1e-3, "s", 1e-9), "1.000000 ms");
        assert_eq!(si_res(2.0, "V", 0.01), "2.00 V");
        // Rounds up to 1000 → next prefix.
        assert_eq!(si_res(999.9996e-6, "s", 1e-6), "1.000 ms");
        // Rounded to 0 → no prefix.
        assert_eq!(si_res(1e-10, "s", 1e-9), "0 s");
        // Resolution is rounded to a power of ten.
        assert_eq!(si_res(1.234, "V", 0.03), "1.23 V");
    }

    #[test]
    fn time_full_examples() {
        let ns = |n: i128| CaptureTime::from_fs(n * 1_000_000);
        assert_eq!(time_full(ns(850_851)), "850.851 µs");
        assert_eq!(time_full(ns(3_744_850_851)), "3.744 850 851 s");
        assert_eq!(time_full(ns(1_234_000_000_001)), "1,234.000 000 001 s");
        assert_eq!(time_full(ns(-12)), "-12 ns");
        assert_eq!(time_full(ns(1_500_000)), "1.500000 ms");
        assert_eq!(time_full(CaptureTime::ZERO), "0 s");
        // Sub-ns rounds to the nearest ns.
        assert_eq!(time_full(CaptureTime::from_fs(999_999_999_600_000)), "1.000 000 000 s");
    }

    #[test]
    fn time_short_examples() {
        assert_eq!(time_short(2.556_481e-3), "2.55648 ms");
        assert_eq!(time_short(4.8312e-3), "4.8312 ms");
        assert_eq!(time_short(1.0), "1 s");
    }

    #[test]
    fn wall_clock_formats() {
        // 2026-03-05 14:03:22.125 UTC
        let start = WallClock { unix_ms: 1_772_719_402_125, frac_ms: 0.431 };
        let utc = WallClockFormat { iso: false, utc: true };
        assert_eq!(wall_clock(&start, CaptureTime::ZERO, utc), "Mar 05, 2026 02:03:22.125 431 PM");
        let iso = WallClockFormat { iso: true, utc: true };
        assert_eq!(wall_clock(&start, CaptureTime::ZERO, iso), "2026-03-05 14:03:22.125431");
        // Whole milliseconds keep the millisecond group only.
        let whole = WallClock { unix_ms: 1_772_719_402_125, frac_ms: 0.0 };
        assert_eq!(wall_clock(&whole, CaptureTime::ZERO, iso), "2026-03-05 14:03:22.125");
        // Offset of 1 ns shows all three groups.
        let off = CaptureTime::from_fs(1_000_000);
        assert_eq!(wall_clock(&whole, off, iso), "2026-03-05 14:03:22.125000001");
    }

    #[test]
    fn counts() {
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(count(99), "99");
        assert_eq!(count_capped(100), "99+");
        assert_eq!(count_capped(42), "42");
    }

    #[test]
    fn parse_examples() {
        let p = |s| parse_time(s, TimeUnit::S);
        assert_eq!(p("1.5ms"), Ok(0.0015));
        assert!((p("250 µs").unwrap() - 250e-6).abs() < 1e-18);
        assert!((p("250 μs").unwrap() - 250e-6).abs() < 1e-18);
        assert_eq!(p("-3e-6"), Ok(-3e-6));
        assert_eq!(p("2h"), Ok(7200.0));
        assert_eq!(p("1m"), Ok(60.0));
        assert_eq!(p("10 MS"), Ok(0.01));
        assert_eq!(p("+.5"), Ok(0.5));
        assert_eq!(p("  7 ns "), Ok(7e-9));
        assert_eq!(parse_time("3", TimeUnit::Ms), Ok(0.003));
    }

    #[test]
    fn parse_rejects() {
        let p = |s| parse_time(s, TimeUnit::S);
        assert_eq!(p(""), Err(TimeParseError::Empty));
        assert_eq!(p("abc"), Err(TimeParseError::InvalidNumber));
        assert_eq!(p("."), Err(TimeParseError::InvalidNumber));
        assert_eq!(p("1e"), Err(TimeParseError::InvalidNumber));
        assert_eq!(p("- 1"), Err(TimeParseError::InvalidNumber));
        assert_eq!(p("1 ks"), Err(TimeParseError::UnknownUnit("ks".into())));
        assert_eq!(p("1 s s"), Err(TimeParseError::UnknownUnit("s s".into())));
        assert_eq!(p("inf"), Err(TimeParseError::InvalidNumber));
        assert_eq!(p("1e400"), Err(TimeParseError::InvalidNumber));
    }

    /// U-03: formatting and parsing round-trip.
    #[test]
    fn u03_round_trips() {
        // (No values ≥ 1000 s: `k` is not a unit.)
        for &v in &[1.5e-3, 250e-6, -3e-6, 720.0, 1e-9, 42.125, 0.0, -0.75] {
            let text = si(v, "s", None);
            let back = parse_time(&text, TimeUnit::S).unwrap();
            assert!((back - v).abs() <= v.abs() * 1e-12, "{v} → {text} → {back}");
        }
        for ns in [1i128, 999, 1_000, 850_851, 3_744_850_851, -5_000_000_007] {
            let t = CaptureTime::from_fs(ns * 1_000_000);
            // `time_full` output minus digit grouping is valid input.
            let text = time_full(t).replace([' ', ','], "");
            let back = parse_capture_time(&text, TimeUnit::S).unwrap();
            assert_eq!(back, t, "{ns} ns → {text}");
        }
    }
}
