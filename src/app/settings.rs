//! Persisted application settings.
//!
//! A single JSON file in the platform config directory, written atomically
//! (temp file + fsync + rename). Unknown keys are ignored and missing keys
//! defaulted; another `schema_version` resets the file, keeping a backup
//! eframe persists the window geometry itself.

use crate::core::capture::CaptureSettings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 2;
/// Default label column width.
pub const DEFAULT_LABEL_WIDTH: f32 = 105.0;
/// Default and minimum Inspector width.
pub const INSPECTOR_MIN_WIDTH: f32 = 300.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InspectorSettings {
    /// The Inspector panel is shown.
    pub open: bool,
    /// The selected tab (`"device"`, `"analyzers"`, …).
    pub tab: String,
    pub width: f32,
    /// The Device tab's *Command line* row is expanded.
    pub command_line_open: bool,
    /// The Inspector is in a window of its own.
    pub detached: bool,
    /// The detached window's last position and size.
    pub window: Option<WindowRect>,
}

impl Default for InspectorSettings {
    fn default() -> Self {
        Self {
            open: false,
            tab: "device".into(),
            width: INSPECTOR_MIN_WIDTH,
            command_line_open: false,
            detached: false,
            window: None,
        }
    }
}

/// A window's outer position and inner size, in points.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The wheel and keyboard zoom fix-point.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelativeZoom {
    #[default]
    Mouse,
    Center,
}

/// The zoom tool's range mode.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarqueeMode {
    #[default]
    X,
    Y,
    XY,
}

/// At most this many recent zooms.
pub const RECENT_ZOOMS: usize = 3;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LastDirs {
    pub export: Option<PathBuf>,
    /// The last folder a capture was saved to or opened from.
    pub capture: Option<PathBuf>,
}

/// *When recording on an opened file*.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordOnOpened {
    #[default]
    Ask,
    NewSession,
    Replace,
}

/// Export Format.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    #[default]
    Csv,
    Binary,
    Vcd,
    Sigrok,
}

/// Time Range types.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportRange {
    #[default]
    All,
    Visible,
    Markers,
    Measurement,
}

/// Export choices remembered app-wide.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportPrefs {
    pub format: ExportFormat,
    pub range: ExportRange,
    pub downsample: u64,
    pub iso8601: bool,
}

impl Default for ExportPrefs {
    fn default() -> Self {
        Self { format: ExportFormat::Csv, range: ExportRange::All, downsample: 1, iso8601: false }
    }
}

/// At most this many recent palette commands.
pub const RECENT_COMMANDS: usize = 20;
/// At most this many recent files.
pub const RECENT_FILES: usize = 10;

/// The theme choice; *System* follows the OS.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    Light,
    Dark,
    #[default]
    System,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::Light, ThemePref::Dark, ThemePref::System];

    pub fn label(self) -> &'static str {
        let en = match self {
            ThemePref::Light => "Light",
            ThemePref::Dark => "Dark",
            ThemePref::System => "System",
        };
        crate::l10n::tr(en)
    }

    /// The theme to draw, given the OS appearance (Dark when unknown).
    pub fn resolve(self, system: Option<egui::Theme>) -> crate::ui::theme::ThemeKind {
        use crate::ui::theme::ThemeKind;
        match self {
            ThemePref::Light => ThemeKind::Light,
            ThemePref::Dark => ThemeKind::Dark,
            ThemePref::System => match system {
                Some(egui::Theme::Light) => ThemeKind::Light,
                _ => ThemeKind::Dark,
            },
        }
    }
}

/// UI scale limits and step.
pub const UI_SCALE_MIN: f32 = 0.3;
pub const UI_SCALE_MAX: f32 = 2.0;
pub const UI_SCALE_STEP: f32 = 0.1;

/// `appearance`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemePref,
    pub ui_scale: f32,
    pub show_tooltips: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { theme: ThemePref::System, ui_scale: 1.0, show_tooltips: true }
    }
}

