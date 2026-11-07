#![cfg(feature = "mock")]
//! Sessions: the switching, closing and quit rules.

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::ScriptedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::core::{ChannelId, ManualClock};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
    dialogs: Arc<ScriptedDialogs>,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let clock = ManualClock::new();
        let dir = tempfile::tempdir().unwrap();
        let dialogs = Arc::new(ScriptedDialogs { folder: Some(dir.path().to_owned()), ..Default::default() });
        let (c, d) = (clock.clone(), dialogs.clone());
        let h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::new(options)),
                    clock: Arc::new(c),
                    dialogs: d,
                    settings_path: None,
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: None,
                    extra_builtin_profiles: Vec::new(),
                    mock_decoders: cfg!(feature = "mock"),
                },
            )
        });
        Rig { h, clock, dialogs, _dir: dir }
    }

    fn until(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.step();
    }

    /// The 16-channel demo device from the start screen.
    fn open_demo(&mut self) {
        self.h.run();
        self.h.get_by_label("Mock 16 digital + 4 analog").click();
        self.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    }

    fn new_session(&mut self) {
        let n = self.h.state().session_count();
        self.h.key_press_modifiers(Modifiers::COMMAND, Key::T);
        self.until("new session open", move |a| a.session_count() == n + 1 && a.session().unwrap().caps.is_some());
    }

    fn start(&mut self) {
        self.h.key_press(Key::Space);
        self.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
    }

    fn record(&mut self, ms: u64) {
        self.start();
        self.clock.advance(Duration::from_millis(ms));
        let want = ms * RATE / 1000;
        self.until("data", move |a| {
            a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|r| r.1 >= want)
        });
        self.h.key_press(Key::Space);
        self.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    }

    fn active(&self) -> String {
        self.h.state().session().map(|s| s.name.clone()).unwrap_or_default()
    }

    fn names(&self) -> Vec<String> {
        self.h.state().sessions().iter().map(|s| s.name.clone()).collect()
    }

    fn popover(&mut self) {
        let label = self.button_label();
        self.h.get_by_label(&label).click();
        self.h.run_steps(4);
        self.h.get_by_label("SESSIONS");
    }

    fn button_label(&self) -> String {
        let n = self.h.state().session_count();
        if n > 1 { format!("{} ({n} sessions)", self.active()) } else { self.active() }
    }
}

/// `Ctrl/Cmd+T` creates "Session 1" on the same device with the same
/// enabled channels; `Ctrl+Tab` cycles; closing activates the previous one.
#[test]
fn new_session_cycle_close() {
    let mut r = Rig::new();
    r.open_demo();
    r.h.state_mut().session_mut().unwrap().settings.enabled.push(ChannelId::digital(5));
    let enabled = r.h.state().session().unwrap().settings.enabled.clone();
    r.new_session();
    assert_eq!(r.active(), "Session 1");
    let s = r.h.state().session().unwrap();
    assert_eq!(s.device.id, r.h.state().sessions()[0].device.id);
    assert_eq!(s.settings.enabled, enabled);
    r.h.get_by_label("Session 1 (2 sessions)");

    r.h.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    r.h.run();
    assert_eq!(r.active(), "Session 0");
    // The popover flashes with the new row highlighted.
    r.h.get_by_label("SESSIONS");
    r.clock.advance(Duration::from_millis(900));
    r.h.run();
    assert!(r.h.query_by_label("SESSIONS").is_none(), "the flash ends");
    r.h.key_press_modifiers(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab);
    r.h.run();
    assert_eq!(r.active(), "Session 1", "wraps around");

    // Closing activates the previous one in display order.
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    r.h.run();
    assert_eq!(r.names(), ["Session 0"]);
    assert_eq!(r.active(), "Session 0");
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    r.h.run();
    assert!(r.h.state().session().is_none());
    r.h.get_by_label("Devices");
    // The counter keeps counting during the run.
    r.open_demo();
    assert_eq!(r.active(), "Session 2");
}

