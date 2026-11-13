//! Acceptance: scan and capture with oxrok's `demo` driver, through the
//! same `Backend` traits the UI uses.

use oxrok_ui::backend::*;
use oxrok_ui::core::ChannelId;
use oxrok_ui::oxrok::OxrokBackend;
use oxrok_ui::store::LogicIngest;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Collected {
    header: Option<StreamHeader>,
    ingest: Option<LogicIngest>,
    packets: usize,
    analog: std::collections::BTreeMap<ChannelId, Vec<f32>>,
    trigger: Option<u64>,
    end: Option<EndReason>,
}

#[derive(Clone, Default)]
struct Collect(Arc<Mutex<Collected>>);

impl SampleSink for Collect {
    fn on_header(&mut self, h: StreamHeader) {
        self.0.lock().unwrap().header = Some(h);
    }
    fn on_logic(&mut self, first_sample: u64, unit_size: usize, data: &[u8]) {
        let mut c = self.0.lock().unwrap();
        let bits = c.header.as_ref().unwrap().logic_channels.iter().map(|c| c.index).collect();
        // The packet's unit size is authoritative (it may differ from the header's hint).
        let ingest = c.ingest.get_or_insert_with(|| LogicIngest::new(unit_size, bits));
        ingest.push(first_sample, data).unwrap();
        c.packets += 1;
    }
    fn on_analog(&mut self, ch: ChannelId, first: u64, volts: &[f32]) {
        let mut c = self.0.lock().unwrap();
        let v = c.analog.entry(ch).or_default();
        assert_eq!(v.len() as u64, first, "analog samples are contiguous");
        v.extend_from_slice(volts);
    }
    fn on_hw_trigger(&mut self, sample: u64) {
        self.0.lock().unwrap().trigger = Some(sample);
    }
    fn on_frame(&mut self, _: bool) {}
    fn on_end(&mut self, reason: EndReason) {
        self.0.lock().unwrap().end = Some(reason);
    }
}

fn backend() -> OxrokBackend {
    OxrokBackend::with_drivers(Some(vec!["demo".into()]))
}

fn demo() -> (OxrokBackend, DeviceInfo) {
    let backend = backend();
    let demo = backend.scan_devices().into_iter().find(|d| d.driver == "demo").expect("demo device");
    (backend, demo)
}

