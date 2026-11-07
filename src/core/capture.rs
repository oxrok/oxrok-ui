//! Capture domain types, sample-rate menus and
//! the memory policy.

use crate::core::channel::ChannelId;
use crate::core::device::{DeviceOptions, ThresholdChoice};
use serde::{Deserialize, Serialize};

/// Capture state machine.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureState {
    #[default]
    Empty,
    Starting,
    Recording,
    Stopping,
    Finished,
}

impl CaptureState {
    pub fn is_recording(self) -> bool {
        self == CaptureState::Recording
    }

    /// Starting, Recording or Stopping.
    pub fn is_busy(self) -> bool {
        matches!(self, CaptureState::Starting | CaptureState::Recording | CaptureState::Stopping)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureMode {
    /// "Continuous": keeps the newest data within the memory buffer.
    #[default]
    Continuous,
    Timed,
    Triggered,
}

/// Memory buffer limits, in MB.
pub const BUFFER_MB_DEFAULT: u32 = 4096;
pub const BUFFER_MB_MIN: u32 = 512;
pub const BUFFER_MB_STEP: u32 = 512;

/// Settings for the next capture (the subset used so far).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureSettings {
    pub mode: CaptureMode,
    pub sample_rate: u64,
    pub enabled: Vec<ChannelId>,
    pub buffer_mb: u32,
    /// Timer mode duration, seconds.
    pub timer_seconds: f64,
    /// Device compression, when supported.
    pub rle: bool,
    /// 14 the trigger of Trigger mode.
    #[serde(default)]
    pub trigger: crate::core::trigger::DigitalTrigger,
    /// `None` until the device reports its current threshold.
    #[serde(default)]
    pub logic_threshold: Option<ThresholdChoice>,
    /// Options that differ from the driver's defaults.
    #[serde(default)]
    pub device_options: DeviceOptions,
    /// re-arm after each Trigger capture. Saved with a
    /// capture; a new session starts with it off.
    #[serde(default)]
    pub repeat: bool,
}

/// Timed mode defaults.
pub const TIMER_SECONDS_DEFAULT: f64 = 1.0;
pub const TIMER_SECONDS_MIN: f64 = 0.1;

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            mode: CaptureMode::Continuous,
            sample_rate: 0,
            enabled: Vec::new(),
            buffer_mb: BUFFER_MB_DEFAULT,
            timer_seconds: TIMER_SECONDS_DEFAULT,
            rle: true,
            trigger: Default::default(),
            logic_threshold: None,
            device_options: DeviceOptions::new(),
            repeat: false,
        }
    }
}

impl CaptureSettings {
    /// Repeat applies (Continuous never ends by itself).
    pub fn repeats(&self) -> bool {
        self.repeat && self.mode == CaptureMode::Triggered
    }

    pub fn is_enabled(&self, id: ChannelId) -> bool {
        self.enabled.contains(&id)
    }

    /// Toggles a channel, keeping `enabled` sorted.
    pub fn set_enabled(&mut self, id: ChannelId, on: bool) {
        self.enabled.retain(|c| *c != id);
        if on {
            self.enabled.push(id);
            self.enabled.sort();
        }
    }
}

/// Settings latched when a capture starts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureSnapshot {
    pub mode: CaptureMode,
    pub sample_rate: u64,
    pub enabled: Vec<ChannelId>,
    pub buffer_mb: u32,
    pub timer_seconds: f64,
    /// Compression was on for this capture.
    #[serde(default)]
    pub rle: bool,
    /// Trigger mode: the trigger (latching).
    #[serde(default)]
    pub trigger: Option<crate::core::trigger::DigitalTrigger>,
    /// The threshold the device accepted.
    #[serde(default)]
    pub logic_threshold: Option<ThresholdChoice>,
    /// The options read back after applying, where they differ
    /// from the driver's defaults.
    #[serde(default)]
    pub device_options: DeviceOptions,
}

