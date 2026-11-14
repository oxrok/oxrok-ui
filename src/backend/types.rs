//! Data types of the backend contract.

pub use crate::core::device::{ConfValue, DeviceOptions, OptionScope, ThresholdChoice};
use crate::core::{ChannelId, WallClock};
use serde::{Deserialize, Serialize};

/// Device key: `driver/serial`, or `driver/connection_id` without a serial.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceId(pub String);

impl DeviceId {
    pub fn new(driver: &str, serial_or_conn: &str) -> Self {
        Self(format!("{driver}/{serial_or_conn}"))
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub id: ChannelId,
    /// Backend channel name, e.g. `D3` or `CH1` (shown in Device Information).
    pub device_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub driver: String,
    pub vendor: String,
    pub model: String,
    pub version: String,
    pub serial_or_conn: String,
    /// libsigrok connection id (e.g. `3.12`), when the device has one
    /// (`-d driver:conn=…`).
    #[serde(default)]
    pub conn: Option<String>,
    pub is_demo: bool,
    pub channels: Vec<ChannelInfo>,
}

impl DeviceInfo {
    /// `vendor model`, skipping empty parts.
    pub fn display_name(&self) -> String {
        [self.vendor.as_str(), self.model.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn digital_channels(&self) -> impl Iterator<Item = &ChannelInfo> {
        self.channels.iter().filter(|c| c.id.kind == crate::core::ChannelKind::Digital)
    }

    pub fn analog_channels(&self) -> impl Iterator<Item = &ChannelInfo> {
        self.channels.iter().filter(|c| c.id.kind == crate::core::ChannelKind::Analog)
    }
}

/// A USB serial port that, by its USB ids, may hold a device its driver
/// finds only when told the port. Nothing was sent to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialCandidate {
    pub driver: String,
    /// The port's path, the `conn` to scan.
    pub port: String,
    /// The USB serial number, which names the board whatever port it is on.
    pub usb_serial: Option<String>,
    /// What the ids say it is, e.g. "Raspberry Pi Pico".
    pub label: String,
    /// The firmware the driver talks to.
    pub firmware: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rates {
    List(Vec<u64>),
    Range { min: u64, max: u64, step: u64 },
}

/// Whether digital and analog share one sample rate.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RateCoupling {
    Shared,
    Independent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdOption {
    /// Backend value (e.g. a libsigrok threshold string).
    pub id: String,
    pub label: String,
}

/// A numeric range with a step, in the key's unit.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct NumRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl NumRange {
    /// Clamped to the range and snapped to the step.
    pub fn snap(&self, v: f64) -> f64 {
        let v = v.clamp(self.min, self.max);
        if self.step > 0.0 {
            let n = ((v - self.min) / self.step).round();
            (self.min + n * self.step).clamp(self.min, self.max)
        } else {
            v
        }
    }
}

/// The logic thresholds a device offers.
#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdCaps {
    pub options: Vec<ThresholdOption>,
    /// A custom voltage, in volts (*Custom…*).
    pub custom: Option<NumRange>,
}

/// The unit a key's values are shown in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Unit {
    Seconds,
    Volts,
    Amps,
    Hertz,
    /// A probe factor, "10×".
    Factor,
}

impl Unit {
    pub fn symbol(self) -> &'static str {
        match self {
            Unit::Seconds => "s",
            Unit::Volts => "V",
            Unit::Amps => "A",
            Unit::Hertz => "Hz",
            Unit::Factor => "×",
        }
    }

    /// By libsigrok key id.
    pub fn for_key(id: &str) -> Option<Unit> {
        let unit = match id {
            "timebase" | "sample_interval" | "gate_time" => Unit::Seconds,
            "vdiv" | "amplitude" | "offset" | "triggerlevel" | "trigger_level" => Unit::Volts,
            "output_frequency" | "output_frequency_target" | "center_frequency" => Unit::Hertz,
            "probe_factor" => Unit::Factor,
            _ if id.starts_with("voltage") || id.ends_with("protection_threshold") => Unit::Volts,
            _ if id.starts_with("current") => Unit::Amps,
            _ => return None,
        };
        Some(unit)
    }
}