fn wait_end(sink: &Collect) {
    let t0 = Instant::now();
    while sink.0.lock().unwrap().end.is_none() {
        assert!(t0.elapsed() < Duration::from_secs(60), "acquisition did not end");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn scan_lists_demo_device() {
    let (_backend, demo) = demo();
    assert!(demo.is_demo);
    assert!(demo.digital_channels().count() >= 8, "{demo:?}");
    assert_eq!(demo.digital_channels().next().unwrap().id, ChannelId::digital(0));
}

/// Rescans replace a driver's devices (same ids, no
/// duplicates); a driver with an open device is left alone, and its
/// device can still be used.
#[test]
fn rescan_is_stable_and_skips_open_drivers() {
    let (backend, demo) = demo();
    let ids = |v: Vec<DeviceInfo>| v.into_iter().map(|d| d.id).collect::<Vec<_>>();
    let first = ids(backend.scan_devices());
    for _ in 0..3 {
        assert_eq!(ids(backend.scan_devices()), first);
    }
    let dev = backend.open(&demo.id).expect("open");
    assert_eq!(ids(backend.scan_devices()), first, "open driver skipped");
    assert!(dev.capabilities().supports_continuous, "the open device still works");
    drop(dev);
    assert_eq!(ids(backend.scan_devices()), first);
}

/// A driver at a connection; an unknown driver is an error.
#[test]
fn scan_at_a_connection() {
    let (backend, _) = demo();
    let found = backend.scan_conn("demo", "tcp-raw/127.0.0.1/5555").expect("demo scan");
    assert!(found.is_empty());
    assert!(matches!(backend.scan_conn("no-such-driver", "x"), Err(BackendError::Unsupported(_))));
    let n = backend.scan_devices().len();
    assert_eq!(backend.scan_devices().len(), n, "stable after a conn scan");
    assert!(backend.library_version().is_some_and(|v| v.starts_with("oxrok ")));
}

#[test]
fn capture_fixed_samples_into_store() {
    let (backend, info) = demo();
    let mut dev = backend.open(&info.id).unwrap();
    assert!(!matches!(dev.capabilities().samplerates, Rates::List(ref l) if l.is_empty()));
    // The demo driver lists continuous sampling without get/set
    // flags (as fx2lafw does); it still streams.
    assert!(dev.capabilities().supports_continuous);
    let enabled: Vec<_> = (0..8).map(ChannelId::digital).collect();
    let applied = dev.apply(&DeviceConfig::new(enabled.clone(), 1_000_000)).unwrap();
    assert_eq!(applied.sample_rate, 1_000_000);
    assert_eq!(applied.enabled, enabled);

    let sink = Collect::default();
    dev.start(AcquisitionPlan::new(AcquisitionLimit::Samples(100_000)), Box::new(sink.clone())).unwrap();
    wait_end(&sink);

    let c = sink.0.lock().unwrap();
    assert_eq!(c.end, Some(EndReason::Completed));
    assert_eq!(c.header.as_ref().unwrap().sample_rate, 1_000_000);
    let ingest = c.ingest.as_ref().unwrap();
    assert_eq!(ingest.next_sample(), 100_000);
    let transitions: u64 = ingest.channels().iter().map(|ch| ch.transition_count()).sum();
    assert!(transitions > 0, "demo pattern should toggle");
    eprintln!(
        "demo: {} packets, {} transitions over 8 channels, unit size {}",
        c.packets,
        transitions,
        c.header.as_ref().unwrap().logic_unit_size
    );
}

/// The demo with its counter pattern: channel `k` is high for `2^k`
/// samples and low for as many.
fn counter_demo() -> Box<dyn Device> {
    let (backend, info) = demo();
    let mut dev = backend.open(&info.id).unwrap();
    let mut cfg = DeviceConfig::new((0..8).map(ChannelId::digital).collect(), 1_000_000);
    cfg.options
        .entry(OptionScope::Group("Logic".into()))
        .or_default()
        .insert("pattern".into(), ConfValue::Str("incremental".into()));
    let applied = dev.apply(&cfg).unwrap();
    assert!(applied.rejected.is_empty(), "{:?}", applied.rejected);
    dev
}

/// A pulse trigger in the device: the demo streams, and its trigger
/// takes the width of the pulse an edge ends.
#[test]
fn pulse_trigger_in_the_device() {
    use TriggerMatch::{Falling, One, Rising};
    let d = ChannelId::digital;
    let mut dev = counter_demo();
    let caps = dev.capabilities().hw_trigger.clone().expect("the demo has a trigger");
    assert!(caps.pulse && caps.capture_ratio, "{caps:?}");

    // (edge channel, edge, widths, linked high channel) and the level
    // the pulse has.
    let cases = [
        (3, Falling, (8, 8), 5, true),
        (3, Rising, (8, 8), 5, false),
        (2, Falling, (3, 5), 6, true),
        (4, Rising, (1, 16), 7, false),
    ];
    for (ch, edge, widths, linked, pulse_level) in cases {
        let sink = Collect::default();
        let plan = AcquisitionPlan {
            trigger: Some(HwTrigger::one_stage(vec![(d(ch), edge), (d(linked), One)], 50).pulse(widths.0, widths.1)),
            ..AcquisitionPlan::new(AcquisitionLimit::Samples(2_000))
        };
        dev.start(plan, Box::new(sink.clone())).unwrap();
        wait_end(&sink);
        let c = sink.0.lock().unwrap();
        let t = c.trigger.expect("the trigger fired");
        let ingest = c.ingest.as_ref().unwrap();
        let level = |ch: u16, at: u64| ingest.channels()[ch as usize].level_at(at).unwrap();
        // The edge that ends the pulse is the trigger sample, the pulse
        // is as wide as the channel's half period, and the linked
        // channel is high there.
        let width = 1u64 << ch;
        assert!(t > width, "{ch} {edge:?}: the pulse's start is in the data (trigger at {t})");
        assert_eq!(level(ch, t), !pulse_level, "{ch} {edge:?}");
        for back in 1..=width {
            assert_eq!(level(ch, t - back), pulse_level, "{ch} {edge:?}: {back} before the trigger");
        }
        assert_eq!(level(ch, t - width - 1), !pulse_level, "{ch} {edge:?}: before the pulse");
        assert!(level(linked, t), "{ch} {edge:?}: D{linked} high at the trigger");
        assert!(ingest.next_sample() > t, "{ch} {edge:?}: data after the trigger");
        drop(c);
        dev.stop().unwrap();
    }

    // No pulse of the counter is that wide: nothing arrives until Stop.
    let sink = Collect::default();
    let plan = AcquisitionPlan {
        trigger: Some(HwTrigger::one_stage(vec![(d(3), Falling)], 50).pulse(9, 15)),
        ..AcquisitionPlan::new(AcquisitionLimit::Samples(2_000))
    };
    dev.start(plan, Box::new(sink.clone())).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(sink.0.lock().unwrap().end.is_none(), "still waiting for the trigger");
    dev.stop().unwrap();
    wait_end(&sink);
    let c = sink.0.lock().unwrap();
    assert_eq!(c.trigger, None);
    assert!(c.ingest.is_none(), "no samples without a trigger");
}

#[test]
fn capture_analog_channels() {
    let (backend, info) = demo();
    let mut dev = backend.open(&info.id).unwrap();
    let enabled = vec![ChannelId::digital(0), ChannelId::analog(0), ChannelId::analog(1)];
    dev.apply(&DeviceConfig::new(enabled, 100_000)).unwrap();
    let sink = Collect::default();
    dev.start(AcquisitionPlan::new(AcquisitionLimit::Samples(10_000)), Box::new(sink.clone())).unwrap();
    wait_end(&sink);
    let c = sink.0.lock().unwrap();
    assert_eq!(c.header.as_ref().unwrap().analog_channels, vec![ChannelId::analog(0), ChannelId::analog(1)]);
    for ch in [ChannelId::analog(0), ChannelId::analog(1)] {
        let v = &c.analog[&ch];
        assert_eq!(v.len(), 10_000, "{ch:?}");
        let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |m, &x| (m.0.min(x), m.1.max(x)));
        assert!(hi > lo, "{ch:?} varies: {lo}..{hi}");
        eprintln!("demo {ch:?}: {} samples, {lo:.3}…{hi:.3} V", v.len());
    }
}

#[test]
fn continuous_capture_stops_on_request() {
    let (backend, info) = demo();
    let mut dev = backend.open(&info.id).unwrap();
    dev.apply(&DeviceConfig::new(vec![ChannelId::digital(0)], 1_000_000)).unwrap();
    let sink = Collect::default();
    dev.start(AcquisitionPlan::new(AcquisitionLimit::Continuous), Box::new(sink.clone())).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    dev.stop().unwrap();
    let c = sink.0.lock().unwrap();
    assert_eq!(c.end, Some(EndReason::Stopped));
    assert!(c.ingest.as_ref().is_some_and(|i| i.next_sample() > 0));
}

/// The demo driver's generic options, per channel group; a
/// changed pattern applies and reads back, and a bad value is refused.
#[test]
fn device_options() {
    let (backend, info) = demo();
    let mut dev = backend.open(&info.id).unwrap();
    let groups = dev.capabilities().options.clone();
    for g in &groups {
        let names: Vec<String> = g.options.iter().map(|o| format!("{} = {:?} ({:?})", o.id, o.value, o.kind)).collect();
        println!("{:?}: {names:#?}", g.scope);
    }
    let logic = groups.iter().find(|g| g.scope == OptionScope::Group("Logic".into())).expect("Logic group");
    let pattern = logic.options.iter().find(|o| o.id == "pattern").expect("pattern mode");
    let OptionKind::Choice(choices) = &pattern.kind else { panic!("pattern is a choice") };
    assert!(choices.len() > 1);
    assert_eq!(pattern.default, pattern.value);
    let other = choices.iter().find(|c| Some(*c) != pattern.value.as_ref()).unwrap().clone();

    let mut cfg = DeviceConfig::new(vec![ChannelId::digital(0)], 1_000_000);
    let scope = OptionScope::Group("Logic".into());
    cfg.options.entry(scope.clone()).or_default().insert("pattern".into(), other.clone());
    cfg.options.entry(scope.clone()).or_default().insert("nonsense".into(), ConfValue::Bool(true));
    let applied = dev.apply(&cfg).unwrap();
    assert_eq!(applied.options[&scope]["pattern"], other);
    assert!(applied.rejected.is_empty(), "unknown keys are ignored: {:?}", applied.rejected);

    cfg.options.get_mut(&scope).unwrap().insert("pattern".into(), ConfValue::Str("no such pattern".into()));
    let applied = dev.apply(&cfg).unwrap();
    assert_eq!(applied.rejected.len(), 1);
    assert_eq!(applied.rejected[0].kept, Some(other.clone()));

    // Defaults stay the first-seen values; reset restores them.
    let fresh = dev.read_options();
    let p = fresh.iter().find(|g| g.scope == scope).unwrap().options.iter().find(|o| o.id == "pattern").unwrap();
    assert_eq!(p.value, Some(other));
    cfg.options.get_mut(&scope).unwrap().insert("pattern".into(), pattern.default.clone().unwrap());
    dev.apply(&cfg).unwrap();
}

/// Live pins by a 1-sample capture, repeatedly; a normal
/// capture afterwards is unaffected.
#[test]
fn read_current_state_by_short_captures() {
    let (backend, info) = demo();
    let mut dev = backend.open(&info.id).unwrap();
    assert!(dev.capabilities().supports_state_read);
    let enabled: Vec<_> = (0..8).map(ChannelId::digital).chain([ChannelId::analog(0)]).collect();
    let cfg = DeviceConfig::new(enabled, 1_000_000);
    let t0 = Instant::now();
    for _ in 0..20 {
        let levels = dev.read_current_state(&cfg).expect("read");
        let ids: Vec<_> = levels.iter().map(|l| l.0).collect();
        assert_eq!(ids, (0..8).map(ChannelId::digital).collect::<Vec<_>>(), "digital channels only");
    }
    eprintln!("20 pin reads: {:?}", t0.elapsed());

    let sink = Collect::default();
    dev.apply(&DeviceConfig::new((0..8).map(ChannelId::digital).collect(), 1_000_000)).unwrap();
    dev.start(AcquisitionPlan::new(AcquisitionLimit::Samples(10_000)), Box::new(sink.clone())).unwrap();
    wait_end(&sink);
    let c = sink.0.lock().unwrap();
    assert_eq!(c.end, Some(EndReason::Completed));
    assert_eq!(c.ingest.as_ref().unwrap().next_sample(), 10_000);
}
