#![cfg(feature = "mock")]
//! Files: saving, opening, recent and the file banner.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::dialogs::ScriptedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::core::{CaptureTime, ChannelId, ManualClock};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
    dialogs: Arc<ScriptedDialogs>,
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let clock = ManualClock::new();
        let dir = tempfile::tempdir().unwrap();
        let dialogs = Arc::new(ScriptedDialogs { folder: Some(dir.path().to_owned()), ..Default::default() });
        let (c, d) = (clock.clone(), dialogs.clone());
        let h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
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
        Rig { h, clock, dialogs, dir }
    }

    fn until(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            if t0.elapsed() > Duration::from_secs(30) {
                let toasts: Vec<String> =
                    self.h.state().toasts().iter().map(|t| format!("{:?}: {}", t.heading, t.body)).collect();
                panic!("timed out waiting for {what}; toasts: {toasts:?}");
            }
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.step();
    }

    /// Opens the 16-channel mock and records `ms` of data.
    fn record(&mut self, ms: u64) {
        if self.h.state().session().is_none() {
            self.h.run();
            self.h.get_by_label("Mock 16 digital + 4 analog").click();
            self.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
        }
        self.h.key_press(Key::Space);
        self.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
        self.clock.advance(Duration::from_millis(ms));
        let want = ms * RATE / 1000;
        self.until("data", move |a| {
            a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|r| r.1 >= want)
        });
        self.h.key_press(Key::Space);
        self.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    }

    fn save(&mut self) {
        self.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::S);
        self.until("saved", |a| !a.session().unwrap().is_unsaved());
    }

    fn open(&mut self, path: &Path) {
        *self.dialogs.open.lock() = vec![path.to_owned()];
        self.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::O);
        let p = path.to_owned();
        self.until("opened", move |a| a.session().is_some_and(|s| s.opened_from.as_deref() == Some(&*p)));
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn has_toast(&self, text: &str) -> bool {
        self.h.state().toasts().iter().any(|t| t.body.contains(text) || t.heading.as_deref() == Some(text))
    }
}

fn levels(app: &LaApp, ch: ChannelId) -> Vec<Option<bool>> {
    let s = app.session().unwrap();
    let cap = s.capture.as_ref().unwrap().read();
    let c = cap.channel(ch).unwrap();
    let (b, e) = c.data_range().unwrap();
    (b..e).step_by(997).map(|x| c.level_at(x)).collect()
}

/// U-16 and: save → reopen keeps the samples and the annotations; the
/// reopened session is read-only and shows the banner.
#[test]
fn save_and_reopen() {
    let mut r = Rig::new();
    r.record(20);
    assert!(r.h.state().session().unwrap().is_unsaved(), "a new capture is unsaved");
    r.h.run_steps(2);
    let title = r.h.state().window_title().to_owned();
    assert!(title.starts_with("• "), "the title marks it: {title:?}");
    {
        let s = r.h.state_mut().session_mut().unwrap();
        s.markers.add_single(CaptureTime::from_secs_f64(0.005));
        s.notes = "bench run".into();
        s.rows[0].name = "CLK".into();
    }
    let before_d1 = levels(r.h.state(), ChannelId::digital(1));
    r.save();
    let path = r.path("Session 0.oxrok");
    assert!(path.exists(), "saved under the tab name");
    r.h.run_steps(2);
    assert!(!r.h.state().window_title().starts_with("• "), "saved: no mark");
    let s = r.h.state().session().unwrap();
    assert_eq!(s.file.as_deref(), Some(path.as_path()));
    assert!(r.has_toast("Saved Session 0.oxrok"));
    assert_eq!(r.h.state().settings().files.recent_files.first(), Some(&path));

    r.open(&path);
    let s = r.h.state().session().unwrap();
    assert!(s.is_read_only());
    assert!(!s.is_unsaved(), "a file just opened is saved");
    assert_eq!(s.notes, "bench run");
    assert_eq!(s.rows[0].name, "CLK");
    assert_eq!(s.markers.items.len(), 1);
    assert_eq!(levels(r.h.state(), ChannelId::digital(1)), before_d1, "samples survive the round trip");
    r.h.run();
    r.h.get_by_label("Opened from Session 0.oxrok");
    // The banner closes into a toolbar icon and comes back;
    // closing is per session, so the next open shows it again.
    r.h.get_by_label("Hide banner").click();
    r.h.run();
    assert!(r.h.query_by_label("Opened from Session 0.oxrok").is_none());
    r.h.get_by_label("Opened from a file").click();
    r.h.run();
    r.h.get_by_label("Opened from Session 0.oxrok");
    r.h.get_by_label("Hide banner").click();
    r.h.run();
    let first = r.h.state().session().unwrap().id;
    r.open(&path);
    r.until("opened again", move |a| a.session().is_some_and(|s| s.id != first));
    r.h.run();
    r.h.get_by_label("Opened from Session 0.oxrok");

    // Recording asks first; Cancel keeps the file's data.
    r.h.key_press(Key::Space);
    r.h.run();
    r.h.get_by_label("Record on Session 0.oxrok?");
    r.h.get_by_label("Cancel").click();
    r.h.run();
    assert_eq!(r.h.state().session().unwrap().state, CaptureState::Finished);
    assert!(r.h.state().session().unwrap().is_read_only());

    // A new marker makes it unsaved; Save writes it back without a dialog.
    r.h.state_mut().session_mut().unwrap().markers.add_single(CaptureTime::from_secs_f64(0.010));
    assert!(r.h.state().session().unwrap().is_unsaved());
    *r.dialogs.save.lock() = Some(r.path("must-not-be-used.oxrok"));
    r.save();
    assert!(!r.path("must-not-be-used.oxrok").exists(), "Save writes in place");
    r.dialogs.save.lock().take();
    let meta: oxrok_ui::app::file::SessionMeta =
        serde_json::from_slice(&oxrok_ui::file::load_meta(&path).unwrap()).unwrap();
    assert_eq!(meta.annotations.markers.items.len(), 2);
}