/// `waveform`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Waveform {
    /// Traces in channel colors (off: one neutral color).
    pub color_waveforms: bool,
    /// Trace line width.
    pub trace_width: crate::ui::waveform::TraceWidth,
    /// Auto Layout.
    pub auto_layout: bool,
    /// Hover readouts.
    pub show_interactive_measurement: bool,
    /// *Zoom relative to*.
    pub zoom_relative_to: RelativeZoom,
    /// The pause between live pin reads, 20–1000 ms.
    pub wire_status_interval_ms: u32,
}

impl Default for Waveform {
    fn default() -> Self {
        Self {
            color_waveforms: true,
            trace_width: Default::default(),
            auto_layout: false,
            show_interactive_measurement: true,
            zoom_relative_to: RelativeZoom::Mouse,
            wire_status_interval_ms: 100,
        }
    }
}

/// `time_labels` (wall-clock labels).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeLabels {
    pub utc: bool,
    pub iso8601: bool,
}

impl TimeLabels {
    pub fn format(&self) -> crate::core::format::WallClockFormat {
        crate::core::format::WallClockFormat { utc: self.utc, iso: self.iso8601 }
    }
}

/// `capture`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CapturePrefs {
    /// *Center on trigger after capture*.
    pub center_on_trigger: bool,
    /// *Ask before replacing an unsaved capture*.
    pub ask_replace_unsaved: bool,
    /// What recording on an opened file does.
    pub record_on_opened: RecordOnOpened,
    /// *Pause analyzers during Repeat*.
    pub pause_analyzers_on_repeat: bool,
}

impl Default for CapturePrefs {
    fn default() -> Self {
        Self {
            center_on_trigger: true,
            ask_replace_unsaved: false,
            record_on_opened: RecordOnOpened::Ask,
            pause_analyzers_on_repeat: false,
        }
    }
}

/// `files`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Files {
    /// *Rename the session to the saved file name*.
    pub rename_on_save: bool,
    /// The *Default export folder*: the export dialog's folder until an
    /// export was made.
    pub export_dir: Option<PathBuf>,
    /// Recently opened or saved captures, newest first.
    pub recent_files: Vec<PathBuf>,
}

impl Default for Files {
    fn default() -> Self {
        Self { rename_on_save: true, export_dir: None, recent_files: Vec::new() }
    }
}

/// `decoders`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DecoderPrefs {
    /// An extra libsigrokdecode search path (read at start).
    pub extra_dir: Option<PathBuf>,
}

/// `history`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    /// Commands run from the palette, most recent first.
    pub recent_commands: Vec<String>,
    /// Recent time scales, newest first, distinct.
    pub recent_zooms: Vec<f64>,
}

/// `measurement_metrics`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeasurementMetrics {
    pub digital: Vec<crate::core::ranges::Metric>,
    pub analog: Vec<crate::core::ranges::Metric>,
}

impl Default for MeasurementMetrics {
    fn default() -> Self {
        Self {
            digital: crate::core::ranges::Metric::DIGITAL_DEFAULT.to_vec(),
            analog: crate::core::ranges::Metric::ANALOG_DEFAULT.to_vec(),
        }
    }
}

/// The settings file, `schema_version` 2: grouped to mirror the
/// Settings dialog; app state the dialog doesn't show stays at the top.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub schema_version: u32,
    /// The UI language (English by default).
    pub language: crate::l10n::Language,
    pub inspector: InspectorSettings,
    pub label_width: f32,
    /// The zoom tool's current mode.
    pub marquee_mode: MarqueeMode,
    pub appearance: Appearance,
    pub waveform: Waveform,
    pub time_labels: TimeLabels,
    pub capture: CapturePrefs,
    pub files: Files,
    pub decoders: DecoderPrefs,
    pub history: History,
    /// Last capture settings per device key.
    pub device_settings: BTreeMap<String, CaptureSettings>,
    /// Device key → the user's name for it.
    pub device_nicknames: BTreeMap<String, String>,
    pub measurement_metrics: MeasurementMetrics,
    pub last_dirs: LastDirs,
    /// The last import option values per libsigrok input module.
    pub import_options: BTreeMap<String, BTreeMap<String, String>>,
    /// The export choices remembered app-wide.
    pub export: ExportPrefs,
    /// One-time hints already shown (`"rows_scroll"`).
    pub hints_shown: Vec<String>,
    pub onboarding_done: bool,
    /// The last "driver:conn" entries that found a device.
    pub recent_connections: Vec<String>,
    /// Boards on USB serial ports that were connected once, as
    /// `(driver, USB serial number)`: they connect again without asking,
    /// on whatever port.
    pub known_serial_devices: Vec<(String, String)>,
}

