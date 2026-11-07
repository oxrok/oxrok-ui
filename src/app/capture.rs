//! Capture data shared between the acquisition thread and the UI, and the
//! [`SampleSink`] that fills it.

use crate::backend::{EndReason, SampleSink, StreamHeader};
use crate::core::capture::{CaptureMode, MemoryDecision, MemoryUse, StopReason, TriggerProgress, memory_policy};
use crate::core::{CaptureTime, ChannelId};
use crate::store::{AnalogChannel, DigitalChannel, LogicIngest};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Trim at most this often.
const TRIM_INTERVAL: Duration = Duration::from_millis(100);
/// Logic packets wait at most this long before they are stored…
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(20);
/// …or until this many bytes are waiting. Drivers send small packets (a
/// few KB). Waiting for the shared lock on each one would stall the
/// acquisition thread (which is also the driver's) behind every repaint.
const FLUSH_BYTES: usize = 8 << 20;

/// Logic data received but not stored yet: first sample, unit size, bytes,
/// and when the batch started.
#[derive(Debug)]
pub struct PendingLogic {
    first: u64,
    unit_size: usize,
    bytes: Vec<u8>,
    since: Instant,
}

/// Shared between the acquisition thread (appends) and whoever stores it
/// (either thread, always while holding the capture's write lock, so the
/// batches stay in order).
pub type SharedPending = Arc<parking_lot::Mutex<Option<PendingLogic>>>;

#[derive(Default)]
pub struct CaptureData {
    pub header: Option<StreamHeader>,
    pub ingest: Option<LogicIngest>,
    /// Channel ids in `ingest` order.
    pub ids: Vec<ChannelId>,
    /// Analog channels in header order.
    pub analog: Vec<(ChannelId, AnalogChannel)>,
    pub end: Option<EndReason>,
    /// The memory policy asked to stop.
    pub stop_request: Option<StopReason>,
    /// The memory policy discarded old data at least once.
    pub trimmed: bool,
    /// Logic data the acquisition thread hasn't stored yet.
    pub pending: SharedPending,
    /// The trigger sample, once found (written by the UI thread).
    pub trigger: Option<u64>,
    /// How far the software trigger has scanned (sample).
    pub trigger_scan: u64,
    /// The capture uses the device's trigger…
    pub hw_trigger_expected: bool,
    /// …which the device reported at this sample.
    pub hw_trigger: Option<u64>,
}

pub type SharedCapture = Arc<RwLock<CaptureData>>;

impl CaptureData {
    pub fn sample_rate(&self) -> Option<u64> {
        self.header.as_ref().map(|h| h.sample_rate)
    }

    /// A digital channel's data.
    pub fn channel(&self, id: ChannelId) -> Option<&DigitalChannel> {
        let k = self.ids.iter().position(|c| *c == id)?;
        self.ingest.as_ref()?.channels().get(k)
    }

    pub fn analog(&self, id: ChannelId) -> Option<&AnalogChannel> {
        self.analog.iter().find(|(c, _)| *c == id).map(|(_, a)| a)
    }

    /// Sample interval every channel has data for: `[begin, end)`.
    pub fn sample_range(&self) -> Option<(u64, u64)> {
        let digital = self.ingest.iter().flat_map(|i| i.channels().iter().map(DigitalChannel::data_range));
        let analog = self.analog.iter().map(|(_, a)| a.data_range());
        let ranges: Vec<_> = digital.chain(analog).collect();
        if ranges.is_empty() || ranges.iter().any(Option::is_none) {
            return None;
        }
        let begin = ranges.iter().flatten().map(|r| r.0).max()?;
        let end = ranges.iter().flatten().map(|r| r.1).min()?;
        (end > begin).then_some((begin, end))
    }

    /// Stored interval as times since capture start.
    pub fn time_range(&self) -> Option<(CaptureTime, CaptureTime)> {
        let rate = self.sample_rate()?;
        let (b, e) = self.sample_range()?;
        Some((CaptureTime::from_sample(b, rate), CaptureTime::from_sample(e, rate)))
    }

    /// A new ingest for this capture's channels.
    fn new_ingest(&self, unit_size: usize) -> LogicIngest {
        LogicIngest::new(unit_size, self.ids.iter().map(|c| c.index).collect())
    }

