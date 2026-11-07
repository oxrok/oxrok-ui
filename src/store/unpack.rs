//! Unpacking libsigrok logic packets into per-channel transitions.
//!
//! A logic packet carries `unit_size` bytes per sample; bit `i` of a unit
//! (little-endian) is the logic channel with index `i` among all logic
//! channels. Consecutive units are XOR-ed to find transitions. Units that
//! equal their predecessor (on the channels we keep) are skipped eight bytes
//! at a time.

use crate::store::digital::DigitalChannel;

/// Turns a stream of logic packets into [`DigitalChannel`]s.
#[derive(Debug)]
pub struct LogicIngest {
    unit_size: usize,
    /// Bit index within the unit, per stored channel.
    bits: Vec<u16>,
    channels: Vec<DigitalChannel>,
    /// Next expected sample index.
    next_sample: u64,
    /// Previous unit, masked to `bits` (only for `unit_size ≤ 8`).
    prev_word: u64,
    /// Previous levels per channel (general path).
    prev_levels: Vec<bool>,
    started: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// The packet does not start where the previous one ended.
    Discontiguous { expected: u64, got: u64 },
    /// `data.len()` is not a multiple of the unit size.
    PartialUnit,
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestError::Discontiguous { expected, got } => {
                write!(f, "logic packet starts at sample {got}, expected {expected}")
            }
            IngestError::PartialUnit => write!(f, "logic packet length is not a multiple of the unit size"),
        }
    }
}

impl std::error::Error for IngestError {}

/// `pattern` repeated in every `unit_size`-byte lane of a u64.
fn broadcast(pattern: u64, unit_size: usize) -> u64 {
    let mut w = 0u64;
    for lane in 0..8 / unit_size {
        w |= pattern << (lane * unit_size * 8);
    }
    w
}

#[inline]
fn load_unit(data: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf[..data.len()].copy_from_slice(data);
    u64::from_le_bytes(buf)
}

#[inline]
fn bit(unit: &[u8], b: u16) -> bool {
    unit.get(b as usize / 8).is_some_and(|byte| byte >> (b % 8) & 1 == 1)
}

impl LogicIngest {
    /// `bits` are the logic channel indices to store, in output order.
    pub fn new(unit_size: usize, bits: Vec<u16>) -> Self {
        assert!(unit_size > 0, "unit_size must be > 0");
        let n = bits.len();
        Self {
            unit_size,
            bits,
            channels: vec![DigitalChannel::new(); n],
            next_sample: 0,
            prev_word: 0,
            prev_levels: vec![false; n],
            started: false,
        }
    }

    /// Wraps finished channels (a loaded file): nothing more is
    /// pushed, but the capture reads them like recorded ones.
    pub fn from_channels(unit_size: usize, bits: Vec<u16>, channels: Vec<DigitalChannel>) -> Self {
        let mut ingest = Self::new(unit_size, bits);
        ingest.next_sample = channels.iter().filter_map(|c| c.data_range()).map(|r| r.1).max().unwrap_or(0);
        ingest.started = true;
        ingest.channels = channels;
        ingest
    }

    pub fn channels(&self) -> &[DigitalChannel] {
        &self.channels
    }

    pub fn channels_mut(&mut self) -> &mut [DigitalChannel] {
        &mut self.channels
    }

    /// Approximate heap usage of all channels.
    pub fn memory_bytes(&self) -> usize {
        self.channels.iter().map(DigitalChannel::memory_bytes).sum()
    }

    pub fn into_channels(self) -> Vec<DigitalChannel> {
        self.channels
    }

    /// Next expected sample index.
    pub fn next_sample(&self) -> u64 {
        self.next_sample
    }

    fn mask(&self) -> u64 {
        self.bits.iter().fold(0, |m, &b| m | 1u64 << b)
    }

    fn fast_path(&self) -> bool {
        self.unit_size <= 8 && self.bits.iter().all(|&b| (b as usize) < self.unit_size * 8)
    }

    /// Appends one logic packet whose first unit is sample `first_sample`.
    pub fn push(&mut self, first_sample: u64, data: &[u8]) -> Result<(), IngestError> {
        let us = self.unit_size;
        if !data.len().is_multiple_of(us) {
            return Err(IngestError::PartialUnit);
        }
        if self.started && first_sample != self.next_sample {
            return Err(IngestError::Discontiguous { expected: self.next_sample, got: first_sample });
        }
        let n = data.len() / us;
        if n == 0 {
            return Ok(());
        }
        let mut start_idx = 0;
        if !self.started {
            let unit = &data[..us];
            for (k, ch) in self.channels.iter_mut().enumerate() {
                let level = bit(unit, self.bits[k]);
                ch.start(first_sample, level);
                self.prev_levels[k] = level;
            }
            if us <= 8 {
                self.prev_word = load_unit(unit) & self.mask();
            }
            self.started = true;
            start_idx = 1;
        }
        if self.fast_path() {
            self.push_fast(first_sample, data, start_idx);
        } else {
            self.push_general(first_sample, data, start_idx);
        }
        self.next_sample = first_sample + n as u64;
        for ch in &mut self.channels {
            ch.extend_to(self.next_sample);
        }
        Ok(())
    }

    fn emit(&mut self, sample: u64, diff: u64) {
        for (k, &b) in self.bits.iter().enumerate() {
            if diff >> b & 1 == 1 {
                self.channels[k].push_transition(sample);
            }
        }
    }

