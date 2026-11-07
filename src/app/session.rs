//! One session: a device binding plus everything recorded with it.

use crate::app::capture::SharedCapture;
use crate::app::worker::SessionId;
use crate::backend::{
    ConfValue, DeviceCaps, DeviceConfig, DeviceInfo, DeviceOption, DeviceOptions, OptionScope, Rates, ThresholdChoice,
};
use crate::core::analog::{AnalogView, DEFAULT_INPUT_RANGE};
use crate::core::animator::ViewAnimator;
use crate::core::bus::{Bus, BusFormat, Endianness};
use crate::core::capture::{CaptureSettings, CaptureSnapshot, CaptureState, default_sample_rate, rates_from_range};
use crate::core::nav::DataBounds;
use crate::core::{CaptureTime, ChannelId, ChannelKind, ViewState};

/// Default and minimum row heights.
pub const DIGITAL_ROW_HEIGHT: f32 = 25.0;
pub const DIGITAL_ROW_MIN: f32 = 20.0;
pub const ANALOG_ROW_HEIGHT: f32 = 100.0;
pub const ANALOG_ROW_MIN: f32 = 45.0;
/// Resizes snap to multiples of this.
pub const ROW_SNAP: f32 = 5.0;
/// Visible duration before the first capture *(design choice)*.
pub const DEFAULT_TIME_SCALE: f64 = 1.0;
/// Channels enabled per type on a new device.
const DEFAULT_ENABLED_PER_TYPE: usize = 2;

/// A Repeat run.
#[derive(Clone, Debug, Default)]
pub struct RepeatRun {
    /// 1 for the first capture.
    pub round: u32,
    /// The round ended by itself, so the next one starts.
    pub(crate) auto_end: bool,
    /// The user stopped: the run ends.
    pub(crate) user_stop: bool,
    /// The recording round's done flag, until it replaces the previous one.
    pub(crate) done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The last complete round's time range: the view limits keep it while
    /// the next round fills in.
    pub(crate) span: Option<(CaptureTime, CaptureTime)>,
}

/// What the screen shows while the next round records.
#[derive(Clone, Default)]
pub(crate) struct PrevRound {
    pub capture: Option<SharedCapture>,
    pub trigger: Option<CaptureTime>,
    pub snapshot: Option<CaptureSnapshot>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Row {
    pub channel: ChannelId,
    /// Backend channel name, the identifier, e.g. `D3`, `CLK1`.
    pub ident: String,
    pub name: String,
    pub height: f32,
    /// Analog rows: the vertical view, set on first layout.
    pub analog: Option<AnalogView>,
    /// Analog rows: the inner height `analog` was laid out for.
    pub analog_inner: Option<f32>,
    /// Hidden by the user; keeps all its settings.
    pub hidden: bool,
    /// Step 1: a color the user set on this row.
    pub color: Option<[u8; 3]>,
}

impl Row {
    /// Identifier, e.g. `D3` or `CLK1`.
    pub fn identifier(&self) -> String {
        self.ident.clone()
    }

    pub fn min_height(&self) -> f32 {
        match self.channel.kind {
            ChannelKind::Digital | ChannelKind::Bus => DIGITAL_ROW_MIN,
            ChannelKind::Analog => ANALOG_ROW_MIN,
        }
    }

    /// Release: snap to 5 pt, not below the minimum.
    pub fn snapped(&self, h: f32) -> f32 {
        ((h / ROW_SNAP).round() * ROW_SNAP).max(self.min_height())
    }

