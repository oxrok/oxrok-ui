//! Application shell: wires a [`Backend`] to the UI.
//!
//! State lives in [`LaApp`]; the `ui` modules read it and record
//! intents that are applied right away or at the end of the frame.

pub mod analyzers;
pub mod capture;
pub mod cli;
pub mod commands;
pub mod dialogs;
pub mod file;
pub mod firmware;
pub mod profiles;
pub mod session;
mod sessions;
pub mod settings;
mod ui;
pub mod worker;

#[cfg(feature = "mock")]
use crate::backend::MockBackend;
use crate::backend::{AcquisitionLimit, AcquisitionPlan, AppliedConfig, Backend, DeviceInfo, EndReason};
#[cfg(feature = "mock")]
use crate::core::ManualClock;
use crate::core::animator::{Anchor, JUMP_DURATION, KEY_PAN_DURATION, ViewRequest, WHEEL_ZOOM_DURATION};
use crate::core::capture::{CaptureMode, CaptureSnapshot, CaptureState, StopReason};
use crate::core::nav::{
    FOLLOW_FRACTION, WheelClassifier, clamp_view, clamp_zoom_factor, follow_view, time_per_px_range,
};
use crate::core::{CaptureTime, Clock, MonotonicClock, ViewState};
use crate::ui::ThemeExt;
use crate::ui::gallery::GalleryState;
use crate::ui::toast::{Toast, ToastKind, Toasts};
use capture::{SharedCapture, StoreSink};
use crossbeam_channel::{Receiver, Sender, unbounded};
#[cfg(feature = "mock")]
use dialogs::FixedDialogs;
use dialogs::{FileDialogs, NativeDialogs};
use egui::Ui;
use session::Session;
use settings::AppSettings;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use worker::{Worker, WorkerCmd, WorkerEvent};

/// Product name (neutral; see README).
pub const APP_NAME: &str = "Oxrok UI";

/// The decoder library this build uses.
pub(crate) fn decoder_library() -> String {
    #[cfg(feature = "mock")]
    {
        "built-in decoders".to_owned()
    }
    #[cfg(not(feature = "mock"))]
    format!("{} (RustPython)", crate::oxrok::library_info())
}
/// The least memory a later Repeat round records into, in MB.
const BUFFER_MB_FLOOR: u32 = 64;
/// Target sizes.
pub const BOTTOM_BAR_HEIGHT: f32 = 40.0;
/// Tooltips appear after this long (seconds).
const TOOLTIP_DELAY: f32 = 0.4;
/// Settings write debounce.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// How the app is wired; tests substitute every part.
pub struct AppOptions {
    pub backend: Arc<dyn Backend>,
    pub clock: Arc<dyn Clock>,
    pub dialogs: Arc<dyn FileDialogs>,
    /// `None` = don't persist settings.
    pub settings_path: Option<PathBuf>,
    /// Animation speed scale; 0 makes every view change immediate.
    pub animation_speed: f64,
    /// Scan devices on the UI thread at start (tests) instead of a worker thread.
    pub scan_sync: bool,
    /// User probe profiles; `None` = no user profiles.
    pub profiles_dir: Option<PathBuf>,
    /// Extra built-in profiles as JSON (tests).
    pub extra_builtin_profiles: Vec<String>,
}

/// Results from background jobs.
pub(crate) enum AppEvent {
    FolderPicked(Option<PathBuf>),
    ExportDone(Result<(PathBuf, u64), String>),
    /// The folder for an analyzer's CSV.
    /// The file to extract from was chosen (or not).
    FirmwareInput(Option<PathBuf>),
    /// A line of the script's output, then its result.
    FirmwareLine(String),
    FirmwareDone(Result<Vec<PathBuf>, String>),
    /// A "driver:conn" scan finished.
    Connected(String, Result<Vec<DeviceInfo>, String>),
    /// The CSV lines to write, and where (None: cancelled).
    TableExportFile(Vec<String>, Option<PathBuf>),
    /// Save and load jobs.
    File(ui::files::FileEvent),
    /// A Settings dialog *Browse* finished.
    SettingsFolder(ui::settings::Folder, Option<PathBuf>),
}

#[derive(Default)]
pub(crate) struct UiState {
    /// Install Firmware, while open.
    pub install_firmware: Option<ui::help::InstallFirmware>,
    /// About, while open, and the icon it shows.
    pub about: Option<ui::help::About>,
    pub app_icon: Option<egui::TextureHandle>,
    /// The oxrok wordmark on the start screen, per theme.
    pub logo_light: Option<egui::TextureHandle>,
    pub logo_dark: Option<egui::TextureHandle>,
    /// The Devices popover is open (polls faster).
    pub devices_menu_open: bool,
    /// Device Connection Help, while open.
    pub connection_help: Option<ui::help::ConnectionHelp>,
    /// *Group Channels…* / *Edit Group…*, while open.
    pub bus_dialog: Option<ui::buses::BusDialog>,
    /// The mode to restore from the "Switched to Continuous" toast.
    pub analyzer_trigger_undo: Option<(worker::SessionId, CaptureMode)>,
    /// The Analyzer Trigger card is expanded.
    pub analyzer_trigger_open: bool,
    /// The Export Analyzer Data dialog, while open.
    pub table_export: Option<ui::datatable::TableExport>,
    /// The welcome wizard's step (0–2), while open.
    pub onboarding: Option<u8>,
    /// *Pause Analyzers*, for the app run.
    pub analyzers_paused: bool,
    /// The sessions whose decoding is paused, as last sent.
    pub decode_paused: std::collections::HashSet<worker::SessionId>,
    /// This frame's context, for background jobs and deferred actions.
    pub ctx: egui::Context,
    /// The detached Inspector window's builder, fixed while it
    /// is open (a changing builder would move the window).
    pub inspector_window: Option<egui::ViewportBuilder>,
    /// The Inspector's content is being drawn, so dialogs opened
    /// now belong to it.
    pub in_inspector: bool,
    /// The Notes bottom panel is open (per run).
    pub notes_panel_open: bool,
    /// The analyzer dialog's own window, while it has one.
    pub analyzer_window: Option<ui::analyzers::DialogWindow>,
    /// The open unsaved-work prompt.
    pub prompt: Option<ui::files::Prompt>,
    /// A save or a load is running.
    pub saving: bool,
    pub loading: bool,
    /// The user chose to quit; let the close through.
    pub quit_confirmed: bool,
    /// Exports running (any session).
    pub exporting: u32,
    /// No device to create a session on, so open the Devices menu.
    pub open_devices_menu: bool,
    /// Key presses for egui's next frame (⌘A / ⌘Z to a text field).
    pub forward_events: Vec<egui::Event>,
    /// `Ctrl+Tab` moved egui's focus: drop it next frame.
    pub drop_focus: bool,
    /// `Ctrl+Tab` shows the Sessions popover until then.
    pub sessions_flash_until: Option<Duration>,
    /// The Sessions popover: open, filter, keyboard highlight.
    pub sessions_open: bool,
    /// A row was clicked; close unless a double-click follows.
    pub sessions_close_at: Option<Duration>,
    pub sessions_filter: String,
    pub sessions_highlight: Option<usize>,
    /// The session being renamed inline, and the text.
    pub session_rename: Option<(worker::SessionId, String)>,
    /// The row being dragged in the popover.
    pub session_drag: Option<worker::SessionId>,
    /// The next start may replace an unsaved capture.
    pub replace_confirmed: bool,
    /// Start recording once the device this session waits for opens.
    pub start_when_opened: bool,
    /// The graph area of the last frame.
    pub graph_rect: ui::DefaultRect,
    /// Where the ruler was right-clicked, from the plot's left.
    pub ruler_menu_x: f32,
    /// The Import Options dialog, when open.
    pub import: Option<ui::import::ImportDialog>,
    /// Files waiting to be opened, one after another.
    pub open_queue: std::collections::VecDeque<PathBuf>,
    /// The last import with default options, for the toast's
    /// *Change Import Options…*.
    pub last_import: Option<(PathBuf, String)>,
    /// The Delete Data confirmation, when open.
    pub trim: Option<ui::trim::TrimRequest>,
    /// The channels last exported in this session.
    pub export_included: Option<Vec<crate::core::ChannelId>>,
    /// Files behind toasts' *Show in Folder*, by toast id.
    pub reveal: std::collections::HashMap<String, PathBuf>,
    pub device_info_open: bool,
    /// The Name field's text while the dialog is open.
    pub device_name_edit: Option<String>,
    pub export: Option<ui::export::ExportDialog>,
    pub zoom_text: String,
    pub wheel: WheelClassifier,
    /// Vertical scroll offset of the row stack.
    pub rows_scroll: f32,
    pub show_gallery: bool,
    /// Editor; `open_probe_editor` asks to open it next frame.
    pub probe_editor: Option<ui::probe::ProbeEditor>,
    pub open_probe_editor: bool,
    /// The progress card is minimized into the rail.
    pub progress_minimized: bool,
    /// The analog row under the pointer, from the last frame.
    pub hover_row: Option<ui::HoverRow>,
    pub row_geometry: Vec<ui::RowGeom>,
    pub drag: Option<ui::DragState>,
    /// Drag velocity for the fling.
    pub velocity: crate::core::nav::VelocityTracker,
    /// Range zoom in progress.
    pub marquee: Option<ui::Marquee>,
    /// The zoom tool is armed for the next drag.
    pub zoom_tool_armed: bool,
    /// Jump to Time: open, and whether the last Enter was out of range.
    pub jump_open: bool,
    /// The Settings dialog, when open.
    pub settings_dialog: Option<ui::settings::SettingsDialog>,
    /// The command palette, when open.
    pub palette: Option<ui::palette::Palette>,
    /// CMD *Find in Data Table*: focus the table's search next frame.
    pub table_focus: bool,
    /// *Add Analyzer…*: open the toolbar's picker next frame.
    pub open_picker: bool,
    /// Keyboard Shortcuts dialog.
    pub shortcuts_open: bool,
    pub shortcuts_filter: String,
    pub jump_error: bool,
    /// The digital row under the pointer.
    pub hover_digital: Option<crate::core::ChannelId>,
    /// Inline name editor.
    pub rename: Option<ui::Rename>,
    /// "Other channel conditions" is expanded.
    pub trigger_conditions_open: bool,
    /// The Device options row is expanded, and its scope.
    pub device_options_open: bool,
    pub device_options_scope: usize,
    /// When the read-only options were last refreshed.
    pub options_read_at: Option<Duration>,
    /// Table or terminal.
    pub data_mode: ui::datatable::DataMode,
    /// The merged table rows and its search.
    pub table: ui::datatable::TableCache,
    /// Per analyzer, the frames already cleared from the terminal.
    pub terminal_cleared: std::collections::HashMap<u32, usize>,
    /// The terminal's line count last frame, and whether the user
    /// scrolled away from the bottom (then auto-scroll pauses).
    pub terminal_len: usize,
    pub terminal_detached: bool,
    /// The analyzer settings dialog.
    pub analyzer_dialog: Option<ui::analyzers::AnalyzerDialog>,
    /// The Analyzers panel's decoder list filter (empty panel).
    pub panel_filter: String,
    /// Picker filter text and keyboard highlight.
    pub picker_filter: String,
    pub picker_highlight: usize,
    /// The last deleted analyzer and its position, for Undo.
    pub deleted_analyzer: Option<(usize, analyzers::Analyzer)>,
    /// The pulse under the pointer (channel, e0, e1), for `Shift+R`.
    pub pulse_under: std::cell::Cell<Option<(crate::core::ChannelId, u64, u64)>>,
    /// A range being placed.
    pub range_place: Option<ui::ranges::RangePlace>,
    /// The range edge being dragged: id, and whether it's the end.
    pub range_edge: Option<(u32, bool)>,
    /// The open popup.
    pub range_popup: Option<u32>,
    pub metric_cache: ui::ranges::MetricCache,
    /// When Copy was last pressed ("Copied!").
    pub copied_at: Option<std::time::Duration>,
    /// A marker being placed.
    pub placing: Option<ui::markers::Placing>,
    /// The marker line being dragged.
    pub marker_drag: Option<crate::core::markers::MarkerRef>,
    /// The open edit popup: item id and its position.
    pub marker_edit: Option<(u32, egui::Pos2)>,
    /// The last clicked marker (Shift+click ranges).
    pub last_marker_click: Option<u32>,
    /// How far PageUp / PageDown scroll the rows.
    pub rows_page: f32,
    /// The open confirmation dialog.
    pub confirm: Option<ui::dialogs::Confirm>,
    /// The row a plot right-click landed on.
    pub context_row: Option<crate::core::ChannelId>,
    /// A right-click asks to open the row menu here (next frame).
    pub open_row_menu_at: Option<egui::Pos2>,
    /// Where the open row menu is anchored.
    pub row_menu_pos: egui::Pos2,
    /// A row being dragged to a new position.
    pub reorder: Option<ui::Reorder>,
    /// What the last Auto Layout ran for (viewport height, rows).
    pub auto_layout_for: Option<(f32, Vec<crate::core::ChannelId>)>,
    /// Inner heights of the analog rows, from the last frame.
    pub row_inner_height: std::collections::HashMap<crate::core::ChannelId, f32>,
    pub gallery: GalleryState,
}

