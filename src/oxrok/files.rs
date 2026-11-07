//! Files through oxrok: sigrok sessions (`.sr`), the input
//! modules of, and `.sr` export.

use crate::backend::*;
use crate::core::{ChannelId, WallClock};
use oxrok::core::{AnalogData, Channel, ChannelKind, Packet, Value};
use oxrok::formats::input::{self};
use oxrok::formats::sr::{SessionReader, SessionWriter};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The input modules this build offers, in oxrok's order.
/// Modules oxrok does not have are skipped; `.sr` is handled separately.
pub const ALLOWED_INPUTS: &[&str] = &["vcd", "binary"];

/// File bytes fed to an input module per call.
const CHUNK: usize = 4 << 20;
/// Samples packed per logic packet on export.
const EXPORT_CHUNK: usize = 1 << 20;

pub struct ImportJob {
    pub path: PathBuf,
    pub format: ImportFormat,
    pub options: BTreeMap<String, OptionValue>,
    pub sink: Box<dyn SampleSink>,
    pub progress: ProgressFn,
}

/// The allowed modules with their options.
pub fn input_formats() -> Vec<InputFormat> {
    let mut out = Vec::new();
    for id in ALLOWED_INPUTS {
        let Some(m) = input::find(id) else { continue };
        // libsigrok's binary module claims `.bin`; oxrok's module lists no
        // extension, so files could not be routed to it.
        let mut extensions: Vec<String> = m.extensions.iter().map(|e| (*e).to_owned()).collect();
        if *id == "binary" {
            extensions.push("bin".to_owned());
        }
        out.push(InputFormat {
            id: m.id.to_owned(),
            name: m.name.to_owned(),
            description: m.description.to_owned(),
            extensions,
            options: m
                .options
                .iter()
                .map(|o| {
                    let kind = match o.kind {
                        input::InputOptionKind::U64 | input::InputOptionKind::I32 => InputOptionKind::Int,
                        input::InputOptionKind::Str => InputOptionKind::Str,
                        input::InputOptionKind::Bool => InputOptionKind::Bool,
                    };
                    let default = match kind {
                        InputOptionKind::Str => OptionValue::Str(o.default.to_owned()),
                        InputOptionKind::Bool => OptionValue::Int(i64::from(o.default == "true" || o.default == "1")),
                        _ => OptionValue::Int(o.default.parse().unwrap_or(0)),
                    };
                    InputOption {
                        id: o.id.to_owned(),
                        name: o.name.to_owned(),
                        description: o.description.to_owned(),
                        kind,
                        default,
                        values: Vec::new(),
                    }
                })
                .collect(),
        });
    }
    out
}

/// Content detection: the allowed module that claims the file.
pub fn detect(path: &Path) -> Option<String> {
    let mut head = vec![0; 4096];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    head.truncate(n);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let m = input::detect(&head, name.as_deref())?;
    ALLOWED_INPUTS.contains(&m.id).then(|| m.id.to_owned())
}

/// A module option map in oxrok's text form.
fn module_options(options: &BTreeMap<String, OptionValue>) -> input::Options {
    options.iter().map(|(k, v)| (k.clone(), v.cli())).collect()
}

/// The channels with their app ids: index within type.
fn channel_ids(channels: &[Channel]) -> Vec<(usize, ChannelInfo)> {
    let (mut d, mut a) = (0u16, 0u16);
    channels
        .iter()
        .map(|c| {
            let id = match c.kind {
                ChannelKind::Logic => {
                    d += 1;
                    ChannelId::digital(d - 1)
                }
                ChannelKind::Analog => {
                    a += 1;
                    ChannelId::analog(a - 1)
                }
            };
            (c.index, ChannelInfo { id, device_name: c.name.clone() })
        })
        .collect()
}

/// Turns packet streams into [`SampleSink`] calls.
struct Feed {
    sink: Box<dyn SampleSink>,
    /// oxrok channel index → app channel.
    channels: Vec<(usize, ChannelInfo)>,
    header: Option<StreamHeader>,
    rate: u64,
    pos: u64,
    /// Per analog channel: oxrok index, id, next sample.
    analog: Vec<(usize, ChannelId, u64)>,
    error: Option<String>,
    ended: bool,
}

impl Feed {
    fn new(sink: Box<dyn SampleSink>, channels: Vec<(usize, ChannelInfo)>) -> Self {
        let analog = channels
            .iter()
            .filter(|(_, c)| c.id.kind == crate::core::ChannelKind::Analog)
            .map(|(i, c)| (*i, c.id, 0))
            .collect();
        Feed { sink, channels, header: None, rate: 0, pos: 0, analog, error: None, ended: false }
    }

