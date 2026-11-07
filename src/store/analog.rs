//! Analog channel storage: f32 samples plus a min/max pyramid
//! with a 4× reduction per level, for O(log n) min/max of any sample range.

use std::collections::VecDeque;

/// Pyramid depth: level `k` (0-based) holds blocks of `4^(k+1)` samples.
pub const LEVELS: usize = 10;
/// Trims drop data in multiples of the largest block so every level stays aligned.
const TRIM_ALIGN: u64 = 1 << (2 * LEVELS as u32);

#[derive(Clone, Debug, Default)]
pub struct AnalogChannel {
    started: bool,
    /// Sample index of `samples[0]`.
    base: u64,
    /// First visible sample (≥ `base`; trims hide a partial head block).
    data_begin: u64,
    samples: VecDeque<f32>,
    levels: Vec<VecDeque<(f32, f32)>>,
}

fn merge(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0.min(b.0), a.1.max(b.1))
}

impl AnalogChannel {
    pub fn new() -> Self {
        Self { levels: vec![VecDeque::new(); LEVELS], ..Self::default() }
    }

    /// One past the last stored sample.
    pub fn end(&self) -> u64 {
        self.base + self.samples.len() as u64
    }

    /// Stored interval `[begin, end)`, or `None` before the first sample.
    pub fn data_range(&self) -> Option<(u64, u64)> {
        (self.started && self.end() > self.data_begin).then(|| (self.data_begin, self.end()))
    }

    /// Appends samples starting at `first_sample`, which must continue the data.
    pub fn push(&mut self, first_sample: u64, values: &[f32]) -> Result<(), (u64, u64)> {
        if self.levels.is_empty() {
            self.levels = vec![VecDeque::new(); LEVELS];
        }
        if !self.started {
            self.started = true;
            self.base = first_sample;
            self.data_begin = first_sample;
        } else if first_sample != self.end() {
            return Err((self.end(), first_sample));
        }
        for &v in values {
            self.samples.push_back(v);
            if self.samples.len().is_multiple_of(4) {
                let n = self.samples.len();
                let block =
                    self.samples.range(n - 4..).fold((f32::INFINITY, f32::NEG_INFINITY), |m, &x| merge(m, (x, x)));
                self.levels[0].push_back(block);
                for k in 0..LEVELS - 1 {
                    if !self.levels[k].len().is_multiple_of(4) {
                        break;
                    }
                    let n = self.levels[k].len();
                    let block =
                        self.levels[k].range(n - 4..).fold((f32::INFINITY, f32::NEG_INFINITY), |m, &x| merge(m, x));
                    self.levels[k + 1].push_back(block);
                }
            }
        }
        Ok(())
    }

    /// Sample `s`, if stored.
    pub fn get(&self, s: u64) -> Option<f32> {
        (s >= self.data_begin && s < self.end()).then(|| self.samples[(s - self.base) as usize])
    }

    /// Linear interpolation at fractional sample position `x`.
    pub fn value_at(&self, x: f64) -> Option<f32> {
        let s0 = x.floor();
        if s0 < 0.0 {
            return None;
        }
        let a = self.get(s0 as u64)?;
        let frac = (x - s0) as f32;
        Some(match self.get(s0 as u64 + 1) {
            Some(b) if frac > 0.0 => a + (b - a) * frac,
            _ => a,
        })
    }

