//! Protocol analyzers: the decoder engine contract, the frame
//! store, sample packing, formatting, the mock decoders and the decoder
//! thread.
//!
//! No FFI here: `crate::oxrok` implements [`DecoderEngine`] over oxrok's
//! decoders; the `mock` module implements it in Rust for tests.

pub mod format;
pub mod frames;
#[cfg(feature = "mock")]
pub mod mock;
pub mod pack;
pub mod runner;

pub use crate::backend::{AnnRow, DecChannel, DecOption, DecoderInfo, OptionValue};
pub use frames::{Frame, FrameStore};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Numeric rendering.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Radix {
    /// The decoder's own text.
    Native,
    #[default]
    Hex,
    Dec,
    Bin,
    Ascii,
}

impl Radix {
    pub const ALL: [Radix; 5] = [Radix::Native, Radix::Bin, Radix::Dec, Radix::Hex, Radix::Ascii];

    pub fn label(self) -> &'static str {
        let en = match self {
            Radix::Native => "Native",
            Radix::Hex => "Hexadecimal",
            Radix::Dec => "Decimal",
            Radix::Bin => "Binary",
            Radix::Ascii => "Ascii",
        };
        crate::l10n::tr(en)
    }

    /// The value of a sigrok decoder's `format` option for this radix;
    /// `None` leaves the decoder's default.
    pub fn sigrok_format(self) -> Option<&'static str> {
        match self {
            Radix::Native => None,
            Radix::Hex => Some("hex"),
            Radix::Dec => Some("dec"),
            Radix::Bin => Some("bin"),
            Radix::Ascii => Some("ascii"),
        }
    }
}

/// How an analyzer's radix reaches its text.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RadixSupport {
    /// The decoder's `format` option.
    FormatOption,
    /// The app formats the raw values from the decoder's binary output.
    RawValues,
    /// Only [`Radix::Native`].
    NativeOnly,
}

pub fn radix_support(info: &DecoderInfo) -> RadixSupport {
    if info.options.iter().any(|o| o.id == "format") {
        RadixSupport::FormatOption
    } else if !info.binary.is_empty() {
        RadixSupport::RawValues
    } else {
        RadixSupport::NativeOnly
    }
}

/// The radix a new analyzer starts with.
pub fn default_radix(info: &DecoderInfo) -> Radix {
    if info.id == "uart" { Radix::Ascii } else { Radix::Native }
}

/// The annotation class whose raw value binary class `bin` gives.
pub fn binary_target(info: &DecoderInfo, bin: usize) -> Option<usize> {
    let id = &info.binary.get(bin)?.0;
    let data = format!("{id}-data");
    info.annotations.iter().position(|a| a.0 == *id || a.0 == data)
}

/// One `SRD_OUTPUT_BINARY` item.
#[derive(Clone, Debug, PartialEq)]
pub struct RawBinary {
    pub start: u64,
    pub end: u64,
    pub class: usize,
    pub bytes: Vec<u8>,
}

/// Gives each annotation the value of the binary item with the
/// same start whose class maps to its class. The ends may differ: I²C's
/// address annotation stops before the R/W bit, its binary item doesn't.
pub fn attach_binary(info: &DecoderInfo, anns: &mut [RawAnn], bins: &[RawBinary]) {
    if bins.is_empty() {
        return;
    }
    let mut by_key = std::collections::HashMap::new();
    for b in bins {
        if let Some(class) = binary_target(info, b.class)
            && !b.bytes.is_empty()
            && b.bytes.len() <= 8
        {
            let v = b.bytes.iter().fold(0u64, |v, &x| v << 8 | x as u64);
            by_key.insert((b.start, class), (v, 8 * b.bytes.len() as u32));
        }
    }
    for a in anns.iter_mut().filter(|a| a.value.is_none()) {
        a.value = by_key.get(&(a.start, a.class)).copied();
    }
}

/// What to decode: one decoder instance over packed samples.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodeSpec {
    pub decoder: String,
    pub samplerate: u64,
    pub options: BTreeMap<String, OptionValue>,
    /// Channel role → bit index in the packed unit.
    pub channels: BTreeMap<String, usize>,
}

/// One annotation as a decoder emits it.
#[derive(Clone, Debug, PartialEq)]
pub struct RawAnn {
    pub start: u64,
    pub end: u64,
    /// Index into the decoder's annotation classes.
    pub class: usize,
    /// Several strings, longest first.
    pub texts: Vec<String>,
    /// Raw value and its width in bits, from the binary output.
    pub value: Option<(u64, u32)>,
}

/// A running decoder instance. Not `Send`: it lives on the decoder
/// thread, where the engine was created.
pub trait DecodeSession {
    /// Feeds `[start, start + n)` as packed units; returns the
    /// annotations completed so far.
    fn send(&mut self, start: u64, data: &[u8], unit_size: usize) -> Result<Vec<RawAnn>, String>;
    /// End of data: flushes and returns the last annotations.
    fn finish(&mut self) -> Result<Vec<RawAnn>, String>;
}