    /// The label text: the identifier, plus the name when it differs.
    pub fn display(&self) -> String {
        if self.name == self.ident { self.ident.clone() } else { format!("{} {}", self.ident, self.name) }
    }
}

/// The live pin levels; stale once polling stopped.
#[derive(Clone, Debug, PartialEq)]
pub struct LivePins {
    pub levels: Vec<(ChannelId, bool)>,
    pub stale: bool,
}

impl LivePins {
    pub fn level(&self, c: ChannelId) -> Option<bool> {
        self.levels.iter().find(|l| l.0 == c).map(|l| l.1)
    }
}

pub struct Session {
    pub id: SessionId,
    pub name: String,
    pub device: DeviceInfo,
    /// Known once the worker has opened the device.
    pub caps: Option<DeviceCaps>,
    pub open_error: Option<String>,
    /// Settings for the next capture.
    pub settings: CaptureSettings,
    /// Settings latched at capture start.
    pub snapshot: Option<CaptureSnapshot>,
    pub state: CaptureState,
    pub capture: Option<SharedCapture>,
    /// One row per channel the device offers, digital first;
    /// plus one per signal group, its members right after it.
    pub rows: Vec<Row>,
    /// Signal groups; their rows are in `rows`.
    pub buses: Vec<Bus>,
    /// Wire Status: on, the last pins read, a read in flight and
    /// when the last one ended.
    pub wire_status: bool,
    pub pins: Option<LivePins>,
    pub pins_pending: bool,
    pub pins_read_at: Option<std::time::Duration>,
    /// Reads in a row that took longer than the interval; the interval
    /// doubles for each, up to a second, so a slow driver isn't polled
    /// back to back.
    pub pins_slow_reads: u32,
    pub view: ViewAnimator,
    /// follow-latest mode.
    pub follow: bool,
    /// The device's probe profile, if one matches.
    pub probe: Option<(crate::core::probe::ProbeProfile, crate::app::profiles::ProfileSource)>,
    /// View history.
    pub history: crate::core::history::ViewHistory,
    /// The last value typed into Jump to Time.
    pub jump_text: String,
    /// 10: timing markers and pairs.
    pub markers: crate::core::markers::Markers,
    /// Range measurements.
    pub ranges: crate::core::ranges::Ranges,
    /// undo/redo for range add and delete.
    pub range_history: crate::core::ranges::RangeHistory,
    /// Time origin.
    pub origin: crate::core::markers::TimeOrigin,
    /// 12: protocol analyzers, in panel order.
    pub analyzers: Vec<crate::app::analyzers::Analyzer>,
    /// Set when the current capture has ended (the decoders finish then).
    pub capture_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The trigger time of the current capture, once found.
    pub trigger: Option<CaptureTime>,
    /// The software trigger scanning the current capture.
    pub scanner: Option<crate::engine::trigger::TriggerScanner>,
    /// The `.oxrok` this session saves to, once it has one.
    pub file: Option<std::path::PathBuf>,
    /// Opened from a file (any format), so the samples are
    /// read-only; the file shown in the banner.
    pub opened_from: Option<std::path::PathBuf>,
    /// Capture notes.
    pub notes: String,
    /// The fingerprint of what was last saved or loaded; `None`
    /// until then.
    pub saved: Option<u64>,
    /// `file` holds the current samples, so Save only rewrites
    /// `metadata.json`. False after a new capture or a trim.
    pub samples_saved: bool,
    /// The Repeat run in progress.
    pub repeat: Option<RepeatRun>,
    /// What Record does on this opened file, chosen in the
    /// banner; `None` follows the preference (17).
    pub record_choice: Option<crate::app::settings::RecordOnOpened>,
    /// The banner closed after a choice (this session only).
    pub banner_closed: bool,
    /// The Analyzer Trigger, and its scan.
    pub analyzer_trigger: crate::app::analyzers::AnalyzerTrigger,
    pub trigger_scan: crate::app::analyzers::TriggerScan,
    /// The previous round, kept on screen until the recording
    /// round has data to show.
    pub(crate) previous: Option<PrevRound>,
    /// The previous round is swapped into `capture` for drawing.
    pub(crate) previous_shown: bool,
    /// The view state the UI keeps while this session is in the background.
    pub parked_ui: ParkedUi,
}

/// UI state that belongs to a session but lives in the app's UI
/// state while the session is active.
#[derive(Default)]
pub struct ParkedUi {
    pub rows_scroll: f32,
    pub(crate) table: crate::app::ui::datatable::TableCache,
    pub terminal_cleared: std::collections::HashMap<u32, usize>,
    pub export_included: Option<Vec<ChannelId>>,
}

impl Session {
    pub fn new(id: SessionId, device: DeviceInfo, saved: Option<CaptureSettings>, width_px: f64) -> Self {
        let rows = device
            .digital_channels()
            .chain(device.analog_channels())
            .map(|c| {
                let ident = if c.device_name.is_empty() { c.id.short_name() } else { c.device_name.clone() };
                let height = match c.id.kind {
                    ChannelKind::Digital | ChannelKind::Bus => DIGITAL_ROW_HEIGHT,
                    ChannelKind::Analog => ANALOG_ROW_HEIGHT,
                };
                Row {
                    channel: c.id,
                    name: ident.clone(),
                    ident,
                    height,
                    analog: None,
                    analog_inner: None,
                    hidden: false,
                    color: None,
                }
            })
            .collect();
        let settings = saved.unwrap_or_else(|| CaptureSettings {
            enabled: device
                .digital_channels()
                .take(DEFAULT_ENABLED_PER_TYPE)
                .chain(device.analog_channels().take(DEFAULT_ENABLED_PER_TYPE))
                .map(|c| c.id)
                .collect(),
            ..CaptureSettings::default()
        });
        let width = width_px.max(1.0);
        Self {
            id,
            name: format!("Session {id}"),
            device,
            caps: None,
            open_error: None,
            settings,
            snapshot: None,
            state: CaptureState::Empty,
            capture: None,
            rows,
            buses: Vec::new(),
            wire_status: false,
            pins: None,
            pins_pending: false,
            pins_read_at: None,
            pins_slow_reads: 0,
            view: ViewAnimator::new(ViewState::new(CaptureTime::ZERO, DEFAULT_TIME_SCALE / width, width)),
            follow: false,
            probe: None,
            history: Default::default(),
            jump_text: String::new(),
            markers: Default::default(),
            ranges: Default::default(),
            range_history: Default::default(),
            origin: Default::default(),
            analyzers: Vec::new(),
            capture_done: Default::default(),
            trigger: None,
            scanner: None,
            file: None,
            opened_from: None,
            notes: String::new(),
            saved: None,
            samples_saved: false,
            repeat: None,
            record_choice: None,
            banner_closed: false,
            analyzer_trigger: Default::default(),
            trigger_scan: Default::default(),
            previous: None,
            previous_shown: false,
            parked_ui: ParkedUi::default(),
        }
    }

