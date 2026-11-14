//! [`Backend`] over oxrok: drivers, devices and files, with no C sigrok
//! library.
//!
//! oxrok's devices are Rust objects: scanning hands back a boxed device,
//! configuration is `Device::set`, and an acquisition is a packet stream
//! from [`start`](oxrok::core::Device::start) to
//! [`Packet::End`](oxrok::core::Packet::End). This file maps that onto
//! [`crate::backend`], which is the shape the app was written against.

mod caps;
mod decode;
mod files;
mod log;

use crate::backend::*;
use crate::core::{ChannelId, ChannelKind, WallClock};
use oxrok::core::{
    Device as OxDevice, Driver, Key, Packet, PacketSink, Registry, Trigger, TriggerMatch as OxMatch, Value,
};
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use decode::OxrokDecoders;

/// The library line the app shows and prints at start.
pub fn library_info() -> String {
    format!("oxrok {}", oxrok::core::VERSION)
}

fn device_error(e: oxrok::core::Error) -> BackendError {
    match e {
        oxrok::core::Error::Busy => BackendError::Busy,
        oxrok::core::Error::Disconnected => BackendError::Device("device disconnected".into()),
        oxrok::core::Error::Firmware(s) => {
            // Some devices load theirs when a capture starts, outside
            // the scan's log.
            let msg = format!("firmware file not found: {s}");
            log::note(&msg);
            BackendError::Device(msg)
        }
        other => BackendError::Device(other.to_string()),
    }
}

/// Channels with their app ids and their positions in the
/// device's list.
fn channel_map(channels: &[oxrok::core::Channel]) -> Vec<(ChannelId, usize)> {
    let (mut d, mut a) = (0u16, 0u16);
    channels
        .iter()
        .enumerate()
        .map(|(pos, c)| {
            let id = match c.kind {
                oxrok::core::ChannelKind::Logic => {
                    d += 1;
                    ChannelId::digital(d - 1)
                }
                oxrok::core::ChannelKind::Analog => {
                    a += 1;
                    ChannelId::analog(a - 1)
                }
            };
            (id, pos)
        })
        .collect()
}

/// Step 4, device key.
fn describe(driver: &str, dev: &dyn OxDevice, n: usize) -> DeviceInfo {
    let ox = dev.info();
    let key = [ox.serial.as_str(), ox.conn.as_deref().unwrap_or("")]
        .into_iter()
        .find(|s| !s.is_empty())
        .map_or(n.to_string(), str::to_owned);
    let channels = dev
        .channels()
        .iter()
        .zip(channel_map(dev.channels()))
        .map(|(c, (id, _))| ChannelInfo { id, device_name: c.name.clone() })
        .collect();
    DeviceInfo {
        id: DeviceId::new(driver, &key),
        driver: driver.to_owned(),
        vendor: ox.vendor.clone(),
        model: ox.model.clone(),
        version: ox.version.clone(),
        conn: ox.conn.clone().filter(|c| !c.is_empty()),
        serial_or_conn: if ox.serial.is_empty() { ox.conn.clone().unwrap_or_default() } else { ox.serial.clone() },
        is_demo: driver == "demo",
        channels,
    }
}

/// The values a device first reported, for *Reset to defaults*.
type Defaults = Arc<Mutex<Option<BTreeMap<(OptionScope, String), ConfValue>>>>;

/// A scanned device and the slot it goes into while open.
struct Cached {
    info: DeviceInfo,
    slot: Arc<Mutex<Option<Box<dyn OxDevice>>>>,
    defaults: Defaults,
    /// The connection it was found at, when it was asked for.
    conn: Option<String>,
}

/// USB serial devices a driver finds only at their port: the ids, the
/// driver, what the ids say it is, and the firmware the driver talks to.
/// The ids of the Pico are those of every Pico SDK program with a USB
/// serial port, so the port is a candidate, to be probed when asked.
const SERIAL_IDS: [(u16, u16, &str, &str, &str); 1] =
    [(0x2E8A, 0x000A, "raspberrypi-pico", "Raspberry Pi Pico", "sigrok-pico")];

/// The ports among `ports` that may hold a known device, without those
/// in `connected`.
fn serial_candidates(ports: &[oxrok::serial::UsbPort], connected: &[String]) -> Vec<SerialCandidate> {
    ports
        .iter()
        .filter(|p| !connected.contains(&p.name))
        .filter_map(|p| {
            let (_, _, driver, label, firmware) = SERIAL_IDS.iter().find(|id| (id.0, id.1) == (p.vid, p.pid))?;
            Some(SerialCandidate {
                driver: (*driver).to_owned(),
                port: p.name.clone(),
                usb_serial: usb_identity(ports, &p.name)?.2,
                label: (*label).to_owned(),
                firmware: (*firmware).to_owned(),
            })
        })
        .collect()
}

