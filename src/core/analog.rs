//! Analog row vertical model.

use serde::{Deserialize, Serialize};

/// Defaults when the device reports no input range.
pub const DEFAULT_INPUT_RANGE: (f64, f64) = (-10.0, 10.0);
pub const DEFAULT_MIN_FSR: f64 = 0.1;
/// Y Fit margin, as a fraction of the data span, top and bottom.
pub const FIT_MARGIN: f64 = 0.1;

/// Vertical view of one analog row.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalogView {
    /// Units (e.g. volts) per point; > 0.
    pub units_per_px: f64,
    /// Value at the vertical middle of the inner area.
    pub center_value: f64,
    /// Device input range `(min, max)`.
    pub input_range: (f64, f64),
    /// Smallest full scale allowed.
    pub min_fsr: f64,
}

impl AnalogView {
    /// The full input range fits the inner height, with 20 pt of
    /// margin in total for rows ≥ 100 pt and 10 % otherwise.
    pub fn fit_input(input_range: (f64, f64), inner_height: f64) -> Self {
        let (lo, hi) = input_range;
        let margin = if inner_height >= 100.0 { 20.0 } else { inner_height * 0.1 };
        let usable = (inner_height - margin).max(1.0);
        Self { units_per_px: (hi - lo) / usable, center_value: (lo + hi) / 2.0, input_range, min_fsr: DEFAULT_MIN_FSR }
    }

    /// `y` from the top of the inner area.
    pub fn value_to_y(&self, v: f64, inner_height: f64) -> f64 {
        inner_height / 2.0 - (v - self.center_value) / self.units_per_px
    }

    pub fn y_to_value(&self, y: f64, inner_height: f64) -> f64 {
        self.center_value + (inner_height / 2.0 - y) * self.units_per_px
    }

    /// Visible `(bottom, top)` values.
    pub fn visible_range(&self, inner_height: f64) -> (f64, f64) {
        let half = self.units_per_px * inner_height / 2.0;
        (self.center_value - half, self.center_value + half)
    }

    /// Clamps the full scale, then the center.
    pub fn clamped(mut self, inner_height: f64) -> Self {
        let (lo, hi) = self.input_range;
        let max_fsr = (hi - lo).max(self.min_fsr);
        let h = inner_height.max(1.0);
        let fsr = (self.units_per_px * h).clamp(self.min_fsr, max_fsr);
        self.units_per_px = fsr / h;
        let (c_lo, c_hi) = (lo + fsr / 2.0, hi - fsr / 2.0);
        self.center_value = if c_lo <= c_hi { self.center_value.clamp(c_lo, c_hi) } else { (lo + hi) / 2.0 };
        self
    }

    /// Zoom by `k` about value `v`, keeping `v` at its pixel.
    pub fn zoom_about(self, k: f64, v: f64, inner_height: f64) -> Self {
        let before = self.units_per_px;
        let clamped = Self { units_per_px: before * k, ..self }.clamped(inner_height);
        let upp = clamped.units_per_px;
        let center = v - (v - self.center_value) * upp / before;
        Self { units_per_px: upp, center_value: center, ..self }.clamped(inner_height)
    }

    /// Vertical pan by `dy` points (positive = content moves down).
    pub fn pan(self, dy: f64, inner_height: f64) -> Self {
        Self { center_value: self.center_value + dy * self.units_per_px, ..self }.clamped(inner_height)
    }

    /// Fit `[min, max]` with a 10 % margin top and bottom.
    pub fn fit(self, min: f64, max: f64, inner_height: f64) -> Self {
        let span = (max - min).max(self.min_fsr / (1.0 + 2.0 * FIT_MARGIN));
        let full = span * (1.0 + 2.0 * FIT_MARGIN);
        Self { units_per_px: full / inner_height.max(1.0), center_value: (min + max) / 2.0, ..self }
            .clamped(inner_height)
    }
}

/// Vertical zoom factor; `fine` (Shift) multiplies the exponent by 0.1.
pub fn vertical_zoom_factor(dy: f64, fine: bool) -> f64 {
    let e = -dy / 120.0;
    (if fine { e * 0.1 } else { e }).exp2()
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f64 = 82.0;

    #[test]
    fn default_view_fits_input_range() {
        let v = AnalogView::fit_input(DEFAULT_INPUT_RANGE, 200.0);
        let (lo, hi) = v.visible_range(200.0);
        // 20 pt margin in total: 10 pt top and bottom.
        assert!((v.value_to_y(10.0, 200.0) - 10.0).abs() < 1e-9);
        assert!((v.value_to_y(-10.0, 200.0) - 190.0).abs() < 1e-9);
        assert!(lo < -10.0 && hi > 10.0);
        // Small rows: 10 % margin.
        let v = AnalogView::fit_input((0.0, 5.0), 50.0);
        assert!((v.value_to_y(5.0, 50.0) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn y_value_round_trip() {
        let v = AnalogView::fit_input(DEFAULT_INPUT_RANGE, H);
        for y in [0.0, 10.5, 41.0, 82.0] {
            assert!((v.value_to_y(v.y_to_value(y, H), H) - y).abs() < 1e-9);
        }
    }

    #[test]
    fn zoom_keeps_value_under_pointer() {
        let v = AnalogView::fit_input(DEFAULT_INPUT_RANGE, H);
        let y = 20.0;
        let at = v.y_to_value(y, H);
        let z = v.zoom_about(0.25, at, H);
        assert!(z.units_per_px < v.units_per_px);
        assert!((z.value_to_y(at, H) - y).abs() < 1e-9);
    }

    #[test]
    fn limits() {
        let v = AnalogView::fit_input(DEFAULT_INPUT_RANGE, H);
        // Can't zoom out past the input range.
        let z = v.zoom_about(100.0, 0.0, H);
        assert!((z.units_per_px * H - 20.0).abs() < 1e-9);
        assert_eq!(z.center_value, 0.0);
        // Can't zoom in past min_fsr.
        let z = v.zoom_about(1e-9, 3.0, H);
        assert!((z.units_per_px * H - DEFAULT_MIN_FSR).abs() < 1e-12);
        // Can't pan past the range.
        let p = z.pan(1e9, H);
        assert!((p.visible_range(H).1 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn fit_adds_ten_percent_margins() {
        let v = AnalogView::fit_input(DEFAULT_INPUT_RANGE, H).fit(-1.0, 3.0, H);
        let (lo, hi) = v.visible_range(H);
        assert!((lo - -1.4).abs() < 1e-9 && (hi - 3.4).abs() < 1e-9);
        // A flat signal still gets a minimum span.
        let v = v.fit(2.0, 2.0, H);
        assert!((v.units_per_px * H - DEFAULT_MIN_FSR).abs() < 1e-9);
    }

    #[test]
    fn zoom_factor() {
        assert_eq!(vertical_zoom_factor(120.0, false), 0.5);
        assert!((vertical_zoom_factor(120.0, true) - 0.5f64.powf(0.1)).abs() < 1e-12);
    }
}
