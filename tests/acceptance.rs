#![cfg(feature = "mock")]
//! Exit criteria: single session, pan only, CSV only.

use egui::accesskit::Toggled;
use egui::{Event, Key, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2, TouchPhase, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::core::{ManualClock, ViewState};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Default mock rate (highest ≤ 10 MS/s).
const RATE: u64 = 8_000_000;
/// Window layout at 1280 × 800, Inspector open on the left.
const LABEL_W: f32 = 105.0;
const INSPECTOR_W: f32 = 300.0;
const PLOT_POINT: Pos2 = Pos2::new(500.0, 110.0);

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
}

fn rig_with(export_dir: Option<PathBuf>) -> Rig {
    let clock = ManualClock::new();
    let c = clock.clone();
    let h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
        LaApp::with_options(
            cc,
            AppOptions {
                backend: Arc::new(MockBackend::new(options)),
                clock: Arc::new(c),
                dialogs: Arc::new(FixedDialogs(export_dir)),
                settings_path: None,
                animation_speed: 0.0,
                scan_sync: true,
                profiles_dir: None,
                extra_builtin_profiles: Vec::new(),
            },
        )
    });
    Rig { h, clock }
}

impl Rig {
    /// Steps until `cond` holds (backend events arrive asynchronously).
    fn until(&mut self, what: &str, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.step();
    }

    fn view(&self) -> ViewState {
        self.h.state().session().unwrap().view.state()
    }

    /// Start screen → Mock 16 digital + 4 analog in the Devices list.
    fn open_demo(&mut self) {
        self.h.run();
        self.h.get_by_label("Mock 16 digital + 4 analog").click();
        self.until("device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    }

    /// Records `secs` of mock data (flow).
    fn capture(&mut self, secs: u64) {
        self.h.key_press(Key::Space);
        self.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
        self.clock.advance(Duration::from_secs(secs));
        let want = secs * RATE;
        self.until("data", move |a| {
            a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|r| r.1 >= want)
        });
        self.h.key_press(Key::Space);
        self.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    }

    fn time_at_px(&self, x: f32) -> oxrok_ui::core::CaptureTime {
        self.view().px_to_time((x - LABEL_W - INSPECTOR_W) as f64)
    }
}

fn wheel(unit: MouseWheelUnit, delta: egui::Vec2) -> Event {
    let source = if unit == MouseWheelUnit::Point { MouseWheelSource::Trackpad } else { MouseWheelSource::Wheel };
    Event::MouseWheel { unit, delta, phase: TouchPhase::Move, modifiers: Modifiers::NONE, source }
}

#[test]
fn open_demo_device() {
    let mut r = rig_with(None);
    r.open_demo();
    let s = r.h.state().session().unwrap();
    assert_eq!(s.name, "Session 0");
    // Default 2 channels per type.
    for row in ["D0", "D1", "A0", "A1"] {
        r.h.get_by_role_and_label(egui::Role::Label, row);
    }
    assert!(r.h.query_by_role_and_label(egui::Role::Label, "D2").is_none());
    r.h.get_by_label("Start Demo");
}

#[test]
fn channel_chips_toggle_rows() {
    let mut r = rig_with(None);
    r.open_demo();
    // The Inspector starts closed: open it on the Device tab.
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
    r.h.run_steps(3);
    let chip = r.h.get_by_role_and_label(egui::Role::CheckBox, "D3");
    assert_eq!(chip.accesskit_node().toggled(), Some(Toggled::False));
    chip.click();
    r.h.run();
    r.h.get_by_role_and_label(egui::Role::Label, "D3");
    assert_eq!(r.h.get_by_role_and_label(egui::Role::CheckBox, "D3").accesskit_node().toggled(), Some(Toggled::True));

    // The Digital section's Clear (the first one).
    r.h.get_all_by_label("Clear").next().unwrap().click();
    r.h.run();
    for i in 0..16 {
        let name = format!("D{i}");
        let chip = r.h.get_by_role_and_label(egui::Role::CheckBox, &name);
        assert_eq!(chip.accesskit_node().toggled(), Some(Toggled::False), "chip {i}");
    }
    assert!(r.h.query_by_role_and_label(egui::Role::Label, "D0").is_none());
    // Analog channels are unaffected.
    r.h.get_by_role_and_label(egui::Role::Label, "A0");

    r.h.get_all_by_label("All").next().unwrap().click();
    r.h.run();
    // 16 + 2 rows overflow the window; only the ones in view are on screen.
    r.h.get_by_role_and_label(egui::Role::Label, "D0");
    assert_eq!(r.h.state().session().unwrap().visible_rows().count(), 18);
}

#[test]
fn record_with_r() {
    let mut r = rig_with(None);
    r.open_demo();
    r.h.key_press(Key::Space);
    r.until("recording", |a| a.session().unwrap().state == CaptureState::Recording);
    r.h.get_by_label("Stop");
    let live = r.h.get_by_role_and_label(egui::Role::Button, "Live");
    assert_eq!(live.accesskit_node().toggled(), Some(Toggled::True), "following latest data");
    r.h.get_by_label("Memory");

    r.clock.advance(Duration::from_secs(2));
    r.until("2 s of data", |a| {
        a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|x| x.1 >= 2 * RATE)
    });
    r.h.key_press(Key::Space);
    r.until("finished", |a| a.session().unwrap().state == CaptureState::Finished);
    r.h.get_by_label("Start Demo");
    assert!(r.h.query_by_label("Live").is_none());

