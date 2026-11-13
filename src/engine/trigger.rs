//! The software trigger engine: scans committed
//! digital data incrementally for the trigger.

use crate::core::ChannelId;
use crate::store::{DigitalChannel, Transitions};

/// An edge the scanner looks for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Edge {
    Rising,
    Falling,
    Either,
}

impl Edge {
    /// Whether an edge to `high` is this one.
    fn is(self, high: bool) -> bool {
        match self {
            Edge::Rising => high,
            Edge::Falling => !high,
            Edge::Either => true,
        }
    }
}

/// What the scanner can look for: one stage, everything on the same
/// sample. Counts, durations, value ranges and further stages are the
/// device's trigger's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanSpec {
    /// Edges that must fall on the same sample.
    pub edges: Vec<(ChannelId, Edge)>,
    /// Levels on the trigger sample: the sample of the edges, or with no
    /// edge the first sample where they all hold.
    pub levels: Vec<(ChannelId, bool)>,
    /// With one rising or falling edge: the width in samples, both ends
    /// inclusive, of the pulse it ends. A falling edge ends a high pulse,
    /// a rising edge a low one.
    pub pulse: Option<(u64, u64)>,
}

pub struct TriggerScanner {
    spec: ScanSpec,
    /// Next sample to scan (`trigger_progress`).
    pos: u64,
}

impl TriggerScanner {
    pub fn new(spec: ScanSpec) -> Self {
        Self { spec, pos: 0 }
    }

    /// Continues the scan from `pos` (e.g. near a hardware trigger).
    pub fn skip_to(&mut self, pos: u64) {
        self.pos = pos;
    }

    /// How far the scan has got.
    pub fn progress(&self) -> u64 {
        self.pos
    }