    fn push_fast(&mut self, first_sample: u64, data: &[u8], start_idx: usize) {
        let us = self.unit_size;
        let n = data.len() / us;
        let mask = self.mask();
        // Run skipping works when whole units fit in a u64 an integer number of times.
        let lanes = if 8 % us == 0 { 8 / us } else { 1 };
        let mask_b = broadcast(mask, us);
        let mut i = start_idx;
        while i < n {
            // Any unit size: a run of identical units is a byte sequence
            // equal to itself shifted by one unit. Compare 8 bytes at a time
            // and skip every unit the equal stretch fully covers.
            if lanes == 1 && i >= 1 {
                let mut off = i * us;
                while off + 8 <= data.len() && data[off..off + 8] == data[off - us..off - us + 8] {
                    off += 8;
                }
                let covered = off / us;
                if covered > i {
                    i = covered;
                    continue;
                }
            }
            if lanes > 1 && i + lanes <= n {
                let off = i * us;
                let w = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
                if w & mask_b == broadcast(self.prev_word, us) {
                    i += lanes;
                    continue;
                }
            }
            let w = load_unit(&data[i * us..(i + 1) * us]) & mask;
            let diff = w ^ self.prev_word;
            if diff != 0 {
                self.emit(first_sample + i as u64, diff);
                self.prev_word = w;
            }
            i += 1;
        }
    }

    fn push_general(&mut self, first_sample: u64, data: &[u8], start_idx: usize) {
        let us = self.unit_size;
        for (i, unit) in data.chunks_exact(us).enumerate().skip(start_idx) {
            for k in 0..self.bits.len() {
                let level = bit(unit, self.bits[k]);
                if level != self.prev_levels[k] {
                    self.prev_levels[k] = level;
                    self.channels[k].push_transition(first_sample + i as u64);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    /// Packed units with long constant runs and random bursts.
    fn random_units(rng: &mut StdRng, unit_size: usize, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n * unit_size);
        let mut unit: Vec<u8> = (0..unit_size).map(|_| rng.random()).collect();
        while out.len() < n * unit_size {
            let run = if rng.random_bool(0.3) { rng.random_range(1..4) } else { rng.random_range(1..200) };
            for _ in 0..run {
                if out.len() >= n * unit_size {
                    break;
                }
                out.extend_from_slice(&unit);
            }
            let byte = rng.random_range(0..unit_size);
            unit[byte] ^= 1 << rng.random_range(0..8);
        }
        out
    }

    /// Naive reference: initial level and transition samples per channel.
    fn reference(data: &[u8], unit_size: usize, bits: &[u16]) -> Vec<(bool, Vec<u64>)> {
        bits.iter()
            .map(|&b| {
                let levels: Vec<bool> = data.chunks_exact(unit_size).map(|u| bit(u, b)).collect();
                let tr = (1..levels.len()).filter(|&i| levels[i] != levels[i - 1]).map(|i| i as u64).collect();
                (levels[0], tr)
            })
            .collect()
    }

    fn transitions(ch: &DigitalChannel) -> Vec<u64> {
        let mut out = Vec::new();
        let mut x = 0;
        while let Some(t) = ch.next_transition(x) {
            out.push(t);
            x = t;
        }
        out
    }

    /// U-14: unit sizes 1, 2, 4 (plus odd sizes), channel subsets, random packet splits.
    #[test]
    fn u14_unpack_matches_reference() {
        let mut rng = StdRng::seed_from_u64(14);
        for &us in &[1usize, 2, 3, 4, 5, 6, 7, 8, 9] {
            for _ in 0..10 {
                let n = rng.random_range(1..20_000);
                let data = random_units(&mut rng, us, n);
                let all: Vec<u16> = (0..(us * 8) as u16).collect();
                let mut bits: Vec<u16> = all.iter().copied().filter(|_| rng.random_bool(0.5)).collect();
                if bits.is_empty() {
                    bits.push(0);
                }
                let mut ingest = LogicIngest::new(us, bits.clone());
                let mut pos = 0;
                while pos < n {
                    let len = rng.random_range(1..=(n - pos).min(5000));
                    ingest.push(pos as u64, &data[pos * us..(pos + len) * us]).unwrap();
                    pos += len;
                }
                assert_eq!(ingest.next_sample(), n as u64);
                let expect = reference(&data, us, &bits);
                for (k, ch) in ingest.channels().iter().enumerate() {
                    assert_eq!(ch.data_range(), Some((0, n as u64)));
                    assert_eq!(ch.level_at(0), Some(expect[k].0), "us {us} bit {}", bits[k]);
                    assert_eq!(transitions(ch), expect[k].1, "us {us} bit {}", bits[k]);
                }
            }
        }
    }

    #[test]
    fn rejects_bad_packets() {
        let mut ingest = LogicIngest::new(2, vec![0, 9]);
        assert_eq!(ingest.push(0, &[0, 0, 1]), Err(IngestError::PartialUnit));
        ingest.push(100, &[0, 0, 1, 0]).unwrap();
        assert_eq!(ingest.channels()[0].data_range(), Some((100, 102)));
        assert_eq!(ingest.channels()[0].next_transition(100), Some(101));
        assert_eq!(ingest.push(103, &[0, 0]), Err(IngestError::Discontiguous { expected: 102, got: 103 }));
    }

    #[test]
    fn unchanged_masked_bits_are_ignored() {
        // Bit 1 toggles every sample, but only bit 0 is stored.
        let data: Vec<u8> = (0..64).map(|i| ((i % 2) << 1) as u8).collect();
        let mut ingest = LogicIngest::new(1, vec![0]);
        ingest.push(0, &data).unwrap();
        assert_eq!(ingest.channels()[0].transition_count(), 0);
    }
}
