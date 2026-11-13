//! The software trigger engine: scans committed
//! digital data incrementally for the trigger event.

use crate::core::ChannelId;
use crate::core::trigger::{DigitalTrigger, TriggerEvent};
use crate::store::{DigitalChannel, Transitions};

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
    ///
    /// One pass over the trigger channel's transitions, carrying its
    /// level and the previous transition along, and a cursor per linked
    /// channel moved in step: no index search per edge, so a busy line
    /// costs about what reading its edges does.
    pub fn scan<'a>(&mut self, channel: impl Fn(ChannelId) -> Option<&'a DigitalChannel>, end: u64) -> Option<u64> {
        let id = self.trigger.channel?;
        let ch = channel(id)?;
        if end <= self.pos {
            return None;
        }
        // Trimmed data leaves transitions from before it in the channel;
        // they are no edges any more.
        let from = self.pos.max(ch.data_range().map_or(0, |r| r.0));
        // A linked channel that was not captured never holds.
        let mut linked: Vec<(Option<Cursor<'a>>, bool)> =
            self.trigger.linked.iter().map(|&(c, level)| (Cursor::new(channel(c), from), level)).collect();
        let in_width = |a: u64, b: u64| {
            let w = (b - a) as f64 / self.rate as f64;
            let eps = 1e-12 * w.max(1e-12);
            w + eps >= self.trigger.pulse_min && w - eps <= self.trigger.pulse_max
        };
        // The level after each transition flips, and each one is the
        // previous of the next; only the first needs a search.
        let mut high: Option<bool> = None;
        let mut prev: Option<u64> = None;
        for t in ch.transitions_from(from) {
            if t >= end {
                break;
            }
            let (high_now, prev_now) = match high {
                Some(h) => (!h, prev),
                None => (ch.level_at(t) == Some(true), ch.prev_transition(t)),
            };
            (high, prev) = (Some(high_now), Some(t));
            let hit = match self.trigger.event {
                TriggerEvent::Rising => high_now,
                TriggerEvent::Falling => !high_now,
                // The trigger is the pulse's end: a falling edge ends a
                // high pulse, a rising edge a low one.
                TriggerEvent::PositivePulse => !high_now && prev_now.is_some_and(|r| in_width(r, t)),
                TriggerEvent::NegativePulse => high_now && prev_now.is_some_and(|f| in_width(f, t)),
            };
            if hit && linked.iter_mut().all(|(c, level)| c.as_mut().and_then(|c| c.level_at(t)) == Some(*level)) {
                self.pos = t + 1;
                return Some(t);
            }
        }
        self.pos = end;
        None
    }
}

/// A channel's level, read at increasing samples: the level at the
/// start and its transitions after it, taken as the samples pass.
struct Cursor<'a> {
    ch: Option<&'a DigitalChannel>,
    edges: Transitions<'a>,
    /// The next transition, `u64::MAX` after the last.
    next: u64,
    /// The level before `next`.
    level: bool,
    /// The end of the stored data; 0 when the scan began outside it,
    /// and every level is looked up.
    end: u64,
}

impl<'a> Cursor<'a> {
    fn new(ch: Option<&'a DigitalChannel>, from: u64) -> Option<Self> {
        let ch = ch?;
        let (level, end) = match (ch.level_at(from), ch.data_range()) {
            (Some(level), Some((_, end))) => (level, end),
            _ => (false, 0),
        };
        // `level` already counts a transition at `from` itself.
        let mut edges = ch.transitions_from(from + 1);
        let next = edges.next().unwrap_or(u64::MAX);
        Some(Cursor { ch: Some(ch), edges, next, level, end })
    }

