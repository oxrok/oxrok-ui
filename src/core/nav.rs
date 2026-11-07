//! Navigation rules that need no UI: zoom and pan limits
//! and wheel/touchpad classification.

use crate::core::time::CaptureTime;
use crate::core::view::ViewState;
use std::collections::VecDeque;
use std::time::Duration;

/// The stored data a view may show.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DataBounds {
    pub begin: CaptureTime,
    pub end: CaptureTime,
    /// Highest active sample rate of the capture.
    pub max_sample_rate: u64,
}

impl DataBounds {
    pub fn duration_secs(&self) -> f64 {
        self.end.secs_since(self.begin)
    }
}

/// Minimum visible samples.
pub const MIN_SAMPLES_ON_SCREEN: f64 = 4.0;
/// zoom-in stops when one sample period spans this many pixels.
pub const MAX_PX_PER_SAMPLE: f64 = 400.0;
/// Zoomed out, the data may shrink to this fraction of the width.
pub const MIN_DATA_FRACTION: f64 = 0.2;

/// The allowed range of `time_per_px` for `width_px`.
///
/// `recording` lifts the zoom-out limit (only "fit" operations may zoom
/// out further, and they bypass this clamp).
pub fn time_per_px_range(bounds: &DataBounds, width_px: f64, recording: bool) -> (f64, f64) {
    let period = 1.0 / bounds.max_sample_rate.max(1) as f64;
    let min_scale = (MIN_SAMPLES_ON_SCREEN * period).max(width_px * period / MAX_PX_PER_SAMPLE);
    let min_tpp = min_scale / width_px;
    let max_tpp =
        if recording { f64::INFINITY } else { (bounds.duration_secs() / MIN_DATA_FRACTION / width_px).max(min_tpp) };
    (min_tpp, max_tpp)
}

/// Clamps a zoom factor so that `base.time_per_px × factor` stays in range.
pub fn clamp_zoom_factor(base: &ViewState, factor: f64, bounds: &DataBounds, recording: bool) -> f64 {
    let (lo, hi) = time_per_px_range(bounds, base.width_px, recording);
    (base.time_per_px * factor).clamp(lo, hi) / base.time_per_px
}

/// Clamps `left_time` so the view keeps showing data.
///
/// - finished: the viewport stays inside the data when the data is wider,
///   otherwise the data stays fully visible;
/// - recording: at least 20 % of the viewport shows data.
pub fn clamp_pan(view: ViewState, bounds: &DataBounds, recording: bool) -> ViewState {
    let scale = CaptureTime::from_secs_f64(view.time_scale());
    let (lo, hi) = if recording {
        let keep = CaptureTime::from_secs_f64(view.time_scale() * MIN_DATA_FRACTION);
        (bounds.begin - scale + keep, bounds.end - keep)
    } else if bounds.end - bounds.begin >= scale {
        (bounds.begin, bounds.end - scale)
    } else {
        (bounds.end - scale, bounds.begin)
    };
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    ViewState { left_time: view.left_time.clamp(lo, hi), ..view }
}

/// Applies both and, keeping `fix_px` in place when the
/// scale has to change.
pub fn clamp_view(view: ViewState, bounds: &DataBounds, recording: bool, fix_px: f64) -> ViewState {
    let (lo, hi) = time_per_px_range(bounds, view.width_px, recording);
    let tpp = view.time_per_px.clamp(lo, hi);
    let view = if tpp != view.time_per_px {
        ViewState { time_per_px: tpp, ..view }.with_time_at(view.px_to_time(fix_px), fix_px)
    } else {
        view
    };
    clamp_pan(view, bounds, recording)
}

/// The follow-latest view (newest sample at 95 % of the width).
pub fn follow_view(view: ViewState, newest: CaptureTime) -> ViewState {
    view.with_time_at(newest, view.width_px * FOLLOW_FRACTION)
}

/// Data occupies the left 95 % while following.
pub const FOLLOW_FRACTION: f64 = 0.95;

/// Pointer velocity from the newest samples of a drag.
#[derive(Clone, Debug, Default)]
pub struct VelocityTracker {
    samples: VecDeque<(Duration, f64)>,
}

/// Parameters.
const VELOCITY_SAMPLES: usize = 10;
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

impl VelocityTracker {
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Records the pointer x at `now`.
    pub fn push(&mut self, now: Duration, x: f64) {
        self.samples.push_back((now, x));
        while self.samples.len() > VELOCITY_SAMPLES {
            self.samples.pop_front();
        }
    }

