//! re-materializes packed logic samples from transitions.

use crate::store::DigitalChannel;

/// Units for `[s0, s1)`: bit `k` of each `ceil(n / 8)`-byte unit is the
/// level of `channels[k]`. Returns the buffer and the unit size, or `None`
/// if a channel has no data there.
pub fn pack(channels: &[&DigitalChannel], s0: u64, s1: u64) -> Option<(Vec<u8>, usize)> {
    let us = channels.len().div_ceil(8).max(1);
    let n = s1.checked_sub(s0)? as usize;
    let mut buf = vec![0u8; n * us];
    for (k, ch) in channels.iter().enumerate() {
        let (byte, bit) = (k / 8, 1u8 << (k % 8));
        let mut level = ch.level_at(s0)?;
        let mut from = s0;
        let fill = |a: u64, b: u64, buf: &mut Vec<u8>| {
            for i in (a - s0) as usize..(b - s0) as usize {
                buf[i * us + byte] |= bit;
            }
        };
        for t in ch.transitions_from(s0 + 1) {
            if t >= s1 {
                break;
            }
            if level {
                fill(from, t, &mut buf);
            }
            level = !level;
            from = t;
        }
        if level {
            fill(from, s1, &mut buf);
        }
    }
    Some((buf, us))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_two_channels() {
        let mut a = DigitalChannel::new();
        a.start(0, false);
        a.push_transition(2);
        a.push_transition(5);
        a.extend_to(8);
        let mut b = DigitalChannel::new();
        b.start(0, true);
        b.push_transition(4);
        b.extend_to(8);
        let (buf, us) = pack(&[&a, &b], 1, 7).unwrap();
        assert_eq!(us, 1);
        // Samples 1..7: a = 0 1 1 1 0 0, b = 1 1 1 0 0 0.
        assert_eq!(buf, [0b10, 0b11, 0b11, 0b01, 0b00, 0b00]);
        // Nine channels need two bytes per unit.
        let chans: Vec<&DigitalChannel> = std::iter::repeat_n(&b, 9).collect();
        let (buf, us) = pack(&chans, 0, 2).unwrap();
        assert_eq!((us, buf), (2, vec![0xFF, 0x01, 0xFF, 0x01]));
    }
}
