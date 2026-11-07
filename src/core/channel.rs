//! Channel identifiers.

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ChannelKind {
    Digital,
    Analog,
    /// A signal group's row. Not a device channel: it carries no
    /// samples and is computed from its member channels.
    Bus,
}

/// Stable per device: the channel's index within its kind, in libsigrok order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChannelId {
    pub kind: ChannelKind,
    pub index: u16,
}

impl ChannelId {
    pub const fn digital(index: u16) -> Self {
        Self { kind: ChannelKind::Digital, index }
    }

    pub const fn analog(index: u16) -> Self {
        Self { kind: ChannelKind::Analog, index }
    }

    /// The row of signal group `index` (0-based).
    pub const fn bus(index: u16) -> Self {
        Self { kind: ChannelKind::Bus, index }
    }

    pub fn is_bus(&self) -> bool {
        self.kind == ChannelKind::Bus
    }

    /// Short identifier as shown in row labels, e.g. `D3`, `A0`.
    pub fn short_name(&self) -> String {
        match self.kind {
            ChannelKind::Digital => format!("D{}", self.index),
            ChannelKind::Analog => format!("A{}", self.index),
            // 1-based.
            ChannelKind::Bus => format!("G{}", self.index + 1),
        }
    }

    /// Default row name.
    pub fn default_name(&self) -> String {
        format!("Channel {}", self.index)
    }
}