    /// Scans `[progress, end)`; returns the trigger sample of the first
    /// match. `channel` looks up captured channels.
    pub fn scan<'a>(&mut self, channel: impl Fn(ChannelId) -> Option<&'a DigitalChannel>, end: u64) -> Option<u64> {
        if end <= self.pos {
            return None;
        }
        let found = match self.spec.edges.split_first() {
            Some((first, more)) => self.scan_edges(*first, more, &channel, end),
            None => self.scan_levels(&channel, end),
        };
        self.pos = found.map_or(end, |t| t + 1);
        found
    }

    /// One pass over the first edge channel's transitions, carrying its
    /// level and the previous transition along, and a cursor per level
    /// channel moved in step: no index search per edge, so a busy line
    /// costs about what reading its edges does.
    fn scan_edges<'a>(
        &self,
        (id, edge): (ChannelId, Edge),
        more: &[(ChannelId, Edge)],
        channel: &impl Fn(ChannelId) -> Option<&'a DigitalChannel>,
        end: u64,
    ) -> Option<u64> {
        let ch = channel(id)?;
        // Trimmed data leaves transitions from before it in the channel;
        // they are no edges any more.
        let from = self.pos.max(ch.data_range().map_or(0, |r| r.0));
        // A level channel that was not captured never holds.
        let mut levels: Vec<(Option<Cursor<'a>>, bool)> =
            self.spec.levels.iter().map(|&(c, level)| (Cursor::new(channel(c), from), level)).collect();
        // Further edges are rare: each is looked up when the first hits.
        let also = |t: u64| {
            more.iter().all(|&(c, e)| {
                channel(c).is_some_and(|ch| {
                    ch.data_range().is_some_and(|r| t >= r.0)
                        && ch.transitions_from(t).next() == Some(t)
                        && ch.level_at(t).is_some_and(|high| e.is(high))
                })
            })
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
            // The trigger is the pulse's end.
            let hit = edge.is(high_now)
                && self.spec.pulse.is_none_or(|(min, max)| prev_now.is_some_and(|p| (min..=max).contains(&(t - p))));
            if hit
                && levels.iter_mut().all(|(c, level)| c.as_mut().and_then(|c| c.level_at(t)) == Some(*level))
                && also(t)
            {
                return Some(t);
            }
        }
        None
    }

    /// The first sample where every level holds: from one that fails,
    /// on to the next transition of the channels that fail.
    fn scan_levels<'a>(&self, channel: &impl Fn(ChannelId) -> Option<&'a DigitalChannel>, end: u64) -> Option<u64> {
        let chans: Vec<(&DigitalChannel, bool)> =
            self.spec.levels.iter().map(|&(c, level)| Some((channel(c)?, level))).collect::<Option<_>>()?;
        // Nothing to look for; data from before every channel began.
        let begin = chans.iter().map(|(ch, _)| ch.data_range().map(|r| r.0)).collect::<Option<Vec<_>>>()?;
        let mut t = self.pos.max(begin.into_iter().max()?);
        while t < end {
            let mut next = t;
            for (ch, level) in &chans {
                if ch.level_at(t)? != *level {
                    next = next.max(ch.next_transition(t)?);
                }
            }
            if next == t {
                return Some(t);
            }
            t = next;
        }
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

    const D: fn(u16) -> ChannelId = ChannelId::digital;

    fn chan(initial: bool, transitions: &[u64], end: u64) -> DigitalChannel {
        let mut c = DigitalChannel::new();
        c.start(0, initial);
        for &t in transitions {
            c.push_transition(t);
        }
        c.extend_to(end);
        c
    }

    fn spec(edge: Edge, pulse: Option<(u64, u64)>, levels: Vec<(ChannelId, bool)>) -> ScanSpec {
        ScanSpec { edges: vec![(D(0), edge)], levels, pulse }
    }

    /// U-08: edges, pulses (min/max inclusive), level conditions; the
    /// trigger sample is the pulse end.
    #[test]
    fn u08_trigger_semantics() {
        // D0: low, rises 10, falls 13 (3 high), rises 20, falls 30 (10 high).
        let d0 = chan(false, &[10, 13, 20, 30], 40);
        // D1 is high from 15 to 25.
        let d1 = chan(false, &[15, 25], 40);
        let get = |c: ChannelId| match c.index {
            0 => Some(&d0),
            1 => Some(&d1),
            _ => None,
        };
        let first = |s: ScanSpec| TriggerScanner::new(s).scan(get, 40);
        assert_eq!(first(spec(Edge::Rising, None, vec![])), Some(10));
        assert_eq!(first(spec(Edge::Falling, None, vec![])), Some(13));
        assert_eq!(first(ScanSpec { edges: vec![(D(1), Edge::Either)], ..ScanSpec::default() }), Some(15));
        // D1 high at the edge → the second rising edge.
        assert_eq!(first(spec(Edge::Rising, None, vec![(D(1), true)])), Some(20));
        // Pulses end the trigger; widths are inclusive.
        assert_eq!(first(spec(Edge::Falling, Some((3, 3)), vec![])), Some(13));
        assert_eq!(first(spec(Edge::Falling, Some((4, 10)), vec![])), Some(30));
        assert_eq!(first(spec(Edge::Falling, Some((4, 9)), vec![])), None);
        // A low pulse 13..20 (7 long): its end is the rise at 20.
        assert_eq!(first(spec(Edge::Rising, Some((7, 7)), vec![])), Some(20));
        // Levels at the pulse end: D1 low at 30 (it fell at 25).
        assert_eq!(first(spec(Edge::Falling, Some((1, 100)), vec![(D(1), false)])), Some(13));
        assert_eq!(first(spec(Edge::Falling, Some((5, 100)), vec![(D(1), true)])), None);
        // A channel that was not captured never matches.
        assert_eq!(first(spec(Edge::Rising, None, vec![(D(7), false)])), None);
        assert_eq!(first(ScanSpec { edges: vec![(D(7), Edge::Rising)], ..ScanSpec::default() }), None);
    }

    /// Edges on two channels match where both change on one sample.
    #[test]
    fn edges_on_several_channels() {
        let d0 = chan(false, &[10, 13, 20, 30], 40);
        let d1 = chan(true, &[13, 20, 22], 40);
        let get = |c: ChannelId| [&d0, &d1].get(c.index as usize).copied();
        let first = |edges: Vec<(ChannelId, Edge)>| {
            TriggerScanner::new(ScanSpec { edges, ..ScanSpec::default() }).scan(get, 40)
        };
        // 13: D0 falls and D1 falls. 20: both rise.
        assert_eq!(first(vec![(D(0), Edge::Falling), (D(1), Edge::Falling)]), Some(13));
        assert_eq!(first(vec![(D(0), Edge::Rising), (D(1), Edge::Rising)]), Some(20));
        assert_eq!(first(vec![(D(0), Edge::Either), (D(1), Edge::Either)]), Some(13));
        assert_eq!(first(vec![(D(0), Edge::Rising), (D(1), Edge::Falling)]), None);
        assert_eq!(first(vec![(D(1), Edge::Falling), (D(0), Edge::Either)]), Some(13));
    }

    /// Levels alone: the first sample where all hold, the very first
    /// one included.
    #[test]
    fn levels_without_an_edge() {
        let d0 = chan(false, &[10, 13, 20, 30], 40);
        let d1 = chan(false, &[15, 25], 40);
        let get = |c: ChannelId| [&d0, &d1].get(c.index as usize).copied();
        let scanner = |levels: Vec<(ChannelId, bool)>| TriggerScanner::new(ScanSpec { levels, ..ScanSpec::default() });
        assert_eq!(scanner(vec![(D(0), false)]).scan(get, 40), Some(0));
        assert_eq!(scanner(vec![(D(0), true)]).scan(get, 40), Some(10));
        assert_eq!(scanner(vec![(D(0), true), (D(1), true)]).scan(get, 40), Some(20));
        assert_eq!(scanner(vec![(D(0), false), (D(1), true)]).scan(get, 40), Some(15));
        assert_eq!(scanner(vec![(D(7), true)]).scan(get, 40), None);
        // In steps: nothing before the data has it, then where it holds.
        let mut s = scanner(vec![(D(0), true), (D(1), true)]);
        assert_eq!((s.scan(get, 12), s.progress()), (None, 12));
        assert_eq!((s.scan(get, 18), s.progress()), (None, 18));
        assert_eq!((s.scan(get, 40), s.progress()), (Some(20), 21));
        // On past a match: it still holds on the next sample.
        assert_eq!(s.scan(get, 40), Some(21));
        // Never both high after 25.
        s.skip_to(25);
        assert_eq!((s.scan(get, 40), s.progress()), (None, 40));
    }

    #[test]
    fn incremental_scan_resumes() {
        let d0 = chan(false, &[10, 13, 20, 30], 40);
        let get = |_| Some(&d0);
        let mut s = TriggerScanner::new(spec(Edge::Falling, Some((9, 11)), vec![]));
        assert_eq!(s.scan(get, 25), None);
        assert_eq!(s.progress(), 25);
        // The pulse began before the last scan; it still qualifies.
        assert_eq!(s.scan(get, 40), Some(30));
    }

    /// The scanner as it was: an index search for every edge's level,
    /// previous transition and the levels. Kept to compare with, and
    /// for the timing below. One change: transitions from before the
    /// stored data are skipped, where it read their level as low.
    pub(super) fn reference<'a>(
        spec: &ScanSpec,
        pos: &mut u64,
        channel: impl Fn(ChannelId) -> Option<&'a DigitalChannel>,
        end: u64,
    ) -> Option<u64> {
        let from = *pos;
        if end <= from {
            return None;
        }
        *pos = end;
        let &(id, edge) = spec.edges.first()?;
        let ch = channel(id)?;
        let levels_ok =
            |t: u64| spec.levels.iter().all(|(c, level)| channel(*c).and_then(|ch| ch.level_at(t)) == Some(*level));
        for t in ch.transitions_from(from) {
            if t >= end {
                break;
            }
            if ch.data_range().is_some_and(|r| t < r.0) {
                continue;
            }
            let high = ch.level_at(t) == Some(true);
            let width_ok = spec
                .pulse
                .is_none_or(|(min, max)| ch.prev_transition(t).is_some_and(|p| (min..=max).contains(&(t - p))));
            if edge.is(high) && width_ok && levels_ok(t) {
                *pos = t + 1;
                return Some(t);
            }
        }
        None
    }

    /// Random channels (some trimmed, some shorter than others, some
    /// missing), random triggers, scanned in random steps and on past
    /// each match: the same matches and progress as the reference.
    #[test]
    fn scans_like_the_reference() {
        use rand::{Rng, SeedableRng, rngs::StdRng};
        let mut rng = StdRng::seed_from_u64(0x5CA7);
        let (mut hits, mut level_hits, mut pulse_hits) = (0, 0, 0);
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
            let min = rng.random_range(0..6u64);
            let spec = ScanSpec {
                edges: vec![(D(0), [Edge::Rising, Edge::Falling, Edge::Either][rng.random_range(0..3)])],
                // Channel 7 was never captured.
                levels: (0..rng.random_range(0..4))
                    .map(|_| (D([1, 2, 3, 4, 7][rng.random_range(0..5)]), rng.random()))
                    .collect(),
                pulse: rng.random_bool(0.5).then(|| (min, min + rng.random_range(0..8u64))),
            };
            let mut scanner = TriggerScanner::new(spec.clone());
            let mut pos = 0;
            let mut end = 0;
            while end < len + 10 {
                end += rng.random_range(1..=len / 3);
                // On past each match, as a repeated capture does.
                loop {
                    let want = reference(&spec, &mut pos, get, end);
                    assert_eq!(scanner.scan(get, end), want, "case {case} to {end}: {spec:?}");
                    assert_eq!(scanner.progress(), pos, "case {case} to {end}");
                    if want.is_none() {
                        break;
                    }
                    hits += 1;
                    level_hits += usize::from(!spec.levels.is_empty());
                    pulse_hits += usize::from(spec.pulse.is_some());
                }
            }
        }
        // Enough of each kind match for the comparison to mean something.
        assert!(hits > 20_000 && level_hits > 2000 && pulse_hits > 2000, "{hits} {level_hits} {pulse_hits}");
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use std::time::Instant;

    /// A trigger on a busy line with level channels must keep up with
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
        let levels: Vec<(ChannelId, bool)> = (1..4).map(|k| (ChannelId::digital(k), true)).collect();
        for (name, edge, pulse) in [
            ("rising + 3 levels", Edge::Rising, None),
            ("pulse + 3 levels", Edge::Falling, Some((24_000_000, 48_000_000))),
        ] {
            let spec = ScanSpec { edges: vec![(ChannelId::digital(0), edge)], levels: levels.clone(), pulse };
            // In the chunks a streaming device delivers.
            let mut s = TriggerScanner::new(spec.clone());
            let t0 = Instant::now();
            for end in (0..=samples).step_by(240 * 1024).skip(1).chain([samples]) {
                assert_eq!(s.scan(get, end), None);
            }
            let secs = t0.elapsed().as_secs_f64();
            // The scanner as it was, over the first 1/16.
            let (mut pos, part) = (0, samples / 16);
            let t0 = Instant::now();
            assert_eq!(super::tests::reference(&spec, &mut pos, get, part), None);
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
