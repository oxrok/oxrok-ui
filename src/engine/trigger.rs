//! The software trigger engine: scans committed
//! digital data incrementally for the trigger event.

use crate::core::ChannelId;
use crate::core::trigger::{DigitalTrigger, TriggerEvent};
use crate::store::DigitalChannel;

pub struct TriggerScanner {
    trigger: DigitalTrigger,
    rate: u64,
    /// Next sample to scan (`trigger_progress`).
    pos: u64,
}

impl TriggerScanner {
    pub fn new(trigger: DigitalTrigger, rate: u64) -> Self {
        Self { trigger, rate, pos: 0 }
    }

    /// Continues the scan from `pos` (e.g. near a hardware trigger).
    pub fn skip_to(&mut self, pos: u64) {
        self.pos = pos;
    }

    /// How far the scan has got.
    pub fn progress(&self) -> u64 {
        self.pos
    }

    /// Scans transitions in `[progress, end)`; returns the trigger sample
    /// of the first match. `channel` looks up captured channels.
    pub fn scan<'a>(&mut self, channel: impl Fn(ChannelId) -> Option<&'a DigitalChannel>, end: u64) -> Option<u64> {
        let id = self.trigger.channel?;
        let ch = channel(id)?;
        let from = self.pos;
        if end <= from {
            return None;
        }
        let linked_ok = |t: u64| {
            self.trigger.linked.iter().all(|(c, level)| channel(*c).and_then(|ch| ch.level_at(t)) == Some(*level))
        };
        let in_width = |a: u64, b: u64| {
            let w = (b - a) as f64 / self.rate as f64;
            let eps = 1e-12 * w.max(1e-12);
            w + eps >= self.trigger.pulse_min && w - eps <= self.trigger.pulse_max
        };
        for t in ch.transitions_from(from) {
            if t >= end {
                break;
            }
            let high = ch.level_at(t) == Some(true);
            let hit = match self.trigger.event {
                TriggerEvent::Rising => high,
                TriggerEvent::Falling => !high,
                // The trigger is the pulse's end: a falling edge ends a
                // high pulse, a rising edge a low one.
                TriggerEvent::PositivePulse => !high && ch.prev_transition(t).is_some_and(|r| in_width(r, t)),
                TriggerEvent::NegativePulse => high && ch.prev_transition(t).is_some_and(|f| in_width(f, t)),
            };
            if hit && linked_ok(t) {
                self.pos = t + 1;
                return Some(t);
            }
        }
        self.pos = end;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chan(initial: bool, transitions: &[u64], end: u64) -> DigitalChannel {
        let mut c = DigitalChannel::new();
        c.start(0, initial);
        for &t in transitions {
            c.push_transition(t);
        }
        c.extend_to(end);
        c
    }

    fn trig(event: TriggerEvent, min: f64, max: f64, linked: Vec<(ChannelId, bool)>) -> DigitalTrigger {
        DigitalTrigger {
            channel: Some(ChannelId::digital(0)),
            event,
            pulse_min: min,
            pulse_max: max,
            linked,
            post_trigger_seconds: 1.0,
            pre_trigger_seconds: 0.0,
        }
    }

    /// U-08: edges, pulses (min/max inclusive), linked conditions; the
    /// trigger time is the pulse end.
    #[test]
    fn u08_trigger_semantics() {
        // D0 at 1 S/s: low, rises 10, falls 13 (3 s high), rises 20, falls 30 (10 s high).
        let d0 = chan(false, &[10, 13, 20, 30], 40);
        // D1 is high from 15 to 25.
        let d1 = chan(false, &[15, 25], 40);
        let get = |c: ChannelId| match c.index {
            0 => Some(&d0),
            1 => Some(&d1),
            _ => None,
        };
        let first = |t: DigitalTrigger| TriggerScanner::new(t, 1).scan(get, 40);
        assert_eq!(first(trig(TriggerEvent::Rising, 0.0, 0.0, vec![])), Some(10));
        assert_eq!(first(trig(TriggerEvent::Falling, 0.0, 0.0, vec![])), Some(13));
        // Linked: D1 high at the edge → the second rising edge.
        assert_eq!(first(trig(TriggerEvent::Rising, 0.0, 0.0, vec![(ChannelId::digital(1), true)])), Some(20));
        // Pulses end the trigger; widths are inclusive.
        assert_eq!(first(trig(TriggerEvent::PositivePulse, 3.0, 3.0, vec![])), Some(13));
        assert_eq!(first(trig(TriggerEvent::PositivePulse, 4.0, 10.0, vec![])), Some(30));
        assert_eq!(first(trig(TriggerEvent::PositivePulse, 4.0, 9.0, vec![])), None);
        // A low pulse 13..20 (7 s): its end is the rise at 20.
        assert_eq!(first(trig(TriggerEvent::NegativePulse, 7.0, 7.0, vec![])), Some(20));
        // Linked conditions at the pulse end: D1 low at 30 (it fell at 25).
        assert_eq!(
            first(trig(TriggerEvent::PositivePulse, 0.0, 100.0, vec![(ChannelId::digital(1), false)])),
            Some(13)
        );
        assert_eq!(first(trig(TriggerEvent::PositivePulse, 5.0, 100.0, vec![(ChannelId::digital(1), true)])), None);
    }

    #[test]
    fn incremental_scan_resumes() {
        let d0 = chan(false, &[10, 13, 20, 30], 40);
        let get = |_| Some(&d0);
        let mut s = TriggerScanner::new(trig(TriggerEvent::PositivePulse, 9.0, 11.0, vec![]), 1);
        assert_eq!(s.scan(get, 25), None);
        assert_eq!(s.progress(), 25);
        // The pulse began before the last scan; it still qualifies.
        assert_eq!(s.scan(get, 40), Some(30));
    }
}