impl AppSettings {
    /// Puts `path` first in the recent files.
    pub fn add_recent_file(&mut self, path: &Path) {
        let r = &mut self.files.recent_files;
        r.retain(|p| p != path);
        r.insert(0, path.to_owned());
        r.truncate(RECENT_FILES);
    }

    /// Puts `id` first in the recent commands.
    pub fn add_recent_command(&mut self, id: &str) {
        let r = &mut self.history.recent_commands;
        r.retain(|c| c != id);
        r.insert(0, id.to_owned());
        r.truncate(RECENT_COMMANDS);
    }

    /// Puts `scale` first, dropping an equal older entry.
    pub fn add_recent_zoom(&mut self, scale: f64) {
        let r = &mut self.history.recent_zooms;
        r.retain(|z| (z / scale - 1.0).abs() > 1e-9);
        r.insert(0, scale);
        r.truncate(RECENT_ZOOMS);
    }

    /// *Reset all settings*: everything back to its default except
    /// the remembered device settings, the nicknames and onboarding.
    pub fn reset_all(&mut self) {
        let keep = (
            std::mem::take(&mut self.device_settings),
            std::mem::take(&mut self.device_nicknames),
            self.onboarding_done,
        );
        *self = AppSettings::default();
        (self.device_settings, self.device_nicknames, self.onboarding_done) = keep;
    }

    /// The device's name: the user's nickname, else vendor + model.
    pub fn device_name(&self, d: &crate::backend::DeviceInfo) -> String {
        self.device_nicknames.get(&d.id.0).cloned().unwrap_or_else(|| d.display_name())
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            language: crate::l10n::Language::default(),
            inspector: InspectorSettings::default(),
            label_width: DEFAULT_LABEL_WIDTH,
            marquee_mode: MarqueeMode::X,
            appearance: Appearance::default(),
            waveform: Waveform::default(),
            time_labels: TimeLabels::default(),
            capture: CapturePrefs::default(),
            files: Files::default(),
            decoders: DecoderPrefs::default(),
            history: History::default(),
            device_settings: BTreeMap::new(),
            device_nicknames: BTreeMap::new(),
            measurement_metrics: MeasurementMetrics::default(),
            last_dirs: LastDirs::default(),
            import_options: BTreeMap::new(),
            export: Default::default(),
            hints_shown: Vec::new(),
            onboarding_done: false,
            recent_connections: Vec::new(),
            known_serial_devices: Vec::new(),
        }
    }
}

/// `<config dir>/logic-analyzer/settings.json`.
pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("logic-analyzer").join("settings.json"))
}

impl AppSettings {
    /// Loads `path`: defaults when it's missing. An unreadable file, or one
    /// of another `schema_version`, is renamed to a backup next to it and
    /// the defaults are used; the backup's path is returned.
    pub fn load(path: &Path) -> (Self, Option<PathBuf>) {
        let Ok(bytes) = std::fs::read(path) else { return (Self::default(), None) };
        let value: Option<serde_json::Value> = serde_json::from_slice(&bytes).ok();
        let version = value.as_ref().and_then(|v| v.get("schema_version")).and_then(|v| v.as_u64());
        let parsed = value.and_then(|v| serde_json::from_value::<AppSettings>(v).ok());
        match parsed {
            Some(mut s) if version == Some(SCHEMA_VERSION as u64) => {
                s.inspector.width = s.inspector.width.max(INSPECTOR_MIN_WIDTH);
                s.appearance.ui_scale = s.appearance.ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX);
                (s, None)
            }
            _ => {
                let tag = version.map_or_else(|| "invalid".to_owned(), |v| v.to_string());
                (Self::default(), backup(path, &tag))
            }
        }
    }

    /// Writes atomically: temp file, fsync, rename.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(tmp, path)
    }
}

