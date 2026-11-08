//! Raw-data export: CSV, Binary and VCD.

use crate::core::{CaptureTime, WallClock};
use crate::store::{AnalogChannel, DigitalChannel};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// The time column of a CSV export.
#[derive(Copy, Clone, Debug)]
pub enum TimeColumn {
    /// Seconds relative to T0, 9 decimals.
    Relative(CaptureTime),
    /// *Use ISO8601 Timestamps*: wall-clock UTC instants.
    Iso(WallClock),
}

impl TimeColumn {
    fn header(&self) -> &'static str {
        match self {
            TimeColumn::Relative(_) => "Time [s]",
            TimeColumn::Iso(_) => "Time",
        }
    }

    fn format(&self, t: CaptureTime) -> String {
        match self {
            TimeColumn::Relative(origin) => secs_9(t - *origin),
            TimeColumn::Iso(start) => crate::core::format::iso8601_utc(start, t),
        }
    }
}

/// The samples `[s0, s1)` to export (Time Range).
pub type SampleRange = (u64, u64);

/// The interval every channel has data for, within `range`.
fn common(ranges: impl Iterator<Item = Option<(u64, u64)>>, range: SampleRange) -> Option<SampleRange> {
    let ranges: Vec<Option<(u64, u64)>> = ranges.collect();
    if ranges.is_empty() || ranges.iter().any(Option::is_none) {
        return None;
    }
    let begin = ranges.iter().flatten().map(|r| r.0).max()?.max(range.0);
    let end = ranges.iter().flatten().map(|r| r.1).min()?.min(range.1);
    (end > begin).then_some((begin, end))
}

/// One exported column.
pub struct CsvColumn<'a> {
    pub name: &'a str,
    pub data: &'a DigitalChannel,
}

/// Quotes a CSV field when needed (RFC 4180).
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_owned() }
}

/// Seconds with exactly 9 decimals, rounded to the nanosecond.
pub fn secs_9(t: CaptureTime) -> String {
    let fs = t.fs();
    let ns = (fs + fs.signum() * 500_000) / 1_000_000;
    let sign = if ns < 0 { "-" } else { "" };
    let a = ns.unsigned_abs();
    format!("{sign}{}.{:09}", a / 1_000_000_000, a % 1_000_000_000)
}

#[derive(Debug)]
pub enum ExportError {
    Io(io::Error),
    Cancelled,
    NoData,
}

impl From<io::Error> for ExportError {
    fn from(e: io::Error) -> Self {
        ExportError::Io(e)
    }
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io(e) => write!(f, "{e}"),
            ExportError::Cancelled => f.write_str("export cancelled"),
            ExportError::NoData => f.write_str("no data to export"),
        }
    }
}

impl std::error::Error for ExportError {}

/// *CSV digital*: header `Time [s],<names…>`, then one row for the
/// first common sample and one per sample where any column changes. Time is
/// seconds relative to `origin` (T0) with 9 decimals. Returns the number of
/// data rows written.
pub fn write_digital_csv(
    out: impl Write,
    columns: &[CsvColumn<'_>],
    sample_rate: u64,
    time: TimeColumn,
    range: SampleRange,
    cancel: &AtomicBool,
) -> Result<u64, ExportError> {
    let (begin, end) = common(columns.iter().map(|c| c.data.data_range()), range).ok_or(ExportError::NoData)?;

    let mut w = io::BufWriter::with_capacity(1 << 20, out);
    write!(w, "{}", time.header())?;
    for c in columns {
        write!(w, ",{}", field(c.name))?;
    }
    writeln!(w)?;

    let mut levels: Vec<bool> = columns.iter().map(|c| c.data.level_at(begin).unwrap_or(false)).collect();
    let mut iters: Vec<_> = columns.iter().map(|c| c.data.transitions_from(begin + 1).peekable()).collect();
    let row = |w: &mut io::BufWriter<_>, sample: u64, levels: &[bool]| -> io::Result<()> {
        write!(w, "{}", time.format(CaptureTime::from_sample(sample, sample_rate)))?;
        for &l in levels {
            w.write_all(if l { b",1" } else { b",0" })?;
        }
        writeln!(w)
    };
    row(&mut w, begin, &levels)?;
    let mut rows = 1u64;
    loop {
        let next = iters.iter_mut().filter_map(|i| i.peek().copied()).filter(|&s| s < end).min();
        let Some(s) = next else { break };
        for (k, it) in iters.iter_mut().enumerate() {
            if it.peek() == Some(&s) {
                it.next();
                levels[k] = !levels[k];
            }
        }
        row(&mut w, s, &levels)?;
        rows += 1;
        if rows.is_multiple_of(65_536) && cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
    }
    w.flush()?;
    Ok(rows)
}

/// One exported analog column.
pub struct AnalogCsvColumn<'a> {
    pub name: &'a str,
    pub data: &'a AnalogChannel,
}