/// A 1-2-5 sequence inside `[min, max]`, highest first.
pub fn rates_from_range(min: u64, max: u64, step: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let mut decade = 1u64;
    while decade <= max {
        for m in [1, 2, 5] {
            let r = decade.saturating_mul(m);
            let on_step = step <= 1 || (r - min.min(r)) % step == 0;
            if r >= min && r <= max && on_step {
                out.push(r);
            }
        }
        decade = match decade.checked_mul(10) {
            Some(d) => d,
            None => break,
        };
    }
    if !out.contains(&max) && max >= min && max > 0 {
        out.push(max);
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
    out.dedup();
    out
}

/// The default rate is the highest one ≤ 10 MS/s (or the lowest available).
pub fn default_sample_rate(rates: &[u64]) -> Option<u64> {
    const CAP: u64 = 10_000_000;
    rates.iter().copied().filter(|&r| r <= CAP).max().or_else(|| rates.iter().copied().min())
}

/// Why the memory policy stops a capture.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    BufferFull,
    Backlog,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MemoryDecision {
    Keep,
    Stop(StopReason),
    /// Discard everything before `processed_end − keep_secs`.
    Trim {
        keep_secs: f64,
    },
}

/// Never trim below this much data.
pub const MIN_KEEP_SECS: f64 = 0.1;

/// Inputs of the memory policy (notation).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MemoryUse {
    /// `P`: processed (stored) MB.
    pub processed_mb: f64,
    /// `L`: backlog MB.
    pub backlog_mb: f64,
    /// `B`: buffer size, MB.
    pub buffer_mb: f64,
    /// Duration of the processed interval.
    pub processed_secs: f64,
}

/// The trigger's state for the memory policy (Triggered mode).
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TriggerProgress {
    /// Found at this many seconds after the processed start.
    Found { at_secs: f64 },
    /// Still searching; this much processed data is not scanned yet.
    Searching { unsearched_secs: f64 },
}

/// TRIM: how much to keep, newest first.
fn trim_keep(m: &MemoryUse) -> Option<f64> {
    if m.processed_mb <= 0.0 || m.processed_secs <= 0.0 {
        return None;
    }
    let sec_per_mb = m.processed_secs / m.processed_mb;
    let target_mb = (m.buffer_mb - m.backlog_mb - MIN_KEEP_SECS / sec_per_mb).max(0.0);
    Some((target_mb * sec_per_mb).max(MIN_KEEP_SECS))
}

