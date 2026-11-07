//! Tick generation for the time ruler and the analog value
//! axis.
//!
//! Time ticks are computed with exact integer arithmetic on femtoseconds, so
//! labels never show float noise.

use crate::core::format::thousands;
use crate::core::time::CaptureTime;

/// Minimum spacing between time ticks, in points.
pub const TIME_TICK_MIN_SPACING: f64 = 45.0;
/// Minimum spacing between value ticks, in points.
pub const VALUE_TICK_MIN_SPACING: f64 = 30.0;

/// Smallest tick span: 1 ps (10³ fs). The formatter has no femto prefix.
const MIN_SPAN_EXP: u32 = 3;
/// Largest tick span: 10¹⁸ s, far beyond any capture.
const MAX_SPAN_EXP: u32 = 33;

#[derive(Clone, Debug, PartialEq)]
pub struct TimeTick {
    /// Position in plot-area points.
    pub x: f64,
    /// Origin-relative time (time − T0).
    pub value: CaptureTime,
    pub major: bool,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimeTicks {
    pub minor_span: CaptureTime,
    pub major_span: CaptureTime,
    /// Ticks from 45 pt left of the plot to 45 pt past its right edge.
    pub ticks: Vec<TimeTick>,
    /// When no major tick lies in the left half, the nearest major
    /// to the left, to be drawn at x = 0 with a "◂" arrow.
    pub offscreen_major: Option<TimeTick>,
}

fn pow10(exp: u32) -> i128 {
    10i128.pow(exp)
}

/// Largest multiple-of-3 exponent ≤ `span_exp`, capped at seconds (10¹⁵ fs).
fn unit_exp(span_exp: u32) -> u32 {
    ((span_exp / 3) * 3).min(15)
}

fn unit_name(exp: u32) -> &'static str {
    match exp {
        0 => "fs",
        3 => "ps",
        6 => "ns",
        9 => "µs",
        12 => "ms",
        _ => "s",
    }
}

/// Smallest `k` such that `10^k fs ≥ min_span_fs`.
fn span_exp_for(min_span_fs: f64) -> u32 {
    if min_span_fs.is_nan() || min_span_fs <= 1.0 {
        return MIN_SPAN_EXP;
    }
    // `tpp × 45` may land a few ulps above an exact power of ten.
    let min_span_fs = min_span_fs * (1.0 - 1e-12);
    let mut k = min_span_fs.log10().ceil().max(0.0) as u32;
    // Correct float error in log10 at exact powers of ten.
    while k > 0 && (pow10(k - 1) as f64) >= min_span_fs {
        k -= 1;
    }
    while (pow10(k) as f64) < min_span_fs {
        k += 1;
    }
    k.clamp(MIN_SPAN_EXP, MAX_SPAN_EXP)
}

/// Major label: `3 s: 750 ms`, `12 ms: 500 µs`, `0 s`, `- 1 s`.
///
/// Groups run from the largest non-zero group down to the unit of the major
/// span; trailing zero groups are omitted (`4 s`, not `4 s : 0 ms`).
pub fn major_label(value: CaptureTime, major_span_exp: u32) -> String {
    let v = value.fs();
    if v == 0 {
        return "0 s".to_owned();
    }
    let a = v.unsigned_abs();
    let lowest = unit_exp(major_span_exp);
    let mut groups: Vec<String> = Vec::new();
    let secs = a / 10u128.pow(15);
    if secs > 0 {
        groups.push(format!("{} s", thousands(secs)));
    }
    let mut exp = 12;
    while exp >= lowest && exp < 15 {
        let g = (a / 10u128.pow(exp)) % 1000;
        if g > 0 || !groups.is_empty() {
            groups.push(format!("{g} {}", unit_name(exp)));
        }
        if exp == 0 {
            break;
        }
        exp -= 3;
    }
    while groups.len() > 1 && groups.last().is_some_and(|g| g.starts_with("0 ")) {
        groups.pop();
    }
    let sign = if v < 0 { "- " } else { "" };
    format!("{sign}{}", groups.join(" : "))
}

