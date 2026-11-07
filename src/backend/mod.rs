//! Backend contract between the UI and acquisition/decoding, plus
//! the deterministic [`MockBackend`] (the `mock` feature).
//!
//! The UI depends only on these traits; `crate::oxrok` provides the
//! oxrok implementation.

#[cfg(feature = "mock")]
pub mod mock;
mod types;

#[cfg(feature = "mock")]
pub use mock::{MockBackend, MockOptions, Streaming};
pub mod firmware;
pub use types::*;

use crate::core::ChannelId;
use std::path::Path;

pub trait Backend: Send + Sync + 'static {
    /// May block; call off the UI thread.
    fn scan_devices(&self) -> Vec<DeviceInfo>;
    fn open(&self, id: &DeviceId) -> Result<Box<dyn Device>, BackendError>;
    /// Step 3: scans one driver at a connection
    /// (`conn=…`, e.g. `tcp-raw/192.168.1.20/5555`); the devices found
    /// join the list. May block; call off the UI thread.
    fn scan_conn(&self, _driver: &str, _conn: &str) -> Result<Vec<DeviceInfo>, BackendError> {
        Err(BackendError::Unsupported("connecting by address".into()))
    }

    /// The device library's version, e.g. "libsigrok 0.6.0".
    fn library_version(&self) -> Option<String> {
        None
    }

    /// Missing firmware from the last scan, and the search paths.
    fn firmware_status(&self) -> FirmwareStatus {
        FirmwareStatus::default()
    }

    /// Extraction needs no bundled script (the mock's copy).
    fn extracts_without_script(&self) -> bool {
        false
    }

    /// Runs a sigrok-util extraction script on `input`, writing
    /// into `out_dir`; returns the files written. Blocks.
    fn extract_firmware(
        &self,
        script: &Path,
        input: &Path,
        out_dir: &Path,
        progress: &mut dyn FnMut(String),
    ) -> Result<Vec<std::path::PathBuf>, BackendError> {
        firmware::run_script(script, input, out_dir, progress)
    }
    /// Loaded once at start.
    fn decoders(&self) -> &[DecoderInfo];
    /// The input formats this backend reads. None by default.
    fn input_formats(&self) -> Vec<InputFormat> {
        Vec::new()
    }

    /// Content detection for a file no extension identifies.
    fn detect_format(&self, _path: &Path) -> Option<String> {
        None
    }

    /// Reads `path` into `sink` (header, samples, end).
    /// Blocks; call off the UI thread.
    fn import_file(
        &self,
        _path: &Path,
        _format: ImportFormat,
        _options: &std::collections::BTreeMap<String, OptionValue>,
        _sink: Box<dyn SampleSink>,
        _progress: ProgressFn,
    ) -> Result<ImportedCapture, BackendError> {
        Err(BackendError::Unsupported("importing files needs libsigrok".into()))
    }

    /// Whether [`save_sr_file`](Self::save_sr_file) works.
    fn can_save_sr(&self) -> bool {
        false
    }

    /// Writes a sigrok session file. Blocks.
    fn save_sr_file(
        &self,
        _path: &Path,
        _source: &dyn ExportSource,
        _progress: ProgressFn,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported("sigrok session files need libsigrok".into()))
    }
}

pub trait Device: Send {
    fn info(&self) -> &DeviceInfo;
    fn capabilities(&self) -> &DeviceCaps;
    /// Returns what the device actually accepted.
    fn apply(&mut self, cfg: &DeviceConfig) -> Result<AppliedConfig, BackendError>;
    fn start(&mut self, plan: AcquisitionPlan, sink: Box<dyn SampleSink>) -> Result<(), BackendError>;
    fn stop(&mut self) -> Result<(), BackendError>;
    /// The options with fresh values, for the read-only rows.
    fn read_options(&mut self) -> Vec<OptionGroup> {
        self.capabilities().options.clone()
    }