    // Fixed view: fit all, then zoom in 9 times about the center (~3.9 ms).
    r.h.key_press(Key::F);
    r.h.run();
    for _ in 0..9 {
        r.h.key_press(Key::Equals);
        r.h.run();
    }
    r.h.remove_cursor();
    r.h.run();
    r.h.snapshot("d0_after_capture");
}

#[test]
fn wheel_zooms_about_pointer_and_touchpad_pans() {
    let mut r = rig_with(None);
    r.open_demo();
    // The Inspector starts closed: open it, so the plot starts past it.
    r.h.key_press_modifiers(Modifiers::ALT, Key::Num1);
    r.h.run_steps(3);
    r.capture(1);
    r.h.key_press(Key::F);
    r.h.run();

    r.h.hover_at(PLOT_POINT);
    r.h.run();
    let t = r.time_at_px(PLOT_POINT.x);
    let scale = r.view().time_scale();
    r.h.event(wheel(MouseWheelUnit::Line, vec2(0.0, 1.0)));
    r.h.run();
    assert!(r.view().time_scale() < scale, "zoomed in");
    let x = r.view().time_to_px(t) as f32 + LABEL_W + INSPECTOR_W;
    assert!((x - PLOT_POINT.x).abs() < 0.5, "pointer time stays under the pointer ({x})");

    // Touchpad: a horizontal swipe pans (after 5 events of history, rule 5).
    let left = r.view().left_time;
    for _ in 0..8 {
        r.h.event(wheel(MouseWheelUnit::Point, vec2(-20.0, 0.0)));
        r.h.run();
    }
    assert!(r.view().left_time > left, "content moved left → later times");
}

#[test]
fn drag_pans_one_to_one() {
    let mut r = rig_with(None);
    r.open_demo();
    r.capture(1);
    r.h.key_press(Key::F);
    r.h.run();
    // Zoom in so there is room to pan.
    for _ in 0..4 {
        r.h.key_press(Key::Equals);
        r.h.run();
    }
    let before = r.view();
    let start = PLOT_POINT;
    let end = start + vec2(120.0, 0.0);
    r.h.hover_at(start);
    r.h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    r.h.run();
    for k in 1..=6 {
        r.h.hover_at(start + vec2(20.0 * k as f32, 0.0));
        r.h.run();
    }
    r.h.drop_at(end);
    r.h.run();
    let after = r.view();
    let moved_px = before.time_to_px(after.left_time);
    assert!((moved_px + 120.0).abs() < 1.0, "dragging right by 120 pt shows earlier data 1:1 ({moved_px})");
}

