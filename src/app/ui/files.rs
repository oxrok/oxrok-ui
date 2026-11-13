//! Save, Save As, Open, Open Recent and drag and drop, the
//! unsaved-capture prompts () and the
//! read-only banner.

use super::dialogs::modal_frame;
use crate::app::file::{self, LoadedCapture, SessionMeta};
use crate::app::worker::SessionId;
use crate::app::{AppEvent, LaApp};
use crate::core::capture::CaptureState;
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::theme::ThemeExt;
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use crate::ui::widgets::Button;
use egui::{Context, Frame, Id, Margin, Modal, Rect, Sense, Ui, vec2};
use std::path::{Path, PathBuf};

/// Banner height.
pub(crate) const BANNER_H: f32 = 32.0;
const SAVE_TOAST: &str = "save";
const LOAD_TOAST: &str = "load";
const SAVED_TOAST: &str = "saved";

/// Something to do once the user has answered "Save Capture?" (or saved).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pending {
    /// A new session with the loaded file's settings.
    NewCaptureWithSettings,
    /// Quit the app.
    Quit,
    /// *Save Checked…*: save these sessions one after another, then quit.
    SaveThenQuit(Vec<SessionId>),
    /// Start recording over the capture.
    StartCapture,
    /// Close these sessions, in order.
    Close(Vec<SessionId>),
}

/// The modal asking about unsaved work, if open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Prompt {
    /// "Save Capture?" before closing `session`.
    SaveCapture { session: SessionId, then: Pending },
    /// "Save Captures?" when quitting; a checkbox per capture.
    Quit { checked: Vec<(SessionId, bool)> },
    /// A save is running.
    QuitWhileSaving,
    /// An export is running.
    QuitWhileExporting,
    /// "Replace Capture?" (opt-in).
    ReplaceCapture { dont_ask: bool },
    /// "Record on ‹file›?": a new session or this one.
    RecordOnOpened { remember: bool },
}

/// Results of the background save and load jobs.
pub(crate) enum FileEvent {
    Progress {
        toast: &'static str,
        heading: String,
        fraction: f32,
    },
    Saved {
        session: SessionId,
        result: Result<PathBuf, String>,
        fingerprint: u64,
        full: bool,
        then: Option<Pending>,
    },
    Picked(Vec<PathBuf>),
    Loaded(Box<Result<LoadedCapture, String>>),
    /// Content detection finished.
    Detected {
        path: PathBuf,
        id: Option<String>,
        candidates: Vec<String>,
    },
    /// An import finished.
    Imported(Box<Result<Imported, String>>),
}

/// An import's result, ready to become a session.
pub(crate) struct Imported {
    pub path: PathBuf,
    pub info: crate::backend::ImportedCapture,
    pub capture: crate::app::capture::SharedCapture,
    /// The module and whether it read with default options (toast).
    pub module: Option<(String, bool)>,
}

/// How a file is read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Route {
    Lacap,
    Session,
    /// An input module; `ask` opens the Import Options dialog first.
    Module {
        id: String,
        ask: bool,
    },
    /// Several modules (or none) claim the extension: ask libsigrok.
    Detect {
        candidates: Vec<String>,
    },
    Unsupported,
}

