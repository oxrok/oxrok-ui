//! Horizontal view state.

use crate::core::time::{CaptureTime, FS_PER_S};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    /// Absolute time at the left edge of the plot area.
    pub left_time: CaptureTime,
    /// Seconds per logical pixel (> 0).
    pub time_per_px: f64,
    /// Width of the plot area in logical pixels.
    pub width_px: f64,
}

impl ViewState {
    pub fn new(left_time: CaptureTime, time_per_px: f64, width_px: f64) -> Self {
        debug_assert!(time_per_px > 0.0);
        Self { left_time, time_per_px, width_px }
    }

    /// A view showing `[t0, t1]` across `width_px`.
    pub fn from_range(t0: CaptureTime, t1: CaptureTime, width_px: f64) -> Self {
        Self::new(t0, t1.secs_since(t0) / width_px, width_px)
    }

    /// Visible duration in seconds.
    pub fn time_scale(&self) -> f64 {
        self.time_per_px * self.width_px
    }

    pub fn center_time(&self) -> CaptureTime {
        self.px_to_time(self.width_px / 2.0)
    }

    pub fn right_time(&self) -> CaptureTime {
        self.px_to_time(self.width_px)
    }

    pub fn px_to_time(&self, x: f64) -> CaptureTime {
        self.left_time + CaptureTime::from_secs_f64(x * self.time_per_px)
    }

    pub fn time_to_px(&self, t: CaptureTime) -> f64 {
        (t - self.left_time).fs() as f64 / (self.time_per_px * FS_PER_S as f64)
    }

    /// Same scale, with `t` at pixel `x`.
    pub fn with_time_at(&self, t: CaptureTime, x: f64) -> Self {
        Self { left_time: t - CaptureTime::from_secs_f64(x * self.time_per_px), ..*self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_time_round_trip_at_large_offsets() {
        // Ten hours into a capture at 1 ns/px.
        let v = ViewState::new(CaptureTime::from_secs_f64(36_000.0), 1e-9, 1000.0);
        let t = v.px_to_time(123.25);
        assert!((v.time_to_px(t) - 123.25).abs() < 1e-6);
        assert!((v.time_scale() - 1e-6).abs() < 1e-18);
    }
}
