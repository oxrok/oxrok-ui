//! Measurement engine: hover readouts and range metrics.

use crate::store::{AnalogChannel, DigitalChannel};

/// The pulse under a sample.
///
/// `e0 ≤ s < e1` are the transitions around `s`; `e2` is the next transition
/// of the same polarity as `e0` (i.e. the one after `e1`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Pulse {
    pub e0: u64,
    pub e1: Option<u64>,
    pub e2: Option<u64>,
    /// Level between `e0` and `e1`.
    pub level: bool,
}

impl Pulse {
    pub fn width(&self) -> Option<u64> {
        self.e1.map(|e1| e1 - self.e0)
    }

    pub fn period(&self) -> Option<u64> {
        self.e2.map(|e2| e2 - self.e0)
    }

    /// Width / period × 100.
    pub fn duty_percent(&self) -> Option<f64> {
        Some(self.width()? as f64 / self.period()? as f64 * 100.0)
    }
}

/// `measure_digital`: O(log n) with the transition index.
pub fn measure_digital(ch: &DigitalChannel, sample: u64) -> Option<Pulse> {
    let level = ch.level_at(sample)?;
    let e0 = ch.prev_transition(sample + 1)?;
    let e1 = ch.next_transition(sample);
    let e2 = e1.and_then(|e| ch.next_transition(e));
    Some(Pulse { e0, e1, e2, level })
}

/// `measure_analog`: the interpolated value at a fractional sample.
pub fn measure_analog(ch: &AnalogChannel, x: f64) -> Option<f32> {
    ch.value_at(x)
}

/// Analog metrics over `[s0, s1)`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AnalogMetrics {
    pub count: u64,
    pub min: f64,
    pub max: f64,
    pub pp: f64,
    pub mean: f64,
    pub rms: f64,
    /// Time of the min/max, seconds relative to `s0`.
    pub min_t: Option<f64>,
    pub max_t: Option<f64>,
}

/// For analog channels; streams the samples (min/max come from the pyramid).
pub fn analog_metrics(ch: &AnalogChannel, s0: u64, s1: u64, rate: u64) -> Option<AnalogMetrics> {
    let (min, max) = ch.min_max(s0, s1)?;
    let (mut n, mut sum, mut sq) = (0u64, 0f64, 0f64);
    let (mut min_v, mut max_v, mut min_i, mut max_i) = (f64::INFINITY, f64::NEG_INFINITY, 0u64, 0u64);
    for (i, v) in ch.samples(s0, s1) {
        let v = v as f64;
        n += 1;
        sum += v;
        sq += v * v;
        if v < min_v {
            min_v = v;
            min_i = i;
        }
        if v > max_v {
            max_v = v;
            max_i = i;
        }
    }
    let (min, max) = (min as f64, max as f64);
    let t = |i: u64| (n > 0 && rate > 0).then(|| (i.saturating_sub(s0)) as f64 / rate as f64);
    Some(AnalogMetrics {
        count: n,
        min,
        max,
        pp: max - min,
        mean: sum / n as f64,
        rms: (sq / n as f64).sqrt(),
        min_t: t(min_i),
        max_t: t(max_i),
    })
}

/// Digital metrics over `[s0, s1)`; `None` fields are N/A.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct DigitalMetrics {
    pub n_rising: u64,
    pub n_falling: u64,
    /// Complete periods / time from the first to the last same-polarity edge.
    pub f_mean: Option<f64>,
    pub f_min: Option<f64>,
    pub f_max: Option<f64>,
    /// Population standard deviation of the periods, seconds.
    pub t_std: Option<f64>,
    pub pw_pos: Option<f64>,
    pub pw_neg: Option<f64>,
    /// High time / ΔT, percent.
    pub duty: Option<f64>,
    /// Extreme pulse widths, seconds.
    pub pw_pos_min: Option<f64>,
    pub pw_pos_max: Option<f64>,
    pub pw_neg_min: Option<f64>,
    pub pw_neg_max: Option<f64>,
    /// RMS of (period − mean) / mean × 100, percent.
    pub f_jitter: Option<f64>,
}

