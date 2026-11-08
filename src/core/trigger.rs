//! The digital trigger of Trigger mode.

use crate::core::ChannelId;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerEvent {
    #[default]
    Rising,
    Falling,
    PositivePulse,
    NegativePulse,
}

impl TriggerEvent {
    pub const ALL: [TriggerEvent; 4] =
        [TriggerEvent::Rising, TriggerEvent::Falling, TriggerEvent::PositivePulse, TriggerEvent::NegativePulse];

    pub fn label(self) -> &'static str {
        let en = match self {
            TriggerEvent::Rising => "Rising Edge",
            TriggerEvent::Falling => "Falling Edge",
            TriggerEvent::PositivePulse => "Positive Pulse",
            TriggerEvent::NegativePulse => "Negative Pulse",
        };
        crate::l10n::tr(en)
    }

    pub fn is_pulse(self) -> bool {
        matches!(self, TriggerEvent::PositivePulse | TriggerEvent::NegativePulse)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DigitalTrigger {
    pub channel: Option<ChannelId>,
    pub event: TriggerEvent,
    /// Seconds; pulse events only.
    pub pulse_min: f64,
    pub pulse_max: f64,
    /// Levels other channels must have at the trigger instant.
    pub linked: Vec<(ChannelId, bool)>,
    pub post_trigger_seconds: f64,
    /// Buffered devices only: data to keep before the trigger.
    /// Streaming devices keep everything (subject to the memory policy).
    pub pre_trigger_seconds: f64,
}

/// Defaults.
impl Default for DigitalTrigger {
    fn default() -> Self {
        Self {
            channel: None,
            event: TriggerEvent::Rising,
            pulse_min: 1e-3,
            pulse_max: 10e-3,
            linked: Vec::new(),
            post_trigger_seconds: 1.0,
            pre_trigger_seconds: 0.1,
        }
    }
}

impl DigitalTrigger {
    /// The defaults, on the lowest enabled digital channel.
    pub fn reset(enabled: &[ChannelId]) -> Self {
        Self { channel: lowest_digital(enabled), ..Self::default() }
    }

    /// A channel, enabled, and for pulses `0 ≤ min ≤ max`.
    pub fn is_valid(&self, enabled: &[ChannelId]) -> bool {
        let ch_ok = self.channel.is_some_and(|c| enabled.contains(&c));
        let pulse_ok = !self.event.is_pulse() || (0.0 <= self.pulse_min && self.pulse_min <= self.pulse_max);
        ch_ok && pulse_ok && self.pre_trigger_seconds >= 0.0
    }

    /// A disabled channel moves to the lowest enabled digital one;
    /// linked conditions on disabled channels are dropped.
    pub fn follow_enables(&mut self, enabled: &[ChannelId]) {
        if !self.channel.is_some_and(|c| enabled.contains(&c)) {
            self.channel = lowest_digital(enabled);
        }
        self.linked.retain(|(c, _)| enabled.contains(c) && Some(*c) != self.channel);
    }
}

fn lowest_digital(enabled: &[ChannelId]) -> Option<ChannelId> {
    enabled.iter().copied().filter(|c| c.kind == crate::core::ChannelKind::Digital).min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_validity() {
        let en = [ChannelId::analog(0), ChannelId::digital(3), ChannelId::digital(1)];
        let mut t = DigitalTrigger::reset(&en);
        assert_eq!(
            (t.channel, t.event, t.post_trigger_seconds),
            (Some(ChannelId::digital(1)), TriggerEvent::Rising, 1.0)
        );
        assert!(t.is_valid(&en));
        t.event = TriggerEvent::PositivePulse;
        t.pulse_min = 0.02;
        assert!(!t.is_valid(&en), "min > max");
        t.channel = Some(ChannelId::digital(7));
        t.linked = vec![(ChannelId::digital(3), true), (ChannelId::digital(9), false)];
        t.follow_enables(&en);
        assert_eq!(t.channel, Some(ChannelId::digital(1)));
        assert_eq!(t.linked, [(ChannelId::digital(3), true)]);
        assert_eq!(DigitalTrigger::reset(&[ChannelId::analog(0)]).channel, None);
    }
}
