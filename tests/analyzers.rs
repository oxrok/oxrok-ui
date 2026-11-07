#![cfg(feature = "mock")]
//! Analyzers, bubbles, the data table and the terminal, with the mock decoders.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::capture::{CaptureMode, CaptureState};
use oxrok_ui::core::{CaptureTime, ChannelId, ManualClock};
use oxrok_ui::decode::runner::Status;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RATE: u64 = 8_000_000;

struct Rig {
    h: Harness<'static, LaApp>,
    clock: ManualClock,
}

impl Rig {
    /// The 16-channel mock with `ms` milliseconds recorded.
    fn new(ms: u64) -> Self {
        Self::new_in(ms, None)
    }

    /// With file dialogs that answer `dir`.
    fn new_in(ms: u64, dir: Option<std::path::PathBuf>) -> Self {
        let clock = ManualClock::new();
        let c = clock.clone();
        let mut h = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
            let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::new(options)),
                    clock: Arc::new(c),
                    dialogs: Arc::new(FixedDialogs(dir.clone())),
                    settings_path: None,
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: None,
                    extra_builtin_profiles: Vec::new(),
                    mock_decoders: cfg!(feature = "mock"),
                },
            )
        });
        h.run();
        h.get_by_label("Mock 16 digital + 4 analog").click();
        let mut r = Rig { h, clock: clock.clone() };
        r.until(|a| a.session().is_some_and(|s| s.caps.is_some()));
        r.h.key_press(Key::Space);
        r.until(|a| a.session().unwrap().state == CaptureState::Recording);
        clock.advance(Duration::from_millis(ms));
        let want = ms * RATE / 1000;
        r.until(move |a| {
            a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|r| r.1 >= want)
        });
        r.h.key_press(Key::Space);
        r.until(|a| a.session().unwrap().state == CaptureState::Finished);
        r
    }

    fn until(&mut self, cond: impl Fn(&LaApp) -> bool) {
        let t0 = Instant::now();
        while !cond(self.h.state()) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out");
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.run_steps(3);
    }

    /// Opens the Analyzers tab of the Inspector.
    fn open_panel(&mut self) {
        self.h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::Num2);
        self.h.run_steps(3);
    }

    /// Analyzers panel → Async Serial → RX = D1 → Save.
    fn add_uart(&mut self) {
        self.open_panel();
        self.h.get_by_role_and_label(egui::Role::MenuItem, "Async Serial").click();
        self.h.run_steps(10);
        self.h.get_by_label("RX channel").click();
        self.h.run_steps(3);
        self.h.get_by_label("01.  D1").click();
        self.h.run_steps(3);
        self.h.get_by_label("Save").click();
        self.h.run_steps(5);
        self.until(|a| {
            a.session().unwrap().analyzers.first().is_some_and(|x| *x.out.status.lock() == Status::Complete)
        });
    }

    fn texts(&self) -> Vec<String> {
        let s = self.h.state().session().unwrap();
        let out = &s.analyzers[0].out;
        let f = out.frames.read();
        f.order().iter().filter_map(|&i| f.get(i)).filter(|f| f.class == 0).map(|f| f.longest().to_owned()).collect()
    }
}

/// (first part): the dialog validates, then bubbles read the text.
#[test]
fn async_serial() {
    let mut r = Rig::new(20);
    r.open_panel();
    r.h.get_by_role_and_label(egui::Role::MenuItem, "Async Serial").click();
    r.h.run_steps(10);
    r.h.get_by_label("Save").click();
    r.h.run_steps(5);
    r.h.get_by_label("!  Invalid channel(s)");
    r.h.get_by_label("Cancel").click();
    r.h.run_steps(5);
    r.add_uart();
    let text: String = r.texts().concat();
    assert!(text.starts_with("Hello, world!"), "{text:?}");
    // Zoomed onto the first characters, the bubbles show them.
    let first = {
        let s = r.h.state().session().unwrap();
        let f = s.analyzers[0].out.frames.read();
        let (_, h) = f.by_order(0).unwrap();
        (h.start, h.end)
    };
    let _ = first;
    r.h.key_press(Key::Home);
    for _ in 0..9 {
        r.h.key_press(Key::Equals);
        r.h.run_steps(1);
    }
    r.h.key_press(Key::Home);
    r.h.run_steps(3);
    r.h.get_by_role_and_label(egui::Role::Label, "H");
    r.h.get(By::new().role(egui::Role::Label).label("Async Serial complete"));
    if std::env::var_os("LA_SHOT").is_some() {
        r.h.remove_cursor();
        r.h.run_steps(2);
        r.h.render().unwrap().save("/tmp/claude-501/fq/a60.png").unwrap();
    }
    let _ = ChannelId::digital(1);
}

