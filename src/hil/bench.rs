//! Captures from the LogicPort through [`OxrokBackend`] into our stores.

use crate::backend::{
    AcquisitionLimit, AcquisitionPlan, Backend, Device, DeviceConfig, DeviceInfo, EndReason, HwTrigger, SampleSink,
    StreamHeader,
};
use crate::core::ChannelId;
use crate::oxrok::OxrokBackend;
use crate::store::{DigitalChannel, LogicIngest};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const DRIVER: &str = "intronix-logicport";
/// Without RLE the LogicPort always captures this many samples.
pub const MEM_DEPTH: u64 = 2048;

pub struct Bench {
    pub backend: OxrokBackend,
    pub device: Box<dyn Device>,
    pub info: DeviceInfo,
}

#[derive(Clone, Debug)]
pub struct CaptureSpec {
    pub rate: u64,
    pub samples: u64,
    pub channels: Vec<ChannelId>,
    pub trigger: Option<HwTrigger>,
    pub rle: bool,
}

impl CaptureSpec {
    pub fn new(rate: u64, channels: &[ChannelId]) -> Self {
        Self { rate, samples: MEM_DEPTH, channels: channels.to_vec(), trigger: None, rle: false }
    }

    pub fn trigger(mut self, t: HwTrigger) -> Self {
        self.trigger = Some(t);
        self
    }
}

pub struct Captured {
    pub rate: u64,
    pub ids: Vec<ChannelId>,
    pub channels: Vec<DigitalChannel>,
    pub trigger: Option<u64>,
    pub end: EndReason,
}

impl Captured {
    pub fn get(&self, id: ChannelId) -> &DigitalChannel {
        let k = self.ids.iter().position(|c| *c == id).unwrap_or_else(|| panic!("{id:?} not captured"));
        &self.channels[k]
    }

    pub fn samples(&self) -> u64 {
        self.channels.first().and_then(DigitalChannel::data_range).map_or(0, |(b, e)| e - b)
    }

    /// All transitions of a channel.
    pub fn edges(&self, id: ChannelId) -> Vec<u64> {
        self.get(id).transitions_from(0).collect()
    }
}

#[derive(Default)]
struct State {
    header: Option<StreamHeader>,
    ingest: Option<LogicIngest>,
    trigger: Option<u64>,
    end: Option<EndReason>,
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<State>>);

impl SampleSink for Sink {
    fn on_header(&mut self, h: StreamHeader) {
        self.0.lock().header = Some(h);
    }
    fn on_logic(&mut self, first: u64, unit_size: usize, data: &[u8]) {
        let mut s = self.0.lock();
        let bits = s.header.as_ref().map(|h| h.logic_channels.iter().map(|c| c.index).collect()).unwrap_or_default();
        let ingest = s.ingest.get_or_insert_with(|| LogicIngest::new(unit_size, bits));
        if let Err(e) = ingest.push(first, data) {
            s.end = Some(EndReason::Error(e.to_string()));
        }
    }
    fn on_analog(&mut self, _: ChannelId, _: u64, _: &[f32]) {}
    fn on_hw_trigger(&mut self, sample: u64) {
        self.0.lock().trigger.get_or_insert(sample);
    }
    fn on_frame(&mut self, _: bool) {}
    fn on_end(&mut self, reason: EndReason) {
        self.0.lock().end.get_or_insert(reason);
    }
}

impl Bench {
    /// Opens the first LogicPort, scanning only its driver.
    pub fn open() -> Result<Self, String> {
        let backend = OxrokBackend::with_drivers(Some(vec![DRIVER.into()]));
        let info = backend
            .scan_devices()
            .into_iter()
            .find(|d| d.driver == DRIVER)
            .ok_or("no LogicPort found (is it plugged in?)")?;
        let device = backend.open(&info.id).map_err(|e| e.to_string())?;
        Ok(Self { backend, device, info })
    }

    /// Runs one capture. `stimulus` is called once the acquisition is under
    /// way, then again every `repeat` until the capture ends, so a trigger
    /// missed while the device was still arming is retried.
    pub fn capture(
        &mut self,
        spec: &CaptureSpec,
        mut stimulus: impl FnMut(),
        repeat: Duration,
    ) -> Result<Captured, String> {
        let config = DeviceConfig::new(spec.channels.clone(), spec.rate);
        let applied = self.device.apply(&config).map_err(|e| e.to_string())?;
        if applied.sample_rate != spec.rate {
            return Err(format!("device runs at {} S/s, wanted {}", applied.sample_rate, spec.rate));
        }
        let sink = Sink::default();
        let plan = AcquisitionPlan {
            limit: AcquisitionLimit::Samples(spec.samples),
            trigger: spec.trigger.clone(),
            rle: spec.rle,
        };
        self.device.start(plan, Box::new(sink.clone())).map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        // Let the acquisition start (FPGA configuration and threshold DAC
        // settling take a while) before the first stimulus.
        let mut next = Some(t0 + Duration::from_millis(400));
        loop {
            if sink.0.lock().end.is_some() {
                break;
            }
            if t0.elapsed() > Duration::from_secs(60) {
                let _ = self.device.stop();
                return Err("capture timed out (trigger never fired?)".into());
            }
            if next.is_some_and(|n| Instant::now() >= n) {
                stimulus();
                // `Duration::MAX` means "once".
                next = Instant::now().checked_add(repeat);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = self.device.stop();
        let mut s = sink.0.lock();
        let end = s.end.take().unwrap_or(EndReason::Completed);
        if let EndReason::Error(e) = &end {
            return Err(e.clone());
        }
        let ids = s.header.as_ref().map(|h| h.logic_channels.clone()).unwrap_or_default();
        let channels = s.ingest.take().map(LogicIngest::into_channels).unwrap_or_default();
        Ok(Captured { rate: spec.rate, ids, channels, trigger: s.trigger, end })
    }
}
