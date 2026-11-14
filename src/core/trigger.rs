//! The digital trigger of Trigger mode.

use crate::core::ChannelId;
use serde::{Deserialize, Serialize};

/// What a channel must do for a stage to match.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Match {
    Rising,
    Falling,
    /// Either edge.
    Edge,
    High,
    Low,
}

impl Match {
    /// The order a click on a row's trigger icon steps through.
    pub const CYCLE: [Match; 5] = [Match::Rising, Match::Falling, Match::Edge, Match::High, Match::Low];

    pub fn is_edge(self) -> bool {
        matches!(self, Match::Rising | Match::Falling | Match::Edge)
    }

    pub fn label(self) -> &'static str {
        let en = match self {
            Match::Rising => "Rising Edge",
            Match::Falling => "Falling Edge",
            Match::Edge => "Either Edge",
            Match::High => "High",
            Match::Low => "Low",
        };
        crate::l10n::tr(en)
    }

    /// The match after `cur` in [`Match::CYCLE`], keeping to `allowed`;
    /// `None` (no condition) after the last.
    pub fn next(cur: Option<Match>, allowed: &[Match]) -> Option<Match> {
        let from = cur.and_then(|c| Match::CYCLE.iter().position(|m| *m == c)).map_or(0, |k| k + 1);
        Match::CYCLE[from..].iter().copied().find(|m| allowed.contains(m))
    }
}

/// Fire on the Nth time a stage matches instead of the first.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCount {
    pub n: u32,
    /// Only matches in a row count.
    pub consecutive: bool,
}

/// How long a stage must hold, in seconds. On a stage with one rising
/// or falling edge it is the width of the pulse that edge ends (a
/// falling edge ends a high pulse, a rising edge a low one); on a stage
/// of levels only `Longer` applies.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Hold {
    Longer(f64),
    Shorter(f64),
    /// Both ends inclusive.
    Between(f64, f64),
}

/// A group of channels read as a number, within or outside a range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueRange {
    /// Least significant first.
    pub channels: Vec<ChannelId>,
    /// Both bounds inclusive.
    pub low: u64,
    pub high: u64,
    pub inside: bool,
}

/// One stage: every condition must hold on the same sample.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stage {
    /// At most one per channel, in channel order.
    pub conditions: Vec<(ChannelId, Match)>,
    pub count: Option<StageCount>,
    pub hold: Option<Hold>,
    pub range: Option<ValueRange>,
}

impl Stage {
    /// Nothing to match on.
    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty() && self.range.is_none()
    }

    pub fn get(&self, ch: ChannelId) -> Option<Match> {
        self.conditions.iter().find(|(c, _)| *c == ch).map(|x| x.1)
    }

    pub fn set(&mut self, ch: ChannelId, m: Option<Match>) {
        self.conditions.retain(|(c, _)| *c != ch);
        if let Some(m) = m {
            self.conditions.push((ch, m));
            self.conditions.sort_by_key(|(c, _)| *c);
        }
        self.tidy();
    }

    pub fn has_edge(&self) -> bool {
        self.conditions.iter().any(|(_, m)| m.is_edge())
    }

    /// The edge that ends the pulse, when the hold is a pulse width.
    pub fn pulse_edge(&self) -> Option<(ChannelId, Match)> {
        self.hold?;
        self.conditions.iter().copied().find(|(_, m)| m.is_edge())
    }

    /// Extras mean nothing without a condition.
    fn tidy(&mut self) {
        if self.is_empty() {
            *self = Stage::default();
        }
    }
}

/// How stage B follows stage A.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Combine {
    /// A, then B.
    #[default]
    Then,
    Or,
    /// Both, in any order.
    And,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DigitalTrigger {
    /// Stage A and stage B; an empty stage is not part of the trigger.
    pub stages: [Stage; 2],
    pub combine: Combine,
    pub post_trigger_seconds: f64,
    /// Data to keep before the trigger, where the device waits for it:
    /// a buffered device, or one whose own trigger takes this one. A
    /// streaming device the app scans keeps everything (subject to the
    /// memory policy).
    pub pre_trigger_seconds: f64,
}

/// Defaults: no condition.
impl Default for DigitalTrigger {
    fn default() -> Self {
        Self { stages: Default::default(), combine: Combine::Then, post_trigger_seconds: 1.0, pre_trigger_seconds: 0.1 }
    }
}

