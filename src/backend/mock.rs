//! Deterministic mock backend, digital signals only for now.
//!
//! Signals are pure functions of `(seed, channel, sample, rate)`:
//! - D0: 1 kHz clock, 50 % duty, high first;
//! - D1: UART 8N1 at 115 200 baud sending `"Hello, world!\n"` at the start of
//!   every 10 ms period, idle high;
//! - D2–D5: SPI mode 0 at 1 MHz (D2 CS̅, D3 SCK, D4 MOSI, D5 MISO); one
//!   4-byte transfer per 1 ms period, starting 100 µs in, bytes derived from
//!   the seed and period index;
//! - D6 and up: slow counter bits (bit `n − 6` of the millisecond count,
//!   XOR one seed bit);
//! - A0: 1 kHz sine, ±2.5 V; A1: 100 Hz triangle, −2…+2 V; A2: 250 Hz sine,
//!   0…3.3 V; A3: 1 Hz ramp, 0…5 V. Analog shares the digital sample rate.

use crate::backend::*;
use crate::core::{ChannelId, Clock, WallClock};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

pub const UART_TEXT: &[u8] = b"Hello, world!\n";
pub const UART_BAUD: u64 = 115_200;
pub const SPI_BYTES_PER_TRANSFER: usize = 4;

/// How fast the mock produces samples.
#[derive(Clone)]
pub enum Streaming {
    /// As fast as possible.
    Instant,
    /// Paced by `clock`: samples up to `elapsed × rate` are available.
    RealTime(Arc<dyn Clock>),
}