/// A hardware trigger for an edge event with level conditions,
/// when the device has the needed match types.
fn hw_trigger_for(
    t: &crate::core::trigger::DigitalTrigger,
    caps: Option<&crate::backend::DeviceCaps>,
    pre_trigger_percent: u8,
) -> Option<crate::backend::HwTrigger> {
    use crate::backend::TriggerMatch;
    use crate::core::trigger::TriggerEvent;
    let hw = caps?.hw_trigger.as_ref()?;
    let event = match t.event {
        TriggerEvent::Rising => TriggerMatch::Rising,
        TriggerEvent::Falling => TriggerMatch::Falling,
        _ => return None,
    };
    let mut matches = vec![(t.channel?, event)];
    matches.extend(t.linked.iter().map(|(c, high)| (*c, if *high { TriggerMatch::One } else { TriggerMatch::Zero })));
    matches
        .iter()
        .all(|(_, m)| hw.matches.contains(m))
        .then_some(crate::backend::HwTrigger { matches, pre_trigger_percent })
}

/// A buffered device's sample limit and pre-trigger percentage
/// (`SR_CONF_CAPTURE_RATIO`, whole percent) for `pre` and `post` seconds.
/// The limit is sized so both parts are at least as long as asked after
/// rounding the percentage.
pub fn buffered_trigger_plan(pre: f64, post: f64, rate: u64) -> (u64, u8) {
    let pre = (pre.max(0.0) * rate as f64).ceil();
    let post = (post.max(0.0) * rate as f64).ceil().max(1.0);
    let ratio = (100.0 * pre / (pre + post)).round().clamp(0.0, 99.0);
    let need_post = (post * 100.0 / (100.0 - ratio)).ceil();
    let need_pre = if ratio > 0.0 { (pre * 100.0 / ratio).ceil() } else { 0.0 };
    (need_post.max(need_pre) as u64, ratio as u8)
}

/// The decoder library: the mock decoders with the `mock` feature,
/// otherwise oxrok. `extra` is the user's additional decoder directory.
fn decoder_engine(extra: Option<PathBuf>) -> Result<Box<dyn crate::decode::DecoderEngine>, String> {
    #[cfg(feature = "mock")]
    {
        let _ = extra;
        Ok(Box::new(crate::decode::mock::MockEngine::new()))
    }
    #[cfg(not(feature = "mock"))]
    match crate::oxrok::OxrokDecoders::new(extra.as_deref()) {
        Ok(e) => {
            eprintln!(
                "{}: {} decoders",
                crate::oxrok::library_info(),
                crate::decode::DecoderEngine::decoders(&e).len()
            );
            Ok(Box::new(e))
        }
        Err(e) => Err(e),
    }
}

pub struct LaApp {
    pub(crate) backend: Arc<dyn Backend>,
    /// The decoder thread.
    pub(crate) decode: crate::decode::runner::DecodeService,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) dialogs: Arc<dyn FileDialogs>,
    pub(crate) worker: Worker,
    pub(crate) devices: Option<Vec<DeviceInfo>>,
    scan_rx: Option<Receiver<(Vec<DeviceInfo>, crate::backend::FirmwareStatus)>>,
    /// The firmware status of the last scan.
    pub(crate) firmware: crate::backend::FirmwareStatus,
    /// Scans run on the UI thread (tests).
    scan_sync: bool,
    /// When the last scan was asked for.
    last_scan_at: Option<Duration>,
    /// The active session.
    pub(crate) session: Option<Session>,
    /// The other sessions, in no particular order.
    pub(crate) parked: Vec<Session>,
    /// Display order of every session.
    pub(crate) order: Vec<worker::SessionId>,
    next_session_id: u64,
    pub(crate) settings: AppSettings,
    pub(crate) profiles: profiles::ProfileStore,
    settings_path: Option<PathBuf>,
    settings_dirty_at: Option<Duration>,
    pub(crate) toasts: Toasts,
    pub(crate) ui: UiState,
    pub(crate) events_tx: Sender<AppEvent>,
    events_rx: Receiver<AppEvent>,
    animation_speed: f64,
    title: String,
    /// The theme drawn now (the settings may say System).
    theme_kind: crate::ui::theme::ThemeKind,
    /// The formats the backend imports, read at start.
    pub(crate) input_formats: Vec<crate::backend::InputFormat>,
    /// The native macOS menu, when installed.
    #[cfg(all(target_os = "macos", feature = "macos-menu"))]
    pub(crate) native_menu: Option<ui::macmenu::NativeMenu>,
}

impl LaApp {
    /// Production constructor: system clock, native dialogs, persisted settings.
    pub fn new(cc: &eframe::CreationContext<'_>, backend: Arc<dyn Backend>) -> Self {
        // macOS: Option (Alt) + a number or letter types a special character
        // (⌥1 = "¡") or a dead key (⌥I = a dead circumflex), which macOS's
        // input pipeline swallows before egui sees a Key event. Treat Option
        // as Alt so the panel shortcuts Alt+1…Alt+5 / Alt+I reach the app.
        #[cfg(target_os = "macos")]
        if let Some(window) = cc.winit_window() {
            use winit::platform::macos::{OptionAsAlt, WindowExtMacOS};
            window.set_option_as_alt(OptionAsAlt::Both);
        }
        #[allow(unused_mut)]
        let mut app = Self::with_options(
            cc,
            AppOptions {
                backend,
                clock: Arc::new(MonotonicClock::new()),
                dialogs: Arc::new(NativeDialogs),
                settings_path: settings::default_path(),
                animation_speed: 1.0,
                scan_sync: false,
                profiles_dir: profiles::default_dir(),
                extra_builtin_profiles: Vec::new(),
            },
        );
        // The native macOS menu, in the real app only.
        #[cfg(all(target_os = "macos", feature = "macos-menu"))]
        {
            app.ui.ctx = cc.egui_ctx.clone();
            app.install_native_menu();
        }
        // On first launch.
        app.open_onboarding_on_first_run();
        app
    }