    /// Takes `src`'s setup — capture settings, rows,
    /// analyzers (without results), the time origin type and the zoom. Not
    /// its data, markers, measurements or notes.
    pub fn copy_setup_from(&mut self, src: &Session) {
        self.settings = src.settings.clone();
        self.buses = src.buses.clone();
        self.apply_rows(src.rows.iter().cloned());
        self.analyzers = src
            .analyzers
            .iter()
            .map(|a| crate::app::analyzers::Analyzer { out: Default::default(), ..a.clone() })
            .collect();
        self.origin = src.origin.for_capture_start(src.settings.mode == crate::core::capture::CaptureMode::Triggered);
        let scale = src.view.state().time_scale();
        let width = self.view.state().width_px;
        self.view = ViewAnimator::new(ViewState::new(CaptureTime::ZERO, scale / width.max(1.0), width));
        self.view.speed_scale = src.view.speed_scale;
    }

    /// Takes `rows` (from a source session or a file)
    /// in their order, first; this device's channels they lack keep their
    /// default rows after them. Rows for channels it doesn't have are
    /// dropped.
    pub fn apply_rows(&mut self, rows: impl IntoIterator<Item = Row>) {
        let mut ordered: Vec<Row> = Vec::with_capacity(self.rows.len());
        for r in rows {
            if let Some(k) = self.rows.iter().position(|x| x.channel == r.channel) {
                self.rows.remove(k);
                ordered.push(r);
            } else if r.channel.is_bus() && self.bus(r.channel.index).is_some() {
                // A group's row comes with its setup.
                ordered.push(r);
            }
        }
        ordered.append(&mut self.rows);
        self.rows = ordered;
        self.normalize_buses();
    }

    /// A finished capture that was never saved, or whose saved
    /// annotations changed since.
    pub fn is_unsaved(&self) -> bool {
        self.state == CaptureState::Finished
            && self.has_data()
            && (self.saved != Some(crate::app::file::Annotations::of(self).fingerprint())
                // A trim changed samples that were saved.
                || (self.file.is_some() && !self.samples_saved))
    }

    /// The samples came from a file and can't be re-recorded.
    pub fn is_read_only(&self) -> bool {
        self.opened_from.is_some()
    }

    /// Display name, `[DEMO] ` prefixed for demo devices.
    pub fn device_display_name(&self) -> String {
        device_display_name(&self.device)
    }

    /// The rate menu, highest first.
    pub fn rates(&self) -> Vec<u64> {
        match self.caps.as_ref().map(|c| &c.samplerates) {
            Some(Rates::List(l)) => {
                let mut l = l.clone();
                l.sort_unstable_by(|a, b| b.cmp(a));
                l.dedup();
                l
            }
            Some(Rates::Range { min, max, step }) => rates_from_range(*min, *max, *step),
            None => Vec::new(),
        }
    }

    /// The device was opened: keep the saved rate if valid, else 's default.
    pub fn on_opened(&mut self, caps: DeviceCaps) {
        // Continuous needs a streaming device.
        if !caps.supports_continuous && self.settings.mode == crate::core::capture::CaptureMode::Continuous {
            self.settings.mode = crate::core::capture::CaptureMode::Timed;
        }
        self.caps = Some(caps);
        let rates = self.rates();
        if !rates.contains(&self.settings.sample_rate)
            && let Some(r) = default_sample_rate(&rates)
        {
            self.settings.sample_rate = r;
        }
        let offered: Vec<ChannelId> = self.rows.iter().map(|r| r.channel).collect();
        self.settings.enabled.retain(|c| offered.contains(c));
        // A remembered threshold the device still offers, else its current one.
        let caps = self.caps.as_ref().unwrap();
        let valid = match (&self.settings.logic_threshold, &caps.thresholds) {
            (Some(ThresholdChoice::Preset(id)), Some(t)) => t.options.iter().any(|o| &o.id == id),
            (Some(ThresholdChoice::Custom(_)), Some(t)) => t.custom.is_some(),
            _ => false,
        };
        if !valid {
            self.settings.logic_threshold = caps.threshold.clone();
        }
        // Forget options the device doesn't have.
        let known = |scope: &OptionScope, id: &str| {
            caps.options.iter().any(|g| &g.scope == scope && g.options.iter().any(|o| o.id == id && o.writable))
        };
        for (scope, values) in self.settings.device_options.iter_mut() {
            values.retain(|id, _| known(scope, id));
        }
        self.settings.device_options.retain(|_, v| !v.is_empty());
    }

    /// An option's value for the next capture.
    pub fn option_value(&self, scope: &OptionScope, o: &DeviceOption) -> Option<ConfValue> {
        self.settings.device_options.get(scope).and_then(|m| m.get(&o.id)).cloned().or_else(|| o.default.clone())
    }

    /// Stores `v` when it differs from the default.
    pub fn set_option(&mut self, scope: &OptionScope, o: &DeviceOption, v: ConfValue) {
        let map = self.settings.device_options.entry(scope.clone()).or_default();
        if o.default.as_ref() == Some(&v) {
            map.remove(&o.id);
        } else {
            map.insert(o.id.clone(), v);
        }
        self.settings.device_options.retain(|_, v| !v.is_empty());
    }

    /// Header count: options that differ from the defaults.
    pub fn changed_options(&self) -> usize {
        self.settings.device_options.values().map(|m| m.len()).sum()
    }

