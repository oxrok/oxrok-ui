//! Capture files: the session state in `metadata.json`, saving and
//! loading on a background thread, and the unsaved-capture fingerprint

use crate::app::analyzers::Analyzer;
use crate::app::capture::{CaptureData, SharedCapture};
use crate::app::session::{Row, Session};
use crate::app::worker::SessionId;
use crate::backend::{DeviceInfo, EndReason, OptionValue, StreamHeader};
use crate::core::capture::{CaptureSettings, CaptureSnapshot, CaptureState};
use crate::core::markers::{Markers, TimeOrigin};
use crate::core::ranges::Ranges;
use crate::core::{CaptureTime, ChannelId, ChannelKind, ViewState, WallClock};
use crate::decode::Radix;
use crate::store::LogicIngest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub use crate::file::{EXTENSION, FileError};

/// An analyzer as saved: its configuration, not its results.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerMeta {
    pub id: u32,
    pub decoder: String,
    pub name: String,
    pub color: [u8; 3],
    pub channels: Vec<(String, ChannelId)>,
    pub options: BTreeMap<String, OptionValue>,
    pub radix: Radix,
    pub show_in_table: bool,
    pub to_terminal: bool,
    pub rows_visible: Vec<String>,
}

impl AnalyzerMeta {
    fn of(a: &Analyzer) -> Self {
        Self {
            id: a.id,
            decoder: a.decoder.clone(),
            name: a.name.clone(),
            color: a.color,
            channels: a.channels.clone(),
            options: a.options.clone(),
            radix: a.radix,
            show_in_table: a.show_in_table,
            to_terminal: a.to_terminal,
            rows_visible: a.rows_visible.clone(),
        }
    }

    fn into_analyzer(self) -> Analyzer {
        Analyzer {
            id: self.id,
            decoder: self.decoder,
            name: self.name,
            color: self.color,
            channels: self.channels,
            options: self.options,
            radix: self.radix,
            show_in_table: self.show_in_table,
            to_terminal: self.to_terminal,
            rows_visible: self.rows_visible,
            out: Default::default(),
        }
    }
}

/// What a session can change without new samples. Also what
/// the unsaved fingerprint covers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotations {
    pub name: String,
    pub rows: Vec<Row>,
    pub analyzers: Vec<AnalyzerMeta>,
    pub markers: Markers,
    pub ranges: Ranges,
    pub origin: TimeOrigin,
    pub notes: String,
    #[serde(default)]
    pub analyzer_trigger: crate::app::analyzers::AnalyzerTrigger,
    /// Signal groups (their rows are in `rows`).
    #[serde(default)]
    pub buses: Vec<crate::core::bus::Bus>,
}

/// `metadata.json`: everything except the samples.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub schema_version: u32,
    pub app_version: String,
    pub device: DeviceInfo,
    pub settings: CaptureSettings,
    pub snapshot: Option<CaptureSnapshot>,
    /// Wall-clock capture start.
    pub start: WallClock,
    pub sample_rate: u64,
    pub logic_unit_size: usize,
    pub trigger: Option<CaptureTime>,
    /// Nested, not flattened: serde's flatten buffers values in a form
    /// that can't hold the `i128` capture times.
    pub annotations: Annotations,
    pub view: Option<ViewState>,
    #[serde(default)]
    pub table_search: String,
}

impl Annotations {
    pub fn of(s: &Session) -> Self {
        Self {
            name: s.name.clone(),
            rows: s.rows.clone(),
            analyzers: s.analyzers.iter().map(AnalyzerMeta::of).collect(),
            markers: s.markers.clone(),
            ranges: s.ranges.clone(),
            origin: s.origin,
            notes: s.notes.clone(),
            analyzer_trigger: s.analyzer_trigger.clone(),
            buses: s.buses.clone(),
        }
    }

    /// Changes when anything saved changes. The view and analog
    /// row zoom don't count.
    pub fn fingerprint(&self) -> u64 {
        let mut a = self.clone();
        for r in &mut a.rows {
            r.analog = None;
            r.analog_inner = None;
        }
        let json = serde_json::to_vec(&a).unwrap_or_default();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        json.hash(&mut h);
        h.finish()
    }
}

impl SessionMeta {
    pub fn of(s: &Session, table_search: &str) -> Option<Self> {
        let cap = s.capture.as_ref()?.read();
        let header = cap.header.as_ref()?;
        Some(Self {
            schema_version: crate::file::SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            device: s.device.clone(),
            settings: s.settings.clone(),
            snapshot: s.snapshot.clone(),
            start: header.start,
            sample_rate: header.sample_rate,
            logic_unit_size: header.logic_unit_size,
            trigger: s.trigger,
            annotations: Annotations::of(s),
            view: Some(s.view.target()),
            table_search: table_search.to_owned(),
        })
    }
}