    /// Min and max over samples `[a, b)`, clamped to the stored data.
    pub fn min_max(&self, a: u64, b: u64) -> Option<(f32, f32)> {
        let (begin, end) = self.data_range()?;
        let (a, b) = (a.max(begin), b.min(end));
        if b <= a {
            return None;
        }
        let mut acc = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut lo, mut hi) = ((a - self.base) as usize, (b - self.base) as usize);
        // Level −1 is the raw samples; level k has blocks of 4^(k+1).
        let mut level: isize = -1;
        while lo < hi {
            let next_len = if (level + 1) < LEVELS as isize { self.levels[(level + 1) as usize].len() } else { 0 };
            let can_go_up = next_len > 0 && hi - lo >= 8;
            if !can_go_up {
                for i in lo..hi {
                    acc = merge(acc, self.at(level, i));
                }
                break;
            }
            while lo % 4 != 0 && lo < hi {
                acc = merge(acc, self.at(level, lo));
                lo += 1;
            }
            // Only complete blocks of the next level may be used.
            while hi > lo && (hi % 4 != 0 || hi / 4 > next_len) {
                hi -= 1;
                acc = merge(acc, self.at(level, hi));
            }
            lo /= 4;
            hi /= 4;
            level += 1;
        }
        Some(acc)
    }

    fn at(&self, level: isize, i: usize) -> (f32, f32) {
        if level < 0 {
            let v = self.samples[i];
            (v, v)
        } else {
            self.levels[level as usize][i]
        }
    }

    /// Per-column min/max for columns `[first + c·spc, first + (c+1)·spc)`
    /// (the integer samples in `[⌈x0⌉, ⌈x1⌉)`); `None` for columns without a sample.
    pub fn column_min_max(&self, first: f64, samples_per_col: f64, n: usize) -> Vec<Option<(f32, f32)>> {
        let edge = |c: usize| (first + c as f64 * samples_per_col).ceil().max(0.0) as u64;
        let mut out = Vec::with_capacity(n);
        let mut a = edge(0);
        for c in 0..n {
            let b = edge(c + 1);
            out.push(self.min_max(a, b));
            a = b;
        }
        out
    }

    /// Samples `[a, b)` clamped to the stored data.
    pub fn samples(&self, a: u64, b: u64) -> impl Iterator<Item = (u64, f32)> + '_ {
        // Without data, or outside it, the range is empty (not a slice
        // starting past the end).
        let (a, b) = match self.data_range() {
            Some((begin, end)) => (a.clamp(begin, end), b.clamp(begin, end)),
            None => (self.base, self.base),
        };
        let b = b.max(a);
        let base = self.base;
        self.samples.range((a - base) as usize..(b - base) as usize).enumerate().map(move |(i, &v)| (a + i as u64, v))
    }

    /// Drops data before `sample`.
    pub fn trim_before(&mut self, sample: u64) {
        if !self.started || sample <= self.data_begin {
            return;
        }
        let sample = sample.min(self.end());
        let drop = (sample - self.base) / TRIM_ALIGN * TRIM_ALIGN;
        if drop > 0 {
            self.samples.drain(..drop as usize);
            for (k, l) in self.levels.iter_mut().enumerate() {
                let n = (drop >> (2 * (k + 1))) as usize;
                l.drain(..n.min(l.len()));
            }
            self.base += drop;
            // Give the memory back once less than half is used, so
            // a rolling Continuous capture doesn't keep its high-water mark.
            fn shrink<T>(v: &mut VecDeque<T>) {
                if v.capacity() > 2 * v.len() + 1024 {
                    v.shrink_to(v.len() + v.len() / 4);
                }
            }
            shrink(&mut self.samples);
            self.levels.iter_mut().for_each(shrink);
        }
        self.data_begin = sample;
    }

    /// Drops data at and after `end` (trims); rebuilds the
    /// pyramid from the kept samples.
    pub fn truncate(&mut self, end: u64) {
        let Some((begin, old_end)) = self.data_range() else { return };
        if end >= old_end {
            return;
        }
        let end = end.max(begin + 1);
        let values: Vec<f32> = self.samples(begin, end).map(|(_, v)| v).collect();
        let mut kept = AnalogChannel::new();
        // Data follows on from `begin`; the first push sets the base.
        let _ = kept.push(begin, &values);
        *self = kept;
    }

    /// Bytes held by stored samples and their pyramid. The deques
    /// are ring buffers: space freed by trims is reused by new samples, so
    /// capacity is not released and is not counted.
    pub fn memory_bytes(&self) -> usize {
        // What is allocated, not just what is used.
        self.samples.capacity() * 4
            + self.levels.capacity() * std::mem::size_of::<VecDeque<(f32, f32)>>()
            + self.levels.iter().map(|l| l.capacity() * 8).sum::<usize>()
    }
}

#[cfg(test)]
mod empty_tests {
    use super::*;