/// For digital channels. Edges in `[s0, s1)` count; periods use the
/// rising edges, or the falling ones when there are fewer than two rising.
pub fn digital_metrics(ch: &DigitalChannel, s0: u64, s1: u64, rate: u64) -> Option<DigitalMetrics> {
    let (b, e) = ch.data_range()?;
    let (s0, s1) = (s0.max(b), s1.min(e));
    if s1 <= s0 || rate == 0 {
        return None;
    }
    let secs = |n: u64| n as f64 / rate as f64;
    let start_level = ch.level_at(s0)?;
    // Edges after s0 alternate starting from the level at s0; an edge at
    // s0 itself is the one that produced that level.
    let mut edges: Vec<(u64, bool)> = Vec::new();
    if ch.prev_transition(s0 + 1) == Some(s0) {
        edges.push((s0, start_level));
    }
    let mut level = start_level;
    let mut high = 0u64;
    let mut last = s0;
    for t in ch.transitions_from(s0 + 1) {
        if t >= s1 {
            break;
        }
        if level {
            high += t - last;
        }
        level = !level;
        last = t;
        edges.push((t, level));
    }
    if level {
        high += s1 - last;
    }
    let rising: Vec<u64> = edges.iter().filter(|e| e.1).map(|e| e.0).collect();
    let falling: Vec<u64> = edges.iter().filter(|e| !e.1).map(|e| e.0).collect();
    let marks = if rising.len() >= 2 { &rising } else { &falling };
    let periods: Vec<u64> = marks.windows(2).map(|w| w[1] - w[0]).collect();
    let mean = |v: &[u64]| (!v.is_empty()).then(|| secs(v.iter().sum::<u64>()) / v.len() as f64);
    let widths = |from: bool| -> Vec<u64> {
        edges.windows(2).filter(|w| w[0].1 == from && w[1].1 != from).map(|w| w[1].0 - w[0].0).collect()
    };
    let (f_mean, f_min, f_max, t_std, f_jitter) = if periods.is_empty() {
        (None, None, None, None, None)
    } else {
        let span = secs(marks[marks.len() - 1] - marks[0]);
        let m = secs(periods.iter().sum::<u64>()) / periods.len() as f64;
        let var = periods.iter().map(|&p| (secs(p) - m).powi(2)).sum::<f64>() / periods.len() as f64;
        let t_std = var.sqrt();
        let f_jitter = (periods.len() >= 2 && m > 0.0).then(|| t_std / m * 100.0);
        (
            Some(periods.len() as f64 / span),
            Some(1.0 / secs(*periods.iter().max().unwrap())),
            Some(1.0 / secs(*periods.iter().min().unwrap())),
            Some(t_std),
            f_jitter,
        )
    };
    let pos = widths(true);
    let neg = widths(false);
    let minmax = |v: &[u64]| -> (Option<f64>, Option<f64>) {
        if v.is_empty() {
            (None, None)
        } else {
            (Some(secs(*v.iter().min().unwrap())), Some(secs(*v.iter().max().unwrap())))
        }
    };
    let (pw_pos_min, pw_pos_max) = minmax(&pos);
    let (pw_neg_min, pw_neg_max) = minmax(&neg);
    Some(DigitalMetrics {
        n_rising: rising.len() as u64,
        n_falling: falling.len() as u64,
        f_mean,
        f_min,
        f_max,
        t_std,
        pw_pos: mean(&pos),
        pw_neg: mean(&neg),
        duty: Some(high as f64 / (s1 - s0) as f64 * 100.0),
        pw_pos_min,
        pw_pos_max,
        pw_neg_min,
        pw_neg_max,
        f_jitter,
    })
}

