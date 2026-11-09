//! What the registry's commands do: `dispatch`, whether a
//! command applies now, its state, its sub-commands,
//! and the key handler that maps key presses to commands.

use crate::app::LaApp;
use crate::app::commands::{self, Category, Command};
use crate::core::animator::{KEY_PAN_DURATION, WHEEL_ZOOM_DURATION};
use crate::core::capture::{CaptureMode, CaptureState};

/// A sub-command: its key (passed back to [`LaApp::dispatch_child`]) and label.
pub(crate) type Child = (String, String);

pub(crate) const MAC: bool = cfg!(target_os = "macos");

impl LaApp {
    // ---- applicability ----

    /// `Ok` when `id` can run now, else why not (the menus' tooltip).
    pub(crate) fn command_available(&self, id: &str) -> Result<(), &'static str> {
        let s = self.session.as_ref();
        let need = |ok: bool, why: &'static str| if ok { Ok(()) } else { Err(crate::l10n::tr(why)) };
        let session = || need(s.is_some(), "Open a device or a capture first.");
        let data = || need(s.is_some_and(|s| s.has_data()), "There is no data yet.");
        let finished =
            || need(s.is_some_and(|s| s.state == CaptureState::Finished && s.has_data()), "Record a capture first.");
        let idle = || need(!s.is_some_and(|s| s.state.is_busy()), "Stop recording first.");
        match id {
            "capture.toggle" => {
                session()?;
                need(
                    !s.is_some_and(|s| s.is_import() && !s.state.is_busy()),
                    "An imported file has no device to record with.",
                )?;
                need(!s.is_some_and(|s| s.state == CaptureState::Stopping), "The capture is stopping.")
            }
            "file.save" | "file.save_as" => {
                finished()?;
                need(!self.ui.saving, "A save is in progress.")
            }
            "file.open" => idle(),
            "file.open_recent" => {
                idle()?;
                need(self.settings.files.recent_files.iter().any(|p| p.exists()), "No recent captures.")
            }
            "file.export" => finished(),
            "capture.copy_cli" => {
                session()?;
                need(!s.is_some_and(|s| s.is_import()), "An imported file has no device to record with.")
            }
            "panel.device" => need(!s.is_some_and(|s| s.is_import()), "An imported file has no device."),
            "view.wire_status" => {
                session()?;
                need(s.is_some_and(|s| !s.is_read_only()), "An opened file has no live pins.")?;
                need(
                    s.is_some_and(|s| s.caps.as_ref().is_some_and(|c| c.supports_state_read)),
                    "This device can't report live pin state",
                )
            }
            "capture.repeat" => {
                session()?;
                need(!s.is_some_and(|s| s.is_read_only()), "Opened files record once.")?;
                need(s.is_some_and(|s| s.settings.mode == CaptureMode::Triggered), "Repeat needs Trigger mode")
            }
            "nav.center_trigger" => need(s.is_some_and(|s| s.trigger.is_some()), "This capture has no trigger."),
            "analog.y_fit_all" => {
                data()?;
                need(
                    s.is_some_and(|s| s.visible_rows().any(|r| r.channel.kind == crate::core::ChannelKind::Analog)),
                    "There are no analog rows.",
                )
            }
            id if id.starts_with("nav.") || id.starts_with("analog.") => data(),
            "session.new" => self.create_blocked().map_or(Ok(()), |why| Err(crate::l10n::tr(why))),
            "session.close" => {
                session()?;
                self.active_id().and_then(|id| self.close_refused(id)).map_or(Ok(()), |why| Err(crate::l10n::tr(why)))
            }
            "session.show" | "session.rename" => session(),
            "session.next" | "session.prev" => {
                need(self.session_count() > 1, "There is only one session.")?;
                self.switch_blocked().map_or(Ok(()), |why| Err(crate::l10n::tr(why)))
            }
            "session.goto" => {
                need(self.session_count() > 1, "There is only one session.")?;
                self.switch_blocked().map_or(Ok(()), |why| Err(crate::l10n::tr(why)))
            }
            id if id.starts_with("session.goto_") => {
                self.switch_blocked().map_or(Ok(()), |why| Err(crate::l10n::tr(why)))
            }
            "session.close_others" => need(self.session_count() > 1, "There are no other sessions."),
            "marker.add" | "marker.add_pair" | "range.add" | "range.pulse" => data(),
            "analyzer.add" => {
                session()?;
                idle()
            }
            id if id.starts_with("panel.") => session(),
            "table.find" => session(),
            "view.ui_bigger" => need(
                self.settings.appearance.ui_scale < crate::app::settings::UI_SCALE_MAX - 1e-3,
                "The UI is at its largest.",
            ),
            "view.ui_smaller" => need(
                self.settings.appearance.ui_scale > crate::app::settings::UI_SCALE_MIN + 1e-3,
                "The UI is at its smallest.",
            ),
            "view.ui_reset" => need((self.settings.appearance.ui_scale - 1.0).abs() > 1e-3, "The UI is at 100 %."),
            "palette.reset_recent" => need(!self.settings.history.recent_commands.is_empty(), "No recent commands."),
            "edit.undo" => need(s.is_some_and(|s| s.range_history.can_undo()), "Nothing to undo."),
            "edit.redo" => need(s.is_some_and(|s| s.range_history.can_redo()), "Nothing to redo."),
            id if id.starts_with("data.") => need(self.can_trim(), "Only a finished capture can be trimmed."),
            "history.back" => need(s.is_some_and(|s| s.history.can_go_back()), "No previous view."),
            "history.forward" => need(s.is_some_and(|s| s.history.can_go_forward()), "No next view."),
            _ => Ok(()),
        }
    }

    /// Checkable commands: their state (menus' ✓, the palette's label).
    pub(crate) fn command_checked(&self, id: &str) -> Option<bool> {
        let tab = |p: &str| self.settings.inspector.open && self.inspector_tab() == p;
        match id {
            "capture.repeat" => Some(self.session.as_ref().is_some_and(|s| s.settings.repeat)),
            "analyzer.pause" => Some(self.ui.analyzers_paused),
            "view.traces_color" => Some(self.settings.waveform.color_waveforms),
            "view.wire_status" => Some(self.session.as_ref().is_some_and(|s| s.wire_status)),
            "inspector.toggle" => Some(self.settings.inspector.open),
            "inspector.detach" => Some(self.settings.inspector.detached),
            "panel.device" => Some(tab(super::inspector::DEVICE_PANEL)),
            "panel.analyzers" => Some(tab(super::analyzers::ANALYZERS_PANEL)),
            "panel.measurements" => Some(tab(super::ranges::RANGES_PANEL)),
            "panel.markers" => Some(tab(super::markers::MARKERS_PANEL)),
            "panel.notes" => Some(self.ui.notes_panel_open),
            _ => None,
        }
    }

    /// The label with the command's state, e.g. "Theme: Dark".
    pub(crate) fn command_label(&self, c: &Command) -> String {
        let tr = crate::l10n::tr;
        let one = |template: &'static str, v: &str| tr(template).replacen("{}", v, 1);
        match c.id {
            "capture.toggle" => {
                let busy = self.session.as_ref().is_some_and(|s| s.state.is_busy());
                tr(if busy { "Stop Recording" } else { "Start Recording" }).to_owned()
            }
            "view.theme" => one("Theme: {}", self.settings.appearance.theme.label()),
            "view.trace_width" => one("Trace Width: {}", self.settings.waveform.trace_width.label()),
            "analyzer.pause" => {
                let on = if self.ui.analyzers_paused { tr("on") } else { tr("off") };
                one("Pause Analyzers: {}", on)
            }
            "capture.repeat" => {
                let on = if self.command_checked(c.id) == Some(true) { tr("on") } else { tr("off") };
                one("Repeat Capture: {}", on)
            }
            "view.wire_status" => {
                let on = if self.command_checked(c.id) == Some(true) { tr("on") } else { tr("off") };
                one("Wire Status: {}", on)
            }
            "view.traces_color" => {
                let state = if !self.settings.waveform.color_waveforms { tr("neutral") } else { tr("by channel") };
                one("Toggle Traces Color: {}", state)
            }
            _ => tr(c.label).to_owned(),
        }
    }

    // ---- sub-commands ----

    pub(crate) fn command_children(&self, id: &str) -> Vec<Child> {
        match id {
            "file.open_recent" => self
                .settings
                .files
                .recent_files
                .iter()
                .filter(|p| p.exists())
                .map(|p| {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    (p.to_string_lossy().into_owned(), name)
                })
                .collect(),
            "session.goto" => self.sessions().iter().map(|s| (s.id.to_string(), s.name.clone())).collect(),
            "view.trace_width" => crate::ui::waveform::TraceWidth::ALL
                .iter()
                .map(|w| (w.label().to_owned(), w.label().to_owned()))
                .collect(),
            "view.theme" => crate::app::settings::ThemePref::ALL
                .iter()
                .map(|t| (t.label().to_owned(), t.label().to_owned()))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn dispatch_child(&mut self, id: &str, key: &str) {
        match id {
            "file.open_recent" => self.open_paths(vec![std::path::PathBuf::from(key)]),
            "session.goto" => {
                if let Ok(sid) = key.parse()
                    && !self.activate(sid)
                {
                    self.toast(crate::ui::toast::Toast::new(
                        crate::ui::toast::ToastKind::Info,
                        crate::app::sessions::SWITCH_BLOCKED,
                    ));
                }
            }
            "view.trace_width" => {
                if let Some(w) = crate::ui::waveform::TraceWidth::ALL.into_iter().find(|w| w.label() == key) {
                    self.settings.waveform.trace_width = w;
                    self.mark_settings_dirty();
                }
            }
            "view.theme" => {
                if let Some(t) = crate::app::settings::ThemePref::ALL.into_iter().find(|t| t.label() == key) {
                    self.set_theme_pref(t);
                }
            }
            _ => {}
        }
    }

    // ---- dispatch ----

    /// Runs command `id`, if it applies.
    pub(crate) fn dispatch(&mut self, id: &str) {
        if self.command_available(id).is_err() && id != "capture.toggle" {
            return;
        }
        let ctx = self.ui.ctx.clone();
        let width = self.session.as_ref().map_or(1.0, |s| s.view.state().width_px);
        let hover_digital = self.ui.hover_digital;
        let hover_row = self.ui.hover_row;
        match id {
            "capture.toggle" => self.toggle_capture(&ctx),
            "file.save" => self.save_capture(false, None),
            "file.save_as" => self.save_capture(true, None),
            "file.open" => self.open_capture(),
            "file.export" => self.open_export_dialog(),
            "capture.copy_cli" => self.copy_cli_command(&ctx),
            "analyzer.pause" => self.ui.analyzers_paused = !self.ui.analyzers_paused,
            // On reads right away; off keeps the last state, dimmed.
            "view.wire_status" => {
                if let Some(s) = self.session.as_mut() {
                    s.wire_status = !s.wire_status;
                    s.pins_read_at = None;
                }
            }
            "help.onboarding" => self.start_onboarding(),
            "help.connection" => self.open_connection_help(),
            "help.about" => self.ui.about = Some(Default::default()),
            "capture.repeat" => {
                if let Some(s) = self.session.as_mut() {
                    s.settings.repeat = !s.settings.repeat;
                    self.mark_settings_dirty();
                }
            }
            "nav.jump" => self.open_jump(),
            "nav.prev_edge" | "nav.next_edge" => {
                if let Some(ch) = hover_digital {
                    self.center_edge(ch, id == "nav.next_edge");
                }
            }
            "nav.prev_result" | "nav.next_result" => {
                if let Some(ch) = hover_digital {
                    self.step_result(ch, id == "nav.next_result");
                }
            }
            "nav.start" => self.jump_to_edge(false),
            "nav.end" => self.jump_to_edge(true),
            "nav.zoom_in" => self.zoom_view(0.5, width / 2.0, WHEEL_ZOOM_DURATION),
            "nav.zoom_out" => self.zoom_view(2.0, width / 2.0, WHEEL_ZOOM_DURATION),
            "nav.full_zoom_in" => self.full_zoom_in(),
            "nav.fit" => self.fit_view(),
            "nav.pan_left" => self.pan_view(-width / 3.0, KEY_PAN_DURATION),
            "nav.pan_right" => self.pan_view(width / 3.0, KEY_PAN_DURATION),
            "nav.page_left" => self.pan_view(-width, KEY_PAN_DURATION),
            "nav.page_right" => self.pan_view(width, KEY_PAN_DURATION),
            "nav.rows_up" => self.ui.rows_scroll -= self.ui.rows_page,
            "nav.rows_down" => self.ui.rows_scroll += self.ui.rows_page,
            "nav.center_trigger" => self.center_trigger(),
            "analog.y_zoom_in" | "analog.y_zoom_out" => {
                let k = if id == "analog.y_zoom_in" { 0.5 } else { 2.0 };
                if let Some(h) = hover_row {
                    self.zoom_analog(h.channel, k, h.y);
                }
            }
            "analog.y_fit" => {
                if let Some(h) = hover_row {
                    self.y_fit(h.channel);
                }
            }
            "analog.y_fit_all" => self.y_fit_all(),
            "session.new" => self.new_session(),
            "session.close" => {
                if let Some(sid) = self.active_id() {
                    self.request_close(sid);
                }
            }
            "session.show" => self.toggle_sessions_popover(),
            "session.next" => self.activate_step(true),
            "session.prev" => self.activate_step(false),
            "session.rename" => {
                if let Some(sid) = self.active_id() {
                    self.start_session_rename(sid);
                }
            }
            "session.close_others" => {
                if let Some(sid) = self.active_id() {
                    self.close_others(sid);
                }
            }
            id if id.starts_with("session.goto_") => {
                if let Ok(n) = id["session.goto_".len()..].parse() {
                    self.activate_number(n);
                }
            }
            "marker.add" => self.begin_marker_placement(false),
            "marker.add_pair" => self.begin_marker_placement(true),
            "marker.delete_selected" | "range.delete_selected" => self.delete_selected_annotations(),
            "marker.select_all" | "range.select_all" => self.select_all_annotations(),
            "range.add" => self.begin_range_placement(),
            "analyzer.add" => self.ui.open_picker = true,
            "range.pulse" => self.quick_add_pulse(),
            "panel.device" => self.toggle_panel(super::inspector::DEVICE_PANEL),
            "panel.analyzers" => self.toggle_panel(super::analyzers::ANALYZERS_PANEL),
            "panel.measurements" => self.toggle_panel(super::ranges::RANGES_PANEL),
            "panel.markers" => self.toggle_panel(super::markers::MARKERS_PANEL),
            "panel.notes" => self.toggle_notes_panel(),
            "help.shortcuts" => {
                self.ui.shortcuts_open = !self.ui.shortcuts_open;
                self.ui.shortcuts_filter.clear();
            }
            "inspector.toggle" => self.toggle_inspector(),
            "inspector.detach" => self.set_inspector_detached(!self.settings.inspector.detached),
            "table.find" => {
                self.settings.inspector.tab = super::analyzers::ANALYZERS_PANEL.into();
                self.settings.inspector.open = true;
                self.ui.data_mode = super::datatable::DataMode::Table;
                self.ui.table_focus = true;
                self.mark_settings_dirty();
            }
            "view.traces_color" => {
                self.settings.waveform.color_waveforms = !self.settings.waveform.color_waveforms;
                self.mark_settings_dirty();
            }
            "palette.open" => self.open_palette(),
            "settings.open" => self.open_settings(super::settings::Section::Appearance),
            "view.ui_bigger" => self.step_ui_scale(Some(1)),
            "view.ui_smaller" => self.step_ui_scale(Some(-1)),
            "view.ui_reset" => self.step_ui_scale(None),
            "palette.reset_recent" => {
                self.settings.history.recent_commands.clear();
                self.mark_settings_dirty();
            }
            "edit.undo" => self.undo_range(),
            "edit.redo" => self.redo_range(),
            "data.delete_off_screen" | "data.delete_before_view" | "data.delete_after_view" => {
                let Some(v) = self.session.as_ref().map(|s| s.view.state()) else { return };
                let (a, b) = (v.left_time, v.right_time());
                let item: super::trim::TrimItem = match id {
                    "data.delete_off_screen" => ("Off Screen", Some(a), Some(b), "the visible range".into()),
                    "data.delete_before_view" => ("Before the View", Some(a), None, "the view's left edge".into()),
                    _ => ("After the View", None, Some(b), "the view's right edge".into()),
                };
                self.request_trim(&item);
            }
            "history.back" => self.history_step(false),
            "history.forward" => self.history_step(true),
            _ => {}
        }
    }

    /// Open the Inspector on a tab (and switch to it); it
    /// stays open. Closing is `Alt+I` / `⌘\`.
    pub(crate) fn toggle_panel(&mut self, id: &str) {
        self.settings.inspector.tab = id.into();
        self.settings.inspector.open = true;
        self.mark_settings_dirty();
    }

    /// Toggle the Inspector.
    pub(crate) fn toggle_inspector(&mut self) {
        self.settings.inspector.open = !self.settings.inspector.open;
        self.mark_settings_dirty();
    }

    /// Toggle the Notes bottom panel (per run).
    pub(crate) fn toggle_notes_panel(&mut self) {
        self.ui.notes_panel_open = !self.ui.notes_panel_open;
    }

    /// Detach the Inspector to a window of its own, or attach
    /// it again. Either way it is shown (closing the window attaches and
    /// hides it instead, `inspector_window`).
    pub(crate) fn set_inspector_detached(&mut self, detached: bool) {
        self.settings.inspector.detached = detached;
        self.settings.inspector.open = true;
        self.mark_settings_dirty();
    }

    /// `Ctrl/Cmd+A`: markers when their tab is selected, else measurements.
    fn select_all_annotations(&mut self) {
        let tab = self.inspector_tab().to_owned();
        let Some(s) = self.session.as_mut() else { return };
        if tab == super::markers::MARKERS_PANEL {
            s.markers.select_all();
        } else {
            s.ranges.items.iter_mut().for_each(|r| r.selected = true);
        }
    }

    /// No modal dialog is open, so menu items may run.
    #[cfg(all(target_os = "macos", feature = "macos-menu"))]
    pub(crate) fn menus_allowed(&self, _ctx: &egui::Context) -> bool {
        !(self.ui.shortcuts_open
            || self.ui.settings_dialog.is_some()
            || self.ui.palette.is_some()
            || self.ui.prompt.is_some()
            || self.ui.confirm.is_some()
            || self.ui.analyzer_dialog.is_some()
            || self.ui.export.is_some()
            || self.ui.probe_editor.is_some()
            || self.ui.trim.is_some()
            || self.ui.import.is_some())
    }

    // ---- keys ----

    /// Maps this frame's key presses to commands. Runs before the UI, so
    /// `Space` never reaches a focused button.
    pub(crate) fn handle_command_keys(&mut self, ctx: &egui::Context) {
        use egui::{Event, Key};
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let events: Vec<Event> = ctx.input(|i| i.events.clone());
        let mods = ctx.input(|i| i.modifiers);
        // Which commands may run: a modal takes the keys, except `?` in the
        // Keyboard Shortcuts dialog and the Sessions keys in their popover.
        let allowed = |c: &Command| -> bool {
            if self.ui.shortcuts_open {
                return c.id == "help.shortcuts";
            }
            if self.ui.palette.is_some()
                || self.ui.settings_dialog.is_some()
                || self.ui.prompt.is_some()
                || self.ui.confirm.is_some()
                || self.ui.session_rename.is_some()
            {
                return false;
            }
            if self.ui.sessions_open {
                return c.category == Category::Sessions;
            }
            !(self.ui.analyzer_dialog.is_some()
                || self.ui.jump_open
                || self.ui.rename.is_some()
                || egui::Popup::is_any_open(ctx)
                || self.ui.export.is_some()
                || self.ui.probe_editor.is_some()
                || self.ui.trim.is_some()
                || self.ui.import.is_some())
        };
        // Pointer-focus commands first.
        let ordered: Vec<&'static Command> = {
            let r = commands::registry();
            r.iter().filter(|c| c.pointer).chain(r.iter().filter(|c| !c.pointer)).collect()
        };
        // Plain keys that aren't commands apply when commands could run.
        let free = allowed(commands::get("nav.fit").expect("registered"));
        let (mut run, mut escape, mut gallery): (Vec<&'static str>, bool, bool) = (Vec::new(), false, false);
        for e in &events {
            match e {
                Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
                    let found = ordered.iter().find(|c| c.keys.iter().any(|k| k.matches_key(*key, *modifiers, MAC)));
                    match found {
                        Some(c) if allowed(c) => {
                            ctx.input_mut(|i| i.consume_key(*modifiers, *key));
                            run.push(c.id);
                        }
                        Some(_) => {}
                        None if *key == Key::Escape && free => escape = true,
                        None if *key == Key::F12 && modifiers.is_none() && free => gallery = true,
                        None => {}
                    }
                }
                Event::Text(text) => {
                    if let Some(c) = ordered.iter().find(|c| c.keys.iter().any(|k| k.matches_text(text, mods)))
                        && allowed(c)
                    {
                        run.push(c.id);
                    }
                }
                _ => {}
            }
        }
        if escape {
            self.cancel_marker_placement();
            self.ui.range_place = None;
        }
        if gallery {
            self.ui.show_gallery = !self.ui.show_gallery;
        }
        for id in run {
            if id == "session.next" || id == "session.prev" {
                // egui moved the keyboard focus on this Tab; take it back.
                self.ui.drop_focus = true;
            }
            self.dispatch(id);
        }
    }
}