    /// px/s over the samples of the last 100 ms (0 with fewer than two).
    pub fn velocity(&self, now: Duration) -> f64 {
        let recent: Vec<_> = self.samples.iter().filter(|(t, _)| now.saturating_sub(*t) <= VELOCITY_WINDOW).collect();
        match (recent.first(), recent.last()) {
            (Some(a), Some(b)) if b.0 > a.0 => (b.1 - a.1) / (b.0 - a.0).as_secs_f64(),
            _ => 0.0,
        }
    }
}

/// What a wheel/touchpad event does.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum WheelAction {
    /// Multiply the time scale by this factor.
    HorizontalZoom { factor: f64, discrete: bool },
    /// Pan by this many pixels; positive is later in time.
    HorizontalPan { dx_px: f64 },
    /// Rule 5 needs more history before it decides.
    Ignore,
}

/// One normalized wheel event: deltas in notch units × 120,
/// positive `dy` = zoom in.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WheelInput {
    pub dx: f64,
    pub dy: f64,
    pub discrete: bool,
    /// Ctrl (Cmd on macOS) held.
    pub command: bool,
}

/// Classifies wheel events per rules 3–5 (rules 1–2 are UI concerns).
#[derive(Clone, Debug, Default)]
pub struct WheelClassifier {
    history: VecDeque<(Duration, f64, f64)>,
}

/// Rule 5 parameters.
const HISTORY_LEN: usize = 15;
const HISTORY_WINDOW: Duration = Duration::from_millis(200);
const HISTORY_MIN_EVENTS: usize = 5;