impl DigitalTrigger {
    /// The defaults, with a rising edge on the lowest enabled digital
    /// channel.
    pub fn reset(enabled: &[ChannelId]) -> Self {
        let mut t = Self::default();
        if let Some(ch) = lowest_digital(enabled) {
            t.stages[0].set(ch, Some(Match::Rising));
        }
        t
    }

    /// One condition in stage A.
    pub fn on(ch: ChannelId, m: Match) -> Self {
        let mut t = Self::default();
        t.stages[0].set(ch, Some(m));
        t
    }

    /// The edge that ends a pulse of `min` to `max` seconds on `ch`: a
    /// high pulse when `positive`.
    pub fn pulse(ch: ChannelId, positive: bool, min: f64, max: f64) -> Self {
        let mut t = Self::on(ch, if positive { Match::Falling } else { Match::Rising });
        t.stages[0].hold = Some(Hold::Between(min, max));
        t
    }

    /// No stage has a condition.
    pub fn is_empty(&self) -> bool {
        self.stages.iter().all(Stage::is_empty)
    }

    pub fn get(&self, stage: usize, ch: ChannelId) -> Option<Match> {
        self.stages.get(stage)?.get(ch)
    }

    pub fn set(&mut self, stage: usize, ch: ChannelId, m: Option<Match>) {
        if let Some(s) = self.stages.get_mut(stage) {
            s.set(ch, m);
        }
    }

    /// The stages with something to match, in order.
    pub fn active(&self) -> impl Iterator<Item = &Stage> {
        self.stages.iter().filter(|s| !s.is_empty())
    }

    /// Conditions on disabled channels are dropped, and a value range
    /// that lost a channel.
    pub fn follow_enables(&mut self, enabled: &[ChannelId]) {
        for s in &mut self.stages {
            s.conditions.retain(|(c, _)| enabled.contains(c));
            if s.range.as_ref().is_some_and(|r| r.channels.iter().any(|c| !enabled.contains(c))) {
                s.range = None;
            }
            s.tidy();
        }
    }
}

fn lowest_digital(enabled: &[ChannelId]) -> Option<ChannelId> {
    enabled.iter().copied().filter(|c| c.kind == crate::core::ChannelKind::Digital).min()
}

/// The trigger as files and settings hold it: the stages, or the one
/// event channel with linked levels of earlier versions.
#[derive(Deserialize)]
#[serde(default)]
struct TriggerRepr {
    stages: Option<[Stage; 2]>,
    combine: Combine,
    post_trigger_seconds: f64,
    pre_trigger_seconds: f64,
    channel: Option<ChannelId>,
    event: LegacyEvent,
    pulse_min: f64,
    pulse_max: f64,
    linked: Vec<(ChannelId, bool)>,
}

#[derive(Copy, Clone, Default, Deserialize)]
enum LegacyEvent {
    #[default]
    Rising,
    Falling,
    PositivePulse,
    NegativePulse,
}

impl Default for TriggerRepr {
    fn default() -> Self {
        let t = DigitalTrigger::default();
        Self {
            stages: None,
            combine: t.combine,
            post_trigger_seconds: t.post_trigger_seconds,
            pre_trigger_seconds: t.pre_trigger_seconds,
            channel: None,
            event: LegacyEvent::Rising,
            pulse_min: 1e-3,
            pulse_max: 10e-3,
            linked: Vec::new(),
        }
    }
}

impl From<TriggerRepr> for DigitalTrigger {
    fn from(r: TriggerRepr) -> Self {
        let stages = r.stages.unwrap_or_else(|| {
            let mut a = Stage::default();
            // Without its event channel the old trigger was not valid.
            if let Some(ch) = r.channel {
                // A pulse is the edge that ends it.
                let (edge, pulse) = match r.event {
                    LegacyEvent::Rising => (Match::Rising, false),
                    LegacyEvent::Falling => (Match::Falling, false),
                    LegacyEvent::PositivePulse => (Match::Falling, true),
                    LegacyEvent::NegativePulse => (Match::Rising, true),
                };
                a.set(ch, Some(edge));
                for (c, high) in r.linked {
                    if c != ch {
                        a.set(c, Some(if high { Match::High } else { Match::Low }));
                    }
                }
                a.hold = pulse.then_some(Hold::Between(r.pulse_min, r.pulse_max));
            }
            [a, Stage::default()]
        });
        Self {
            stages,
            combine: r.combine,
            post_trigger_seconds: r.post_trigger_seconds,
            pre_trigger_seconds: r.pre_trigger_seconds,
        }
    }
}

