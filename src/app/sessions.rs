//! Several sessions and the switching rules (04).
//!
//! The active session lives in [`LaApp::session`], so the UI code borrows it
//! directly; the others wait in [`LaApp::parked`]. [`LaApp::order`] is the
//! display order of all of them.

use crate::app::LaApp;
use crate::app::session::{ParkedUi, Session};
use crate::app::ui::files::{Pending, Prompt};
use crate::app::worker::{SessionId, WorkerCmd};
use crate::backend::DeviceInfo;
use crate::core::capture::CaptureState;
use crate::ui::toast::{Toast, ToastKind};
use std::time::Duration;

/// Tooltips.
pub const SWITCH_BLOCKED: &str = "Stop recording to switch sessions";
pub const CREATE_BLOCKED: &str = "Stop recording to create a new session";
/// How long `Ctrl+Tab` shows the popover.
pub const SWITCH_FLASH: Duration = Duration::from_millis(800);

impl LaApp {
    // ---- lookup ----

    /// Every session in display order.
    pub fn sessions(&self) -> Vec<&Session> {
        self.order.iter().filter_map(|id| self.session_by_id(*id)).collect()
    }

    pub fn session_count(&self) -> usize {
        self.order.len()
    }

    pub fn session_by_id(&self, id: SessionId) -> Option<&Session> {
        self.session.as_ref().filter(|s| s.id == id).or_else(|| self.parked.iter().find(|s| s.id == id))
    }