/// The memory policy, every mode.
pub fn memory_policy(mode: CaptureMode, m: MemoryUse, trigger: Option<TriggerProgress>) -> MemoryDecision {
    if m.processed_mb + m.backlog_mb <= m.buffer_mb {
        return MemoryDecision::Keep;
    }
    let trim = |keep: Option<f64>| keep.map_or(MemoryDecision::Keep, |keep_secs| MemoryDecision::Trim { keep_secs });
    match mode {
        CaptureMode::Timed => MemoryDecision::Stop(StopReason::BufferFull),
        CaptureMode::Continuous if m.backlog_mb > 0.9 * m.buffer_mb => MemoryDecision::Stop(StopReason::Backlog),
        CaptureMode::Continuous => trim(trim_keep(&m)),
        CaptureMode::Triggered => match trigger {
            // Never trim past the trigger; stop once there.
            Some(TriggerProgress::Found { at_secs }) => {
                if at_secs <= 0.0 {
                    return MemoryDecision::Stop(StopReason::BufferFull);
                }
                trim(trim_keep(&m).map(|k| k.max(m.processed_secs - at_secs)))
            }
            Some(TriggerProgress::Searching { unsearched_secs }) if unsearched_secs > 0.0 => {
                let sec_per_mb = if m.processed_mb > 0.0 { m.processed_secs / m.processed_mb } else { f64::INFINITY };
                let unsearched_mb = unsearched_secs / sec_per_mb;
                if m.backlog_mb + unsearched_mb > 0.9 * m.buffer_mb {
                    return MemoryDecision::Stop(StopReason::Backlog);
                }
                // Never discard data the search hasn't scanned yet.
                match trim_keep(&m) {
                    Some(k) if k < unsearched_secs => MemoryDecision::Stop(StopReason::Backlog),
                    k => trim(k),
                }
            }
            // Caught up (or no trigger state yet): plain TRIM.
            _ => trim(trim_keep(&m)),
        },
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn rate_menu_from_range() {
        assert_eq!(rates_from_range(1_000, 50_000, 1), vec![50_000, 20_000, 10_000, 5_000, 2_000, 1_000]);
        // `max` is always offered.
        assert_eq!(rates_from_range(1, 30, 1), vec![30, 20, 10, 5, 2, 1]);
    }

    #[test]
    fn default_rate_is_highest_up_to_10_msps() {
        assert_eq!(default_sample_rate(&[24_000_000, 8_000_000, 1_000_000]), Some(8_000_000));
        assert_eq!(default_sample_rate(&[50_000_000, 20_000_000]), Some(20_000_000));
        assert_eq!(default_sample_rate(&[]), None);
    }

    #[test]
    fn settings_enable_toggle_is_sorted() {
        let mut s = CaptureSettings::default();
        s.set_enabled(ChannelId::digital(3), true);
        s.set_enabled(ChannelId::digital(1), true);
        s.set_enabled(ChannelId::digital(3), true);
        assert_eq!(s.enabled, vec![ChannelId::digital(1), ChannelId::digital(3)]);
        s.set_enabled(ChannelId::digital(1), false);
        assert_eq!(s.enabled, vec![ChannelId::digital(3)]);
    }

    /// U-07: the memory policy decision table, all branches.
    #[test]
    fn u07_memory_policy_branches() {
        use CaptureMode::*;
        let m = |p, l, b, secs| MemoryUse { processed_mb: p, backlog_mb: l, buffer_mb: b, processed_secs: secs };
        let keep = |d: MemoryDecision| match d {
            MemoryDecision::Trim { keep_secs } => keep_secs,
            d => panic!("{d:?}"),
        };
        assert_eq!(memory_policy(Continuous, m(100.0, 0.0, 512.0, 10.0), None), MemoryDecision::Keep);
        assert_eq!(
            memory_policy(Timed, m(600.0, 0.0, 512.0, 10.0), None),
            MemoryDecision::Stop(StopReason::BufferFull)
        );
        assert_eq!(
            memory_policy(Continuous, m(50.0, 470.0, 512.0, 1.0), None),
            MemoryDecision::Stop(StopReason::Backlog)
        );
        // 600 MB for 60 s → 0.1 s/MB; target 512 − 0 − 1 = 511 MB → 51.1 s.
        assert!((keep(memory_policy(Continuous, m(600.0, 0.0, 512.0, 60.0), None)) - 51.1).abs() < 1e-9);
        // Never below 0.1 s.
        assert_eq!(keep(memory_policy(Continuous, m(600.0, 0.0, 0.5, 0.01), None)), MIN_KEEP_SECS);

        let full = m(600.0, 0.0, 512.0, 60.0);
        // Found, far from the start: trim, but keep from T on.
        let found = |at| Some(TriggerProgress::Found { at_secs: at });
        assert!((keep(memory_policy(Triggered, full, found(30.0))) - 51.1).abs() < 1e-9);
        assert!((keep(memory_policy(Triggered, full, found(8.0))) - 52.0).abs() < 1e-9, "60 − 8");
        // Found and already trimmed to T: stop, buffer full.
        assert_eq!(memory_policy(Triggered, full, found(0.0)), MemoryDecision::Stop(StopReason::BufferFull));
        // Searching, caught up: plain trim.
        let searching = |u| Some(TriggerProgress::Searching { unsearched_secs: u });
        assert!((keep(memory_policy(Triggered, full, searching(0.0))) - 51.1).abs() < 1e-9);
        // Searching, unscanned data too large: stop for the backlog.
        assert_eq!(memory_policy(Triggered, full, searching(50.0)), MemoryDecision::Stop(StopReason::Backlog));
        // Searching, near the backlog limit: unscanned data still fits in
        // what the trim keeps (29.9 s ≥ 26 s), so it trims; just past the
        // limit it stops. (The backlog check makes "a trim would drop
        // unscanned data" unreachable for buffers over ~10 MB.)
        let tight = m(600.0, 0.0, 300.0, 60.0);
        assert!((keep(memory_policy(Triggered, tight, searching(26.0))) - 29.9).abs() < 1e-9);
        assert_eq!(memory_policy(Triggered, tight, searching(27.5)), MemoryDecision::Stop(StopReason::Backlog));
        // Searching, the trim keeps all unscanned data: trim.
        assert!((keep(memory_policy(Triggered, full, searching(5.0))) - 51.1).abs() < 1e-9);
    }
}