/// The popover lists sessions with device and state; clicking a row
/// activates it; double-click renames inline; *Close Other Sessions*
/// leaves one; while recording the other rows are dimmed.
#[test]
fn sessions_popover() {
    let mut r = Rig::new();
    r.open_demo();
    r.record(10);
    r.new_session();
    r.new_session();
    assert_eq!(r.names(), ["Session 0", "Session 1", "Session 2"]);
    r.popover();
    r.h.get_by_label("[DEMO] Mock 16 digital + 4 analog · Finished · 10 ms");
    assert_eq!(r.h.get_all_by_label("[DEMO] Mock 16 digital + 4 analog · Empty").count(), 2);
    r.h.get_by_label("New Session");
    r.h.snapshot("sessions_popover_dark");
    r.h.get_by_label("Session 0").click();
    r.h.run();
    assert_eq!(r.active(), "Session 0");
    r.clock.advance(Duration::from_millis(400));
    r.h.run();
    assert!(r.h.query_by_label("SESSIONS").is_none(), "activating closes the popover");

    // Double-click renames inline; Enter commits.
    r.popover();
    // Let the button's click age, or the two row clicks count as a triple-click.
    for _ in 0..40 {
        r.h.step();
    }
    r.h.get_by_label("Session 1").click();
    r.h.step();
    r.h.get_by_label("Session 1").click();
    r.h.run();
    let edit = r.h.get_by_label("Session name");
    edit.focus();
    r.h.run();
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    r.h.get_by_label("Session name").type_text("I2C bench");
    r.h.key_press(Key::Enter);
    r.h.run();
    assert_eq!(r.names(), ["Session 0", "I2C bench", "Session 2"]);

    // Ctrl/Cmd+3 goes to the third, Ctrl/Cmd+9 to the last.
    r.h.key_press(Key::Escape);
    r.h.run();
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::Num2);
    r.h.run();
    assert_eq!(r.active(), "I2C bench");
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::Num9);
    r.h.run();
    assert_eq!(r.active(), "Session 2");

    // While recording, the other rows are dimmed and don't activate.
    r.start();
    r.popover();
    r.h.get_by_label("Session 0").click();
    r.h.run_steps(4);
    assert_eq!(r.active(), "Session 2");
    // Tooltips wait for the pointer to move after a click.
    r.h.hover_at(egui::pos2(600.0, 300.0));
    r.h.run_steps(2);
    r.h.get_by_label("Session 0").hover();
    r.h.run_steps(40);
    r.h.get_by_label("Stop recording to switch sessions");
    r.h.key_press(Key::Escape);
    r.h.run_steps(2);
    assert!(r.h.query_by_label("SESSIONS").is_none(), "Escape closes the popover");
    r.clock.advance(Duration::from_millis(10));
    r.until("data", |a| a.session().unwrap().has_data());
    r.h.key_press(Key::Space);
    r.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);

    // Right-click → Close Other Sessions leaves one (Session 0 is unsaved: Don't Save).
    r.popover();
    r.h.get_by_label("I2C bench").click_secondary();
    r.h.run();
    r.h.get_by_label("Close Other Sessions").click();
    r.h.run();
    r.h.get_by_label("Save Capture?");
    r.h.get_by_label("Session 0 has a capture that hasn't been saved.");
    r.h.get_by_label("Don't Save").click();
    r.h.run();
    r.h.get_by_label("Session 2 has a capture that hasn't been saved.");
    r.h.get_by_label("Don't Save").click();
    r.h.run();
    assert_eq!(r.names(), ["I2C bench"]);
    assert_eq!(r.active(), "I2C bench");
}

/// Dragging a row reorders the sessions.
#[test]
fn drag_reorders_sessions() {
    let mut r = Rig::new();
    r.open_demo();
    r.new_session();
    r.new_session();
    r.popover();
    let from = r.h.get_by_label("Session 2").rect().center();
    let to = r.h.get_by_label("Session 0").rect().center() - egui::vec2(0.0, 10.0);
    r.h.drag_at(from);
    r.h.run();
    r.h.hover_at(to);
    r.h.run();
    r.h.drop_at(to);
    r.h.run();
    assert_eq!(r.names(), ["Session 2", "Session 0", "Session 1"]);
}

/// *Save…* saves and closes only after the save completes.
#[test]
fn close_unsaved_saves_first() {
    let mut r = Rig::new();
    r.open_demo();
    r.record(10);
    r.new_session();
    // Close Session 0 from its row while Session 1 is active.
    r.popover();
    r.h.get_by_label("Session 0").hover();
    r.h.run_steps(4);
    r.h.get_by_label("Close Session 0").click();
    r.h.run_steps(4);
    r.h.get_by_label("Save Capture?");
    // Cancelling the save dialog keeps it open.
    r.dialogs.cancel_save.store(true, std::sync::atomic::Ordering::SeqCst);
    r.h.get_by_label("Save…").click();
    let d = r.dialogs.clone();
    r.until("save dialog cancelled", move |_| !d.cancel_save.load(std::sync::atomic::Ordering::SeqCst));
    r.h.run_steps(3);
    assert_eq!(r.names(), ["Session 0", "Session 1"]);
    // Again, saving this time.
    let path = r._dir.path().join("zero.oxrok");
    *r.dialogs.save.lock() = Some(path.clone());
    r.popover();
    r.h.get_by_label("Session 0").hover();
    r.h.run_steps(4);
    r.h.get_by_label("Close Session 0").click();
    r.h.run_steps(3);
    r.h.get_by_label("Save…").click();
    r.until("closed after saving", |a| a.session_count() == 1);
    assert!(path.exists());
    assert_eq!(r.active(), "Session 1");
}