    pub(crate) fn session_by_id_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        match self.session.as_mut() {
            Some(s) if s.id == id => Some(s),
            _ => self.parked.iter_mut().find(|s| s.id == id),
        }
    }

    pub(crate) fn active_id(&self) -> Option<SessionId> {
        self.session.as_ref().map(|s| s.id)
    }

    /// Why switching sessions isn't possible now.
    pub(crate) fn switch_blocked(&self) -> Option<&'static str> {
        let busy = self.session.as_ref().is_some_and(|s| s.state.is_busy());
        (busy || self.ui.loading).then_some(SWITCH_BLOCKED)
    }

    /// Why creating a session isn't possible now.
    pub(crate) fn create_blocked(&self) -> Option<&'static str> {
        let busy =
            self.session.as_ref().is_some_and(|s| s.state.is_busy()) || self.parked.iter().any(|s| s.state.is_busy());
        (busy || self.ui.loading).then_some(CREATE_BLOCKED)
    }

    // ---- creating ----

    /// Adds `s` after the others and makes it active; its analyzers join
    /// the decoder thread.
    pub(crate) fn add_session(&mut self, s: Session) {
        let id = s.id;
        self.park_active();
        self.order.push(id);
        self.session = Some(s);
        self.register_decoders();
        self.ui.rows_scroll = 0.0;
    }

    pub(crate) fn next_id(&mut self) -> SessionId {
        let id = self.next_session_id;
        self.next_session_id += 1;
        id
    }

    /// A new session on `device`. With `source`, its settings,
    /// rows, analyzers and zoom are copied; otherwise the device's last
    /// settings apply.
    pub(crate) fn open_session(&mut self, device: &DeviceInfo, width_px: f64) {
        self.open_session_from(device, width_px, None);
    }

    pub(crate) fn open_session_from(&mut self, device: &DeviceInfo, width_px: f64, source: Option<SessionId>) {
        if let Some(why) = self.create_blocked() {
            self.toast(Toast::new(ToastKind::Info, why).id("session-blocked"));
            return;
        }
        let id = self.next_id();
        let saved = self.settings.device_settings.get(&device.id.0).cloned();
        let mut s = Session::new(id, device.clone(), saved, width_px);
        s.set_probe(self.profiles.select(device));
        s.view.speed_scale = self.animation_speed;
        if let Some(src) = source.and_then(|id| self.session_by_id(id)) {
            s.copy_setup_from(src);
        }
        // A new session never starts repeating.
        s.settings.repeat = false;
        self.worker.send(WorkerCmd::Open { session: id, device: device.id.clone() });
        self.add_session(s);
    }

    /// *New Session*: on the active session's device if it's still
    /// connected, else the most recently connected device, else the
    /// Devices popover opens.
    pub(crate) fn new_session(&mut self) {
        if let Some(why) = self.create_blocked() {
            self.toast(Toast::new(ToastKind::Info, why).id("session-blocked"));
            return;
        }
        let devices = self.devices.clone().unwrap_or_default();
        let width = self.session.as_ref().map_or(800.0, |s| s.view.state().width_px);
        let active = self.session.as_ref().filter(|s| !s.is_read_only() && s.open_error.is_none());
        if let Some(s) = active
            && let Some(d) = devices.iter().find(|d| d.id == s.device.id)
        {
            let (d, src) = (d.clone(), s.id);
            self.open_session_from(&d, width, Some(src));
        } else if let Some(d) = devices.iter().rev().find(|d| !d.is_demo) {
            let d = d.clone();
            self.open_session(&d, width);
        } else {
            self.ui.open_devices_menu = true;
        }
    }

    // ---- switching ----

    /// Moves the active session to the background with its view state.
    fn park_active(&mut self) {
        let Some(mut s) = self.session.take() else { return };
        s.parked_ui = ParkedUi {
            rows_scroll: self.ui.rows_scroll,
            table: std::mem::take(&mut self.ui.table),
            terminal_cleared: std::mem::take(&mut self.ui.terminal_cleared),
            export_included: self.ui.export_included.take(),
        };
        self.mark_settings_dirty_for(&s);
        self.reset_session_ui();
        self.parked.push(s);
    }

    /// Per-session transient UI state that must not leak into another session.
    fn reset_session_ui(&mut self) {
        let u = &mut self.ui;
        u.rename = None;
        u.range_popup = None;
        u.range_place = None;
        u.range_edge = None;
        u.marker_edit = None;
        u.marker_drag = None;
        u.placing = None;
        u.last_marker_click = None;
        u.drag = None;
        u.marquee = None;
        u.reorder = None;
        u.context_row = None;
        u.auto_layout_for = None;
        u.row_inner_height.clear();
        u.row_geometry.clear();
        u.hover_row = None;
        u.hover_digital = None;
        u.metric_cache.clear();
        u.deleted_analyzer = None;
        u.trim = None;
        u.jump_open = false;
    }

    /// Activates session `id`. False when switching is blocked.
    pub(crate) fn activate(&mut self, id: SessionId) -> bool {
        if self.active_id() == Some(id) {
            return true;
        }
        if self.switch_blocked().is_some() {
            return false;
        }
        let Some(i) = self.parked.iter().position(|s| s.id == id) else { return false };
        self.park_active();
        let i = self.parked.iter().position(|s| s.id == id).unwrap_or(i);
        let mut s = self.parked.remove(i);
        let p = std::mem::take(&mut s.parked_ui);
        self.ui.rows_scroll = p.rows_scroll;
        self.ui.table = p.table;
        self.ui.terminal_cleared = p.terminal_cleared;
        self.ui.export_included = p.export_included;
        self.session = Some(s);
        self.ui.ctx.request_repaint();
        true
    }

    /// `Ctrl+Tab` / `Ctrl+Shift+Tab`, wrapping; flashes the popover.
    pub(crate) fn activate_step(&mut self, forward: bool) {
        let n = self.order.len();
        let Some(i) = self.active_id().and_then(|id| self.order.iter().position(|x| *x == id)) else { return };
        if n < 2 {
            return;
        }
        let j = if forward { (i + 1) % n } else { (i + n - 1) % n };
        if self.activate(self.order[j]) {
            self.ui.sessions_flash_until = Some(self.now() + SWITCH_FLASH);
        } else {
            self.toast(Toast::new(ToastKind::Info, crate::l10n::tr(SWITCH_BLOCKED)).id("session-blocked"));
        }
    }

    /// `Ctrl/Cmd+1…8`, and `Ctrl/Cmd+9` for the last.
    pub(crate) fn activate_number(&mut self, k: usize) {
        let i = if k == 9 { self.order.len().saturating_sub(1) } else { k - 1 };
        if let Some(&id) = self.order.get(i)
            && !self.activate(id)
        {
            self.toast(Toast::new(ToastKind::Info, crate::l10n::tr(SWITCH_BLOCKED)).id("session-blocked"));
        }
    }

    // ---- closing ----

    /// Why session `id` can't close now.
    pub(crate) fn close_refused(&self, id: SessionId) -> Option<&'static str> {
        let s = self.session_by_id(id)?;
        if s.state.is_busy() {
            Some("Stop recording before closing this session.")
        } else if self.ui.exporting > 0 && self.active_id() == Some(id) {
            Some("Wait for the export to finish before closing this session.")
        } else if self.ui.saving || self.ui.loading {
            Some("Wait for the save or load to finish before closing this session.")
        } else {
            None
        }
    }

    /// Close session `id`, asking "Save Capture?" when it's unsaved
    pub(crate) fn request_close(&mut self, id: SessionId) {
        if let Some(why) = self.close_refused(id) {
            self.toast(Toast::new(ToastKind::Info, crate::l10n::tr(why)).id("close-refused"));
            return;
        }
        let Some(s) = self.session_by_id(id) else { return };
        if s.is_unsaved() {
            self.ui.prompt = Some(Prompt::SaveCapture { session: id, then: Pending::Close(vec![id]) });
        } else {
            self.close_now(id);
        }
    }

    /// Closes session `id` without asking; then.
    pub(crate) fn close_now(&mut self, id: SessionId) {
        let Some(pos) = self.order.iter().position(|x| *x == id) else { return };
        self.order.remove(pos);
        self.worker.send(WorkerCmd::Close { session: id });
        self.decode.remove_group(id);
        if self.active_id() == Some(id) {
            let s = self.session.take();
            if let Some(s) = &s {
                self.mark_settings_dirty_for(s);
            }
            self.reset_session_ui();
            self.ui.table = Default::default();
            self.ui.terminal_cleared.clear();
            self.ui.export_included = None;
            // The previous one in display order, else the next.
            let next = pos.checked_sub(1).or((!self.order.is_empty()).then_some(0)).map(|i| self.order[i]);
            if let Some(n) = next {
                self.activate(n);
            }
        } else {
            self.parked.retain(|s| s.id != id);
        }
    }

    /// *Close Other Sessions*: one by one in display order, asking
    /// about unsaved ones; the ones that can't close are reported once.
    pub(crate) fn close_others(&mut self, keep: SessionId) {
        let others: Vec<SessionId> = self.order.iter().copied().filter(|id| *id != keep).collect();
        self.close_list(others);
    }

    /// Closes `ids` in order, stopping at the first one that asks.
    pub(crate) fn close_list(&mut self, ids: Vec<SessionId>) {
        let mut refused = 0;
        for (k, id) in ids.iter().enumerate() {
            if self.close_refused(*id).is_some() {
                refused += 1;
                continue;
            }
            let Some(s) = self.session_by_id(*id) else { continue };
            if s.is_unsaved() {
                self.ui.prompt = Some(Prompt::SaveCapture { session: *id, then: Pending::Close(ids[k..].to_vec()) });
                break;
            }
            self.close_now(*id);
        }
        if refused > 0 {
            let body = if refused == 1 {
                crate::l10n::tr("1 session couldn't be closed while it's busy.").to_owned()
            } else {
                crate::l10n::tr("{} sessions couldn't be closed while they're busy.").replacen(
                    "{}",
                    &refused.to_string(),
                    1,
                )
            };
            self.toast(Toast::new(ToastKind::Info, body).id("close-refused"));
        }
    }

    // ---- editing ----

    /// Rename: an empty name keeps the old one.
    pub(crate) fn rename_session(&mut self, id: SessionId, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(s) = self.session_by_id_mut(id) {
            s.name = name.to_owned();
        }
    }

    /// Drag to reorder: `id` moves just before `before` (or last).
    pub(crate) fn move_session(&mut self, id: SessionId, before: Option<SessionId>) {
        let Some(from) = self.order.iter().position(|x| *x == id) else { return };
        self.order.remove(from);
        let to = before.and_then(|b| self.order.iter().position(|x| *x == b)).unwrap_or(self.order.len());
        self.order.insert(to, id);
    }

    /// The state line under a session's name.
    pub fn session_state_text(&self, s: &Session) -> String {
        let state = match s.state {
            CaptureState::Starting | CaptureState::Recording | CaptureState::Stopping => {
                crate::l10n::tr("Recording").to_owned()
            }
            _ if s.opened_from.is_some() => {
                let name = s.opened_from.as_deref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy());
                crate::l10n::tr("Opened from {}").replacen("{}", &name.unwrap_or_default(), 1)
            }
            CaptureState::Finished => match s.bounds() {
                Some(b) => crate::l10n::tr("Finished · {}").replacen(
                    "{}",
                    &crate::core::format::time_short(b.duration_secs()),
                    1,
                ),
                None => crate::l10n::tr("Finished").to_owned(),
            },
            _ => crate::l10n::tr("Empty").to_owned(),
        };
        format!("{} · {state}", self.device_label(&s.device))
    }
}