    /// The menus are native, so the in-window bar is hidden.
    fn native_menu_on(&self) -> bool {
        #[cfg(all(target_os = "macos", feature = "macos-menu"))]
        return self.native_menu.is_some();
        #[cfg(not(all(target_os = "macos", feature = "macos-menu")))]
        false
    }

    /// Test constructor: manual clock, immediate animations,
    /// synchronous scan, no persistence.
    #[cfg(feature = "mock")]
    pub fn new_for_test(cc: &eframe::CreationContext<'_>, backend: MockBackend) -> Self {
        Self::with_options(
            cc,
            AppOptions {
                backend: Arc::new(backend),
                clock: Arc::new(ManualClock::new()),
                dialogs: Arc::new(FixedDialogs(None)),
                settings_path: None,
                animation_speed: 0.0,
                scan_sync: true,
                profiles_dir: None,
                extra_builtin_profiles: Vec::new(),
            },
        )
    }

    pub fn with_options(cc: &eframe::CreationContext<'_>, o: AppOptions) -> Self {
        let ctx = cc.egui_ctx.clone();
        let (settings, settings_backup) = o.settings_path.as_deref().map(AppSettings::load).unwrap_or_default();
        let theme_kind = settings.appearance.theme.resolve(ctx.system_theme());
        crate::ui::install(&ctx, theme_kind.theme());
        // The UI size keys are commands, not egui's own zoom keys.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        crate::ui::clock::set_clock(&ctx, o.clock.clone());
        let repaint_ctx = ctx.clone();
        let worker = Worker::spawn(o.backend.clone(), move || repaint_ctx.request_repaint());
        let (events_tx, events_rx) = unbounded();
        let profiles = profiles::ProfileStore::load(o.profiles_dir.clone(), &o.extra_builtin_profiles);
        let mut app = Self {
            decode: {
                let extra = settings.decoders.extra_dir.clone();
                crate::decode::runner::DecodeService::spawn(move || decoder_engine(extra))
            },
            backend: o.backend,
            clock: o.clock,
            dialogs: o.dialogs,
            worker,
            devices: None,
            scan_rx: None,
            firmware: Default::default(),
            scan_sync: o.scan_sync,
            last_scan_at: None,
            session: None,
            parked: Vec::new(),
            order: Vec::new(),
            next_session_id: 0,
            settings,
            profiles,
            settings_path: o.settings_path,
            settings_dirty_at: None,
            toasts: Toasts::default(),
            ui: UiState::default(),
            events_tx,
            events_rx,
            animation_speed: o.animation_speed,
            title: String::new(),
            input_formats: Vec::new(),
            theme_kind,
            #[cfg(all(target_os = "macos", feature = "macos-menu"))]
            native_menu: None,
        };
        app.input_formats = app.backend.input_formats();
        // The old or broken settings file was kept as a backup.
        if let Some(b) = settings_backup {
            let name = b.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            app.ui.reveal.insert("settings-reset".into(), b);
            let action = if cfg!(target_os = "macos") { "Show in Finder" } else { "Show in Explorer" };
            app.toast(
                Toast::new(
                    ToastKind::Warning,
                    format!("Settings were reset. The previous file could not be used and was kept as {name}."),
                )
                .id("settings-reset")
                .action(action),
            );
        }
        // Report probe profiles that failed to load.
        for (file, err) in app.profiles.errors.clone() {
            app.toast(Toast::new(ToastKind::Warning, format!("{file}: {err}")).heading("Probe profile skipped"));
        }
        app.ui.ctx = ctx;
        app.rescan_devices();
        app
    }

    /// A capture file is being read.
    pub fn is_loading(&self) -> bool {
        self.ui.loading
    }

    /// The missing firmware files of the last scan (tests).
    pub fn firmware_missing(&self) -> Vec<String> {
        self.firmware.missing.iter().map(|m| m.file.clone()).collect()
    }

    pub fn now(&self) -> Duration {
        self.clock.now()
    }

