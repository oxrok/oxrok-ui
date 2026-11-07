//! The oxrok decoder engine for `la-decode`.
//!
//! oxrok's native decoders (`uart`, `i2c`, `spi`) come first in the
//! registry, so they win over the libsigrokdecode Python decoders of the
//! same id. With the `rustpython` feature the Python host is built into
//! the binary.

use crate::decode::{
    AnnRow, DecChannel, DecOption, DecodeSession, DecodeSpec, DecoderEngine, DecoderInfo, OptionValue, RawAnn,
    RawBinary, attach_binary,
};
use oxrok::decode::{Config, DecoderDef, Output, Registry, Stack};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// The oxrok decoder registry, plus the metadata in `la-decode`'s shape.
pub struct OxrokDecoders {
    registry: Registry,
    infos: Vec<DecoderInfo>,
}

impl OxrokDecoders {
    /// Loads the native decoders and, when a feature provides one, the
    /// Python host. `extra` replaces the default decoders directory, as
    /// `libsigrokdecode`'s `srd_init(dir)` did.
    pub fn new(extra: Option<&std::path::Path>) -> Result<Self, String> {
        if let Some(dir) = extra {
            // SAFETY: the app sets this before the decoder thread reads it;
            // no other thread is in `oxrok::pydecode`.
            unsafe { std::env::set_var("OXROK_DECODERS_DIR", dir) };
        }
        let mut registry = Registry::new();
        registry.register(&oxrok::decoders::ALL);
        registry.register(&oxrok::pydecode::defs());
        let mut infos: Vec<DecoderInfo> = registry.decoders().map(|d| info(d.info)).collect();
        infos.sort_by_key(|d| d.longname.to_lowercase());
        Ok(Self { registry, infos })
    }
}

/// Metadata of one decoder.
fn info(d: &'static oxrok::decode::DecoderInfo) -> DecoderInfo {
    let channel = |c: &oxrok::decode::info::ChannelDesc| DecChannel {
        id: c.id.to_owned(),
        name: c.name.to_owned(),
        desc: c.desc.to_owned(),
    };
    DecoderInfo {
        id: d.id.to_owned(),
        name: d.name.to_owned(),
        longname: d.longname.to_owned(),
        desc: d.desc.to_owned(),
        license: d.license.to_owned(),
        tags: d.tags.iter().map(|t| (*t).to_owned()).collect(),
        inputs: d.inputs.iter().map(|s| (*s).to_owned()).collect(),
        outputs: d.outputs.iter().map(|s| (*s).to_owned()).collect(),
        channels: d.channels.iter().map(channel).collect(),
        opt_channels: d.optional_channels.iter().map(channel).collect(),
        options: d
            .options
            .iter()
            .map(|o| DecOption {
                id: o.id.to_owned(),
                desc: o.desc.to_owned(),
                default: value(&o.default),
                values: o.values.iter().map(value).collect(),
            })
            .collect(),
        annotations: d.annotations.iter().map(|a| (a.id.to_owned(), a.desc.to_owned())).collect(),
        annotation_rows: d
            .annotation_rows
            .iter()
            .map(|r| AnnRow { id: r.id.to_owned(), desc: r.desc.to_owned(), classes: r.classes.to_vec() })
            .collect(),
        binary: d.binary.iter().map(|b| (b.id.to_owned(), b.desc.to_owned())).collect(),
        doc_url: Some(format!("https://sigrok.org/wiki/Protocol_decoder:{}", d.id.replace('_', ""))),
    }
}

fn value(v: &oxrok::decode::info::OptionValue) -> OptionValue {
    match v {
        oxrok::decode::info::OptionValue::Int(i) => OptionValue::Int(*i),
        oxrok::decode::info::OptionValue::Float(f) => OptionValue::Float(*f),
        oxrok::decode::info::OptionValue::Str(s) => OptionValue::Str(s.to_string()),
    }
}

fn ox_value(v: &OptionValue) -> oxrok::decode::info::OptionValue {
    match v {
        OptionValue::Int(i) => oxrok::decode::info::OptionValue::Int(*i),
        OptionValue::Float(f) => oxrok::decode::info::OptionValue::Float(*f),
        OptionValue::Str(s) => oxrok::decode::info::OptionValue::Str(std::borrow::Cow::Owned(s.clone())),
    }
}

impl DecoderEngine for OxrokDecoders {
    fn decoders(&self) -> &[DecoderInfo] {
        &self.infos
    }