/// "Show in Folder" for a saved or exported file.
pub(crate) fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
    if let Err(e) = r {
        eprintln!("show in folder: {e}");
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

impl LaApp {
    /// The sessions quitting asks about, in display order.
    pub(crate) fn sessions_to_save(&self) -> Vec<SessionId> {
        self.sessions().iter().filter(|s| s.is_unsaved()).map(|s| s.id).collect()
    }

    pub(crate) fn run_pending(&mut self, p: Pending) {
        match p {
            Pending::NewCaptureWithSettings => self.new_capture_with_settings(),
            Pending::Quit => {
                self.ui.quit_confirmed = true;
                self.ui.ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            }
            Pending::SaveThenQuit(mut ids) => {
                if ids.is_empty() {
                    return self.run_pending(Pending::Quit);
                }
                let id = ids.remove(0);
                self.save_session(id, false, Some(Pending::SaveThenQuit(ids)));
            }
            Pending::Close(ids) => self.close_list(ids),
            Pending::StartCapture => {
                self.ui.replace_confirmed = true;
                let ctx = self.ui.ctx.clone();
                self.start_capture(&ctx);
            }
        }
    }

    /// *Don't Save*: close `id` now, then go on with the rest.
    fn dont_save(&mut self, id: SessionId, then: Pending) {
        match then {
            Pending::Close(mut ids) => {
                ids.retain(|x| *x != id);
                self.close_now(id);
                self.close_list(ids);
            }
            p => self.run_pending(p),
        }
    }

    /// *Save…*: save `id`, and close it only once the save completes.
    fn save_then(&mut self, id: SessionId, then: Pending) {
        if self.session_by_id(id).is_some_and(|s| s.is_unsaved()) {
            self.save_session(id, false, Some(then));
        } else {
            self.run_pending(then);
        }
    }

    // ---- save ----

    /// `Ctrl/Cmd+S` (`save_as` false) and `Ctrl/Cmd+Shift+S`, for the
    /// active session.
    pub(crate) fn save_capture(&mut self, save_as: bool, then: Option<Pending>) {
        if let Some(id) = self.active_id() {
            self.save_session(id, save_as, then);
        }
    }

    /// Saves session `id` (active or not), then runs `then`.
    pub(crate) fn save_session(&mut self, id: SessionId, save_as: bool, then: Option<Pending>) {
        if self.ui.saving {
            return;
        }
        let typed = if self.active_id() == Some(id) {
            self.ui.table.typed.clone()
        } else {
            self.session_by_id(id).map(|s| s.parked_ui.table.typed.clone()).unwrap_or_default()
        };
        let Some(s) = self.session_by_id(id) else { return };
        if s.state != CaptureState::Finished || !s.has_data() {
            self.toast(Toast::new(ToastKind::Info, tr("Record a capture before saving.")).id("save-nothing"));
            return;
        }
        let Some(meta) = SessionMeta::of(s, &typed) else { return };
        let Some(capture) = s.capture.clone() else { return };
        let fingerprint = meta.annotations.fingerprint();
        let target = (!save_as).then(|| s.file.clone()).flatten();
        let full = !(target.is_some() && s.samples_saved);
        let name = file::default_file_name(&s.name);
        let start = self.settings.last_dirs.capture.clone();
        let dialogs = self.dialogs.clone();
        let tx = self.events_tx.clone();
        let ctx = self.ui.ctx.clone();
        self.ui.saving = true;
        std::thread::Builder::new()
            .name("save".into())
            .spawn(move || {
                let send = |e| {
                    let _ = tx.send(AppEvent::File(e));
                    ctx.request_repaint();
                };
                let path = match target {
                    Some(p) => p,
                    None => {
                        let filters = [(tr("capture (*.oxrok)").to_owned(), vec![file::EXTENSION.to_owned()])];
                        let picked = dialogs.save_file(
                            tr("Choose the save location for the capture"),
                            start.as_deref(),
                            &name,
                            &filters,
                        );
                        match picked {
                            Some(p) => file::with_extension(p),
                            None => {
                                send(FileEvent::Saved {
                                    session: id,
                                    result: Err(String::new()),
                                    fingerprint,
                                    full,
                                    then,
                                });
                                return;
                            }
                        }
                    }
                };
                let heading = tr("Saving {}…").replacen("{}", &file_name(&path), 1);
                let result = if full {
                    file::save(&path, &meta, &capture, &mut |f| {
                        send(FileEvent::Progress { toast: SAVE_TOAST, heading: heading.clone(), fraction: f });
                        true
                    })
                } else {
                    file::save_annotations(&path, &meta)
                };
                let result = result.map(|()| path).map_err(|e| e.to_string());
                send(FileEvent::Saved { session: id, result, fingerprint, full, then });
            })
            .expect("spawn save thread");
    }

    fn on_saved(
        &mut self,
        id: SessionId,
        result: Result<PathBuf, String>,
        fingerprint: u64,
        full: bool,
        then: Option<Pending>,
    ) {
        self.ui.saving = false;
        self.toasts.dismiss(SAVE_TOAST);
        match result {
            // The user cancelled the save dialog: nothing happens, and
            // nothing that waited on the save either.
            Err(e) if e.is_empty() => {}
            Err(e) => self.toast(Toast::new(ToastKind::Error, e).heading(tr("Could not save the capture"))),
            Ok(path) => {
                let rename = self.settings.files.rename_on_save;
                if let Some(s) = self.session_by_id_mut(id) {
                    s.file = Some(path.clone());
                    s.saved = Some(fingerprint);
                    if full {
                        s.samples_saved = true;
                    }
                    if rename && let Some(stem) = path.file_stem() {
                        s.name = stem.to_string_lossy().into_owned();
                        s.saved = Some(file::Annotations::of(s).fingerprint());
                    }
                }
                if let Some(dir) = path.parent() {
                    self.settings.last_dirs.capture = Some(dir.to_owned());
                }
                self.settings.add_recent_file(&path);
                self.settings_dirty_at.get_or_insert(self.clock.now());
                self.ui.reveal.insert(SAVED_TOAST.into(), path.clone());
                self.toast(
                    Toast::new(ToastKind::Success, tr("Saved {}").replacen("{}", &file_name(&path), 1))
                        .id(SAVED_TOAST)
                        .action("Show in Folder"),
                );
                if let Some(p) = then {
                    self.run_pending(p);
                }
            }
        }
    }

    // ---- open ----

    /// `Ctrl/Cmd+O` and the start screen's button.
    pub(crate) fn open_capture(&mut self) {
        if self.session.as_ref().is_some_and(|s| s.state.is_busy()) {
            self.toast(Toast::new(ToastKind::Info, tr("Stop recording before opening a capture.")));
            return;
        }
        self.pick_files_to_open();
    }

    /// Filters: everything supported, `.oxrok`, `.sr` (with
    /// libsigrok), one per import format, and all files.
    fn open_filters(&self) -> Vec<crate::app::dialogs::Filter> {
        let sigrok = !self.input_formats.is_empty();
        let mut all = vec![file::EXTENSION.to_owned()];
        if sigrok {
            all.push("sr".into());
        }
        for f in &self.input_formats {
            all.extend(f.extensions.iter().cloned());
        }
        all.sort();
        all.dedup();
        let mut v = vec![
            (tr("All supported files").to_owned(), all),
            (tr("capture (*.oxrok)").to_owned(), vec![file::EXTENSION.into()]),
        ];
        if sigrok {
            v.push((tr("sigrok session (*.sr)").into(), vec!["sr".into()]));
        }
        for f in self.input_formats.iter().filter(|f| !f.extensions.is_empty()) {
            let globs: Vec<String> = f.extensions.iter().map(|e| format!("*.{e}")).collect();
            v.push((format!("{} ({})", f.description, globs.join(", ")), f.extensions.clone()));
        }
        v.push((tr("All files").into(), vec!["*".into()]));
        v
    }

    fn pick_files_to_open(&mut self) {
        let dialogs = self.dialogs.clone();
        let start = self.settings.last_dirs.capture.clone();
        let tx = self.events_tx.clone();
        let ctx = self.ui.ctx.clone();
        let filters = self.open_filters();
        std::thread::Builder::new()
            .name("open-dialog".into())
            .spawn(move || {
                let files = dialogs.open_files(tr("Pick a capture file to open"), start.as_deref(), &filters);
                let _ = tx.send(AppEvent::File(FileEvent::Picked(files)));
                ctx.request_repaint();
            })
            .expect("spawn open dialog thread");
    }

    /// Opens files picked, dropped or chosen from Recent, each in a new
    /// session.
    pub(crate) fn open_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if self.session.as_ref().is_some_and(|s| s.state.is_busy()) {
            self.toast(Toast::new(ToastKind::Info, tr("Stop recording before opening a capture.")));
            return;
        }
        self.load_files(paths);
    }

    /// How this build reads `path`.
    pub(crate) fn route(&self, path: &Path) -> Route {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ext == file::EXTENSION {
            return Route::Lacap;
        }
        if self.input_formats.is_empty() {
            return Route::Unsupported;
        }
        if ext == "sr" {
            return Route::Session;
        }
        let claims: Vec<&crate::backend::InputFormat> =
            self.input_formats.iter().filter(|f| f.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext))).collect();
        match claims.as_slice() {
            [f] if !f.needs_options() => Route::Module { id: f.id.clone(), ask: false },
            [] if !ext.is_empty() => Route::Unsupported,
            _ => Route::Detect { candidates: claims.iter().map(|f| f.id.clone()).collect() },
        }
    }

    /// Whether this build may be able to open `path`.
    pub(crate) fn can_open(&self, path: &Path) -> bool {
        self.route(path) != Route::Unsupported
    }

    fn load_files(&mut self, paths: Vec<PathBuf>) {
        let (ok, bad): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| self.can_open(p));
        for p in bad {
            self.toast(
                Toast::new(
                    ToastKind::Warning,
                    tr("{} isn't a supported capture file").replacen("{}", &file_name(&p), 1),
                )
                .id(format!("unsupported-{}", p.display()))
                .action("Supported formats…"),
            );
        }
        // Queued and loaded one after another.
        self.ui.open_queue.extend(ok);
        if !self.ui.loading && self.ui.import.is_none() {
            self.open_next_queued();
        }
    }

    /// Starts loading the next queued file, if any.
    pub(crate) fn open_next_queued(&mut self) {
        if self.ui.loading || self.ui.import.is_some() {
            return;
        }
        let Some(path) = self.ui.open_queue.pop_front() else { return };
        match self.route(&path) {
            Route::Lacap => {}
            Route::Session => {
                return self.start_import(path, crate::backend::ImportFormat::Session, Default::default());
            }
            Route::Module { id, ask: false } => {
                let options = self.saved_import_options(&id);
                return self.start_import(path, crate::backend::ImportFormat::Module(id), options);
            }
            Route::Module { id, ask: true } => return self.open_import_dialog(path, &id, false),
            Route::Detect { candidates } => return self.detect_then_import(path, candidates),
            Route::Unsupported => return self.open_next_queued(),
        }
        self.ui.loading = true;
        let tx = self.events_tx.clone();
        let ctx = self.ui.ctx.clone();
        std::thread::Builder::new()
            .name("load".into())
            .spawn(move || {
                let heading = tr("Reading {}…").replacen("{}", &file_name(&path), 1);
                let r = file::load(&path, &mut |f| {
                    let _ = tx.send(AppEvent::File(FileEvent::Progress {
                        toast: LOAD_TOAST,
                        heading: heading.clone(),
                        fraction: f,
                    }));
                    ctx.request_repaint();
                    true
                })
                .map_err(|e| format!("{}: {e}", file_name(&path)));
                let _ = tx.send(AppEvent::File(FileEvent::Loaded(Box::new(r))));
                ctx.request_repaint();
            })
            .expect("spawn load thread");
    }

    fn on_loaded(&mut self, r: Result<LoadedCapture, String>) {
        self.ui.loading = false;
        self.toasts.dismiss(LOAD_TOAST);
        let l = match r {
            Ok(l) => l,
            Err(e) => {
                self.toast(Toast::new(ToastKind::Error, e).heading(tr("Could not open the capture")));
                return self.open_next_queued();
            }
        };
        let path = l.path.clone();
        // A new session named after the file.
        let id = self.next_id();
        let width = self.session.as_ref().map_or(800.0, |s| s.view.state().width_px);
        let mut s = file::session_from(id, l, width);
        // Named after the file.
        if let Some(stem) = path.file_stem() {
            s.name = stem.to_string_lossy().into_owned();
        }
        s.view.speed_scale = self.animation_speed;
        s.set_probe(self.profiles.select(&s.device));
        let typed = std::mem::take(&mut s.parked_ui.table.typed);
        self.add_session(s);
        self.ui.table.typed = typed;
        if let Some(dir) = path.parent() {
            self.settings.last_dirs.capture = Some(dir.to_owned());
        }
        self.settings.add_recent_file(&path);
        self.settings_dirty_at.get_or_insert(self.clock.now());
        self.toast(Toast::new(ToastKind::Success, tr("Opened {}").replacen("{}", &file_name(&path), 1)).id("opened"));
        self.open_next_queued();
    }

    /// A connected device for the opened file: the
    /// same device, else the most recent one of the same driver.
    pub(crate) fn matching_device(&self) -> Option<crate::backend::DeviceInfo> {
        let s = self.session.as_ref()?;
        let devices = self.devices.as_ref()?;
        devices
            .iter()
            .find(|d| d.id == s.device.id)
            .or_else(|| devices.iter().rev().find(|d| d.driver == s.device.driver))
            .cloned()
    }

    pub(crate) fn toast_no_matching_device(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        let text = if s.is_import() {
            super::graph::import_cannot_record(s)
        } else {
            tr("Connect a {} to record with these settings.").replacen("{}", &s.device.display_name(), 1)
        };
        self.toast(Toast::new(ToastKind::Info, text).id("no-matching-device"));
    }

    /// *Replace Data* / the banner's *Record Here*: this session
    /// binds to the matching device and records; it keeps its file, name,
    /// notes, rows and analyzers.
    pub(crate) fn record_here(&mut self) {
        let Some(device) = self.matching_device() else {
            self.toast_no_matching_device();
            return;
        };
        let Some(s) = self.session.as_mut() else { return };
        s.device = device.clone();
        s.opened_from = None;
        s.caps = None;
        s.open_error = None;
        let id = s.id;
        self.ui.start_when_opened = true;
        self.worker.send(crate::app::worker::WorkerCmd::Open { session: id, device: device.id });
    }

    /// A new session on a connected device of the same driver,
    /// with the file's settings, rows and analyzers; it records once the
    /// device is open.
    pub(crate) fn new_capture_with_settings(&mut self) {
        let Some(device) = self.matching_device() else {
            self.toast_no_matching_device();
            return;
        };
        let Some(s) = self.session.as_ref() else { return };
        let (src, width) = (s.id, s.view.state().width_px);
        if let Some(why) = self.create_blocked() {
            self.toast(Toast::new(ToastKind::Info, why).id("session-blocked"));
            return;
        }
        self.open_session_from(&device, width, Some(src));
        self.ui.start_when_opened = true;
        self.mark_settings_dirty();
    }

    // ---- imports ----

    /// The last option values used with `module`.
    pub(crate) fn saved_import_options(
        &self,
        module: &str,
    ) -> std::collections::BTreeMap<String, crate::backend::OptionValue> {
        let Some(saved) = self.settings.import_options.get(module) else { return Default::default() };
        let Some(f) = self.input_formats.iter().find(|f| f.id == module) else { return Default::default() };
        f.options
            .iter()
            .filter_map(|o| saved.get(&o.id).and_then(|t| super::import::parse_value(o, t)).map(|v| (o.id.clone(), v)))
            .collect()
    }

    fn detect_then_import(&mut self, path: PathBuf, candidates: Vec<String>) {
        self.ui.loading = true;
        let (backend, tx, ctx) = (self.backend.clone(), self.events_tx.clone(), self.ui.ctx.clone());
        std::thread::Builder::new()
            .name("detect".into())
            .spawn(move || {
                let id = backend.detect_format(&path);
                let _ = tx.send(AppEvent::File(FileEvent::Detected { path, id, candidates }));
                ctx.request_repaint();
            })
            .expect("spawn detect thread");
    }

    fn on_detected(&mut self, path: PathBuf, id: Option<String>, candidates: Vec<String>) {
        self.ui.loading = false;
        let needs = |id: &str| self.input_formats.iter().any(|f| f.id == id && f.needs_options());
        match id {
            Some(id) if !needs(&id) => {
                let options = self.saved_import_options(&id);
                self.start_import(path, crate::backend::ImportFormat::Module(id), options);
            }
            // Known but needs options, or unknown among several: ask, with
            // the Format dropdown when the format isn't certain.
            Some(id) => self.open_import_dialog(path, &id, false),
            None if !candidates.is_empty() => self.open_import_dialog(path, &candidates[0], true),
            None => {
                self.toast(
                    Toast::new(
                        ToastKind::Warning,
                        tr("{} isn't a supported capture file").replacen("{}", &file_name(&path), 1),
                    )
                    .id(format!("unsupported-{}", path.display()))
                    .action("Supported formats…"),
                );
                self.open_next_queued();
            }
        }
    }

    /// On a background thread, into an [`ImportSink`].
    pub(crate) fn start_import(
        &mut self,
        path: PathBuf,
        format: crate::backend::ImportFormat,
        options: std::collections::BTreeMap<String, crate::backend::OptionValue>,
    ) {
        self.ui.loading = true;
        let module = match &format {
            crate::backend::ImportFormat::Module(id) => Some((id.clone(), options.is_empty())),
            crate::backend::ImportFormat::Session => None,
        };
        let (backend, tx, ctx) = (self.backend.clone(), self.events_tx.clone(), self.ui.ctx.clone());
        std::thread::Builder::new()
            .name("import".into())
            .spawn(move || {
                let capture = crate::app::capture::SharedCapture::default();
                let sink = Box::new(file::ImportSink { data: capture.clone() });
                let heading = tr("Reading {}…").replacen("{}", &file_name(&path), 1);
                let (ptx, pctx) = (tx.clone(), ctx.clone());
                let progress = Box::new(move |f: f32| {
                    let _ = ptx.send(AppEvent::File(FileEvent::Progress {
                        toast: LOAD_TOAST,
                        heading: heading.clone(),
                        fraction: f,
                    }));
                    pctx.request_repaint();
                });
                let r = backend
                    .import_file(&path, format, &options, sink, progress)
                    .map_err(|e| format!("{}: {e}", file_name(&path)))
                    .and_then(|info| {
                        let failed = capture.read().end.as_ref().and_then(|e| match e {
                            crate::backend::EndReason::Error(m) => Some(m.clone()),
                            _ => None,
                        });
                        match failed {
                            Some(m) => Err(format!("{}: {m}", file_name(&path))),
                            None if capture.read().sample_range().is_none() => {
                                Err(tr("{}: the file holds no samples").replacen("{}", &file_name(&path), 1))
                            }
                            None => Ok(Imported { path: path.clone(), info, capture, module }),
                        }
                    });
                let _ = tx.send(AppEvent::File(FileEvent::Imported(Box::new(r))));
                ctx.request_repaint();
            })
            .expect("spawn import thread");
    }

    fn on_imported(&mut self, r: Result<Imported, String>) {
        self.ui.loading = false;
        self.toasts.dismiss(LOAD_TOAST);
        let im = match r {
            Ok(im) => im,
            Err(e) => {
                self.toast(Toast::new(ToastKind::Error, e).heading(tr("Could not open the capture")));
                return self.open_next_queued();
            }
        };
        let id = self.next_id();
        let width = self.session.as_ref().map_or(800.0, |s| s.view.state().width_px);
        let mut s = file::imported_session(id, &im.path, im.info, im.capture, width);
        s.view.speed_scale = self.animation_speed;
        self.add_session(s);
        self.fit_view();
        if let Some(dir) = im.path.parent() {
            self.settings.last_dirs.capture = Some(dir.to_owned());
        }
        self.settings.add_recent_file(&im.path);
        self.settings_dirty_at.get_or_insert(self.clock.now());
        let mut t =
            Toast::new(ToastKind::Success, tr("Opened {}").replacen("{}", &file_name(&im.path), 1)).id("opened");
        // Read with defaults → offer to change them.
        if let Some((module, true)) = &im.module
            && self.input_formats.iter().any(|f| f.id == *module && !f.options.is_empty())
        {
            self.ui.last_import = Some((im.path.clone(), module.clone()));
            t = t.action("Change Import Options…");
        }
        self.toast(t);
        self.open_next_queued();
    }

    // ---- events ----

    pub(crate) fn handle_file_event(&mut self, e: FileEvent) {
        match e {
            FileEvent::Progress { toast, heading, fraction } => {
                let t = Toast::new(ToastKind::Info, format!("{} %", (fraction * 100.0).round() as u32))
                    .heading(heading)
                    .id(toast)
                    .persistent()
                    .progress(fraction);
                self.toast(t);
            }
            FileEvent::Saved { session, result, fingerprint, full, then } => {
                self.on_saved(session, result, fingerprint, full, then)
            }
            FileEvent::Picked(files) => self.load_files(files),
            FileEvent::Loaded(r) => self.on_loaded(*r),
            FileEvent::Detected { path, id, candidates } => self.on_detected(path, id, candidates),
            FileEvent::Imported(r) => self.on_imported(*r),
        }
    }

    /// Toast actions that belong to files.
    pub(crate) fn file_toast_action(&mut self, id: &str, action: &str) -> bool {
        match action {
            // 's toast names the platform's file manager.
            "Show in Folder" | "Show in Finder" | "Show in Explorer" => {
                if let Some(p) = self.ui.reveal.get(id) {
                    reveal(p);
                }
                true
            }
            "Supported formats…" => {
                self.toasts.dismiss(id);
                let text = self.supported_formats_text();
                self.toast(Toast::new(ToastKind::Info, text).heading(tr("Supported formats")).id("formats"));
                true
            }
            "Change Import Options…" => {
                self.toasts.dismiss(id);
                if let Some((path, module)) = self.ui.last_import.clone() {
                    self.open_import_dialog(path, &module, false);
                }
                true
            }
            "New Capture with These Settings" => {
                self.toasts.dismiss(id);
                self.run_pending(Pending::NewCaptureWithSettings);
                true
            }
            _ => false,
        }
    }

    // ---- quit ----

    /// Called every frame: intercepts the window's close request.
    pub(crate) fn intercept_quit(&mut self, ctx: &Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.ui.quit_confirmed {
            return;
        }
        // The in-progress reasons first.
        let unsaved = self.sessions_to_save();
        let prompt = if self.ui.exporting > 0 {
            Some(Prompt::QuitWhileExporting)
        } else if self.ui.saving {
            Some(Prompt::QuitWhileSaving)
        } else if !unsaved.is_empty() {
            Some(Prompt::Quit { checked: unsaved.into_iter().map(|id| (id, true)).collect() })
        } else {
            None
        };
        if let Some(p) = prompt {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // Quitting while recording stops the capture first.
            self.stop_capture();
            if self.ui.prompt.is_none() {
                self.ui.prompt = Some(p);
            }
        } else {
            self.stop_capture();
        }
    }

    // ---- the prompts ----

    pub(crate) fn prompt_ui(&mut self, ctx: &Context) {
        let Some(prompt) = self.ui.prompt.clone() else { return };
        // The session asked about, which may be in the background.
        let asked = match &prompt {
            Prompt::SaveCapture { session, .. } => self.session_by_id(*session),
            _ => None,
        };
        let save_body = asked.map(|s| tr("{} has a capture that hasn't been saved.").replacen("{}", &s.name, 1));
        let quit_names: std::collections::HashMap<SessionId, String> = self
            .sessions()
            .iter()
            .map(|s| {
                let mark = if s.is_unsaved() { "• " } else { "  " };
                (s.id, format!("{mark}{}", s.name))
            })
            .collect();
        let theme = ctx.la_theme();
        let body = |ui: &mut Ui, text: &str| {
            ui.label(egui::RichText::new(text).text_style(Text::Prose.style()).color(theme.color(Token::Ink2)));
        };
        let mut answer: Option<&'static str> = None;
        let mut next = prompt.clone();
        let (title, width) = match &prompt {
            Prompt::SaveCapture { .. } => (tr("Save Capture?"), 460.0),
            Prompt::Quit { .. } => (tr("Save Captures?"), 460.0),
            Prompt::QuitWhileSaving => (tr("Saving the capture…"), 460.0),
            Prompt::QuitWhileExporting => (tr("Export in progress!"), 460.0),
            Prompt::ReplaceCapture { .. } => (tr("Replace Capture?"), 460.0),
            Prompt::RecordOnOpened { .. } => ("", 480.0),
        };
        let opened = self.session.as_ref().and_then(|s| s.opened_from.clone());
        let opened_name = opened.as_deref().map(file_name).unwrap_or_default();
        let record_title = tr("Record on {}?").replacen("{}", &opened_name, 1);
        let title = if matches!(prompt, Prompt::RecordOnOpened { .. }) { record_title.as_str() } else { title };
        let overwrites = opened.as_deref().is_some_and(|p| p.extension().is_some_and(|e| e == file::EXTENSION));
        let resp = Modal::new(Id::unique("unsaved_prompt")).frame(modal_frame(ctx).inner_margin(Margin::ZERO)).show(
            ctx,
            |ui| {
                ui.set_width(width);
                Frame::new().inner_margin(Margin::same(32)).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(title).text_style(Text::Display.style()).color(theme.color(Token::Ink1)),
                    );
                });
                ui.separator();
                Frame::new().inner_margin(Margin::symmetric(32, 20)).show(ui, |ui| match &mut next {
                    Prompt::SaveCapture { .. } => body(ui, save_body.as_deref().unwrap_or_default()),
                    Prompt::Quit { checked } => {
                        let n = checked.len();
                        body(
                            ui,
                            &if n == 1 {
                                tr("1 capture hasn't been saved:").to_owned()
                            } else {
                                tr("{} captures haven't been saved:").replacen("{}", &n.to_string(), 1)
                            },
                        );
                        ui.add_space(8.0);
                        for (id, on) in checked.iter_mut() {
                            let label = quit_names.get(id).cloned().unwrap_or_default();
                            ui.add(crate::ui::widgets::Checkbox::new(on, &label));
                        }
                    }
                    Prompt::QuitWhileSaving => {
                        body(ui, tr("Quitting now will corrupt the in-progress save. Quit anyway?"))
                    }
                    Prompt::QuitWhileExporting => {
                        body(ui, tr("Quitting now will corrupt the in-progress export. Quit anyway?"))
                    }
                    Prompt::ReplaceCapture { dont_ask } => {
                        body(ui, tr("The current capture hasn't been saved. Starting a new recording discards it."));
                        ui.add_space(12.0);
                        ui.add(crate::ui::widgets::Checkbox::new(dont_ask, tr("Don't ask again")));
                    }
                    Prompt::RecordOnOpened { remember } => {
                        body(ui, &tr("This session shows the data from {}.").replacen("{}", &opened_name, 1));
                        let then = if overwrites {
                            tr("With Replace Data, saving afterwards overwrites {}.").replacen("{}", &opened_name, 1)
                        } else {
                            tr("With Replace Data, saving afterwards asks where to save.").to_owned()
                        };
                        body(ui, &then);
                        ui.add_space(12.0);
                        ui.add(crate::ui::widgets::Checkbox::new(remember, tr("Remember my choice")));
                    }
                });
                ui.separator();
                Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let any_checked = matches!(&next, Prompt::Quit { checked } if checked.iter().any(|c| c.1));
                        let buttons: &[(&'static str, bool)] = match &next {
                            Prompt::SaveCapture { .. } => &[("Cancel", false), ("Don't Save", false), ("Save…", true)],
                            Prompt::Quit { .. } if any_checked => {
                                &[("Cancel", false), ("Don't Save", false), ("Save Checked…", true)]
                            }
                            Prompt::Quit { .. } => &[("Cancel", false), ("Don't Save", true)],
                            Prompt::QuitWhileSaving | Prompt::QuitWhileExporting => {
                                &[("Yes, Exit", false), ("Don't Exit", true)]
                            }
                            Prompt::ReplaceCapture { .. } => &[("Cancel", false), ("Save…", false), ("Replace", true)],
                            Prompt::RecordOnOpened { .. } => {
                                &[("Cancel", false), ("New Session", false), ("Replace Data", true)]
                            }
                        };
                        if ui.add(Button::secondary(tr(buttons[0].0))).clicked() {
                            answer = Some(buttons[0].0);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            for (label, primary) in buttons[1..].iter().rev() {
                                let shown = tr(label);
                                let b = if *primary {
                                    if *label == "Replace" {
                                        Button::destructive(shown)
                                    } else {
                                        Button::primary(shown)
                                    }
                                } else {
                                    Button::secondary(shown)
                                };
                                if ui.add(b).clicked() {
                                    answer = Some(label);
                                }
                                ui.add_space(8.0);
                            }
                        });
                    });
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    answer = match &next {
                        Prompt::SaveCapture { .. } => Some("Save…"),
                        Prompt::Quit { checked } if checked.iter().any(|c| c.1) => Some("Save Checked…"),
                        Prompt::Quit { .. } => Some("Don't Save"),
                        Prompt::QuitWhileSaving | Prompt::QuitWhileExporting => Some("Don't Exit"),
                        Prompt::ReplaceCapture { .. } => Some("Replace"),
                        Prompt::RecordOnOpened { .. } => Some("Replace Data"),
                    };
                }
            },
        );
        if resp.should_close() && answer.is_none() {
            answer = Some("Cancel");
        }
        let Some(a) = answer else {
            self.ui.prompt = Some(next);
            return;
        };
        self.ui.prompt = None;
        match (next, a) {
            (_, "Cancel" | "Don't Exit") => {}
            (Prompt::SaveCapture { session, then }, "Don't Save") => self.dont_save(session, then),
            (Prompt::SaveCapture { session, then }, _) => self.save_then(session, then),
            (Prompt::Quit { .. }, "Don't Save") => self.run_pending(Pending::Quit),
            (Prompt::Quit { checked }, _) => {
                let ids = checked.into_iter().filter(|c| c.1).map(|c| c.0).collect();
                self.run_pending(Pending::SaveThenQuit(ids));
            }
            (Prompt::QuitWhileSaving | Prompt::QuitWhileExporting, _) => {
                // The in-progress reason is answered; the unsaved list comes next.
                let unsaved = self.sessions_to_save();
                if unsaved.is_empty() || self.ui.saving || self.ui.exporting > 0 {
                    self.run_pending(Pending::Quit);
                } else {
                    self.ui.prompt = Some(Prompt::Quit { checked: unsaved.into_iter().map(|id| (id, true)).collect() });
                }
            }
            (Prompt::RecordOnOpened { remember }, label) => {
                let replace = label == "Replace Data";
                if remember {
                    self.settings.capture.record_on_opened = if replace {
                        crate::app::settings::RecordOnOpened::Replace
                    } else {
                        crate::app::settings::RecordOnOpened::NewSession
                    };
                    self.settings_dirty_at.get_or_insert(self.clock.now());
                }
                if replace {
                    self.record_here();
                } else {
                    self.run_pending(Pending::NewCaptureWithSettings);
                }
            }
            (Prompt::ReplaceCapture { dont_ask }, label) => {
                if dont_ask {
                    self.settings.capture.ask_replace_unsaved = false;
                    self.settings_dirty_at.get_or_insert(self.clock.now());
                }
                if label == "Save…" {
                    self.save_capture(false, Some(Pending::StartCapture));
                } else {
                    self.run_pending(Pending::StartCapture);
                }
            }
        }
    }

    // ---- banner ----

    pub(crate) fn file_banner(&mut self, ui: &mut Ui, rect: Rect) {
        let Some(path) = self.session.as_ref().and_then(|s| s.opened_from.clone()) else { return };
        let theme = ui.la_theme();
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, theme.color(Token::Card));
        p.line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, theme.color(Token::Seam)));
        let text = tr("Opened from {}").replacen("{}", &file_name(&path), 1);
        let info = ui.interact(rect, ui.make_persistent_id("file-banner"), Sense::hover());
        info.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));

        // What Record will do. The right side is laid out first;
        // the file name gets what is left. Short labels when it's tight.
        use crate::app::settings::RecordOnOpened as R;
        let choices = [
            (R::Ask, tr("I will decide later"), tr("Decide later")),
            (R::Replace, tr("Will record here"), tr("Record here")),
            (R::NewSession, tr("New capture with these settings"), tr("New capture")),
        ];
        let font = Text::CaptionMed.font_id();
        let width_of =
            |s: &str| ui.painter().layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE).size().x + 24.0;
        let full: f32 = choices.iter().map(|c| width_of(c.1)).sum::<f32>() + 140.0;
        let compact = full > rect.width() - 220.0;

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(12.0, 3.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        // Closes into a toolbar icon, for this session only.
        let hide = super::ranges::icon_button(&mut child, tr("Hide banner"), crate::ui::icons::chevron_up)
            .on_hover_text(tr("Hide banner"))
            .clicked();
        child.add_space(8.0);
        let current =
            self.session.as_ref().and_then(|s| s.record_choice).unwrap_or(self.settings.capture.record_on_opened);
        let mut picked = None;
        // An imported file has no device to record with: no choices.
        if !self.session.as_ref().is_some_and(|s| s.is_import()) {
            for (choice, label, short) in choices {
                let shown = if compact { short } else { label };
                let resp = choice_pill(&mut child, shown, label, current == choice);
                let resp = if compact { resp.on_hover_text(label) } else { resp };
                if resp.clicked() {
                    picked = Some(choice);
                }
                child.add_space(4.0);
            }
            child.add_space(4.0);
            child.label(egui::RichText::new(tr("Record:")).color(theme.color(Token::Ink4)));
        }
        let right = child.min_rect().left();
        // A choice closes the banner for this session (the toolbar icon
        // brings it back).
        if let Some(s) = self.session.as_mut() {
            if let Some(c) = picked {
                s.record_choice = Some(c);
            }
            s.banner_closed |= hide || picked.is_some();
        }

        // The file name, cut with "…" to the space left.
        let room = (right - 12.0 - (rect.left() + 16.0)).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(text, Text::Prose.font_id(), theme.color(Token::Ink2));
        job.wrap = egui::text::TextWrapping {
            max_width: room,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = ui.painter().layout_job(job);
        p.galley(
            egui::pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0),
            galley,
            egui::Color32::PLACEHOLDER,
        );
    }

    /// The collapsed banner, a file icon right of the Inspector
    /// toggle, which holds the toolbar's left end.
    pub(crate) fn file_banner_icon(&mut self, ui: &mut Ui, l: &super::graph::Layout) {
        let Some(path) = self.session.as_ref().and_then(|s| s.opened_from.clone()) else { return };
        let r =
            Rect::from_min_size(l.toolbar.left_top() + vec2(super::graph::CAPTURE_ITEM_W + 8.0, 3.0), vec2(24.0, 24.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let choice = match self.session.as_ref().and_then(|s| s.record_choice) {
            Some(crate::app::settings::RecordOnOpened::NewSession) => tr("Record: new capture with these settings. "),
            Some(crate::app::settings::RecordOnOpened::Replace) => tr("Record: will record here. "),
            Some(crate::app::settings::RecordOnOpened::Ask) => tr("Record: you will decide then. "),
            None => "",
        };
        let tip = tr("Opened from {}. {}Click to show the banner.")
            .replacen("{}", &file_name(&path), 1)
            .replacen("{}", choice, 1);
        if super::ranges::icon_button(&mut child, tr("Opened from a file"), crate::ui::icons::file)
            .on_hover_text(tip)
            .clicked()
            && let Some(s) = self.session.as_mut()
        {
            s.banner_closed = false;
        }
    }

    /// The formats this build opens, for the toast's action.
    pub(crate) fn supported_formats_text(&self) -> String {
        let mut v = vec![".oxrok".to_owned()];
        if !self.input_formats.is_empty() {
            v.push(tr("sigrok session (.sr)").into());
        }
        for f in &self.input_formats {
            let exts: Vec<String> = f.extensions.iter().map(|e| format!(".{e}")).collect();
            v.push(if exts.is_empty() {
                f.description.clone()
            } else {
                format!("{} ({})", f.description, exts.join(", "))
            });
        }
        v.join(" · ")
    }

    // ---- drag and drop ----

    pub(crate) fn drop_files(&mut self, ctx: &Context) {
        let (hovered, dropped) = ctx.input(|i| {
            (
                i.raw.hovered_files.iter().filter_map(|f| f.path.clone()).collect::<Vec<_>>(),
                i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect::<Vec<_>>(),
            )
        });
        if !dropped.is_empty() {
            // Files dropped on Install Firmware are what it installs from.
            if self.ui.install_firmware.is_some() {
                self.on_firmware_input(dropped);
                return;
            }
            self.open_paths(dropped);
            return;
        }
        let n = ctx.input(|i| i.raw.hovered_files.len());
        if n == 0 {
            return;
        }
        let ok: Vec<&PathBuf> = hovered.iter().filter(|p| self.can_open(p)).collect();
        let text = match (ok.len(), hovered.len()) {
            (1, _) => tr("Drop to open {}").replacen("{}", &file_name(ok[0]), 1),
            (0, 0) => tr("Drop capture files to open").to_owned(),
            (0, _) => tr("{} can't be opened").replacen("{}", &file_name(&hovered[0]), 1),
            (k, _) => tr("Drop to open {} captures (each in a new session)").replacen("{}", &k.to_string(), 1),
        };
        let bad: Vec<String> = hovered.iter().filter(|p| !self.can_open(p)).map(|p| file_name(p)).collect();
        let theme = ctx.la_theme();
        let screen = ctx.content_rect();
        let layer = egui::LayerId::new(egui::Order::Foreground, Id::unique("drop-overlay"));
        let p = ctx.layer_painter(layer);
        p.rect_filled(screen, 0.0, theme.color(Token::Canvas).gamma_multiply(0.6));
        let inner = screen.shrink(24.0);
        let stroke = egui::Stroke::new(2.0, theme.color(Token::Ink3));
        for side in [
            [inner.left_top(), inner.right_top()],
            [inner.right_top(), inner.right_bottom()],
            [inner.right_bottom(), inner.left_bottom()],
            [inner.left_bottom(), inner.left_top()],
        ] {
            p.add(egui::Shape::dashed_line(&side, stroke, 10.0, 6.0));
        }
        p.text(inner.center(), egui::Align2::CENTER_CENTER, text, Text::Headline.font_id(), theme.color(Token::Ink1));
        if !ok.is_empty() && !bad.is_empty() {
            p.text(
                inner.center() + vec2(0.0, 28.0),
                egui::Align2::CENTER_CENTER,
                tr("{} can't be opened").replacen("{}", &bad.join(", "), 1),
                Text::Prose.font_id(),
                theme.color(Token::Ink3),
            );
        }
    }
}

/// One of the banner's choices, a pill showing `label`, named
/// `name` for accessibility; selected is filled.
fn choice_pill(ui: &mut Ui, label: &str, name: &str, on: bool) -> egui::Response {
    let theme = ui.la_theme();
    let font = Text::CaptionMed.font_id();
    let color = theme.color(if on { Token::InkOnFlare } else { Token::Ink2 });
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    let size = vec2(galley.size().x + 20.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, name));
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 11.0, theme.color(Token::Mark));
    } else {
        let c = if resp.hovered() { Token::RingHover } else { Token::Seam };
        p.rect_stroke(rect, 11.0, egui::Stroke::new(1.0, theme.color(c)), egui::StrokeKind::Inside);
    }
    p.galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}