    pub fn devices(&self) -> Option<&[DeviceInfo]> {
        self.devices.as_deref()
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// The window title last sent.
    pub fn window_title(&self) -> &str {
        &self.title
    }

    /// The device as lists show it: its nickname or
    /// vendor + model, `[DEMO] ` prefixed for demo devices.
    pub fn device_label(&self, d: &DeviceInfo) -> String {
        let name = self.settings.device_name(d);
        if d.is_demo { format!("[DEMO] {name}") } else { name }
    }

    /// For tests: change preferences directly.
    pub fn settings_mut(&mut self) -> &mut AppSettings {
        &mut self.settings
    }

    /// For tests and automation: change the session's settings directly.
    pub fn session_mut(&mut self) -> Option<&mut Session> {
        self.session.as_mut()
    }

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// The Notes bottom panel is open.
    pub fn notes_panel_open(&self) -> bool {
        self.ui.notes_panel_open
    }

    pub fn toasts(&self) -> &Toasts {
        &self.toasts
    }

    pub(crate) fn toast(&mut self, toast: Toast) {
        let now = self.now();
        self.toasts.show(toast, now);
    }

    /// Remembers the session's settings for its device and schedules a save.
    pub(crate) fn mark_settings_dirty(&mut self) {
        if let Some(s) = self.session.take() {
            self.mark_settings_dirty_for(&s);
            self.session = Some(s);
        }
        self.settings_dirty_at.get_or_insert(self.clock.now());
    }

    pub(crate) fn mark_settings_dirty_for(&mut self, s: &Session) {
        if !s.is_read_only() {
            self.settings.device_settings.insert(s.device.id.0.clone(), s.settings.clone());
        }
        self.settings_dirty_at.get_or_insert(self.clock.now());
    }

    // ---- sessions and capture (06) ----

    /// The active session's analyzers join the decoder thread, on its
    /// capture when it has one.
    pub(crate) fn register_decoders(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        let sid = s.id;
        if let Some(cap) = s.capture.clone() {
            let source = analyzers::CaptureSource { capture: cap, done: s.capture_done.clone() };
            self.decode.set_source(sid, Some(Arc::new(source)));
        }
        let analyzers = s.analyzers.clone();
        for a in analyzers {
            if let Some(info) = self.decoder_info(&a.decoder) {
                self.decode.set(sid, a.id, a.task(&info), a.out.clone());
            }
        }
    }

    /// Start (Continuous mode only in 0.1.0).
    pub(crate) fn start_capture(&mut self, ctx: &egui::Context) {
        let Some(s) = self.session.as_mut() else {
            self.toast(Toast::new(ToastKind::Error, "Recording needs a device. Pick one to begin."));
            return;
        };
        if s.state.is_busy() {
            return;
        }
        // An opened file records into a new session or into itself.
        if s.is_read_only() {
            let choice = s.record_choice.unwrap_or(self.settings.capture.record_on_opened);
            if self.matching_device().is_none() {
                self.toast_no_matching_device();
                return;
            }
            // The banner's choice, or the preference.
            match choice {
                settings::RecordOnOpened::Ask => {
                    self.ui.prompt = Some(ui::files::Prompt::RecordOnOpened { remember: false })
                }
                settings::RecordOnOpened::NewSession => self.run_pending(ui::files::Pending::NewCaptureWithSettings),
                settings::RecordOnOpened::Replace => self.record_here(),
            }
            return;
        }
        // Replacing an unsaved capture.
        if s.is_unsaved() && !std::mem::take(&mut self.ui.replace_confirmed) {
            if self.settings.capture.ask_replace_unsaved {
                self.ui.prompt = Some(ui::files::Prompt::ReplaceCapture { dont_ask: false });
                return;
            }
            self.toast(
                Toast::new(ToastKind::Info, "Previous capture replaced")
                    .id("replaced")
                    .lifetime(Duration::from_secs(4)),
            );
        }
        let Some(s) = self.session.as_mut() else { return };
        if s.caps.is_none() {
            let msg = s.open_error.clone().unwrap_or_else(|| "Switching device…".into());
            self.toast(Toast::new(ToastKind::Error, msg));
            return;
        }
        let enabled = s.settings.enabled.clone();
        if enabled.is_empty() {
            self.toast(Toast::new(ToastKind::Error, "Enable at least one channel to record."));
            return;
        }
        let triggered = s.settings.mode == CaptureMode::Triggered;
        if triggered && !s.settings.trigger.is_valid(&enabled) {
            self.toast(Toast::new(ToastKind::Error, "The digital trigger setup is invalid."));
            return;
        }
        s.reset_annotations();
        // A Repeat run starts with round 1.
        s.repeat = s.settings.repeats().then(|| session::RepeatRun { round: 1, ..Default::default() });
        s.previous = None;
        self.begin_capture(ctx, false);
    }

    /// Steps 3–4, for a new capture or the next Repeat round
    /// which keeps the previous round on screen.
    fn begin_capture(&mut self, ctx: &egui::Context, next_round: bool) {
        let Some(s) = self.session.as_mut() else { return };
        let enabled = s.settings.enabled.clone();
        let triggered = s.settings.mode == CaptureMode::Triggered;
        // Both rounds count toward the memory buffer.
        let kept_mb =
            if next_round { s.capture.as_ref().map_or(0, |c| (c.read().memory_bytes() / 1_000_000) as u32) } else { 0 };
        if next_round {
            let span = s.capture.as_ref().and_then(|c| c.read().time_range());
            if let Some(r) = s.repeat.as_mut() {
                r.span = span;
            }
            s.previous = Some(session::PrevRound {
                capture: s.capture.take(),
                trigger: s.trigger.take(),
                snapshot: s.snapshot.clone(),
            });
            s.scanner = None;
        }
        // Compression when the device has it and it's on.
        let rle = s.caps.as_ref().is_some_and(|c| c.rle) && s.settings.rle;
        // Timed mode: a sample limit of ⌈t × rate⌉.
        let rate = s.settings.sample_rate;
        let streams = s.caps.as_ref().is_some_and(|c| c.supports_continuous);
        let trig = s.settings.trigger.clone();
        let mut hw_trigger = None;
        let limit = match s.settings.mode {
            CaptureMode::Timed => AcquisitionLimit::Samples((s.settings.timer_seconds * rate as f64).ceil() as u64),
            // Streaming devices record continuously while the
            // software trigger scans.
            CaptureMode::Triggered if streams => AcquisitionLimit::Continuous,
            // A buffered device records one buffer: with a hardware edge
            // trigger when it has one, sized for the pre- and
            // post-trigger times. The software trigger finds the
            // same instant in the data.
            CaptureMode::Triggered => {
                let (limit, pre) = buffered_trigger_plan(trig.pre_trigger_seconds, trig.post_trigger_seconds, rate);
                hw_trigger = hw_trigger_for(&trig, s.caps.as_ref(), pre);
                AcquisitionLimit::Samples(limit)
            }
            CaptureMode::Continuous => AcquisitionLimit::Continuous,
        };
        // Session reset: drop the data, keep settings, rows and zoom.
        let data = SharedCapture::default();
        let repaint_ctx = ctx.clone();
        let sink = StoreSink::new(
            data.clone(),
            s.settings.mode,
            s.settings.buffer_mb.saturating_sub(kept_mb).max(BUFFER_MB_FLOOR),
            Box::new(move || repaint_ctx.request_repaint()),
        );
        data.write().hw_trigger_expected = hw_trigger.is_some();
        if triggered {
            s.scanner = Some(crate::engine::trigger::TriggerScanner::new(trig.clone(), rate));
        }
        s.capture = Some(data.clone());
        if next_round {
            // The decoders follow this round once it is shown.
            if let Some(r) = s.repeat.as_mut() {
                r.round += 1;
                r.auto_end = false;
                r.done = Default::default();
            }
        } else {
            // The decoders follow the new capture.
            s.capture_done = Default::default();
            let source = analyzers::CaptureSource { capture: data, done: s.capture_done.clone() };
            self.decode.set_source(s.id, Some(Arc::new(source)));
        }
        let s = self.session.as_mut().unwrap();
        s.state = CaptureState::Starting;
        // The file no longer holds this session's samples.
        s.samples_saved = false;
        s.snapshot = Some(CaptureSnapshot {
            mode: s.settings.mode,
            sample_rate: s.settings.sample_rate,
            enabled: enabled.clone(),
            buffer_mb: s.settings.buffer_mb,
            timer_seconds: s.settings.timer_seconds,
            rle,
            trigger: triggered.then(|| trig.clone()),
            logic_threshold: s.settings.logic_threshold.clone(),
            device_options: s.settings.device_options.clone(),
        });
        // Later rounds keep the view where it is.
        s.follow = !next_round;
        s.arm_analyzer_trigger();
        if !next_round {
            self.ui.rows_scroll = 0.0;
        }
        self.worker.send(WorkerCmd::Start {
            session: s.id,
            config: s.device_config(enabled),
            plan: AcquisitionPlan { rle, trigger: hw_trigger, ..AcquisitionPlan::new(limit) },
            sink: Box::new(sink),
        });
    }

    /// The snapshot takes what the device accepted; a
    /// refused option reverts with a toast.
    fn on_applied(&mut self, applied: AppliedConfig) {
        let Some(device) = self.session.as_ref().map(|s| self.device_label(&s.device)) else { return };
        let s = self.session.as_mut().unwrap();
        let read = s.non_default(&applied.options);
        if let Some(snap) = s.snapshot.as_mut() {
            if applied.threshold.is_some() {
                snap.logic_threshold = applied.threshold.clone();
            }
            if !applied.options.is_empty() {
                snap.device_options = read;
            }
        }
        let mut toasts = Vec::new();
        for r in &applied.rejected {
            let Some(o) = s.caps.as_ref().and_then(|c| {
                c.options.iter().find(|g| g.scope == r.scope)?.options.iter().find(|o| o.id == r.id).cloned()
            }) else {
                continue;
            };
            let value = crate::backend::format_value(&r.requested, o.unit);
            match r.kept.clone().or_else(|| o.default.clone()) {
                Some(kept) => s.set_option(&r.scope, &o, kept),
                None => {
                    if let Some(m) = s.settings.device_options.get_mut(&r.scope) {
                        m.remove(&o.id);
                    }
                }
            }
            toasts.push(format!("{device} didn't accept {} = {value}", o.name));
        }
        if !toasts.is_empty() {
            self.mark_settings_dirty();
        }
        for t in toasts {
            self.toast(Toast::new(ToastKind::Warning, t));
        }
    }

    /// Stop.
    pub(crate) fn stop_capture(&mut self) {
        if let Some(s) = self.session.as_mut()
            && matches!(s.state, CaptureState::Starting | CaptureState::Recording)
        {
            s.state = CaptureState::Stopping;
            self.worker.send(WorkerCmd::Stop { session: s.id });
        }
    }

    pub(crate) fn toggle_capture(&mut self, ctx: &egui::Context) {
        match self.session.as_ref().map(|s| s.state) {
            Some(CaptureState::Starting | CaptureState::Recording) => {
                // The user ends a Repeat run.
                if let Some(r) = self.session.as_mut().and_then(|s| s.repeat.as_mut()) {
                    r.user_stop = true;
                }
                self.stop_capture()
            }
            // Between rounds, Stop still ends the run.
            Some(CaptureState::Stopping) => {
                if let Some(r) = self.session.as_mut().and_then(|s| s.repeat.as_mut()) {
                    r.user_stop = true;
                }
            }
            _ => self.start_capture(ctx),
        }
    }

    /// The pointer over `ui`'s layer (the waveform): `None` while the
    /// command palette is open, or a dialog or popover covers that point
    /// (nothing behind a dialog reacts).
    pub(crate) fn plot_pointer(&self, ui: &egui::Ui) -> Option<egui::Pos2> {
        if self.ui.palette.is_some() {
            return None;
        }
        let p = ui.input(|i| i.pointer.hover_pos())?;
        ui.ctx().layer_id_at(p).is_none_or(|l| l == ui.layer_id()).then_some(p)
    }

    /// Whether `s`'s analyzers are paused: by hand, or during a
    /// Repeat run with *Pause analyzers during Repeat*.
    pub(crate) fn analyzers_paused(&self, s: &Session) -> bool {
        self.ui.analyzers_paused || (self.settings.capture.pause_analyzers_on_repeat && s.repeat.is_some())
    }

    /// Tells the decoder thread when a session's pause changes.
    fn sync_analyzer_pause(&mut self) {
        let want: Vec<(worker::SessionId, bool)> =
            self.session.iter().chain(&self.parked).map(|s| (s.id, self.analyzers_paused(s))).collect();
        for (id, paused) in want {
            if paused != self.ui.decode_paused.contains(&id) {
                self.decode.pause(id, paused);
                if paused {
                    self.ui.decode_paused.insert(id);
                } else {
                    self.ui.decode_paused.remove(&id);
                }
            }
        }
    }

    /// The recording round replaces the previous one on screen.
    fn promote_round(&mut self) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let Some(prev) = s.previous.take() else { return };
        // The view, markers and ranges stay where they are relative to the
        // trigger; the view limits then keep it on the new round's data.
        if let (Some(old), Some(new)) = (prev.trigger, s.trigger)
            && old != new
        {
            let dt = new - old;
            let view = s.view.state();
            s.view.request(ViewRequest::Immediate(ViewState { left_time: view.left_time + dt, ..view }), now);
            s.markers.shift(dt);
            s.ranges.shift(dt);
            s.range_history.shift(dt);
            if let Some((a, b)) = s.repeat.as_mut().and_then(|r| r.span.as_mut()) {
                (*a, *b) = (*a + dt, *b + dt);
            }
        }
        let done = s.repeat.as_ref().map(|r| r.done.clone()).unwrap_or_default();
        s.capture_done = done.clone();
        if let Some(cap) = s.capture.clone() {
            let source = analyzers::CaptureSource { capture: cap, done };
            self.decode.set_source(s.id, Some(Arc::new(source)));
        }
    }