/// A value with its key's unit ("2 ms", "500 mV", "1/3 s", "10×").
pub fn format_value(v: &ConfValue, unit: Option<Unit>) -> String {
    use crate::core::format::si;
    match (v, unit) {
        (ConfValue::Bool(b), _) => (if *b { "On" } else { "Off" }).into(),
        (ConfValue::Str(s), _) => s.clone(),
        (_, Some(Unit::Factor)) => format!("{}×", v.as_f64().map_or_else(|| v.cli(), trim_num)),
        (ConfValue::Rational(p, q), Some(u)) => {
            let f = *p as f64 / (*q).max(1) as f64;
            // Exact in decimal only when q has no factors but 2 and 5.
            let mut d = *q;
            while d > 1 && d % 2 == 0 {
                d /= 2;
            }
            while d > 1 && d % 5 == 0 {
                d /= 5;
            }
            if d == 1 { si(f, u.symbol(), None) } else { format!("{p}/{q} {}", u.symbol()) }
        }
        (ConfValue::Pair(a, b), Some(u)) if a == b => si(*a, u.symbol(), None),
        (ConfValue::Pair(a, b), u) => {
            format!("{}–{}{}", trim_num(*a), trim_num(*b), u.map_or(String::new(), |u| format!(" {}", u.symbol())))
        }
        (ConfValue::UPair(a, b), _) => format!("{a}–{b}"),
        (_, Some(u)) => v.as_f64().map_or_else(|| v.cli(), |f| si(f, u.symbol(), None)),
        (ConfValue::Float(f), None) => trim_num(*f),
        (ConfValue::Rational(p, q), None) => format!("{p}/{q}"),
        (_, None) => v.cli(),
    }
}

fn trim_num(f: f64) -> String {
    let s = format!("{f:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// How a setting is edited.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionKind {
    Bool,
    Choice(Vec<ConfValue>),
    Range(NumRange),
    Free,
}

/// One driver setting.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceOption {
    /// libsigrok key id, e.g. `external_clock` (also sigrok-cli's name).
    pub id: String,
    /// Display name, e.g. "External clock".
    pub name: String,
    pub description: String,
    pub kind: OptionKind,
    pub unit: Option<Unit>,
    /// The current value, if the driver reports one.
    pub value: Option<ConfValue>,
    /// The value when the device was first seen (*Reset to defaults*).
    pub default: Option<ConfValue>,
    pub writable: bool,
}

/// The settings of one scope.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionGroup {
    pub scope: OptionScope,
    pub options: Vec<DeviceOption>,
}

/// A device option the driver refused, with the value it kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    pub scope: OptionScope,
    pub id: String,
    pub requested: ConfValue,
    pub kept: Option<ConfValue>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TriggerMatch {
    Zero,
    One,
    Rising,
    Falling,
    Edge,
}

/// What the device's own trigger takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct HwTriggerCaps {
    pub matches: Vec<TriggerMatch>,
    pub capture_ratio: bool,
    /// An edge can carry the width of the pulse it ends.
    pub pulse: bool,
    /// The most stages, 0 for any number.
    pub max_stages: usize,
    /// A stage can fire on its Nth match.
    pub count: bool,
    /// The count can be of matches in a row.
    pub consecutive: bool,
    /// A stage of levels can have to hold for a while.
    pub duration: bool,
    /// A stage can compare a group of channels with a value range.
    pub range: bool,
    /// Two stages can combine with or.
    pub or: bool,
    /// Two stages can combine with and.
    pub and: bool,
    /// The trigger can be armed only after a number of samples.
    pub holdoff: bool,
}