    /// Builds and sends the header, once. `unit_size` comes from the first
    /// logic packet, which may differ from `ceil(count / 8)` in a file.
    fn header(&mut self, unit_size: usize) {
        if self.header.is_some() {
            return;
        }
        let logic: Vec<ChannelId> =
            self.channels.iter().map(|(_, c)| c.id).filter(|id| id.kind == crate::core::ChannelKind::Digital).collect();
        let analog: Vec<ChannelId> =
            self.channels.iter().map(|(_, c)| c.id).filter(|id| id.kind == crate::core::ChannelKind::Analog).collect();
        let unit_size = unit_size.max(if logic.is_empty() { 0 } else { logic.len().div_ceil(8) });
        let h = StreamHeader {
            start: WallClock::now(),
            sample_rate: self.rate,
            logic_unit_size: unit_size,
            logic_channels: logic,
            analog_channels: analog,
        };
        self.header = Some(h.clone());
        self.sink.on_header(h);
    }

    fn packet(&mut self, packet: &Packet) {
        match packet {
            // Imports learn the rate from a META packet after the header, so
            // the header waits until the first data.
            Packet::Header { .. } => {}
            Packet::Meta(items) => {
                if self.header.is_none() {
                    for (k, v) in items {
                        if k.id() == "samplerate"
                            && let Value::U64(rate) = v
                        {
                            self.rate = *rate;
                        }
                    }
                }
            }
            Packet::Logic(d) => {
                self.header(d.unit_size);
                self.sink.on_logic(self.pos, d.unit_size, &d.data);
                self.pos += d.len() as u64;
            }
            Packet::Analog(a) => {
                self.header(0);
                self.analog_packet(a);
            }
            Packet::Trigger => self.sink.on_hw_trigger(self.pos),
            Packet::FrameBegin => self.sink.on_frame(true),
            Packet::FrameEnd => self.sink.on_frame(false),
            Packet::Failed(why) => self.error = Some(why.clone()),
            Packet::End => {
                let reason = match self.error.take() {
                    Some(e) => EndReason::Error(e),
                    None => EndReason::Completed,
                };
                self.header(0);
                self.sink.on_end(reason);
                self.ended = true;
            }
        }
    }

    /// Splits the interleaved samples of the channels the file has.
    fn analog_packet(&mut self, a: &AnalogData) {
        let n = a.len();
        if n == 0 {
            return;
        }
        let columns = a.channels.len().max(1);
        for slot in 0..self.analog.len() {
            let index = self.analog[slot].0;
            let Some(k) = a.channels.iter().position(|c| c == &index) else { continue };
            let mut column = Vec::with_capacity(n);
            column.extend((0..n).map(|s| a.samples[s * columns + k]));
            let (_, id, pos) = self.analog[slot];
            self.sink.on_analog(id, pos, &column);
            self.analog[slot].2 += n as u64;
        }
    }
}

fn err(e: impl std::fmt::Display) -> BackendError {
    BackendError::Device(e.to_string())
}

/// A sigrok session.
fn import_session(
    path: &Path,
    sink: Box<dyn SampleSink>,
    mut progress: ProgressFn,
) -> Result<ImportedCapture, BackendError> {
    let mut reader = SessionReader::open(path).map_err(err)?;
    let rate = reader.metadata().samplerate.unwrap_or(0);
    let channels = channel_ids(reader.channels());
    let mut feed = Feed::new(sink, channels);
    feed.rate = rate;
    let total = path.metadata().map(|m| m.len()).unwrap_or(0);
    let mut done = 0u64;
    reader
        .read(&mut |packet| {
            feed.packet(&packet);
            if let Packet::Logic(d) = packet {
                done += d.data.len() as u64;
                if total > 0 {
                    progress((done as f64 / total as f64).min(1.0) as f32);
                }
            }
        })
        .map_err(err)?;
    if !feed.ended {
        feed.header(0);
        feed.sink.on_end(EndReason::Error("the session file ended unexpectedly".into()));
    }
    Ok(ImportedCapture {
        channels: feed.channels.into_iter().map(|(_, c)| c).collect(),
        sample_rate: rate,
        format: "Sigrok session".to_owned(),
    })
}

/// A libsigrok-compatible input module.
fn import_module(
    path: &Path,
    id: &str,
    options: &BTreeMap<String, OptionValue>,
    sink: Box<dyn SampleSink>,
    mut progress: ProgressFn,
) -> Result<ImportedCapture, BackendError> {
    let module = input::find(id).ok_or_else(|| BackendError::Unsupported(format!("no input module named {id}")))?;
    let mut input = module.instantiate(&module_options(options)).map_err(err)?;
    let total = path.metadata().map(|m| m.len()).unwrap_or(0);
    let mut file = std::fs::File::open(path).map_err(|e| BackendError::Io(e.to_string()))?;
    let mut feed: Option<Feed> = None;
    let mut sink = Some(sink);
    let mut buf = vec![0; CHUNK];
    let mut done = 0u64;
    let mut packets = Vec::new();
    loop {
        let n = file.read(&mut buf).map_err(|e| BackendError::Io(e.to_string()))?;
        packets.clear();
        if n == 0 {
            input.end(&mut packets).map_err(err)?;
        } else {
            input.feed(&buf[..n], &mut packets).map_err(err)?;
            done += n as u64;
            if total > 0 {
                progress((done as f64 / total as f64).min(1.0) as f32);
            }
        }
        if feed.is_none()
            && let Some(channels) = input.channels()
            && let Some(sink) = sink.take()
        {
            feed = Some(Feed::new(sink, channel_ids(channels)));
        }
        let Some(f) = feed.as_mut() else {
            if n == 0 {
                return Err(err("the input produced no channels"));
            }
            continue;
        };
        for packet in &packets {
            f.packet(packet);
        }
        if n == 0 {
            break;
        }
    }
    let mut f = feed.ok_or_else(|| err("the input produced no channels"))?;
    if !f.ended {
        f.header(0);
        f.sink.on_end(EndReason::Error("the file ended unexpectedly".into()));
    }
    if id == "vcd" {
        strip_common_scope(&mut f.channels);
    }
    Ok(ImportedCapture {
        channels: f.channels.into_iter().map(|(_, c)| c).collect(),
        sample_rate: f.rate,
        format: module.name.to_owned(),
    })
}