/// What is plugged in at a USB serial port: the USB ids, and the serial
/// number where it has one. An empty serial number is none.
type UsbIdentity = (u16, u16, Option<String>);

fn usb_identity(ports: &[oxrok::serial::UsbPort], port: &str) -> Option<UsbIdentity> {
    let p = ports.iter().find(|p| p.name == port)?;
    Some((p.vid, p.pid, p.serial.clone().filter(|s| !s.is_empty())))
}

/// A connection asked for, scanned again with its driver.
#[derive(Clone, PartialEq, Eq)]
struct Conn {
    driver: String,
    conn: String,
    /// What was at the port when the device was found, for a USB serial
    /// port. The connection lasts as long as that is there: whatever is
    /// plugged in next is not asked anything.
    usb: Option<UsbIdentity>,
}

impl Conn {
    /// Whether what was found at a USB serial port is still there. Other
    /// connections (a network address, a USB address) always are.
    fn still_there(&self, usb_ports: &[oxrok::serial::UsbPort]) -> bool {
        self.usb.is_none() || usb_identity(usb_ports, &self.conn) == self.usb
    }
}

/// A device found at `conn` that names neither itself nor its
/// connection is named by its port: the USB serial number where the
/// port has one, so the board is the same device on any port, else the
/// port itself. Two such devices are then two devices.
fn name_by_conn(info: &mut DeviceInfo, own_serial: &str, conn: &str, usb_serial: Option<&str>) {
    if info.conn.is_none() {
        info.conn = Some(conn.to_owned());
    }
    if own_serial.is_empty() || own_serial == "N/A" {
        let key = usb_serial.unwrap_or(conn);
        info.id = DeviceId::new(&info.driver, key);
        info.serial_or_conn = key.to_owned();
    }
}

/// A device found by a scan, not open yet.
pub struct OxrokBackend {
    registry: Registry,
    /// Drivers to scan, from `with_drivers`; `None` for all.
    only: Option<Vec<String>>,
    cache: Arc<Mutex<BTreeMap<DeviceId, Cached>>>,
    /// Connections asked for, scanned again with their driver.
    conns: Mutex<Vec<Conn>>,
}

impl OxrokBackend {
    /// Registers every driver oxrok has; no hardware is touched until a
    /// scan.
    pub fn new() -> Self {
        Self::with_drivers(None)
    }

    /// Like [`new`](Self::new), but scans only the named drivers.
    pub fn with_drivers(only: Option<Vec<String>>) -> Self {
        log::install();
        let mut registry = Registry::new();
        oxrok::register_all(&mut registry);
        Self { registry, only, cache: Arc::default(), conns: Mutex::new(Vec::new()) }
    }

    /// Every driver this build knows, as `(name, long name)`.
    pub fn drivers(&self) -> Vec<(String, String)> {
        self.registry.drivers().map(|d| (d.id().to_owned(), d.long_name().to_owned())).collect()
    }

    fn wanted(&self, id: &str) -> bool {
        self.only.as_ref().is_none_or(|d| d.contains(&id.to_owned()))
    }

    /// Ends the connections whose board went, or whose port another board
    /// has: the device goes and the port is not asked again. What is
    /// plugged in next is a candidate, asked about first. An open device
    /// stays until it is closed, and its connection with it.
    fn end_gone(&self, cache: &mut BTreeMap<DeviceId, Cached>, usb_ports: &[oxrok::serial::UsbPort]) {
        self.conns.lock().retain(|asked| {
            if asked.still_there(usb_ports) {
                return true;
            }
            let at = |c: &Cached| c.info.driver == asked.driver && c.conn.as_deref() == Some(&asked.conn);
            cache.retain(|_, c| !at(c) || c.slot.lock().is_none());
            cache.values().any(at)
        });
    }