    /// What to apply, with every writable option's value
    /// so a reset reaches drivers that keep their settings.
    pub fn device_config(&self, enabled: Vec<ChannelId>) -> DeviceConfig {
        let mut cfg = DeviceConfig::new(enabled, self.settings.sample_rate);
        cfg.threshold = self.settings.logic_threshold.clone();
        if let Some(caps) = &self.caps {
            for g in &caps.options {
                for o in g.options.iter().filter(|o| o.writable) {
                    if let Some(v) = self.option_value(&g.scope, o) {
                        cfg.options.entry(g.scope.clone()).or_default().insert(o.id.clone(), v);
                    }
                }
            }
        }
        cfg
    }

    /// read-back values that differ from the defaults.
    pub fn non_default(&self, read: &DeviceOptions) -> DeviceOptions {
        let mut out = DeviceOptions::new();
        let Some(caps) = &self.caps else { return out };
        for g in &caps.options {
            let Some(values) = read.get(&g.scope) else { continue };
            for o in g.options.iter().filter(|o| o.writable) {
                if let Some(v) = values.get(&o.id).filter(|v| o.default.as_ref() != Some(*v)) {
                    out.entry(g.scope.clone()).or_default().insert(o.id.clone(), v.clone());
                }
            }
        }
        out
    }

