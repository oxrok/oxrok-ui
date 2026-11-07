//! Signal groups: a bus's options and its value.

use crate::core::ChannelId;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusFormat {
    #[default]
    Hex,
    Decimal,
    Binary,
    /// No value: the group only keeps its channels together.
    None,
}

impl BusFormat {
    pub const ALL: [BusFormat; 4] = [BusFormat::Hex, BusFormat::Decimal, BusFormat::Binary, BusFormat::None];

    pub fn label(self) -> &'static str {
        match self {
            BusFormat::Hex => "Hex",
            BusFormat::Decimal => "Decimal",
            BusFormat::Binary => "Binary",
            BusFormat::None => "No Data",
        }
    }

    /// The group has a value to show.
    pub fn has_value(self) -> bool {
        self != BusFormat::None
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endianness {
    /// Wire 0 is the most significant bit.
    #[default]
    MsbFirst,
    /// Wire 0 is the least significant bit.
    LsbFirst,
}

impl Endianness {
    pub const ALL: [Endianness; 2] = [Endianness::MsbFirst, Endianness::LsbFirst];

    pub fn label(self) -> &'static str {
        match self {
            Endianness::MsbFirst => "Wire MSB → Data MSB",
            Endianness::LsbFirst => "Wire LSB → Data LSB",
        }
    }
}

/// At most this many members: the value is a `u64`.
pub const MAX_MEMBERS: usize = 64;

/// A signal group. Its row is `ChannelId::bus(id)`; the wire
/// order is the order of the member rows under it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bus {
    pub id: u16,
    pub members: Vec<ChannelId>,
    #[serde(default)]
    pub format: BusFormat,
    #[serde(default)]
    pub endianness: Endianness,
    #[serde(default)]
    pub signed: bool,
    #[serde(default)]
    pub collapsed: bool,
}

impl Bus {
    pub fn row(&self) -> ChannelId {
        ChannelId::bus(self.id)
    }

    /// The raw N-bit value of `levels`, given in wire order.
    pub fn raw(&self, levels: impl IntoIterator<Item = bool>) -> u64 {
        raw_value(levels, self.endianness)
    }

    /// `raw` (N bits) as text.
    pub fn format(&self, raw: u64, n: usize) -> String {
        format_value(raw, n, self.format, self.signed)
    }
}

/// Levels in wire order → the N-bit value.
pub fn raw_value(levels: impl IntoIterator<Item = bool>, endianness: Endianness) -> u64 {
    let levels: Vec<bool> = levels.into_iter().take(MAX_MEMBERS).collect();
    let n = levels.len();
    levels.iter().enumerate().filter(|(_, l)| **l).fold(0u64, |v, (i, _)| {
        let bit = match endianness {
            Endianness::MsbFirst => n - 1 - i,
            Endianness::LsbFirst => i,
        };
        v | 1 << bit
    })
}

/// The value as Hex (raw pattern), Decimal (signed when
/// asked, 2's complement) or Binary (raw pattern).
pub fn format_value(raw: u64, n: usize, format: BusFormat, signed: bool) -> String {
    let n = n.clamp(1, MAX_MEMBERS);
    let mask = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
    let raw = raw & mask;
    match format {
        BusFormat::Hex => format!("0x{raw:0width$X}", width = n.div_ceil(4)),
        BusFormat::Binary => format!("0b{raw:0n$b}"),
        BusFormat::Decimal if signed && raw >> (n - 1) & 1 == 1 => {
            let v = raw as i128 - (1i128 << n);
            v.to_string()
        }
        BusFormat::Decimal => raw.to_string(),
        BusFormat::None => String::new(),
    }
}

/// The default name, `‹first id›–‹last id›`, or `‹id›`
/// for one channel.
pub fn default_name(first: &str, last: &str) -> String {
    if first == last { first.to_owned() } else { format!("{first}–{last}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_by_endianness() {
        // Wires 0..3 = 1, 1, 0, 0.
        let l = [true, true, false, false];
        assert_eq!(raw_value(l, Endianness::MsbFirst), 0b1100);
        assert_eq!(raw_value(l, Endianness::LsbFirst), 0b0011);
        assert_eq!(raw_value([], Endianness::MsbFirst), 0);
    }

    #[test]
    fn formats() {
        assert_eq!(format_value(7, 8, BusFormat::Hex, false), "0x07");
        assert_eq!(format_value(0xFF, 8, BusFormat::Hex, true), "0xFF");
        assert_eq!(format_value(0xF, 4, BusFormat::Hex, false), "0xF");
        assert_eq!(format_value(0x1F, 5, BusFormat::Hex, false), "0x1F");
        assert_eq!(format_value(3, 4, BusFormat::Binary, false), "0b0011");
        assert_eq!(format_value(3, 4, BusFormat::Decimal, false), "3");
        // 2's complement only for Decimal.
        assert_eq!(format_value(0b1111, 4, BusFormat::Decimal, true), "-1");
        assert_eq!(format_value(0b1000, 4, BusFormat::Decimal, true), "-8");
        assert_eq!(format_value(0b0111, 4, BusFormat::Decimal, true), "7");
        assert_eq!(format_value(0b1111, 4, BusFormat::Binary, true), "0b1111");
        assert_eq!(format_value(u64::MAX, 64, BusFormat::Decimal, true), "-1");
        assert_eq!(format_value(u64::MAX, 64, BusFormat::Decimal, false), u64::MAX.to_string());
        assert_eq!(default_name("D0", "D7"), "D0–D7");
    }
}