    /// The level at `t`, which must not go backwards between calls.
    #[inline]
    fn level_at(&mut self, t: u64) -> Option<bool> {
        if t >= self.end {
            // Past the stored data, or it began outside it: look it up.
            return self.ch.and_then(|c| c.level_at(t));
        }
        while self.next <= t {
            self.level = !self.level;
            self.next = self.edges.next().unwrap_or(u64::MAX);
        }
        Some(self.level)
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

    /// The scanner as it was: an index search for every edge's level,
    /// previous transition and linked levels. Kept to compare with, and
    /// for the timing below. One change: transitions from before the
    /// stored data are skipped, where it read their level as low.
    pub(super) fn reference<'a>(
        trigger: &DigitalTrigger,
        rate: u64,
        pos: &mut u64,
        channel: impl Fn(ChannelId) -> Option<&'a DigitalChannel>,
        end: u64,
    ) -> Option<u64> {
        let ch = channel(trigger.channel?)?;
        let from = *pos;
        if end <= from {
            return None;
        }
        let linked_ok =
            |t: u64| trigger.linked.iter().all(|(c, level)| channel(*c).and_then(|ch| ch.level_at(t)) == Some(*level));
        let in_width = |a: u64, b: u64| {
            let w = (b - a) as f64 / rate as f64;
            let eps = 1e-12 * w.max(1e-12);
            w + eps >= trigger.pulse_min && w - eps <= trigger.pulse_max
        };
        for t in ch.transitions_from(from) {
            if t >= end {
                break;
            }
            if ch.data_range().is_some_and(|r| t < r.0) {
                continue;
            }
            let high = ch.level_at(t) == Some(true);
            let hit = match trigger.event {
                TriggerEvent::Rising => high,
                TriggerEvent::Falling => !high,
                TriggerEvent::PositivePulse => !high && ch.prev_transition(t).is_some_and(|r| in_width(r, t)),
                TriggerEvent::NegativePulse => high && ch.prev_transition(t).is_some_and(|f| in_width(f, t)),
            };
            if hit && linked_ok(t) {
                *pos = t + 1;
                return Some(t);
            }
        }
        *pos = end;
        None
    }