    /// Drops the recording round; the previous one is the capture again.
    fn restore_previous(&mut self) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        let Some(p) = s.previous.take() else { return false };
        s.capture = p.capture;
        s.trigger = p.trigger;
        s.snapshot = p.snapshot;
        s.scanner = None;
        s.follow = false;
        s.state = CaptureState::Finished;
        true
    }

    fn finish_capture(&mut self) {
        let Some(s) = self.session.as_mut() else { return };
        // A round that ended by itself goes on; a stop
        // between rounds keeps the previous one.
        let run = s.repeat.clone();
        let auto = run.as_ref().is_some_and(|r| r.auto_end && !r.user_stop);
        let triggered_without_trigger = s.snapshot.as_ref().is_some_and(|x| x.trigger.is_some()) && s.trigger.is_none();
        if s.previous.is_some() {
            if auto && !triggered_without_trigger {
                self.promote_round();
            } else {
                self.restore_previous();
                self.next_round_or_end(auto);
                return;
            }
        }
        let Some(s) = self.session.as_mut() else { return };
        s.capture_done.store(true, std::sync::atomic::Ordering::SeqCst);
        s.follow = false;
        if s.has_data() {
            s.state = CaptureState::Finished;
        } else {
            // Finished(empty) → reset.
            s.state = CaptureState::Empty;
            s.capture = None;
            return;
        }
        // A final trigger scan over the last data.
        self.scan_trigger();
        // Step 2: center the trigger, keeping the zoom
        // (17 *Center on trigger after capture*). Later Repeat rounds keep
        // the view.
        let later_round = self.session.as_ref().and_then(|s| s.repeat.as_ref()).is_some_and(|r| r.round > 1);
        let center = self.settings.capture.center_on_trigger && !later_round;
        if let Some(t) = self.session.as_ref().and_then(|s| s.trigger).filter(|_| center) {
            let now = self.now();
            let s = self.session.as_mut().unwrap();
            s.view.request(ViewRequest::MoveToTime { time: t, anchor: Anchor::Center, duration: Duration::ZERO }, now);
        }
        let auto = self.session.as_ref().and_then(|s| s.repeat.as_ref()).is_some_and(|r| r.auto_end && !r.user_stop);
        self.next_round_or_end(auto);
    }

    /// Starts the next round while Repeat is on, else ends the run.
    fn next_round_or_end(&mut self, auto: bool) {
        let Some(s) = self.session.as_mut() else { return };
        if auto && s.repeat.is_some() && s.settings.repeats() && s.caps.is_some() {
            let ctx = self.ui.ctx.clone();
            self.begin_capture(&ctx, true);
        } else {
            s.repeat = None;
        }
    }

    fn handle_worker_event(&mut self, ev: WorkerEvent) {
        let current = self.session.as_ref().map(|s| s.id);
        // Live pins.
        if let WorkerEvent::Pins { session, result, took } = &ev {
            let now = self.now();
            let every = self.wire_status_interval();
            let Some(s) = self.session.iter_mut().chain(self.parked.iter_mut()).find(|s| s.id == *session) else {
                return;
            };
            s.pins_pending = false;
            s.pins_read_at = Some(now);
            // Back-off: three slow reads in a row stretch the interval
            // (poll_pins) and say so once.
            if *took > every {
                s.pins_slow_reads = s.pins_slow_reads.saturating_add(1);
                if s.pins_slow_reads == PINS_SLOW_READS_NOTICE && pins_interval(every, s.pins_slow_reads) > every {
                    let msg = format!(
                        "Wire status reads take {} on this device; polling less often.",
                        crate::core::format::time_short(took.as_secs_f64())
                    );
                    self.toast(Toast::new(ToastKind::Info, msg).id("wire-status-slow"));
                }
            } else {
                s.pins_slow_reads = 0;
            }
            let Some(s) = self.session.iter_mut().chain(self.parked.iter_mut()).find(|s| s.id == *session) else {
                return;
            };
            match result {
                Ok(levels) if s.wire_status => {
                    s.pins = Some(session::LivePins { levels: levels.clone(), stale: false })
                }
                Ok(_) => {}
                Err(e) => {
                    s.wire_status = false;
                    if let Some(p) = s.pins.as_mut() {
                        p.stale = true;
                    }
                    let msg = format!("Wire status stopped: {e}");
                    self.toast(Toast::new(ToastKind::Warning, msg).id("wire-status"));
                }
            }
            return;
        }
        // The read-only rows' values.
        if let WorkerEvent::Options { session, groups } = &ev {
            let s = self.session.iter_mut().chain(self.parked.iter_mut()).find(|s| s.id == *session);
            if let Some(caps) = s.and_then(|s| s.caps.as_mut()) {
                caps.options = groups.clone();
            }
            return;
        }
        // A session in the background finished opening its device.
        if let WorkerEvent::Opened { session, result } = &ev
            && Some(*session) != current
        {
            if let Some(s) = self.parked.iter_mut().find(|s| s.id == *session) {
                match result {
                    Ok(caps) => s.on_opened(caps.clone()),
                    Err(e) => s.open_error = Some(e.clone()),
                }
            }
            return;
        }
        match ev {
            WorkerEvent::Opened { session, result } if Some(session) == current => match result {
                Ok(caps) => {
                    self.session.as_mut().unwrap().on_opened(caps);
                    if std::mem::take(&mut self.ui.start_when_opened) {
                        let ctx = self.ui.ctx.clone();
                        self.start_capture(&ctx);
                    }
                }
                Err(e) => {
                    self.session.as_mut().unwrap().open_error = Some(e.clone());
                    self.toast(Toast::new(ToastKind::Error, e).heading("Failed to open device"));
                }
            },
            WorkerEvent::Started { session, result } if Some(session) == current => match result {
                Ok(applied) => {
                    let s = self.session.as_mut().unwrap();
                    let requested = s.settings.sample_rate;
                    if let Some(snap) = s.snapshot.as_mut() {
                        snap.sample_rate = applied.sample_rate;
                    }
                    if s.state == CaptureState::Starting {
                        s.state = CaptureState::Recording;
                    }
                    if applied.sample_rate != requested {
                        let rate = crate::core::format::sample_rate(applied.sample_rate as f64);
                        self.toast(Toast::new(ToastKind::Info, format!("Sample rate adjusted to {rate}")));
                    }
                    self.on_applied(applied);
                }
                Err(e) => {
                    // A later round that fails ends the run, keeping the last one.
                    if self.restore_previous() {
                        self.session.as_mut().unwrap().repeat = None;
                        self.toast(Toast::new(ToastKind::Error, e).heading("Could not begin the capture."));
                        return;
                    }
                    let s = self.session.as_mut().unwrap();
                    s.repeat = None;
                    s.state = CaptureState::Empty;
                    s.capture = None;
                    s.follow = false;
                    // Drivers may refuse compression at high rates,
                    // with an error that doesn't say so.
                    let e = if s.snapshot.as_ref().is_some_and(|snap| snap.rle) {
                        format!(
                            "{e}\nTry disabling Compression (RLE), as some devices don't support it at higher sample rates."
                        )
                    } else {
                        e
                    };
                    self.toast(Toast::new(ToastKind::Error, e).heading("Could not begin the capture."));
                }
            },
            WorkerEvent::Stopped { session }
                if Some(session) == current && self.session.as_ref().is_some_and(|s| s.state.is_busy()) =>
            {
                self.finish_capture();
            }
            _ => {}
        }
    }

    /// Reacts to the acquisition ending by itself and to memory-policy stops.
    fn poll_capture(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        if s.state != CaptureState::Recording {
            return;
        }
        let Some(cap) = s.capture.as_ref() else { return };
        let (end, stop_request) = {
            let mut d = cap.write();
            // Logic data the acquisition thread left waiting (it stores
            // batches when it receives packets; this covers quiet spells).
            d.store_pending(capture::FLUSH_INTERVAL);
            (d.end.clone(), d.stop_request.take())
        };
        if let Some(reason) = stop_request {
            let msg = match reason {
                StopReason::BufferFull => {
                    "The capture stopped at the memory buffer limit. Raise the buffer in Device Settings to go longer."
                }
                StopReason::Backlog => {
                    "Capture stopped because unprocessed data exceeded 90 % of the buffer. Try a lower sample rate or fewer channels."
                }
            };
            self.toast(Toast::new(ToastKind::Error, msg).id("memory-stop"));
            self.stop_capture();
            return;
        }
        // The software trigger scans the new data; after
        // the trigger, the capture runs for the post-trigger time.
        let done = self.scan_trigger();
        // A Trigger round is shown once its trigger is found.
        if self.session.as_ref().is_some_and(|s| s.previous.is_some() && s.trigger.is_some()) {
            self.promote_round();
        }
        if done {
            self.mark_auto_end();
            self.stop_capture();
            return;
        }
        let Some(s) = self.session.as_ref() else { return };
        let Some(cap) = s.capture.as_ref() else { return };
        // Timer branch (backstop; the device also has a sample limit).
        let timer_done = s.snapshot.as_ref().is_some_and(|snap| {
            snap.mode == CaptureMode::Timed
                && cap
                    .read()
                    .sample_range()
                    .is_some_and(|(b, e)| (e - b) as f64 >= snap.timer_seconds * snap.sample_rate as f64)
        });
        if timer_done && end.is_none() {
            self.mark_auto_end();
            self.stop_capture();
            return;
        }
        if let Some(reason) = end {
            if reason == EndReason::Completed {
                self.check_short_capture();
                self.mark_auto_end();
            }
            match reason {
                EndReason::Error(e) => self.toast(Toast::new(ToastKind::Error, e).heading("Capture error")),
                EndReason::Disconnected => {
                    self.toast(Toast::new(ToastKind::Error, "The device dropped. Reconnect it or start a new session."))
                }
                EndReason::Completed | EndReason::Stopped => {}
            }
            // Let the worker join the acquisition; its `Stopped` event finishes the capture.
            self.stop_capture();
        }
    }

    /// The round ended by itself.
    fn mark_auto_end(&mut self) {
        if let Some(r) = self.session.as_mut().and_then(|s| s.repeat.as_mut()) {
            r.auto_end = true;
        }
    }

    /// One software-trigger step. Returns true when the capture
    /// has run `post_trigger_seconds` past the trigger.
    fn scan_trigger(&mut self) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        let (Some(mut scanner), Some(cap)) = (s.scanner.take(), s.capture.clone()) else { return false };
        let post = s.snapshot.as_ref().and_then(|x| x.trigger.as_ref()).map_or(0.0, |t| t.post_trigger_seconds);
        let (found, progress, rate, end, known) = {
            let d = cap.read();
            let (Some(rate), Some((_, end))) = (d.sample_rate(), d.sample_range()) else {
                s.scanner = Some(scanner);
                return false;
            };
            // With the device's trigger, the data before it is
            // pre-trigger data and may match too. Wait for its position,
            // then look for the exact edge just around it (the software
            // match), falling back to the device's position.
            let found = match (d.trigger, d.hw_trigger_expected, d.hw_trigger) {
                (Some(_), ..) => None,
                (None, true, None) if d.end.is_none() => None,
                (None, true, Some(h)) => {
                    const WINDOW: u64 = 64;
                    if scanner.progress() < h.saturating_sub(WINDOW) {
                        scanner.skip_to(h.saturating_sub(WINDOW));
                    }
                    let stop = end.min(h + WINDOW);
                    match scanner.scan(|c| d.channel(c), stop) {
                        Some(t) => Some(t),
                        None if stop >= h + WINDOW || d.end.is_some() => Some(h.min(end.saturating_sub(1))),
                        None => None,
                    }
                }
                // No device trigger, or the capture ended without one.
                (None, ..) => scanner.scan(|c| d.channel(c), end),
            };
            (found, scanner.progress(), rate, end, d.trigger)
        };
        {
            let mut d = cap.write();
            d.trigger_scan = progress;
            if let Some(t) = found {
                d.trigger = Some(t);
            }
        }
        s.scanner = Some(scanner);
        let trigger = known.or(found);
        if let Some(t) = found {
            s.trigger = Some(CaptureTime::from_sample(t, rate));
        }
        trigger.is_some_and(|t| end >= t + (post * rate as f64).ceil() as u64)
    }

    /// A Timed capture the device ended early (its memory filled).
    fn check_short_capture(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        let (Some(snap), Some(cap)) = (s.snapshot.as_ref(), s.capture.as_ref()) else { return };
        if snap.mode != CaptureMode::Timed {
            return;
        }
        let Some((b, e)) = cap.read().sample_range() else { return };
        let got = e - b;
        let wanted = (snap.timer_seconds * snap.sample_rate as f64).ceil() as u64;
        if (got as f64) >= wanted as f64 * 0.99 {
            return;
        }
        let duration = crate::core::format::time_short(got as f64 / snap.sample_rate as f64);
        let head =
            format!("The device's memory filled after {duration} ({} samples).", crate::core::format::count(got));
        let rle_available = s.caps.as_ref().is_some_and(|c| c.rle);
        // A note, not advice: the user may have turned compression off
        // because the device refuses it at this rate.
        let advice = if rle_available && !snap.rle {
            "Compression (RLE) was off."
        } else {
            "Lower the sample rate or shorten the capture."
        };
        self.toast(
            Toast::new(ToastKind::Warning, format!("{head} {advice}"))
                .heading("Capture ended early")
                .id("short-capture"),
        );
    }

    // ---- view (08) ----

    /// Advances the view animation and applies follow mode and the limits.
    pub(crate) fn update_view(&mut self, width_px: f64) -> Option<ViewState> {
        let now = self.now();
        let s = self.session.as_mut()?;
        if (s.view.state().width_px - width_px).abs() > 1e-9 {
            s.view.set_width(width_px, now);
        }
        let state = s.view.advance(now);
        let recording = s.state.is_busy();
        let Some(mut bounds) = s.bounds() else { return Some(state) };
        // A later Repeat round is held to the previous round's span
        // too, so the view doesn't move while the round fills in.
        if let Some((a, b)) = s.repeat.as_ref().and_then(|r| r.span).filter(|_| recording) {
            bounds.begin = bounds.begin.min(a);
            bounds.end = bounds.end.max(b);
        }
        let live_trigger = s.trigger.filter(|_| recording && s.follow && s.follows_analyzer_trigger());
        let next = if let Some(t) = live_trigger {
            // "Live Trigger": the latest trigger event centered.
            clamp_view(state.with_time_at(t, width_px / 2.0), &bounds, recording, width_px / 2.0)
        } else if recording && s.follow {
            follow_view(state, bounds.end)
        } else {
            clamp_view(state, &bounds, recording, width_px / 2.0)
        };
        if next != state {
            s.view.request(ViewRequest::Immediate(next), now);
        }
        Some(next)
    }

    /// Wheel/keyboard zoom by `factor` about `fix_px` (061).
    pub(crate) fn zoom_view(&mut self, factor: f64, fix_px: f64, duration: Duration) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let Some(bounds) = s.bounds() else { return };
        let base = s.view.target();
        let factor = clamp_zoom_factor(&base, factor, &bounds, s.state.is_busy());
        // While following, zoom about the newest data and keep following.
        let fix_px = if s.follow && s.state.is_busy() { base.width_px * FOLLOW_FRACTION } else { fix_px };
        s.view.request(ViewRequest::Zoom { fix_px, factor, duration }, now);
    }

    /// Manual pan by `dx_px` (positive = later); cancels follow.
    pub(crate) fn pan_view(&mut self, dx_px: f64, duration: Duration) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        if !s.has_data() {
            return;
        }
        s.follow = false;
        s.view.request(ViewRequest::PanBy { dx_px, duration }, now);
    }

    /// Fit all data.
    pub(crate) fn fit_view(&mut self) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let Some(b) = s.bounds() else { return };
        if s.follow && s.state.is_busy() {
            let v = s.view.target();
            let tpp = b.duration_secs() / (v.width_px * FOLLOW_FRACTION);
            let v = follow_view(ViewState { time_per_px: tpp.max(f64::MIN_POSITIVE), ..v }, b.end);
            s.view.request(ViewRequest::Immediate(v), now);
        } else {
            s.view.request(ViewRequest::MoveToRange { t0: b.begin, t1: b.end, duration: JUMP_DURATION }, now);
        }
    }

    /// Full zoom in about the center.
    pub(crate) fn full_zoom_in(&mut self) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let Some(b) = s.bounds() else { return };
        let v = s.view.target();
        let (min_tpp, _) = time_per_px_range(&b, v.width_px, s.state.is_busy());
        let fix_px = if s.follow && s.state.is_busy() { v.width_px * FOLLOW_FRACTION } else { v.width_px / 2.0 };
        let req = ViewRequest::ZoomAbsolute { fix_px, time_scale: min_tpp * v.width_px, duration: WHEEL_ZOOM_DURATION };
        s.view.request(req, now);
    }

    /// The theme choice (View → Theme, the palette, Settings).
    pub fn set_theme_pref(&mut self, pref: settings::ThemePref) {
        self.settings.appearance.theme = pref;
        self.settings_dirty_at.get_or_insert(self.clock.now());
        let ctx = self.ui.ctx.clone();
        self.apply_appearance(&ctx);
    }

    /// An explicit Light or Dark choice.
    pub fn set_theme(&mut self, kind: crate::ui::theme::ThemeKind) {
        self.set_theme_pref(match kind {
            crate::ui::theme::ThemeKind::Light => settings::ThemePref::Light,
            crate::ui::theme::ThemeKind::Dark => settings::ThemePref::Dark,
        });
    }

    /// The theme drawn now (System resolved).
    pub fn theme(&self) -> crate::ui::theme::ThemeKind {
        self.theme_kind
    }

    /// Every frame: the System theme follows the OS, and the UI
    /// scale and tooltips follow the settings.
    pub(crate) fn apply_appearance(&mut self, ctx: &egui::Context) {
        let a = &self.settings.appearance;
        let kind = a.theme.resolve(ctx.system_theme());
        if kind != self.theme_kind {
            self.theme_kind = kind;
            crate::ui::theme::set_theme(ctx, kind.theme());
        }
        if (ctx.zoom_factor() - a.ui_scale).abs() > 1e-4 {
            ctx.set_zoom_factor(a.ui_scale);
        }
        // With tooltips off, hover tooltips never appear.
        let delay = if a.show_tooltips { TOOLTIP_DELAY } else { f32::MAX };
        if ctx.global_style().interaction.tooltip_delay != delay {
            ctx.all_styles_mut(|s| s.interaction.tooltip_delay = delay);
        }
    }

    /// The UI scale, `delta` steps up or down, or back to 1.0.
    pub(crate) fn step_ui_scale(&mut self, delta: Option<i32>) {
        use settings::{UI_SCALE_MAX, UI_SCALE_MIN, UI_SCALE_STEP};
        let a = &mut self.settings.appearance;
        a.ui_scale = match delta {
            None => 1.0,
            Some(d) => ((a.ui_scale + d as f32 * UI_SCALE_STEP) * 10.0).round() / 10.0,
        }
        .clamp(UI_SCALE_MIN, UI_SCALE_MAX);
        self.settings_dirty_at.get_or_insert(self.clock.now());
    }

    /// Zoom an analog row by `k` about `y` (points from the inner top).
    pub(crate) fn zoom_analog(&mut self, id: crate::core::ChannelId, k: f64, y: f32) {
        let Some(h) = self.ui.hover_row.filter(|h| h.channel == id).map(|h| h.inner_height) else { return };
        let Some(s) = self.session.as_mut() else { return };
        let Some(v) = s.analog_view(id, h as f64) else { return };
        let at = v.y_to_value(y as f64, h as f64);
        s.set_analog_view(id, v.zoom_about(k, at, h as f64));
    }

    /// Y Fit: the visible data's min/max with a 10 % margin.
    pub(crate) fn y_fit(&mut self, id: crate::core::ChannelId) {
        let inner_h = self.ui.row_inner_height.get(&id).copied().unwrap_or(80.0) as f64;
        let Some(s) = self.session.as_mut() else { return };
        let view = s.view.state();
        let Some(cap) = s.capture.clone() else { return };
        let range = {
            let d = cap.read();
            let (Some(rate), Some(ch)) = (d.sample_rate(), d.analog(id)) else { return };
            let a = view.left_time.to_sample_floor(rate).max(0) as u64;
            let b = view.right_time().to_sample_floor(rate).max(0) as u64 + 1;
            ch.min_max(a, b)
        };
        let (Some((lo, hi)), Some(v)) = (range, s.analog_view(id, inner_h)) else { return };
        s.set_analog_view(id, v.fit(lo as f64, hi as f64, inner_h));
    }

    /// Jump to the data start or end.
    pub(crate) fn jump_to_edge(&mut self, end: bool) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let Some(b) = s.bounds() else { return };
        s.follow = false;
        let (time, anchor) = if end { (b.end, Anchor::Right) } else { (b.begin, Anchor::Left) };
        s.view.request(ViewRequest::MoveToTime { time, anchor, duration: KEY_PAN_DURATION }, now);
    }

    /// Y Fit on every visible analog row.
    pub(crate) fn y_fit_all(&mut self) {
        let ids: Vec<crate::core::ChannelId> = self.session.as_ref().map_or(Vec::new(), |s| {
            s.visible_rows().filter(|r| r.channel.kind == crate::core::ChannelKind::Analog).map(|r| r.channel).collect()
        });
        for id in ids {
            self.y_fit(id);
        }
    }

    /// Deletes the selected markers and ranges, without
    /// confirmation.
    fn delete_selected_annotations(&mut self) {
        let removed: Vec<_> = self
            .session
            .as_ref()
            .map(|s| s.ranges.items.iter().filter(|r| r.selected).cloned().collect())
            .unwrap_or_default();
        if let Some(s) = self.session.as_mut() {
            s.markers.remove_selected();
            s.ranges.remove_selected();
        }
        self.record_range_delete(removed);
        self.fix_origin();
    }

    /// Records a range add for undo.
    fn record_range_add(&mut self, id: u32) {
        let r = self.session.as_ref().and_then(|s| s.ranges.get(id)).cloned();
        if let Some(r) = r
            && let Some(s) = self.session.as_mut()
        {
            s.range_history.record(crate::core::ranges::RangeOp::Add(r));
        }
    }

    /// Records a range delete and shows an Undo toast.
    fn record_range_delete(&mut self, removed: Vec<crate::core::ranges::RangeMeasurement>) {
        if removed.is_empty() {
            return;
        }
        let n = removed.len();
        if let Some(s) = self.session.as_mut() {
            s.range_history.record(crate::core::ranges::RangeOp::Delete(removed));
        }
        let body = if n == 1 { "1 measurement deleted".into() } else { format!("{n} measurements deleted") };
        self.toast(
            Toast::new(ToastKind::Info, body)
                .heading("Deleted")
                .action("Undo")
                .id("range-deleted")
                .lifetime(Duration::from_secs(5)),
        );
    }

    /// `Ctrl/Cmd+Z` — undo the last add or delete.
    fn undo_range(&mut self) {
        let Some(op) = self.session.as_mut().and_then(|s| s.range_history.undo()) else { return };
        let s = self.session.as_mut().unwrap();
        match op {
            crate::core::ranges::RangeOp::Add(r) => s.ranges.remove(r.id),
            crate::core::ranges::RangeOp::Delete(v) => v.into_iter().for_each(|r| s.ranges.insert(r)),
        }
    }

    /// `Ctrl/Cmd+Shift+Z` / `Ctrl/Cmd+Y` — redo.
    fn redo_range(&mut self) {
        let Some(op) = self.session.as_mut().and_then(|s| s.range_history.redo()) else { return };
        let s = self.session.as_mut().unwrap();
        match op {
            crate::core::ranges::RangeOp::Add(r) => s.ranges.insert(r),
            crate::core::ranges::RangeOp::Delete(v) => v.iter().for_each(|r| s.ranges.remove(r.id)),
        }
    }

    /// `Shift+R` — a range wrapping the pulse under the pointer.
    fn quick_add_pulse(&mut self) {
        let Some((ch, e0, e1)) = self.ui.pulse_under.take() else { return };
        let Some(rate) = self
            .session
            .as_ref()
            .filter(|s| s.has_data() && !s.state.is_busy())
            .and_then(|s| s.capture.as_ref().and_then(|c| c.read().sample_rate()))
        else {
            return;
        };
        let t0 = crate::core::CaptureTime::from_sample(e0, rate);
        let t1 = crate::core::CaptureTime::from_sample(e1, rate);
        if t1 <= t0 {
            return;
        }
        let id = {
            let s = self.session.as_mut().unwrap();
            let id = s.ranges.add(ch, t0, t1);
            s.ranges.select_only(id);
            id
        };
        self.ui.range_popup = Some(id);
        self.record_range_add(id);
    }

    /// Scans for devices, off the UI thread (inline in tests).
    pub(crate) fn rescan_devices(&mut self) {
        if self.scan_rx.is_some() {
            return;
        }
        self.last_scan_at = Some(self.now());
        if self.scan_sync {
            let devices = self.backend.scan_devices();
            let firmware = self.backend.firmware_status();
            self.on_scanned(devices, firmware);
            return;
        }
        let (tx, rx) = unbounded();
        let backend = self.backend.clone();
        let ctx = self.ui.ctx.clone();
        std::thread::Builder::new()
            .name("device-scan".into())
            .spawn(move || {
                let devices = backend.scan_devices();
                let _ = tx.send((devices, backend.firmware_status()));
                ctx.request_repaint();
            })
            .expect("spawn device-scan thread");
        self.scan_rx = Some(rx);
    }

    /// A scan is running ('s Rescan spinner).
    pub(crate) fn scanning(&self) -> bool {
        self.scan_rx.is_some()
    }

    /// The new list; sessions whose device couldn't be opened
    /// re-bind when it is back.
    fn on_scanned(&mut self, devices: Vec<DeviceInfo>, firmware: crate::backend::FirmwareStatus) {
        let ids: Vec<crate::backend::DeviceId> = devices.iter().map(|d| d.id.clone()).collect();
        self.devices = Some(devices);
        self.firmware = firmware;
        let mut reopen = Vec::new();
        for s in self.session.iter_mut().chain(self.parked.iter_mut()) {
            if s.caps.is_none() && s.open_error.is_some() && ids.contains(&s.device.id) {
                s.open_error = None;
                reopen.push((s.id, s.device.id.clone()));
            }
        }
        for (session, device) in reopen {
            self.worker.send(WorkerCmd::Open { session, device });
        }
    }

    /// Every 2 s while devices are being looked at, else 10 s;
    /// never while a session records.
    fn poll_devices(&mut self) {
        let busy =
            self.session.as_ref().is_some_and(|s| s.state.is_busy()) || self.parked.iter().any(|s| s.state.is_busy());
        // Device Connection Help scans only on its Rescan.
        if busy || self.scan_rx.is_some() || self.ui.connection_help.is_some() {
            return;
        }
        let looking = self.session.is_none() || self.ui.devices_menu_open;
        let every = Duration::from_secs(if looking { 2 } else { 10 });
        let due = self.last_scan_at.is_none_or(|t| self.now().saturating_sub(t) >= every);
        if due {
            self.rescan_devices();
        } else if let Some(t) = self.last_scan_at {
            self.ui.ctx.request_repaint_after(every.saturating_sub(self.now().saturating_sub(t)));
        }
    }

    /// The active session reads its pins every interval,
    /// counted from the end of the previous read; marks the last
    /// state stale whenever polling can't go on.
    /// The configured Wire Status interval.
    fn wire_status_interval(&self) -> Duration {
        Duration::from_millis(self.settings.waveform.wire_status_interval_ms.clamp(20, 1000) as u64)
    }

    fn poll_pins(&mut self) {
        let now = self.now();
        let every = self.wire_status_interval();
        let Some(s) = self.session.as_mut() else { return };
        let every = pins_interval(every, s.pins_slow_reads);
        if !s.wire_status || !s.can_read_pins() {
            if !s.pins_pending
                && let Some(p) = s.pins.as_mut()
            {
                p.stale = true;
            }
            return;
        }
        if s.pins_pending {
            return;
        }
        let due = s.pins_read_at.is_none_or(|t| now.saturating_sub(t) >= every);
        if !due {
            let wait = every.saturating_sub(now.saturating_sub(s.pins_read_at.unwrap_or(now)));
            self.ui.ctx.request_repaint_after(wait);
            return;
        }
        let enabled =
            s.settings.enabled.iter().copied().filter(|c| c.kind == crate::core::ChannelKind::Digital).collect();
        let config = s.device_config(enabled);
        s.pins_pending = true;
        let id = s.id;
        self.worker.send(WorkerCmd::ReadPins { session: id, config });
    }

    fn drain_events(&mut self) {
        if let Some(rx) = &self.scan_rx
            && let Ok((devices, firmware)) = rx.try_recv()
        {
            self.scan_rx = None;
            self.on_scanned(devices, firmware);
        }
        while let Some(ev) = self.worker.try_recv() {
            self.handle_worker_event(ev);
        }
        while let Ok(ev) = self.events_rx.try_recv() {
            match ev {
                AppEvent::File(e) => self.handle_file_event(e),
                AppEvent::SettingsFolder(which, p) => self.on_settings_folder(which, p),
                AppEvent::FolderPicked(folder) => {
                    if let Some(d) = self.ui.export.as_mut() {
                        d.picking = false;
                        if let Some(f) = folder {
                            self.settings.last_dirs.export = Some(f.clone());
                            d.folder = f;
                            self.settings_dirty_at.get_or_insert(self.clock.now());
                        }
                    }
                }
                AppEvent::ExportDone(Ok((path, rows))) => {
                    let body = format!("{} ({} rows)", path.display(), crate::core::format::count(rows));
                    self.ui.reveal.insert("export-done".into(), path);
                    self.toast(
                        Toast::new(ToastKind::Success, body)
                            .heading("Export complete")
                            .id("export-done")
                            .action("Show in Folder"),
                    );
                }
                AppEvent::ExportDone(Err(e)) => self.toast(Toast::new(ToastKind::Error, e).heading("Export failed")),
                AppEvent::TableExportFile(lines, Some(path)) => self.write_table_export(lines, path),
                AppEvent::TableExportFile(_, None) => {}
                AppEvent::Connected(spec, r) => self.on_connected(spec, r),
                AppEvent::FirmwareInput(p) => self.on_firmware_input(p),
                AppEvent::FirmwareLine(l) => self.on_firmware_line(l),
                AppEvent::FirmwareDone(r) => self.on_firmware_done(r),
            }
        }
    }

    /// Window title.
    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.session {
            None => APP_NAME.to_owned(),
            Some(s) => {
                let state = if s.is_read_only() {
                    "File"
                } else if s.device.is_demo {
                    "Demo"
                } else {
                    "Connected"
                };
                // `• ` marks an unsaved capture.
                let mark = if s.is_unsaved() { "• " } else { "" };
                format!("{mark}{APP_NAME} [{} - {state}] [{}]", self.settings.device_name(&s.device), s.name)
            }
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn save_settings_if_due(&mut self, ctx: &egui::Context) {
        let Some(at) = self.settings_dirty_at else { return };
        let elapsed = self.now().saturating_sub(at);
        if elapsed < SAVE_DEBOUNCE {
            ctx.request_repaint_after(SAVE_DEBOUNCE - elapsed);
            return;
        }
        self.settings_dirty_at = None;
        if let Some(path) = &self.settings_path
            && let Err(e) = self.settings.save(path)
        {
            self.toast(Toast::new(ToastKind::Warning, e.to_string()).heading("Could not save settings"));
        }
    }
}