    /// Stores the pending logic data if it has waited at least `min_age`.
    /// Returns whether anything was stored.
    pub fn store_pending(&mut self, min_age: Duration) -> bool {
        let pending = self.pending.clone();
        let mut guard = pending.lock();
        if !guard.as_ref().is_some_and(|p| p.since.elapsed() >= min_age) {
            return false;
        }
        let p = guard.take().unwrap();
        drop(guard);
        if self.ingest.is_none() {
            self.ingest = Some(self.new_ingest(p.unit_size));
        }
        let ingest = self.ingest.as_mut().unwrap();
        if let Err(e) = ingest.push(p.first, &p.bytes) {
            self.end.get_or_insert(EndReason::Error(e.to_string()));
        }
        true
    }

    pub fn memory_bytes(&self) -> usize {
        self.ingest.as_ref().map_or(0, LogicIngest::memory_bytes)
            + self.analog.iter().map(|(_, a)| a.memory_bytes()).sum::<usize>()
    }
}

/// Writes the stream into a [`SharedCapture`] and applies the memory
/// policy. Runs on the backend's acquisition thread.
pub struct StoreSink {
    data: SharedCapture,
    repaint: Box<dyn Fn() + Send>,
    mode: CaptureMode,
    buffer_bytes: f64,
    last_trim: Option<Instant>,
    pending: SharedPending,
}

impl StoreSink {
    pub fn new(data: SharedCapture, mode: CaptureMode, buffer_mb: u32, repaint: Box<dyn Fn() + Send>) -> Self {
        let pending = data.read().pending.clone();
        Self { data, repaint, mode, buffer_bytes: buffer_mb as f64 * 1e6, last_trim: None, pending }
    }

    /// Stores the batched logic data now.
    pub fn flush(&mut self) {
        let shared = self.data.clone();
        let mut d = shared.write();
        self.store(&mut d);
    }

    fn store(&mut self, d: &mut CaptureData) {
        if d.store_pending(Duration::ZERO) {
            self.apply_memory_policy(d);
            (self.repaint)();
        }
    }

    fn apply_memory_policy(&mut self, d: &mut CaptureData) {
        let (Some(rate), Some((begin, end))) = (d.sample_rate(), d.sample_range()) else { return };
        let use_ = MemoryUse {
            processed_mb: d.memory_bytes() as f64 / 1e6,
            backlog_mb: 0.0,
            buffer_mb: self.buffer_bytes / 1e6,
            processed_secs: (end - begin) as f64 / rate as f64,
        };
        // Triggered mode needs the trigger's state.
        let trigger = (self.mode == crate::core::capture::CaptureMode::Triggered).then(|| match d.trigger {
            Some(t) => TriggerProgress::Found { at_secs: t.saturating_sub(begin) as f64 / rate as f64 },
            None => TriggerProgress::Searching {
                unsearched_secs: end.saturating_sub(d.trigger_scan.max(begin)) as f64 / rate as f64,
            },
        });
        match memory_policy(self.mode, use_, trigger) {
            MemoryDecision::Keep => {}
            MemoryDecision::Stop(reason) => d.stop_request = Some(reason),
            MemoryDecision::Trim { keep_secs } => {
                if self.last_trim.is_some_and(|t| t.elapsed() < TRIM_INTERVAL) {
                    return;
                }
                self.last_trim = Some(Instant::now());
                let keep = (keep_secs * rate as f64) as u64;
                let new_begin = end.saturating_sub(keep).max(begin);
                if let Some(ingest) = d.ingest.as_mut() {
                    for ch in ingest.channels_mut() {
                        ch.trim_before(new_begin);
                    }
                }
                for (_, a) in &mut d.analog {
                    a.trim_before(new_begin);
                }
                d.trimmed = true;
            }
        }
    }
}

impl SampleSink for StoreSink {
    fn on_header(&mut self, h: StreamHeader) {
        let mut d = self.data.write();
        d.ids = h.logic_channels.clone();
        d.analog = h.analog_channels.iter().map(|&c| (c, AnalogChannel::new())).collect();
        d.header = Some(h);
        drop(d);
        (self.repaint)();
    }