    /// Visible iff the channel is enabled and the row isn't
    /// hidden. A group's members also need the group shown and expanded
    pub fn visible_rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| !r.hidden && self.row_enabled(r.channel) && self.group_open(r.channel))
    }

    /// Rows of enabled channels, hidden or not, in display order.
    pub fn enabled_rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| self.row_enabled(r.channel))
    }

    /// A channel's enable state; a group counts as enabled while any member is.
    pub fn row_enabled(&self, id: ChannelId) -> bool {
        match id.kind {
            ChannelKind::Bus => {
                self.bus(id.index).is_some_and(|b| b.members.iter().any(|m| self.settings.is_enabled(*m)))
            }
            _ => self.settings.is_enabled(id),
        }
    }

    /// False for a member of a collapsed or hidden group.
    fn group_open(&self, id: ChannelId) -> bool {
        self.bus_of(id).is_none_or(|b| !b.collapsed && self.row(b.row()).is_some_and(|r| !r.hidden))
    }

    /// Polling is possible now (device open and able,
    /// not read-only, not recording).
    pub fn can_read_pins(&self) -> bool {
        self.caps.as_ref().is_some_and(|c| c.supports_state_read) && !self.is_read_only() && !self.state.is_busy()
    }

    /// A group's live value, once every member has a level.
    pub fn live_bus_value(&self, index: u16) -> Option<String> {
        let (pins, bus) = (self.pins.as_ref()?, self.bus(index)?);
        if !bus.format.has_value() {
            return None;
        }
        let levels: Option<Vec<bool>> = self.wire_order(index).iter().map(|c| pins.level(*c)).collect();
        let levels = levels?;
        Some(bus.format(bus.raw(levels.iter().copied()), levels.len()))
    }

    // ---- signal groups ----

    pub fn bus(&self, index: u16) -> Option<&Bus> {
        self.buses.iter().find(|b| b.id == index)
    }

    pub fn bus_mut(&mut self, index: u16) -> Option<&mut Bus> {
        self.buses.iter_mut().find(|b| b.id == index)
    }

    /// The group `id` (a channel) belongs to.
    pub fn bus_of(&self, id: ChannelId) -> Option<&Bus> {
        self.buses.iter().find(|b| b.members.contains(&id))
    }

    /// A group's members in wire order, the order of their rows.
    pub fn wire_order(&self, index: u16) -> Vec<ChannelId> {
        let Some(b) = self.bus(index) else { return Vec::new() };
        self.rows.iter().map(|r| r.channel).filter(|c| b.members.contains(c)).collect()
    }

    /// `‹first id›–‹last id›` in wire order.
    pub fn bus_default_name(&self, index: u16) -> String {
        let order = self.wire_order(index);
        let ident = |c: Option<&ChannelId>| c.and_then(|c| self.row(*c)).map(|r| r.ident.clone()).unwrap_or_default();
        crate::core::bus::default_name(&ident(order.first()), &ident(order.last()))
    }

    /// A group of `members` (≥ 1, none in another group) where the
    /// first of them was, with them under it in the given (wire) order.
    /// Its row id.
    pub fn create_bus(
        &mut self,
        members: &[ChannelId],
        name: &str,
        format: BusFormat,
        endianness: Endianness,
        signed: bool,
    ) -> Option<ChannelId> {
        let mut members: Vec<ChannelId> = members
            .iter()
            .copied()
            .filter(|c| c.kind == ChannelKind::Digital && self.bus_of(*c).is_none() && self.row(*c).is_some())
            .collect();
        dedup_keep_order(&mut members);
        members.truncate(crate::core::bus::MAX_MEMBERS);
        if members.is_empty() {
            return None;
        }
        let id = (0..).find(|i| self.bus(*i).is_none())?;
        let at = self.rows.iter().position(|r| members.contains(&r.channel)).unwrap_or(self.rows.len());
        let row_id = ChannelId::bus(id);
        let ident = row_id.short_name();
        self.rows.insert(
            at,
            Row {
                channel: row_id,
                name: ident.clone(),
                ident,
                height: DIGITAL_ROW_HEIGHT,
                analog: None,
                analog_inner: None,
                hidden: false,
                color: None,
            },
        );
        self.buses.push(Bus { id, members: members.clone(), format, endianness, signed, collapsed: false });
        self.place_members(id, &members);
        let name = name.trim();
        let name = if name.is_empty() { self.bus_default_name(id) } else { name.to_owned() };
        if let Some(r) = self.row_mut(row_id) {
            r.name = name;
        }
        Some(row_id)
    }

    /// *Edit Group…*: new members in the given (wire) order, name
    /// and options.
    pub fn update_bus(
        &mut self,
        index: u16,
        members: &[ChannelId],
        name: &str,
        format: BusFormat,
        endianness: Endianness,
        signed: bool,
    ) {
        let ok = |s: &Self, c: &ChannelId| {
            c.kind == ChannelKind::Digital && s.bus_of(*c).is_none_or(|b| b.id == index) && s.row(*c).is_some()
        };
        let mut order: Vec<ChannelId> = members.iter().copied().filter(|c| ok(self, c)).collect();
        dedup_keep_order(&mut order);
        order.truncate(crate::core::bus::MAX_MEMBERS);
        if order.is_empty() {
            return;
        }
        // A default name follows the members.
        let old_default = self.bus_default_name(index);
        let Some(b) = self.bus_mut(index) else { return };
        b.members = order.clone();
        b.format = format;
        b.endianness = endianness;
        b.signed = signed;
        self.place_members(index, &order);
        let name = name.trim();
        let name = if name.is_empty() || name == old_default { self.bus_default_name(index) } else { name.to_owned() };
        if let Some(r) = self.row_mut(ChannelId::bus(index)) {
            r.name = name;
        }
    }

    /// *Remove Group*: its members stay where they are.
    pub fn remove_bus(&mut self, index: u16) {
        self.buses.retain(|b| b.id != index);
        self.rows.retain(|r| r.channel != ChannelId::bus(index));
    }

    /// Puts `order`'s rows right under group `index`'s row, in that order.
    fn place_members(&mut self, index: u16, order: &[ChannelId]) {
        let mut moved: Vec<Row> = Vec::new();
        for c in order {
            if let Some(k) = self.rows.iter().position(|r| r.channel == *c) {
                moved.push(self.rows.remove(k));
            }
        }
        let at = self.rows.iter().position(|r| r.channel == ChannelId::bus(index)).map_or(self.rows.len(), |k| k + 1);
        self.rows.splice(at..at, moved);
        self.normalize_buses();
    }

    /// Every group's members right after its row, in their
    /// current relative order; rows of unknown groups are dropped.
    pub fn normalize_buses(&mut self) {
        let buses = self.buses.clone();
        self.rows.retain(|r| !r.channel.is_bus() || buses.iter().any(|b| b.row() == r.channel));
        let member_of = |c: ChannelId| buses.iter().find(|b| b.members.contains(&c)).map(|b| b.id);
        let rows = std::mem::take(&mut self.rows);
        let mut out = Vec::with_capacity(rows.len());
        let (members, others): (Vec<Row>, Vec<Row>) = rows.into_iter().partition(|r| member_of(r.channel).is_some());
        let mut members: Vec<Option<Row>> = members.into_iter().map(Some).collect();
        for r in others {
            let bus = r.channel.is_bus().then_some(r.channel.index);
            out.push(r);
            if let Some(id) = bus {
                for m in members.iter_mut() {
                    if m.as_ref().is_some_and(|m| member_of(m.channel) == Some(id)) {
                        out.push(m.take().unwrap());
                    }
                }
            }
        }
        out.extend(members.into_iter().flatten());
        self.rows = out;
    }

    pub fn row_mut(&mut self, id: ChannelId) -> Option<&mut Row> {
        self.rows.iter_mut().find(|r| r.channel == id)
    }

    /// The default name, the probe label or the backend name.
    pub fn default_name(&self, ident: &str) -> String {
        self.probe
            .as_ref()
            .and_then(|(p, _)| p.channel(ident))
            .and_then(|c| c.label.clone())
            .unwrap_or_else(|| ident.to_owned())
    }

    /// Commits a name; empty means the default.
    pub fn rename_row(&mut self, id: ChannelId, name: &str) {
        let Some(default) = self.row(id).map(|r| self.row_default_name(r)) else { return };
        let name = name.trim();
        let name = if name.is_empty() { default } else { name.to_owned() };
        if let Some(r) = self.row_mut(id) {
            r.name = name;
        }
    }

    /// A row's default name.
    pub fn row_default_name(&self, row: &Row) -> String {
        if row.channel.is_bus() { self.bus_default_name(row.channel.index) } else { self.default_name(&row.ident) }
    }

    /// *Reset Channel Names*.
    pub fn reset_names(&mut self) {
        let names: Vec<String> = self.rows.iter().map(|r| self.row_default_name(r)).collect();
        for (r, n) in self.rows.iter_mut().zip(names) {
            r.name = n;
        }
    }

    /// Moves the row `id` so it ends up just before `before` (or
    /// last), keeping every other row's relative order.
    ///
    /// A group moves with its members; a member moved elsewhere
    /// stays in its group, at the matching wire position; a row dropped
    /// between members lands after the group.
    pub fn move_row(&mut self, id: ChannelId, before: Option<ChannelId>) {
        let block: Vec<ChannelId> =
            if id.is_bus() { std::iter::once(id).chain(self.wire_order(id.index)).collect() } else { vec![id] };
        if before.is_some_and(|b| block.contains(&b)) {
            return;
        }
        let mut moved = Vec::new();
        for c in &block {
            if let Some(k) = self.rows.iter().position(|r| r.channel == *c) {
                moved.push(self.rows.remove(k));
            }
        }
        let to = before.and_then(|b| self.rows.iter().position(|r| r.channel == b)).unwrap_or(self.rows.len());
        self.rows.splice(to..to, moved);
        self.normalize_buses();
    }

    pub fn row(&self, id: ChannelId) -> Option<&Row> {
        self.rows.iter().find(|r| r.channel == id)
    }

    pub fn has_data(&self) -> bool {
        self.capture.as_ref().is_some_and(|c| c.read().sample_range().is_some())
    }

    /// The stored interval, for the view limits.
    pub fn bounds(&self) -> Option<DataBounds> {
        let c = self.capture.as_ref()?.read();
        let (begin, end) = c.time_range()?;
        Some(DataBounds { begin, end, max_sample_rate: c.sample_rate()? })
    }

    /// Applies a probe profile: default row names follow its labels
    /// Rows the user renamed keep their names.
    pub fn set_probe(
        &mut self,
        probe: Option<(crate::core::probe::ProbeProfile, crate::app::profiles::ProfileSource)>,
    ) {
        let default_of = |p: &Option<(crate::core::probe::ProbeProfile, _)>, ident: &str| {
            p.as_ref()
                .and_then(|(p, _)| p.channel(ident))
                .and_then(|c| c.label.clone())
                .unwrap_or_else(|| ident.to_owned())
        };
        for row in &mut self.rows {
            if row.name == default_of(&self.probe, &row.ident) {
                row.name = default_of(&probe, &row.ident);
            }
        }
        self.probe = probe;
    }

    /// T0 as an absolute time (no software trigger yet).
    pub fn t0(&self) -> CaptureTime {
        self.origin.resolve(&self.markers, self.trigger)
    }

    /// The capture being recorded, also while the previous round
    /// is swapped in for drawing.
    pub fn live_capture(&self) -> Option<&SharedCapture> {
        if self.previous_shown {
            self.previous.as_ref().and_then(|p| p.capture.as_ref())
        } else {
            self.capture.as_ref()
        }
    }

    /// The round number from round 2 on.
    pub fn repeat_round(&self) -> Option<u32> {
        self.repeat.as_ref().map(|r| r.round).filter(|&n| n >= 2 && self.state.is_busy())
    }

    /// Swaps the previous round in (or back out) around drawing.
    pub(crate) fn swap_previous(&mut self, show: bool) {
        if self.previous_shown == show {
            return;
        }
        let Some(p) = self.previous.as_mut() else { return };
        std::mem::swap(&mut self.capture, &mut p.capture);
        std::mem::swap(&mut self.trigger, &mut p.trigger);
        self.previous_shown = show;
    }

    /// The analyzers' decoded results, in bytes.
    pub fn analyzer_memory_bytes(&self) -> usize {
        self.analyzers.iter().map(|a| a.out.frames.read().memory_bytes()).sum()
    }

    /// Arms the Analyzer Trigger for the capture about to record,
    /// or (`now`) for the frames the recording capture decodes from now on.
    pub(crate) fn arm_analyzer_trigger(&mut self) {
        self.arm_analyzer_trigger_at(false);
    }

    pub(crate) fn arm_analyzer_trigger_at(&mut self, now: bool) {
        let at = &self.analyzer_trigger;
        let a = at.analyzer.and_then(|id| self.analyzers.iter().find(|a| a.id == id));
        self.trigger_scan = match a {
            Some(a)
                if at.enabled
                    && !at.query.is_empty()
                    && self.settings.mode == crate::core::capture::CaptureMode::Continuous =>
            {
                let g = a.out.generation.load(std::sync::atomic::Ordering::SeqCst);
                let (armed_gen, scanned) = if now { (g.wrapping_sub(1), a.out.frames.read().len()) } else { (g, 0) };
                crate::app::analyzers::TriggerScan { armed: true, armed_gen, generation: g, scanned, last: None }
            }
            _ => Default::default(),
        };
    }

    /// New frames of the selected analyzer whose text contains the
    /// query move the trigger. Returns true if it moved.
    pub(crate) fn scan_analyzer_trigger(&mut self) -> bool {
        use std::sync::atomic::Ordering;
        if !self.trigger_scan.armed || !self.analyzer_trigger.enabled {
            return false;
        }
        let at = self.analyzer_trigger.clone();
        let Some(a) = at.analyzer.and_then(|id| self.analyzers.iter().find(|a| a.id == id)) else { return false };
        let Some(rate) = self.capture.as_ref().and_then(|c| c.read().sample_rate()) else { return false };
        let g = a.out.generation.load(Ordering::SeqCst);
        let scan = &mut self.trigger_scan;
        if g != scan.generation {
            scan.generation = g;
            scan.scanned = 0;
        }
        if g == scan.armed_gen {
            return false; // still the previous capture's frames
        }
        let q = at.query.to_lowercase();
        let mut fired = None;
        {
            let f = a.out.frames.read();
            for i in scan.scanned..f.len() {
                let Some(fr) = f.get(i as u32) else { continue };
                if fr.texts.iter().any(|t| t.to_lowercase().contains(&q)) {
                    scan.last = Some(fr.start);
                    fired = Some(fr.start);
                }
            }
            scan.scanned = f.len();
        }
        // The scan ends once this capture is fully decoded.
        let complete = *a.out.status.lock() == crate::decode::runner::Status::Complete;
        if complete && !self.state.is_busy() {
            self.trigger_scan.armed = false;
        }
        let Some(sample) = fired else { return false };
        self.trigger = Some(CaptureTime::from_sample(sample, rate));
        self.origin = self.origin.for_capture_start(true);
        true
    }

    /// Following centers the latest trigger event.
    pub fn follows_analyzer_trigger(&self) -> bool {
        self.trigger_scan.last.is_some() && self.analyzer_trigger.enabled
    }

    /// A new capture discards markers, ranges and the view history.
    pub fn reset_annotations(&mut self) {
        self.markers = Default::default();
        self.ranges = Default::default();
        self.range_history = Default::default();
        self.history = Default::default();
        // Trigger mode puts T0 on the trigger.
        let triggered = self.settings.mode == crate::core::capture::CaptureMode::Triggered;
        self.origin = self.origin.for_capture_start(triggered);
        self.trigger = None;
        self.scanner = None;
    }

    pub fn digital_enabled(&self) -> impl Iterator<Item = ChannelId> + '_ {
        self.settings.enabled.iter().copied().filter(|c| c.kind == ChannelKind::Digital)
    }

    /// The channel's input range, ±10 V when unknown.
    pub fn input_range(&self, id: ChannelId) -> (f64, f64) {
        self.caps
            .as_ref()
            .and_then(|c| c.analog_ranges.iter().find(|(c, _)| *c == id))
            .and_then(|(_, r)| *r)
            .unwrap_or(DEFAULT_INPUT_RANGE)
    }

    /// The analog row's vertical view, created on first use. When
    /// the row's height changed, the scale follows so the visible value
    /// range stays the same.
    pub fn analog_view(&mut self, id: ChannelId, inner_height: f64) -> Option<AnalogView> {
        let range = self.input_range(id);
        let row = self.rows.iter_mut().find(|r| r.channel == id && r.channel.kind == ChannelKind::Analog)?;
        let mut v = *row.analog.get_or_insert_with(|| AnalogView::fit_input(range, inner_height));
        if let Some(old) = row.analog_inner
            && (old as f64 - inner_height).abs() > 1e-3
            && inner_height > 0.0
        {
            v.units_per_px *= old as f64 / inner_height;
            row.analog = Some(v);
        }
        row.analog_inner = Some(inner_height as f32);
        Some(v)
    }

    pub fn set_row_height(&mut self, id: ChannelId, h: f32) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.channel == id) {
            r.height = h.max(r.min_height());
        }
    }

    pub fn set_analog_view(&mut self, id: ChannelId, view: AnalogView) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.channel == id) {
            r.analog = Some(view);
        }
    }
}

