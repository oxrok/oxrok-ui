//! Timing markers, marker pairs and the time origin (10).

use crate::core::time::CaptureTime;
use serde::{Deserialize, Serialize};

/// The marker palette, cycled by id.
pub const MARKER_PALETTE: [[u8; 3]; 6] = [
    [0xD4, 0xA8, 0x5A],
    [0x5A, 0xC8, 0xD4],
    [0xC8, 0x5A, 0x7A],
    [0x8A, 0xC8, 0x5A],
    [0x9A, 0x7A, 0xC8],
    [0xE8, 0x5A, 0x8A],
];

/// Snapping distance and minimum pulse width, in points.
pub const SNAP_PT: f64 = 8.0;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MarkerKind {
    Single {
        t: CaptureTime,
    },
    /// Always sorted: `t[0] ≤ t[1]` (A then B).
    Pair {
        t: [CaptureTime; 2],
        expanded: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimingItem {
    pub id: u32,
    pub label: String,
    /// Index into [`MARKER_PALETTE`].
    pub color: usize,
    pub note: Option<String>,
    pub selected: bool,
    pub kind: MarkerKind,
}

/// One line on the graph: a single marker, or one side of a pair.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MarkerRef {
    pub id: u32,
    /// `None` for single markers; 0 = A, 1 = B for pairs.
    pub side: Option<u8>,
}

impl TimingItem {
    pub fn is_pair(&self) -> bool {
        matches!(self.kind, MarkerKind::Pair { .. })
    }

    /// `T<label>` or `P<label>`.
    pub fn name(&self) -> String {
        if self.is_pair() { format!("P{}", self.label) } else { format!("T{}", self.label) }
    }

    /// `T3`, `P1-A`, `P1-B`.
    pub fn side_name(&self, side: Option<u8>) -> String {
        match side {
            Some(s) if self.is_pair() => format!("P{}-{}", self.label, if s == 0 { 'A' } else { 'B' }),
            _ => self.name(),
        }
    }

    /// The lines this item draws.
    pub fn refs(&self) -> Vec<MarkerRef> {
        match self.kind {
            MarkerKind::Single { .. } => vec![MarkerRef { id: self.id, side: None }],
            MarkerKind::Pair { .. } => {
                vec![MarkerRef { id: self.id, side: Some(0) }, MarkerRef { id: self.id, side: Some(1) }]
            }
        }
    }

    pub fn time(&self, side: Option<u8>) -> CaptureTime {
        match self.kind {
            MarkerKind::Single { t } => t,
            MarkerKind::Pair { t, .. } => t[side.unwrap_or(0).min(1) as usize],
        }
    }

    /// The item's span (a single marker is a zero-length span).
    pub fn span(&self) -> (CaptureTime, CaptureTime) {
        match self.kind {
            MarkerKind::Single { t } => (t, t),
            MarkerKind::Pair { t, .. } => (t[0], t[1]),
        }
    }
}

/// The markers of one session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Markers {
    pub items: Vec<TimingItem>,
}

impl Markers {
    /// Data model: next id = max(existing) + 1, or 0.
    pub fn next_id(&self) -> u32 {
        self.items.iter().map(|i| i.id + 1).max().unwrap_or(0)
    }

    fn push(&mut self, kind: MarkerKind) -> u32 {
        let id = self.next_id();
        self.items.push(TimingItem {
            id,
            label: id.to_string(),
            color: id as usize % MARKER_PALETTE.len(),
            note: None,
            selected: false,
            kind,
        });
        self.items.sort_by_key(|i| i.id);
        id
    }

    pub fn add_single(&mut self, t: CaptureTime) -> u32 {
        self.push(MarkerKind::Single { t })
    }

    pub fn add_pair(&mut self, a: CaptureTime, b: CaptureTime) -> u32 {
        self.push(MarkerKind::Pair { t: if a <= b { [a, b] } else { [b, a] }, expanded: true })
    }

    pub fn get(&self, id: u32) -> Option<&TimingItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut TimingItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn time(&self, r: MarkerRef) -> Option<CaptureTime> {
        Some(self.get(r.id)?.time(r.side))
    }

    /// Moves one line to `t`. A pair keeps A ≤ B by swapping; the returned
    /// ref is where the moved side ended up.
    pub fn set_time(&mut self, r: MarkerRef, t: CaptureTime) -> MarkerRef {
        let Some(item) = self.get_mut(r.id) else { return r };
        match &mut item.kind {
            MarkerKind::Single { t: s } => {
                *s = t;
                r
            }
            MarkerKind::Pair { t: ts, .. } => {
                let side = r.side.unwrap_or(0).min(1) as usize;
                ts[side] = t;
                if ts[0] > ts[1] {
                    ts.swap(0, 1);
                    MarkerRef { id: r.id, side: Some(1 - side as u8) }
                } else {
                    r
                }
            }
        }
    }

    pub fn remove(&mut self, id: u32) {
        self.items.retain(|i| i.id != id);
    }