/// VCD files usually put every signal in one scope, and oxrok prefixes the
/// scope (`capture.D5`). When every channel shares a scope path, drop it:
/// the prefix is noise. Files with several scopes keep their paths.
fn strip_common_scope(channels: &mut [(usize, ChannelInfo)]) {
    if channels.len() < 2 {
        return;
    }
    let stripped = {
        let names: Vec<&str> = channels.iter().map(|(_, c)| c.device_name.as_str()).collect();
        let first = names[0];
        let parts: Vec<&str> = first.split('.').collect();
        let prefix = (1..parts.len()).rev().map(|n| parts[..n].join(".")).find(|p| {
            !p.is_empty() && names.iter().all(|n| n.strip_prefix(p.as_str()).is_some_and(|r| r.starts_with('.')))
        });
        prefix.and_then(|p| {
            let stripped: Vec<String> = names.iter().map(|n| n[p.len() + 1..].to_owned()).collect();
            // Never create two channels with one name.
            let unique: std::collections::HashSet<&str> = stripped.iter().map(String::as_str).collect();
            (unique.len() == stripped.len()).then_some(stripped)
        })
    };
    let Some(stripped) = stripped else { return };
    for ((_, c), name) in channels.iter_mut().zip(stripped) {
        c.device_name = name;
    }
}

pub fn import(job: ImportJob) -> Result<ImportedCapture, BackendError> {
    match &job.format {
        ImportFormat::Session => import_session(&job.path, job.sink, job.progress),
        ImportFormat::Module(id) => import_module(&job.path, id, &job.options, job.sink, job.progress),
    }
}

/// A sigrok session from `source`.
pub fn save_sr(path: &Path, source: &dyn ExportSource, mut progress: ProgressFn) -> Result<(), BackendError> {
    let digital = source.digital();
    let analog = source.analog();
    let (begin, end) = source.range();
    if end <= begin {
        return Err(err("nothing to export"));
    }
    let rate = source.sample_rate();
    let mut channels: Vec<Channel> =
        digital.iter().enumerate().map(|(i, (_, name))| Channel::new(i, ChannelKind::Logic, name.clone())).collect();
    channels.extend(
        analog
            .iter()
            .enumerate()
            .map(|(i, (_, name))| Channel::new(digital.len() + i, ChannelKind::Analog, name.clone())),
    );
    let mut writer = SessionWriter::create(path, &channels, Some(rate)).map_err(err)?;
    let total = (end - begin) as f64 * (1 + analog.len()) as f64;
    let mut done = 0.0;
    if !digital.is_empty() {
        let unit = digital.len().div_ceil(8).max(1);
        let mut bytes = Vec::new();
        let mut at = begin;
        while at < end {
            let n = ((end - at) as usize).min(EXPORT_CHUNK);
            bytes.clear();
            source.logic(at, n, unit, &mut bytes);
            writer.receive(&Packet::logic(unit, bytes.clone())).map_err(err)?;
            at += n as u64;
            done += n as f64;
            progress((done / total) as f32);
        }
    }
    for (i, (id, _)) in analog.iter().enumerate() {
        let mut at = begin;
        while at < end {
            let n = ((end - at) as usize).min(EXPORT_CHUNK);
            let mut samples = Vec::with_capacity(n);
            source.analog_samples(*id, at, n, &mut samples);
            writer
                .receive(&Packet::Analog(AnalogData {
                    channels: vec![digital.len() + i],
                    samples,
                    mq: oxrok::core::Mq::Voltage,
                    flags: oxrok::core::MqFlags::DC,
                    unit: oxrok::core::Unit::Volt,
                    digits: 2,
                }))
                .map_err(err)?;
            at += n as u64;
            done += n as f64;
            progress((done / total) as f32);
        }
    }
    writer.receive(&Packet::End).map_err(err)?;
    writer.finish().map_err(err)?;
    progress(1.0);
    Ok(())
}

/// The `OptionValue` text form, as `-I option=value` takes it.
trait Cli {
    fn cli(&self) -> String;
}

impl Cli for OptionValue {
    fn cli(&self) -> String {
        match self {
            OptionValue::Int(i) => i.to_string(),
            OptionValue::Float(f) => f.to_string(),
            OptionValue::Str(s) => s.clone(),
        }
    }
}