pub fn device_display_name(device: &DeviceInfo) -> String {
    let name = device.display_name();
    if device.is_demo { format!("[DEMO] {name}") } else { name }
}

/// Removes repeated channels, keeping the first of each.
fn dedup_keep_order(v: &mut Vec<ChannelId>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|c| seen.insert(*c));
}

#[cfg(all(test, feature = "mock"))]
mod tests {
    use super::*;
    use crate::backend::{Backend, MockBackend};

    fn mock16() -> DeviceInfo {
        MockBackend::seeded(1).scan_devices().remove(1)
    }

    #[test]
    fn new_session_defaults() {
        let s = Session::new(0, mock16(), None, 1000.0);
        assert_eq!(s.name, "Session 0");
        assert_eq!(s.rows.len(), 20);
        let enabled = [ChannelId::digital(0), ChannelId::digital(1), ChannelId::analog(0), ChannelId::analog(1)];
        assert_eq!(s.settings.enabled, enabled);
        let visible: Vec<_> = s.visible_rows().map(Row::identifier).collect();
        assert_eq!(visible, ["D0", "D1", "A0", "A1"]);
        // The default name is the backend name, shown once.
        assert_eq!(s.rows[3].display(), "D3");
        // Compact digital rows, roomy analog ones; 5 pt snapping.
        assert_eq!((s.rows[0].height, s.rows[16].height), (25.0, 100.0));
        assert_eq!(s.rows[0].snapped(33.0), 35.0);
        assert_eq!(s.rows[0].snapped(3.0), 20.0);
        assert_eq!(s.rows[16].snapped(30.0), 45.0);
        assert_eq!(s.device_display_name(), "[DEMO] Mock 16 digital + 4 analog");
    }

