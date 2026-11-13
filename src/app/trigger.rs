//! From the trigger as it is set up to what evaluates it: the device's
//! own trigger where it takes it, else the app's scanner.

use crate::backend::{HwCombine, HwDuration, HwRange, HwStage, HwTrigger, HwTriggerCaps, TriggerMatch};
use crate::core::trigger::{Combine, DigitalTrigger, Hold, Match, Stage};
use crate::engine::trigger::{Edge, ScanSpec};
use crate::l10n::tr;

/// Why a trigger cannot be used as it is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TriggerError {
    /// No stage has a condition.
    Empty,
    /// The device's trigger has no such match.
    Match(Match),
    Stages,
    Or,
    And,
    Count,
    Consecutive,
    /// A consecutive count counts edges.
    ConsecutiveNeedsEdge,
    Duration,
    Pulse,
    /// A pulse width is that of one rising or falling edge.
    PulseEdge,
    /// Levels can only hold for longer than a time.
    HoldLevels,
    CountAndHold,
    /// No whole number of samples is in the time range.
    Width,
    Range,
    /// The range's bounds, or its channels.
    RangeValues,
    /// The app's scanner looks for one stage of edges and levels.
    NeedsDevice,
}

impl TriggerError {
    pub(crate) fn text(&self) -> String {
        let en = match self {
            TriggerError::Empty => "Set a trigger condition on a channel first.",
            TriggerError::Match(m) => {
                return tr("This device cannot trigger on: {}").replacen("{}", m.label(), 1);
            }
            TriggerError::Stages => "This device has one trigger stage.",
            TriggerError::Or => "This device cannot combine stages with or.",
            TriggerError::And => "This device cannot combine stages with and.",
            TriggerError::Count => "This device's trigger cannot count.",
            TriggerError::Consecutive => "This device's trigger cannot count consecutive matches.",
            TriggerError::ConsecutiveNeedsEdge => "A consecutive count needs an edge in the stage.",
            TriggerError::Duration => "This device's trigger has no durations.",
            TriggerError::Pulse => "This device's trigger has no pulse widths.",
            TriggerError::PulseEdge => "A pulse width needs one rising or falling edge in the stage.",
            TriggerError::HoldLevels => "Levels can only be held for longer than a time.",
            TriggerError::CountAndHold => "A stage has either a count or a duration.",
            TriggerError::Width => "No whole number of samples fits the duration at this sample rate.",
            TriggerError::Range => "This device's trigger has no value ranges.",
            TriggerError::RangeValues => "The value range does not fit its channels.",
            TriggerError::NeedsDevice => "This trigger needs a device with its own trigger.",
        };
        tr(en).to_owned()
    }
}

/// What evaluates a trigger.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Evaluator {
    /// The device's own trigger. The scanner, where it can look for the
    /// same, confirms the position in the data.
    Device(HwTrigger, Option<ScanSpec>),
    /// The app's scanner, on the data as it comes.
    App(ScanSpec),
}

impl Evaluator {
    pub(crate) fn device(&self) -> Option<&HwTrigger> {
        match self {
            Evaluator::Device(hw, _) => Some(hw),
            Evaluator::App(_) => None,
        }
    }

    pub(crate) fn scan(&self) -> Option<&ScanSpec> {
        match self {
            Evaluator::Device(_, spec) => spec.as_ref(),
            Evaluator::App(spec) => Some(spec),
        }
    }
}

/// Slack for the rounding of seconds × rate: a width of exactly the
/// time given counts.
const SLACK: f64 = 1e-9;

/// `hold` in samples at `rate`, as oxrok's durations count them.
fn duration(hold: Hold, rate: u64, edge: bool) -> Result<HwDuration, TriggerError> {
    let rate = rate as f64;
    Ok(match hold {
        // Longer than a time below one sample is any pulse.
        Hold::Longer(s) => {
            let n = (s.max(0.0) * rate * (1.0 + SLACK)).floor() as u64;
            if n == 0 && !edge {
                return Err(TriggerError::Width);
            }
            HwDuration::Longer(n)
        }
        Hold::Shorter(s) => {
            let n = (s.max(0.0) * rate * (1.0 - SLACK)).ceil() as u64;
            if n < 2 {
                return Err(TriggerError::Width);
            }
            HwDuration::Shorter(n)
        }
        Hold::Between(min, max) => {
            let lo = (min.max(0.0) * rate * (1.0 - SLACK)).ceil().max(1.0);
            let hi = (max * rate * (1.0 + SLACK)).floor();
            if lo > hi || hi.is_nan() {
                return Err(TriggerError::Width);
            }
            HwDuration::Between(lo as u64, hi as u64)
        }
    })
}

