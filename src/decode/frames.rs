//! The per-analyzer frame store: append-only, indexed by
//! start sample overall and per annotation row.

use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub start: u64,
    pub end: u64,
    /// Annotation class index.
    pub class: usize,
    /// Annotation row index (see [`crate::decode::class_rows`]).
    pub row: usize,
    /// Longest first.
    pub texts: Arc<[String]>,
    /// Error heuristic.
    pub error: bool,
}

impl Frame {
    pub fn longest(&self) -> &str {
        self.texts.first().map_or("", String::as_str)
    }

    pub fn shortest(&self) -> &str {
        self.texts.last().map_or("", String::as_str)
    }
}

#[derive(Clone, Debug, Default)]
pub struct FrameStore {
    frames: Vec<Frame>,
    /// Frame ids in (start, id) order.
    order: Vec<u32>,
    /// Per row: frame ids in (start, id) order.
    rows: Vec<Vec<u32>>,
    /// Heap bytes of the frames' texts, counted as they come.
    text_bytes: usize,
}

/// Inserts `id` keeping `v` sorted by `key`; frames mostly arrive in order,
/// so this is usually a push.
fn insert_sorted(v: &mut Vec<u32>, id: u32, key: impl Fn(u32) -> (u64, u32)) {
    let k = key(id);
    if v.last().is_none_or(|&l| key(l) <= k) {
        v.push(id);
    } else {
        let at = v.partition_point(|&x| key(x) <= k);
        v.insert(at, id);
    }
}

impl FrameStore {
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn push(&mut self, f: Frame) {
        // The `Arc<[String]>`: its counts and slice, plus each string.
        let n = f.texts.len();
        self.text_bytes += 16 + n * std::mem::size_of::<String>() + f.texts.iter().map(String::capacity).sum::<usize>();
        let id = self.frames.len() as u32;
        let row = f.row;
        self.frames.push(f);
        let frames = &self.frames;
        let key = |i: u32| (frames[i as usize].start, i);
        insert_sorted(&mut self.order, id, key);
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, Vec::new);
        }
        insert_sorted(&mut self.rows[row], id, key);
    }

    /// The store's heap bytes (frames, indexes, texts).
    pub fn memory_bytes(&self) -> usize {
        self.frames.capacity() * std::mem::size_of::<Frame>()
            + self.order.capacity() * 4
            + self.rows.capacity() * std::mem::size_of::<Vec<u32>>()
            + self.rows.iter().map(|r| r.capacity() * 4).sum::<usize>()
            + self.text_bytes
    }

    pub fn get(&self, id: u32) -> Option<&Frame> {
        self.frames.get(id as usize)
    }

    /// The k-th frame in start order (the data table).
    pub fn by_order(&self, k: usize) -> Option<(u32, &Frame)> {
        let id = *self.order.get(k)?;
        Some((id, &self.frames[id as usize]))
    }

    /// Frame ids in start order.
    pub fn order(&self) -> &[u32] {
        &self.order
    }

    /// Frames of `row` overlapping `[s0, s1)`, in start order.
    pub fn in_row(&self, row: usize, s0: u64, s1: u64) -> impl Iterator<Item = (u32, &Frame)> + '_ {
        let ids = self.rows.get(row).map_or(&[][..], Vec::as_slice);
        // Frames in a row rarely overlap; scan back over any that still
        // reach into the window.
        let mut first = ids.partition_point(|&i| self.frames[i as usize].start < s0);
        while first > 0 && self.frames[ids[first - 1] as usize].end > s0 {
            first -= 1;
        }
        ids[first..]
            .iter()
            .map(|&i| (i, &self.frames[i as usize]))
            .take_while(move |(_, f)| f.start < s1)
            .filter(move |(_, f)| f.end > s0 || f.start >= s0)
    }

    /// The first frame of `row` starting after `s`.
    pub fn next_in_row(&self, row: usize, s: u64) -> Option<(u32, &Frame)> {
        let ids = self.rows.get(row)?;
        let k = ids.partition_point(|&i| self.frames[i as usize].start <= s);
        ids.get(k).map(|&i| (i, &self.frames[i as usize]))
    }

    /// The last frame of `row` ending before `s`.
    pub fn prev_in_row(&self, row: usize, s: u64) -> Option<(u32, &Frame)> {
        let ids = self.rows.get(row)?;
        let k = ids.partition_point(|&i| self.frames[i as usize].start < s);
        ids[..k].iter().rev().map(|&i| (i, &self.frames[i as usize])).find(|(_, f)| f.end <= s)
    }

    /// `frame_at`: a frame of `row` containing sample `s`.
    pub fn at(&self, row: usize, s: u64) -> Option<(u32, &Frame)> {
        self.in_row(row, s, s + 1).find(|(_, f)| f.start <= s && s < f.end.max(f.start + 1))
    }

    /// The position of frame `id` in start order.
    pub fn order_index(&self, id: u32) -> Option<usize> {
        let f = self.frames.get(id as usize)?;
        let k = (f.start, id);
        let at = self.order.partition_point(|&i| (self.frames[i as usize].start, i) < k);
        (self.order.get(at) == Some(&id)).then_some(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(start: u64, end: u64, row: usize, t: &str) -> Frame {
        Frame { start, end, class: 0, row, texts: vec![t.to_owned()].into(), error: false }
    }

    #[test]
    fn queries_by_row_and_order() {
        let mut s = FrameStore::default();
        s.push(f(10, 20, 0, "a"));
        s.push(f(30, 40, 0, "b"));
        s.push(f(5, 50, 1, "packet"));
        s.push(f(50, 60, 0, "c"));
        // Out of order on arrival: start order is kept.
        assert_eq!(s.order(), [2, 0, 1, 3]);
        let hits: Vec<&str> = s.in_row(0, 15, 35).map(|(_, f)| f.longest()).collect();
        assert_eq!(hits, ["a", "b"]);
        assert_eq!(s.in_row(1, 45, 46).count(), 1, "a long frame reaching into the window");
        assert_eq!(s.next_in_row(0, 30).unwrap().1.longest(), "c");
        assert_eq!(s.prev_in_row(0, 30).unwrap().1.longest(), "a");
        assert_eq!(s.at(0, 35).unwrap().1.longest(), "b");
        assert!(s.at(0, 25).is_none());
        assert_eq!(s.order_index(3), Some(3));
        assert_eq!(s.order_index(2), Some(0));
    }
}
