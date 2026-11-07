//! Range measurements: the model and the metric catalogue.

use crate::core::ChannelId;
use crate::core::time::CaptureTime;
use serde::{Deserialize, Serialize};

/// Range colors (`range-measurement-0..7`).
pub const RANGE_COLORS: [[u8; 3]; 8] = [
    [0xC8, 0x5A, 0xA0], // orchid
    [0x5A, 0xC8, 0x7A], // mint
    [0x5A, 0x8A, 0xC8], // denim
    [0xC8, 0x5A, 0x5A], // brick
    [0xD4, 0xA8, 0x4A], // ochre
    [0x8A, 0x5A, 0xC8], // plum
    [0x5A, 0xC8, 0xC8], // aqua
    [0xC8, 0xC8, 0x5A], // khaki
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangeMeasurement {
    pub id: u32,
    pub name: String,
    pub channel: ChannelId,
    /// `t.0 < t.1`.
    pub t: (CaptureTime, CaptureTime),
    /// Index into [`RANGE_COLORS`].
    pub color: usize,
    pub comment: String,
    pub selected: bool,
    pub expanded: bool,
    /// The popup's user-dragged offset.
    pub popup_offset: Option<(f32, f32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ranges {
    pub items: Vec<RangeMeasurement>,
}

impl Ranges {
    pub fn next_id(&self) -> u32 {
        self.items.iter().map(|r| r.id + 1).max().unwrap_or(0)
    }

    /// Adds `M<id>` on `channel`; the bounds are sorted.
    pub fn add(&mut self, channel: ChannelId, a: CaptureTime, b: CaptureTime) -> u32 {
        let id = self.next_id();
        self.items.push(RangeMeasurement {
            id,
            name: format!("M{id}"),
            channel,
            t: if a <= b { (a, b) } else { (b, a) },
            color: id as usize % RANGE_COLORS.len(),
            comment: String::new(),
            selected: false,
            expanded: false,
            popup_offset: None,
        });
        id
    }

    pub fn get(&self, id: u32) -> Option<&RangeMeasurement> {
        self.items.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut RangeMeasurement> {
        self.items.iter_mut().find(|r| r.id == id)
    }

    pub fn remove(&mut self, id: u32) {
        self.items.retain(|r| r.id != id);
    }

    pub fn remove_selected(&mut self) -> usize {
        let n = self.items.len();
        self.items.retain(|r| !r.selected);
        n - self.items.len()
    }

    pub fn select_only(&mut self, id: u32) {
        for r in &mut self.items {
            r.selected = r.id == id;
        }
    }

    /// Moves one edge; edges that cross swap.
    pub fn set_edge(&mut self, id: u32, end: bool, t: CaptureTime) -> bool {
        let Some(r) = self.get_mut(id) else { return end };
        if end {
            r.t.1 = t
        } else {
            r.t.0 = t
        }
        if r.t.0 > r.t.1 {
            r.t = (r.t.1, r.t.0);
            !end
        } else {
            end
        }
    }

    /// A trim keeps only ranges fully inside `[begin, end]`.
    pub fn trim(&mut self, begin: CaptureTime, end: CaptureTime) {
        self.items.retain(|r| r.t.0 >= begin && r.t.1 <= end);
    }

    /// Moves every range by `dt`, e.g. with a new round's trigger.
    pub fn shift(&mut self, dt: CaptureTime) {
        for r in &mut self.items {
            r.t = (r.t.0 + dt, r.t.1 + dt);
        }
    }

    /// Inserts `r` (used by undo/redo to restore a range).
    pub fn insert(&mut self, r: RangeMeasurement) {
        if self.items.iter().all(|x| x.id != r.id) {
            self.items.push(r);
        }
    }
}

/// One undoable range operation.
#[derive(Clone, Debug, PartialEq)]
pub enum RangeOp {
    Add(RangeMeasurement),
    Delete(Vec<RangeMeasurement>),
}

/// The per-session undo/redo stack for range add and delete.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RangeHistory {
    undo: Vec<RangeOp>,
    redo: Vec<RangeOp>,
}

impl RangeHistory {
    /// At most this many steps.
    pub const MAX: usize = 50;

    /// Keeps undo and redo in step with [`Ranges::shift`].
    pub fn shift(&mut self, dt: CaptureTime) {
        for op in self.undo.iter_mut().chain(&mut self.redo) {
            match op {
                RangeOp::Add(r) => r.t = (r.t.0 + dt, r.t.1 + dt),
                RangeOp::Delete(rs) => rs.iter_mut().for_each(|r| r.t = (r.t.0 + dt, r.t.1 + dt)),
            }
        }
    }

    /// Records an operation and clears the redo stack.
    pub fn record(&mut self, op: RangeOp) {
        self.undo.push(op);
        if self.undo.len() > Self::MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Pops the last operation to undo; the caller applies its inverse.
    pub fn undo(&mut self) -> Option<RangeOp> {
        let op = self.undo.pop()?;
        self.redo.push(op.clone());
        Some(op)
    }

    /// Pops the last undone operation; the caller re-applies it.
    pub fn redo(&mut self) -> Option<RangeOp> {
        let op = self.redo.pop()?;
        self.undo.push(op.clone());
        Some(op)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Metric keys.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Metric {
    Dt,
    NRising,
    NFalling,
    FMean,
    FMin,
    FMax,
    TStd,
    PwPos,
    PwNeg,
    Duty,
    PwPosMin,
    PwPosMax,
    PwNegMin,
    PwNegMax,
    FJitter,
    VMin,
    VMax,
    Vpp,
    VMean,
    VRms,
    VMinT,
    VMaxT,
}

impl Metric {
    pub const DIGITAL: [Metric; 15] = [
        Metric::Dt,
        Metric::NRising,
        Metric::NFalling,
        Metric::FMean,
        Metric::FMin,
        Metric::FMax,
        Metric::TStd,
        Metric::PwPos,
        Metric::PwNeg,
        Metric::Duty,
        Metric::PwPosMin,
        Metric::PwPosMax,
        Metric::PwNegMin,
        Metric::PwNegMax,
        Metric::FJitter,
    ];
    pub const ANALOG: [Metric; 8] = [
        Metric::Dt,
        Metric::VMin,
        Metric::VMax,
        Metric::Vpp,
        Metric::VMean,
        Metric::VRms,
        Metric::VMinT,
        Metric::VMaxT,
    ];
    /// Default visible sets.
    pub const DIGITAL_DEFAULT: [Metric; 7] =
        [Metric::Dt, Metric::NRising, Metric::NFalling, Metric::FMin, Metric::FMax, Metric::FMean, Metric::TStd];
    pub const ANALOG_DEFAULT: [Metric; 3] = [Metric::Dt, Metric::VRms, Metric::Vpp];

    /// The label as `(base, subscript)`, e.g. `("f", "mean")`, and whether
    /// the base is italic.
    pub fn label(self) -> (&'static str, &'static str, bool) {
        match self {
            Metric::Dt => ("ΔT", "", false),
            Metric::NRising => ("N", "rising", false),
            Metric::NFalling => ("N", "falling", false),
            Metric::FMean => ("f", "mean", true),
            Metric::FMin => ("f", "min", true),
            Metric::FMax => ("f", "max", true),
            Metric::TStd => ("T", "std", false),
            Metric::PwPos => ("Positive pulse width (mean)", "", false),
            Metric::PwNeg => ("Negative pulse width (mean)", "", false),
            Metric::Duty => ("Duty cycle", "", false),
            Metric::PwPosMin => ("Positive pulse width (min)", "", false),
            Metric::PwPosMax => ("Positive pulse width (max)", "", false),
            Metric::PwNegMin => ("Negative pulse width (min)", "", false),
            Metric::PwNegMax => ("Negative pulse width (max)", "", false),
            Metric::FJitter => ("f", "jitter", true),
            Metric::VMin => ("V", "min", false),
            Metric::VMax => ("V", "max", false),
            Metric::Vpp => ("V", "pp", false),
            Metric::VMean => ("V", "mean", false),
            Metric::VRms => ("V", "RMS", false),
            Metric::VMinT => ("V", "min time", false),
            Metric::VMaxT => ("V", "max time", false),
        }
    }

    /// Plain text label for copies and accessibility, e.g. `f mean`.
    pub fn plain_label(self) -> String {
        let (b, s, _) = self.label();
        if s.is_empty() { b.to_owned() } else { format!("{b} {s}") }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Metric::Dt
            | Metric::TStd
            | Metric::PwPos
            | Metric::PwNeg
            | Metric::PwPosMin
            | Metric::PwPosMax
            | Metric::PwNegMin
            | Metric::PwNegMax
            | Metric::VMinT
            | Metric::VMaxT => "s",
            Metric::NRising | Metric::NFalling => "",
            Metric::FMean | Metric::FMin | Metric::FMax => "Hz",
            Metric::Duty | Metric::FJitter => "%",
            _ => "V",
        }
    }

    /// What the metric measures, for tooltips.
    pub fn tooltip(self) -> &'static str {
        use Metric::*;
        match self {
            Dt => "Range duration: end minus begin.",
            NRising => "Number of low-to-high transitions in the range.",
            NFalling => "Number of high-to-low transitions in the range.",
            FMean => {
                "Mean frequency: complete periods divided by the time between the first and last same-polarity edge."
            }
            FMin => "Minimum frequency: 1 / longest period.",
            FMax => "Maximum frequency: 1 / shortest period.",
            TStd => "Standard deviation of the periods (period-to-period variation).",
            PwPos => "Mean width of the high (positive) pulses.",
            PwNeg => "Mean width of the low (negative) pulses.",
            Duty => "Duty cycle: total high time divided by the range duration.",
            PwPosMin => "Shortest high (positive) pulse.",
            PwPosMax => "Longest high (positive) pulse.",
            PwNegMin => "Shortest low (negative) pulse.",
            PwNegMax => "Longest low (negative) pulse.",
            FJitter => "Frequency jitter: RMS of (period − mean period) / mean period, as a percentage.",
            VMin => "Minimum voltage in the range.",
            VMax => "Maximum voltage in the range.",
            Vpp => "Peak-to-peak voltage: maximum minus minimum.",
            VMean => "Arithmetic mean of the samples.",
            VRms => "Root-mean-square voltage: square root of the mean of the squares.",
            VMinT => "Time of the minimum voltage, relative to the range start.",
            VMaxT => "Time of the maximum voltage, relative to the range start.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(id: u32) -> RangeMeasurement {
        RangeMeasurement {
            id,
            name: format!("M{id}"),
            channel: ChannelId::digital(0),
            t: (CaptureTime::from_secs_f64(id as f64), CaptureTime::from_secs_f64(id as f64 + 1.0)),
            color: 0,
            comment: String::new(),
            selected: false,
            expanded: false,
            popup_offset: None,
        }
    }

    /// undo/redo of add and delete, with the cap and clear.
    #[test]
    fn history_undo_redo() {
        let mut h = RangeHistory::default();
        assert!(!h.can_undo() && !h.can_redo());
        h.record(RangeOp::Add(range(0)));
        assert!(h.can_undo());
        assert_eq!(h.undo(), Some(RangeOp::Add(range(0))));
        assert!(h.can_redo() && !h.can_undo());
        assert_eq!(h.redo(), Some(RangeOp::Add(range(0))));
        // A new operation clears the redo stack.
        h.record(RangeOp::Delete(vec![range(1)]));
        assert!(!h.can_redo());
        assert_eq!(h.undo(), Some(RangeOp::Delete(vec![range(1)])));
        // The cap drops the oldest step.
        for i in 0..(RangeHistory::MAX as u32 + 5) {
            h.record(RangeOp::Add(range(i)));
        }
        assert_eq!(h.undo().unwrap(), RangeOp::Add(range(RangeHistory::MAX as u32 + 4)));
        h.clear();
        assert!(!h.can_undo() && !h.can_redo());
    }

    #[test]
    fn insert_dedupes_by_id() {
        let mut r = Ranges::default();
        r.insert(range(3));
        r.insert(range(3));
        assert_eq!(r.items.len(), 1);
    }

    /// Ranges, and the undo stack, follow a new round's trigger.
    #[test]
    fn shift_moves_ranges_and_history() {
        let dt = CaptureTime::from_secs_f64(0.5);
        let mut r = Ranges { items: vec![range(1)] };
        let mut h = RangeHistory::default();
        h.record(RangeOp::Add(range(2)));
        r.shift(dt);
        h.shift(dt);
        assert_eq!(r.items[0].t, (CaptureTime::from_secs_f64(1.5), CaptureTime::from_secs_f64(2.5)));
        let Some(RangeOp::Add(x)) = h.undo() else { panic!() };
        assert_eq!(x.t.0, CaptureTime::from_secs_f64(2.5));
    }
}