impl std::fmt::Debug for Streaming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Streaming::Instant => f.write_str("Instant"),
            Streaming::RealTime(_) => f.write_str("RealTime"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MockOptions {
    pub seed: u64,
    pub streaming: Streaming,
    pub samplerates: Vec<u64>,
    /// Samples per logic packet.
    pub chunk_samples: usize,
    /// Injected error: `start` fails (naming `missing_firmware`'s file,
    /// when set).
    pub fail_start: bool,
    /// Injected error: the device disconnects after this many samples.
    pub disconnect_after_samples: Option<u64>,
    /// Simulate a buffered device with this memory depth: it
    /// can't stream and offers RLE. Without RLE a capture ends after this
    /// many samples; with RLE the (simulated) compression always fits.
    pub buffered: Option<u64>,
    /// With `buffered`: refuse RLE above this rate, like real drivers do.
    pub rle_max_rate: Option<u64>,
    /// List logic thresholds.
    pub thresholds: bool,
    /// Report this file `(driver, file)` missing while it isn't in
    /// `firmware_dir`, where installs go.
    pub missing_firmware: Option<(String, String)>,
    pub firmware_dir: Option<std::path::PathBuf>,
    /// A firmware download never ends by itself: it waits for Cancel.
    pub firmware_download_stalls: bool,
    /// An external unpacker is installed. A test may install one later.
    pub unpacker: Arc<AtomicBool>,
    /// The device has a trigger of its own, with these capabilities. It
    /// waits for the trigger and then delivers the data around it.
    pub hw_trigger: Option<HwTriggerCaps>,
    /// Injected error: `start` refuses a trigger with this message, as
    /// a driver that only checks it then.
    pub refuse_trigger: Option<String>,
}

impl Default for MockOptions {
    fn default() -> Self {
        Self {
            seed: 1,
            streaming: Streaming::Instant,
            samplerates: vec![
                1_000,
                10_000,
                100_000,
                1_000_000,
                2_000_000,
                4_000_000,
                8_000_000,
                12_000_000,
                16_000_000,
                24_000_000,
                50_000_000,
                100_000_000,
            ],
            chunk_samples: 1 << 16,
            fail_start: false,
            disconnect_after_samples: None,
            buffered: None,
            rle_max_rate: None,
            thresholds: false,
            missing_firmware: None,
            firmware_dir: None,
            firmware_download_stalls: false,
            unpacker: Arc::new(AtomicBool::new(true)),
            hw_trigger: None,
            refuse_trigger: None,
        }
    }
}

pub struct MockBackend {
    options: MockOptions,
    /// Plugged in and out by tests.
    devices: parking_lot::Mutex<Vec<DeviceInfo>>,
    /// USB serial ports plugged in by tests, and whether the firmware
    /// the driver talks to answers on each.
    serial_ports: parking_lot::Mutex<Vec<(SerialCandidate, bool)>>,
    /// Every `(driver, conn)` asked for: what was sent to a port.
    probed: parking_lot::Mutex<Vec<(String, String)>>,
}

fn mock_device(serial: &str, model: &str, digital: u16, analog: u16) -> DeviceInfo {
    let channels = (0..digital)
        .map(|i| ChannelInfo { id: ChannelId::digital(i), device_name: format!("D{i}") })
        .chain((0..analog).map(|i| ChannelInfo { id: ChannelId::analog(i), device_name: format!("A{i}") }))
        .collect();
    DeviceInfo {
        id: DeviceId::new("mock", serial),
        // So the sigrok-cli command runs anywhere.
        driver: "demo".into(),
        vendor: String::new(),
        model: model.into(),
        version: "1.0".into(),
        serial_or_conn: serial.into(),
        conn: None,
        is_demo: true,
        channels,
    }
}

impl MockBackend {
    pub fn new(options: MockOptions) -> Self {
        let devices = vec![
            mock_device("mock-8", "Mock 8 digital", 8, 0),
            mock_device("mock-16-4", "Mock 16 digital + 4 analog", 16, 4),
        ];
        Self {
            options,
            devices: parking_lot::Mutex::new(devices),
            serial_ports: Default::default(),
            probed: Default::default(),
        }
    }

    /// Tests: a physical (non-demo) mock device appears…
    pub fn plug(&self, serial: &str, model: &str) -> DeviceInfo {
        let info = DeviceInfo { is_demo: false, ..mock_device(serial, model, 8, 0) };
        self.devices.lock().push(info.clone());
        info
    }

    /// …or goes.
    pub fn unplug(&self, serial: &str) {
        self.devices.lock().retain(|d| d.serial_or_conn != serial);
    }

    /// Tests: a board with a USB serial port appears at `port`; `answers`
    /// says whether the firmware its driver talks to is on it.
    pub fn plug_serial(&self, port: &str, usb_serial: &str, answers: bool) -> SerialCandidate {
        let c = SerialCandidate {
            driver: "raspberrypi-pico".into(),
            port: port.into(),
            usb_serial: Some(usb_serial.into()),
            label: "Raspberry Pi Pico".into(),
            firmware: "sigrok-pico".into(),
        };
        self.serial_ports.lock().push((c.clone(), answers));
        c
    }

    /// …or goes, with the device found on it.
    pub fn unplug_serial(&self, port: &str) {
        self.serial_ports.lock().retain(|(c, _)| c.port != port);
        self.devices.lock().retain(|d| d.conn.as_deref() != Some(port));
    }

    /// Every `(driver, conn)` a scan was asked for, in order.
    pub fn probed(&self) -> Vec<(String, String)> {
        self.probed.lock().clone()
    }

    pub fn seeded(seed: u64) -> Self {
        Self::new(MockOptions { seed, ..MockOptions::default() })
    }

    pub fn options(&self) -> &MockOptions {
        &self.options
    }
}

impl Backend for MockBackend {
    fn scan_devices(&self) -> Vec<DeviceInfo> {
        self.devices.lock().clone()
    }

    fn open(&self, id: &DeviceId) -> Result<Box<dyn Device>, BackendError> {
        let info = self
            .devices
            .lock()
            .iter()
            .find(|d| &d.id == id)
            .cloned()
            .ok_or_else(|| BackendError::NotFound(id.clone()))?;
        Ok(Box::new(MockDevice::new(info, self.options.clone())))
    }

    fn decoders(&self) -> &[DecoderInfo] {
        &[]
    }

    fn serial_candidates(&self) -> Vec<SerialCandidate> {
        let devices = self.devices.lock();
        let connected = |port: &str| devices.iter().any(|d| d.conn.as_deref() == Some(port));
        self.serial_ports.lock().iter().filter(|(c, _)| !connected(&c.port)).map(|(c, _)| c.clone()).collect()
    }

    /// The `demo` driver at any connection finds one mock device there;
    /// a serial port's driver finds its board when the firmware answers.
    fn scan_conn(&self, driver: &str, conn: &str) -> Result<Vec<DeviceInfo>, BackendError> {
        // Found there before: the device has the port.
        if let Some(d) = self.devices.lock().iter().find(|d| d.conn.as_deref() == Some(conn)) {
            return Ok(vec![d.clone()]);
        }
        self.probed.lock().push((driver.to_owned(), conn.to_owned()));
        let board = self.serial_ports.lock().iter().find(|(c, _)| c.port == conn && c.driver == driver).cloned();
        if let Some((c, answers)) = board {
            if !answers {
                return Ok(Vec::new());
            }
            // Named by the board's serial number, on whatever port.
            let serial = c.usb_serial.unwrap_or_else(|| conn.to_owned());
            let info = DeviceInfo {
                id: DeviceId::new(driver, &serial),
                driver: driver.to_owned(),
                conn: Some(conn.to_owned()),
                is_demo: false,
                ..mock_device(&serial, &c.label, 8, 0)
            };
            self.devices.lock().push(info.clone());
            return Ok(vec![info]);
        }
        if driver != "demo" {
            return Ok(Vec::new());
        }
        let info = DeviceInfo { conn: Some(conn.to_owned()), ..self.plug(conn, "Mock network instrument") };
        let mut list = self.devices.lock();
        if let Some(d) = list.iter_mut().find(|d| d.serial_or_conn == conn) {
            *d = info.clone();
        }
        Ok(vec![info])
    }

    fn library_version(&self) -> Option<String> {
        Some("mock backend".into())
    }

    fn firmware_status(&self) -> FirmwareStatus {
        let dir = self.options.firmware_dir.clone();
        let missing = match &self.options.missing_firmware {
            Some((driver, file)) if !dir.as_ref().is_some_and(|d| d.join(file).is_file()) => {
                vec![MissingFirmware { driver: driver.clone(), file: file.clone() }]
            }
            _ => Vec::new(),
        };
        let paths = dir.iter().map(|d| (d.clone(), d.exists())).collect();
        FirmwareStatus { missing, paths, install_dir: dir }
    }

    fn firmware_unpacker(&self) -> bool {
        self.options.unpacker.load(Ordering::SeqCst)
    }

    /// A download without the network: reports each of the package's
    /// downloads in two halves, then writes a stand-in for every file. One
    /// that stalls stays half way until it is cancelled.
    fn fetch_firmware(
        &self,
        pkg: &'static fetch::Package,
        dest: &std::path::Path,
        progress: &dyn fetch::Progress,
    ) -> Result<fetch::Installed, fetch::Error> {
        for d in pkg.downloads {
            progress.message(&format!("Downloading {}", d.name));
            progress.download(d.name, d.size / 2, d.size);
            while self.options.firmware_download_stalls && !progress.cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            if progress.cancelled() {
                return Err(fetch::Error::Cancelled);
            }
            progress.download(d.name, d.size, d.size);
        }
        std::fs::create_dir_all(dest)?;
        let mut installed = fetch::Installed::default();
        for f in pkg.files {
            let path = dest.join(f);
            std::fs::write(&path, b"mock firmware")?;
            progress.message(&format!("Installed {}", path.display()));
            installed.files.push(path);
        }
        Ok(installed)
    }
}

/// The generic options of the mock devices.
fn mock_options(info: &DeviceInfo) -> Vec<OptionGroup> {
    let opt = |id: &str, name: &str, description: &str, kind, value: ConfValue, writable| DeviceOption {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        kind,
        unit: None,
        value: Some(value.clone()),
        default: Some(value),
        writable,
    };
    let s = |v: &str| ConfValue::Str(v.into());
    let mut groups = vec![OptionGroup {
        scope: OptionScope::Device,
        options: vec![
            opt(
                "external_clock",
                "External clock",
                "Sample on an external clock",
                OptionKind::Bool,
                ConfValue::Bool(false),
                true,
            ),
            opt(
                "pattern",
                "Pattern mode",
                "Pattern",
                OptionKind::Choice(vec![s("Sigrok logo"), s("Sine"), s("Random")]),
                s("Sigrok logo"),
                true,
            ),
            opt(
                "avg_samples",
                "Average samples",
                "Number of samples to average over",
                OptionKind::Range(NumRange { min: 1.0, max: 1024.0, step: 1.0 }),
                ConfValue::UInt(1),
                true,
            ),
            opt(
                "over_voltage_protection_active",
                "Over-voltage protection",
                "Over-voltage protection active",
                OptionKind::Free,
                s("active"),
                false,
            ),
        ],
    }];
    if info.analog_channels().next().is_some() {
        groups.push(OptionGroup {
            scope: OptionScope::Group("Analog".into()),
            options: vec![opt(
                "coupling",
                "Coupling",
                "Coupling",
                OptionKind::Choice(vec![s("DC"), s("AC")]),
                s("DC"),
                true,
            )],
        });
    }
    groups
}

/// Every option's current value, by scope.
fn current_options(groups: &[OptionGroup]) -> DeviceOptions {
    groups
        .iter()
        .map(|g| {
            let values = g.options.iter().filter_map(|o| Some((o.id.clone(), o.value.clone()?))).collect();
            (g.scope.clone(), values)
        })
        .collect()
}

struct Worker {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

pub struct MockDevice {
    info: DeviceInfo,
    caps: DeviceCaps,
    options: MockOptions,
    config: AppliedConfig,
    worker: Option<Worker>,
    /// When it opened: the live pins follow the pattern from here.
    opened: std::time::Instant,
    opened_clock: Option<std::time::Duration>,
}

impl MockDevice {
    fn new(info: DeviceInfo, options: MockOptions) -> Self {
        let caps = DeviceCaps {
            samplerates: Rates::List(options.samplerates.clone()),
            rates_coupled: RateCoupling::Shared,
            max_limit_samples: None,
            supports_continuous: options.buffered.is_none(),
            supports_limit_msec: true,
            rle: options.buffered.is_some(),
            thresholds: options.thresholds.then(|| ThresholdCaps {
                options: vec![
                    ThresholdOption { id: "1.8".into(), label: "1.8 Volts".into() },
                    ThresholdOption { id: "3.3".into(), label: "3.3+ Volts".into() },
                ],
                custom: Some(NumRange { min: 0.0, max: 5.0, step: 0.01 }),
            }),
            threshold: options.thresholds.then(|| ThresholdChoice::Preset("3.3".into())),
            options: mock_options(&info),
            hw_trigger: options.hw_trigger.clone(),
            analog_ranges: info.analog_channels().map(|c| (c.id, Some((-10.0, 10.0)))).collect(),
            supports_state_read: true,
        };
        // The first two channels of each kind start enabled.
        let enabled = info.digital_channels().take(2).chain(info.analog_channels().take(2)).map(|c| c.id).collect();
        let sample_rate = *options.samplerates.last().unwrap_or(&1_000_000);
        let config = AppliedConfig {
            enabled,
            sample_rate,
            threshold: caps.threshold.clone(),
            options: current_options(&caps.options),
            rejected: Vec::new(),
        };
        let opened_clock = match &options.streaming {
            Streaming::RealTime(clock) => Some(clock.now()),
            Streaming::Instant => None,
        };
        Self { info, caps, options, config, worker: None, opened: std::time::Instant::now(), opened_clock }
    }

    fn join_finished_worker(&mut self) {
        if self.worker.as_ref().is_some_and(|w| w.handle.is_finished()) {
            let _ = self.worker.take().unwrap().handle.join();
        }
    }
}

impl Device for MockDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn capabilities(&self) -> &DeviceCaps {
        &self.caps
    }

    fn apply(&mut self, cfg: &DeviceConfig) -> Result<AppliedConfig, BackendError> {
        self.join_finished_worker();
        if self.worker.is_some() {
            return Err(BackendError::Busy);
        }
        // Snap to the nearest supported rate, like real hardware.
        let sample_rate = self
            .options
            .samplerates
            .iter()
            .copied()
            .min_by_key(|r| r.abs_diff(cfg.sample_rate))
            .ok_or_else(|| BackendError::Device("no sample rates".into()))?;
        let enabled = cfg.enabled.iter().copied().filter(|id| self.info.channels.iter().any(|c| c.id == *id)).collect();
        let threshold = match (&cfg.threshold, &self.caps.thresholds) {
            (Some(ThresholdChoice::Custom(v)), Some(ThresholdCaps { custom: Some(r), .. })) => {
                Some(ThresholdChoice::Custom(r.snap(*v)))
            }
            (Some(ThresholdChoice::Preset(id)), Some(t)) if t.options.iter().any(|o| &o.id == id) => {
                Some(ThresholdChoice::Preset(id.clone()))
            }
            _ => self.config.threshold.clone(),
        };
        // Refuse values outside a range or a list, keep the old one.
        let mut rejected = Vec::new();
        for group in &mut self.caps.options {
            let Some(wanted) = cfg.options.get(&group.scope) else { continue };
            for o in group.options.iter_mut().filter(|o| o.writable) {
                let Some(v) = wanted.get(&o.id) else { continue };
                let ok = match &o.kind {
                    OptionKind::Range(r) => v.as_f64().is_some_and(|f| f >= r.min && f <= r.max),
                    OptionKind::Choice(c) => c.contains(v),
                    OptionKind::Bool => matches!(v, ConfValue::Bool(_)),
                    OptionKind::Free => true,
                };
                if ok {
                    o.value = Some(v.clone());
                } else {
                    rejected.push(Rejected {
                        scope: group.scope.clone(),
                        id: o.id.clone(),
                        requested: v.clone(),
                        kept: o.value.clone(),
                    });
                }
            }
        }
        self.config =
            AppliedConfig { enabled, sample_rate, threshold, options: current_options(&self.caps.options), rejected };
        Ok(self.config.clone())
    }

    /// The pattern's levels at the time elapsed since opening.
    fn read_current_state(&mut self, cfg: &DeviceConfig) -> Result<Vec<(ChannelId, bool)>, BackendError> {
        self.apply(cfg)?;
        let rate = self.config.sample_rate;
        let elapsed = match (&self.options.streaming, self.opened_clock) {
            (Streaming::RealTime(clock), Some(t0)) => clock.now().saturating_sub(t0),
            _ => self.opened.elapsed(),
        };
        let s = (elapsed.as_secs_f64() * rate as f64) as u64;
        Ok(self
            .config
            .enabled
            .iter()
            .filter(|c| c.kind == crate::core::ChannelKind::Digital)
            .map(|c| (*c, level(self.options.seed, c.index, s, rate)))
            .collect())
    }

    fn start(&mut self, plan: AcquisitionPlan, mut sink: Box<dyn SampleSink>) -> Result<(), BackendError> {
        self.join_finished_worker();
        if self.worker.is_some() {
            return Err(BackendError::Busy);
        }
        if self.options.fail_start {
            // With a missing firmware file, as a device that loads its
            // firmware when the acquisition starts.
            let error = match &self.options.missing_firmware {
                Some((_, file)) => format!("firmware not found: {file}"),
                None => "injected start failure".into(),
            };
            return Err(BackendError::Device(error));
        }
        let rate = self.config.sample_rate;
        let n_digital = self.info.digital_channels().count();
        let unit_size = n_digital.div_ceil(8).max(1);
        // The device's own trigger: oxrok's software one, on the bits of
        // the logic units.
        let armed = match &plan.trigger {
            None => None,
            Some(t) => {
                let Some(caps) = &self.caps.hw_trigger else {
                    return Err(BackendError::Unsupported("hardware triggers on the mock device".into()));
                };
                if let Some(why) = &self.options.refuse_trigger {
                    return Err(BackendError::Device(why.clone()));
                }
                let bit = |c: ChannelId| (c.kind == crate::core::ChannelKind::Digital).then_some(c.index as usize);
                let trigger = crate::oxrok::trigger::to_oxrok(t, bit).map_err(BackendError::Device)?;
                let features = ::oxrok::core::TriggerFeatures {
                    count: caps.count,
                    consecutive: caps.consecutive,
                    duration: caps.duration,
                    pulse: caps.pulse,
                    range: caps.range,
                    or: caps.or,
                    and: caps.and,
                    max_stages: caps.max_stages,
                    holdoff: caps.holdoff,
                };
                trigger.check(&features).map_err(|e| BackendError::Device(e.to_string()))?;
                let matcher = ::oxrok::core::SoftTrigger::new(&trigger, unit_size)
                    .ok_or_else(|| BackendError::Device("a trigger without stages".into()))?;
                Some((matcher, t.pre_trigger_percent))
            }
        };
        let of_kind = |k| self.config.enabled.iter().copied().filter(|c| c.kind == k).collect::<Vec<ChannelId>>();
        let logic_channels = of_kind(crate::core::ChannelKind::Digital);
        let analog_channels = of_kind(crate::core::ChannelKind::Analog);
        let header = StreamHeader {
            start: WallClock::now(),
            sample_rate: rate,
            logic_unit_size: unit_size,
            logic_channels: logic_channels.clone(),
            analog_channels: analog_channels.clone(),
        };
        let mut limit = match plan.limit {
            AcquisitionLimit::Continuous => None,
            AcquisitionLimit::Samples(n) => Some(n),
            AcquisitionLimit::Millis(ms) => Some((ms as u128 * rate as u128).div_ceil(1000) as u64),
        };
        if plan.rle && !self.caps.rle {
            return Err(BackendError::Unsupported("run-length compression on this device".into()));
        }
        if plan.rle && self.options.rle_max_rate.is_some_and(|max| rate > max) {
            return Err(BackendError::Device("invalid argument".into()));
        }
        // A buffered device without compression stops when its memory is full.
        if let Some(depth) = self.options.buffered
            && !plan.rle
        {
            limit = Some(limit.map_or(depth, |l| l.min(depth)));
        }
        let mut generator = Generator::new(self.options.seed, rate, unit_size, &logic_channels);
        generator.analog = analog_channels;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let options = self.options.clone();
        let started_at = match &options.streaming {
            Streaming::RealTime(clock) => clock.now(),
            Streaming::Instant => Duration::ZERO,
        };
        let handle = std::thread::Builder::new()
            .name("mock-acquisition".into())
            .spawn(move || {
                sink.on_header(header);
                let reason = match armed {
                    Some((matcher, pre)) => {
                        let wait = Wait { matcher, pre_percent: pre, limit: limit.unwrap_or(u64::MAX) };
                        stream_triggered(&generator, &options, started_at, wait, &stop_flag, sink.as_mut())
                    }
                    None => stream(&generator, &options, started_at, limit, &stop_flag, sink.as_mut()),
                };
                sink.on_end(reason);
            })
            .map_err(|e| BackendError::Device(e.to_string()))?;
        self.worker = Some(Worker { stop, handle });
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        if let Some(w) = self.worker.take() {
            w.stop.store(true, Ordering::SeqCst);
            let _ = w.handle.join();
        }
        Ok(())
    }
}

impl Drop for MockDevice {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn stream(
    generator: &Generator,
    options: &MockOptions,
    start: Duration,
    limit: Option<u64>,
    stop: &AtomicBool,
    sink: &mut dyn SampleSink,
) -> EndReason {
    let chunk = options.chunk_samples.max(1) as u64;
    let mut buf = Vec::new();
    let mut volts = Vec::new();
    let mut s = 0u64;
    loop {
        if stop.load(Ordering::SeqCst) {
            return EndReason::Stopped;
        }
        if limit.is_some_and(|l| s >= l) {
            return EndReason::Completed;
        }
        if options.disconnect_after_samples.is_some_and(|d| s >= d) {
            return EndReason::Disconnected;
        }
        let available = match &options.streaming {
            Streaming::Instant => u64::MAX,
            Streaming::RealTime(clock) => {
                let elapsed = clock.now().saturating_sub(start);
                (elapsed.as_nanos() * generator.rate as u128 / 1_000_000_000) as u64
            }
        };
        let mut end = available.min(s.saturating_add(chunk));
        if let Some(l) = limit {
            end = end.min(l);
        }
        if let Some(d) = options.disconnect_after_samples {
            end = end.min(d);
        }
        if end <= s {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        if !generator.channels.is_empty() {
            generator.fill(s, end, &mut buf);
            sink.on_logic(s, generator.unit_size, &buf);
        }
        for &ch in &generator.analog {
            generator.fill_analog(ch.index, s, end, &mut volts);
            sink.on_analog(ch, s, &volts);
        }
        s = end;
    }
}

/// A device's own trigger, armed.
struct Wait {
    matcher: ::oxrok::core::SoftTrigger,
    /// The share of `limit` to deliver from before the trigger.
    pre_percent: u8,
    limit: u64,
}

/// As [`stream`], on a device that waits for its trigger: nothing until
/// it fires, then the data from before it, its position, and the rest
/// of the limit. Sample 0 is the first one delivered.
fn stream_triggered(
    generator: &Generator,
    options: &MockOptions,
    start: Duration,
    mut wait: Wait,
    stop: &AtomicBool,
    sink: &mut dyn SampleSink,
) -> EndReason {
    let chunk = options.chunk_samples.max(1) as u64;
    let mut buf = Vec::new();
    let mut volts = Vec::new();
    // The pattern's samples there are by now.
    let available = || match &options.streaming {
        Streaming::Instant => u64::MAX,
        Streaming::RealTime(clock) => {
            let elapsed = clock.now().saturating_sub(start);
            (elapsed.as_nanos() * generator.rate as u128 / 1_000_000_000) as u64
        }
    };
    let mut s = 0u64;
    let fired = loop {
        if stop.load(Ordering::SeqCst) {
            return EndReason::Stopped;
        }
        if options.disconnect_after_samples.is_some_and(|d| s >= d) {
            return EndReason::Disconnected;
        }
        let end = available().min(s.saturating_add(chunk));
        if end <= s || generator.channels.is_empty() {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        generator.fill(s, end, &mut buf);
        if let Some(i) = wait.matcher.feed(&buf) {
            break s + i as u64;
        }
        s = end;
    };
    let pre = ((u128::from(wait.limit) * u128::from(wait.pre_percent) / 100) as u64).min(fired);
    let first = fired - pre;
    let last = first.saturating_add(wait.limit);
    let mut s = first;
    let mut reported = false;
    loop {
        if !reported && s >= fired {
            sink.on_hw_trigger(fired - first);
            reported = true;
        }
        if stop.load(Ordering::SeqCst) {
            return EndReason::Stopped;
        }
        if s >= last {
            return EndReason::Completed;
        }
        // The data before the trigger is in the device already.
        let stop_at = if reported { last } else { fired };
        let end = available().max(fired).min(s.saturating_add(chunk)).min(stop_at);
        if end <= s {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        generator.fill(s, end, &mut buf);
        sink.on_logic(s - first, generator.unit_size, &buf);
        for &ch in &generator.analog {
            generator.fill_analog(ch.index, s, end, &mut volts);
            sink.on_analog(ch, s - first, &volts);
        }
        s = end;
    }
}

/// Packs the mock signals into logic units.
#[derive(Clone, Debug)]
pub struct Generator {
    seed: u64,
    rate: u64,
    unit_size: usize,
    channels: Vec<u16>,
    /// Enabled analog channels.
    pub analog: Vec<ChannelId>,
}

fn mix(mut x: u64) -> u64 {
    // SplitMix64 finalizer.
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

impl Generator {
    pub fn new(seed: u64, rate: u64, unit_size: usize, enabled: &[ChannelId]) -> Self {
        let channels =
            enabled.iter().filter(|c| c.kind == crate::core::ChannelKind::Digital).map(|c| c.index).collect();
        Self { seed, rate, unit_size, channels, analog: Vec::new() }
    }

    /// Volts of analog channel `ch` for samples `[s0, s1)` into `out`.
    pub fn fill_analog(&self, ch: u16, s0: u64, s1: u64, out: &mut Vec<f32>) {
        out.clear();
        out.extend((s0..s1).map(|s| analog_value(ch, s, self.rate)));
    }

    /// Packed units for samples `[s0, s1)` into `out`.
    pub fn fill(&self, s0: u64, s1: u64, out: &mut Vec<u8>) {
        out.clear();
        out.resize((s1 - s0) as usize * self.unit_size, 0);
        for &ch in &self.channels {
            let (byte, bit) = (ch as usize / 8, ch % 8);
            if byte >= self.unit_size {
                continue;
            }
            for (i, s) in (s0..s1).enumerate() {
                if level(self.seed, ch, s, self.rate) {
                    out[i * self.unit_size + byte] |= 1 << bit;
                }
            }
        }
    }
}

/// Value of mock analog channel `ch` at `sample`, in volts.
pub fn analog_value(ch: u16, sample: u64, rate: u64) -> f32 {
    use std::f64::consts::TAU;
    // Phase within the period, computed from the exact sample position.
    let phase = |hz: u64| ((sample as u128 * hz as u128) % rate as u128) as f64 / rate as f64;
    let v = match ch {
        0 => 2.5 * (TAU * phase(1_000)).sin(),
        1 => {
            let p = phase(100);
            -2.0 + 4.0 * if p < 0.5 { 2.0 * p } else { 2.0 - 2.0 * p }
        }
        2 => 1.65 + 1.65 * (TAU * phase(250)).sin(),
        _ => 5.0 * phase(1),
    };
    v as f32
}

/// Byte `j` of SPI transfer `k` on MOSI (`line = 0`) or MISO (`line = 1`).
pub fn spi_byte(seed: u64, k: u64, j: usize, line: u64) -> u8 {
    mix(seed ^ mix(k.wrapping_mul(8).wrapping_add(j as u64)) ^ (line << 63)) as u8
}

/// Level of mock channel `ch` at `sample`.
pub fn level(seed: u64, ch: u16, sample: u64, rate: u64) -> bool {
    let (s, r) = (sample as u128, rate as u128);
    match ch {
        0 => (s * 2000 / r) % 2 == 0,
        1 => {
            // Time within the 10 ms period, in UART bit periods.
            let bit = ((s * 100) % r) * UART_BAUD as u128 / 100 / r;
            let (c, b) = ((bit / 10) as usize, bit % 10);
            match UART_TEXT.get(c) {
                None => true,
                Some(_) if b == 0 => false,
                Some(&byte) if b <= 8 => byte >> (b - 1) & 1 == 1,
                Some(_) => true,
            }
        }
        2..=5 => {
            let k = (s * 1000 / r) as u64;
            // Half-microseconds since the start of the 1 ms period.
            let h = ((s * 1000) % r) * 2000 / r;
            let bits = (SPI_BYTES_PER_TRANSFER * 8) as u128;
            let (cs_from, data_from) = (200u128, 202u128);
            let data_to = data_from + 2 * bits;
            let in_data = h >= data_from && h < data_to;
            let i = ((h.saturating_sub(data_from)) / 2) as usize;
            let bit_of = |line| spi_byte(seed, k, i / 8, line) >> (7 - i % 8) & 1 == 1;
            match ch {
                2 => !(h >= cs_from && h < data_to + 2),
                3 => in_data && (h - data_from) % 2 == 1,
                4 => in_data && bit_of(0),
                _ => in_data && bit_of(1),
            }
        }
        _ => {
            let ms = s * 1000 / r;
            ((ms >> (ch - 6).min(60)) & 1 == 1) ^ (mix(seed) >> ch & 1 == 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ManualClock;
    use crate::store::LogicIngest;
    use std::sync::Mutex;

    /// Collects a stream into a [`LogicIngest`].
    #[derive(Clone, Default)]
    struct Collect(Arc<Mutex<Collected>>);

    #[derive(Default)]
    struct Collected {
        header: Option<StreamHeader>,
        ingest: Option<LogicIngest>,
        end: Option<EndReason>,
    }

    impl SampleSink for Collect {
        fn on_header(&mut self, h: StreamHeader) {
            let bits = h.logic_channels.iter().map(|c| c.index).collect();
            let mut c = self.0.lock().unwrap();
            c.ingest = Some(LogicIngest::new(h.logic_unit_size, bits));
            c.header = Some(h);
        }
        fn on_logic(&mut self, first_sample: u64, _unit_size: usize, data: &[u8]) {
            self.0.lock().unwrap().ingest.as_mut().unwrap().push(first_sample, data).unwrap();
        }
        fn on_analog(&mut self, _: ChannelId, _: u64, _: &[f32]) {}
        fn on_hw_trigger(&mut self, _: u64) {}
        fn on_frame(&mut self, _: bool) {}
        fn on_end(&mut self, reason: EndReason) {
            self.0.lock().unwrap().end = Some(reason);
        }
    }

    fn device(backend: &MockBackend, which: usize) -> Box<dyn Device> {
        let id = backend.scan_devices()[which].id.clone();
        backend.open(&id).unwrap()
    }

    fn capture(options: MockOptions, enabled: Vec<ChannelId>, rate: u64, limit: AcquisitionLimit) -> Collect {
        let backend = MockBackend::new(options);
        let mut dev = device(&backend, 1);
        dev.apply(&DeviceConfig::new(enabled, rate)).unwrap();
        let sink = Collect::default();
        dev.start(AcquisitionPlan::new(limit), Box::new(sink.clone())).unwrap();
        while sink.0.lock().unwrap().end.is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
        sink
    }

    #[test]
    fn lists_two_devices() {
        let b = MockBackend::seeded(1);
        let names: Vec<_> = b.scan_devices().iter().map(DeviceInfo::display_name).collect();
        assert_eq!(names, ["Mock 8 digital", "Mock 16 digital + 4 analog"]);
        let d = &b.scan_devices()[1];
        assert_eq!(d.digital_channels().count(), 16);
        assert_eq!(d.analog_channels().count(), 4);
        assert!(matches!(b.open(&DeviceId("nope".into())), Err(BackendError::NotFound(_))));
    }

    #[test]
    fn apply_snaps_sample_rate() {
        let b = MockBackend::seeded(1);
        let mut d = device(&b, 0);
        let cfg = DeviceConfig::new(vec![ChannelId::digital(0)], 23_000_000);
        assert_eq!(d.apply(&cfg).unwrap().sample_rate, 24_000_000);
    }

    /// The generic options, a refused value and thresholds.
    #[test]
    fn options_and_thresholds() {
        let b = MockBackend::new(MockOptions { thresholds: true, ..MockOptions::default() });
        let mut d = device(&b, 1);
        let caps = d.capabilities().clone();
        assert_eq!(caps.options.len(), 2, "device scope and the Analog group");
        assert_eq!(caps.options[0].options.len(), 4);
        assert_eq!(caps.threshold, Some(ThresholdChoice::Preset("3.3".into())));
        let mut cfg = DeviceConfig::new(vec![ChannelId::digital(0)], 1_000_000);
        cfg.threshold = Some(ThresholdChoice::Custom(1.234));
        let dev = cfg.options.entry(OptionScope::Device).or_default();
        dev.insert("external_clock".into(), ConfValue::Bool(true));
        dev.insert("avg_samples".into(), ConfValue::UInt(5000));
        let applied = d.apply(&cfg).unwrap();
        assert_eq!(applied.threshold, Some(ThresholdChoice::Custom(1.23)));
        assert_eq!(applied.options[&OptionScope::Device]["external_clock"], ConfValue::Bool(true));
        assert_eq!(applied.options[&OptionScope::Device]["avg_samples"], ConfValue::UInt(1));
        assert_eq!(applied.rejected.len(), 1);
        assert_eq!(applied.rejected[0].id, "avg_samples");
        assert_eq!(d.read_options()[0].options[0].value, Some(ConfValue::Bool(true)));
    }

    #[test]
    fn clock_is_1khz() {
        let c = capture(MockOptions::default(), vec![ChannelId::digital(0)], 1_000_000, AcquisitionLimit::Millis(10));
        let c = c.0.lock().unwrap();
        assert_eq!(c.end, Some(EndReason::Completed));
        let ch = &c.ingest.as_ref().unwrap().channels()[0];
        assert_eq!(ch.data_range(), Some((0, 10_000)));
        // 2 transitions per ms; the first edge is the falling edge at 0.5 ms.
        assert_eq!(ch.transition_count(), 19);
        assert_eq!(ch.next_transition(0), Some(500));
        assert_eq!(ch.level_at(0), Some(true));
    }

    #[test]
    fn uart_sends_hello_world() {
        let rate = 1_152_000; // 10 samples per bit
        let opts = MockOptions { samplerates: vec![rate], ..MockOptions::default() };
        let c = capture(opts, vec![ChannelId::digital(1)], rate, AcquisitionLimit::Samples(rate / 100));
        let c = c.0.lock().unwrap();
        let ch = &c.ingest.as_ref().unwrap().channels()[0];
        // Sample in the middle of each bit and decode 8N1.
        let bit = |i: u64| ch.level_at(i * 10 + 5).unwrap();
        let text: Vec<u8> = (0..UART_TEXT.len() as u64)
            .map(|j| {
                assert!(!bit(j * 10), "start bit");
                assert!(bit(j * 10 + 9), "stop bit");
                (0..8).fold(0u8, |b, k| b | (bit(j * 10 + 1 + k) as u8) << k)
            })
            .collect();
        assert_eq!(text, UART_TEXT);
        assert!(bit(UART_TEXT.len() as u64 * 10 + 5), "idle high");
    }

    #[test]
    fn spi_mode0_transfer() {
        let rate = 10_000_000; // 5 samples per SCK half-period
        let enabled = (2..=5).map(ChannelId::digital).collect();
        let c = capture(MockOptions { seed: 42, ..MockOptions::default() }, enabled, rate, AcquisitionLimit::Millis(1));
        let c = c.0.lock().unwrap();
        let chs = c.ingest.as_ref().unwrap().channels();
        let (cs, sck, mosi, miso) = (&chs[0], &chs[1], &chs[2], &chs[3]);
        // Sample MOSI/MISO on each SCK rising edge while CS is low.
        let mut edges = Vec::new();
        let mut x = 0;
        while let Some(t) = sck.next_transition(x) {
            if sck.level_at(t) == Some(true) {
                assert_eq!(cs.level_at(t), Some(false));
                edges.push(t);
            }
            x = t;
        }
        assert_eq!(edges.len(), SPI_BYTES_PER_TRANSFER * 8);
        let bytes = |ch: &crate::store::DigitalChannel| -> Vec<u8> {
            edges.chunks(8).map(|e| e.iter().fold(0u8, |b, &t| b << 1 | ch.level_at(t).unwrap() as u8)).collect()
        };
        let expect = |line| (0..SPI_BYTES_PER_TRANSFER).map(|j| spi_byte(42, 0, j, line)).collect::<Vec<_>>();
        assert_eq!(bytes(mosi), expect(0));
        assert_eq!(bytes(miso), expect(1));
    }

    #[test]
    fn analog_signals() {
        let rate = 1_000_000;
        assert!((analog_value(0, 250, rate) - 2.5).abs() < 1e-6, "sine peak at 1/4 period");
        assert!((analog_value(1, 0, rate) + 2.0).abs() < 1e-6);
        assert!((analog_value(1, 5_000, rate) - 2.0).abs() < 1e-6, "triangle top at half period");
        assert!((analog_value(3, 500_000, rate) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn buffered_mock_fills_its_memory_without_rle() {
        let opts = MockOptions { buffered: Some(2048), ..MockOptions::default() };
        let caps = MockBackend::new(opts.clone())
            .open(&MockBackend::seeded(1).scan_devices()[0].id)
            .unwrap()
            .capabilities()
            .clone();
        assert!(!caps.supports_continuous && caps.rle);
        let c = capture(opts.clone(), vec![ChannelId::digital(0)], 1_000_000, AcquisitionLimit::Samples(100_000));
        assert_eq!(c.0.lock().unwrap().ingest.as_ref().unwrap().next_sample(), 2048);
        // With RLE the whole request fits.
        let backend = MockBackend::new(opts);
        let mut dev = device(&backend, 1);
        dev.apply(&DeviceConfig::new(vec![ChannelId::digital(0)], 1_000_000)).unwrap();
        let sink = Collect::default();
        let plan = AcquisitionPlan { rle: true, ..AcquisitionPlan::new(AcquisitionLimit::Samples(100_000)) };
        dev.start(plan, Box::new(sink.clone())).unwrap();
        while sink.0.lock().unwrap().end.is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(sink.0.lock().unwrap().ingest.as_ref().unwrap().next_sample(), 100_000);
    }

    #[test]
    fn deterministic_per_seed() {
        let generator = |seed| {
            let mut buf = Vec::new();
            Generator::new(seed, 1_000_000, 2, &(0..16).map(ChannelId::digital).collect::<Vec<_>>())
                .fill(0, 50_000, &mut buf);
            buf
        };
        assert_eq!(generator(1), generator(1));
        assert_ne!(generator(1), generator(2));
    }

    #[test]
    fn injected_errors() {
        let b = MockBackend::new(MockOptions { fail_start: true, ..MockOptions::default() });
        let mut d = device(&b, 0);
        let plan = AcquisitionPlan::new(AcquisitionLimit::Continuous);
        assert!(d.start(plan.clone(), Box::new(Collect::default())).is_err());

        let opts = MockOptions { disconnect_after_samples: Some(1234), ..MockOptions::default() };
        let c = capture(opts, vec![ChannelId::digital(0)], 1_000_000, AcquisitionLimit::Continuous);
        let c = c.0.lock().unwrap();
        assert_eq!(c.end, Some(EndReason::Disconnected));
        assert_eq!(c.ingest.as_ref().unwrap().next_sample(), 1234);
    }

    #[test]
    fn real_time_streaming_follows_clock_and_stops() {
        let clock = ManualClock::new();
        let opts = MockOptions { streaming: Streaming::RealTime(Arc::new(clock.clone())), ..MockOptions::default() };
        let b = MockBackend::new(opts);
        let mut d = device(&b, 0);
        d.apply(&DeviceConfig::new(vec![ChannelId::digital(0)], 1_000_000)).unwrap();
        let sink = Collect::default();
        d.start(AcquisitionPlan::new(AcquisitionLimit::Continuous), Box::new(sink.clone())).unwrap();
        clock.advance(Duration::from_millis(20));
        let got = |s: &Collect| s.0.lock().unwrap().ingest.as_ref().map_or(0, |i| i.next_sample());
        let t0 = std::time::Instant::now();
        while got(&sink) < 20_000 && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(got(&sink), 20_000, "never more than the clock allows");
        d.stop().unwrap();
        assert_eq!(sink.0.lock().unwrap().end, Some(EndReason::Stopped));
    }
}