/// Save As always asks, and the session follows the new file.
#[test]
fn save_as_asks_for_a_new_file() {
    let mut r = Rig::new();
    r.record(10);
    r.save();
    let copy = r.path("copy.oxrok");
    *r.dialogs.save.lock() = Some(r.path("copy"));
    r.h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::S);
    r.until("saved as", |a| a.session().unwrap().file.as_ref().is_some_and(|f| f.ends_with("copy.oxrok")));
    assert!(copy.exists(), ".oxrok is appended");
    // The tab follows the file name.
    assert_eq!(r.h.state().session().unwrap().name, "copy");
}

/// A new capture over an unsaved one is replaced with a toast;
/// closing the session asks "Save Capture?"; quitting with two unsaved
/// sessions lists both; after Save the dot is gone.
#[test]
fn unsaved_captures() {
    let mut r = Rig::new();
    r.record(10);
    r.h.get_by_label("Session 0").hover();
    r.h.run();
    r.h.get_by_label("Not saved");
    r.record(10);
    assert!(r.has_toast("Previous capture replaced"));

    // Closing asks; Cancel keeps it.
    r.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::W);
    r.h.run();
    r.h.get_by_label("Save Capture?");
    r.h.get_by_label("Session 0 has a capture that hasn't been saved.");
    r.h.get_by_label("Cancel").click();
    r.h.run();
    assert_eq!(r.h.state().session_count(), 1);

    // A second unsaved session: quitting lists both.
    r.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::T);
    r.until("session 1 open", |a| a.session().is_some_and(|s| s.name == "Session 1" && s.caps.is_some()));
    r.record(10);
    r.h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    r.h.run_steps(3);
    r.h.get_by_label("Save Captures?");
    r.h.get_by_label("2 captures haven't been saved:");
    r.h.get_by_label("• Session 0");
    r.h.get_by_label("• Session 1");
    r.h.get_by_label("Cancel").click();
    r.h.run();

    r.save();
    r.h.get_by_label("Session 1 (2 sessions)").hover();
    r.h.run();
    assert!(r.h.query_by_label("Not saved").is_none(), "the dot is gone");
}

/// With the preference on, replacing asks; Save… saves first.
#[test]
fn replace_capture_can_ask() {
    let mut r = Rig::new();
    r.record(10);
    r.h.state_mut().settings_mut().capture.ask_replace_unsaved = true;
    r.h.key_press(Key::Space);
    r.h.run();
    r.h.get_by_label("Replace Capture?");
    r.h.get_by_label("Save…").click();
    r.until("recording after save", |a| a.session().unwrap().state == CaptureState::Recording);
    assert!(r.path("Session 0.oxrok").exists());
}

/// Open Recent lists files newest first; missing ones drop out.
#[test]
fn recent_files() {
    let mut r = Rig::new();
    r.record(10);
    *r.dialogs.save.lock() = Some(r.path("a.oxrok"));
    r.save();
    *r.dialogs.save.lock() = Some(r.path("b.oxrok"));
    r.h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::S);
    r.until("saved b", |a| a.session().unwrap().file.as_ref().is_some_and(|f| f.ends_with("b.oxrok")));
    let recent = r.h.state().settings().files.recent_files.clone();
    assert_eq!(recent, [r.path("b.oxrok"), r.path("a.oxrok")]);
}

