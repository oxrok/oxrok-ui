#![cfg(feature = "mock")]
//! Regenerates the README screenshots in `docs/screenshots/` (2× scale).
//!
//! Not a regression test (that's what snapshots are for), so it is ignored
//! by default. Run it with:
//!
//! ```sh
//! cargo test --test screenshots -- --ignored
//! ```

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use oxrok_ui::core::ManualClock;
use oxrok_ui::core::capture::CaptureState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/screenshots");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn save(h: &mut Harness<'_, LaApp>, name: &str) {
    let path = out_dir().join(format!("{name}.png"));
    h.render().expect("render").save(&path).expect("save png");
    eprintln!("wrote {}", path.display());
}

fn until(h: &mut Harness<'_, LaApp>, what: &str, cond: impl Fn(&LaApp) -> bool) {
    let t0 = Instant::now();
    while !cond(h.state()) {
        assert!(t0.elapsed() < Duration::from_secs(60), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run_steps(3);
}

#[test]
#[ignore = "generates README screenshots"]
fn readme_screenshots() {
    let clock = ManualClock::new();
    let c = clock.clone();
    let mut h = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_pixels_per_point(2.0)
        .with_render_cursor(false)
        .build_eframe(move |cc| {
            let options = MockOptions { streaming: Streaming::RealTime(Arc::new(c.clone())), ..MockOptions::default() };
            LaApp::with_options(
                cc,
                AppOptions {
                    backend: Arc::new(MockBackend::new(options)),
                    clock: Arc::new(c),
                    dialogs: Arc::new(FixedDialogs(None)),
                    settings_path: None,
                    animation_speed: 0.0,
                    scan_sync: true,
                    profiles_dir: None,
                    extra_builtin_profiles: Vec::new(),
                },
            )
        });
    h.run();
    save(&mut h, "start-screen");

    // Demo device: clock, UART and SPI CS/SCK/MOSI (D0–D4) and the A0 sine.
    h.get_by_label("Mock 16 digital + 4 analog").click();
    until(&mut h, "device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    h.key_press_modifiers(egui::Modifiers::ALT, Key::Num1);
    h.run();
    for chip in ["D2", "D3", "D4", "A1"] {
        h.get_by_role_and_label(egui::Role::CheckBox, chip).click();
        h.run();
    }
    h.key_press(Key::Space);
    until(&mut h, "recording", |a| a.session().unwrap().state == CaptureState::Recording);
    clock.advance(Duration::from_secs(2));
    until(&mut h, "data", |a| {
        a.session().unwrap().capture.as_ref().and_then(|c| c.read().sample_range()).is_some_and(|r| r.1 >= 16_000_000)
    });
    h.key_press(Key::Space);
    until(&mut h, "finished", |a| a.session().unwrap().state == CaptureState::Finished);

    // About 2 ms at the start: a UART frame, SPI transfers, clock edges.
    h.key_press(Key::F);
    h.run();
    for _ in 0..10 {
        h.key_press(Key::Equals);
        h.run();
    }
    h.key_press(Key::Home);
    h.run();
    h.remove_cursor();
    h.run();
    save(&mut h, "capture");
    h.state_mut().set_theme(oxrok_ui::ui::theme::ThemeKind::Light);
    h.run();
    save(&mut h, "capture-light");
    h.state_mut().set_theme(oxrok_ui::ui::theme::ThemeKind::Dark);
    h.run();

    // 0.3.0: named rows (inline rename), the row menu and the zoom tool.
    for (row, name) in [("D0", "CLK"), ("D1", "UART TX"), ("D2", "SPI CS"), ("D3", "SPI SCK"), ("D4", "SPI MOSI")] {
        h.get_by_label(&format!("Rename {row}")).click();
        h.run_steps(3);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text(name.into()));
        h.key_press(Key::Enter);
        h.run_steps(3);
    }
    let label = h.get_by_role_and_label(egui::Role::Label, "D2 SPI CS").rect().center();
    h.hover_at(label);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: label,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.run_steps(4);
    h.get_by_label("Show/Hide Channels").click();
    h.run_steps(6);
    save(&mut h, "row-menu");
    h.key_press(Key::Escape);
    h.run_steps(4);
    h.get_by_label("Zoom Tool Options").click();
    h.run_steps(4);
    h.remove_cursor();
    h.run_steps(2);
    save(&mut h, "zoom-tool");
    h.key_press(Key::Escape);
    h.run_steps(4);

    // 0.4.0: a marker pair on the SPI transfer, a range on the clock, and
    // the Timing Markers panel.
    let click = |h: &mut Harness<'_, LaApp>, at: egui::Pos2| {
        h.hover_at(at);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(3);
    };
    let below = h.get_by_role_and_label(egui::Role::Label, "A0").rect().bottom() + 60.0;
    h.key_press_modifiers(egui::Modifiers::SHIFT, Key::M);
    h.run_steps(2);
    click(&mut h, egui::pos2(160.0, below));
    click(&mut h, egui::pos2(560.0, below));
    h.get_by_role_and_label(egui::Role::Button, "Markers").click();
    h.run_steps(4);
    h.key_press(Key::R);
    h.run_steps(2);
    let clk = h.get_by_role_and_label(egui::Role::Label, "D0 CLK").rect().center().y;
    h.hover_at(egui::pos2(300.0, clk));
    h.event(egui::Event::PointerButton {
        pos: egui::pos2(300.0, clk),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
    for k in 1..=6 {
        h.hover_at(egui::pos2(300.0 + 70.0 * k as f32, clk));
        h.run_steps(2);
    }
    h.drop_at(egui::pos2(720.0, clk));
    h.run_steps(4);
    h.remove_cursor();
    h.run_steps(4);
    save(&mut h, "markers");

    // 0.5.0: Async Serial on D1 and SPI on D2–D4, with the data table.
    h.key_press(Key::Escape);
    h.run_steps(4);
    {
        let s = h.state_mut().session_mut().unwrap();
        s.markers.items.clear();
        s.ranges.items.clear();
    }
    h.run_steps(2);
    h.get_by_role_and_label(egui::Role::Button, "Analyzers").click();
    h.run_steps(4);
    let pick = |h: &mut Harness<'_, LaApp>, combo: &str, item: &str| {
        h.get_by_label(combo).click();
        h.run_steps(3);
        // The list may open scrolled to the current choice.
        h.get_by_label(item).scroll_to_me();
        h.run_steps(3);
        h.get_by_label(item).click();
        h.run_steps(3);
    };
    h.get_by_role_and_label(egui::Role::MenuItem, "Async Serial").click();
    h.run_steps(10);
    pick(&mut h, "RX channel", "01.  D1 UART TX");
    h.get_by_label("Save").click();
    h.run_steps(5);
    // The Analyzers panel's ＋ (the rightmost of the Add Analyzer buttons).
    h.query_all_by_label("Add Analyzer").max_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap().click();
    h.run_steps(4);
    h.get_by_label("SPI").click();
    h.run_steps(10);
    pick(&mut h, "CLK channel", "03.  D3 SPI SCK");
    pick(&mut h, "MOSI channel", "04.  D4 SPI MOSI");
    pick(&mut h, "CS# channel", "02.  D2 SPI CS");
    h.get_by_label("Save").click();
    h.run_steps(5);
    until(&mut h, "decoded", |a| {
        let s = a.session().unwrap();
        s.analyzers.len() == 2
            && s.analyzers.iter().all(|x| *x.out.status.lock() == oxrok_ui::decode::runner::Status::Complete)
    });
    h.remove_cursor();
    h.run_steps(6);
    save(&mut h, "analyzers");
}