/// Renames `path` to `settings.json.bak-‹tag›-‹YYYYMMDD-HHMMSS›`,
/// never over an earlier backup.
fn backup(path: &Path, tag: &str) -> Option<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = path.file_name()?.to_string_lossy().into_owned();
    let base = path.with_file_name(format!("{name}.bak-{tag}-{stamp}"));
    let target = std::iter::once(base.clone())
        .chain((2..).map(|n| PathBuf::from(format!("{}-{n}", base.display()))))
        .find(|p| !p.exists())?;
    std::fs::rename(path, &target).ok().map(|()| target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ChannelId;

    #[test]
    fn round_trip_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/settings.json");
        let mut s = AppSettings { label_width: 140.0, ..AppSettings::default() };
        s.device_settings.insert(
            "mock/mock-8".into(),
            CaptureSettings { sample_rate: 8_000_000, enabled: vec![ChannelId::digital(3)], ..Default::default() },
        );
        s.appearance.ui_scale = 1.3;
        s.save(&path).unwrap();
        assert_eq!(AppSettings::load(&path), (s, None));

        let v2 = r#"{"schema_version": 2, "label_width": 120, "future_key": 1, "inspector": {"width": 10},
            "waveform": {"trace_width": "bold"}}"#;
        std::fs::write(&path, v2).unwrap();
        let (s, backup) = AppSettings::load(&path);
        assert!(backup.is_none(), "unknown and missing keys don't reset");
        assert_eq!(s.label_width, 120.0);
        assert_eq!(s.inspector.width, INSPECTOR_MIN_WIDTH);
        assert_eq!(s.waveform.trace_width, crate::ui::waveform::TraceWidth::Bold);
        assert!(s.waveform.color_waveforms, "missing keys take defaults");

        let mut z = AppSettings::default();
        for v in [1e-3, 2e-3, 1e-3, 5e-3, 7e-3] {
            z.add_recent_zoom(v);
        }
        assert_eq!(z.history.recent_zooms, [7e-3, 5e-3, 1e-3], ": distinct, newest first, three");
    }

    /// Version 1 or unparsable files are backed up and reset.
    #[test]
    fn old_or_broken_files_are_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for (content, tag) in [(r#"{"schema_version": 1, "theme": "light"}"#, "1"), ("not json", "invalid")] {
            std::fs::write(&path, content).unwrap();
            let (s, backup) = AppSettings::load(&path);
            assert_eq!(s, AppSettings::default());
            let backup = backup.expect("backed up");
            assert!(!path.exists());
            assert_eq!(std::fs::read_to_string(&backup).unwrap(), content);
            let name = backup.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with(&format!("settings.json.bak-{tag}-")), "{name}");
        }
        // Two resets in the same second keep both backups.
        std::fs::write(&path, "x").unwrap();
        let first = AppSettings::load(&path).1.unwrap();
        std::fs::write(&path, "y").unwrap();
        let second = AppSettings::load(&path).1.unwrap();
        assert_ne!(first, second);
        assert!(first.exists() && second.exists());
    }

    /// Keeps the device settings and nicknames.
    #[test]
    fn reset_all_keeps_devices() {
        let mut s = AppSettings::default();
        s.appearance.ui_scale = 1.5;
        s.device_nicknames.insert("demo".into(), "Bench".into());
        s.device_settings.insert("demo".into(), CaptureSettings::default());
        s.reset_all();
        assert_eq!(s.appearance.ui_scale, 1.0);
        assert_eq!(s.device_nicknames["demo"], "Bench");
        assert!(s.device_settings.contains_key("demo"));
    }
}