    pub fn remove_selected(&mut self) -> usize {
        let before = self.items.len();
        self.items.retain(|i| !i.selected);
        before - self.items.len()
    }

    pub fn select_only(&mut self, id: u32) {
        for i in &mut self.items {
            i.selected = i.id == id;
        }
    }

    pub fn deselect_all(&mut self) {
        for i in &mut self.items {
            i.selected = false;
        }
    }

    pub fn select_all(&mut self) {
        for i in &mut self.items {
            i.selected = true;
        }
    }

    /// Shift+click: select ids between `a` and `b` inclusive.
    pub fn select_range(&mut self, a: u32, b: u32) {
        let (lo, hi) = (a.min(b), a.max(b));
        for i in &mut self.items {
            if (lo..=hi).contains(&i.id) {
                i.selected = true;
            }
        }
    }

    pub fn selected_singles(&self) -> Vec<u32> {
        self.items.iter().filter(|i| i.selected && !i.is_pair()).map(|i| i.id).collect()
    }

    /// *Create Pair*: merges two single markers into a pair with
    /// the earlier one's color and both notes joined by ", ".
    pub fn merge_into_pair(&mut self, a: u32, b: u32) -> Option<u32> {
        let (x, y) = (self.get(a)?.clone(), self.get(b)?.clone());
        let (first, second) = if x.time(None) <= y.time(None) { (x, y) } else { (y, x) };
        let notes: Vec<String> = [first.note.clone(), second.note.clone()].into_iter().flatten().collect();
        self.remove(a);
        self.remove(b);
        let id = self.add_pair(first.time(None), second.time(None));
        let item = self.get_mut(id)?;
        item.color = first.color;
        item.note = (!notes.is_empty()).then(|| notes.join(", "));
        item.selected = true;
        Some(id)
    }

    /// *Ungroup Pair*: two singles that keep color, note and selection.
    pub fn ungroup(&mut self, id: u32) -> Option<[u32; 2]> {
        let item = self.get(id)?.clone();
        let MarkerKind::Pair { t, .. } = item.kind else { return None };
        self.remove(id);
        let ids = [self.add_single(t[0]), self.add_single(t[1])];
        for new in ids {
            let i = self.get_mut(new)?;
            i.color = item.color;
            i.note = item.note.clone();
            i.selected = item.selected;
        }
        Some(ids)
    }

    /// A trim keeps `[begin, end]`; items with a line outside go.
    pub fn trim(&mut self, begin: CaptureTime, end: CaptureTime) {
        self.items.retain(|i| {
            let (a, b) = i.span();
            a >= begin && b <= end
        });
    }

    /// Moves every marker by `dt`, e.g. with a new round's trigger.
    pub fn shift(&mut self, dt: CaptureTime) {
        for i in &mut self.items {
            match &mut i.kind {
                MarkerKind::Single { t } => *t += dt,
                MarkerKind::Pair { t, .. } => t.iter_mut().for_each(|t| *t += dt),
            }
        }
    }

    /// Every line, for drawing and hit-testing.
    pub fn lines(&self) -> impl Iterator<Item = (MarkerRef, &TimingItem)> {
        self.items.iter().flat_map(|i| i.refs().into_iter().map(move |r| (r, i)))
    }
}

/// What the time labels are relative to.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeOrigin {
    #[default]
    CaptureStart,
    Trigger,
    Marker(MarkerRef),
    /// Origin at the capture start; labels show date and time.
    WallClock,
}

impl TimeOrigin {
    /// The origin's absolute time. Trigger falls back to the capture start
    /// until a trigger exists, and a missing marker does too.
    pub fn resolve(&self, markers: &Markers, trigger: Option<CaptureTime>) -> CaptureTime {
        match self {
            TimeOrigin::CaptureStart | TimeOrigin::WallClock => CaptureTime::ZERO,
            TimeOrigin::Trigger => trigger.unwrap_or(CaptureTime::ZERO),
            TimeOrigin::Marker(r) => markers.time(*r).unwrap_or(CaptureTime::ZERO),
        }
    }

    /// The default when a capture starts.
    pub fn for_capture_start(self, triggered: bool) -> TimeOrigin {
        match self {
            TimeOrigin::WallClock => TimeOrigin::WallClock,
            _ if triggered => TimeOrigin::Trigger,
            _ => TimeOrigin::CaptureStart,
        }
    }
}

