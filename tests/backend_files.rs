//! Importing files and writing sigrok sessions, through
//! the `Backend` trait.

use oxrok_ui::backend::*;
use oxrok_ui::core::ChannelId;
use oxrok_ui::oxrok::OxrokBackend;
use oxrok_ui::store::LogicIngest;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Collected {
    header: Option<StreamHeader>,
    ingest: Option<LogicIngest>,
    analog: BTreeMap<ChannelId, Vec<f32>>,
    end: Option<EndReason>,
}

#[derive(Clone, Default)]
struct Collect(Arc<Mutex<Collected>>);

impl SampleSink for Collect {
    fn on_header(&mut self, h: StreamHeader) {
        self.0.lock().unwrap().header = Some(h);
    }
    fn on_logic(&mut self, first: u64, unit_size: usize, data: &[u8]) {
        let mut c = self.0.lock().unwrap();
        let bits = c.header.as_ref().unwrap().logic_channels.iter().map(|c| c.index).collect();
        c.ingest.get_or_insert_with(|| LogicIngest::new(unit_size, bits)).push(first, data).unwrap();
    }
    fn on_analog(&mut self, ch: ChannelId, _first: u64, volts: &[f32]) {
        self.0.lock().unwrap().analog.entry(ch).or_default().extend_from_slice(volts);
    }
    fn on_hw_trigger(&mut self, _: u64) {}
    fn on_frame(&mut self, _: bool) {}
    fn on_end(&mut self, r: EndReason) {
        self.0.lock().unwrap().end = Some(r);
    }
}

fn backend() -> OxrokBackend {
    OxrokBackend::with_drivers(Some(vec!["demo".into()]))
}

fn levels(c: &Collected, k: usize) -> Vec<bool> {
    let ch = &c.ingest.as_ref().unwrap().channels()[k];
    let (b, e) = ch.data_range().unwrap();
    (b..e).map(|s| ch.level_at(s).unwrap()).collect()
}

#[test]
fn allowed_formats_are_listed() {
    let b = backend();
    let formats = b.input_formats();
    let ids: Vec<&str> = formats.iter().map(|f| f.id.as_str()).collect();
    assert!(ids.contains(&"vcd") && ids.contains(&"binary"), "{ids:?}");
    // Only oxrok's allowlist.
    assert!(ids.iter().all(|i| ["vcd", "binary"].contains(i)), "{ids:?}");
    let binary = formats.iter().find(|f| f.id == "binary").unwrap();
    assert!(binary.needs_options(), "a raw binary file has no sample rate");
    assert!(!formats.iter().find(|f| f.id == "vcd").unwrap().needs_options());
}

#[test]
fn vcd_import() {
    let b = backend();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.vcd");
    std::fs::write(
        &path,
        "$timescale 1 us $end\n$scope module top $end\n$var wire 1 ! clk $end\n$var wire 1 \" data $end\n\
         $upscope $end\n$enddefinitions $end\n#0\n0!\n1\"\n#2\n1!\n#4\n0!\n0\"\n#6\n1!\n#8\n",
    )
    .unwrap();
    assert_eq!(b.detect_format(&path).as_deref(), Some("vcd"));
    let sink = Collect::default();
    let info = b
        .import_file(
            &path,
            ImportFormat::Module("vcd".into()),
            &BTreeMap::new(),
            Box::new(sink.clone()),
            Box::new(|_| {}),
        )
        .unwrap();
    let names: Vec<&str> = info.channels.iter().map(|c| c.device_name.as_str()).collect();
    assert_eq!(names, ["clk", "data"], "a single scope is dropped");
    assert_eq!(info.sample_rate, 1_000_000);
    let c = sink.0.lock().unwrap();
    assert_eq!(c.end, Some(EndReason::Completed));
    let clk = levels(&c, 0);
    assert_eq!(&clk[..8], &[false, false, true, true, false, false, true, true]);
}

fn import_vcd(text: &str) -> Vec<String> {
    let b = backend();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.vcd");
    std::fs::write(&path, text).unwrap();
    let info = b
        .import_file(
            &path,
            ImportFormat::Module("vcd".into()),
            &BTreeMap::new(),
            Box::new(Collect::default()),
            Box::new(|_| {}),
        )
        .unwrap();
    info.channels.iter().map(|c| c.device_name.clone()).collect()
}