/// The decoder library.
pub trait DecoderEngine {
    fn decoders(&self) -> &[DecoderInfo];
    fn start(&mut self, spec: &DecodeSpec) -> Result<Box<dyn DecodeSession>, String>;
}

/// The row of each class: the first row that lists it, else a
/// synthetic row per class (numbered after the real rows).
pub fn class_rows(info: &DecoderInfo) -> Vec<usize> {
    let n_rows = info.annotation_rows.len();
    (0..info.annotations.len())
        .map(|c| info.annotation_rows.iter().position(|r| r.classes.contains(&c)).unwrap_or(n_rows + c))
        .collect()
}

/// Every row an analyzer can show: `(id, description)`, real rows first.
pub fn all_rows(info: &DecoderInfo) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = info.annotation_rows.iter().map(|r| (r.id.clone(), r.desc.clone())).collect();
    for (c, row) in class_rows(info).into_iter().enumerate() {
        if row >= info.annotation_rows.len() {
            let (id, desc) = &info.annotations[c];
            rows.push((id.clone(), desc.clone()));
        }
    }
    rows
}

/// Error heuristic: the class id, or its row id.
pub fn is_error_class(info: &DecoderInfo, class: usize) -> bool {
    let id = info.annotations.get(class).map_or("", |a| a.0.as_str());
    let by_class = ["warning", "error", "-err", "frame-error", "parity-err"].iter().any(|k| id.contains(k));
    let row = class_rows(info).get(class).copied().and_then(|r| info.annotation_rows.get(r));
    by_class || row.is_some_and(|r| r.id == "warnings" || r.id.ends_with("-warnings"))
}

/// bit-level rows.
pub fn is_bit_row(row_id: &str) -> bool {
    row_id.contains("bit")
}

/// Rows visible by default: the data rows, not bit or warning rows.
pub fn default_visible_rows(info: &DecoderInfo) -> Vec<String> {
    let rows = all_rows(info);
    let data: Vec<String> =
        rows.iter().map(|r| r.0.clone()).filter(|id| id.contains("data") && !is_bit_row(id)).collect();
    if !data.is_empty() {
        return data;
    }
    rows.into_iter().map(|r| r.0).filter(|id| !is_bit_row(id) && !id.contains("warning")).take(2).collect()
}

/// Analyzer palette.
pub const ANALYZER_PALETTE: [[u8; 3]; 8] = [
    [0x7A, 0x9A, 0xC8],
    [0xC8, 0x7A, 0xC8],
    [0xC8, 0x7A, 0x7A],
    [0x7A, 0xC8, 0xC8],
    [0xC8, 0xB8, 0x7A],
    [0x7A, 0xC8, 0x8A],
    [0x8A, 0x9A, 0xB8],
    [0xB8, 0xA8, 0x7A],
];

/// Display names for the suggested decoders.
pub fn display_name(info: &DecoderInfo) -> String {
    match info.id.as_str() {
        "uart" => "Async Serial".into(),
        "i2c" => "I²C".into(),
        "spi" => "SPI".into(),
        _ => info.longname.clone(),
    }
}

/// The suggested decoders, in order.
pub const SUGGESTED: [&str; 3] = ["uart", "i2c", "spi"];

#[cfg(all(test, feature = "mock"))]
mod tests {
    use super::*;

    #[test]
    fn rows_errors_and_defaults() {
        let uart = mock::MockEngine::new().decoders().iter().find(|d| d.id == "uart").unwrap().clone();
        let rows = class_rows(&uart);
        let data = uart.annotations.iter().position(|a| a.0 == "rx-data").unwrap();
        assert_eq!(uart.annotation_rows[rows[data]].id, "rx-data");
        let warn = uart.annotations.iter().position(|a| a.0 == "rx-warnings").unwrap();
        assert!(is_error_class(&uart, warn));
        assert!(!is_error_class(&uart, data));
        assert_eq!(default_visible_rows(&uart), ["rx-data", "tx-data"]);
        assert_eq!(display_name(&uart), "Async Serial");
    }

    /// Where the radix comes from, and the starting radix.
    #[test]
    fn radix_support_and_defaults() {
        let e = mock::MockEngine::new();
        let get = |id: &str| e.decoders().iter().find(|d| d.id == id).unwrap().clone();
        let (uart, spi, i2c) = (get("uart"), get("spi"), get("i2c"));
        assert_eq!(radix_support(&uart), RadixSupport::FormatOption);
        assert_eq!(radix_support(&spi), RadixSupport::RawValues);
        assert_eq!(radix_support(&i2c), RadixSupport::RawValues);
        let bare = DecoderInfo { binary: Vec::new(), ..i2c.clone() };
        assert_eq!(radix_support(&bare), RadixSupport::NativeOnly);
        assert_eq!(default_radix(&uart), Radix::Ascii);
        assert_eq!(default_radix(&spi), Radix::Native);
        // SPI `mosi` → `mosi-data`; I²C ids match exactly.
        let class = |d: &DecoderInfo, id: &str| d.annotations.iter().position(|a| a.0 == id);
        assert_eq!(binary_target(&spi, 1), class(&spi, "mosi-data"));
        assert_eq!(binary_target(&i2c, 0), class(&i2c, "address-read"));
    }
}