impl eframe::App for LaApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ui.ctx = ctx.clone();
        // Cmd/Ctrl+Q asks the window to close, so quitting goes
        // through the unsaved-capture prompt; Cmd+M minimizes (macOS).
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if cfg!(target_os = "macos") && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::M)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.apply_appearance(&ctx);
        self.intercept_quit(&ctx);
        self.drain_events();
        self.poll_devices();
        self.poll_pins();
        self.poll_capture();
        self.sync_analyzer_pause();
        // Every session's Analyzer Trigger.
        for s in self.session.iter_mut().chain(self.parked.iter_mut()) {
            s.scan_analyzer_trigger();
        }
        if std::mem::take(&mut self.ui.drop_focus) {
            ctx.memory_mut(|m| {
                if let Some(f) = m.focused() {
                    m.surrender_focus(f);
                }
            });
        }
        self.handle_command_keys(&ctx);
        // Draw the previous round while the next one has nothing to show.
        if let Some(s) = self.session.as_mut() {
            s.swap_previous(true);
        }

        if self.native_menu_on() {
            #[cfg(all(target_os = "macos", feature = "macos-menu"))]
            self.native_menu_frame(&ctx);
        } else {
            self.menu_bar(ui);
        }
        self.bottom_bar(ui);
        self.notes_panel_bottom(ui);
        self.inspector(ui);
        let theme = ui.la_theme();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme.color(crate::ui::Token::Canvas))).show(
            ui,
            |ui| {
                if self.ui.show_gallery {
                    crate::ui::gallery::gallery(ui, &mut self.ui.gallery);
                } else if self.session.is_some() {
                    self.graph(ui);
                } else {
                    self.start_screen(ui);
                }
            },
        );
        self.observe_history();
        self.dialogs_ui(&ctx);
        self.onboarding_ui(&ctx);
        self.palette_ui(&ctx);
        self.prompt_ui(&ctx);
        self.drop_files(&ctx);
        for s in self.session.iter_mut().chain(self.parked.iter_mut()) {
            s.swap_previous(false);
        }
        let now = self.now();
        let actions = self.toasts.ui(&ctx, now, BOTTOM_BAR_HEIGHT + 10.0);
        for (id, action) in actions {
            if id == "analyzer-deleted" && action == "Undo" {
                self.toasts.dismiss(&id);
                self.undo_delete_analyzer();
            } else if id == "analyzer-trigger-mode" && action == "Undo" {
                // Back to the previous mode, the trigger off.
                self.toasts.dismiss(&id);
                if let Some((sid, mode)) = self.ui.analyzer_trigger_undo.take()
                    && let Some(s) = self.session_by_id_mut(sid)
                {
                    s.settings.mode = mode;
                    s.analyzer_trigger.enabled = false;
                }
            } else if id == "range-deleted" && action == "Undo" {
                self.toasts.dismiss(&id);
                self.undo_range();
            } else {
                self.file_toast_action(&id, &action);
            }
        }
        self.update_title(&ctx);
        self.save_settings_if_due(&ctx);

        // Repaint only while something moves.
        if let Some(s) = &self.session
            && (s.view.is_animating() || s.state.is_busy())
        {
            ctx.request_repaint();
        } else if self.session.as_ref().is_some_and(|s| s.history.is_pending()) {
            // Come back when the debounce is over.
            ctx.request_repaint_after(crate::core::history::HISTORY_DEBOUNCE);
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input.events.append(&mut self.ui.forward_events);
    }

    fn on_exit(&mut self) {
        // Quitting while recording stops the capture.
        if let Some(s) = self.session.as_ref().filter(|s| s.state.is_busy()) {
            self.worker.send(WorkerCmd::Stop { session: s.id });
        }
        if let Some(path) = &self.settings_path {
            let _ = self.settings.save(path);
        }
    }
}