    fn start(&mut self, spec: &DecodeSpec) -> Result<Box<dyn DecodeSession>, String> {
        let def: &'static DecoderDef =
            self.registry.find(&spec.decoder).ok_or_else(|| format!("no decoder named {}", spec.decoder))?;
        let info = self
            .infos
            .iter()
            .find(|d| d.id == spec.decoder)
            .cloned()
            .ok_or_else(|| format!("no decoder named {}", spec.decoder))?;
        let bin_targets = (0..info.binary.len()).map(|b| crate::decode::binary_target(&info, b)).collect();
        Ok(Box::new(Session {
            def,
            info,
            bin_targets,
            samplerate: spec.samplerate,
            options: spec.options.iter().map(|(k, v)| (k.clone(), ox_value(v))).collect(),
            channels: spec.channels.clone(),
            stack: None,
            rx: None,
            pending: Vec::new(),
            ended: false,
            error: None,
        }))
    }
}

/// A running decoder instance, created on the first `send` so the
/// unit size is the caller's.
struct Session {
    def: &'static DecoderDef,
    info: DecoderInfo,
    /// Binary class → annotation class, computed once.
    bin_targets: Vec<Option<usize>>,
    samplerate: u64,
    options: HashMap<String, oxrok::decode::info::OptionValue>,
    channels: std::collections::BTreeMap<String, usize>,
    stack: Option<Stack>,
    rx: Option<Receiver<Output>>,
    /// Binary items still waiting for their annotation.
    pending: Vec<RawBinary>,
    ended: bool,
    error: Option<String>,
}

impl Session {
    /// Builds the decoder configuration: required channels, then optional
    /// ones, each at its bit in the packed unit.
    fn config(&self) -> Config {
        let ids = self.info.channels.iter().chain(&self.info.opt_channels).map(|c| c.id.as_str());
        let channels = ids.map(|id| self.channels.get(id).copied()).collect();
        Config { options: self.options.clone(), channels }
    }

    fn start_stack(&mut self, unit_size: usize) -> Result<(), String> {
        if self.stack.is_some() {
            return Ok(());
        }
        let mut stack = Stack::new();
        let instance = format!("{}-1", self.def.info.id);
        stack.add(instance, (self.def.new)(), &self.config(), unit_size, self.samplerate).map_err(|e| e.to_string())?;
        self.rx = stack.outputs();
        self.stack = Some(stack);
        Ok(())
    }

    /// Takes everything the decoders emitted so far.
    fn drain(&mut self, wait: bool) -> Vec<RawAnn> {
        let mut anns = Vec::new();
        let Some(rx) = &self.rx else { return anns };
        loop {
            let out = if wait {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(o) => o,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        self.ended = true;
                        break;
                    }
                }
            } else {
                match rx.try_recv() {
                    Ok(o) => o,
                    Err(_) => break,
                }
            };
            match out {
                Output::Annotation { ss, es, class, texts, .. } => {
                    anns.push(RawAnn { start: ss, end: es, class, texts, value: None });
                }
                Output::Binary { ss, es, class, data, .. } => {
                    self.pending.push(RawBinary { start: ss, end: es, class, bytes: data });
                }
                Output::End { error, .. } => {
                    self.ended = true;
                    if let Some(e) = error {
                        self.error = Some(e);
                    }
                }
                Output::Python { .. } | Output::Meta { .. } => {}
            }
        }
        attach_binary(&self.info, &mut anns, &self.pending);
        // Consumed items go; the rest may pair with a later annotation.
        let pending = std::mem::take(&mut self.pending);
        let seen: std::collections::HashSet<(u64, usize)> = anns.iter().map(|a| (a.start, a.class)).collect();
        self.pending = pending
            .into_iter()
            .filter(|b| match self.bin_targets.get(b.class).copied().flatten() {
                Some(class) => !seen.contains(&(b.start, class)),
                None => false,
            })
            .collect();
        anns
    }
}

impl DecodeSession for Session {
    fn send(&mut self, _start: u64, data: &[u8], unit_size: usize) -> Result<Vec<RawAnn>, String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        self.start_stack(unit_size)?;
        if let Some(stack) = &mut self.stack {
            stack.feed(data);
        }
        Ok(self.drain(false))
    }

    fn finish(&mut self) -> Result<Vec<RawAnn>, String> {
        if let Some(stack) = &mut self.stack {
            stack.end();
        }
        let mut anns = Vec::new();
        // Wait for the decoders to reach their end, or give up on a stuck one.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !self.ended && self.rx.is_some() && std::time::Instant::now() < deadline {
            anns.extend(self.drain(true));
        }
        anns.extend(self.drain(false));
        self.stack = None;
        self.rx = None;
        match self.error.take() {
            Some(e) => Err(e),
            None => Ok(anns),
        }
    }
}