/// *Save Checked…* saves each checked capture, then quits.
#[test]
fn quit_saves_checked_captures() {
    let mut r = Rig::new();
    r.open_demo();
    r.record(10);
    r.new_session();
    r.record(10);
    *r.dialogs.save.lock() = Some(r._dir.path().join("one.oxrok"));
    r.h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    r.h.run_steps(3);
    r.h.get_by_label("Save Captures?");
    // Uncheck Session 0: only Session 1 is saved.
    r.h.get_by_label("• Session 0").click();
    r.h.run();
    r.h.get_by_label("Save Checked…").click();
    r.until("saved", |a| a.sessions().iter().any(|s| s.name == "one"));
    r.h.run_steps(3);
    assert!(r.h.state().sessions()[0].is_unsaved(), "unchecked stays unsaved");
}

/// Several files open, each in its own session; the last is active.
#[test]
fn several_files_open_in_their_own_sessions() {
    let mut r = Rig::new();
    r.open_demo();
    r.record(10);
    let a = r._dir.path().join("a.oxrok");
    *r.dialogs.save.lock() = Some(a.clone());
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    r.until("saved", |a| a.session().is_some_and(|s| !s.is_unsaved()));
    let b = r._dir.path().join("b.oxrok");
    std::fs::copy(&a, &b).unwrap();
    *r.dialogs.open.lock() = vec![a, b];
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    r.until("both open", |a| a.session_count() == 3);
    r.h.run_steps(3);
    assert_eq!(r.active(), "b", "{:?}", r.names());
    assert_eq!(r.names(), ["a", "a", "b"]);
}

/// Background sessions keep their analyzers' results; switching
/// doesn't restart decoding.
#[test]
fn switching_keeps_decoded_results() {
    let mut r = Rig::new();
    r.open_demo();
    r.record(20);
    // Analyzers tab → Async Serial → RX = D1 → Save.
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num2);
    r.h.run_steps(3);
    r.h.get_by_role_and_label(egui::Role::MenuItem, "Async Serial").click();
    r.h.run_steps(10);
    r.h.get_by_label("RX channel").click();
    r.h.run_steps(3);
    r.h.get_by_label("01.  D1").click();
    r.h.run_steps(3);
    r.h.get_by_label("Save").click();
    r.h.run_steps(5);
    let out = r.h.state().session().unwrap().analyzers[0].out.clone();
    let done = move |_: &LaApp| *out.status.lock() == oxrok_ui::decode::runner::Status::Complete;
    r.until("decoded", done.clone());
    let n = r.h.state().session().unwrap().analyzers[0].out.frames.read().len();
    assert!(n > 0);
    r.new_session();
    r.record(10);
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::Num1);
    r.h.run();
    let s = r.h.state().session().unwrap();
    assert_eq!(s.name, "Session 0");
    assert_eq!(*s.analyzers[0].out.status.lock(), oxrok_ui::decode::runner::Status::Complete);
    assert_eq!(s.analyzers[0].out.frames.read().len(), n);
}

/// A long file name in the state line is ellipsized inside the
/// popover, with the full text on hover.
#[test]
fn long_state_lines_fit_the_popover() {
    let mut r = Rig::new();
    r.open_demo();
    r.record(10);
    let long = r._dir.path().join("Session 0 2026-10-20 10-48 with a really long name.vcd.oxrok");
    *r.dialogs.save.lock() = Some(long.clone());
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    r.until("saved", |a| a.session().is_some_and(|s| !s.is_unsaved()));
    *r.dialogs.open.lock() = vec![long];
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    r.until("opened", |a| a.session_count() == 2);
    r.popover();
    let state = r.h.state().session_state_text(r.h.state().session().unwrap());
    let popover = r.h.get_by_label("SESSIONS").rect();
    let line = r.h.get_by_label(&state).rect();
    let row = r.h.get_all_by_label("Session 0 2026-10-20 10-48 with a really long name.vcd").next().unwrap().rect();
    assert!(line.right() <= row.right() - 30.0, "{line:?} inside {row:?} (title {popover:?})");
}