#[test]
fn vcd_scopes_names() {
    // Nested scopes shared by every signal: the common path goes.
    let nested = import_vcd(
        "$timescale 1 us $end\n$scope module top $end\n$scope module dut $end\n\
         $var wire 1 ! a $end\n$var wire 1 \" b $end\n$upscope $end\n$upscope $end\n\
         $enddefinitions $end\n#0\n0!\n1\"\n",
    );
    assert_eq!(nested, ["a", "b"]);
    // Different scopes keep the paths, so the names stay unique.
    let split = import_vcd(
        "$timescale 1 us $end\n$scope module top $end\n$var wire 1 ! a $end\n$upscope $end\n\
         $scope module other $end\n$var wire 1 \" b $end\n$upscope $end\n\
         $enddefinitions $end\n#0\n0!\n1\"\n",
    );
    assert_eq!(split, ["top.a", "other.b"]);
}

#[test]
fn binary_import_with_options() {
    let b = backend();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.bin");
    // 8 channels, one byte per sample: a counter.
    let bytes: Vec<u8> = (0..=255u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    let mut opts = BTreeMap::new();
    opts.insert("samplerate".to_owned(), OptionValue::Int(1_000_000));
    opts.insert("numchannels".to_owned(), OptionValue::Int(8));
    let sink = Collect::default();
    let info = b
        .import_file(&path, ImportFormat::Module("binary".into()), &opts, Box::new(sink.clone()), Box::new(|_| {}))
        .unwrap();
    assert_eq!(info.sample_rate, 1_000_000);
    let c = sink.0.lock().unwrap();
    // Bit 0 toggles every sample, bit 7 after 128 samples.
    let d0 = levels(&c, 0);
    assert_eq!(&d0[..4], &[false, true, false, true]);
    let d7 = levels(&c, 7);
    assert!(!d7[127] && d7[128]);
}

/// A counter over 4 digital channels and a ramp on one analog channel.
struct Source;

impl ExportSource for Source {
    fn sample_rate(&self) -> u64 {
        2_000_000
    }
    fn range(&self) -> (u64, u64) {
        (0, 3000)
    }
    fn digital(&self) -> Vec<(ChannelId, String)> {
        (0..4).map(|i| (ChannelId::digital(i), format!("bit{i}"))).collect()
    }
    fn analog(&self) -> Vec<(ChannelId, String)> {
        vec![(ChannelId::analog(0), "ramp".into())]
    }
    fn logic(&self, first: u64, n: usize, unit_size: usize, out: &mut Vec<u8>) {
        assert_eq!(unit_size, 1);
        out.extend((first..first + n as u64).map(|s| (s % 16) as u8));
    }
    fn analog_samples(&self, _id: ChannelId, first: u64, n: usize, out: &mut Vec<f32>) {
        out.extend((first..first + n as u64).map(|s| s as f32 * 0.001));
    }
}

#[test]
fn sr_round_trip() {
    let b = backend();
    assert!(b.can_save_sr());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.sr");
    b.save_sr_file(&path, &Source, Box::new(|_| {})).unwrap();
    let sink = Collect::default();
    let info = b
        .import_file(&path, ImportFormat::Session, &BTreeMap::new(), Box::new(sink.clone()), Box::new(|_| {}))
        .unwrap();
    assert_eq!(info.sample_rate, 2_000_000);
    let names: Vec<&str> = info.channels.iter().map(|c| c.device_name.as_str()).collect();
    assert_eq!(names, ["bit0", "bit1", "bit2", "bit3", "ramp"]);
    let c = sink.0.lock().unwrap();
    let bit3 = levels(&c, 3);
    assert_eq!(bit3.len(), 3000);
    assert!(!bit3[7] && bit3[8], "bit 3 of the counter");
    let ramp = &c.analog[&ChannelId::analog(0)];
    assert_eq!(ramp.len(), 3000);
    assert!((ramp[1234] - 1.234).abs() < 1e-4, "{}", ramp[1234]);
}