/// One metric's value (`None` = N/A) for a range of `[s0, s1)`.
pub fn metric_value(
    m: crate::core::ranges::Metric,
    dt: f64,
    digital: Option<&DigitalMetrics>,
    analog: Option<&AnalogMetrics>,
) -> Option<f64> {
    use crate::core::ranges::Metric::*;
    match m {
        Dt => Some(dt),
        NRising => digital.map(|d| d.n_rising as f64),
        NFalling => digital.map(|d| d.n_falling as f64),
        FMean => digital?.f_mean,
        FMin => digital?.f_min,
        FMax => digital?.f_max,
        TStd => digital?.t_std,
        PwPos => digital?.pw_pos,
        PwNeg => digital?.pw_neg,
        Duty => digital?.duty,
        PwPosMin => digital?.pw_pos_min,
        PwPosMax => digital?.pw_pos_max,
        PwNegMin => digital?.pw_neg_min,
        PwNegMax => digital?.pw_neg_max,
        FJitter => digital?.f_jitter,
        VMin => analog.map(|a| a.min),
        VMax => analog.map(|a| a.max),
        Vpp => analog.map(|a| a.pp),
        VMean => analog.map(|a| a.mean),
        VRms => analog.map(|a| a.rms),
        VMinT => analog.and_then(|a| a.min_t),
        VMaxT => analog.and_then(|a| a.max_t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> DigitalChannel {
        // High 0..30, low 30..100, high 100..130, low 130..200, …
        let mut ch = DigitalChannel::new();
        ch.start(0, true);
        for k in 0..10u64 {
            ch.push_transition(k * 100 + 30);
            ch.push_transition(k * 100 + 100);
        }
        ch.extend_to(1001);
        ch
    }

    #[test]
    fn pulse_under_pointer() {
        let ch = clock();
        let p = measure_digital(&ch, 115).unwrap();
        assert_eq!(p, Pulse { e0: 100, e1: Some(130), e2: Some(200), level: true });
        assert_eq!(p.width(), Some(30));
        assert_eq!(p.period(), Some(100));
        assert!((p.duty_percent().unwrap() - 30.0).abs() < 1e-12);
        // On the transition itself, the new pulse starts.
        assert_eq!(measure_digital(&ch, 130).unwrap().e0, 130);
        // Before the first edge there is no e0.
        assert_eq!(measure_digital(&ch, 10), None);
    }

    /// U-10 (digital part): the clock fixture at 1 kS/s. High 30 of every
    /// 100 samples: falling edges at 30, 130, …, 930; rising at 100, …, 1000.
    #[test]
    fn u10_digital_metrics() {
        let ch = clock();
        let m = digital_metrics(&ch, 0, 1001, 1000).unwrap();
        assert_eq!((m.n_rising, m.n_falling), (10, 10));
        // Nine 100-sample periods between rising edges (0.1 s each).
        assert!((m.f_mean.unwrap() - 10.0).abs() < 1e-9);
        assert!((m.f_min.unwrap() - 10.0).abs() < 1e-9 && (m.f_max.unwrap() - 10.0).abs() < 1e-9);
        assert!(m.t_std.unwrap().abs() < 1e-12);
        assert!((m.pw_pos.unwrap() - 0.030).abs() < 1e-12);
        assert!((m.pw_neg.unwrap() - 0.070).abs() < 1e-12);
        // High: 0..30, nine 30-sample pulses, and 1000..1001 = 301 of 1001.
        assert!((m.duty.unwrap() - 301.0 / 1001.0 * 100.0).abs() < 1e-9);
        // Uniform pulses → min == max, and no jitter.
        assert!((m.pw_pos_min.unwrap() - 0.030).abs() < 1e-12 && (m.pw_pos_max.unwrap() - 0.030).abs() < 1e-12);
        assert!((m.pw_neg_min.unwrap() - 0.070).abs() < 1e-12 && (m.pw_neg_max.unwrap() - 0.070).abs() < 1e-12);
        assert!((m.f_jitter.unwrap() - 0.0).abs() < 1e-9);

        // A window with one edge: counts, but no period or complete pulse.
        let m = digital_metrics(&ch, 110, 160, 1000).unwrap();
        assert_eq!((m.n_rising, m.n_falling), (0, 1));
        assert_eq!((m.f_mean, m.pw_pos, m.pw_neg), (None, None, None));
        assert!((m.duty.unwrap() - 40.0).abs() < 1e-9, "130 − 110 of 50");
        assert_eq!(
            (m.pw_pos_min, m.pw_pos_max, m.pw_neg_min, m.pw_neg_max, m.f_jitter),
            (None, None, None, None, None)
        );

        // Uneven periods: f_min/f_max/T_std.
        let mut ch = DigitalChannel::new();
        ch.start(0, false);
        for t in [10, 20, 30, 50, 60, 90] {
            ch.push_transition(t);
        }
        ch.extend_to(100);
        let m = digital_metrics(&ch, 0, 100, 1).unwrap();
        // Rising at 10, 30, 60: periods 20 and 30.
        assert!((m.f_min.unwrap() - 1.0 / 30.0).abs() < 1e-12);
        assert!((m.f_max.unwrap() - 1.0 / 20.0).abs() < 1e-12);
        assert!((m.t_std.unwrap() - 5.0).abs() < 1e-12);
        assert!((m.f_mean.unwrap() - 2.0 / 50.0).abs() < 1e-12);
        // Periods 20 and 30 → jitter 20 %; positive pulses 10, 20, 30.
        assert!((m.f_jitter.unwrap() - 20.0).abs() < 1e-12);
        assert!((m.pw_pos_min.unwrap() - 10.0).abs() < 1e-12 && (m.pw_pos_max.unwrap() - 30.0).abs() < 1e-12);
        assert!((m.pw_neg_min.unwrap() - 10.0).abs() < 1e-12 && (m.pw_neg_max.unwrap() - 10.0).abs() < 1e-12);
        // An edge exactly at s0 counts.
        let m = digital_metrics(&ch, 30, 40, 1).unwrap();
        assert_eq!(m.n_rising, 1);
    }

    /// U-10 (analog part): metrics against hand-computed values.
    #[test]
    fn u10_analog_metrics() {
        let mut ch = AnalogChannel::new();
        ch.push(0, &[1.0, -1.0, 3.0, -3.0, 0.0]).unwrap();
        let m = analog_metrics(&ch, 0, 5, 1000).unwrap();
        assert_eq!(m.count, 5);
        assert_eq!((m.min, m.max, m.pp), (-3.0, 3.0, 6.0));
        assert_eq!(m.mean, 0.0);
        assert!((m.rms - (20.0f64 / 5.0).sqrt()).abs() < 1e-12);
        // Min at index 3, max at index 2, relative to s0.
        assert_eq!((m.min_t, m.max_t), (Some(0.003), Some(0.002)));
        // Sub-range [1, 3): −1, 3.
        let m = analog_metrics(&ch, 1, 3, 1000).unwrap();
        assert_eq!((m.min, m.max, m.mean), (-1.0, 3.0, 1.0));
        assert!((m.rms - 5f64.sqrt()).abs() < 1e-12);
        assert_eq!((m.min_t, m.max_t), (Some(0.0), Some(0.001)));
        assert_eq!(analog_metrics(&ch, 7, 9, 1000), None);
        assert_eq!(measure_analog(&ch, 1.5), Some(1.0));
    }
}