    /// Scans one driver, `conn` when given, and replaces its cache entries
    /// of that connection. Returns the devices this scan found; the
    /// driver's error when it was asked for a connection and found none.
    fn scan_driver(
        &self,
        cache: &mut BTreeMap<DeviceId, Cached>,
        driver: &dyn Driver,
        conn: Option<&str>,
    ) -> Result<Vec<DeviceInfo>, oxrok::core::Error> {
        let options = conn.map_or_else(Vec::new, |c| vec![(Key::Conn, Value::Str(c.to_owned()))]);
        log::begin(driver.id());
        let found = driver.scan(&options);
        log::end();
        let (found, error) = match found {
            Ok(found) => (found, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let usb_serial = conn.and_then(|c| usb_identity(&oxrok::serial::list_usb_ports(), c)?.2);
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for (n, dev) in found.into_iter().enumerate() {
            let mut info = describe(driver.id(), dev.as_ref(), n);
            if let Some(c) = conn {
                name_by_conn(&mut info, &dev.info().serial, c, usb_serial.as_deref());
            }
            seen.insert(info.id.clone());
            out.push(info.clone());
            let open = cache.get(&info.id).is_some_and(|c| c.slot.lock().is_none());
            if open {
                continue;
            }
            cache.insert(
                info.id.clone(),
                Cached {
                    info,
                    slot: Arc::new(Mutex::new(Some(dev))),
                    defaults: Arc::new(Mutex::new(None)),
                    conn: conn.map(str::to_owned),
                },
            );
        }
        // Devices the driver no longer finds go, unless one is open. Those
        // found at a connection go with a scan of that connection only.
        cache.retain(|id, c| {
            c.info.driver != driver.id() || c.conn.as_deref() != conn || seen.contains(id) || c.slot.lock().is_none()
        });
        match error {
            Some(e) if conn.is_some() && out.is_empty() => Err(e),
            _ => Ok(out),
        }
    }
}

impl Default for OxrokBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for OxrokBackend {
    fn scan_devices(&self) -> Vec<DeviceInfo> {
        let mut cache = self.cache.lock();
        let drivers: Vec<&dyn Driver> = self.registry.drivers().filter(|d| self.wanted(d.id())).collect();
        let (ports, usb_ports) = if self.conns.lock().is_empty() {
            (Vec::new(), Vec::new())
        } else {
            (oxrok::serial::list_ports(), oxrok::serial::list_usb_ports())
        };
        // Before anything is skipped for an open device: only the list of
        // ports is read.
        self.end_gone(&mut cache, &usb_ports);
        for driver in drivers {
            if cache.values().any(|c| c.info.driver == driver.id() && c.slot.lock().is_none()) {
                continue;
            }
            let _ = self.scan_driver(&mut cache, driver, None);
            let conns: Vec<Conn> = self.conns.lock().iter().filter(|c| c.driver == driver.id()).cloned().collect();
            for asked in conns {
                let conn = asked.conn.clone();
                // A device found at a serial port stays while the port
                // is there: asking again would reset it (the Pico's
                // identify starts with a reset), every few seconds.
                let held = cache.values().any(|c| c.info.driver == driver.id() && c.conn.as_deref() == Some(&conn));
                if held && ports.contains(&conn) {
                    continue;
                }
                let _ = self.scan_driver(&mut cache, driver, Some(&conn));
            }
        }
        cache.values().map(|c| c.info.clone()).collect()
    }

    fn scan_conn(&self, driver: &str, conn: &str) -> Result<Vec<DeviceInfo>, BackendError> {
        let Some(d) = self.registry.find(driver) else {
            return Err(BackendError::Unsupported(format!("no driver named {driver}")));
        };
        let mut cache = self.cache.lock();
        // Found there before: the device has the port, in use or not.
        let held: Vec<DeviceInfo> = cache
            .values()
            .filter(|c| c.info.driver == driver && c.conn.as_deref() == Some(conn))
            .map(|c| c.info.clone())
            .collect();
        if !held.is_empty() {
            return Ok(held);
        }
        if cache.values().any(|c| c.info.driver == driver && c.slot.lock().is_none()) {
            return Err(BackendError::Busy);
        }
        let found = self.scan_driver(&mut cache, d, Some(conn)).map_err(device_error)?;
        if !found.is_empty() {
            let usb = usb_identity(&oxrok::serial::list_usb_ports(), conn);
            let key = Conn { driver: driver.to_owned(), conn: conn.to_owned(), usb };
            let mut conns = self.conns.lock();
            conns.retain(|c| (c.driver.as_str(), c.conn.as_str()) != (driver, conn));
            conns.push(key);
        }
        Ok(found)
    }

    fn serial_candidates(&self) -> Vec<SerialCandidate> {
        let connected: Vec<String> = self.cache.lock().values().filter_map(|c| c.conn.clone()).collect();
        let mut found = serial_candidates(&oxrok::serial::list_usb_ports(), &connected);
        found.retain(|c| self.wanted(&c.driver));
        found
    }

    fn library_version(&self) -> Option<String> {
        Some(library_info())
    }

    fn firmware_status(&self) -> FirmwareStatus {
        FirmwareStatus {
            missing: log::missing(),
            paths: oxrok::core::firmware::search_dirs().into_iter().map(|p| (p.clone(), p.is_dir())).collect(),
            // The first per-user folder of `search_dirs`.
            install_dir: crate::backend::fetch::default_install_dir(),
        }
    }

    fn open(&self, id: &DeviceId) -> Result<Box<dyn crate::backend::Device>, BackendError> {
        let (info, slot, defaults) = {
            let cache = self.cache.lock();
            let c = cache.get(id).ok_or_else(|| BackendError::NotFound(id.clone()))?;
            (c.info.clone(), c.slot.clone(), c.defaults.clone())
        };
        let dev = slot.lock().take().ok_or(BackendError::Busy)?;
        Ok(Box::new(OxrokDevice::new(info, dev, slot, defaults)?))
    }

    fn decoders(&self) -> &[DecoderInfo] {
        &[]
    }

    fn input_formats(&self) -> Vec<InputFormat> {
        files::input_formats()
    }

    fn detect_format(&self, path: &Path) -> Option<String> {
        files::detect(path)
    }

    fn import_file(
        &self,
        path: &Path,
        format: ImportFormat,
        options: &BTreeMap<String, OptionValue>,
        sink: Box<dyn SampleSink>,
        progress: ProgressFn,
    ) -> Result<ImportedCapture, BackendError> {
        // Imports touch no device, so they run on the caller's thread.
        files::import(files::ImportJob { path: path.to_owned(), format, options: options.clone(), sink, progress })
    }

    fn can_save_sr(&self) -> bool {
        true
    }

    fn save_sr_file(&self, path: &Path, source: &dyn ExportSource, progress: ProgressFn) -> Result<(), BackendError> {
        files::save_sr(path, source, progress)
    }
}

/// One running acquisition.
struct Acq {
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

/// An open device.
pub struct OxrokDevice {
    info: DeviceInfo,
    caps: DeviceCaps,
    dev: Option<Box<dyn OxDevice>>,
    /// Where the device goes when this handle drops, so `open` works again.
    slot: Arc<Mutex<Option<Box<dyn OxDevice>>>>,
    defaults: Defaults,
    /// App channel → position in the device's list.
    channels: Vec<(ChannelId, usize)>,
    acquisition: Option<Acq>,
}

impl OxrokDevice {
    fn new(
        info: DeviceInfo,
        mut dev: Box<dyn OxDevice>,
        slot: Arc<Mutex<Option<Box<dyn OxDevice>>>>,
        defaults: Defaults,
    ) -> Result<Self, BackendError> {
        log::watch(&info.driver);
        let opened = dev.open();
        log::end();
        if let Err(e) = opened {
            *slot.lock() = Some(dev);
            return Err(device_error(e));
        }
        let channels = channel_map(dev.channels());
        let mut caps = caps::capabilities(dev.as_ref(), &channels);
        let (thresholds, threshold) = caps::thresholds(dev.as_ref());
        caps.thresholds = thresholds;
        caps.threshold = threshold;
        caps.options = caps::option_groups(dev.as_ref(), &mut defaults.lock());
        Ok(Self { info, caps, dev: Some(dev), slot, defaults, channels, acquisition: None })
    }

    fn dev(&mut self) -> Result<&mut Box<dyn OxDevice>, BackendError> {
        self.dev.as_mut().ok_or(BackendError::Busy)
    }

    fn reap(&mut self) {
        if self.acquisition.as_ref().is_some_and(|a| a.handle.is_finished())
            && let Some(a) = self.acquisition.take()
        {
            let _ = a.handle.join();
        }
    }
}

impl crate::backend::Device for OxrokDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn capabilities(&self) -> &DeviceCaps {
        &self.caps
    }

    fn apply(&mut self, cfg: &DeviceConfig) -> Result<AppliedConfig, BackendError> {
        self.reap();
        if self.acquisition.is_some() {
            return Err(BackendError::Busy);
        }
        let channels = self.channels.clone();
        let dev = self.dev()?;
        for (id, pos) in &channels {
            dev.enable_channel(*pos, cfg.enabled.contains(id)).map_err(device_error)?;
        }
        let enabled: Vec<ChannelId> =
            channels.iter().filter(|(id, _)| cfg.enabled.contains(id)).map(|(id, _)| *id).collect();
        dev.set(&Key::Samplerate, &Value::U64(cfg.sample_rate), None).map_err(device_error)?;
        let sample_rate = dev.get(&Key::Samplerate, None).ok().and_then(|v| v.as_u64()).unwrap_or(cfg.sample_rate);
        if let Some(t) = &cfg.threshold {
            caps::apply_threshold(dev.as_mut(), t)?;
        }
        let threshold = caps::thresholds(dev.as_ref()).1;
        let (options, rejected) = caps::apply_options(dev.as_mut(), &cfg.options);
        self.caps.threshold = threshold.clone();
        Ok(AppliedConfig { enabled, sample_rate, threshold, options, rejected })
    }

    fn start(&mut self, plan: AcquisitionPlan, sink: Box<dyn SampleSink>) -> Result<(), BackendError> {
        self.reap();
        if self.acquisition.is_some() {
            return Err(BackendError::Busy);
        }
        let channels = self.channels.clone();
        let dev = self.dev()?;
        let rate = dev.get(&Key::Samplerate, None).ok().and_then(|v| v.as_u64()).unwrap_or(0);
        set_limit(dev.as_mut(), &plan, rate)?;
        set_compression(dev.as_mut(), plan.rle)?;
        set_trigger(dev.as_mut(), plan.trigger.as_ref(), &channels)?;

        let header = header_of(dev.as_ref(), &channels, rate);
        let analog: Vec<(usize, ChannelId)> = dev
            .channels()
            .iter()
            .zip(&channels)
            .filter(|(c, (_, _))| c.kind == oxrok::core::ChannelKind::Analog && c.enabled)
            .map(|(c, (id, _))| (c.index, *id))
            .collect();

        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        dev.start(PacketSink::new(tx, 0)).map_err(device_error)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = std::thread::Builder::new()
            .name("oxrok-acquisition".into())
            .spawn(move || feed(rx, sink, header, analog, stopped))
            .map_err(|e| BackendError::Device(e.to_string()))?;
        self.acquisition = Some(Acq { stop, handle });
        Ok(())
    }

    /// A 1-sample capture (no device reads its pins directly).
    fn read_current_state(&mut self, cfg: &DeviceConfig) -> Result<Vec<(ChannelId, bool)>, BackendError> {
        self.reap();
        if self.acquisition.is_some() {
            return Err(BackendError::Busy);
        }
        crate::backend::read_state_by_capture(self, cfg, std::time::Duration::from_secs(5))
    }

    fn read_options(&mut self) -> Vec<OptionGroup> {
        let defaults = self.defaults.clone();
        if let Ok(dev) = self.dev() {
            self.caps.options = caps::option_groups(dev.as_ref(), &mut defaults.lock());
        }
        self.caps.options.clone()
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        if let Some(a) = self.acquisition.take() {
            a.stop.store(true, Ordering::SeqCst);
            if let Some(dev) = self.dev.as_mut() {
                // A capture that ended by itself has no thread to stop; the
                // call is allowed then.
                let _ = dev.stop();
            }
            let _ = a.handle.join();
        }
        Ok(())
    }
}

impl Drop for OxrokDevice {
    fn drop(&mut self) {
        let _ = self.stop();
        if let Some(mut dev) = self.dev.take() {
            log::watch(&self.info.driver);
            let _ = dev.close();
            log::end();
            *self.slot.lock() = Some(dev);
        }
    }
}

/// A one-stage trigger, after setting the capture ratio.
fn set_trigger(
    dev: &mut dyn OxDevice,
    trigger: Option<&HwTrigger>,
    channels: &[(ChannelId, usize)],
) -> Result<(), BackendError> {
    let descs = dev.options(None).unwrap_or_default();
    let settable = |id: &str| descs.iter().any(|o| o.key.id() == id && o.access.set);
    let ratio = trigger.map_or(0, |t| t.pre_trigger_percent.min(100));
    if settable("captureratio") {
        dev.set(&Key::CaptureRatio, &Value::U64(ratio.into()), None).map_err(device_error)?;
    }
    let Some(t) = trigger else {
        return dev.set_trigger(None).map_err(device_error);
    };
    let mut trig = Trigger::new();
    for (id, m) in &t.matches {
        let (_, pos) = channels
            .iter()
            .find(|(c, _)| c == id)
            .ok_or_else(|| BackendError::Device(format!("no channel {id:?} for the trigger")))?;
        let m = match m {
            TriggerMatch::Zero => OxMatch::Zero,
            TriggerMatch::One => OxMatch::One,
            TriggerMatch::Rising => OxMatch::Rising,
            TriggerMatch::Falling => OxMatch::Falling,
            TriggerMatch::Edge => OxMatch::Edge,
        };
        trig.add(0, *pos, m);
    }
    dev.set_trigger(Some(trig)).map_err(device_error)
}

/// The limit through whichever key the device has.
fn set_limit(dev: &mut dyn OxDevice, plan: &AcquisitionPlan, rate: u64) -> Result<(), BackendError> {
    let descs = dev.options(None).unwrap_or_default();
    let settable = |id: &str| descs.iter().any(|o| o.key.id() == id && o.access.set);
    let set = |dev: &mut dyn OxDevice, key: Key, v: Value| dev.set(&key, &v, None).map_err(device_error);
    match plan.limit {
        AcquisitionLimit::Samples(n) => set(dev, Key::LimitSamples, Value::U64(n)),
        AcquisitionLimit::Millis(ms) if settable("limit_time") => set(dev, Key::LimitMsec, Value::U64(ms)),
        AcquisitionLimit::Millis(ms) => {
            set(dev, Key::LimitSamples, Value::U64((ms as u128 * rate as u128).div_ceil(1000) as u64))
        }
        AcquisitionLimit::Continuous if settable("continuous") => set(dev, Key::Continuous, Value::Bool(true)),
        AcquisitionLimit::Continuous => set(dev, Key::LimitSamples, Value::U64(CONTINUOUS_LIMIT_SAMPLES)),
    }
}

/// Sample limit used for Continuous mode on drivers without `continuous`.
const CONTINUOUS_LIMIT_SAMPLES: u64 = 1 << 62;

/// Device-side compression, set either way: the setting persists.
fn set_compression(dev: &mut dyn OxDevice, rle: bool) -> Result<(), BackendError> {
    let descs = dev.options(None).unwrap_or_default();
    let Some(o) = descs.iter().find(|o| o.key == Key::Rle) else {
        return if rle {
            Err(BackendError::Unsupported("run-length compression on this device".into()))
        } else {
            Ok(())
        };
    };
    if o.access.set {
        dev.set(&Key::Rle, &Value::Bool(rle), None).map_err(device_error)?;
    }
    Ok(())
}

/// The header of the next stream, from the channels the device enabled.
fn header_of(dev: &dyn OxDevice, channels: &[(ChannelId, usize)], rate: u64) -> StreamHeader {
    let mut logic = Vec::new();
    let mut analog = Vec::new();
    for (id, pos) in channels {
        if !dev.channels().get(*pos).is_some_and(|c| c.enabled) {
            continue;
        }
        match id.kind {
            ChannelKind::Digital => logic.push(*id),
            ChannelKind::Analog => analog.push(*id),
            ChannelKind::Bus => {}
        }
    }
    StreamHeader {
        start: WallClock::now(),
        sample_rate: rate,
        logic_unit_size: logic.len().div_ceil(8).max(1),
        logic_channels: logic,
        analog_channels: analog,
    }
}

/// The acquisition thread: oxrok packets → [`SampleSink`].
fn feed(
    rx: std::sync::mpsc::Receiver<(usize, Packet)>,
    mut sink: Box<dyn SampleSink>,
    header: StreamHeader,
    analog: Vec<(usize, ChannelId)>,
    stop: Arc<AtomicBool>,
) {
    let mut header = Some(header);
    let send_header = |sink: &mut Box<dyn SampleSink>, header: &mut Option<StreamHeader>| {
        if let Some(h) = header.take() {
            sink.on_header(h);
        }
    };
    let mut pos = 0u64;
    let mut analog_pos = vec![0u64; analog.len()];
    let mut error: Option<String> = None;
    let mut ended = false;
    for (_, packet) in rx {
        match packet {
            Packet::Header { .. } => send_header(&mut sink, &mut header),
            Packet::Meta(_) => {}
            Packet::Logic(d) => {
                send_header(&mut sink, &mut header);
                sink.on_logic(pos, d.unit_size, &d.data);
                pos += d.len() as u64;
            }
            Packet::Analog(a) => {
                send_header(&mut sink, &mut header);
                let n = a.len();
                let columns = a.channels.len().max(1);
                for (slot, (index, id)) in analog.iter().enumerate() {
                    let Some(k) = a.channels.iter().position(|c| c == index) else { continue };
                    let mut column = Vec::with_capacity(n);
                    column.extend((0..n).map(|s| a.samples[s * columns + k]));
                    sink.on_analog(*id, analog_pos[slot], &column);
                    analog_pos[slot] += n as u64;
                }
            }
            Packet::Trigger => sink.on_hw_trigger(pos),
            Packet::FrameBegin => sink.on_frame(true),
            Packet::FrameEnd => sink.on_frame(false),
            Packet::Failed(why) => error = Some(why),
            Packet::End => {
                ended = true;
                break;
            }
        }
    }
    send_header(&mut sink, &mut header);
    let reason = match error {
        Some(e) => EndReason::Error(e),
        None if ended && stop.load(Ordering::SeqCst) => EndReason::Stopped,
        None if ended => EndReason::Completed,
        None if stop.load(Ordering::SeqCst) => EndReason::Stopped,
        None => EndReason::Error("the device stopped without an end packet".into()),
    };
    sink.on_end(reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A firmware error outside a scan (a capture start) is a missing
    /// file too, in the words of the log.
    #[test]
    fn firmware_errors_are_recorded() {
        let e = device_error(oxrok::core::Error::Firmware("oxrok-test-start.rbf (install it)".into()));
        assert!(
            matches!(&e, BackendError::Device(m) if m == "firmware file not found: oxrok-test-start.rbf (install it)")
        );
        assert!(log::missing().iter().any(|m| m.file == "oxrok-test-start.rbf" && m.driver.is_empty()));
    }
    /// A port with the Pico's USB ids is a candidate, by the ids alone;
    /// other ports are none, nor one a device was found at.
    #[test]
    fn serial_candidates_go_by_the_usb_ids() {
        let port = |name: &str, vid, pid, serial: Option<&str>| oxrok::serial::UsbPort {
            name: name.into(),
            vid,
            pid,
            serial: serial.map(str::to_owned),
            manufacturer: None,
            product: None,
        };
        let ports = [
            port("/dev/cu.usbmodem11301", 0x2E8A, 0x000A, Some("E6625887D3405136")),
            // The bench's ESP32: a CP2102 bridge, as any other board has.
            port("/dev/cu.usbserial-0001", 0x10C4, 0xEA60, Some("0001")),
            port("/dev/cu.usbmodem2", 0x2E8A, 0x000A, Some("")),
            // The Pico's vendor with another product.
            port("/dev/cu.usbmodem3", 0x2E8A, 0x0003, None),
        ];
        let found = serial_candidates(&ports, &[]);
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].driver.as_str(), found[0].port.as_str(), found[0].usb_serial.as_deref()),
            ("raspberrypi-pico", "/dev/cu.usbmodem11301", Some("E6625887D3405136"))
        );
        assert_eq!((found[1].port.as_str(), found[1].usb_serial.as_deref()), ("/dev/cu.usbmodem2", None));
        let connected = ["/dev/cu.usbmodem11301".to_owned()];
        assert_eq!(serial_candidates(&ports, &connected).len(), 1);
    }