fn stage(s: &Stage, rate: u64) -> Result<HwStage, TriggerError> {
    let matches = s
        .conditions
        .iter()
        .map(|&(c, m)| {
            let m = match m {
                Match::Rising => TriggerMatch::Rising,
                Match::Falling => TriggerMatch::Falling,
                Match::Edge => TriggerMatch::Edge,
                Match::High => TriggerMatch::One,
                Match::Low => TriggerMatch::Zero,
            };
            (c, m)
        })
        .collect();
    // The first match is what a stage does without a count.
    let count = s.count.filter(|c| c.n > 1 || c.consecutive).map(|c| (c.n.max(1), c.consecutive));
    if count.is_some_and(|c| c.1) && !s.has_edge() {
        return Err(TriggerError::ConsecutiveNeedsEdge);
    }
    let duration = match s.hold {
        None => None,
        Some(_) if count.is_some() => return Err(TriggerError::CountAndHold),
        Some(hold) if s.has_edge() => {
            let mut edges = s.conditions.iter().filter(|(_, m)| m.is_edge());
            let one = edges.next().is_some_and(|(_, m)| *m != Match::Edge) && edges.next().is_none();
            if !one {
                return Err(TriggerError::PulseEdge);
            }
            Some(duration(hold, rate, true)?)
        }
        Some(hold @ Hold::Longer(_)) => Some(duration(hold, rate, false)?),
        Some(_) => return Err(TriggerError::HoldLevels),
    };
    let range = match &s.range {
        None => None,
        Some(r) => {
            let n = r.channels.len();
            let distinct = r.channels.iter().enumerate().all(|(i, c)| !r.channels[..i].contains(c));
            if n == 0 || n > 63 || !distinct || r.low > r.high || r.high >> n != 0 {
                return Err(TriggerError::RangeValues);
            }
            Some(HwRange { channels: r.channels.clone(), low: r.low, high: r.high, inside: r.inside })
        }
    };
    Ok(HwStage { matches, count, duration, range })
}

/// `t` in the device's terms: the stages with a condition, the times in
/// samples at `rate`.
pub(crate) fn lower(t: &DigitalTrigger, rate: u64, pre_trigger_percent: u8) -> Result<HwTrigger, TriggerError> {
    let stages = t.active().map(|s| stage(s, rate)).collect::<Result<Vec<_>, _>>()?;
    if stages.is_empty() {
        return Err(TriggerError::Empty);
    }
    let combine = match t.combine {
        Combine::Or if stages.len() == 2 => HwCombine::Or,
        Combine::And if stages.len() == 2 => HwCombine::And,
        _ => HwCombine::Sequence,
    };
    Ok(HwTrigger { stages, combine, pre_trigger_percent, holdoff: 0 })
}

/// Whether the device's trigger takes `t`.
pub(crate) fn check(caps: &HwTriggerCaps, t: &HwTrigger) -> Result<(), TriggerError> {
    if caps.max_stages > 0 && t.stages.len() > caps.max_stages {
        return Err(TriggerError::Stages);
    }
    match t.combine {
        HwCombine::Or if !caps.or => return Err(TriggerError::Or),
        HwCombine::And if !caps.and => return Err(TriggerError::And),
        _ => {}
    }
    for s in &t.stages {
        if let Some((_, m)) = s.matches.iter().find(|(_, m)| !caps.matches.contains(m)) {
            let m = match m {
                TriggerMatch::Rising => Match::Rising,
                TriggerMatch::Falling => Match::Falling,
                TriggerMatch::Edge => Match::Edge,
                TriggerMatch::One => Match::High,
                TriggerMatch::Zero => Match::Low,
            };
            return Err(TriggerError::Match(m));
        }
        match s.count {
            Some(_) if !caps.count => return Err(TriggerError::Count),
            Some((_, true)) if !caps.consecutive => return Err(TriggerError::Consecutive),
            _ => {}
        }
        if let Some(d) = s.duration {
            let edge = s.matches.iter().any(|(_, m)| !matches!(m, TriggerMatch::Zero | TriggerMatch::One));
            if !caps.duration {
                return Err(TriggerError::Duration);
            }
            if (edge || !matches!(d, HwDuration::Longer(_))) && !caps.pulse {
                return Err(TriggerError::Pulse);
            }
        }
        if s.range.is_some() && !caps.range {
            return Err(TriggerError::Range);
        }
    }
    Ok(())
}