/// Files of unknown formats are refused with a toast.
#[test]
fn unsupported_files_are_refused() {
    let mut r = Rig::new();
    r.h.run();
    let p = r.path("notes.txt");
    std::fs::write(&p, "hello").unwrap();
    *r.dialogs.open.lock() = vec![p];
    r.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::O);
    r.until("toast", |a| a.toasts().iter().any(|t| t.body.contains("isn't a supported capture file")));
    r.h.run();
    r.h.get_by_label("Supported formats…").click();
    r.h.run();
    assert!(r.has_toast("Supported formats"), "the formats are listed");
}

/// Cmd/Ctrl+N opens Notes; what is typed is the session's notes, saved
/// with the capture.
#[test]
fn notes_panel() {
    let mut r = Rig::new();
    r.record(10);
    r.save();
    r.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::N);
    r.h.run();
    r.h.get_by_role_and_label(egui::Role::TextInput, "Capture notes").focus();
    r.h.run();
    r.h.get_by_role_and_label(egui::Role::TextInput, "Capture notes").type_text("probe on J3");
    r.h.run();
    let s = r.h.state().session().unwrap();
    assert_eq!(s.notes, "probe on J3");
    assert!(s.is_unsaved(), "notes are saved with the capture");
    if std::env::var_os("LA_SHOT").is_some() {
        r.h.render().unwrap().save("/private/tmp/claude-501/-Users-ajo-work-ai-logicanalyzer/b6f2a53a-7940-4097-8671-d2b2ced3bc2e/scratchpad/notes.png").unwrap();
    }
}

/// A reopened capture's analyzers decode the next recording made with
/// New Capture with These Settings: they are registered with
/// the decoder thread, not just copied.
#[test]
fn analyzers_decode_after_new_capture_with_these_settings() {
    use oxrok_ui::decode::runner::Status;
    let mut r = Rig::new();
    r.record(20);
    // A UART analyzer on D1 (the mock's UART line).
    r.h.key_press_modifiers(egui::Modifiers::ALT, Key::Num3);
    r.h.run();
    r.h.get_by_role_and_label(egui::Role::MenuItem, "Async Serial").click();
    r.h.run_steps(10);
    r.h.get_by_label("RX channel").click();
    r.h.run_steps(3);
    r.h.get_by_label("01.  D1").click();
    r.h.run_steps(3);
    r.h.get_by_label("Save").click();
    r.h.run_steps(5);
    let done = |a: &LaApp| {
        a.session()
            .unwrap()
            .analyzers
            .first()
            .is_some_and(|x| *x.out.status.lock() == Status::Complete && x.out.frames.read().order().len() > 3)
    };
    r.until("decoded", done);
    r.save();
    let path = r.path("Session 0.oxrok");
    r.open(&path);
    r.until("re-decoded after opening", done);
    r.h.run();
    // Choosing never records; Record then follows the choice.
    r.h.get_by_label("New capture with these settings").click();
    r.h.run_steps(3);
    assert!(r.h.state().session().unwrap().is_read_only(), "choosing doesn't record");
    r.h.key_press(Key::Space);
    r.until("device session", |a| a.session().is_some_and(|s| !s.is_read_only() && s.caps.is_some()));
    assert_eq!(r.h.state().session().unwrap().analyzers.len(), 1, "the analyzer is copied");
    // It records as soon as the device is open.
    r.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
    r.clock.advance(Duration::from_millis(20));
    r.until("data", |a| {
        a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|x| x.1 >= RATE / 50)
    });
    r.h.key_press(Key::Space);
    r.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    r.until("decoded in the new recording", done);
}

