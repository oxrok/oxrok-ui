//! Exact time representation.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Femtoseconds per second.
pub const FS_PER_S: i128 = 1_000_000_000_000_000;

/// Seconds since capture start, as an exact count of femtoseconds.
///
/// Exact for any integer-Hz sample rate up to 1 PHz; range ±5.4·10⁹ years.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CaptureTime(pub i128);

impl CaptureTime {
    pub const ZERO: Self = Self(0);

    pub const fn from_fs(fs: i128) -> Self {
        Self(fs)
    }

    pub const fn fs(self) -> i128 {
        self.0
    }

    /// Converts seconds to the nearest femtosecond.
    pub fn from_secs_f64(s: f64) -> Self {
        Self((s * FS_PER_S as f64).round() as i128)
    }

    pub fn as_secs_f64(self) -> f64 {
        // Split to keep precision for large values.
        let whole = self.0 / FS_PER_S;
        let frac = self.0 % FS_PER_S;
        whole as f64 + frac as f64 / FS_PER_S as f64
    }

    /// `t = sample / rate`, exact; rounds toward zero.
    pub fn from_sample(sample: u64, rate_hz: u64) -> Self {
        assert!(rate_hz > 0, "sample rate must be > 0");
        Self(sample as i128 * FS_PER_S / rate_hz as i128)
    }

    /// The sample whose period contains `self` (floor), as a signed index.
    pub fn to_sample_floor(self, rate_hz: u64) -> i128 {
        (self.0 * rate_hz as i128).div_euclid(FS_PER_S)
    }

    /// Fractional sample position (for pixel/column mapping).
    pub fn to_sample_f64(self, rate_hz: u64) -> f64 {
        let num = self.0 * rate_hz as i128;
        let whole = num.div_euclid(FS_PER_S);
        let frac = num.rem_euclid(FS_PER_S);
        whole as f64 + frac as f64 / FS_PER_S as f64
    }

    /// `self - other` in seconds, subtracting in the exact domain first.
    pub fn secs_since(self, other: Self) -> f64 {
        (self - other).as_secs_f64()
    }

    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }
}

impl Add for CaptureTime {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for CaptureTime {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for CaptureTime {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for CaptureTime {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl Neg for CaptureTime {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

/// Wall-clock instant of capture start.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WallClock {
    pub unix_ms: i64,
    /// Sub-millisecond part, in milliseconds (`0.0 ≤ frac_ms < 1.0`).
    pub frac_ms: f64,
}

impl WallClock {
    pub fn now() -> Self {
        let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        Self { unix_ms: d.as_millis() as i64, frac_ms: (d.as_nanos() % 1_000_000) as f64 / 1e6 }
    }

    /// Nanoseconds since the Unix epoch of `self + offset`.
    pub fn unix_ns_at(&self, offset: CaptureTime) -> i128 {
        self.unix_ms as i128 * 1_000_000 + (self.frac_ms * 1e6).round() as i128 + offset.fs().div_euclid(1_000_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_conversion_is_exact() {
        // 3 samples at 3 Hz is exactly 1 s, no float drift.
        assert_eq!(CaptureTime::from_sample(3, 3), CaptureTime(FS_PER_S));
        // Hours at GHz rates stay exact.
        let s = 3_600 * 1_000_000_000u64 * 5;
        let t = CaptureTime::from_sample(s, 1_000_000_000);
        assert_eq!(t.fs(), 18_000 * FS_PER_S);
        assert_eq!(t.to_sample_floor(1_000_000_000), s as i128);
    }

    #[test]
    fn rounding_toward_zero() {
        // 1 sample at 3 Hz = 333_333_333_333_333.33 fs → truncated.
        assert_eq!(CaptureTime::from_sample(1, 3).fs(), 333_333_333_333_333);
    }

    #[test]
    fn subtraction_before_conversion_keeps_precision() {
        let a = CaptureTime(10_000 * FS_PER_S + 1_000); // 10 000 s + 1 ps
        let b = CaptureTime(10_000 * FS_PER_S);
        assert_eq!(a.secs_since(b), 1e-12);
    }

    #[test]
    fn negative_floor() {
        assert_eq!(CaptureTime(-1).to_sample_floor(1), -1);
    }
}