    /// A copied setup keeps the source's row order.
    #[test]
    fn copied_rows_keep_their_order() {
        let mut src = Session::new(0, mock16(), None, 1000.0);
        src.move_row(ChannelId::digital(5), Some(ChannelId::digital(1)));
        src.row_mut(ChannelId::digital(5)).unwrap().name = "CS".into();
        let order = |s: &Session| s.rows.iter().map(Row::identifier).take(4).collect::<Vec<_>>();
        assert_eq!(order(&src), ["D0", "D5", "D1", "D2"]);
        let mut s = Session::new(1, mock16(), None, 1000.0);
        s.copy_setup_from(&src);
        assert_eq!(order(&s), ["D0", "D5", "D1", "D2"]);
        assert_eq!(s.rows[1].name, "CS");
        assert_eq!(s.rows.len(), 20);
        // Rows of channels this device lacks are dropped; the rest follow.
        s.apply_rows([src.rows[3].clone(), Row { channel: ChannelId::digital(99), ..src.rows[0].clone() }]);
        assert_eq!(order(&s), ["D2", "D0", "D5", "D1"]);
        assert_eq!(s.rows.len(), 20);
    }

    fn idents(s: &Session) -> Vec<String> {
        s.rows.iter().map(Row::identifier).take(8).collect()
    }