/// What the app's scanner looks for in place of `t`, when it can: one
/// stage of edges and levels, and the width of the pulse an edge ends.
pub(crate) fn scan_spec(t: &HwTrigger) -> Option<ScanSpec> {
    let [s] = t.stages.as_slice() else { return None };
    if s.count.is_some() || s.range.is_some() {
        return None;
    }
    let mut spec = ScanSpec::default();
    for &(c, m) in &s.matches {
        match m {
            TriggerMatch::Rising => spec.edges.push((c, Edge::Rising)),
            TriggerMatch::Falling => spec.edges.push((c, Edge::Falling)),
            TriggerMatch::Edge => spec.edges.push((c, Edge::Either)),
            TriggerMatch::One => spec.levels.push((c, true)),
            TriggerMatch::Zero => spec.levels.push((c, false)),
        }
    }
    if let Some(d) = s.duration {
        // Only a pulse width: how long levels held is the device's.
        let [(_, Edge::Rising | Edge::Falling)] = spec.edges.as_slice() else { return None };
        spec.pulse = Some(d.bounds());
    }
    Some(spec)
}

/// What evaluates `t` on a device whose own trigger is `caps`: that
/// trigger when it takes all of `t`, else the app's scanner when it can
/// look for it; else why neither can.
pub(crate) fn plan(
    t: &DigitalTrigger,
    caps: Option<&HwTriggerCaps>,
    pre_trigger_percent: u8,
    rate: u64,
) -> Result<Evaluator, TriggerError> {
    let mut hw = lower(t, rate, pre_trigger_percent)?;
    let spec = scan_spec(&hw);
    // The time before the trigger is there when the trigger is armed
    // after it: a trigger fires on its first match, which on a running
    // signal is in the first samples.
    if caps.is_some_and(|c| c.holdoff) {
        hw.holdoff = pre_trigger_samples(t, rate);
    }
    let device = match caps {
        Some(c) => check(c, &hw),
        None => Err(TriggerError::NeedsDevice),
    };
    match (device, spec) {
        (Ok(()), spec) => Ok(Evaluator::Device(hw, spec)),
        (Err(_), Some(spec)) => Ok(Evaluator::App(spec)),
        (Err(e), None) => Err(e),
    }
}

/// The samples of the time to keep before the trigger: what passes
/// before the trigger is looked for.
pub(crate) fn pre_trigger_samples(t: &DigitalTrigger, rate: u64) -> u64 {
    (t.pre_trigger_seconds.max(0.0) * rate as f64).ceil() as u64
}

/// The conditions a click on a row's trigger icon steps through: those
/// the device's trigger has, or all of them where the app scans.
pub(crate) fn matches(caps: Option<&HwTriggerCaps>) -> Vec<Match> {
    let Some(hw) = caps else { return Match::CYCLE.to_vec() };
    Match::CYCLE
        .into_iter()
        .filter(|m| {
            hw.matches.contains(&match m {
                Match::Rising => TriggerMatch::Rising,
                Match::Falling => TriggerMatch::Falling,
                Match::Edge => TriggerMatch::Edge,
                Match::High => TriggerMatch::One,
                Match::Low => TriggerMatch::Zero,
            })
        })
        .collect()
}