impl Rig {
    /// Zooms onto the first characters (Home, then 9 zoom steps).
    fn zoom_to_start(&mut self) {
        self.h.key_press(Key::Home);
        for _ in 0..9 {
            self.h.key_press(Key::Equals);
            self.h.run_steps(1);
        }
        self.h.key_press(Key::Home);
        self.h.run_steps(3);
    }

    /// The labels of the table rows on screen.
    fn table_rows(&self) -> Vec<String> {
        self.h
            .query_all(By::new().role(egui::Role::Row))
            .map(|n| {
                let a = n.accesskit_node();
                a.label().or(a.value()).unwrap_or_default().to_owned()
            })
            .collect()
    }

    fn search(&mut self, q: &str) {
        self.h.get_by_label("Search table").click();
        self.h.run_steps(2);
        self.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
        self.h.event(egui::Event::Text(q.into()));
        self.h.run_steps(2);
        // Applied after a 250 ms debounce (app clock).
        self.clock.advance(Duration::from_millis(300));
        self.h.run_steps(4);
    }
}

/// Time order, search `0x6C` keeps the 'l' rows, go-to centers.
#[test]
fn table_order_search_and_goto() {
    let mut r = Rig::new(20);
    r.add_uart();
    let rows = r.table_rows();
    assert!(rows.len() > 5, "{rows:?}");
    let s = r.h.state().session().unwrap();
    let f = s.analyzers[0].out.frames.read();
    let starts: Vec<u64> = f.order().iter().map(|&i| f.get(i).unwrap().start).collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]), "time order");
    drop(f);
    r.search("0x6C");
    let rows = r.table_rows();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|t| t.starts_with("rx-data l ")), "{rows:?}");
    // Go to the first match: the view centers on it.
    let first = r.h.query_all(By::new().role(egui::Role::Row)).next().unwrap().rect().center();
    r.h.hover_at(first);
    r.h.run_steps(2);
    r.h.get_by_label("Go to data").click();
    r.h.run_steps(4);
    let s = r.h.state().session().unwrap();
    let center = s.view.state().center_time().to_sample_floor(RATE) as u64;
    let f = s.analyzers[0].out.frames.read();
    let hit = f.order().iter().filter_map(|&i| f.get(i)).find(|f| f.class == 0 && f.longest() == "l").unwrap();
    assert!((hit.start..=hit.end).contains(&center), "{center} in {}..{}", hit.start, hit.end);
}