    /// A connection at a USB serial port lasts while the board found there
    /// is there. Once it went, or another has its port, the port is not
    /// asked again: the next board is a candidate.
    #[test]
    fn a_serial_connection_ends_with_its_board() {
        let port = |name: &str, serial: Option<&str>| oxrok::serial::UsbPort {
            name: name.into(),
            vid: 0x2E8A,
            pid: 0x000A,
            serial: serial.map(str::to_owned),
            manufacturer: None,
            product: None,
        };
        let at = |ports: &[oxrok::serial::UsbPort], conn: &str| Conn {
            driver: "raspberrypi-pico".into(),
            conn: conn.into(),
            usb: usb_identity(ports, conn),
        };
        let ports = [port("/dev/ttyACM0", Some("E6625887D3405136")), port("/dev/ttyACM1", Some(""))];
        let (a, b) = (at(&ports, "/dev/ttyACM0"), at(&ports, "/dev/ttyACM1"));
        assert_eq!(a.usb, Some((0x2E8A, 0x000A, Some("E6625887D3405136".into()))));
        // An empty serial number is none: the device is named by its port.
        assert_eq!(b.usb, Some((0x2E8A, 0x000A, None)));
        assert!(a.still_there(&ports) && b.still_there(&ports));
        // Unplugged.
        assert!(!a.still_there(&ports[1..]));
        // Another board on the same port name.
        assert!(!a.still_there(&[port("/dev/ttyACM0", Some("E66258881B2A7C2D"))]));
        // Back on its port.
        assert!(a.still_there(&[port("/dev/ttyACM0", Some("E6625887D3405136"))]));
        // A network address is no USB serial port: scanned as before.
        let tcp = at(&ports, "192.168.1.20/5555");
        assert!(tcp.usb.is_none() && tcp.still_there(&[]));
    }