    /// The current level of each enabled digital channel of
    /// `cfg`. Blocks until read; never while a capture runs.
    fn read_current_state(&mut self, _cfg: &DeviceConfig) -> Result<Vec<(ChannelId, bool)>, BackendError> {
        Err(BackendError::Unsupported("live pin state".into()))
    }
}

/// How long [`read_state_by_capture`] waits for the 1-sample capture to
/// end by itself after its data arrived. Drivers stop themselves once the
/// limit is reached, so stopping them too is a double stop: libsigrok's
/// polling drivers (demo among them) then warn about removing a source
/// twice and send a second END packet.
pub const STATE_READ_END_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

/// For devices without a direct read: a 1-sample capture with
/// `cfg`'s digital channels, taking the first logic unit; gives up after
/// `timeout`. Lets the capture end on its own ([`STATE_READ_END_WAIT`])
/// before stopping it.
pub fn read_state_by_capture(
    dev: &mut dyn Device,
    cfg: &DeviceConfig,
    timeout: std::time::Duration,
) -> Result<PinLevels, BackendError> {
    let digital: Vec<ChannelId> =
        cfg.enabled.iter().copied().filter(|c| c.kind == crate::core::ChannelKind::Digital).collect();
    if digital.is_empty() {
        return Ok(Vec::new());
    }
    dev.apply(&DeviceConfig { enabled: digital.clone(), ..cfg.clone() })?;
    // Some drivers (libsigrok's demo, now and then) end a capture that
    // follows a stopped one without any data: try again a few times.
    let no_data = || BackendError::Device("the device sent no data".into());
    for _ in 0..3 {
        let (tx, rx) = crossbeam_channel::bounded(2);
        let sink = FirstUnit { channels: digital.clone(), tx, sent: false };
        dev.start(AcquisitionPlan::new(AcquisitionLimit::Samples(1)), Box::new(sink))?;
        let got = rx.recv_timeout(timeout);
        if matches!(got, Ok(Read::Levels(_))) {
            // The data is in; the driver ends the capture right after.
            let _ = rx.recv_timeout(STATE_READ_END_WAIT.min(timeout));
        }
        dev.stop()?;
        match got {
            Ok(Read::Levels(l)) => return Ok(l),
            Ok(Read::Ended) => continue,
            Ok(Read::Failed(e)) => return Err(BackendError::Device(e)),
            Err(_) => return Err(BackendError::Device("the device sent no data in time".into())),
        }
    }
    Err(no_data())
}

/// Each channel's level.
pub type PinLevels = Vec<(ChannelId, bool)>;

/// What a pin-reading capture reported.
enum Read {
    Levels(PinLevels),
    /// Completed or stopped (after the levels, or without data).
    Ended,
    Failed(String),
}

/// Collects the first logic unit of a stream, then its end.
struct FirstUnit {
    channels: Vec<ChannelId>,
    tx: crossbeam_channel::Sender<Read>,
    sent: bool,
}

impl SampleSink for FirstUnit {
    fn on_header(&mut self, h: StreamHeader) {
        self.channels.retain(|c| h.logic_channels.contains(c));
    }

    fn on_logic(&mut self, _first: u64, unit_size: usize, data: &[u8]) {
        if data.len() < unit_size {
            return;
        }
        if std::mem::replace(&mut self.sent, true) {
            return;
        }
        // Bit `i` of a unit is digital channel index `i` (StreamHeader).
        let bit = |i: u16| data.get(i as usize / 8).is_some_and(|b| b >> (i % 8) & 1 == 1);
        let _ = self.tx.send(Read::Levels(self.channels.iter().map(|c| (*c, bit(c.index))).collect()));
    }

    fn on_analog(&mut self, _channel: ChannelId, _first: u64, _volts: &[f32]) {}

    fn on_hw_trigger(&mut self, _sample: u64) {}

    fn on_frame(&mut self, _begin: bool) {}