/// A value with at most `sig` significant digits, trailing zeros stripped.
pub fn sig_digits(v: f32, sig: usize) -> String {
    let v = v as f64;
    if v == 0.0 || !v.is_finite() {
        return if v.is_finite() { "0".into() } else { v.to_string() };
    }
    let decimals = (sig as i32 - 1 - v.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    if s == "-0" { "0".into() } else { s }
}

/// *CSV analog*: header `Time [s],<names…>`, then one row per
/// analog sample after downsampling by `ratio` (every Nth sample), values in
/// volts with 6 significant digits.
pub fn write_analog_csv(
    out: impl Write,
    columns: &[AnalogCsvColumn<'_>],
    sample_rate: u64,
    time: TimeColumn,
    range: SampleRange,
    ratio: u64,
    cancel: &AtomicBool,
) -> Result<u64, ExportError> {
    let (begin, end) = common(columns.iter().map(|c| c.data.data_range()), range).ok_or(ExportError::NoData)?;
    let ratio = ratio.max(1);
    let mut w = io::BufWriter::with_capacity(1 << 20, out);
    write!(w, "{}", time.header())?;
    for c in columns {
        write!(w, ",{}", field(c.name))?;
    }
    writeln!(w)?;
    let mut rows = 0u64;
    let mut s = begin;
    while s < end {
        write!(w, "{}", time.format(CaptureTime::from_sample(s, sample_rate)))?;
        for c in columns {
            let v = c.data.get(s).unwrap_or(f32::NAN);
            write!(w, ",{}", sig_digits(v, 6))?;
        }
        writeln!(w)?;
        rows += 1;
        if rows.is_multiple_of(65_536) && cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        s += ratio;
    }
    w.flush()?;
    Ok(rows)
}

/// Binary header magic and version.
pub const BIN_MAGIC: &[u8; 8] = b"LACAPBIN";
pub const BIN_VERSION: u32 = 1;

fn bin_header(w: &mut impl Write, kind: u32, rate: u64, begin_time: f64, count: u64) -> io::Result<()> {
    w.write_all(BIN_MAGIC)?;
    w.write_all(&BIN_VERSION.to_le_bytes())?;
    w.write_all(&kind.to_le_bytes())?;
    w.write_all(&(rate as f64).to_le_bytes())?;
    w.write_all(&begin_time.to_le_bytes())?;
    w.write_all(&count.to_le_bytes())
}

/// *Binary* digital: the header (type 0, `count` = transitions),
/// the initial level as u32, then each transition time as f64 seconds
/// relative to `origin`.
pub fn write_digital_bin(
    out: impl Write,
    ch: &DigitalChannel,
    sample_rate: u64,
    origin: CaptureTime,
    range: SampleRange,
    cancel: &AtomicBool,
) -> Result<u64, ExportError> {
    let (begin, end) = common(std::iter::once(ch.data_range()), range).ok_or(ExportError::NoData)?;
    let secs = |s: u64| (CaptureTime::from_sample(s, sample_rate) - origin).as_secs_f64();
    let count = ch.transitions_in(begin + 1, end);
    let mut w = io::BufWriter::with_capacity(1 << 20, out);
    bin_header(&mut w, 0, sample_rate, secs(begin), count)?;
    w.write_all(&u32::from(ch.level_at(begin).unwrap_or(false)).to_le_bytes())?;
    for (k, t) in ch.transitions_from(begin + 1).take(count as usize).enumerate() {
        w.write_all(&secs(t).to_le_bytes())?;
        if k % 65_536 == 65_535 && cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
    }
    w.flush()?;
    Ok(count)
}

/// *Binary* analog: the header (type 1, `count` = samples written),
/// the downsample ratio as u64, then the samples as f32 volts.
pub fn write_analog_bin(
    out: impl Write,
    ch: &AnalogChannel,
    sample_rate: u64,
    origin: CaptureTime,
    range: SampleRange,
    ratio: u64,
    cancel: &AtomicBool,
) -> Result<u64, ExportError> {
    let (begin, end) = common(std::iter::once(ch.data_range()), range).ok_or(ExportError::NoData)?;
    let ratio = ratio.max(1);
    let count = (end - begin).div_ceil(ratio);
    let begin_time = (CaptureTime::from_sample(begin, sample_rate) - origin).as_secs_f64();
    let mut w = io::BufWriter::with_capacity(1 << 20, out);
    bin_header(&mut w, 1, sample_rate, begin_time, count)?;
    w.write_all(&ratio.to_le_bytes())?;
    let mut s = begin;
    let mut k = 0u64;
    while s < end {
        w.write_all(&ch.get(s).unwrap_or(f32::NAN).to_le_bytes())?;
        s += ratio;
        k += 1;
        if k.is_multiple_of(65_536) && cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
    }
    w.flush()?;
    Ok(count)
}

/// *VCD* timescale: the coarsest of 1 ns, 1 ps and 1 fs in which
/// one sample period is a whole number, and that period in its units.
pub fn vcd_timescale(sample_rate: u64) -> (&'static str, u64) {
    for (unit, per_s) in [("1 ns", 1_000_000_000u128), ("1 ps", 1_000_000_000_000), ("1 fs", 1_000_000_000_000_000)] {
        if per_s % sample_rate as u128 == 0 {
            return (unit, (per_s / sample_rate as u128) as u64);
        }
    }
    ("1 fs", (1_000_000_000_000_000u128 / sample_rate as u128) as u64)
}

/// VCD identifiers: printable ASCII from `!`, in base 94.
fn vcd_id(mut k: usize) -> String {
    let mut s = String::new();
    loop {
        s.push((b'!' + (k % 94) as u8) as char);
        k /= 94;
        if k == 0 {
            return s;
        }
        k -= 1;
    }
}

/// VCD names can't hold spaces.
fn vcd_name(name: &str) -> String {
    name.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect()
}

/// *VCD*: IEEE 1364 value change dump, one `wire` per digital
/// channel and one `real` per analog channel (downsampled by `ratio`, only
/// changes written). Times count from the start of `range`.
pub fn write_vcd(
    out: impl Write,
    digital: &[CsvColumn<'_>],
    analog: &[AnalogCsvColumn<'_>],
    sample_rate: u64,
    range: SampleRange,
    ratio: u64,
    cancel: &AtomicBool,
) -> Result<u64, ExportError> {
    let all = digital.iter().map(|c| c.data.data_range()).chain(analog.iter().map(|c| c.data.data_range()));
    let (begin, end) = common(all, range).ok_or(ExportError::NoData)?;
    let ratio = ratio.max(1);
    let (unit, period) = vcd_timescale(sample_rate);
    let mut w = io::BufWriter::with_capacity(1 << 20, out);
    writeln!(w, "$comment exported by oxrok-ui $end")?;
    writeln!(w, "$timescale {unit} $end")?;
    writeln!(w, "$scope module capture $end")?;
    for (k, c) in digital.iter().enumerate() {
        writeln!(w, "$var wire 1 {} {} $end", vcd_id(k), vcd_name(c.name))?;
    }
    for (k, c) in analog.iter().enumerate() {
        writeln!(w, "$var real 64 {} {} $end", vcd_id(digital.len() + k), vcd_name(c.name))?;
    }
    writeln!(w, "$upscope $end")?;
    writeln!(w, "$enddefinitions $end")?;
    let mut levels: Vec<bool> = digital.iter().map(|c| c.data.level_at(begin).unwrap_or(false)).collect();
    let mut values: Vec<f32> = analog.iter().map(|c| c.data.get(begin).unwrap_or(f32::NAN)).collect();
    writeln!(w, "#0")?;
    writeln!(w, "$dumpvars")?;
    for (k, l) in levels.iter().enumerate() {
        writeln!(w, "{}{}", u8::from(*l), vcd_id(k))?;
    }
    for (k, v) in values.iter().enumerate() {
        writeln!(w, "r{} {}", v, vcd_id(digital.len() + k))?;
    }
    writeln!(w, "$end")?;
    let mut iters: Vec<_> = digital.iter().map(|c| c.data.transitions_from(begin + 1).peekable()).collect();
    // The next analog sample to look at.
    let mut next_analog = if analog.is_empty() { u64::MAX } else { begin + ratio };
    let mut changes = 0u64;
    loop {
        let next_digital = iters.iter_mut().filter_map(|i| i.peek().copied()).filter(|&s| s < end).min();
        let s = next_digital.unwrap_or(u64::MAX).min(next_analog);
        if s >= end {
            break;
        }
        let mut lines = Vec::new();
        for (k, it) in iters.iter_mut().enumerate() {
            if it.peek() == Some(&s) {
                it.next();
                levels[k] = !levels[k];
                lines.push(format!("{}{}", u8::from(levels[k]), vcd_id(k)));
            }
        }
        if s == next_analog {
            for (k, c) in analog.iter().enumerate() {
                let v = c.data.get(s).unwrap_or(f32::NAN);
                if v.to_bits() != values[k].to_bits() {
                    values[k] = v;
                    lines.push(format!("r{} {}", v, vcd_id(digital.len() + k)));
                }
            }
            next_analog += ratio;
        }
        if !lines.is_empty() {
            writeln!(w, "#{}", (s - begin) as u128 * period as u128)?;
            for l in lines {
                writeln!(w, "{l}")?;
            }
            changes += 1;
            if changes.is_multiple_of(65_536) && cancel.load(Ordering::Relaxed) {
                return Err(ExportError::Cancelled);
            }
        }
    }
    writeln!(w, "#{}", (end - begin) as u128 * period as u128)?;
    w.flush()?;
    Ok(changes)
}

/// Footer estimate: rows (or records) and bytes for an export.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Estimate {
    pub rows: u64,
    pub bytes: u64,
}

/// Rough sizes: CSV ≈ 13 bytes of time plus 2 per digital column per row
/// (or 10 per analog value), Binary 8 per transition and 4 per sample, VCD
/// ≈ 12 bytes per change line plus a time stamp per change.
pub fn estimate(
    format: ExportFormatKind,
    digital: &[&DigitalChannel],
    analog_samples: u64,
    analog_channels: u64,
    range: SampleRange,
) -> Estimate {
    let transitions: u64 = digital.iter().map(|c| c.transitions_in(range.0 + 1, range.1)).sum();
    let analog_values = analog_samples * analog_channels;
    match format {
        ExportFormatKind::Csv => {
            let digital_rows = if digital.is_empty() { 0 } else { transitions + 1 };
            let bytes = digital_rows * (13 + 2 * digital.len() as u64) + analog_samples * 13 + analog_values * 10;
            Estimate { rows: digital_rows + analog_samples, bytes }
        }
        ExportFormatKind::Binary => {
            Estimate { rows: transitions + analog_values, bytes: transitions * 8 + analog_values * 4 }
        }
        ExportFormatKind::Vcd => {
            Estimate { rows: transitions + analog_values, bytes: transitions * 16 + analog_values * 24 }
        }
    }
}

/// The formats this module writes (sigrok `.sr` goes through libsigrok).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExportFormatKind {
    Csv,
    Binary,
    Vcd,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: SampleRange = (0, u64::MAX);

    fn channel(initial: bool, transitions: &[u64], end: u64) -> DigitalChannel {
        let mut ch = DigitalChannel::new();
        ch.start(0, initial);
        for &t in transitions {
            ch.push_transition(t);
        }
        ch.extend_to(end);
        ch
    }

    fn export(cols: &[CsvColumn<'_>], rate: u64, origin: CaptureTime) -> String {
        let mut buf = Vec::new();
        write_digital_csv(&mut buf, cols, rate, TimeColumn::Relative(origin), ALL, &AtomicBool::new(false)).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn secs_formatting() {
        assert_eq!(secs_9(CaptureTime::ZERO), "0.000000000");
        assert_eq!(secs_9(CaptureTime::from_sample(3, 1_000_000)), "0.000003000");
        assert_eq!(secs_9(CaptureTime::from_sample(1, 3)), "0.333333333");
        assert_eq!(secs_9(-CaptureTime::from_sample(1, 1)), "-1.000000000");
    }

    #[test]
    fn rows_at_changes_plus_first_sample() {
        let a = channel(false, &[2, 5], 10);
        let b = channel(true, &[5, 7], 10);
        let csv = export(
            &[CsvColumn { name: "Channel 0", data: &a }, CsvColumn { name: "Clk, \"main\"", data: &b }],
            1_000_000,
            CaptureTime::ZERO,
        );
        assert_eq!(
            csv,
            "Time [s],Channel 0,\"Clk, \"\"main\"\"\"\n\
             0.000000000,0,1\n\
             0.000002000,1,1\n\
             0.000005000,0,0\n\
             0.000007000,0,1\n"
        );
    }

    #[test]
    fn respects_common_range_and_origin() {
        let mut a = channel(false, &[2, 4, 6, 8], 10);
        a.trim_before(5);
        let csv = export(&[CsvColumn { name: "A", data: &a }], 10, CaptureTime::from_secs_f64(0.5));
        // Starts at sample 5 (level 0 after transitions 2,4 → back to 0), T0 = 0.5 s.
        assert_eq!(csv, "Time [s],A\n0.000000000,0\n0.100000000,1\n0.300000000,0\n");
    }

    #[test]
    fn significant_digits() {
        assert_eq!(sig_digits(1.2345678, 6), "1.23457");
        assert_eq!(sig_digits(-0.000_123_456_79, 6), "-0.000123457");
        assert_eq!(sig_digits(2.5, 6), "2.5");
        assert_eq!(sig_digits(1234567.0, 6), "1234567");
        assert_eq!(sig_digits(0.0, 6), "0");
    }

    #[test]
    fn analog_rows_with_downsampling() {
        let mut a = AnalogChannel::new();
        a.push(0, &[0.0, 0.5, 1.0, 1.5, 2.0]).unwrap();
        let mut b = AnalogChannel::new();
        b.push(0, &[-1.0, -1.0, -1.0, -1.0, 3.3]).unwrap();
        let mut buf = Vec::new();
        let cols = [AnalogCsvColumn { name: "A0", data: &a }, AnalogCsvColumn { name: "A1", data: &b }];
        let rows = write_analog_csv(
            &mut buf,
            &cols,
            1000,
            TimeColumn::Relative(CaptureTime::ZERO),
            ALL,
            2,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(rows, 3);
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "Time [s],A0,A1\n0.000000000,0,-1\n0.002000000,1,-1\n0.004000000,2,3.3\n"
        );
    }

    #[test]
    fn no_data_is_an_error() {
        let empty = DigitalChannel::new();
        let r = write_digital_csv(
            Vec::new(),
            &[CsvColumn { name: "x", data: &empty }],
            1,
            TimeColumn::Relative(CaptureTime::ZERO),
            ALL,
            &AtomicBool::new(false),
        );
        assert!(matches!(r, Err(ExportError::NoData)));
    }

    #[test]
    fn range_limits_the_rows() {
        let a = channel(false, &[2, 5, 8], 10);
        let mut buf = Vec::new();
        let cols = [CsvColumn { name: "A", data: &a }];
        let t = TimeColumn::Relative(CaptureTime::ZERO);
        write_digital_csv(&mut buf, &cols, 10, t, (3, 8), &AtomicBool::new(false)).unwrap();
        // From sample 3 (level 1) to 8 exclusive: the change at 5 only.
        assert_eq!(String::from_utf8(buf).unwrap(), "Time [s],A\n0.300000000,1\n0.500000000,0\n");
    }

    #[test]
    fn iso_time_column() {
        let a = channel(true, &[], 4);
        let mut buf = Vec::new();
        let start = WallClock { unix_ms: 1_700_000_000_000, frac_ms: 0.0 };
        write_digital_csv(
            &mut buf,
            &[CsvColumn { name: "A", data: &a }],
            1000,
            TimeColumn::Iso(start),
            ALL,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "Time,A\n2023-11-14T22:13:20.000000000Z,1\n");
    }

    #[test]
    fn binary_digital_layout() {
        let a = channel(true, &[2, 5], 10);
        let mut buf = Vec::new();
        let n = write_digital_bin(&mut buf, &a, 10, CaptureTime::ZERO, ALL, &AtomicBool::new(false)).unwrap();
        assert_eq!(n, 2);
        assert_eq!(&buf[0..8], BIN_MAGIC);
        assert_eq!(u32::from_le_bytes(buf[8..12].try_into().unwrap()), BIN_VERSION);
        assert_eq!(u32::from_le_bytes(buf[12..16].try_into().unwrap()), 0);
        assert_eq!(f64::from_le_bytes(buf[16..24].try_into().unwrap()), 10.0);
        assert_eq!(f64::from_le_bytes(buf[24..32].try_into().unwrap()), 0.0);
        assert_eq!(u64::from_le_bytes(buf[32..40].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(buf[40..44].try_into().unwrap()), 1);
        assert_eq!(f64::from_le_bytes(buf[44..52].try_into().unwrap()), 0.2);
        assert_eq!(f64::from_le_bytes(buf[52..60].try_into().unwrap()), 0.5);
        assert_eq!(buf.len(), 60);
    }

    #[test]
    fn binary_analog_layout() {
        let mut a = AnalogChannel::new();
        a.push(0, &[0.0, 0.5, 1.0, 1.5, 2.0]).unwrap();
        let mut buf = Vec::new();
        let n = write_analog_bin(&mut buf, &a, 1000, CaptureTime::ZERO, ALL, 2, &AtomicBool::new(false)).unwrap();
        assert_eq!(n, 3);
        assert_eq!(u32::from_le_bytes(buf[12..16].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(buf[40..48].try_into().unwrap()), 2);
        let v: Vec<f32> = buf[48..].as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect();
        assert_eq!(v, [0.0, 1.0, 2.0]);
    }

    #[test]
    fn vcd_timescales() {
        assert_eq!(vcd_timescale(1_000_000), ("1 ns", 1000));
        assert_eq!(vcd_timescale(500_000_000), ("1 ns", 2));
        // No unit holds a 3 MHz period exactly: femtoseconds, truncated.
        assert_eq!(vcd_timescale(3_000_000), ("1 fs", 333_333_333));
        assert_eq!(vcd_timescale(400_000_000_000), ("1 fs", 2500));
        assert_eq!(vcd_timescale(250_000_000_000), ("1 ps", 4));
        assert_eq!(vcd_id(0), "!");
        assert_eq!(vcd_id(93), "~");
        assert_eq!(vcd_id(94), "!!");
    }

    #[test]
    fn vcd_dump() {
        let a = channel(false, &[2, 5], 6);
        let b = channel(true, &[5], 6);
        let mut x = AnalogChannel::new();
        x.push(0, &[1.0, 1.0, 2.5, 2.5, 2.5, 3.0]).unwrap();
        let mut buf = Vec::new();
        let n = write_vcd(
            &mut buf,
            &[CsvColumn { name: "D0", data: &a }, CsvColumn { name: "my clk", data: &b }],
            &[AnalogCsvColumn { name: "A0", data: &x }],
            1_000_000,
            ALL,
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("$timescale 1 ns $end"));
        assert!(out.contains("$var wire 1 \" my_clk $end"));
        assert!(out.contains("$var real 64 # A0 $end"));
        let body = out.split("$enddefinitions $end\n").nth(1).unwrap();
        assert_eq!(body, "#0\n$dumpvars\n0!\n1\"\nr1 #\n$end\n#2000\n1!\nr2.5 #\n#5000\n0!\n0\"\nr3 #\n#6000\n");
        assert_eq!(n, 2);
    }
}