impl HwTriggerCaps {
    /// The stages the UI offers: A, and B when the device has it.
    pub fn stages(&self) -> usize {
        if self.max_stages == 1 { 1 } else { 2 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCaps {
    pub samplerates: Rates,
    pub rates_coupled: RateCoupling,
    pub max_limit_samples: Option<u64>,
    /// The device can stream; buffered devices can't.
    pub supports_continuous: bool,
    pub supports_limit_msec: bool,
    /// Run-length compression can be switched on.
    pub rle: bool,
    /// `None` when the device has no selectable threshold.
    pub thresholds: Option<ThresholdCaps>,
    /// The threshold when the device opened.
    pub threshold: Option<ThresholdChoice>,
    /// The generic settings, device scope first.
    pub options: Vec<OptionGroup>,
    pub hw_trigger: Option<HwTriggerCaps>,
    /// Per analog channel, its input range in volts if known.
    pub analog_ranges: Vec<(ChannelId, Option<(f64, f64)>)>,
    /// The live pin state can be read (Wire Status).
    pub supports_state_read: bool,
}

/// Configuration requested for the next capture.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceConfig {
    pub enabled: Vec<ChannelId>,
    pub sample_rate: u64,
    pub threshold: Option<ThresholdChoice>,
    /// The value of every writable option, defaults included, so
    /// a reset takes effect on a driver that keeps its settings.
    pub options: DeviceOptions,
}

impl DeviceConfig {
    pub fn new(enabled: Vec<ChannelId>, sample_rate: u64) -> Self {
        Self { enabled, sample_rate, threshold: None, options: DeviceOptions::new() }
    }
}

/// What the device actually accepted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppliedConfig {
    pub enabled: Vec<ChannelId>,
    pub sample_rate: u64,
    pub threshold: Option<ThresholdChoice>,
    /// Every option read back after applying.
    pub options: DeviceOptions,
    pub rejected: Vec<Rejected>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AcquisitionLimit {
    /// Stream until stopped.
    Continuous,
    Samples(u64),
    Millis(u64),
}

/// How long a stage must hold, in samples, as oxrok's `TriggerDuration`:
/// on a stage with one rising or falling edge the width of the pulse
/// that edge ends, on a stage of levels (`Longer` only) how long they
/// held.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HwDuration {
    /// More than this many samples.
    Longer(u64),
    /// Fewer than this many samples.
    Shorter(u64),
    /// Both ends inclusive.
    Between(u64, u64),
}

impl HwDuration {
    /// The widths in samples that qualify, both ends inclusive.
    pub fn bounds(self) -> (u64, u64) {
        match self {
            HwDuration::Longer(n) => (n.saturating_add(1), u64::MAX),
            HwDuration::Shorter(n) => (1, n.saturating_sub(1)),
            HwDuration::Between(min, max) => (min, max),
        }
    }
}

/// A group of channels read as a number, least significant first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HwRange {
    pub channels: Vec<ChannelId>,
    /// Both bounds inclusive.
    pub low: u64,
    pub high: u64,
    pub inside: bool,
}

/// One stage of the device's trigger: all matches must hold on the
/// same sample.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HwStage {
    pub matches: Vec<(ChannelId, TriggerMatch)>,
    /// Fire on the Nth match; `true` for matches in a row only.
    pub count: Option<(u32, bool)>,
    pub duration: Option<HwDuration>,
    pub range: Option<HwRange>,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum HwCombine {
    /// Each stage after the previous one.
    #[default]
    Sequence,
    Or,
    And,
}

/// The device's trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HwTrigger {
    pub stages: Vec<HwStage>,
    /// How two stages combine.
    pub combine: HwCombine,
    /// Share of the capture before the trigger, 0–100 % (`SR_CONF_CAPTURE_RATIO`).
    pub pre_trigger_percent: u8,
    /// Samples the device takes before it arms the trigger, so that the
    /// share before the trigger is there. Needs [`HwTriggerCaps::holdoff`].
    pub holdoff: u64,
}

impl HwTrigger {
    /// One stage of level and edge matches.
    pub fn one_stage(matches: Vec<(ChannelId, TriggerMatch)>, pre_trigger_percent: u8) -> Self {
        Self {
            stages: vec![HwStage { matches, ..HwStage::default() }],
            combine: HwCombine::Sequence,
            pre_trigger_percent,
            holdoff: 0,
        }
    }

