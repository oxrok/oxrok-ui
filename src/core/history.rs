//! View history, back and forward.

use crate::core::ChannelId;
use crate::core::analog::AnalogView;
use crate::core::view::ViewState;
use std::time::Duration;

/// Changes settle for this long before they are recorded.
pub const HISTORY_DEBOUNCE: Duration = Duration::from_millis(300);

/// The horizontal view and each visible analog row's vertical view.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewEntry {
    pub view: ViewState,
    pub analog: Vec<(ChannelId, AnalogView)>,
}

impl ViewEntry {
    /// Equal up to float noise; the width doesn't count (window resizes
    /// aren't navigation).
    fn same(&self, other: &ViewEntry) -> bool {
        let v = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs());
        self.view.left_time == other.view.left_time
            && v(self.view.time_per_px, other.view.time_per_px)
            && self.analog.len() == other.analog.len()
            && self.analog.iter().zip(&other.analog).all(|((ca, a), (cb, b))| {
                ca == cb && v(a.units_per_px, b.units_per_px) && v(a.center_value, b.center_value)
            })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ViewHistory {
    back: Vec<ViewEntry>,
    current: Option<ViewEntry>,
    forward: Vec<ViewEntry>,
    /// A change seen at this time, not yet recorded.
    pending: Option<(ViewEntry, Duration)>,
}

/// Oldest entries are dropped beyond this.
const MAX_ENTRIES: usize = 100;

impl ViewHistory {
    /// Feeds the state shown this frame. A change is recorded once it has
    /// held for [`HISTORY_DEBOUNCE`], so a burst of wheel steps or a drag is
    /// one entry.
    pub fn observe(&mut self, entry: ViewEntry, now: Duration) {
        let Some(cur) = &self.current else {
            self.current = Some(entry);
            return;
        };
        if cur.same(&entry) {
            self.pending = None;
            return;
        }
        match &self.pending {
            Some((p, _)) if p.same(&entry) => {}
            _ => {
                self.pending = Some((entry, now));
                return;
            }
        }
        let (_, since) = self.pending.as_ref().unwrap();
        if now.saturating_sub(*since) >= HISTORY_DEBOUNCE {
            let (entry, _) = self.pending.take().unwrap();
            self.back.push(self.current.replace(entry).unwrap());
            if self.back.len() > MAX_ENTRIES {
                self.back.remove(0);
            }
            self.forward.clear();
        }
    }

    /// Whether a change is waiting for its debounce (the caller keeps
    /// repainting until it settles).
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Previous: the entry to apply. A pending change is recorded
    /// first, so "back" returns to where the user was before it.
    pub fn back(&mut self) -> Option<ViewEntry> {
        if let Some((p, _)) = self.pending.take() {
            self.back.push(self.current.replace(p).unwrap());
            self.forward.clear();
        }
        let prev = self.back.pop()?;
        self.forward.push(self.current.replace(prev.clone()).unwrap());
        Some(prev)
    }

    /// Next.
    pub fn forward(&mut self) -> Option<ViewEntry> {
        let next = self.forward.pop()?;
        self.back.push(self.current.replace(next.clone()).unwrap());
        Some(next)
    }

    /// Clear.
    pub fn clear(&mut self) {
        *self = Self { current: self.current.take(), ..Self::default() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CaptureTime;

    fn e(left: f64, tpp: f64) -> ViewEntry {
        ViewEntry { view: ViewState::new(CaptureTime::from_secs_f64(left), tpp, 1000.0), analog: Vec::new() }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn bursts_collapse_after_the_debounce() {
        let mut h = ViewHistory::default();
        h.observe(e(0.0, 1e-3), ms(0));
        // A wheel burst: five scales within 300 ms, then it settles.
        for i in 1..=5 {
            h.observe(e(0.0, 1e-3 / (i as f64 + 1.0)), ms(i * 20));
        }
        assert!(!h.can_go_back());
        h.observe(e(0.0, 1e-3 / 6.0), ms(399));
        assert!(!h.can_go_back(), "299 ms after the last change");
        h.observe(e(0.0, 1e-3 / 6.0), ms(400));
        assert!(h.can_go_back(), "recorded once settled");
        assert_eq!(h.back().unwrap(), e(0.0, 1e-3));
        assert!(h.can_go_forward());
        assert_eq!(h.forward().unwrap(), e(0.0, 1e-3 / 6.0));
    }

    #[test]
    fn applying_an_entry_is_not_recorded_and_new_moves_clear_forward() {
        let mut h = ViewHistory::default();
        h.observe(e(0.0, 1e-3), ms(0));
        h.observe(e(1.0, 1e-3), ms(10));
        h.observe(e(1.0, 1e-3), ms(400));
        let prev = h.back().unwrap();
        // The app now shows `prev`: observing it changes nothing.
        h.observe(prev.clone(), ms(500));
        h.observe(prev, ms(900));
        assert!(h.can_go_forward() && !h.can_go_back());
        h.observe(e(2.0, 1e-3), ms(1000));
        h.observe(e(2.0, 1e-3), ms(1400));
        assert!(!h.can_go_forward(), "a new move drops the forward stack");
        assert_eq!(h.back().unwrap(), e(0.0, 1e-3));
    }

    #[test]
    fn back_records_a_pending_change_first() {
        let mut h = ViewHistory::default();
        h.observe(e(0.0, 1e-3), ms(0));
        h.observe(e(5.0, 1e-3), ms(10));
        assert_eq!(h.back().unwrap(), e(0.0, 1e-3));
        assert_eq!(h.forward().unwrap(), e(5.0, 1e-3));
    }
}
