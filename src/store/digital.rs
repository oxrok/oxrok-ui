//! Digital channel storage.
//!
//! A channel is stored as the sample positions of its transitions,
//! delta-encoded as LEB128 varints in chunks of up to [`CHUNK_TRANSITIONS`].
//! Every chunk keeps a checkpoint per [`CHECKPOINT_EVERY`] transitions.
//!
//! This index is what the pyramid provides, without materializing
//! per-block tables. [`DigitalChannel::rank`] counts the transitions up to
//! any sample in O(log n + 64) varint decodes. Block summaries (first level,
//! last level, transition count) for any `[s0, s1)`
//! ([`DigitalChannel::summary`]) and the per-column classes of
//! ([`DigitalChannel::classify_columns`]) follow from the ranks. Levels
//! alternate at each transition, so a rank's parity gives the level.

use std::collections::VecDeque;

/// Transitions per chunk ("roughly 64 K").
pub const CHUNK_TRANSITIONS: usize = 65_536;
/// Transitions between checkpoints inside a chunk.
pub const CHECKPOINT_EVERY: usize = 64;

#[derive(Clone, Debug)]
struct Chunk {
    /// Absolute ordinal of the chunk's first transition.
    first_ordinal: u64,
    first_sample: u64,
    last_sample: u64,
    count: usize,
    /// Varint deltas of transitions 1.. (transition 0 is `first_sample`).
    bytes: Vec<u8>,
    /// `(sample, byte offset after its varint)` for transitions 0, 64, 128, …
    checkpoints: Vec<(u64, u32)>,
}

fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

#[inline]
fn read_varint(bytes: &[u8], pos: &mut usize) -> u64 {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let b = bytes[*pos];
        *pos += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return v;
        }
        shift += 7;
    }
}

impl Chunk {
    fn new(first_ordinal: u64, sample: u64) -> Self {
        Self {
            first_ordinal,
            first_sample: sample,
            last_sample: sample,
            count: 1,
            bytes: Vec::new(),
            checkpoints: vec![(sample, 0)],
        }
    }

    fn push(&mut self, sample: u64) {
        write_varint(&mut self.bytes, sample - self.last_sample);
        self.last_sample = sample;
        if self.count.is_multiple_of(CHECKPOINT_EVERY) {
            self.checkpoints.push((sample, self.bytes.len() as u32));
        }
        self.count += 1;
    }

    /// Number of this chunk's transitions with sample ≤ `x` (`x ≥ first_sample`).
    fn rank(&self, x: u64) -> usize {
        let cp = self.checkpoints.partition_point(|&(s, _)| s <= x) - 1;
        let (mut s, off) = self.checkpoints[cp];
        let mut pos = off as usize;
        let mut idx = cp * CHECKPOINT_EVERY;
        let end = self.count.min(idx + CHECKPOINT_EVERY);
        // `idx` is the in-chunk index of the transition at `s` (≤ x).
        while idx + 1 < end {
            let next = s + read_varint(&self.bytes, &mut pos);
            if next > x {
                break;
            }
            s = next;
            idx += 1;
        }
        idx + 1
    }

    /// Sample of the in-chunk transition `i`.
    fn sample_of(&self, i: usize) -> u64 {
        let cp = i / CHECKPOINT_EVERY;
        let (mut s, off) = self.checkpoints[cp];
        let mut pos = off as usize;
        for _ in 0..i % CHECKPOINT_EVERY {
            s += read_varint(&self.bytes, &mut pos);
        }
        s
    }

    fn heap_bytes(&self) -> usize {
        self.bytes.capacity() + self.checkpoints.capacity() * std::mem::size_of::<(u64, u32)>()
    }
}

/// Iterator returned by [`DigitalChannel::transitions_from`].
pub struct Transitions<'a> {
    ch: &'a DigitalChannel,
    chunk: usize,
    /// In-chunk index of the transition at `sample` (the next one to yield).
    idx: usize,
    /// Byte offset of the varint after `sample`.
    pos: usize,
    sample: u64,
}