    fn on_logic(&mut self, first_sample: u64, unit_size: usize, data: &[u8]) {
        let continues =
            |p: &PendingLogic| p.unit_size == unit_size && p.first + (p.bytes.len() / unit_size) as u64 == first_sample;
        // A packet that doesn't continue the batch is stored on its own, so
        // the ingest still reports the discontinuity.
        if self.pending.lock().as_ref().is_some_and(|p| !continues(p)) {
            self.flush();
        }
        // Not being painted: store the waiting batch and this packet in
        // place, without copying it.
        if let Some(mut d) = self.data.clone().try_write() {
            d.store_pending(Duration::ZERO);
            if d.ingest.is_none() {
                d.ingest = Some(d.new_ingest(unit_size));
            }
            let ingest = d.ingest.as_mut().unwrap();
            if let Err(e) = ingest.push(first_sample, data) {
                d.end.get_or_insert(EndReason::Error(e.to_string()));
            }
            self.apply_memory_policy(&mut d);
            drop(d);
            (self.repaint)();
            return;
        }
        let urgent = {
            let mut guard = self.pending.lock();
            let p = guard.get_or_insert_with(|| PendingLogic {
                first: first_sample,
                unit_size,
                bytes: Vec::with_capacity(data.len()),
                since: Instant::now(),
            });
            p.bytes.extend_from_slice(data);
            p.bytes.len() >= FLUSH_BYTES || p.since.elapsed() >= FLUSH_INTERVAL
        };
        if urgent {
            self.flush();
        }
    }

    fn on_analog(&mut self, channel: ChannelId, first_sample: u64, volts: &[f32]) {
        let shared = self.data.clone();
        let mut d = shared.write();
        let Some((_, ch)) = d.analog.iter_mut().find(|(c, _)| *c == channel) else { return };
        if let Err((expected, got)) = ch.push(first_sample, volts) {
            let msg = format!("analog packet starts at sample {got}, expected {expected}");
            d.end.get_or_insert(EndReason::Error(msg));
        } else {
            self.apply_memory_policy(&mut d);
        }
        drop(d);
        (self.repaint)();
    }

    fn on_hw_trigger(&mut self, sample: u64) {
        self.data.write().hw_trigger.get_or_insert(sample);
        (self.repaint)();
    }

    fn on_frame(&mut self, _begin: bool) {}

    fn on_end(&mut self, reason: EndReason) {
        self.flush();
        self.data.write().end.get_or_insert(reason);
        (self.repaint)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::WallClock;

    fn header(rate: u64) -> StreamHeader {
        StreamHeader {
            start: WallClock::default(),
            sample_rate: rate,
            logic_unit_size: 1,
            logic_channels: vec![ChannelId::digital(0)],
            analog_channels: vec![ChannelId::analog(0)],
        }
    }

    #[test]
    fn continuous_trims_oldest_data() {
        let data = SharedCapture::default();
        // A tiny buffer: a few hundred bytes.
        let mut sink = StoreSink::new(data.clone(), CaptureMode::Continuous, 0, Box::new(|| {}));
        sink.buffer_bytes = 200.0;
        sink.on_header(header(1_000));
        // Toggle every sample: one transition per sample.
        let packet: Vec<u8> = (0..1000).map(|i| (i % 2) as u8).collect();
        let volts = vec![1.0f32; 1000];
        for k in 0..20u64 {
            sink.on_logic(k * 1000, 1, &packet);
            sink.flush();
            sink.on_analog(ChannelId::analog(0), k * 1000, &volts);
            sink.last_trim = None;
        }
        let d = data.read();
        assert!(d.trimmed);
        let (b, e) = d.sample_range().unwrap();
        assert_eq!(e, 20_000);
        assert!(b > 0, "oldest data discarded");
        // Never below 0.1 s = 100 samples.
        assert!(e - b >= 100);
        assert_eq!(d.analog(ChannelId::analog(0)).unwrap().data_range().unwrap().1, 20_000);
    }

    #[test]
    fn timed_mode_requests_stop_when_full() {
        let data = SharedCapture::default();
        let mut sink = StoreSink::new(data.clone(), CaptureMode::Timed, 0, Box::new(|| {}));
        sink.buffer_bytes = 50.0;
        sink.on_header(header(1_000));
        let packet: Vec<u8> = (0..1000).map(|i| (i % 2) as u8).collect();
        sink.on_logic(0, 1, &packet);
        sink.flush();
        sink.on_analog(ChannelId::analog(0), 0, &[0.0; 1000]);
        assert_eq!(data.read().stop_request, Some(StopReason::BufferFull));
    }
}