/// Forward migrations, oldest first, on the raw `metadata.json`.
/// Schema 1 is the first; later schemas add a step here that turns the
/// previous version's bytes into the next one's.
pub fn migrate(meta: Vec<u8>) -> Result<Vec<u8>, FileError> {
    match crate::file::schema_version(&meta)? {
        1 => Ok(meta),
        v => Err(FileError::Format(format!("unknown capture schema {v}"))),
    }
}

fn meta_bytes(meta: &SessionMeta) -> Result<Vec<u8>, FileError> {
    serde_json::to_vec_pretty(meta).map_err(|e| FileError::Format(e.to_string()))
}

/// Saves `capture` with `meta` to `path`, in full.
pub fn save(
    path: &Path,
    meta: &SessionMeta,
    capture: &SharedCapture,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<(), FileError> {
    let json = meta_bytes(meta)?;
    let cap = capture.read();
    let digital: Vec<_> = cap.ids.iter().filter_map(|id| Some((*id, cap.channel(*id)?))).collect();
    let analog: Vec<_> = cap.analog.iter().map(|(id, a)| (*id, a)).collect();
    crate::file::save(path, &json, &digital, &analog, progress)
}

/// Rewrites only `metadata.json` of an existing `.oxrok`.
pub fn save_annotations(path: &Path, meta: &SessionMeta) -> Result<(), FileError> {
    crate::file::update_meta(path, &meta_bytes(meta)?)
}

/// A file read into memory, ready to become a session on the UI thread.
pub struct LoadedCapture {
    pub path: PathBuf,
    pub meta: SessionMeta,
    pub capture: SharedCapture,
}

pub fn load(path: &Path, progress: &mut dyn FnMut(f32) -> bool) -> Result<LoadedCapture, FileError> {
    let loaded = crate::file::load(path, progress)?;
    let meta: SessionMeta = serde_json::from_slice(&migrate(loaded.meta)?)?;
    let mut data = CaptureData {
        header: Some(StreamHeader {
            start: meta.start,
            sample_rate: meta.sample_rate,
            logic_unit_size: meta.logic_unit_size.max(1),
            logic_channels: loaded.digital.iter().map(|c| c.0).collect(),
            analog_channels: loaded.analog.iter().map(|c| c.0).collect(),
        }),
        end: Some(EndReason::Completed),
        ..Default::default()
    };
    data.ids = loaded.digital.iter().map(|c| c.0).collect();
    data.trigger = meta.trigger.map(|t| t.to_sample_floor(meta.sample_rate).max(0) as u64);
    if !loaded.digital.is_empty() {
        let bits = data.ids.iter().map(|c| c.index).collect();
        let channels = loaded.digital.into_iter().map(|c| c.1).collect();
        data.ingest = Some(LogicIngest::from_channels(meta.logic_unit_size.max(1), bits, channels));
    }
    data.analog = loaded.analog;
    Ok(LoadedCapture { path: path.to_owned(), meta, capture: std::sync::Arc::new(parking_lot::RwLock::new(data)) })
}

/// A read-only session from a loaded file.
pub fn session_from(id: SessionId, l: LoadedCapture, width_px: f64) -> Session {
    let m = l.meta;
    let mut s = Session::new(id, m.device.clone(), Some(m.settings.clone()), width_px);
    let a = m.annotations;
    s.name = a.name;
    // Rows the file knows; any channel it lacks keeps its default row.
    s.buses = a.buses;
    s.apply_rows(a.rows);
    s.analyzers = a.analyzers.into_iter().map(AnalyzerMeta::into_analyzer).collect();
    s.markers = a.markers;
    s.ranges = a.ranges;
    s.origin = a.origin;
    s.notes = a.notes;
    // An opened capture keeps its saved trigger.
    s.analyzer_trigger = a.analyzer_trigger;
    s.trigger_scan = Default::default();
    s.snapshot = m.snapshot;
    s.trigger = m.trigger;
    s.state = CaptureState::Finished;
    s.capture = Some(l.capture);
    s.capture_done.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(v) = m.view {
        s.view = crate::core::animator::ViewAnimator::new(ViewState { width_px: width_px.max(1.0), ..v });
    }
    // The snapshot's channels are the ones with data.
    if let Some(snap) = &s.snapshot {
        s.settings.enabled = snap.enabled.clone();
        // What the device accepted for this capture.
        s.settings.logic_threshold = snap.logic_threshold.clone();
        s.settings.device_options = snap.device_options.clone();
    }
    s.opened_from = Some(l.path.clone());
    if l.path.extension().is_some_and(|e| e == EXTENSION) {
        s.file = Some(l.path);
    }
    s.saved = Some(Annotations::of(&s).fingerprint());
    s.samples_saved = s.file.is_some();
    s
}

/// Collects an import's stream into a capture. Nothing
/// paints it until the import is done, so every packet goes straight in.
pub struct ImportSink {
    pub data: SharedCapture,
}

impl crate::backend::SampleSink for ImportSink {
    fn on_header(&mut self, h: StreamHeader) {
        let mut d = self.data.write();
        d.ids = h.logic_channels.clone();
        d.analog = h.analog_channels.iter().map(|&c| (c, crate::store::AnalogChannel::new())).collect();
        d.header = Some(h);
    }

    fn on_logic(&mut self, first: u64, unit_size: usize, bytes: &[u8]) {
        let mut d = self.data.write();
        if d.ingest.is_none() {
            let bits = d.ids.iter().map(|c| c.index).collect();
            d.ingest = Some(LogicIngest::new(unit_size, bits));
        }
        if let Err(e) = d.ingest.as_mut().unwrap().push(first, bytes) {
            d.end.get_or_insert(EndReason::Error(e.to_string()));
        }
    }

    fn on_analog(&mut self, channel: ChannelId, first: u64, volts: &[f32]) {
        let mut d = self.data.write();
        if let Some((_, ch)) = d.analog.iter_mut().find(|(c, _)| *c == channel)
            && ch.push(first, volts).is_err()
        {
            d.end.get_or_insert(EndReason::Error("analog samples out of order".into()));
        }
    }

    fn on_hw_trigger(&mut self, _sample: u64) {}

    fn on_frame(&mut self, _begin: bool) {}

    fn on_end(&mut self, reason: EndReason) {
        self.data.write().end.get_or_insert(reason);
    }
}

/// A read-only session from an imported file. It has no
/// `.oxrok` yet, so its first Save is a Save As.
pub fn imported_session(
    id: SessionId,
    path: &Path,
    info: crate::backend::ImportedCapture,
    capture: SharedCapture,
    width_px: f64,
) -> Session {
    let enabled: Vec<ChannelId> = info.channels.iter().map(|c| c.id).collect();
    let device = DeviceInfo {
        id: crate::backend::DeviceId::new("file", &path.display().to_string()),
        driver: "file".into(),
        vendor: String::new(),
        model: info.format.clone(),
        version: String::new(),
        serial_or_conn: path.display().to_string(),
        conn: None,
        is_demo: false,
        channels: info.channels,
    };
    let settings = CaptureSettings { enabled: enabled.clone(), sample_rate: info.sample_rate, ..Default::default() };
    let mut s = Session::new(id, device, Some(settings.clone()), width_px);
    s.name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
    s.snapshot = Some(CaptureSnapshot {
        mode: settings.mode,
        sample_rate: info.sample_rate,
        enabled,
        buffer_mb: settings.buffer_mb,
        timer_seconds: settings.timer_seconds,
        rle: false,
        trigger: None,
        logic_threshold: None,
        device_options: Default::default(),
    });
    s.state = CaptureState::Finished;
    s.capture = Some(capture);
    s.capture_done.store(true, std::sync::atomic::Ordering::SeqCst);
    s.opened_from = Some(path.to_owned());
    // Nothing is lost by closing an untouched import: it counts as saved.
    s.saved = Some(Annotations::of(&s).fingerprint());
    s
}

/// The channels of a kind, for code that needs counts.
pub fn channel_count(meta: &SessionMeta, kind: ChannelKind) -> usize {
    meta.snapshot.as_ref().map_or(0, |s| s.enabled.iter().filter(|c| c.kind == kind).count())
}

/// The default file name, the tab name without characters that
/// file systems reject.
pub fn default_file_name(tab: &str) -> String {
    let clean: String =
        tab.chars().map(|c| if r#"/\:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_matches('.').to_owned();
    format!("{}.{EXTENSION}", if clean.is_empty() { "capture" } else { &clean })
}

/// Appends `.oxrok` when the user typed no extension.
pub fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_none() { path.with_extension(EXTENSION) } else { path }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(default_file_name("Session 0"), "Session 0.oxrok");
        assert_eq!(default_file_name("a/b: c?"), "a_b_ c_.oxrok");
        assert_eq!(default_file_name("  "), "capture.oxrok");
        assert_eq!(with_extension("x/cap".into()), PathBuf::from("x/cap.oxrok"));
        assert_eq!(with_extension("x/cap.bin".into()), PathBuf::from("x/cap.bin"));
    }
}