/// Snapping, in pixels along the plot. `prev`/`next` are the
/// transitions around the pointer (the edges of the pulse under it).
/// Returns the x to use: a snapped edge, or the pointer rounded up.
pub fn snap_x(pointer: f64, prev: Option<f64>, next: Option<f64>, snap_px: f64) -> f64 {
    let near = |e: f64| (e - pointer).abs() <= snap_px;
    match (prev, next) {
        (Some(a), Some(b)) => {
            if b - a >= snap_px && (near(a) || near(b)) {
                if (pointer - a).abs() <= (b - pointer).abs() { a } else { b }
            } else {
                pointer.ceil()
            }
        }
        (Some(e), None) | (None, Some(e)) if near(e) => e,
        _ => pointer.ceil(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: f64) -> CaptureTime {
        CaptureTime::from_secs_f64(x)
    }

    /// Markers follow a new round's trigger.
    #[test]
    fn shift_moves_singles_and_pairs() {
        let mut m = Markers::default();
        m.add_single(s(1.0));
        m.add_pair(s(2.0), s(3.0));
        m.shift(s(0.5));
        assert_eq!(m.items[0].span(), (s(1.5), s(1.5)));
        assert_eq!(m.items[1].span(), (s(2.5), s(3.5)));
        m.shift(-s(1.0));
        assert_eq!(m.items[1].span(), (s(1.5), s(2.5)));
    }

    #[test]
    fn ids_names_and_colors() {
        let mut m = Markers::default();
        assert_eq!(m.add_pair(s(2.0), s(1.0)), 0);
        assert_eq!(m.add_single(s(0.5)), 1);
        let p = m.get(0).unwrap();
        assert_eq!((p.name(), p.side_name(Some(0)), p.side_name(Some(1))), ("P0".into(), "P0-A".into(), "P0-B".into()));
        assert_eq!(p.time(Some(0)), s(1.0), "sorted");
        assert_eq!(m.get(1).unwrap().name(), "T1");
        assert_eq!(m.get(1).unwrap().color, 1);
        m.remove(0);
        assert_eq!(m.next_id(), 2, "max + 1");
        m.remove(1);
        assert_eq!(m.next_id(), 0);
    }

    #[test]
    fn moving_a_side_past_the_other_swaps() {
        let mut m = Markers::default();
        let id = m.add_pair(s(1.0), s(2.0));
        let r = m.set_time(MarkerRef { id, side: Some(1) }, s(0.5));
        assert_eq!(r.side, Some(0), "B became A");
        assert_eq!(m.get(id).unwrap().span(), (s(0.5), s(1.0)));
    }

    #[test]
    fn merge_ungroup_and_trim() {
        let mut m = Markers::default();
        let a = m.add_single(s(3.0));
        let b = m.add_single(s(1.0));
        m.get_mut(a).unwrap().note = Some("x".into());
        m.get_mut(b).unwrap().note = Some("y".into());
        let p = m.merge_into_pair(a, b).unwrap();
        let item = m.get(p).unwrap();
        assert_eq!(item.span(), (s(1.0), s(3.0)));
        assert_eq!(item.color, 1, "the earlier marker's color");
        assert_eq!(item.note.as_deref(), Some("y, x"));
        let [c, d] = m.ungroup(p).unwrap();
        assert_eq!((m.get(c).unwrap().color, m.get(d).unwrap().note.as_deref()), (1, Some("y, x")));
        m.trim(s(2.0), s(5.0));
        assert_eq!(m.items.len(), 1);
    }

    #[test]
    fn origin_resolution_and_capture_start_defaults() {
        let mut m = Markers::default();
        let id = m.add_pair(s(1.0), s(2.0));
        let o = TimeOrigin::Marker(MarkerRef { id, side: Some(1) });
        assert_eq!(o.resolve(&m, None), s(2.0));
        assert_eq!(TimeOrigin::Trigger.resolve(&m, None), CaptureTime::ZERO);
        assert_eq!(TimeOrigin::Trigger.resolve(&m, Some(s(0.3))), s(0.3));
        m.remove(id);
        assert_eq!(o.resolve(&m, None), CaptureTime::ZERO);
        assert_eq!(o.for_capture_start(false), TimeOrigin::CaptureStart);
        assert_eq!(TimeOrigin::WallClock.for_capture_start(true), TimeOrigin::WallClock);
        assert_eq!(o.for_capture_start(true), TimeOrigin::Trigger);
    }

    /// U-09: snapping thresholds (8 pt) and edge selection.
    #[test]
    fn u09_snapping() {
        // Pulse 100..140: within 8 of an edge snaps to the nearer one.
        assert_eq!(snap_x(105.0, Some(100.0), Some(140.0), SNAP_PT), 100.0);
        assert_eq!(snap_x(133.5, Some(100.0), Some(140.0), SNAP_PT), 140.0);
        assert_eq!(snap_x(108.0, Some(100.0), Some(140.0), SNAP_PT), 100.0, "exactly 8 snaps");
        // Too far from both: the pointer, rounded up.
        assert_eq!(snap_x(120.2, Some(100.0), Some(140.0), SNAP_PT), 121.0);
        // A pulse narrower than 8 pt doesn't snap.
        assert_eq!(snap_x(102.0, Some(100.0), Some(105.0), SNAP_PT), 102.0);
        // Only one transition nearby.
        assert_eq!(snap_x(95.0, Some(100.0), None, SNAP_PT), 100.0);
        assert_eq!(snap_x(91.5, None, Some(100.0), SNAP_PT), 92.0);
        assert_eq!(snap_x(50.0, None, None, SNAP_PT), 50.0);
    }
}