impl Iterator for Transitions<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        let c = self.ch.chunks.get(self.chunk)?;
        let out = self.sample;
        self.idx += 1;
        if self.idx < c.count {
            self.sample += read_varint(&c.bytes, &mut self.pos);
        } else {
            self.chunk += 1;
            self.idx = 0;
            self.pos = 0;
            if let Some(n) = self.ch.chunks.get(self.chunk) {
                self.sample = n.first_sample;
            }
        }
        Some(out)
    }
}

/// Column classes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ColumnClass {
    Low,
    High,
    /// At least one transition inside the column.
    Transition,
    /// No data at all (outside the capture or channel inactive).
    NoData,
    /// Partly outside the stored data.
    PartialNoData,
}

/// A run of consecutive columns of the same class, `[start, end)`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ColumnRun {
    pub class: ColumnClass,
    pub start: usize,
    pub end: usize,
}

/// Merges consecutive equal classes.
pub fn merge_runs(classes: &[ColumnClass]) -> Vec<ColumnRun> {
    let mut runs: Vec<ColumnRun> = Vec::new();
    for (i, &class) in classes.iter().enumerate() {
        match runs.last_mut() {
            Some(r) if r.class == class => r.end = i + 1,
            _ => runs.push(ColumnRun { class, start: i, end: i + 1 }),
        }
    }
    runs
}

/// Summary of a sample block (the pyramid entry).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BlockSummary {
    pub first_level: bool,
    pub last_level: bool,
    pub transitions: u64,
}

/// Transitions of one digital channel.
///
/// A transition at sample `s` means the level of sample `s` differs from the
/// level of sample `s − 1`.
#[derive(Clone, Debug, Default)]
pub struct DigitalChannel {
    started: bool,
    /// Level before absolute transition ordinal 0.
    base_level: bool,
    /// First stored sample (inclusive).
    data_begin: u64,
    /// One past the last stored sample.
    data_end: u64,
    chunks: VecDeque<Chunk>,
    /// Absolute ordinal of the next transition to be pushed.
    next_ordinal: u64,
    /// Absolute ordinal of the first transition still stored.
    first_ordinal: u64,
}