#[test]
fn fit_and_edges() {
    let mut r = rig_with(None);
    r.open_demo();
    r.capture(1);
    let bounds = r.h.state().session().unwrap().bounds().unwrap();

    r.h.key_press(Key::F);
    r.h.run();
    let v = r.view();
    assert_eq!(v.left_time, bounds.begin);
    assert!((v.right_time().secs_since(bounds.end)).abs() < 1e-9);

    for _ in 0..6 {
        r.h.key_press(Key::Equals);
        r.h.run();
    }
    r.h.key_press(Key::Home);
    r.h.run();
    assert_eq!(r.view().left_time, bounds.begin);
    r.h.key_press(Key::End);
    r.h.run();
    assert!((r.view().right_time().secs_since(bounds.end)).abs() < 1e-9);
}

#[test]
fn export_digital_csv() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = rig_with(Some(dir.path().to_path_buf()));
    r.open_demo();
    r.capture(1);

    // The dialog opens ready (last folder, dated file name), so
    // Enter exports.
    r.h.state_mut().settings_mut().last_dirs.export = Some(dir.path().to_path_buf());
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::E);
    r.h.run();
    r.h.get_by_label("Export Raw Data");
    r.h.key_press(Key::Enter);
    r.until("export complete", |a| a.toasts().iter().any(|t| t.heading.as_deref() == Some("Export complete")));
    let path = exported(dir.path(), " digital.csv");
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("Session 0 20"), "{name}");

    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("Time [s],D0,D1"));
    // Every row matches the mock signals at that sample (D0 clock, D1 UART).
    let mut rows = 0;
    for line in lines.by_ref().take(2000) {
        let mut f = line.split(',');
        let t: f64 = f.next().unwrap().parse().unwrap();
        let sample = (t * RATE as f64).round() as u64;
        for ch in 0..2u16 {
            let v = f.next().unwrap() == "1";
            assert_eq!(v, oxrok_ui::backend::mock::level(1, ch, sample, RATE), "row {line}");
        }
        rows += 1;
    }
    assert!(rows > 100);
    assert!(text.starts_with("Time [s],D0,D1\n0.000000000,1,0\n"));
}

/// Drag a row's bottom border in the label column; the height
/// follows live and snaps to 5 pt on release, never below the minimum.
#[test]
fn row_resize_snaps_to_5pt() {
    let mut r = rig_with(None);
    r.open_demo();
    let height = |r: &Rig| r.h.state().session().unwrap().row(oxrok_ui::core::ChannelId::digital(0)).unwrap().height;
    assert_eq!(height(&r), 25.0, "compact default ()");
    let label = r.h.get_by_role_and_label(egui::Role::Label, "D0").rect();
    let start = egui::pos2(label.center().x, label.bottom() - 1.0);
    r.h.hover_at(start);
    r.h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    r.h.run();
    for dy in [10.0, 20.0, 37.0] {
        r.h.hover_at(start + vec2(0.0, dy));
        r.h.run();
    }
    assert!((height(&r) - 62.0).abs() < 0.5, "live height {}", height(&r));
    r.h.drop_at(start + vec2(0.0, 37.0));
    r.h.run();
    assert_eq!(height(&r), 60.0, "snapped to 5 pt");

    // Dragging far up stops at the minimum.
    let label = r.h.get_by_role_and_label(egui::Role::Label, "D0").rect();
    let start = egui::pos2(label.center().x, label.bottom() - 1.0);
    r.h.hover_at(start);
    r.h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    r.h.run();
    r.h.hover_at(start - vec2(0.0, 200.0));
    r.h.run();
    r.h.drop_at(start - vec2(0.0, 200.0));
    r.h.run();
    assert_eq!(height(&r), 20.0);
}