/// The stages to offer: B too where the device's trigger has it.
pub(crate) fn stages(caps: Option<&HwTriggerCaps>) -> usize {
    caps.map_or(1, HwTriggerCaps::stages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::TriggerMatch::{Falling, One, Rising, Zero};
    use crate::core::ChannelId;
    use crate::core::trigger::{StageCount, ValueRange};

    const D: fn(u16) -> ChannelId = ChannelId::digital;

    fn caps(pulse: bool) -> HwTriggerCaps {
        HwTriggerCaps {
            matches: vec![Zero, One, Rising, Falling],
            capture_ratio: true,
            pulse,
            duration: pulse,
            max_stages: 1,
            ..HwTriggerCaps::default()
        }
    }

    fn full() -> HwTriggerCaps {
        HwTriggerCaps {
            matches: vec![Zero, One, Rising, Falling, TriggerMatch::Edge],
            capture_ratio: true,
            pulse: true,
            max_stages: 0,
            count: true,
            consecutive: true,
            duration: true,
            range: true,
            or: true,
            and: true,
            holdoff: false,
        }
    }

    /// The edge with the levels, and a pulse as the edge that ends it
    /// with the widths in samples: on the device where it takes them,
    /// else in the app.
    #[test]
    fn edges_and_pulses() {
        let rate = 1_000_000;
        let with_levels = |mut t: DigitalTrigger| {
            t.set(0, D(1), Some(Match::High));
            t.set(0, D(2), Some(Match::Low));
            t
        };
        let hw = |edge| HwTrigger::one_stage(vec![(D(0), edge), (D(1), One), (D(2), Zero)], 10);
        for pulse in [false, true] {
            for (m, edge) in [(Match::Rising, Rising), (Match::Falling, Falling)] {
                let got = plan(&with_levels(DigitalTrigger::on(D(0), m)), Some(&caps(pulse)), 10, rate).unwrap();
                assert_eq!(got.device(), Some(&hw(edge)));
                assert!(got.scan().is_some(), "the scanner confirms the position");
            }
        }
        let positive = with_levels(DigitalTrigger::pulse(D(0), true, 2e-6, 5e-6));
        let negative = with_levels(DigitalTrigger::pulse(D(0), false, 2e-6, 5e-6));
        let on = |t: &DigitalTrigger, c: &HwTriggerCaps, rate| plan(t, Some(c), 10, rate);
        assert_eq!(on(&positive, &caps(true), rate).unwrap().device(), Some(&hw(Falling).pulse(2, 5)));
        assert_eq!(on(&negative, &caps(true), rate).unwrap().device(), Some(&hw(Rising).pulse(2, 5)));
        // The widths follow the rate.
        assert_eq!(on(&positive, &caps(true), 4_000_000).unwrap().device(), Some(&hw(Falling).pulse(8, 20)));
        // No pulse widths in the device: the app looks for it.
        let app = on(&positive, &caps(false), rate).unwrap();
        assert_eq!(app.device(), None);
        assert_eq!(app.scan().unwrap().pulse, Some((2, 5)));
        // No whole number of samples between 2.2 and 2.8 µs.
        let narrow = DigitalTrigger::pulse(D(0), true, 2.2e-6, 2.8e-6);
        assert_eq!(on(&narrow, &caps(true), rate), Err(TriggerError::Width));
        // A device without levels leaves the trigger to the app.
        let edges_only = HwTriggerCaps { matches: vec![Rising, Falling], ..caps(true) };
        assert_eq!(
            on(&with_levels(DigitalTrigger::on(D(0), Match::Rising)), &edges_only, rate).unwrap().device(),
            None
        );
        // Nothing set.
        assert_eq!(on(&DigitalTrigger::default(), &caps(true), rate), Err(TriggerError::Empty));
        assert_eq!(plan(&DigitalTrigger::default(), None, 0, rate), Err(TriggerError::Empty));
    }

    #[test]
    fn durations_in_samples() {
        let d = |hold, edge| duration(hold, 1_000_000, edge);
        // The defaults, 1 ms to 10 ms: both ends count.
        assert_eq!(d(Hold::Between(1e-3, 10e-3), true), Ok(HwDuration::Between(1_000, 10_000)));
        assert_eq!(duration(Hold::Between(1e-3, 10e-3), 24_000_000, true), Ok(HwDuration::Between(24_000, 240_000)));
        // Between samples: the whole widths inside.
        assert_eq!(d(Hold::Between(2.5e-6, 4.5e-6), true), Ok(HwDuration::Between(3, 4)));
        // A pulse is at least one sample.
        assert_eq!(d(Hold::Between(0.0, 3e-6), true), Ok(HwDuration::Between(1, 3)));
        // Nothing fits: below a sample, or between two.
        assert_eq!(d(Hold::Between(0.0, 0.4e-6), true), Err(TriggerError::Width));
        assert_eq!(d(Hold::Between(2.2e-6, 2.8e-6), true), Err(TriggerError::Width));
        // Longer than 3 µs is 4 samples and up; shorter than 3 µs is 2 and down.
        assert_eq!(d(Hold::Longer(3e-6), true).map(HwDuration::bounds), Ok((4, u64::MAX)));
        assert_eq!(d(Hold::Shorter(3e-6), true).map(HwDuration::bounds), Ok((1, 2)));
        assert_eq!(d(Hold::Shorter(2.5e-6), true).map(HwDuration::bounds), Ok((1, 2)));
        assert_eq!(d(Hold::Shorter(1e-6), true), Err(TriggerError::Width));
        // Longer than less than a sample: any pulse, but no hold of levels.
        assert_eq!(d(Hold::Longer(0.2e-6), true), Ok(HwDuration::Longer(0)));
        assert_eq!(d(Hold::Longer(0.2e-6), false), Err(TriggerError::Width));
    }

    /// Stages, counts, holds and ranges go to the device that has them;
    /// the app's scanner takes none of them.
    #[test]
    fn extras_need_the_device() {
        let rate = 1_000_000;
        let on = |t: &DigitalTrigger, c: &HwTriggerCaps| plan(t, Some(c), 0, rate);
        // Stage B.
        let mut two = DigitalTrigger::on(D(0), Match::Rising);
        two.set(1, D(1), Some(Match::Falling));
        let got = on(&two, &full()).unwrap();
        assert_eq!(got.device().unwrap().stages.len(), 2);
        assert_eq!(got.scan(), None, "the device's position is the trigger");
        assert_eq!(on(&two, &caps(true)), Err(TriggerError::Stages));
        assert_eq!(plan(&two, None, 0, rate), Err(TriggerError::NeedsDevice));
        // B alone is one stage.
        let mut b = DigitalTrigger::default();
        b.set(1, D(1), Some(Match::Falling));
        assert_eq!(on(&b, &caps(true)).unwrap().device(), Some(&HwTrigger::one_stage(vec![(D(1), Falling)], 0)));
        // Or and and.
        two.combine = Combine::Or;
        assert_eq!(on(&two, &full()).unwrap().device().unwrap().combine, HwCombine::Or);
        assert_eq!(on(&two, &HwTriggerCaps { or: false, ..full() }), Err(TriggerError::Or));
        two.combine = Combine::And;
        assert_eq!(on(&two, &HwTriggerCaps { and: false, ..full() }), Err(TriggerError::And));
        // A count; the first match is no count.
        let mut counted = DigitalTrigger::on(D(0), Match::Rising);
        counted.stages[0].count = Some(StageCount { n: 3, consecutive: false });
        assert_eq!(on(&counted, &full()).unwrap().device().unwrap().stages[0].count, Some((3, false)));
        assert_eq!(on(&counted, &caps(true)), Err(TriggerError::Count));
        counted.stages[0].count = Some(StageCount { n: 3, consecutive: true });
        assert_eq!(on(&counted, &HwTriggerCaps { consecutive: false, ..full() }), Err(TriggerError::Consecutive));
        counted.stages[0].count = Some(StageCount { n: 1, consecutive: false });
        assert_eq!(on(&counted, &caps(true)).unwrap().device().unwrap().stages[0].count, None);
        // Consecutive counts edges.
        let mut levels = DigitalTrigger::on(D(0), Match::High);
        levels.stages[0].count = Some(StageCount { n: 2, consecutive: true });
        assert_eq!(on(&levels, &full()), Err(TriggerError::ConsecutiveNeedsEdge));
        // Levels held for a while; not for less.
        levels.stages[0].count = None;
        levels.stages[0].hold = Some(Hold::Longer(50e-6));
        assert_eq!(on(&levels, &full()).unwrap().device().unwrap().stages[0].duration, Some(HwDuration::Longer(50)));
        assert_eq!(on(&levels, &HwTriggerCaps { duration: false, ..full() }), Err(TriggerError::Duration));
        assert_eq!(plan(&levels, None, 0, rate), Err(TriggerError::NeedsDevice));
        levels.stages[0].hold = Some(Hold::Shorter(50e-6));
        assert_eq!(on(&levels, &full()), Err(TriggerError::HoldLevels));
        // A width is one edge's.
        let mut wide = DigitalTrigger::pulse(D(0), true, 1e-6, 2e-6);
        wide.set(0, D(1), Some(Match::Rising));
        assert_eq!(on(&wide, &full()), Err(TriggerError::PulseEdge));
        let mut either = DigitalTrigger::on(D(0), Match::Edge);
        either.stages[0].hold = Some(Hold::Longer(1e-6));
        assert_eq!(on(&either, &full()), Err(TriggerError::PulseEdge));
        // A count and a hold.
        let mut both = DigitalTrigger::pulse(D(0), true, 1e-6, 2e-6);
        both.stages[0].count = Some(StageCount { n: 2, consecutive: false });
        assert_eq!(on(&both, &full()), Err(TriggerError::CountAndHold));
        // A value range, alone in its stage.
        let mut ranged = DigitalTrigger::default();
        ranged.stages[0].range =
            Some(ValueRange { channels: vec![D(4), D(5), D(6), D(7)], low: 3, high: 9, inside: true });
        assert!(on(&ranged, &full()).unwrap().device().unwrap().stages[0].range.is_some());
        assert_eq!(on(&ranged, &caps(true)), Err(TriggerError::Range));
        ranged.stages[0].range.as_mut().unwrap().high = 16;
        assert_eq!(on(&ranged, &full()), Err(TriggerError::RangeValues));
        // A match the device has not, with a stage only it can do.
        let mut edge = DigitalTrigger::on(D(0), Match::Edge);
        edge.set(1, D(1), Some(Match::High));
        let no_edge = HwTriggerCaps { matches: vec![Zero, One, Rising, Falling], ..full() };
        assert_eq!(on(&edge, &no_edge), Err(TriggerError::Match(Match::Edge)));
    }

    /// What the app's checks pass, oxrok's own check passes.
    #[test]
    fn agrees_with_oxrok() {
        use oxrok::core::TriggerFeatures;
        let rate = 1_000_000;
        let mut triggers = Vec::new();
        for m in Match::CYCLE {
            let mut t = DigitalTrigger::on(D(0), m);
            triggers.push(t.clone());
            t.set(0, D(1), Some(Match::High));
            triggers.push(t.clone());
            for hold in [Hold::Longer(3e-6), Hold::Shorter(3e-6), Hold::Between(2e-6, 5e-6), Hold::Longer(0.0)] {
                let mut h = t.clone();
                h.stages[0].hold = Some(hold);
                triggers.push(h);
            }
            for (n, consecutive) in [(1, false), (4, false), (4, true)] {
                let mut c = t.clone();
                c.stages[0].count = Some(StageCount { n, consecutive });
                triggers.push(c.clone());
                c.stages[0].hold = Some(Hold::Longer(3e-6));
                triggers.push(c);
            }
            let mut r = t.clone();
            r.stages[0].range = Some(ValueRange { channels: vec![D(2), D(3)], low: 1, high: 2, inside: false });
            triggers.push(r.clone());
            for combine in [Combine::Then, Combine::Or, Combine::And] {
                let mut two = r.clone();
                two.set(1, D(4), Some(m));
                two.combine = combine;
                triggers.push(two);
            }
        }
        let mut passed = 0;
        for bits in 0u32..256 {
            let bit = |k: u32| bits >> k & 1 != 0;
            let c = HwTriggerCaps {
                matches: vec![Zero, One, Rising, Falling, TriggerMatch::Edge],
                capture_ratio: true,
                count: bit(0),
                consecutive: bit(0) && bit(1),
                duration: bit(2),
                pulse: bit(2) && bit(3),
                range: bit(4),
                or: bit(5),
                and: bit(6),
                max_stages: usize::from(bit(7)),
                holdoff: false,
            };
            let f = TriggerFeatures {
                count: c.count,
                consecutive: c.consecutive,
                duration: c.duration,
                pulse: c.pulse,
                range: c.range,
                or: c.or,
                and: c.and,
                max_stages: c.max_stages,
                holdoff: c.holdoff,
            };
            for t in &triggers {
                let Ok(hw) = lower(t, rate, 0) else { continue };
                let ox = crate::oxrok::trigger::to_oxrok(&hw, |c| Some(c.index as usize)).unwrap();
                let ours = check(&c, &hw);
                assert_eq!(ours.is_ok(), ox.check(&f).is_ok(), "{t:?} on {c:?}: {ours:?} / {:?}", ox.check(&f));
                passed += usize::from(ours.is_ok());
            }
        }
        assert!(passed > 1000, "{passed}");
        // What `lower` refuses, oxrok refuses with everything on.
        for t in &triggers {
            if let Err(e) = lower(t, rate, 0) {
                assert_ne!(e, TriggerError::Empty, "{t:?}");
            }
        }
    }
}