impl DigitalChannel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the channel at `first_sample` with `level`.
    pub fn start(&mut self, first_sample: u64, level: bool) {
        *self = Self {
            started: true,
            base_level: level,
            data_begin: first_sample,
            data_end: first_sample + 1,
            ..Self::default()
        };
    }

    pub fn is_started(&self) -> bool {
        self.started
    }

    /// Stored sample interval `[begin, end)`, or `None` before [`start`](Self::start).
    pub fn data_range(&self) -> Option<(u64, u64)> {
        self.started.then_some((self.data_begin, self.data_end))
    }

    /// Records a transition at `sample`, which must be after the last stored sample.
    pub fn push_transition(&mut self, sample: u64) {
        assert!(self.started, "push_transition before start");
        assert!(sample >= self.data_end, "transitions must be appended in order");
        match self.chunks.back_mut() {
            Some(c) if c.count < CHUNK_TRANSITIONS => c.push(sample),
            _ => {
                if let Some(c) = self.chunks.back_mut() {
                    c.bytes.shrink_to_fit();
                    c.checkpoints.shrink_to_fit();
                }
                self.chunks.push_back(Chunk::new(self.next_ordinal, sample));
            }
        }
        self.next_ordinal += 1;
        self.data_end = sample + 1;
    }

    /// Marks samples up to `end` (exclusive) as stored, with no new transitions.
    pub fn extend_to(&mut self, end: u64) {
        assert!(self.started, "extend_to before start");
        self.data_end = self.data_end.max(end);
    }

    /// Number of transitions currently stored.
    pub fn transition_count(&self) -> u64 {
        self.next_ordinal - self.first_ordinal
    }

    /// Absolute ordinal count of transitions at samples ≤ `x`.
    fn rank_abs(&self, x: u64) -> u64 {
        let i = self.chunks.partition_point(|c| c.first_sample <= x);
        if i == 0 {
            return self.first_ordinal;
        }
        let c = &self.chunks[i - 1];
        c.first_ordinal + c.rank(x) as u64
    }

    fn level_after_rank(&self, rank_abs: u64) -> bool {
        self.base_level ^ (rank_abs & 1 == 1)
    }

    /// Number of stored transitions at samples ≤ `x`.
    pub fn rank(&self, x: u64) -> u64 {
        self.rank_abs(x) - self.first_ordinal
    }

    /// Level of sample `x`, or `None` outside the stored data.
    pub fn level_at(&self, x: u64) -> Option<bool> {
        (self.started && x >= self.data_begin && x < self.data_end).then(|| self.level_after_rank(self.rank_abs(x)))
    }

    /// Transitions at samples in `[s0, s1)`.
    pub fn transitions_in(&self, s0: u64, s1: u64) -> u64 {
        if s1 <= s0 {
            return 0;
        }
        let below = if s0 == 0 { self.first_ordinal } else { self.rank_abs(s0 - 1) };
        self.rank_abs(s1 - 1) - below
    }

    fn sample_of_ordinal(&self, ord: u64) -> u64 {
        let i = self.chunks.partition_point(|c| c.first_ordinal <= ord) - 1;
        let c = &self.chunks[i];
        c.sample_of((ord - c.first_ordinal) as usize)
    }

    /// First transition at a sample > `x`.
    pub fn next_transition(&self, x: u64) -> Option<u64> {
        let r = self.rank_abs(x);
        (r < self.next_ordinal).then(|| self.sample_of_ordinal(r))
    }

    /// Last transition at a sample < `x`, within the stored data.
    pub fn prev_transition(&self, x: u64) -> Option<u64> {
        if x == 0 {
            return None;
        }
        let r = self.rank_abs(x - 1);
        (r > self.first_ordinal).then(|| self.sample_of_ordinal(r - 1)).filter(|&s| s >= self.data_begin)
    }

    /// Transitions at samples ≥ `from`, in order, decoded sequentially.
    pub fn transitions_from(&self, from: u64) -> Transitions<'_> {
        let ord = if from == 0 { self.first_ordinal } else { self.rank_abs(from - 1) };
        let mut it = Transitions { ch: self, chunk: 0, idx: 0, pos: 0, sample: 0 };
        if ord < self.next_ordinal {
            let ci = self.chunks.partition_point(|c| c.first_ordinal <= ord) - 1;
            let c = &self.chunks[ci];
            let i = (ord - c.first_ordinal) as usize;
            let (s, off) = c.checkpoints[i / CHECKPOINT_EVERY];
            it = Transitions { ch: self, chunk: ci, idx: i - i % CHECKPOINT_EVERY, pos: off as usize, sample: s };
            while it.idx < i {
                it.sample += read_varint(&c.bytes, &mut it.pos);
                it.idx += 1;
            }
        } else {
            it.chunk = self.chunks.len();
        }
        it
    }

    /// Block summary for `[s0, s1)`, clamped to the stored data.
    pub fn summary(&self, s0: u64, s1: u64) -> Option<BlockSummary> {
        let (b, e) = self.data_range()?;
        let (s0, s1) = (s0.max(b), s1.min(e));
        if s1 <= s0 {
            return None;
        }
        Some(BlockSummary {
            first_level: self.level_at(s0)?,
            last_level: self.level_at(s1 - 1)?,
            transitions: self.transitions_in(s0 + 1, s1),
        })
    }

    /// Classifies `n` columns. Column `c` covers the continuous
    /// sample interval `[first + c·spc, first + (c+1)·spc)`; it contains the
    /// integer samples in `[⌈x0⌉, ⌈x1⌉)`.
    ///
    /// Each column boundary costs one rank query, so this is O(n log t).
    pub fn classify_columns(&self, first: f64, samples_per_col: f64, n: usize) -> Vec<ColumnClass> {
        let Some((begin, end)) = self.data_range() else {
            return vec![ColumnClass::NoData; n];
        };
        let (bf, ef) = (begin as f64, end as f64);
        // Ranks of transitions strictly before each boundary sample.
        let boundary = |c: usize| -> (f64, u64) {
            let x = first + c as f64 * samples_per_col;
            let s = x.ceil().clamp(bf, ef) as u64;
            let r = if s == 0 { self.first_ordinal } else { self.rank_abs(s - 1) };
            (x, r)
        };
        let mut out = Vec::with_capacity(n);
        let (mut x0, mut r0) = boundary(0);
        for c in 0..n {
            let (x1, r1) = boundary(c + 1);
            let class = if x1 <= bf || x0 >= ef {
                ColumnClass::NoData
            } else if x0 < bf || x1 > ef {
                ColumnClass::PartialNoData
            } else if r1 > r0 {
                ColumnClass::Transition
            } else {
                // No transition starts in the column: it holds the level of
                // the sample under its left edge.
                let s = (x0.floor() as u64).clamp(begin, end - 1);
                if self.level_after_rank(self.rank_abs(s)) { ColumnClass::High } else { ColumnClass::Low }
            };
            out.push(class);
            (x0, r0) = (x1, r1);
        }
        out
    }

    /// Drops data before `sample` ("trim oldest"): whole
    /// chunks are freed; the head chunk is kept and hidden by `data_begin`.
    pub fn trim_before(&mut self, sample: u64) {
        if !self.started || sample <= self.data_begin {
            return;
        }
        let sample = sample.min(self.data_end - 1);
        // Keep the chunk containing the transition that sets `sample`'s level.
        while self.chunks.len() > 1 && self.chunks[1].first_sample <= sample {
            let c = self.chunks.pop_front().unwrap();
            self.first_ordinal = c.first_ordinal + c.count as u64;
        }
        // The level at the new start must not depend on dropped transitions:
        // parity is computed from absolute ordinals, so it stays correct.
        self.data_begin = sample;
    }

    /// Drops data at and after `end` (trims). Rebuilds the
    /// kept transitions: a trim is a one-off on a finished capture.
    pub fn truncate(&mut self, end: u64) {
        let Some((begin, old_end)) = self.data_range() else { return };
        if end >= old_end {
            return;
        }
        let end = end.max(begin + 1);
        let level = self.level_at(begin).unwrap_or(false);
        let mut kept = DigitalChannel::new();
        kept.start(begin, level);
        for t in self.transitions_from(begin + 1).take_while(|&t| t < end) {
            kept.push_transition(t);
        }
        kept.extend_to(end);
        *self = kept;
    }

    /// Approximate heap usage.
    pub fn memory_bytes(&self) -> usize {
        self.chunks.iter().map(Chunk::heap_bytes).sum::<usize>() + self.chunks.capacity() * std::mem::size_of::<Chunk>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    #[test]
    fn truncate_keeps_levels_before_the_end() {
        let t: Vec<u64> = (1..200_000).map(|i| i * 3).collect();
        let (mut ch, levels) = build(true, &t, 700_000);
        ch.trim_before(1000);
        ch.truncate(400_001);
        assert_eq!(ch.data_range(), Some((1000, 400_001)));
        for x in (1000..400_001).step_by(7) {
            assert_eq!(ch.level_at(x), Some(levels[x as usize]), "sample {x}");
        }
        assert_eq!(ch.level_at(400_001), None);
        assert_eq!(ch.next_transition(399_999), None);
    }

    /// Builds a channel and a reference level vector from transitions.
    fn build(initial: bool, transitions: &[u64], end: u64) -> (DigitalChannel, Vec<bool>) {
        let mut ch = DigitalChannel::new();
        ch.start(0, initial);
        let mut levels = Vec::with_capacity(end as usize);
        let mut level = initial;
        for &t in transitions {
            ch.push_transition(t);
            levels.resize(t as usize, level);
            level = !level;
        }
        levels.resize(end as usize, level);
        ch.extend_to(end);
        (ch, levels)
    }

    fn random_transitions(rng: &mut StdRng, n: usize, max_gap: u64) -> Vec<u64> {
        let mut s = 0;
        (0..n)
            .map(|_| {
                s += rng.random_range(1..=max_gap);
                s
            })
            .collect()
    }

    #[test]
    fn levels_ranks_and_neighbors_match_reference() {
        let mut rng = StdRng::seed_from_u64(7);
        // Enough transitions for several chunks.
        let tr = random_transitions(&mut rng, CHUNK_TRANSITIONS * 2 + 1234, 5);
        let end = tr.last().unwrap() + 10;
        let (ch, levels) = build(true, &tr, end);
        assert_eq!(ch.transition_count(), tr.len() as u64);
        for _ in 0..20_000 {
            let x = rng.random_range(0..end);
            assert_eq!(ch.level_at(x), Some(levels[x as usize]), "level at {x}");
            let rank = tr.partition_point(|&t| t <= x) as u64;
            assert_eq!(ch.rank(x), rank);
            assert_eq!(ch.next_transition(x), tr.get(rank as usize).copied());
            let before = tr.partition_point(|&t| t < x);
            assert_eq!(ch.prev_transition(x), before.checked_sub(1).map(|i| tr[i]));
        }
        assert_eq!(ch.level_at(end), None);
    }

    #[test]
    fn transitions_iterator_matches_reference() {
        let mut rng = StdRng::seed_from_u64(11);
        let tr = random_transitions(&mut rng, CHUNK_TRANSITIONS + 500, 4);
        let end = tr.last().unwrap() + 1;
        let (ch, _) = build(false, &tr, end);
        for from in [0, 1, tr[0], tr[0] + 1, tr[CHUNK_TRANSITIONS - 1], tr[CHUNK_TRANSITIONS], end, end + 5] {
            let got: Vec<u64> = ch.transitions_from(from).collect();
            let expect: Vec<u64> = tr.iter().copied().filter(|&t| t >= from).collect();
            assert_eq!(got, expect, "from {from}");
        }
    }

    #[test]
    fn summary_is_pyramid_entry() {
        let (ch, _) = build(false, &[10, 20, 21, 40], 100);
        assert_eq!(ch.summary(0, 16), Some(BlockSummary { first_level: false, last_level: true, transitions: 1 }));
        // Block starting exactly on a transition: it sets first_level, not counted.
        assert_eq!(ch.summary(20, 40), Some(BlockSummary { first_level: false, last_level: true, transitions: 1 }));
        assert_eq!(ch.summary(200, 300), None);
    }

    #[test]
    fn classify_zoomed_in_is_square_wave() {
        // 10 samples per column → 1 column per 10 samples; spc 0.25 → 4 columns per sample.
        let (ch, _) = build(false, &[5], 10);
        let c = ch.classify_columns(0.0, 0.25, 40);
        assert_eq!(c[19], ColumnClass::Low);
        assert_eq!(c[20], ColumnClass::Transition); // the column where sample 5 starts
        assert_eq!(c[21], ColumnClass::High);
        assert_eq!(c.iter().filter(|&&k| k == ColumnClass::Transition).count(), 1);
    }

    #[test]
    fn classify_matches_reference_and_no_data() {
        let mut rng = StdRng::seed_from_u64(9);
        let tr = random_transitions(&mut rng, 5_000, 300);
        let end = tr.last().unwrap() + 50;
        let (ch, levels) = build(false, &tr, end);
        for _ in 0..200 {
            let spc = 10f64.powf(rng.random_range(-1.0..3.0));
            let first = rng.random_range(-1000.0..end as f64);
            let n = 300;
            let classes = ch.classify_columns(first, spc, n);
            for (c, &class) in classes.iter().enumerate() {
                let x0 = first + c as f64 * spc;
                let x1 = x0 + spc;
                let expect = if x1 <= 0.0 || x0 >= end as f64 {
                    ColumnClass::NoData
                } else if x0 < 0.0 || x1 > end as f64 {
                    ColumnClass::PartialNoData
                } else {
                    let (a, b) = (x0.ceil() as u64, x1.ceil() as u64);
                    if (a..b).any(|s| s > 0 && levels[s as usize] != levels[s as usize - 1]) {
                        ColumnClass::Transition
                    } else if levels[x0.floor() as usize] {
                        ColumnClass::High
                    } else {
                        ColumnClass::Low
                    }
                };
                assert_eq!(class, expect, "col {c} x0 {x0} spc {spc}");
            }
        }
    }

    #[test]
    fn merge_runs_merges() {
        use ColumnClass::*;
        let runs = merge_runs(&[Low, Low, Transition, High, High, High]);
        assert_eq!(
            runs,
            vec![
                ColumnRun { class: Low, start: 0, end: 2 },
                ColumnRun { class: Transition, start: 2, end: 3 },
                ColumnRun { class: High, start: 3, end: 6 },
            ]
        );
    }

    #[test]
    fn trim_keeps_levels() {
        let mut rng = StdRng::seed_from_u64(3);
        let tr = random_transitions(&mut rng, CHUNK_TRANSITIONS * 3, 3);
        let end = tr.last().unwrap() + 1;
        let (mut ch, levels) = build(true, &tr, end);
        let before = ch.memory_bytes();
        let cut = tr[CHUNK_TRANSITIONS * 2 + 17];
        ch.trim_before(cut);
        assert!(ch.memory_bytes() < before);
        assert_eq!(ch.data_range(), Some((cut, end)));
        assert_eq!(ch.level_at(cut - 1), None);
        for x in (cut..end).step_by(97) {
            assert_eq!(ch.level_at(x), Some(levels[x as usize]));
        }
        assert_eq!(ch.prev_transition(cut), None);
        assert_eq!(ch.transitions_in(cut, end), tr.iter().filter(|&&t| t >= cut).count() as u64);
    }

    #[test]
    fn sparse_signal_is_compact() {
        // A 1 kHz clock at 100 MS/s for 10 s: 20 000 transitions, 1 G samples.
        let mut ch = DigitalChannel::new();
        ch.start(0, false);
        for i in 1..=20_000u64 {
            ch.push_transition(i * 50_000);
        }
        ch.extend_to(1_000_000_000);
        assert!(ch.memory_bytes() < 128 * 1024, "{} bytes", ch.memory_bytes());
        let classes = ch.classify_columns(0.0, 1e9 / 2000.0, 2000);
        assert!(classes.iter().all(|&c| c == ColumnClass::Transition));
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use std::time::Instant;

    /// A column summary for 2000 px must take ≤ 0.5 ms from the index.
    /// Run with `cargo test --release --lib -- --ignored perf`.
    #[test]
    #[ignore = "timing; run in release"]
    fn classify_2000_columns_budget() {
        // 1 G samples with 20 M transitions (a 10 kHz-ish clock at 100 MS/s + jitter).
        let mut ch = DigitalChannel::new();
        ch.start(0, false);
        let mut s = 0u64;
        for i in 0..20_000_000u64 {
            s += 40 + (i * 7919) % 20;
            ch.push_transition(s);
        }
        ch.extend_to(s + 1);
        let end = s as f64;
        for (name, first, spc) in
            [("fully zoomed out", 0.0, end / 2000.0), ("mid zoom", end / 3.0, 5_000.0), ("zoomed in", end / 2.0, 0.5)]
        {
            let n = 200;
            let t0 = Instant::now();
            for k in 0..n {
                std::hint::black_box(ch.classify_columns(first + k as f64, spc, 2000));
            }
            let per = t0.elapsed().as_secs_f64() / n as f64 * 1e3;
            eprintln!("{name}: {per:.3} ms per 2000-column row ({} KB index)", ch.memory_bytes() / 1024);
            assert!(per <= 0.5, "{name} took {per:.3} ms");
        }
    }
}