/// The one file in `dir` whose name ends with `suffix`.
fn exported(dir: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let found: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(suffix))
        .collect();
    assert_eq!(found.len(), 1, "files ending in {suffix:?}: {found:?}");
    found[0].clone()
}

/// Types `name` into the export dialog's File name field, replacing it.
/// The field may be scrolled out of view in the 800 pt window, so it is
/// focused through accessibility rather than clicked.
fn set_export_name(r: &mut Rig, name: &str) {
    r.h.get_by_role_and_label(egui::Role::TextInput, "File name").focus();
    r.h.run();
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    r.h.key_press(Key::Backspace);
    r.h.get_by_role_and_label(egui::Role::TextInput, "File name").type_text(name);
    r.h.run();
}

/// Presses Enter in the export dialog and waits for `file` to be written.
fn export_and_wait(r: &mut Rig, file: &std::path::Path) {
    r.h.key_press(Key::Enter);
    // The toast names the last file written, once the export is done.
    let name = file.display().to_string();
    r.until("export complete", move |a| {
        a.toasts().iter().any(|t| t.heading.as_deref() == Some("Export complete") && t.body.starts_with(&name))
    });
}

/// (all formats): Binary over a range measurement, VCD over all time,
/// and the Replace Files confirmation (032).
#[test]
fn binary_vcd_ranges_and_replace() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = rig_with(Some(dir.path().to_path_buf()));
    r.open_demo();
    r.capture(1);
    let t = |s: f64| oxrok_ui::core::CaptureTime::from_secs_f64(s);
    r.h.state_mut().session_mut().unwrap().ranges.add(oxrok_ui::core::ChannelId::digital(0), t(0.1), t(0.2));
    {
        let s = r.h.state_mut().settings_mut();
        s.last_dirs.export = Some(dir.path().to_path_buf());
        s.export.format = oxrok_ui::app::settings::ExportFormat::Binary;
        s.export.range = oxrok_ui::app::settings::ExportRange::Measurement;
    }
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::E);
    r.h.run();
    set_export_name(&mut r, "bin");
    export_and_wait(&mut r, &dir.path().join("bin A1.bin"));
    let names: Vec<String> =
        std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    let bytes = std::fs::read(dir.path().join("bin D0.bin")).unwrap_or_else(|e| panic!("{e}; files: {names:?}"));
    assert_eq!(&bytes[0..8], b"LACAPBIN");
    let begin = f64::from_le_bytes(bytes[24..32].try_into().unwrap());
    assert!((begin - 0.1).abs() < 1e-6, "the range starts at 0.1 s: {begin}");
    let count = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    // D0 is a 1 kHz clock: 200 edges in 0.1 s (± the edge at the end).
    assert!((199..=201).contains(&count), "{count}");

    // VCD over all time.
    {
        let s = r.h.state_mut().settings_mut();
        s.export.format = oxrok_ui::app::settings::ExportFormat::Vcd;
        s.export.range = oxrok_ui::app::settings::ExportRange::All;
    }
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::E);
    r.h.run();
    set_export_name(&mut r, "dump");
    export_and_wait(&mut r, &dir.path().join("dump.vcd"));
    let vcd = std::fs::read_to_string(dir.path().join("dump.vcd")).unwrap();
    assert!(vcd.contains("$timescale 1 ns $end"));
    assert!(vcd.contains("$var wire 1 ! D0 $end"));
    assert!(vcd.contains("$enddefinitions $end"));

    // The same name again: Replace Files asks first.
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::E);
    r.h.run();
    set_export_name(&mut r, "dump");
    r.h.key_press(Key::Enter);
    r.h.run();
    r.h.get_by_label("Replace Files");
    let written = std::fs::metadata(dir.path().join("dump.vcd")).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    r.h.get_by_label("Replace").click();
    let p = dir.path().join("dump.vcd");
    r.until("replaced", move |_| std::fs::metadata(&p).and_then(|m| m.modified()).is_ok_and(|t| t > written));
}
