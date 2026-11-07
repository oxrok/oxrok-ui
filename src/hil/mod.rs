//! Hardware-in-the-loop test support (see `AGENTS.md`, "Local test hardware").
//!
//! An ESP32 signal generator ([`siggen`]) drives known waveforms into an
//! Intronix LogicPort, captured through our own [`crate::oxrok::OxrokBackend`]
//! ([`bench`]). The [`decode`] helpers check the captured bits; they are
//! deliberately minimal and independent of the protocol decoders.
//!
//! The tests only run with `LA_HIL=1` (`make test-hil`), against oxrok's
//! `intronix-logicport` driver.

pub mod bench;
pub mod decode;
pub mod siggen;

/// `LA_HIL=1` enables the hardware tests.
pub fn enabled() -> bool {
    std::env::var("LA_HIL").is_ok_and(|v| v == "1")
}

/// LogicPort channel indices (libsigrok order: D0–D31, CLK1, CLK2).
pub mod ch {
    use crate::core::ChannelId;
    pub const D0: ChannelId = ChannelId::digital(0);
    pub const D1: ChannelId = ChannelId::digital(1);
    pub const D2: ChannelId = ChannelId::digital(2);
    pub const CLK1: ChannelId = ChannelId::digital(32);
    /// UART: ESP32 TX0.
    pub const UART_TX: ChannelId = ChannelId::digital(21);
    /// I2C: GPIO22 / GPIO21.
    pub const SCL: ChannelId = ChannelId::digital(11);
    pub const SDA: ChannelId = ChannelId::digital(12);
    /// SPI: GPIO18 / GPIO23 / GPIO5 / GPIO19.
    pub const SCK: ChannelId = ChannelId::digital(28);
    pub const MOSI: ChannelId = ChannelId::digital(5);
    pub const CS: ChannelId = ChannelId::digital(25);
    pub const MISO: ChannelId = ChannelId::digital(26);
}