/// Wire Status back-off: slow reads in a row (`slow`) double the
/// interval each, up to a second.
const PINS_SLOW_READS_NOTICE: u32 = 3;
fn pins_interval(every: Duration, slow: u32) -> Duration {
    if slow < PINS_SLOW_READS_NOTICE {
        return every;
    }
    let doubled = every.saturating_mul(1 << (slow - PINS_SLOW_READS_NOTICE + 1).min(6));
    doubled.min(Duration::from_secs(1)).max(every)
}

#[cfg(test)]
mod tests {
    use super::buffered_trigger_plan;

    /// Both parts are at least as long as asked after rounding.
    #[test]
    fn buffered_trigger_sizing() {
        assert_eq!(buffered_trigger_plan(0.1, 0.9, 1_000_000), (1_000_000, 10));
        assert_eq!(buffered_trigger_plan(0.0, 1.0, 1_000), (1_000, 0));
        for (pre, post) in [(0.0123, 0.5), (2e-3, 3e-3), (1.0, 1e-3), (0.3, 0.0)] {
            let rate = 100_000_000;
            let (limit, pct) = buffered_trigger_plan(pre, post, rate);
            let pre_got = limit * pct as u64 / 100;
            assert!(pre_got as f64 >= (pre * rate as f64).ceil() - 1.0, "{pre} {post}: {pre_got}");
            assert!((limit - pre_got) as f64 >= (post * rate as f64).ceil(), "{pre} {post}");
            assert!(pct <= 99);
        }
    }
}