/// Ctrl/Cmd+click a bubble selects its table row.
#[test]
fn bubble_reveals_table_row() {
    let mut r = Rig::new(20);
    r.add_uart();
    r.zoom_to_start();
    let e = r.h.get_by_role_and_label(egui::Role::Label, "e").rect().center();
    r.h.hover_at(e);
    r.h.event(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
    r.h.event(egui::Event::PointerButton {
        pos: e,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::COMMAND,
    });
    r.h.event(egui::Event::PointerButton {
        pos: e,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    r.h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    r.h.run_steps(6);
    let selected: Vec<String> =
        r.h.query_all(By::new().role(egui::Role::Row))
            .filter(|n| {
                n.accesskit_node().is_selected() == Some(true)
                    || n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
            })
            .map(|n| n.accesskit_node().label().unwrap_or_default().to_owned())
            .collect();
    assert_eq!(selected.len(), 1, "{selected:?}");
    assert!(selected[0].starts_with("rx-data e "), "{selected:?}");
}

/// The terminal shows the text; Clear empties it.
#[test]
fn terminal() {
    let mut r = Rig::new(25);
    r.add_uart();
    r.h.get_by_label("Terminal").click();
    r.h.run_steps(3);
    let t = r.h.get(By::new().label_contains("Terminal: "));
    let a = t.accesskit_node();
    let text = a.label().or(a.value()).unwrap_or_default().to_owned();
    assert!(text.contains("Hello, world!\nHello, world!\n"), "{text:?}");
    let area = t.rect().center();
    r.h.hover_at(area);
    r.h.run_steps(2);
    r.h.get_by_label("Clear Terminal").click();
    r.h.run_steps(3);
    let t = r.h.get(By::new().label_contains("Terminal: "));
    let a = t.accesskit_node();
    assert_eq!(a.label().or(a.value()).unwrap_or_default(), "Terminal: ");
}

/// Hex radix updates the bubbles and the table.
#[test]
fn radix_to_hex() {
    let mut r = Rig::new(20);
    r.add_uart();
    r.h.get_by_label("Async Serial menu").click();
    r.h.run_steps(3);
    r.h.get_by_label("Hexadecimal").click();
    r.h.run_steps(3);
    r.until(|a| a.session().unwrap().analyzers[0].out.status.lock().clone() == Status::Complete);
    r.until(|a| {
        let f = a.session().unwrap().analyzers[0].out.frames.read();
        f.order().iter().filter_map(|&i| f.get(i)).any(|f| f.class == 0 && f.longest() == "48")
    });
    r.zoom_to_start();
    r.h.get_by_role_and_label(egui::Role::Label, "48");
    r.search("0x48");
    let rows = r.table_rows();
    assert!(!rows.is_empty() && rows.iter().all(|t| t.starts_with("rx-data 48 ")), "{rows:?}");
}

impl Rig {
    /// Save As… in the export dialog, then waits for the file.
    fn save_export(&mut self, path: &std::path::Path) -> String {
        self.h.get_by_label("Save As…").click();
        let t0 = Instant::now();
        while !path.exists() || self.h.state().toasts().iter().all(|t| t.heading.as_deref() != Some("Export complete"))
        {
            assert!(t0.elapsed() < Duration::from_secs(10), "no export");
            self.h.step();
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::read_to_string(path).unwrap()
    }
}

/// The kebab's export opens the dialog with that analyzer; the
/// rows go to a CSV file.
#[test]
fn analyzer_export_csv() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Rig::new_in(20, Some(dir.path().to_path_buf()));
    r.add_uart();
    r.h.get_by_label("Async Serial menu").click();
    r.h.run_steps(3);
    r.h.get_by_label("Export to TXT/CSV…").click();
    r.h.run_steps(3);
    r.h.get_by_label("Export Analyzer Data");
    let csv = r.save_export(&dir.path().join("analyzer-data.csv"));
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("Type,Start,Duration,Value,Text"));
    let data: String = lines.filter(|l| l.starts_with("rx-data,")).map(|l| l.rsplit(',').next().unwrap()).collect();
    assert!(data.starts_with("Hello"), "{data}");
}

/// Rows with analyzer lanes stay resizable.
#[test]
fn rows_with_lanes_resize() {
    let mut r = Rig::new(20);
    r.add_uart();
    let d1 = oxrok_ui::core::ChannelId::digital(1);
    let height = |r: &Rig| r.h.state().session().unwrap().row(d1).unwrap().height;
    let before = height(&r);
    let label = r.h.get_by_role_and_label(egui::Role::Splitter, "Resize D1").rect();
    let start = label.center();
    eprintln!("handle {label:?}, height {before}");
    r.h.hover_at(start);
    r.h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    r.h.run_steps(2);
    for dy in [10.0, 20.0, 40.0] {
        r.h.hover_at(start + egui::vec2(0.0, dy));
        r.h.run_steps(2);
    }
    r.h.drop_at(start + egui::vec2(0.0, 40.0));
    r.h.run_steps(3);
    eprintln!("after: {}", height(&r));
    assert!(height(&r) >= before + 35.0, "{before} → {}", height(&r));
}

/// The empty Analyzers panel lists every decoder, filterable.
#[test]
fn empty_panel_lists_all_decoders() {
    let mut r = Rig::new(5);
    r.open_panel();
    for name in ["Async Serial", "I²C", "SPI"] {
        r.h.get_by_role_and_label(egui::Role::MenuItem, name);
    }
    r.h.get_by_label("Filter decoders").click();
    r.h.run_steps(2);
    r.h.event(egui::Event::Text("spi".into()));
    r.h.run_steps(3);
    assert!(r.h.query_by_role_and_label(egui::Role::MenuItem, "I²C").is_none(), "filtered out");
    r.h.get_by_role_and_label(egui::Role::MenuItem, "SPI").click();
    r.h.run_steps(10);
    r.h.get_by_label("CLK channel");
}

/// `>` / `<` center the next / previous result on the hovered row.
#[test]
fn angle_brackets_step_through_results() {
    let mut r = Rig::new(20);
    r.add_uart();
    r.zoom_to_start();
    let e = r.h.get_by_role_and_label(egui::Role::Label, "e").rect().center();
    r.h.hover_at(e);
    r.h.run_steps(2);
    let center = |r: &Rig| {
        let s = r.h.state().session().unwrap();
        s.view.target().center_time().to_sample_floor(RATE) as u64
    };
    let mids = |r: &Rig| -> Vec<u64> {
        let s = r.h.state().session().unwrap();
        let f = s.analyzers[0].out.frames.read();
        f.order().iter().filter_map(|&i| f.get(i)).map(|f| f.start + (f.end - f.start) / 2).collect()
    };
    let c0 = center(&r);
    r.h.event(egui::Event::Text(">".into()));
    r.h.run_steps(30);
    let c1 = center(&r);
    assert!(c1 > c0, "moved forward: {c0} → {c1}");
    assert!(mids(&r).contains(&c1), "centered on a result");
    r.h.event(egui::Event::Text(">".into()));
    r.h.run_steps(30);
    let c2 = center(&r);
    assert!(c2 > c1, "and on to the next: {c1} → {c2}");
    r.h.event(egui::Event::Text("<".into()));
    r.h.run_steps(30);
    assert_eq!(center(&r), c1, "back to the previous one");
}

/// "N matches", the Filter popover (analyzers and searchable
/// columns, with a badge), and Export Table with the search applied.
#[test]
fn table_filter_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Rig::new_in(20, Some(dir.path().to_path_buf()));
    r.add_uart();
    r.search("0x6C");
    let n = r.table_rows().len();
    assert!(n > 0);
    r.h.get_by_label_contains("matches");
    // Value and Text off: "0x6C" no longer matches anything.
    r.h.get_by_label("Filter").click();
    r.h.run_steps(3);
    r.h.get_by_role_and_label(egui::Role::CheckBox, "Value").click();
    r.h.get_by_role_and_label(egui::Role::CheckBox, "Text").click();
    r.h.run_steps(3);
    assert!(r.table_rows().is_empty(), "no searchable column matches");
    r.h.snapshot("table_filter");
    r.h.get_all_by_label("All").nth(1).unwrap().click();
    r.h.run_steps(3);
    assert_eq!(r.table_rows().len(), n);
    // Hiding the analyzer empties the table.
    r.h.get_all_by_label("None").next().unwrap().click();
    r.h.run_steps(3);
    assert!(r.table_rows().is_empty(), "analyzer hidden");
    r.h.get_all_by_label("All").next().unwrap().click();
    r.h.run_steps(3);
    r.h.key_press(Key::Escape);
    r.h.run_steps(2);
    // Export Table, with the search applied: only the 'l' rows.
    r.h.get_by_label("Export Table").click();
    r.h.run_steps(3);
    r.h.get_by_label("Apply table search");
    let csv = r.save_export(&dir.path().join("analyzer-data.csv"));
    let data: Vec<&str> = csv.lines().skip(1).collect();
    assert_eq!(data.len(), n, "{csv}");
    assert!(data.iter().all(|l| l.starts_with("rx-data,") && l.ends_with(",l")), "{csv}");
}

impl Rig {
    /// The starts of the `H` data frames.
    fn h_starts(&self) -> Vec<u64> {
        let s = self.h.state().session().unwrap();
        let f = s.analyzers[0].out.frames.read();
        f.order()
            .iter()
            .filter_map(|&i| f.get(i))
            .filter(|f| f.class == 0 && f.longest() == "H")
            .map(|f| f.start)
            .collect()
    }

    fn type_into(&mut self, role: egui::Role, label: &str, text: &str) {
        self.h.get_by_role_and_label(role, label).click();
        self.h.run_steps(2);
        self.h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
        self.h.event(egui::Event::Text(text.into()));
        self.h.key_press(Key::Enter);
        self.h.run_steps(3);
    }

    /// Records `ms` more milliseconds, stops, and waits for decoding.
    fn record(&mut self, ms: u64) {
        self.h.key_press(Key::Space);
        self.until(|a| a.session().unwrap().state == CaptureState::Recording);
        for _ in 0..ms / 5 {
            self.clock.advance(Duration::from_millis(5));
            self.h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        self.h.key_press(Key::Space);
        self.until(|a| {
            let s = a.session().unwrap();
            s.state == CaptureState::Finished && s.analyzers.iter().all(|x| *x.out.status.lock() == Status::Complete)
        });
        self.h.run_steps(3);
    }
}

/// From a detached Inspector the analyzer dialog is a
/// window of its own (floating here: kittest embeds viewports), so the
/// Inspector's tabs still respond while it is open.
#[test]
fn dialog_beside_detached_inspector() {
    let mut r = Rig::new(20);
    r.open_panel();
    r.h.get_by_label("Detach Inspector").click();
    r.h.run_steps(3);
    r.h.get_by_role_and_label(egui::Role::MenuItem, "Async Serial").click();
    r.h.run_steps(10);
    r.h.get_by_label("RX channel");
    r.h.get_by_label("Markers").click();
    r.h.run_steps(3);
    assert_eq!(r.h.state().settings().inspector.tab.as_str(), "markers", "no modal in the way");
    r.h.get_by_label("RX channel").click();
    r.h.run_steps(3);
    r.h.get_by_label("01.  D1").click();
    r.h.run_steps(3);
    r.h.get_by_label("Save").click();
    r.h.run_steps(5);
    assert!(r.h.query_by_label("RX channel").is_none(), "saved and closed");
    assert_eq!(r.h.state().session().unwrap().analyzers.len(), 1);
}

/// Analyzer Trigger on Async Serial, query `H`: T0 moves to each
/// matching frame; the last one wins.
#[test]
fn analyzer_trigger() {
    let mut r = Rig::new(20);
    r.add_uart();
    let switch =
        |r: &Rig| r.h.get_by_role_and_label(egui::Role::Switch, "Analyzer Trigger").accesskit_node().is_disabled();
    assert!(switch(&r), "no match yet");
    r.h.get_by_label("Analyzer Trigger details").click();
    r.h.run_steps(3);
    r.type_into(egui::Role::TextInput, "Trigger match", "h");
    assert!(!switch(&r));
    r.h.get_by_role_and_label(egui::Role::Switch, "Analyzer Trigger").click();
    r.h.run_steps(3);
    r.h.get_by_label("Armed");
    // The card's look, before the recording: the trigger (and the view
    // following it) moves with every matching frame, so it is not stable
    // across runs.
    r.h.snapshot("analyzer_trigger");

    // Every matching frame moves the trigger; the last one wins.
    r.record(35);
    let hs = r.h_starts();
    assert!(hs.len() >= 3, "{hs:?}");
    let s = r.h.state().session().unwrap();
    assert_eq!(s.trigger, Some(CaptureTime::from_sample(*hs.last().unwrap(), RATE)));
    assert_eq!(s.origin, oxrok_ui::core::markers::TimeOrigin::Trigger);
    r.h.get_by_label_contains("Last trigger at");
}

/// Turning it on outside Continuous switches mode, with Undo.
#[test]
fn analyzer_trigger_switches_to_continuous() {
    let mut r = Rig::new(20);
    r.add_uart();
    r.h.get_by_label("Analyzer Trigger details").click();
    r.h.run_steps(3);
    r.type_into(egui::Role::TextInput, "Trigger match", "H");
    r.h.state_mut().session_mut().unwrap().settings.mode = CaptureMode::Timed;
    r.h.get_by_role_and_label(egui::Role::Switch, "Analyzer Trigger").click();
    r.h.run_steps(3);
    assert_eq!(r.h.state().session().unwrap().settings.mode, CaptureMode::Continuous);
    r.h.get_by_label("Undo").click();
    r.h.run_steps(3);
    let s = r.h.state().session().unwrap();
    assert_eq!(s.settings.mode, CaptureMode::Timed);
    assert!(!s.analyzer_trigger.enabled);
}