    /// An analog row zoomed in on a channel with no data drew samples
    /// "from sample 7 766 265" of an empty buffer and panicked.
    #[test]
    fn samples_without_or_outside_data_are_empty() {
        let ch = AnalogChannel::new();
        assert_eq!(ch.samples(7_766_265, 7_766_300).count(), 0);
        let mut ch = AnalogChannel::new();
        ch.push(100, &[1.0, 2.0, 3.0]).unwrap();
        assert_eq!(ch.samples(0, 10).count(), 0, "before the data");
        assert_eq!(ch.samples(500, 600).count(), 0, "after the data");
        assert_eq!(ch.samples(101, 500).map(|x| x.1).collect::<Vec<_>>(), [2.0, 3.0]);
        assert_eq!(ch.min_max(7_766_265, 7_766_300), None);
        assert_eq!(AnalogChannel::new().column_min_max(7_766_265.0, 1.0, 3), [None, None, None]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    fn filled(n: usize, seed: u64) -> (AnalogChannel, Vec<f32>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let v: Vec<f32> = (0..n).map(|_| rng.random_range(-5.0..5.0)).collect();
        let mut ch = AnalogChannel::new();
        let mut pos = 0;
        while pos < n {
            let len = rng.random_range(1..5000).min(n - pos);
            ch.push(pos as u64, &v[pos..pos + len]).unwrap();
            pos += len;
        }
        (ch, v)
    }

    fn naive(v: &[f32], a: usize, b: usize) -> (f32, f32) {
        v[a..b].iter().fold((f32::INFINITY, f32::NEG_INFINITY), |m, &x| (m.0.min(x), m.1.max(x)))
    }

    #[test]
    fn min_max_matches_naive() {
        let (ch, v) = filled(300_000, 1);
        let mut rng = StdRng::seed_from_u64(2);
        for _ in 0..3000 {
            let a = rng.random_range(0..v.len());
            let b = rng.random_range(a + 1..=v.len());
            assert_eq!(ch.min_max(a as u64, b as u64), Some(naive(&v, a, b)), "[{a}, {b})");
        }
        assert_eq!(ch.min_max(10, 10), None);
        assert_eq!(ch.min_max(299_999, 400_000), Some((v[299_999], v[299_999])));
    }

    #[test]
    fn columns_and_interpolation() {
        let mut ch = AnalogChannel::new();
        ch.push(0, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]).unwrap();
        let cols = ch.column_min_max(0.0, 4.0, 3);
        assert_eq!(cols, vec![Some((0.0, 3.0)), Some((4.0, 7.0)), None]);
        // Zoomed in: most columns hold no sample.
        let cols = ch.column_min_max(0.5, 0.25, 4);
        assert_eq!(cols, vec![None, None, Some((1.0, 1.0)), None]);
        assert_eq!(ch.value_at(2.5), Some(2.5));
        assert_eq!(ch.value_at(7.0), Some(7.0));
        assert_eq!(ch.value_at(-1.0), None);
        assert!(ch.push(9, &[1.0]).is_err(), "gap rejected");
    }

    #[test]
    fn trim_keeps_values_and_pyramid_aligned() {
        let (mut ch, v) = filled(3 * TRIM_ALIGN as usize + 777, 3);
        let before = ch.memory_bytes();
        let cut = 2 * TRIM_ALIGN + 12345;
        ch.trim_before(cut);
        assert!(ch.memory_bytes() < before);
        assert_eq!(ch.data_range(), Some((cut, v.len() as u64)));
        assert_eq!(ch.get(cut - 1), None);
        assert_eq!(ch.get(cut), Some(v[cut as usize]));
        let mut rng = StdRng::seed_from_u64(4);
        for _ in 0..500 {
            let a = rng.random_range(cut as usize..v.len());
            let b = rng.random_range(a + 1..=v.len());
            assert_eq!(ch.min_max(a as u64, b as u64), Some(naive(&v, a, b)));
        }
        let got: Vec<f32> = ch.samples(cut, cut + 5).map(|(_, x)| x).collect();
        assert_eq!(got, v[cut as usize..cut as usize + 5]);
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use std::time::Instant;

    /// A column summary for 2000 px must take ≤ 0.5 ms.
    /// Run with `cargo test --release --lib -- --ignored perf`.
    #[test]
    #[ignore = "timing; run in release"]
    fn analog_2000_columns_budget() {
        // 100 M samples (the analog target per channel).
        let mut ch = AnalogChannel::new();
        let block: Vec<f32> = (0..1_000_000).map(|i| (i as f32 * 0.001).sin()).collect();
        for k in 0..100u64 {
            ch.push(k * 1_000_000, &block).unwrap();
        }
        let end = ch.end() as f64;
        for (name, first, spc) in
            [("fully zoomed out", 0.0, end / 2000.0), ("mid zoom", end / 3.0, 777.0), ("zoomed in", end / 2.0, 1.5)]
        {
            let n = 200;
            let t0 = Instant::now();
            for k in 0..n {
                std::hint::black_box(ch.column_min_max(first + k as f64, spc, 2000));
            }
            let per = t0.elapsed().as_secs_f64() / n as f64 * 1e3;
            eprintln!("analog {name}: {per:.3} ms per 2000-column row");
            assert!(per <= 0.5, "{name} took {per:.3} ms");
        }
    }

    #[test]
    fn truncate_keeps_the_head() {
        let mut ch = AnalogChannel::new();
        let v: Vec<f32> = (0..10_000).map(|i| i as f32).collect();
        ch.push(100, &v).unwrap();
        ch.truncate(5_100);
        assert_eq!(ch.data_range(), Some((100, 5_100)));
        assert_eq!(ch.get(5_099), Some(4_999.0));
        assert_eq!(ch.get(5_100), None);
        assert_eq!(ch.min_max(100, 5_100), Some((0.0, 4_999.0)));
    }
}
