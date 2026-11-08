//! Minimal reference decoders to check captured bits. Not protocol
//! analyzers: no error recovery, just enough to compare against what the
//! generator sent.

use crate::store::DigitalChannel;

/// Level of `ch` at sample `s` (panics outside the capture).
fn at(ch: &DigitalChannel, s: u64) -> bool {
    ch.level_at(s).unwrap_or_else(|| panic!("sample {s} outside the capture"))
}

/// UART 8N1, idle high: bytes whose start bit falls in the capture.
pub fn uart(ch: &DigitalChannel, rate: u64, baud: u64) -> Vec<u8> {
    let Some((begin, end)) = ch.data_range() else { return Vec::new() };
    let spb = rate as f64 / baud as f64;
    let mut out = Vec::new();
    let mut x = begin;
    // A start bit is a falling edge from the idle (high) level.
    while let Some(e) = ch.next_transition(x) {
        if at(ch, e) {
            x = e;
            continue;
        }
        let center = |bit: f64| e + (spb * (bit + 0.5)) as u64;
        if center(9.0) >= end {
            break;
        }
        let byte = (0..8).fold(0u8, |b, i| b | (at(ch, center(1.0 + i as f64)) as u8) << i);
        if at(ch, center(9.0)) {
            out.push(byte);
        }
        x = center(9.0);
    }
    out
}

/// SPI mode 0, MSB first: one `Vec<u8>` per CS-low frame (MOSI sampled on
/// SCK rising). A frame already in progress when the capture starts is skipped.
pub fn spi_mode0(sck: &DigitalChannel, data: &DigitalChannel, cs: &DigitalChannel) -> Vec<Vec<u8>> {
    let (_, end) = cs.data_range().unwrap_or((0, 0));
    let next_falling = |mut x: u64| {
        while let Some(t) = cs.next_transition(x) {
            if !at(cs, t) {
                return Some(t);
            }
            x = t;
        }
        None
    };
    let mut frames = Vec::new();
    let mut x = 0;
    while let Some(lo) = next_falling(x) {
        let hi = cs.next_transition(lo).unwrap_or(end);
        let mut bits = Vec::new();
        let mut y = lo;
        while let Some(t) = sck.next_transition(y).filter(|&t| t < hi) {
            if at(sck, t) {
                bits.push(at(data, t));
            }
            y = t;
        }
        frames.push(bits.as_chunks::<8>().0.iter().map(|b| b.iter().fold(0u8, |v, &x| v << 1 | x as u8)).collect());
        x = hi;
    }
    frames
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum I2c {
    Start,
    /// A byte and whether it was acknowledged.
    Byte(u8, bool),
    Stop,
}

/// I2C bus events: START/STOP from SDA edges while SCL is high, data
/// sampled on SCL rising edges.
pub fn i2c(scl: &DigitalChannel, sda: &DigitalChannel) -> Vec<I2c> {
    let (begin, end) = scl.data_range().unwrap_or((0, 0));
    let mut edges: Vec<(u64, bool)> = scl.transitions_from(begin + 1).map(|t| (t, true)).collect();
    edges.extend(sda.transitions_from(begin + 1).map(|t| (t, false)));
    edges.sort();
    let mut out = Vec::new();
    let mut bits: Vec<bool> = Vec::new();
    for (t, is_scl) in edges {
        if t >= end {
            break;
        }
        if is_scl {
            if at(scl, t) {
                bits.push(at(sda, t));
                if bits.len() == 9 {
                    let byte = bits[..8].iter().fold(0u8, |v, &b| v << 1 | b as u8);
                    out.push(I2c::Byte(byte, !bits[8]));
                    bits.clear();
                }
            }
        } else if at(scl, t) {
            out.push(if at(sda, t) { I2c::Stop } else { I2c::Start });
            bits.clear();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(levels: &[bool]) -> DigitalChannel {
        let mut ch = DigitalChannel::new();
        ch.start(0, levels[0]);
        for i in 1..levels.len() {
            if levels[i] != levels[i - 1] {
                ch.push_transition(i as u64);
            }
        }
        ch.extend_to(levels.len() as u64);
        ch
    }

    #[test]
    fn uart_decodes_8n1() {
        // 4 samples per bit; idle, 'A' (0x41), idle.
        let mut l = vec![true; 8];
        let mut bits = vec![false];
        bits.extend((0..8).map(|i| 0x41u8 >> i & 1 == 1));
        bits.push(true);
        for b in bits {
            l.extend([b; 4]);
        }
        l.extend([true; 8]);
        assert_eq!(uart(&wave(&l), 4, 1), b"A");
    }

    #[test]
    fn i2c_events() {
        // Sample per quarter period; build START, 0xA5 + ACK, STOP.
        let mut scl = vec![true; 4];
        let mut sda = vec![true; 4];
        let mut put = |c: bool, d: bool, n: usize| {
            scl.extend(std::iter::repeat_n(c, n));
            sda.extend(std::iter::repeat_n(d, n));
        };
        put(true, false, 2);
        put(false, false, 1);
        for i in (0..8).rev() {
            let b = 0xA5u8 >> i & 1 == 1;
            put(false, b, 1);
            put(true, b, 2);
            put(false, b, 1);
        }
        put(false, false, 1);
        put(true, false, 2);
        put(false, false, 1);
        put(false, false, 1);
        put(true, false, 2);
        put(true, true, 4);
        assert_eq!(i2c(&wave(&scl), &wave(&sda)), vec![I2c::Start, I2c::Byte(0xA5, true), I2c::Stop]);
    }
}
