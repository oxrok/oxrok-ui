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
        oxrok::core::Error::Firmware(s) => BackendError::Device(format!("firmware file not found: {s}")),
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
}

/// A device found by a scan, not open yet.
pub struct OxrokBackend {
    registry: Registry,
    /// Drivers to scan, from `with_drivers`; `None` for all.
    only: Option<Vec<String>>,
    cache: Arc<Mutex<BTreeMap<DeviceId, Cached>>>,
    /// Connections asked for, scanned again with their driver.
    conns: Mutex<Vec<(String, String)>>,
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

    /// Scans one driver, `conn` when given, and replaces its cache entries.
    /// Returns the devices this scan found.
    fn scan_driver(
        &self,
        cache: &mut BTreeMap<DeviceId, Cached>,
        driver: &dyn Driver,
        conn: Option<&str>,
    ) -> Vec<DeviceInfo> {
        let options = conn.map_or_else(Vec::new, |c| vec![(Key::Conn, Value::Str(c.to_owned()))]);
        log::begin(driver.id());
        let found = driver.scan(&options);
        log::end();
        let found = found.unwrap_or_default();
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for (n, dev) in found.into_iter().enumerate() {
            let info = describe(driver.id(), dev.as_ref(), n);
            seen.insert(info.id.clone());
            out.push(info.clone());
            let open = cache.get(&info.id).is_some_and(|c| c.slot.lock().is_none());
            if open {
                continue;
            }
            cache.insert(
                info.id.clone(),
                Cached { info, slot: Arc::new(Mutex::new(Some(dev))), defaults: Arc::new(Mutex::new(None)) },
            );
        }
        // Devices the driver no longer finds go, unless one is open.
        cache.retain(|id, c| c.info.driver != driver.id() || seen.contains(id) || c.slot.lock().is_none());
        out
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
        for driver in drivers {
            if cache.values().any(|c| c.info.driver == driver.id() && c.slot.lock().is_none()) {
                continue;
            }
            self.scan_driver(&mut cache, driver, None);
            let conns: Vec<String> =
                self.conns.lock().iter().filter(|c| c.0 == driver.id()).map(|c| c.1.clone()).collect();
            for conn in conns {
                self.scan_driver(&mut cache, driver, Some(&conn));
            }
        }
        cache.values().map(|c| c.info.clone()).collect()
    }

    fn scan_conn(&self, driver: &str, conn: &str) -> Result<Vec<DeviceInfo>, BackendError> {
        let Some(d) = self.registry.find(driver) else {
            return Err(BackendError::Unsupported(format!("no driver named {driver}")));
        };
        let mut cache = self.cache.lock();
        if cache.values().any(|c| c.info.driver == driver && c.slot.lock().is_none()) {
            return Err(BackendError::Busy);
        }
        let found = self.scan_driver(&mut cache, d, Some(conn));
        if !found.is_empty() {
            let key = (driver.to_owned(), conn.to_owned());
            let mut conns = self.conns.lock();
            if !conns.contains(&key) {
                conns.push(key);
            }
        }
        Ok(found)
    }

    fn library_version(&self) -> Option<String> {
        Some(library_info())
    }

    fn firmware_status(&self) -> FirmwareStatus {
        FirmwareStatus {
            missing: log::missing(),
            paths: oxrok::core::firmware::search_dirs().into_iter().map(|p| (p.clone(), p.is_dir())).collect(),
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