    fn on_end(&mut self, reason: EndReason) {
        let _ = self.tx.try_send(match reason {
            EndReason::Error(e) if !self.sent => Read::Failed(e),
            EndReason::Disconnected if !self.sent => Read::Failed("the device disconnected".into()),
            _ => Read::Ended,
        });
    }
}

/// Receives the sample stream. Called from a backend thread.
pub trait SampleSink: Send {
    /// Start time, sample rate(s) and channel layout.
    fn on_header(&mut self, h: StreamHeader);
    fn on_logic(&mut self, first_sample: u64, unit_size: usize, data: &[u8]);
    fn on_analog(&mut self, channel: crate::core::ChannelId, first_sample: u64, volts: &[f32]);
    fn on_hw_trigger(&mut self, sample: u64);
    /// [P2] oscilloscope frames.
    fn on_frame(&mut self, begin: bool);
    fn on_end(&mut self, reason: EndReason);
}

#[cfg(test)]
mod state_read_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A device that, like libsigrok's polling drivers, ends a limited
    /// capture by itself a moment after its data.
    struct SelfEnding {
        /// `end_delay`: data to END; `None` never ends (needs a stop).
        end_delay: Option<Duration>,
        log: Arc<Mutex<Vec<&'static str>>>,
        thread: Option<std::thread::JoinHandle<()>>,
        stopping: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Device for SelfEnding {
        fn info(&self) -> &DeviceInfo {
            unimplemented!()
        }
        fn capabilities(&self) -> &DeviceCaps {
            unimplemented!()
        }
        fn apply(&mut self, _cfg: &DeviceConfig) -> Result<AppliedConfig, BackendError> {
            Ok(AppliedConfig::default())
        }
        fn start(&mut self, _plan: AcquisitionPlan, mut sink: Box<dyn SampleSink>) -> Result<(), BackendError> {
            let (log, delay, stopping) = (self.log.clone(), self.end_delay, self.stopping.clone());
            self.thread = Some(std::thread::spawn(move || {
                sink.on_logic(0, 1, &[0b101]);
                log.lock().unwrap().push("data");
                let Some(delay) = delay else {
                    // Keeps the sink (and the capture) alive until stopped.
                    while !stopping.load(std::sync::atomic::Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    sink.on_end(EndReason::Stopped);
                    return;
                };
                std::thread::sleep(delay);
                log.lock().unwrap().push("end");
                sink.on_end(EndReason::Completed);
            }));
            Ok(())
        }
        fn stop(&mut self) -> Result<(), BackendError> {
            self.log.lock().unwrap().push("stop");
            self.stopping.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            Ok(())
        }
    }

    fn read(end_delay: Option<Duration>) -> (PinLevels, Vec<&'static str>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut dev = SelfEnding { end_delay, log: log.clone(), thread: None, stopping: Default::default() };
        let cfg = DeviceConfig::new(vec![ChannelId::digital(0), ChannelId::digital(2)], 1_000_000);
        let levels = read_state_by_capture(&mut dev, &cfg, Duration::from_secs(2)).expect("levels");
        let log = log.lock().unwrap().clone();
        (levels, log)
    }

    /// The capture ends on its own before it is stopped, so the
    /// stop never reaches a driver that already stopped itself.
    #[test]
    fn stops_after_the_capture_ended() {
        let (levels, log) = read(Some(Duration::from_millis(20)));
        assert_eq!(levels, vec![(ChannelId::digital(0), true), (ChannelId::digital(2), true)]);
        assert_eq!(log, ["data", "end", "stop"]);
    }

    /// A driver that keeps running is stopped after the bounded wait.
    #[test]
    fn stops_a_capture_that_never_ends() {
        let t0 = std::time::Instant::now();
        let (levels, log) = read(None);
        assert_eq!(levels.len(), 2);
        assert_eq!(log, ["data", "stop"]);
        assert!(t0.elapsed() >= STATE_READ_END_WAIT, "{:?}", t0.elapsed());
        assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
    }
}