    /// Create, order, move, edit, remove.
    #[test]
    fn buses_nest_their_members() {
        let mut s = Session::new(0, mock16(), None, 1000.0);
        let d = ChannelId::digital;
        // The group lands where the first member row was (D3), with the
        // members in the given order.
        s.move_row(d(3), Some(d(0)));
        assert_eq!(idents(&s)[..4], ["D3", "D0", "D1", "D2"]);
        let mut t = Session::new(0, mock16(), None, 1000.0);
        t.move_row(d(3), Some(d(0)));
        t.create_bus(&[d(2), d(0), d(1), d(2)], "", BusFormat::Hex, Endianness::MsbFirst, false).unwrap();
        assert_eq!(t.wire_order(0), [d(2), d(0), d(1)]);
        assert_eq!(idents(&t)[..5], ["D3", "G1", "D2", "D0", "D1"]);
        assert_eq!(t.row(ChannelId::bus(0)).unwrap().name, "D2–D1");
        let g = s.create_bus(&[d(0), d(1), d(2), d(3)], "", BusFormat::Hex, Endianness::MsbFirst, false).unwrap();
        assert_eq!(g, ChannelId::bus(0));
        assert_eq!(idents(&s)[..6], ["G1", "D0", "D1", "D2", "D3", "D4"]);
        assert_eq!(s.row(g).unwrap().name, "D0–D3");
        assert_eq!(s.wire_order(0), [d(0), d(1), d(2), d(3)]);
        // A channel is in one group only.
        assert!(s.create_bus(&[d(3)], "", BusFormat::Hex, Endianness::MsbFirst, false).is_none());
        // Moving a member above D0 makes it wire 0; it stays in the group.
        s.move_row(d(3), Some(d(0)));
        assert_eq!(s.wire_order(0), [d(3), d(0), d(1), d(2)]);
        s.move_row(d(3), None);
        assert_eq!(s.wire_order(0), [d(0), d(1), d(2), d(3)]);
        assert_eq!(idents(&s)[..6], ["G1", "D0", "D1", "D2", "D3", "D4"]);
        // The group moves with its members; a row dropped inside lands after.
        s.move_row(g, Some(d(6)));
        assert_eq!(idents(&s)[..8], ["D4", "D5", "G1", "D0", "D1", "D2", "D3", "D6"]);
        s.move_row(d(6), Some(d(1)));
        assert_eq!(idents(&s)[..8], ["D4", "D5", "G1", "D0", "D1", "D2", "D3", "D6"]);
        // Collapsed or hidden: the members aren't visible.
        s.settings.enabled = (0..8).map(d).collect();
        let vis = |s: &Session| s.visible_rows().map(Row::identifier).collect::<Vec<_>>();
        assert_eq!(vis(&s).len(), 9);
        s.bus_mut(0).unwrap().collapsed = true;
        assert_eq!(vis(&s), ["D4", "D5", "G1", "D6", "D7"]);
        s.bus_mut(0).unwrap().collapsed = false;
        s.row_mut(g).unwrap().hidden = true;
        assert_eq!(vis(&s), ["D4", "D5", "D6", "D7"]);
        s.row_mut(g).unwrap().hidden = false;
        // Edit: drop D0, add D7; the default name follows.
        s.update_bus(0, &[d(1), d(2), d(3), d(7)], "D0–D3", BusFormat::Decimal, Endianness::LsbFirst, true);
        assert_eq!(s.wire_order(0), [d(1), d(2), d(3), d(7)]);
        assert_eq!(s.row(g).unwrap().name, "D1–D7");
        assert_eq!(s.bus(0).unwrap().format, BusFormat::Decimal);
        assert_eq!(idents(&s)[..8], ["D4", "D5", "G1", "D1", "D2", "D3", "D7", "D0"]);
        // A copied setup keeps the group; removing leaves the members.
        let mut copy = Session::new(1, mock16(), None, 1000.0);
        copy.copy_setup_from(&s);
        assert_eq!(idents(&copy), idents(&s));
        assert_eq!(copy.wire_order(0), s.wire_order(0));
        s.remove_bus(0);
        assert!(s.buses.is_empty());
        assert_eq!(idents(&s)[..7], ["D4", "D5", "D1", "D2", "D3", "D7", "D0"]);
    }

    #[test]
    fn opened_picks_default_rate() {
        let b = MockBackend::seeded(1);
        let dev = b.open(&mock16().id).unwrap();
        let mut s = Session::new(0, mock16(), None, 1000.0);
        s.on_opened(dev.capabilities().clone());
        assert_eq!(s.settings.sample_rate, 8_000_000);
        assert_eq!(s.rates().first(), Some(&100_000_000));
    }
}