    /// The edge of the first stage must end a pulse `min` to `max`
    /// samples wide, both ends inclusive. Needs [`HwTriggerCaps::pulse`].
    #[must_use]
    pub fn pulse(mut self, min: u64, max: u64) -> Self {
        if let Some(s) = self.stages.first_mut() {
            s.duration = Some(HwDuration::Between(min, max));
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcquisitionPlan {
    pub limit: AcquisitionLimit,
    /// Arm the device's trigger; devices without one fail to start.
    pub trigger: Option<HwTrigger>,
    /// Device-side run-length compression where supported: buffered
    /// analyzers then cover longer stretches of slow signals.
    pub rle: bool,
}

impl AcquisitionPlan {
    pub fn new(limit: AcquisitionLimit) -> Self {
        Self { limit, trigger: None, rle: false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamHeader {
    pub start: WallClock,
    pub sample_rate: u64,
    /// Bytes per logic unit; bit `i` is digital channel index `i`.
    pub logic_unit_size: usize,
    /// Enabled digital channels.
    pub logic_channels: Vec<ChannelId>,
    /// Enabled analog channels.
    pub analog_channels: Vec<ChannelId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndReason {
    Completed,
    Stopped,
    Error(String),
    Disconnected,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    #[error("device not found: {0}")]
    NotFound(DeviceId),
    #[error("device is busy")]
    Busy,
    #[error("not supported: {0}")]
    Unsupported(String),
    #[error("device error: {0}")]
    Device(String),
    #[error("I/O error: {0}")]
    Io(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecChannel {
    pub id: String,
    pub name: String,
    pub desc: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OptionValue {
    Int(i64),
    Float(f64),
    Str(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecOption {
    pub id: String,
    pub desc: String,
    pub default: OptionValue,
    pub values: Vec<OptionValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnRow {
    pub id: String,
    pub desc: String,
    pub classes: Vec<usize>,
}

/// Decoder metadata. Populated from 0.5.0 on.
#[derive(Clone, Debug, PartialEq)]
pub struct DecoderInfo {
    pub id: String,
    pub name: String,
    pub longname: String,
    pub desc: String,
    pub license: String,
    pub tags: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub channels: Vec<DecChannel>,
    pub opt_channels: Vec<DecChannel>,
    pub options: Vec<DecOption>,
    pub annotations: Vec<(String, String)>,
    pub annotation_rows: Vec<AnnRow>,
    pub binary: Vec<(String, String)>,
    pub doc_url: Option<String>,
}

/// What an import reads besides the samples: the file's
/// channels, named, and its sample rate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedCapture {
    pub channels: Vec<ChannelInfo>,
    pub sample_rate: u64,
    /// The format's display name, e.g. "Value Change Dump".
    pub format: String,
}

/// How to read a file.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportFormat {
    /// A sigrok session (`.sr`).
    Session,
    /// A libsigrok input module by id.
    Module(String),
}

/// The kind of an input option's value.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InputOptionKind {
    Int,
    Float,
    Str,
    /// Stored as `Int(0)` / `Int(1)`.
    Bool,
}

/// One option of a libsigrok input module (`sr_input_options_get`).
#[derive(Clone, Debug, PartialEq)]
pub struct InputOption {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: InputOptionKind,
    pub default: OptionValue,
    /// The allowed values, when the module lists them.
    pub values: Vec<OptionValue>,
}

impl InputOption {
    /// An option with no usable default (a sample rate of 0).
    pub fn required(&self) -> bool {
        self.id == "samplerate" && matches!(self.default, OptionValue::Int(0))
    }
}

/// A libsigrok input format.
#[derive(Clone, Debug, PartialEq)]
pub struct InputFormat {
    pub id: String,
    pub name: String,
    pub description: String,
    pub extensions: Vec<String>,
    pub options: Vec<InputOption>,
}

impl InputFormat {
    /// The Import Options dialog opens before reading.
    pub fn needs_options(&self) -> bool {
        self.options.iter().any(InputOption::required)
    }
}

/// Read access to a capture for sigrok `.sr` export, over a
/// sample range the source chooses.
pub trait ExportSource: Sync {
    fn sample_rate(&self) -> u64;
    /// Samples `[begin, end)` to export.
    fn range(&self) -> (u64, u64);
    /// Digital channels in export order, with their names.
    fn digital(&self) -> Vec<(ChannelId, String)>;
    fn analog(&self) -> Vec<(ChannelId, String)>;
    /// Packs `n` samples from `first` as logic units: bit `k` of each unit
    /// is `digital()[k]`. `out` gets `n * unit_size` bytes.
    fn logic(&self, first: u64, n: usize, unit_size: usize, out: &mut Vec<u8>);
    /// `n` analog samples of `id` from `first`, in volts.
    fn analog_samples(&self, id: ChannelId, first: u64, n: usize, out: &mut Vec<f32>);
}

/// Progress from 0 to 1, called from the backend's thread.
pub type ProgressFn = Box<dyn FnMut(f32) + Send>;

/// A firmware file a driver failed to load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingFirmware {
    /// Empty when the file was asked for outside a scan or an open.
    pub driver: String,
    pub file: String,
}

/// The missing files, the firmware search paths with whether each
/// exists, and the folder installs go to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FirmwareStatus {
    pub missing: Vec<MissingFirmware>,
    pub paths: Vec<(std::path::PathBuf, bool)>,
    pub install_dir: Option<std::path::PathBuf>,
}