    /// Random channels (some trimmed, some shorter than others, some
    /// missing), random triggers, scanned in random steps and on past
    /// each match: the same matches and progress as the reference.
    #[test]
    fn scans_like_the_reference() {
        use rand::{Rng, SeedableRng, rngs::StdRng};
        let mut rng = StdRng::seed_from_u64(0x5CA7);
        let (mut hits, mut linked_hits, mut pulse_hits) = (0, 0, 0);
        for case in 0..3000 {
            // A few long ones, past a chunk of transitions.
            let len: u64 = if case % 1000 == 0 { 200_000 } else { rng.random_range(20..400) };
            let chans: Vec<Option<DigitalChannel>> = (0..5)
                .map(|k| {
                    if rng.random_bool(if k == 0 { 0.02 } else { 0.1 }) {
                        return None;
                    }
                    let begin = if rng.random_bool(0.2) { rng.random_range(0..len / 4) } else { 0 };
                    let end = if rng.random_bool(0.2) { rng.random_range(begin + 1..=len) } else { len };
                    let gap = [1, 2, 5, 40][rng.random_range(0..4)];
                    let mut c = DigitalChannel::new();
                    c.start(begin, rng.random());
                    let mut t = begin;
                    loop {
                        t += rng.random_range(1..=gap);
                        if t >= end {
                            break;
                        }
                        c.push_transition(t);
                    }
                    c.extend_to(end);
                    if rng.random_bool(0.2) {
                        c.trim_before(rng.random_range(begin..end));
                    }
                    Some(c)
                })
                .collect();
            let get = |c: ChannelId| chans.get(c.index as usize).and_then(Option::as_ref);
            let min = f64::from(rng.random_range(0..6u32));
            let trigger = DigitalTrigger {
                channel: rng.random_bool(0.97).then(|| ChannelId::digital(0)),
                event: [
                    TriggerEvent::Rising,
                    TriggerEvent::Falling,
                    TriggerEvent::PositivePulse,
                    TriggerEvent::NegativePulse,
                ][rng.random_range(0..4)],
                pulse_min: min,
                pulse_max: min + f64::from(rng.random_range(0..8u32)),
                // Channel 7 was never captured.
                linked: (0..rng.random_range(0..4))
                    .map(|_| (ChannelId::digital([1, 2, 3, 4, 7][rng.random_range(0..5)]), rng.random()))
                    .collect(),
                post_trigger_seconds: 1.0,
                pre_trigger_seconds: 0.0,
            };
            let mut scanner = TriggerScanner::new(trigger.clone(), 1);
            let mut pos = 0;
            let mut end = 0;
            while end < len + 10 {
                end += rng.random_range(1..=len / 3);
                // On past each match, as a repeated capture does.
                loop {
                    let want = reference(&trigger, 1, &mut pos, get, end);
                    assert_eq!(scanner.scan(get, end), want, "case {case} to {end}: {trigger:?}");
                    assert_eq!(scanner.progress(), pos, "case {case} to {end}");
                    if want.is_none() {
                        break;
                    }
                    hits += 1;
                    linked_hits += usize::from(!trigger.linked.is_empty());
                    pulse_hits +=
                        usize::from(matches!(trigger.event, TriggerEvent::PositivePulse | TriggerEvent::NegativePulse));
                }
            }
        }
        // Enough of each kind match for the comparison to mean something.
        assert!(hits > 20_000 && linked_hits > 2000 && pulse_hits > 2000, "{hits} {linked_hits} {pulse_hits}");
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use std::time::Instant;

    /// A trigger on a busy line with linked channels must keep up with
    /// a fast device: 64 Mi samples at 24 MS/s is 2.8 s of fx2lafw, and
    /// a 6 MHz clock there is 32 Mi edges. Run with
    /// `cargo test --release --lib -- --ignored perf`.
    #[test]
    #[ignore = "timing; run in release"]
    fn busy_line_scan_budget() {
        let samples = 64u64 << 20;
        let clock = |period: u64| {
            let mut c = DigitalChannel::new();
            c.start(0, false);
            for t in (period..samples).step_by(period as usize) {
                c.push_transition(t);
            }
            c.extend_to(samples);
            c
        };
        // D0 toggles every 2 samples, D1 every 3, D2 every 1000; D3
        // stays low, so a trigger that needs it high scans everything.
        let chans = [clock(2), clock(3), clock(1000), clock(samples)];
        let get = |c: ChannelId| chans.get(c.index as usize);
        let edges = chans[0].transition_count() + chans[1].transition_count() + chans[2].transition_count();
        let linked = vec![(ChannelId::digital(1), true), (ChannelId::digital(2), true), (ChannelId::digital(3), true)];
        for (name, event, min, max) in [
            ("rising + 3 linked", TriggerEvent::Rising, 0.0, 0.0),
            ("pulse + 3 linked", TriggerEvent::PositivePulse, 1.0, 2.0),
        ] {
            let trigger = DigitalTrigger {
                channel: Some(ChannelId::digital(0)),
                event,
                pulse_min: min,
                pulse_max: max,
                linked: linked.clone(),
                post_trigger_seconds: 1.0,
                pre_trigger_seconds: 0.0,
            };
            // In the chunks a streaming device delivers.
            let mut s = TriggerScanner::new(trigger.clone(), 24_000_000);
            let t0 = Instant::now();
            for end in (0..=samples).step_by(240 * 1024).skip(1).chain([samples]) {
                assert_eq!(s.scan(get, end), None);
            }
            let secs = t0.elapsed().as_secs_f64();
            // The scanner as it was, over the first 1/16.
            let (mut pos, part) = (0, samples / 16);
            let t0 = Instant::now();
            assert_eq!(super::tests::reference(&trigger, 24_000_000, &mut pos, get, part), None);
            let before = t0.elapsed().as_secs_f64() * 16.0;
            eprintln!(
                "{name}: {secs:.3} s for {} Mi samples, {:.0} M edges/s over all four channels ({before:.1} s before)",
                samples >> 20,
                edges as f64 / secs / 1e6
            );
            // 2.8 s of device time scanned in well under 1 s, on a slow core too.
            assert!(secs <= 1.0, "{name} took {secs:.3} s");
        }
    }
}
