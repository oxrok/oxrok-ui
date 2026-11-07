//! Mock decoders in Rust: `uart`, `spi` and `i2c`, with
//! metadata and annotations shaped like sigrok's, so tests need no Python.

use crate::decode::{
    AnnRow, DecChannel, DecOption, DecodeSession, DecodeSpec, DecoderEngine, DecoderInfo, OptionValue, RawAnn,
};

fn ch(id: &str, name: &str, desc: &str) -> DecChannel {
    DecChannel { id: id.into(), name: name.into(), desc: desc.into() }
}

fn s(v: &str) -> OptionValue {
    OptionValue::Str(v.into())
}

fn opt(id: &str, desc: &str, default: OptionValue, values: Vec<OptionValue>) -> DecOption {
    DecOption { id: id.into(), desc: desc.into(), default, values }
}

fn classes(ids: &[(&str, &str)]) -> Vec<(String, String)> {
    ids.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

fn row(id: &str, desc: &str, classes: &[usize]) -> AnnRow {
    AnnRow { id: id.into(), desc: desc.into(), classes: classes.to_vec() }
}

fn info(id: &str, name: &str, longname: &str, desc: &str) -> DecoderInfo {
    DecoderInfo {
        id: id.into(),
        name: name.into(),
        longname: longname.into(),
        desc: desc.into(),
        license: "gplv2+".into(),
        tags: Vec::new(),
        inputs: vec!["logic".into()],
        outputs: vec![id.into()],
        channels: Vec::new(),
        opt_channels: Vec::new(),
        options: Vec::new(),
        annotations: Vec::new(),
        annotation_rows: Vec::new(),
        binary: Vec::new(),
        doc_url: Some(format!("https://sigrok.org/wiki/Protocol_decoder:{}", id.replace('_', ""))),
    }
}

fn uart_info() -> DecoderInfo {
    let mut d = info("uart", "UART", "Universal Asynchronous Receiver/Transmitter", "Asynchronous, serial bus.");
    d.tags = vec!["Embedded/industrial".into()];
    d.opt_channels = vec![ch("rx", "RX", "UART receive line"), ch("tx", "TX", "UART transmit line")];
    d.options = vec![
        opt("baudrate", "Baud rate", OptionValue::Int(115_200), vec![]),
        opt("data_bits", "Data bits", OptionValue::Int(8), (5..=9).map(OptionValue::Int).collect()),
        opt("parity", "Parity", s("none"), vec![s("none"), s("odd"), s("even"), s("zero"), s("one"), s("ignore")]),
        opt(
            "stop_bits",
            "Stop bits",
            OptionValue::Float(1.0),
            vec![OptionValue::Float(0.0), OptionValue::Float(0.5), OptionValue::Float(1.0), OptionValue::Float(1.5)],
        ),
        opt("format", "Data format", s("hex"), vec![s("ascii"), s("dec"), s("hex"), s("oct"), s("bin")]),
    ];
    // Class order as sigrok's uart decoder: 0 rx-data … 11 tx-warning, 12/13 data bits.
    d.annotations = classes(&[
        ("rx-data", "RX data"),
        ("tx-data", "TX data"),
        ("rx-start", "RX start bits"),
        ("tx-start", "TX start bits"),
        ("rx-parity-ok", "RX parity OK bits"),
        ("tx-parity-ok", "TX parity OK bits"),
        ("rx-parity-err", "RX parity error bits"),
        ("tx-parity-err", "TX parity error bits"),
        ("rx-stop", "RX stop bits"),
        ("tx-stop", "TX stop bits"),
        ("rx-warnings", "RX warnings"),
        ("tx-warnings", "TX warnings"),
        ("rx-data-bits", "RX data bits"),
        ("tx-data-bits", "TX data bits"),
        ("rx-break", "RX break"),
        ("tx-break", "TX break"),
        ("rx-packet", "RX packet"),
        ("tx-packet", "TX packet"),
    ]);
    // Row order as sigrok's uart decoder (libsigrokdecode 0.5.3).
    d.annotation_rows = vec![
        row("rx-data-bits", "RX bits", &[12]),
        row("rx-data", "RX", &[0, 2, 4, 6, 8]),
        row("rx-warnings", "RX warnings", &[10]),
        row("rx-break", "RX break", &[14]),
        row("rx-packets", "RX packets", &[16]),
        row("tx-data-bits", "TX bits", &[13]),
        row("tx-data", "TX", &[1, 3, 5, 7, 9]),
        row("tx-warnings", "TX warnings", &[11]),
        row("tx-break", "TX break", &[15]),
        row("tx-packets", "TX packets", &[17]),
    ];
    d
}

fn spi_info() -> DecoderInfo {
    let mut d = info("spi", "SPI", "Serial Peripheral Interface", "Full-duplex, synchronous, serial bus.");
    d.tags = vec!["Embedded/industrial".into()];
    d.channels = vec![ch("clk", "CLK", "Clock")];
    d.opt_channels = vec![
        ch("miso", "MISO", "Master in, slave out"),
        ch("mosi", "MOSI", "Master out, slave in"),
        ch("cs", "CS#", "Chip-select"),
    ];
    d.options = vec![
        opt("cs_polarity", "CS# polarity", s("active-low"), vec![s("active-low"), s("active-high")]),
        opt("cpol", "Clock polarity", OptionValue::Int(0), vec![OptionValue::Int(0), OptionValue::Int(1)]),
        opt("cpha", "Clock phase", OptionValue::Int(0), vec![OptionValue::Int(0), OptionValue::Int(1)]),
        opt("bitorder", "Bit order", s("msb-first"), vec![s("msb-first"), s("lsb-first")]),
        opt("wordsize", "Word size", OptionValue::Int(8), vec![]),
    ];
    d.annotations = classes(&[
        ("miso-data", "MISO data"),
        ("mosi-data", "MOSI data"),
        ("miso-bits", "MISO bits"),
        ("mosi-bits", "MOSI bits"),
        ("warnings", "Human-readable warnings"),
        ("miso-transfer", "MISO transfer"),
        ("mosi-transfer", "MOSI transfer"),
    ]);
    d.annotation_rows = vec![
        row("miso-bits", "MISO bits", &[2]),
        row("miso-data", "MISO data", &[0]),
        row("miso-transfer", "MISO transfer", &[5]),
        row("mosi-bits", "MOSI bits", &[3]),
        row("mosi-data", "MOSI data", &[1]),
        row("mosi-transfer", "MOSI transfer", &[6]),
        row("other", "Other", &[4]),
    ];
    d.binary = classes(&[("miso", "MISO"), ("mosi", "MOSI")]);
    d
}

fn i2c_info() -> DecoderInfo {
    let mut d = info("i2c", "I²C", "Inter-Integrated Circuit", "Two-wire, multi-master, serial bus.");
    d.tags = vec!["Embedded/industrial".into()];
    d.channels = vec![ch("scl", "SCL", "Serial clock line"), ch("sda", "SDA", "Serial data line")];
    d.options =
        vec![opt("address_format", "Displayed slave address format", s("shifted"), vec![s("shifted"), s("unshifted")])];
    d.annotations = classes(&[
        ("start", "Start condition"),
        ("repeat-start", "Repeat start condition"),
        ("stop", "Stop condition"),
        ("ack", "ACK"),
        ("nack", "NACK"),
        ("bit", "Data/address bit"),
        ("address-read", "Address read"),
        ("address-write", "Address write"),
        ("data-read", "Data read"),
        ("data-write", "Data write"),
        ("warnings", "Human-readable warnings"),
    ]);
    d.annotation_rows = vec![
        row("bits", "Bits", &[5]),
        row("addr-data", "Address/data", &[0, 1, 2, 3, 4, 6, 7, 8, 9]),
        row("warnings", "Warnings", &[10]),
    ];
    d.binary = classes(&[
        ("address-read", "Address read"),
        ("address-write", "Address write"),
        ("data-read", "Data read"),
        ("data-write", "Data write"),
    ]);
    d
}

/// The mock decoder library.
pub struct MockEngine {
    decoders: Vec<DecoderInfo>,
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEngine {
    pub fn new() -> Self {
        Self { decoders: vec![i2c_info(), spi_info(), uart_info()] }
    }
}

impl DecoderEngine for MockEngine {
    fn decoders(&self) -> &[DecoderInfo] {
        &self.decoders
    }

    fn start(&mut self, spec: &DecodeSpec) -> Result<Box<dyn DecodeSession>, String> {
        let int = |id: &str, d: i64| match spec.options.get(id) {
            Some(OptionValue::Int(v)) => *v,
            Some(OptionValue::Float(v)) => *v as i64,
            _ => d,
        };
        let text = |id: &str, d: &str| match spec.options.get(id) {
            Some(OptionValue::Str(v)) => v.clone(),
            _ => d.to_owned(),
        };
        if spec.samplerate == 0 {
            return Err("Cannot decode without samplerate.".into());
        }
        match spec.decoder.as_str() {
            "uart" => {
                let baud = int("baudrate", 115_200);
                if baud <= 0 {
                    return Err("Invalid baud rate".into());
                }
                let lines = ["rx", "tx"].map(|r| spec.channels.get(r).map(|&b| UartLine::new(b)));
                Ok(Box::new(Uart {
                    bit: spec.samplerate as f64 / baud as f64,
                    format: text("format", "hex"),
                    lines,
                    next: 0,
                }))
            }
            "spi" => {
                let clk = *spec.channels.get("clk").ok_or("SPI needs a clock channel")?;
                Ok(Box::new(Spi {
                    clk,
                    lines: [spec.channels.get("miso").copied(), spec.channels.get("mosi").copied()],
                    cs: spec.channels.get("cs").copied(),
                    sample_rising: int("cpol", 0) == int("cpha", 0),
                    cs_low: text("cs_polarity", "active-low") == "active-low",
                    prev: None,
                    words: [Word::default(), Word::default()],
                    next: 0,
                }))
            }
            "i2c" => {
                let scl = *spec.channels.get("scl").ok_or("I²C needs SCL")?;
                let sda = *spec.channels.get("sda").ok_or("I²C needs SDA")?;
                Ok(Box::new(I2c { scl, sda, prev: None, state: I2cState::Idle, next: 0 }))
            }
            other => Err(format!("Unknown decoder {other}")),
        }
    }
}

fn bit_of(data: &[u8], us: usize, i: usize, b: usize) -> bool {
    data[i * us + b / 8] >> (b % 8) & 1 == 1
}

fn ann(start: u64, end: u64, class: usize, texts: &[&str]) -> RawAnn {
    RawAnn { start, end, class, texts: texts.iter().map(|t| t.to_string()).collect(), value: None }
}

// ---- UART ----

#[derive(Clone, Debug)]
struct UartLine {
    bit: usize,
    prev: bool,
    /// Start of the frame being received.
    frame: Option<u64>,
    value: u32,
}

impl UartLine {
    fn new(bit: usize) -> Self {
        Self { bit, prev: true, frame: None, value: 0 }
    }
}

struct Uart {
    bit: f64,
    format: String,
    lines: [Option<UartLine>; 2],
    next: u64,
}

impl Uart {
    fn format(&self, v: u32) -> String {
        match self.format.as_str() {
            "ascii" => match v {
                0x20..=0x7E => (v as u8 as char).to_string(),
                _ => format!("[{v:02X}]"),
            },
            "dec" => v.to_string(),
            "oct" => format!("{v:03o}"),
            "bin" => format!("{v:08b}"),
            _ => format!("{v:02X}"),
        }
    }
}

impl DecodeSession for Uart {
    fn send(&mut self, start: u64, data: &[u8], us: usize) -> Result<Vec<RawAnn>, String> {
        let n = data.len() / us;
        let mut out = Vec::new();
        let bt = self.bit;
        let at = |fs: u64, k: f64| fs + (k * bt).round() as u64;
        for (dir, slot) in [0usize, 1].into_iter().zip(0..2) {
            let Some(mut line) = self.lines[slot].clone() else { continue };
            let base = [0usize, 1][dir];
            for i in 0..n {
                let s = start + i as u64;
                let level = bit_of(data, us, i, line.bit);
                match line.frame {
                    None => {
                        if line.prev && !level {
                            line.frame = Some(s);
                            line.value = 0;
                        }
                    }
                    Some(fs) => {
                        // Sample at the middle of each bit: 1..=8 data, 9 stop.
                        for k in 1..=9u32 {
                            if s == at(fs, k as f64 + 0.5) {
                                if k <= 8 {
                                    let b = level as u32;
                                    line.value |= b << (k - 1);
                                    out.push(ann(
                                        at(fs, k as f64),
                                        at(fs, k as f64 + 1.0),
                                        12 + base,
                                        &[&b.to_string()],
                                    ));
                                } else {
                                    out.push(ann(fs, at(fs, 1.0), 2 + base, &["Start bit", "Start", "S"]));
                                    let v = self.format(line.value);
                                    out.push(ann(at(fs, 1.0), at(fs, 9.0), base, &[&v]));
                                    if level {
                                        out.push(ann(at(fs, 9.0), at(fs, 10.0), 8 + base, &["Stop bit", "Stop", "T"]));
                                    } else {
                                        out.push(ann(
                                            at(fs, 9.0),
                                            at(fs, 10.0),
                                            10 + base,
                                            &["Frame error", "Frame err", "FE"],
                                        ));
                                    }
                                    line.frame = None;
                                }
                            }
                        }
                    }
                }
                line.prev = level;
            }
            self.lines[slot] = Some(line);
        }
        self.next = start + n as u64;
        // Annotations in start order, as the frame store expects mostly.
        out.sort_by_key(|a| a.start);
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<RawAnn>, String> {
        Ok(Vec::new())
    }
}

// ---- SPI ----

#[derive(Clone, Debug, Default)]
struct Word {
    first: Option<u64>,
    value: u32,
    bits: u32,
}

struct Spi {
    clk: usize,
    /// MISO, MOSI.
    lines: [Option<usize>; 2],
    cs: Option<usize>,
    sample_rising: bool,
    cs_low: bool,
    prev: Option<(bool, bool)>,
    words: [Word; 2],
    next: u64,
}

impl DecodeSession for Spi {
    fn send(&mut self, start: u64, data: &[u8], us: usize) -> Result<Vec<RawAnn>, String> {
        let n = data.len() / us;
        let mut out = Vec::new();
        for i in 0..n {
            let s = start + i as u64;
            let clk = bit_of(data, us, i, self.clk);
            let active = self.cs.is_none_or(|c| bit_of(data, us, i, c) != self.cs_low);
            if let Some((pclk, pactive)) = self.prev {
                if pactive && !active {
                    self.words = [Word::default(), Word::default()];
                }
                let edge = if self.sample_rising { !pclk && clk } else { pclk && !clk };
                if active && edge {
                    for (k, line) in self.lines.iter().enumerate() {
                        let Some(b) = line else { continue };
                        let v = bit_of(data, us, i, *b) as u32;
                        let w = &mut self.words[k];
                        w.first.get_or_insert(s);
                        w.value = w.value << 1 | v;
                        w.bits += 1;
                        out.push(ann(s, s + 1, 2 + k, &[&v.to_string()]));
                        if w.bits == 8 {
                            let mut a = ann(w.first.unwrap(), s + 1, k, &[&format!("{:02X}", w.value)]);
                            a.value = Some((w.value as u64, 8));
                            out.push(a);
                            *w = Word::default();
                        }
                    }
                }
            }
            self.prev = Some((clk, active));
        }
        self.next = start + n as u64;
        out.sort_by_key(|a| a.start);
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<RawAnn>, String> {
        Ok(Vec::new())
    }
}

// ---- I²C ----

#[derive(Clone, Debug, PartialEq)]
enum I2cState {
    Idle,
    /// Bits of the current byte; `addr` for the first byte after a start.
    Byte {
        first: Option<u64>,
        value: u32,
        bits: u32,
        addr: bool,
        read: bool,
    },
    /// The ACK/NACK bit after a byte.
    Ack {
        read: bool,
    },
}

struct I2c {
    scl: usize,
    sda: usize,
    prev: Option<(bool, bool)>,
    state: I2cState,
    next: u64,
}

impl DecodeSession for I2c {
    fn send(&mut self, start: u64, data: &[u8], us: usize) -> Result<Vec<RawAnn>, String> {
        let n = data.len() / us;
        let mut out = Vec::new();
        for i in 0..n {
            let s = start + i as u64;
            let (scl, sda) = (bit_of(data, us, i, self.scl), bit_of(data, us, i, self.sda));
            if let Some((pscl, psda)) = self.prev {
                if pscl && scl && psda && !sda {
                    let repeat = self.state != I2cState::Idle;
                    let texts: &[&str] = if repeat { &["Start repeat", "Sr"] } else { &["Start", "S"] };
                    out.push(ann(s, s + 1, if repeat { 1 } else { 0 }, texts));
                    self.state = I2cState::Byte { first: None, value: 0, bits: 0, addr: true, read: false };
                } else if pscl && scl && !psda && sda {
                    out.push(ann(s, s + 1, 2, &["Stop", "P"]));
                    self.state = I2cState::Idle;
                } else if !pscl && scl {
                    // Data is sampled on SCL rising.
                    match &mut self.state {
                        I2cState::Byte { first, value, bits, addr, read } => {
                            first.get_or_insert(s);
                            *value = *value << 1 | sda as u32;
                            *bits += 1;
                            out.push(ann(s, s + 1, 5, &[if sda { "1" } else { "0" }]));
                            if *bits == 8 {
                                let (f, v, a) = (first.unwrap(), *value, *addr);
                                let rd = if a { v & 1 == 1 } else { *read };
                                if a {
                                    let addr7 = v >> 1;
                                    let (dir, short) = if rd { ("read", "AR") } else { ("write", "AW") };
                                    let t = [
                                        format!("Address {dir}: {addr7:02X}"),
                                        format!("{short}: {addr7:02X}"),
                                        format!("{addr7:02X}"),
                                    ];
                                    out.push(RawAnn {
                                        start: f,
                                        end: s + 1,
                                        class: if rd { 6 } else { 7 },
                                        texts: t.to_vec(),
                                        value: Some((addr7 as u64, 8)),
                                    });
                                } else {
                                    let (dir, short) = if rd { ("read", "DR") } else { ("write", "DW") };
                                    let t = [
                                        format!("Data {dir}: {v:02X}"),
                                        format!("{short}: {v:02X}"),
                                        format!("{v:02X}"),
                                    ];
                                    out.push(RawAnn {
                                        start: f,
                                        end: s + 1,
                                        class: if rd { 8 } else { 9 },
                                        texts: t.to_vec(),
                                        value: Some((v as u64, 8)),
                                    });
                                }
                                self.state = I2cState::Ack { read: rd };
                            }
                        }
                        I2cState::Ack { read } => {
                            let rd = *read;
                            if sda {
                                out.push(ann(s, s + 1, 4, &["NACK", "N"]));
                            } else {
                                out.push(ann(s, s + 1, 3, &["ACK", "A"]));
                            }
                            self.state = I2cState::Byte { first: None, value: 0, bits: 0, addr: false, read: rd };
                        }
                        I2cState::Idle => {}
                    }
                }
            }
            self.prev = Some((scl, sda));
        }
        self.next = start + n as u64;
        out.sort_by_key(|a| a.start);
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<RawAnn>, String> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn spec(decoder: &str, rate: u64, chans: &[(&str, usize)], opts: &[(&str, OptionValue)]) -> DecodeSpec {
        DecodeSpec {
            decoder: decoder.into(),
            samplerate: rate,
            options: opts.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
            channels: chans.iter().map(|(k, v)| (k.to_string(), *v)).collect::<BTreeMap<_, _>>(),
        }
    }

    fn uart_bytes(bytes: &[u8], spb: usize) -> Vec<u8> {
        let mut out = vec![1u8; spb * 3];
        for &b in bytes {
            let bits = std::iter::once(0).chain((0..8).map(|i| (b >> i) & 1)).chain(std::iter::once(1));
            for bit in bits {
                out.extend(std::iter::repeat_n(bit, spb));
            }
        }
        out.extend(std::iter::repeat_n(1u8, spb * 3));
        out
    }

    fn data_of(anns: &[RawAnn], class: usize) -> Vec<String> {
        anns.iter().filter(|a| a.class == class).map(|a| a.texts[0].clone()).collect()
    }

    #[test]
    fn uart_decodes_in_chunks_with_format() {
        let mut e = MockEngine::new();
        let mut sess =
            e.start(&spec("uart", 1_152_000, &[("rx", 0)], &[("format", OptionValue::Str("ascii".into()))])).unwrap();
        let samples = uart_bytes(b"Hi!", 10);
        let mut anns = Vec::new();
        // Odd chunk sizes: state carries over.
        for (k, chunk) in samples.chunks(7).enumerate() {
            anns.extend(sess.send(k as u64 * 7, chunk, 1).unwrap());
        }
        assert_eq!(data_of(&anns, 0), ["H", "i", "!"]);
        assert_eq!(anns.iter().filter(|a| a.class == 12).count(), 24, "data bits");
        let mut sess = e.start(&spec("uart", 1_152_000, &[("rx", 0)], &[])).unwrap();
        let anns = sess.send(0, &samples, 1).unwrap();
        assert_eq!(data_of(&anns, 0), ["48", "69", "21"], "hex by default");
    }

    #[test]
    fn spi_mode0_and_i2c() {
        // SPI: bit 0 CLK, bit 1 MOSI, bit 2 CS (active low); one byte 0xA5.
        let mut v = vec![0b100u8; 4];
        for i in 0..8 {
            let b = (0xA5u8 >> (7 - i)) & 1;
            v.push(b << 1);
            v.push(b << 1 | 1);
        }
        v.extend([0b100u8; 4]);
        let mut e = MockEngine::new();
        let mut sess = e.start(&spec("spi", 1_000_000, &[("clk", 0), ("mosi", 1), ("cs", 2)], &[])).unwrap();
        let anns = sess.send(0, &v, 1).unwrap();
        assert_eq!(data_of(&anns, 1), ["A5"]);

        // I²C: bit 0 SCL, bit 1 SDA. Start, address 0x50 write, NACK, stop.
        let mut v: Vec<u8> = vec![0b11, 0b11, 0b01, 0b00];
        let byte = 0x50u8 << 1;
        for i in 0..8 {
            let b = (byte >> (7 - i)) & 1;
            v.extend([b << 1, b << 1 | 1, b << 1 | 1, b << 1]);
        }
        v.extend([0b10, 0b11, 0b11, 0b10]); // NACK
        v.extend([0b00, 0b01, 0b11, 0b11]); // stop
        let mut sess = e.start(&spec("i2c", 1_000_000, &[("scl", 0), ("sda", 1)], &[])).unwrap();
        let anns = sess.send(0, &v, 1).unwrap();
        let names: Vec<&str> = anns.iter().filter(|a| a.class != 5).map(|a| a.texts[0].as_str()).collect();
        assert_eq!(names, ["Start", "Address write: 50", "NACK", "Stop"]);
    }
}