/// Cmd+Q asks the window to close; closing with an unsaved
/// capture cancels the close and asks "Save Captures?".
#[test]
fn quit_asks_about_unsaved_captures() {
    let mut r = Rig::new();
    r.record(10);
    // The press, queued by hand so its frame's output can be read.
    r.h.event(egui::Event::Key {
        key: Key::Q,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    r.h.step();
    let closes =
        r.h.output()
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .any(|c| matches!(c, egui::ViewportCommand::Close));
    assert!(closes, "Cmd+Q requests a close");
    // The windowing layer answers with a close request.
    r.h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    r.h.step();
    let cancelled =
        r.h.output()
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .any(|c| matches!(c, egui::ViewportCommand::CancelClose));
    assert!(cancelled, "the close waits for the answer");
    r.h.run_steps(2);
    r.h.get_by_label("Save Captures?");
    r.h.get_by_label("Cancel").click();
    r.h.run_steps(2);
    assert!(r.h.query_by_label("Save Captures?").is_none());
}

/// The banner's choice is noted, not acted on; Record then
/// records here without asking.
#[test]
fn banner_choice_applies_at_record() {
    use egui::accesskit::Toggled;
    let mut r = Rig::new();
    r.record(10);
    r.save();
    let path = r.path("Session 0.oxrok");
    r.open(&path);
    let on = |r: &Rig, l: &str| {
        r.h.get_by_role_and_label(egui::Role::RadioButton, l).accesskit_node().toggled() == Some(Toggled::True)
    };
    assert!(on(&r, "I will decide later"), "from the preference (Ask)");
    r.h.snapshot("file_banner");
    r.h.get_by_label("Will record here").click();
    r.h.run_steps(3);
    // The choice closes the banner; the toolbar icon reopens it.
    assert!(r.h.query_by_label("Will record here").is_none(), "banner closed");
    r.h.get_by_label("Opened from a file").click();
    r.h.run_steps(3);
    assert!(on(&r, "Will record here"));
    // A narrow window: the name is cut with "…", the choices get short
    // labels, nothing overlaps.
    r.h.set_size(egui::vec2(1000.0, 800.0));
    r.h.run_steps(5);
    r.h.snapshot("file_banner_narrow");
    assert!(on(&r, "Will record here"), "the full name stays the accessible label");
    r.h.set_size(egui::vec2(1280.0, 800.0));
    r.h.run_steps(5);
    let s = r.h.state().session().unwrap();
    assert!(s.is_read_only() && s.state == CaptureState::Finished, "nothing recorded yet");
    r.h.key_press(Key::Space);
    r.until("recording here", |a| a.session().unwrap().state == CaptureState::Recording);
    assert!(r.h.query_by_label_contains("Record on").is_none(), "no question");
    assert_eq!(r.h.state().session().unwrap().file.as_deref(), Some(path.as_path()));
}

/// Recording on an opened `.oxrok` asks; Replace Data records into
/// the same session, which keeps its file; New Session with Remember my
/// choice doesn't ask again.
#[test]
fn record_on_an_opened_file() {
    let mut r = Rig::new();
    r.record(10);
    r.h.state_mut().session_mut().unwrap().notes = "bench".into();
    r.save();
    let path = r.path("Session 0.oxrok");
    let before = std::fs::metadata(&path).unwrap().len();
    r.open(&path);

    // Replace Data: the same session records, keeping name, notes, file.
    r.h.key_press(Key::Space);
    r.h.run();
    r.h.get_by_label(
        "Saving afterwards overwrites Session 0.oxrok.".replace("Saving", "With Replace Data, saving").as_str(),
    );
    r.h.get_by_label("Replace Data").click();
    r.until("recording here", |a| a.session().unwrap().state == CaptureState::Recording);
    let s = r.h.state().session().unwrap();
    assert!(!s.is_read_only(), "no longer read-only");
    assert_eq!(s.file.as_deref(), Some(path.as_path()), "still bound to the file");
    assert_eq!(s.notes, "bench");
    assert_eq!(s.name, "Session 0");
    r.clock.advance(Duration::from_millis(30));
    r.until("data", |a| {
        a.session()
            .unwrap()
            .capture
            .as_ref()
            .and_then(|c| c.read().sample_range())
            .is_some_and(|x| x.1 >= RATE * 30 / 1000)
    });
    r.h.key_press(Key::Space);
    r.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    assert!(r.h.state().session().unwrap().is_unsaved());
    // Save writes the new capture into the same file, without a dialog.
    *r.dialogs.save.lock() = Some(r.path("must-not-be-used.oxrok"));
    r.save();
    assert!(!r.path("must-not-be-used.oxrok").exists());
    r.dialogs.save.lock().take();
    // The file now holds the 30 ms recording, not the first 10 ms.
    let loaded = oxrok_ui::file::load(&path, &mut |_| true).unwrap();
    let (b, e) = loaded.digital[0].1.data_range().unwrap();
    assert!(e - b >= RATE * 30 / 1000, "{} samples", e - b);
    let _ = before;

    // New Session, remembered: the next recording doesn't ask.
    r.open(&path);
    r.h.key_press(Key::Space);
    r.h.run();
    r.h.get_by_label("Remember my choice").click();
    r.h.run();
    r.h.get_by_label("New Session").click();
    r.until("new session recording", |a| {
        a.session().is_some_and(|s| !s.is_read_only() && s.file.is_none() && s.state == CaptureState::Recording)
    });
    assert_eq!(r.h.state().settings().capture.record_on_opened, oxrok_ui::app::settings::RecordOnOpened::NewSession);
    r.clock.advance(Duration::from_millis(10));
    r.until("data", |a| {
        a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|x| x.1 >= RATE / 100)
    });
    r.h.key_press(Key::Space);
    r.until("stopped", |a| a.session().unwrap().state == CaptureState::Finished);
    r.save();
    r.open(&path);
    r.h.key_press(Key::Space);
    r.until("recording without asking", |a| a.session().is_some_and(|s| s.state == CaptureState::Recording));
    assert!(r.h.query_by_label("Remember my choice").is_none());
}
