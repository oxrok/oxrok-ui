//! View animation model.
//!
//! Pure and clock-agnostic: callers pass the current time (from a
//! [`Clock`](crate::core::Clock)) to [`ViewAnimator::request`] and
//! [`ViewAnimator::advance`].

use crate::core::time::CaptureTime;
use crate::core::view::ViewState;
use std::time::Duration;

/// Default durations.
pub const WHEEL_ZOOM_DURATION: Duration = Duration::from_millis(100);
pub const KEY_PAN_DURATION: Duration = Duration::from_millis(100);
pub const JUMP_DURATION: Duration = Duration::from_millis(300);
pub const FLING_TAU: Duration = Duration::from_millis(100);

/// A fling stops once less than this many pixels of motion remain.
const DECAY_REST_PX: f64 = 0.5;

/// `factor = 2^(k·(−dy)/120)`, k = 0.5 for a notched wheel and 1
/// for a touchpad. The factor multiplies `time_scale` (< 1 zooms in).
pub fn wheel_zoom_factor(dy: f64, discrete: bool) -> f64 {
    let k = if discrete { 0.5 } else { 1.0 };
    (k * -dy / 120.0).exp2()
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    Left,
    Center,
    Right,
}

/// A view change (table).
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ViewRequest {
    Immediate(ViewState),
    /// Multiply `time_scale` by `factor`, keeping the time under `fix_px` in place.
    Zoom {
        fix_px: f64,
        factor: f64,
        duration: Duration,
    },
    /// Animate `time_scale` to an absolute value about `fix_px`.
    ZoomAbsolute {
        fix_px: f64,
        time_scale: f64,
        duration: Duration,
    },
    /// Move the view by `dx_px` (positive: later in time).
    PanBy {
        dx_px: f64,
        duration: Duration,
    },
    MoveToTime {
        time: CaptureTime,
        anchor: Anchor,
        duration: Duration,
    },
    /// Ends with exactly `[t0, t1]` filling the width.
    MoveToRange {
        t0: CaptureTime,
        t1: CaptureTime,
        duration: Duration,
    },
    /// Exponentially decaying pan: `v(t) = v0·e^(−t/τ)`; positive is later in time.
    Fling {
        velocity_px_s: f64,
        tau: Duration,
    },
    /// Constant-velocity pan until cancelled.
    PanIndefinite {
        velocity_px_s: f64,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Easing {
    OutCubic,
    InOutCubic,
}

impl Easing {
    fn apply(self, p: f64) -> f64 {
        match self {
            Easing::OutCubic => 1.0 - (1.0 - p).powi(3),
            Easing::InOutCubic => {
                if p < 0.5 {
                    4.0 * p * p * p
                } else {
                    1.0 - (-2.0 * p + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Path {
    /// Keeps `time` at pixel `px` throughout (zooms).
    FixPoint { px: f64, time: CaptureTime },
    /// Linear `left_time`, logarithmic `time_per_px`.
    Linear,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Motion {
    Idle,
    Tween { start: ViewState, target: ViewState, t0: Duration, duration: Duration, easing: Easing, path: Path },
    Decay { start: ViewState, t0: Duration, v0: f64, tau: f64 },
    Constant { start: ViewState, t0: Duration, v: f64 },
}

#[derive(Clone, Debug)]
pub struct ViewAnimator {
    state: ViewState,
    motion: Motion,
    /// Developer setting: multiplies every duration.
    pub speed_scale: f64,
}

fn lerp_log(a: f64, b: f64, e: f64) -> f64 {
    a * (b / a).powf(e)
}

fn shift(v: ViewState, px: f64) -> ViewState {
    ViewState { left_time: v.left_time + CaptureTime::from_secs_f64(px * v.time_per_px), ..v }
}

impl ViewAnimator {
    pub fn new(state: ViewState) -> Self {
        Self { state, motion: Motion::Idle, speed_scale: 1.0 }
    }

    pub fn is_animating(&self) -> bool {
        self.motion != Motion::Idle
    }

    /// The final state of the running tween, or the current state.
    pub fn target(&self) -> ViewState {
        match self.motion {
            Motion::Tween { target, .. } => target,
            _ => self.state,
        }
    }

    /// The state computed by the last [`advance`](Self::advance).
    pub fn state(&self) -> ViewState {
        self.state
    }

    /// The interpolated state at `now`, without changing anything.
    pub fn state_at(&self, now: Duration) -> ViewState {
        self.sample(now).0
    }

    fn sample(&self, now: Duration) -> (ViewState, bool) {
        let elapsed = |t0: Duration| now.saturating_sub(t0).as_secs_f64();
        match self.motion {
            Motion::Idle => (self.state, true),
            Motion::Tween { start, target, t0, duration, easing, path } => {
                let p = elapsed(t0) / duration.as_secs_f64();
                if p >= 1.0 {
                    return (target, true);
                }
                let e = easing.apply(p);
                let tpp = lerp_log(start.time_per_px, target.time_per_px, e);
                let left = match path {
                    Path::FixPoint { px, time } => time - CaptureTime::from_secs_f64(px * tpp),
                    Path::Linear => {
                        let d = (target.left_time - start.left_time).fs() as f64;
                        start.left_time + CaptureTime::from_fs((d * e).round() as i128)
                    }
                };
                (ViewState { left_time: left, time_per_px: tpp, ..target }, false)
            }
            Motion::Decay { start, t0, v0, tau } => {
                let t = elapsed(t0);
                let decay = (-t / tau).exp();
                let px = v0 * tau * (1.0 - decay);
                let rest = (v0 * tau * decay).abs() < DECAY_REST_PX;
                (shift(start, px), rest)
            }
            Motion::Constant { start, t0, v } => (shift(start, v * elapsed(t0)), false),
        }
    }

    /// Advances to `now` and returns the current view. Finished motions settle.
    pub fn advance(&mut self, now: Duration) -> ViewState {
        let (state, done) = self.sample(now);
        self.state = state;
        if done {
            self.motion = Motion::Idle;
        }
        state
    }

    /// Stops at the current interpolated state.
    pub fn cancel(&mut self, now: Duration) {
        self.advance(now);
        self.motion = Motion::Idle;
    }

    /// Changes the plot width, keeping `left_time` and the scale.
    pub fn set_width(&mut self, width_px: f64, now: Duration) {
        self.advance(now);
        self.state.width_px = width_px;
        if let Motion::Tween { start, target, .. } = &mut self.motion {
            start.width_px = width_px;
            target.width_px = width_px;
        }
    }

    fn scaled(&self, d: Duration) -> Duration {
        d.mul_f64(self.speed_scale.max(0.0))
    }

    fn tween(
        &mut self,
        now: Duration,
        current: ViewState,
        target: ViewState,
        duration: Duration,
        easing: Easing,
        path: Path,
    ) {
        let duration = self.scaled(duration);
        if duration.is_zero() {
            self.state = target;
            self.motion = Motion::Idle;
        } else {
            self.state = current;
            self.motion = Motion::Tween { start: current, target, t0: now, duration, easing, path };
        }
    }

    /// Starts `req` from the current interpolated state. Zooms and
    /// pans compose with the in-flight target, so quick repeated requests end
    /// where slow ones would.
    pub fn request(&mut self, req: ViewRequest, now: Duration) {
        let current = self.advance(now);
        let base = self.target();
        match req {
            ViewRequest::Immediate(v) => {
                self.state = v;
                self.motion = Motion::Idle;
            }
            ViewRequest::Zoom { fix_px, factor, duration } => {
                self.zoom_to(now, current, fix_px, base.time_per_px * factor, duration);
            }
            ViewRequest::ZoomAbsolute { fix_px, time_scale, duration } => {
                self.zoom_to(now, current, fix_px, time_scale / current.width_px, duration);
            }
            ViewRequest::PanBy { dx_px, duration } => {
                let target = shift(base, dx_px);
                self.tween(now, current, target, duration, Easing::OutCubic, Path::Linear);
            }
            ViewRequest::MoveToTime { time, anchor, duration } => {
                let x = match anchor {
                    Anchor::Left => 0.0,
                    Anchor::Center => base.width_px / 2.0,
                    Anchor::Right => base.width_px,
                };
                let target = base.with_time_at(time, x);
                self.tween(now, current, target, duration, Easing::InOutCubic, Path::Linear);
            }
            ViewRequest::MoveToRange { t0, t1, duration } => {
                let target = ViewState::from_range(t0, t1, current.width_px);
                self.tween(now, current, target, duration, Easing::InOutCubic, Path::Linear);
            }
            ViewRequest::Fling { velocity_px_s, tau } => {
                let tau = self.scaled(tau).as_secs_f64();
                self.state = current;
                self.motion = if tau > 0.0 {
                    Motion::Decay { start: current, t0: now, v0: velocity_px_s, tau }
                } else {
                    Motion::Idle
                };
            }
            ViewRequest::PanIndefinite { velocity_px_s } => {
                self.state = current;
                self.motion = Motion::Constant { start: current, t0: now, v: velocity_px_s };
            }
        }
    }

    fn zoom_to(&mut self, now: Duration, current: ViewState, fix_px: f64, time_per_px: f64, duration: Duration) {
        let time = current.px_to_time(fix_px);
        let target = ViewState { time_per_px, ..current }.with_time_at(time, fix_px);
        self.tween(now, current, target, duration, Easing::OutCubic, Path::FixPoint { px: fix_px, time });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn view() -> ViewState {
        ViewState::new(CaptureTime::from_secs_f64(12.5), 1e-6, 1000.0)
    }

    fn zoom(fix_px: f64, factor: f64) -> ViewRequest {
        ViewRequest::Zoom { fix_px, factor, duration: WHEEL_ZOOM_DURATION }
    }

    #[test]
    fn wheel_factor() {
        assert_eq!(wheel_zoom_factor(120.0, true), 0.5f64.sqrt());
        assert_eq!(wheel_zoom_factor(-240.0, true), 2.0);
        assert_eq!(wheel_zoom_factor(120.0, false), 0.5);
    }

    /// U-04: the fix-point stays at the same pixel within 0.01 px, during
    /// and after each of 100 random wheel zooms.
    #[test]
    fn u04_fix_point_is_stable() {
        let mut rng = StdRng::seed_from_u64(4);
        let mut a = ViewAnimator::new(view());
        let mut now = Duration::ZERO;
        for _ in 0..100 {
            let fix_px = rng.random_range(0.0..1000.0);
            let before = a.advance(now);
            // Keep the scale in the range allows for real devices
            // (≥ 1 ps/px); below that, femtosecond quantization dominates.
            let mut dy: f64 = rng.random_range(-480.0..480.0);
            if before.time_per_px < 1e-11 {
                dy = -dy.abs();
            } else if before.time_per_px > 1e2 {
                dy = dy.abs();
            }
            let t_fix = before.px_to_time(fix_px);
            a.request(zoom(fix_px, wheel_zoom_factor(dy, rng.random_bool(0.5))), now);
            for step in [10, 37, 50, 99, 150] {
                let v = a.advance(now + ms(step));
                let err = (v.time_to_px(t_fix) - fix_px).abs();
                assert!(err < 0.01, "step {step}: err {err} tpp {:e} left {:?}", v.time_per_px, v.left_time);
            }
            now += ms(200);
        }
    }

    /// U-06: ten wheel ticks within 100 ms give the same final scale as ten spaced ones.
    #[test]
    fn u06_retargeted_zoom_composes() {
        let factor = wheel_zoom_factor(120.0, true);
        let mut quick = ViewAnimator::new(view());
        for i in 0..10 {
            quick.request(zoom(400.0, factor), ms(i * 10));
        }
        let q = quick.advance(ms(1000));

        let mut slow = ViewAnimator::new(view());
        for i in 0..10 {
            slow.request(zoom(400.0, factor), ms(i * 300));
        }
        let s = slow.advance(ms(5000));

        assert!(!quick.is_animating() && !slow.is_animating());
        assert!((q.time_per_px / s.time_per_px - 1.0).abs() < 1e-12);
        assert!((q.time_per_px / (view().time_per_px * factor.powi(10)) - 1.0).abs() < 1e-12);
        assert!(s.time_to_px(q.left_time).abs() < 0.01);
    }

    #[test]
    fn retarget_starts_from_interpolated_state() {
        let mut a = ViewAnimator::new(view());
        a.request(ViewRequest::PanBy { dx_px: 100.0, duration: ms(100) }, ms(0));
        let mid = a.state_at(ms(50));
        a.request(ViewRequest::PanBy { dx_px: 100.0, duration: ms(100) }, ms(50));
        // No jump at the retarget instant…
        assert_eq!(a.state_at(ms(50)), mid);
        // …and the pans compose.
        let end = a.advance(ms(500));
        assert!((view().time_to_px(end.left_time) - 200.0).abs() < 1e-6);
    }

    #[test]
    fn cancel_keeps_interpolated_state() {
        let mut a = ViewAnimator::new(view());
        a.request(ViewRequest::PanBy { dx_px: 100.0, duration: ms(100) }, ms(0));
        let mid = a.state_at(ms(30));
        a.cancel(ms(30));
        assert!(!a.is_animating());
        assert_eq!(a.advance(ms(1000)), mid);
    }

    #[test]
    fn move_to_range_ends_exactly() {
        let mut a = ViewAnimator::new(view());
        let t0 = CaptureTime::from_secs_f64(1.0);
        let t1 = CaptureTime::from_secs_f64(3.0);
        a.request(ViewRequest::MoveToRange { t0, t1, duration: JUMP_DURATION }, ms(0));
        assert!(a.is_animating());
        let v = a.advance(ms(300));
        assert_eq!(v.left_time, t0);
        assert_eq!(v.right_time(), t1);
        assert!(!a.is_animating());
    }

    #[test]
    fn move_to_time_center_anchor() {
        let mut a = ViewAnimator::new(view());
        let t = CaptureTime::from_secs_f64(20.0);
        a.request(ViewRequest::MoveToTime { time: t, anchor: Anchor::Center, duration: JUMP_DURATION }, ms(0));
        let v = a.advance(ms(300));
        assert_eq!(v.center_time(), t);
        assert_eq!(v.time_per_px, view().time_per_px);
    }

    #[test]
    fn fling_decays_to_v0_tau() {
        let mut a = ViewAnimator::new(view());
        a.request(ViewRequest::Fling { velocity_px_s: 1000.0, tau: FLING_TAU }, ms(0));
        // Total travel → v0·τ = 100 px.
        let v = a.advance(ms(2000));
        assert!(!a.is_animating());
        assert!((view().time_to_px(v.left_time) - 100.0).abs() < DECAY_REST_PX + 1e-6);
    }

    #[test]
    fn pan_indefinite_until_cancelled() {
        let mut a = ViewAnimator::new(view());
        a.request(ViewRequest::PanIndefinite { velocity_px_s: -50.0 }, ms(0));
        let v = a.advance(ms(2000));
        assert!((view().time_to_px(v.left_time) + 100.0).abs() < 1e-6);
        assert!(a.is_animating());
        a.cancel(ms(2000));
        assert!(!a.is_animating());
    }

    #[test]
    fn zero_duration_is_immediate_and_speed_scale_applies() {
        let mut a = ViewAnimator::new(view());
        a.speed_scale = 0.0;
        a.request(zoom(0.0, 0.5), ms(0));
        assert!(!a.is_animating());
        assert_eq!(a.state().time_per_px, view().time_per_px * 0.5);

        let mut b = ViewAnimator::new(view());
        b.speed_scale = 2.0;
        b.request(zoom(0.0, 0.5), ms(0));
        assert!(b.advance(ms(150)) != b.target());
        assert!(b.advance(ms(200)) == b.target());
    }

    #[test]
    fn easing_endpoints() {
        for e in [Easing::OutCubic, Easing::InOutCubic] {
            assert_eq!(e.apply(0.0), 0.0);
            assert_eq!(e.apply(1.0), 1.0);
            assert!((e.apply(0.5) - 0.5).abs() < 0.5);
        }
    }
}