impl<'de> Deserialize<'de> for DigitalTrigger {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        TriggerRepr::deserialize(d).map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: fn(u16) -> ChannelId = ChannelId::digital;

    #[test]
    fn defaults_and_enables() {
        let en = [ChannelId::analog(0), D(3), D(1)];
        assert!(DigitalTrigger::default().is_empty());
        let mut t = DigitalTrigger::reset(&en);
        assert_eq!((t.get(0, D(1)), t.post_trigger_seconds), (Some(Match::Rising), 1.0), "the lowest enabled channel");
        assert!(DigitalTrigger::reset(&[ChannelId::analog(0)]).is_empty());
        // Conditions stay in channel order, one per channel.
        t.set(0, D(3), Some(Match::High));
        t.set(0, D(1), Some(Match::Falling));
        assert_eq!(t.stages[0].conditions, [(D(1), Match::Falling), (D(3), Match::High)]);
        // Disabled channels lose their conditions; an emptied stage its extras.
        t.set(1, D(7), Some(Match::Edge));
        t.stages[1].count = Some(StageCount { n: 3, consecutive: false });
        t.stages[0].range = Some(ValueRange { channels: vec![D(1), D(9)], low: 0, high: 1, inside: true });
        t.follow_enables(&en);
        assert_eq!(t.stages[1], Stage::default());
        assert_eq!(t.stages[0].range, None);
        assert_eq!(t.active().count(), 1);
        t.set(0, D(1), None);
        t.set(0, D(3), None);
        assert!(t.is_empty());
    }

    #[test]
    fn click_cycle() {
        let all = Match::CYCLE;
        let mut seen = Vec::new();
        let mut cur = None;
        loop {
            cur = Match::next(cur, &all);
            let Some(m) = cur else { break };
            seen.push(m);
        }
        assert_eq!(seen, all);
        // A device without "either edge" skips it.
        let some = [Match::Rising, Match::Falling, Match::High, Match::Low];
        assert_eq!(Match::next(Some(Match::Falling), &some), Some(Match::High));
        assert_eq!(Match::next(Some(Match::Low), &some), None);
        assert_eq!(Match::next(None, &[]), None);
    }

    /// Settings and files of earlier versions: one event channel, its
    /// pulse widths and the linked levels.
    #[test]
    fn reads_the_earlier_shape() {
        let old = |event: &str| {
            format!(
                r#"{{"channel":{{"kind":"Digital","index":2}},"event":"{event}","pulse_min":0.002,"pulse_max":0.005,
                    "linked":[[{{"kind":"Digital","index":5}},true],[{{"kind":"Digital","index":0}},false]],
                    "post_trigger_seconds":0.25,"pre_trigger_seconds":0.5}}"#
            )
        };
        let read = |event: &str| serde_json::from_str::<DigitalTrigger>(&old(event)).unwrap();
        let levels = |edge| vec![(D(0), Match::Low), (D(2), edge), (D(5), Match::High)];
        for (event, edge, hold) in [
            ("Rising", Match::Rising, None),
            ("Falling", Match::Falling, None),
            ("PositivePulse", Match::Falling, Some(Hold::Between(0.002, 0.005))),
            ("NegativePulse", Match::Rising, Some(Hold::Between(0.002, 0.005))),
        ] {
            let t = read(event);
            assert_eq!(t.stages[0].conditions, levels(edge), "{event}");
            assert_eq!(t.stages[0].hold, hold, "{event}");
            assert!(t.stages[1].is_empty());
            assert_eq!((t.post_trigger_seconds, t.pre_trigger_seconds), (0.25, 0.5));
        }
        // No event channel: nothing to trigger on.
        assert!(serde_json::from_str::<DigitalTrigger>(r#"{"channel":null,"event":"Falling"}"#).unwrap().is_empty());
        assert_eq!(serde_json::from_str::<DigitalTrigger>("{}").unwrap(), DigitalTrigger::default());
        // What is written now reads back as it was.
        let mut t = DigitalTrigger::pulse(D(1), true, 1e-3, 2e-3);
        t.set(1, D(4), Some(Match::Edge));
        t.stages[1].count = Some(StageCount { n: 3, consecutive: true });
        t.combine = Combine::Or;
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<DigitalTrigger>(&json).unwrap(), t);
    }
}