    /// A board that went is noticed while another device of its driver is
    /// open: its port is a candidate again. The open device itself stays
    /// until it is closed.
    #[test]
    fn a_gone_board_ends_beside_an_open_device() {
        let port = |name: &str, serial: &str| oxrok::serial::UsbPort {
            name: name.into(),
            vid: 0x2E8A,
            pid: 0x000A,
            serial: Some(serial.into()),
            manufacturer: None,
            product: None,
        };
        let backend = OxrokBackend::with_drivers(Some(vec!["demo".into()]));
        // Any device stands for a closed one: the demo's.
        let closed = || backend.registry.find("demo").unwrap().scan(&[]).unwrap().pop();
        let add = |ports: &[oxrok::serial::UsbPort], conn: &str, slot: Option<Box<dyn OxDevice>>| {
            let usb = usb_identity(ports, conn);
            let id = DeviceId::new("raspberrypi-pico", usb.as_ref().unwrap().2.as_deref().unwrap());
            let info = DeviceInfo {
                id: id.clone(),
                driver: "raspberrypi-pico".into(),
                vendor: "Raspberry Pi".into(),
                model: "PICO".into(),
                version: String::new(),
                serial_or_conn: id.to_string(),
                conn: Some(conn.into()),
                is_demo: false,
                channels: Vec::new(),
            };
            let cached =
                Cached { info, slot: Arc::new(Mutex::new(slot)), defaults: Arc::default(), conn: Some(conn.into()) };
            backend.cache.lock().insert(id.clone(), cached);
            backend.conns.lock().push(Conn { driver: "raspberrypi-pico".into(), conn: conn.into(), usb });
            id
        };
        let held = || backend.cache.lock().values().filter_map(|c| c.conn.clone()).collect::<Vec<_>>();
        let ports = [port("/dev/ttyACM0", "AAAA"), port("/dev/ttyACM1", "BBBB")];
        // The board at ACM0 is open, the one at ACM1 is not.
        let open = add(&ports, "/dev/ttyACM0", None);
        add(&ports, "/dev/ttyACM1", closed());
        backend.end_gone(&mut backend.cache.lock(), &ports);
        assert_eq!(held(), ["/dev/ttyACM0", "/dev/ttyACM1"]);

        // Another board at ACM1: the old one goes, the new is a candidate.
        let ports = [port("/dev/ttyACM0", "AAAA"), port("/dev/ttyACM1", "CCCC")];
        backend.end_gone(&mut backend.cache.lock(), &ports);
        assert_eq!(held(), ["/dev/ttyACM0"]);
        assert_eq!(backend.conns.lock().len(), 1);
        let found = serial_candidates(&ports, &held());
        assert_eq!(found.iter().map(|c| c.port.as_str()).collect::<Vec<_>>(), ["/dev/ttyACM1"]);

        // The open board unplugged: it stays while it is open.
        backend.end_gone(&mut backend.cache.lock(), &ports[1..]);
        assert_eq!((held(), backend.conns.lock().len()), (vec!["/dev/ttyACM0".to_owned()], 1));
        // Closed: gone.
        *backend.cache.lock()[&open].slot.lock() = closed();
        backend.end_gone(&mut backend.cache.lock(), &ports[1..]);
        assert!(held().is_empty() && backend.conns.lock().is_empty());
    }