impl WheelClassifier {
    pub fn classify(&mut self, e: WheelInput, now: Duration) -> WheelAction {
        let zoom = |dy: f64, discrete: bool| WheelAction::HorizontalZoom {
            factor: crate::core::animator::wheel_zoom_factor(dy, discrete),
            discrete,
        };
        // Rule 3.
        if e.command {
            return zoom(e.dy, e.discrete);
        }
        // Rule 4.
        if e.discrete {
            return if e.dy.abs() > e.dx.abs() {
                zoom(e.dy, true)
            } else {
                WheelAction::HorizontalPan { dx_px: -e.dx }
            };
        }
        // Rule 5: touchpad.
        self.history.push_back((now, e.dx, e.dy));
        while self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }
        let recent: Vec<_> = self.history.iter().filter(|(t, _, _)| now.saturating_sub(*t) <= HISTORY_WINDOW).collect();
        if recent.len() < HISTORY_MIN_EVENTS {
            return WheelAction::Ignore;
        }
        let dominant = |pick: fn(&(Duration, f64, f64)) -> f64| {
            let pos: f64 = recent.iter().map(|e| pick(e).max(0.0)).sum();
            let neg: f64 = recent.iter().map(|e| (-pick(e)).max(0.0)).sum();
            pos.max(neg)
        };
        if dominant(|e| e.2) > dominant(|e| e.1) {
            zoom(e.dy, false)
        } else {
            WheelAction::HorizontalPan { dx_px: -e.dx }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(dur_s: f64, rate: u64) -> DataBounds {
        DataBounds { begin: CaptureTime::ZERO, end: CaptureTime::from_secs_f64(dur_s), max_sample_rate: rate }
    }

    fn view(left_s: f64, tpp: f64) -> ViewState {
        ViewState::new(CaptureTime::from_secs_f64(left_s), tpp, 1000.0)
    }

    /// U-05: zoom limits.
    #[test]
    fn zoom_limits() {
        let b = bounds(2.0, 1_000_000);
        let (lo, hi) = time_per_px_range(&b, 1000.0, false);
        // 4 samples on 1000 px beats 400 px per sample (2.5 samples).
        assert!((lo * 1000.0 - 4e-6).abs() < 1e-15);
        // Data may shrink to 20 %: 2 s / 0.2 = 10 s visible.
        assert!((hi * 1000.0 - 10.0).abs() < 1e-12);
        let (_, hi_rec) = time_per_px_range(&b, 1000.0, true);
        assert!(hi_rec.is_infinite());
        // Wide plot: 400 px per sample dominates, 4000/400 = 10 samples.
        let (lo, _) = time_per_px_range(&b, 4000.0, false);
        assert!((lo * 4000.0 - 10e-6).abs() < 1e-15);
    }

    #[test]
    fn zoom_factor_is_clamped() {
        let b = bounds(2.0, 1_000_000);
        let v = view(0.0, 1e-3);
        assert!((clamp_zoom_factor(&v, 100.0, &b, false) - 10.0).abs() < 1e-9);
        assert_eq!(clamp_zoom_factor(&v, 0.5, &b, false), 0.5);
    }

    /// U-05: pan clamps for a finished capture.
    #[test]
    fn pan_limits_finished() {
        let b = bounds(2.0, 1_000_000);
        // Data wider than the view: stay inside.
        let v = clamp_pan(view(-5.0, 1e-3), &b, false);
        assert_eq!(v.left_time, CaptureTime::ZERO);
        let v = clamp_pan(view(5.0, 1e-3), &b, false);
        assert_eq!(v.right_time(), b.end);
        // Data narrower than the view: data fully visible.
        let v = clamp_pan(view(1.0, 4e-3), &b, false);
        assert_eq!(v.left_time, CaptureTime::ZERO);
        let v = clamp_pan(view(-9.0, 4e-3), &b, false);
        assert_eq!(v.right_time(), b.end);
    }

    /// U-05: while recording, 20 % of the view keeps showing data.
    #[test]
    fn pan_limits_recording_keep_20_percent() {
        let b = bounds(2.0, 1_000_000);
        let v = clamp_pan(view(100.0, 1e-3), &b, true);
        // 0.2 s of the 1 s view shows data.
        assert!((v.left_time.secs_since(b.end) + 0.2).abs() < 1e-12);
        let v = clamp_pan(view(-100.0, 1e-3), &b, true);
        assert!((v.right_time().secs_since(b.begin) - 0.2).abs() < 1e-12);
    }

    #[test]
    fn clamp_view_keeps_fix_point() {
        let b = bounds(2.0, 1_000_000);
        let v = view(1.0, 1e-9); // far below the minimum scale
        let fix = 300.0;
        let t = v.px_to_time(fix);
        let c = clamp_view(v, &b, false, fix);
        assert!((c.time_to_px(t) - fix).abs() < 1e-6);
        assert!(c.time_per_px > v.time_per_px);
    }

    #[test]
    fn follow_puts_newest_at_95_percent() {
        let v = follow_view(view(0.0, 1e-3), CaptureTime::from_secs_f64(3.0));
        assert!((v.time_to_px(CaptureTime::from_secs_f64(3.0)) - 950.0).abs() < 1e-6);
    }

    #[test]
    fn velocity_uses_the_last_100_ms() {
        let mut v = VelocityTracker::default();
        let ms = Duration::from_millis;
        // A slow start, then 2 px/ms.
        v.push(ms(0), 0.0);
        v.push(ms(50), 1.0);
        for i in 0..5 {
            v.push(ms(150 + i * 10), 10.0 + 20.0 * i as f64);
        }
        assert!((v.velocity(ms(190)) - 2000.0).abs() < 1e-9);
        // Too late: nothing recent.
        assert_eq!(v.velocity(ms(1000)), 0.0);
    }

    #[test]
    fn wheel_rules() {
        let mut c = WheelClassifier::default();
        let t = Duration::ZERO;
        // Rule 3: Ctrl zooms even for horizontal-ish input.
        let a = c.classify(WheelInput { dx: 0.0, dy: 120.0, discrete: false, command: true }, t);
        assert!(matches!(a, WheelAction::HorizontalZoom { discrete: false, .. }));
        // Rule 4: notched wheel.
        let a = c.classify(WheelInput { dx: 0.0, dy: 120.0, discrete: true, command: false }, t);
        assert_eq!(a, WheelAction::HorizontalZoom { factor: 0.5f64.sqrt(), discrete: true });
        let a = c.classify(WheelInput { dx: 120.0, dy: 0.0, discrete: true, command: false }, t);
        assert_eq!(a, WheelAction::HorizontalPan { dx_px: -120.0 });
    }

    #[test]
    fn touchpad_needs_history_then_follows_dominant_axis() {
        let mut c = WheelClassifier::default();
        let ev = |dx, dy| WheelInput { dx, dy, discrete: false, command: false };
        for i in 0..4 {
            assert_eq!(c.classify(ev(10.0, 1.0), Duration::from_millis(i * 10)), WheelAction::Ignore);
        }
        assert_eq!(c.classify(ev(10.0, 1.0), Duration::from_millis(40)), WheelAction::HorizontalPan { dx_px: -10.0 });
        // Old events fall out of the 200 ms window.
        assert_eq!(c.classify(ev(0.0, 10.0), Duration::from_millis(1000)), WheelAction::Ignore);
        for i in 1..5 {
            let a = c.classify(ev(0.0, 10.0), Duration::from_millis(1000 + i * 10));
            if i == 4 {
                assert!(matches!(a, WheelAction::HorizontalZoom { discrete: false, .. }));
            }
        }
    }
}
