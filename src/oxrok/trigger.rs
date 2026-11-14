//! The app's device trigger as oxrok's.

use crate::backend::{HwCombine, HwDuration, HwTrigger, TriggerMatch};
use crate::core::ChannelId;
use oxrok::core::{Trigger, TriggerCombine, TriggerCount, TriggerDuration, TriggerMatch as OxMatch, TriggerRange};

/// `t` with its channels at the positions `pos` gives, the device's
/// channel indices. The error names a channel the device has not.
pub(crate) fn to_oxrok(t: &HwTrigger, pos: impl Fn(ChannelId) -> Option<usize>) -> Result<Trigger, String> {
    let at = |id: ChannelId| pos(id).ok_or_else(|| format!("no channel {id:?} for the trigger"));
    let mut trig = Trigger::new();
    for (n, stage) in t.stages.iter().enumerate() {
        for (id, m) in &stage.matches {
            let m = match m {
                TriggerMatch::Zero => OxMatch::Zero,
                TriggerMatch::One => OxMatch::One,
                TriggerMatch::Rising => OxMatch::Rising,
                TriggerMatch::Falling => OxMatch::Falling,
                TriggerMatch::Edge => OxMatch::Edge,
            };
            trig.add(n, at(*id)?, m);
        }
        if let Some((count, consecutive)) = stage.count {
            trig.set_count(n, TriggerCount { count, consecutive });
        }
        if let Some(d) = stage.duration {
            let d = match d {
                HwDuration::Longer(n) => TriggerDuration::Longer(n),
                HwDuration::Shorter(n) => TriggerDuration::Shorter(n),
                HwDuration::Between(a, b) => TriggerDuration::Between(a, b),
            };
            trig.set_duration(n, d);
        }
        if let Some(r) = &stage.range {
            let channels = r.channels.iter().map(|c| at(*c)).collect::<Result<Vec<_>, _>>()?;
            trig.set_range(n, TriggerRange { channels, low: r.low, high: r.high, inside: r.inside });
        }
    }
    trig.holdoff = t.holdoff;
    trig.combine = match t.combine {
        HwCombine::Sequence => TriggerCombine::Sequence,
        HwCombine::Or => TriggerCombine::Or,
        HwCombine::And => TriggerCombine::And,
    };
    Ok(trig)
}