/// Minor label: signed offset from the major tick nearer to zero,
/// in the unit of the minor span: `+1 ms`, `+500 µs`, `−3 ms`.
pub fn minor_label(value: CaptureTime, minor_span_exp: u32) -> String {
    let major = pow10(minor_span_exp + 1);
    let v = value.fs();
    // Truncating division: the "previous" major is the one nearer zero, so
    // ticks before T0 get negative offsets.
    let base = (v / major) * major;
    let off = v - base;
    let unit = unit_exp(minor_span_exp);
    let n = off.unsigned_abs() / 10u128.pow(unit);
    let sign = if off < 0 { "\u{2212}" } else { "+" };
    format!("{sign}{n} {}", unit_name(unit))
}

/// time-ruler ticks for a view.
///
/// `left` is the absolute time at x = 0, `origin` is T0; positions and labels
/// use the origin-relative time.
pub fn time_ticks(left: CaptureTime, origin: CaptureTime, time_per_px: f64, width_px: f64) -> TimeTicks {
    assert!(time_per_px > 0.0, "time_per_px must be > 0");
    let tpp_fs = time_per_px * crate::core::time::FS_PER_S as f64;
    let k = span_exp_for(tpp_fs * TIME_TICK_MIN_SPACING);
    let minor = pow10(k);
    let major = minor * 10;

    let rel_left = (left - origin).fs();
    let margin = (TIME_TICK_MIN_SPACING * tpp_fs).ceil() as i128;
    let rel_right = rel_left + (width_px * tpp_fs).ceil() as i128;
    let first = (rel_left - margin).div_euclid(minor) * minor;
    let last = rel_right + margin;

    let to_x = |v: i128| (v - rel_left) as f64 / tpp_fs;
    let mut ticks = Vec::new();
    let mut v = first;
    while v <= last {
        let value = CaptureTime::from_fs(v);
        let is_major = v.rem_euclid(major) == 0;
        let label = if is_major { major_label(value, k + 1) } else { minor_label(value, k) };
        ticks.push(TimeTick { x: to_x(v), value, major: is_major, label });
        v += minor;
    }

    let half = width_px / 2.0;
    let has_left_major = ticks.iter().any(|t| t.major && t.x >= 0.0 && t.x <= half);
    let offscreen_major = (!has_left_major).then(|| {
        let v = rel_left.div_euclid(major) * major;
        let value = CaptureTime::from_fs(v);
        TimeTick { x: 0.0, value, major: true, label: major_label(value, k + 1) }
    });

    TimeTicks {
        minor_span: CaptureTime::from_fs(minor),
        major_span: CaptureTime::from_fs(major),
        ticks,
        offscreen_major,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValueTick {
    /// Position from the top of the row's inner area, in points.
    pub y: f64,
    pub value: f64,
    pub major: bool,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValueTicks {
    pub span: f64,
    /// Shared SI exponent of all labels (multiple of 3).
    pub prefix_exp: i32,
    /// Decimals of all labels (0, 1 or 2).
    pub decimals: usize,
    pub ticks: Vec<ValueTick>,
}

/// `m × 10^n` with `m ∈ {1, 2, 5}`.
#[derive(Copy, Clone, Debug)]
struct Span125 {
    m: i64,
    n: i32,
}

impl Span125 {
    fn value(self) -> f64 {
        scale10(self.m as f64, self.n)
    }

    fn for_min(min_span: f64) -> Self {
        let n = min_span.log10().floor() as i32;
        let tol = 1.0 - 1e-12;
        for (m, dn) in [(1, 0), (2, 0), (5, 0), (1, 1)] {
            let s = Self { m, n: n + dn };
            if s.value() >= min_span * tol {
                return s;
            }
        }
        Self { m: 2, n: n + 1 }
    }
}

/// `x × 10^n`, dividing by the exact power of ten for negative `n` so that
/// e.g. `3 × 10⁻¹` is the double closest to 0.3.
fn scale10(x: f64, n: i32) -> f64 {
    if n >= 0 { x * 10f64.powi(n) } else { x / 10f64.powi(-n) }
}

fn si_prefix(exp: i32) -> &'static str {
    match exp {
        -12 => "p",
        -9 => "n",
        -6 => "µ",
        -3 => "m",
        3 => "k",
        6 => "M",
        9 => "G",
        12 => "T",
        _ => "",
    }
}

/// value-axis ticks for an analog row whose inner area spans
/// `bottom_value` (at y = height) to `top_value` (at y = 0).
pub fn value_ticks(bottom_value: f64, top_value: f64, height_px: f64, unit: &str) -> ValueTicks {
    let range = top_value - bottom_value;
    assert!(range > 0.0 && height_px > 0.0, "empty value range");
    let span = Span125::for_min(range / height_px * VALUE_TICK_MIN_SPACING);
    let span_v = span.value();

    let i0 = (bottom_value / span_v).ceil() as i64;
    let i1 = (top_value / span_v).floor() as i64;
    let values: Vec<(i64, f64)> = (i0..=i1).map(|i| (i, scale10((i * span.m) as f64, span.n))).collect();

    let max_abs = values.iter().map(|(_, v)| v.abs()).fold(0.0, f64::max);
    let mut prefix_exp =
        if max_abs > 0.0 { ((max_abs.log10().floor() as i32).div_euclid(3) * 3).clamp(-12, 12) } else { 0 };
    let decimals = loop {
        let scaled = span_v / 10f64.powi(prefix_exp);
        let d = (-(scaled.log10() + 1e-9).floor()).max(0.0) as usize;
        if d <= 2 || prefix_exp <= -12 {
            break d.min(2);
        }
        prefix_exp -= 3;
    };

    let ticks = values
        .into_iter()
        .map(|(i, value)| {
            let major = if span.m == 1 { i % 10 == 0 } else { (i * span.m) % 10 == 0 };
            let scaled = value / 10f64.powi(prefix_exp);
            let mut num = format!("{scaled:.decimals$}");
            if num.starts_with('-') && num[1..].chars().all(|c| c == '0' || c == '.') {
                num.remove(0);
            }
            ValueTick {
                y: (top_value - value) / range * height_px,
                value,
                major,
                label: format!("{num} {}{unit}", si_prefix(prefix_exp)),
            }
        })
        .collect();

    ValueTicks { span: span_v, prefix_exp, decimals, ticks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::time::FS_PER_S;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    fn secs(s: f64) -> CaptureTime {
        CaptureTime::from_secs_f64(s)
    }

    fn ms(n: i128) -> CaptureTime {
        CaptureTime::from_fs(n * FS_PER_S / 1000)
    }

    /// U-01: the reference view from.
    #[test]
    fn u01_reference_view() {
        let t = time_ticks(secs(3.7249), CaptureTime::ZERO, 10e-6, 1500.0);
        assert_eq!(t.minor_span, ms(1));
        assert_eq!(t.major_span, ms(10));

        let major = t.ticks.iter().find(|t| t.major).unwrap();
        assert_eq!(major.value, ms(3730));
        assert_eq!(major.label, "3 s : 730 ms");
        assert!((major.x - 510.0).abs() < 1e-6);

        let after = t.ticks.iter().find(|t| t.value == ms(3731)).unwrap();
        assert_eq!(after.label, "+1 ms");
        assert!(!after.major);

        // Range: 45 pt beyond both edges.
        assert!(t.ticks.first().unwrap().x >= -45.0 - 100.0);
        assert!(t.ticks.first().unwrap().x <= -45.0 + 100.0);
        assert!(t.ticks.last().unwrap().x <= 1500.0 + 45.0);
        assert!(t.offscreen_major.is_none());
    }

    #[test]
    fn major_label_examples() {
        assert_eq!(major_label(ms(3750), 13), "3 s : 750 ms");
        assert_eq!(major_label(ms(1000), 15), "1 s");
        assert_eq!(major_label(ms(1000), 13), "1 s");
        let us = |n: i128| CaptureTime::from_fs(n * 1_000_000_000);
        assert_eq!(major_label(us(12_500), 11), "12 ms : 500 µs");
        assert_eq!(major_label(us(1_000_500), 11), "1 s : 0 ms : 500 µs");
        assert_eq!(major_label(CaptureTime::ZERO, 13), "0 s");
        assert_eq!(major_label(-ms(1270), 13), "- 1 s : 270 ms");
        assert_eq!(major_label(ms(1_234_000), 16), "1,234 s");
    }

    #[test]
    fn minor_label_examples() {
        assert_eq!(minor_label(ms(3731), 12), "+1 ms");
        let us = |n: i128| CaptureTime::from_fs(n * 1_000_000_000);
        assert_eq!(minor_label(us(1_500), 11), "+500 µs");
        assert_eq!(minor_label(ms(1200), 14), "+200 ms");
        // Before T0: offset from the major nearer zero.
        assert_eq!(minor_label(-ms(1271), 12), "\u{2212}1 ms");
    }

    #[test]
    fn negative_origin() {
        // T0 at 5 s: the view starts 1.2751 s before T0.
        let t = time_ticks(secs(3.7249), secs(5.0), 10e-6, 1500.0);
        let m = t.ticks.iter().find(|t| t.value == -ms(1270)).unwrap();
        assert!(m.major);
        assert_eq!(m.label, "- 1 s : 270 ms");
        let n = t.ticks.iter().find(|t| t.value == -ms(1271)).unwrap();
        assert_eq!(n.label, "\u{2212}1 ms");
        // Minor ticks before T0 count from the major on their right.
        let p = t.ticks.iter().find(|t| t.value == -ms(1269)).unwrap();
        assert_eq!(p.label, "\u{2212}9 ms");
        // After T0, from the major on their left.
        let t = time_ticks(secs(4.99), secs(5.0), 10e-6, 1500.0);
        let q = t.ticks.iter().find(|t| t.value == ms(1)).unwrap();
        assert_eq!(q.label, "+1 ms");
    }

    #[test]
    fn offscreen_major_when_left_half_has_none() {
        // tpp 1 µs/px → minor 100 µs, major 1 ms. View 3.2 ms…3.6 ms: no major.
        let t = time_ticks(secs(3.2e-3), CaptureTime::ZERO, 1e-6, 400.0);
        assert_eq!(t.major_span, ms(1));
        let off = t.offscreen_major.unwrap();
        assert_eq!(off.value, ms(3));
        assert_eq!(off.label, "3 ms");
        assert_eq!(off.x, 0.0);
    }

    #[test]
    fn exact_power_of_ten_spacing() {
        // min span exactly 1 ms → minor 1 ms (not 10 ms).
        let t = time_ticks(CaptureTime::ZERO, CaptureTime::ZERO, 1e-3 / 45.0, 1000.0);
        assert_eq!(t.minor_span, ms(1));
    }

    /// U-01: no float noise over 10⁶ random views.
    #[test]
    fn u01_no_float_noise_random_views() {
        let mut rng = StdRng::seed_from_u64(1);
        let ok = |l: &str| {
            l.split(" : ").all(|g| {
                let g = g.trim_start_matches("- ").trim_start_matches(['+', '\u{2212}']);
                let (num, unit) = g.split_once(' ').unwrap();
                num.replace(',', "").chars().all(|c| c.is_ascii_digit())
                    && ["fs", "ps", "ns", "µs", "ms", "s"].contains(&unit)
            })
        };
        for _ in 0..1_000_000 {
            let tpp = 10f64.powf(rng.random_range(-11.0..1.0));
            let left = CaptureTime::from_fs(rng.random_range(-(10i128.pow(19))..10i128.pow(19)));
            let origin = CaptureTime::from_fs(rng.random_range(-(10i128.pow(18))..10i128.pow(18)));
            let t = time_ticks(left, origin, tpp, 200.0);
            let minor = t.minor_span.fs();
            for tick in &t.ticks {
                assert_eq!(tick.value.fs() % minor, 0);
                assert!(ok(&tick.label), "bad label {:?}", tick.label);
            }
            // Spacing ≥ 45 pt and < 450 pt.
            let dx = minor as f64 / (tpp * FS_PER_S as f64);
            assert!((45.0 * (1.0 - 1e-9)..450.0 * (1.0 + 1e-9)).contains(&dx), "spacing {dx}");
        }
    }

    // ---- / U-02 ----

    #[test]
    fn u02_decimals_per_span() {
        let t = value_ticks(-10.0, 10.0, 1200.0, "V");
        assert_eq!(t.span, 0.5);
        assert_eq!((t.prefix_exp, t.decimals), (0, 1));
        assert!(t.ticks.iter().any(|t| t.label == "2.5 V"));
        assert!(t.ticks.iter().any(|t| t.label == "-10.0 V"));

        let t = value_ticks(-1.0, 1.0, 1200.0, "V");
        assert_eq!(t.span, 0.05);
        assert_eq!((t.prefix_exp, t.decimals), (0, 2));
        assert!(t.ticks.iter().any(|t| t.label == "0.35 V"));

        let t = value_ticks(0.0, 0.5, 1500.0, "V");
        assert_eq!(t.span, 0.01);
        assert_eq!((t.prefix_exp, t.decimals), (-3, 0));
        assert!(t.ticks.iter().any(|t| t.label == "500 mV"));
        assert!(t.ticks.iter().any(|t| t.label == "10 mV"));
    }

    #[test]
    fn u02_spans_are_1_2_5() {
        let mut rng = StdRng::seed_from_u64(2);
        for _ in 0..10_000 {
            let lo = rng.random_range(-100.0..100.0);
            let range = 10f64.powf(rng.random_range(-6.0..3.0));
            let h = rng.random_range(40.0..800.0);
            let t = value_ticks(lo, lo + range, h, "V");
            let mant = t.span / 10f64.powf(t.span.log10().floor());
            assert!([1.0, 2.0, 5.0].iter().any(|m| (mant - m).abs() < 1e-9), "span {}", t.span);
            let spacing = t.span / range * h;
            assert!(spacing >= 30.0 * (1.0 - 1e-9), "spacing {spacing}");
            // Adjacent labels differ.
            for w in t.ticks.windows(2) {
                assert_ne!(w[0].label, w[1].label);
            }
        }
    }

    #[test]
    fn value_majors() {
        // span 1 → every 10th tick major.
        let t = value_ticks(-20.0, 20.0, 1200.0, "V");
        assert_eq!(t.span, 1.0);
        for tick in &t.ticks {
            assert_eq!(tick.major, tick.value.rem_euclid(10.0) == 0.0, "{}", tick.value);
        }
        // span 2 → multiples of 10 major.
        let t = value_ticks(-20.0, 20.0, 600.0, "V");
        assert_eq!(t.span, 2.0);
        assert!(t.ticks.iter().filter(|t| t.major).all(|t| t.value % 10.0 == 0.0));
        assert_eq!(t.ticks.iter().filter(|t| t.major).count(), 5);
    }

    #[test]
    fn value_tick_positions() {
        let t = value_ticks(-1.0, 1.0, 200.0, "V");
        let zero = t.ticks.iter().find(|t| t.value == 0.0).unwrap();
        assert!((zero.y - 100.0).abs() < 1e-9);
        assert_eq!(zero.label, "0.0 V");
    }
}