    /// A device that names neither itself nor its port is named by the
    /// port's USB serial number, else by the port: two are two devices.
    #[test]
    fn devices_at_a_port_are_told_apart() {
        let info = || DeviceInfo {
            id: DeviceId::new("raspberrypi-pico", "N/A"),
            driver: "raspberrypi-pico".into(),
            vendor: "Raspberry Pi".into(),
            model: "PICO".into(),
            version: String::new(),
            serial_or_conn: "N/A".into(),
            conn: None,
            is_demo: false,
            channels: Vec::new(),
        };
        let (mut a, mut b, mut own) = (info(), info(), info());
        name_by_conn(&mut a, "N/A", "/dev/cu.usbmodem1", Some("E6625887D3405136"));
        name_by_conn(&mut b, "", "/dev/cu.usbmodem2", None);
        assert_eq!(a.id, DeviceId::new("raspberrypi-pico", "E6625887D3405136"));
        assert_eq!(b.id, DeviceId::new("raspberrypi-pico", "/dev/cu.usbmodem2"));
        assert_eq!((a.conn.as_deref(), a.serial_or_conn.as_str()), (Some("/dev/cu.usbmodem1"), "E6625887D3405136"));
        // A device with a serial number of its own keeps it.
        name_by_conn(&mut own, "SN42", "/dev/cu.usbmodem3", Some("USB"));
        assert_eq!((own.id.clone(), own.conn.as_deref()), (info().id, Some("/dev/cu.usbmodem3")));
    }
}
