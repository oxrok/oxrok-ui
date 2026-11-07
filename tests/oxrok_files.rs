//! Sigrok `.sr` files through the app, with oxrok.
//!
//! `make test-oxrok` (or `cargo test -p la-app --features oxrok --test oxrok_files`)

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::ScriptedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::core::{ChannelId, MonotonicClock};
use oxrok_ui::oxrok::OxrokBackend;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn until(h: &mut Harness<'_, LaApp>, what: &str, cond: impl Fn(&LaApp) -> bool) {
    let t0 = Instant::now();
    while !cond(h.state()) {
        if t0.elapsed() > Duration::from_secs(30) {
            let toasts: Vec<String> =
                h.state().toasts().iter().map(|t| format!("{:?}: {}", t.heading, t.body)).collect();
            panic!("timed out waiting for {what}; toasts: {toasts:?}");
        }
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    h.step();
}

fn open(h: &mut Harness<'_, LaApp>, dialogs: &ScriptedDialogs, path: &Path) {
    *dialogs.open.lock() = vec![path.to_owned()];
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::O);
    // The import's progress keeps repainting: step, don't wait to settle.
    h.run_steps(3);
}

fn opened(app: &LaApp, path: &Path) -> bool {
    app.session().is_some_and(|s| s.opened_from.as_deref() == Some(path))
}

#[test]
fn imports_and_sr_round_trip() {
    let backend = OxrokBackend::with_drivers(Some(vec!["demo".into()]));
    let dir = tempfile::tempdir().unwrap();
    let dialogs = Arc::new(ScriptedDialogs { folder: Some(dir.path().to_owned()), ..Default::default() });
    let d = dialogs.clone();
    let mut h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        LaApp::with_options(
            cc,
            AppOptions {
                backend: Arc::new(backend),
                clock: Arc::new(MonotonicClock::new()),
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
    h.run();

    // A VCD opens as a read-only session named after its signals.
    let vcd = dir.path().join("bus.vcd");
    std::fs::write(
        &vcd,
        "$timescale 1 us $end\n$scope module top $end\n$var wire 1 ! clk $end\n$var wire 1 \" data $end\n\
         $upscope $end\n$enddefinitions $end\n#0\n0!\n1\"\n#2\n1!\n#4\n0!\n0\"\n#6\n1!\n#100\n",
    )
    .unwrap();
    open(&mut h, &dialogs, &vcd);
    until(&mut h, "vcd opened", |a| opened(a, &vcd));
    let s = h.state().session().unwrap();
    assert!(s.is_read_only() && !s.is_unsaved());
    let names: Vec<&str> = s.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(&names[..2], ["clk", "data"], "the single VCD scope is dropped");
    h.run();
    h.get_by_label("Opened from bus.vcd");

    // A headerless .bin asks first: the Import Options dialog.
    let bin = dir.path().join("capture.bin");
    std::fs::write(&bin, (0..=255u8).collect::<Vec<_>>()).unwrap();
    open(&mut h, &dialogs, &bin);
    until(&mut h, "options dialog", |_| true);
    for _ in 0..50 {
        if h.query_by_label("Import capture.bin").is_some() {
            break;
        }
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    h.get_by_label("Import capture.bin");
    // Raw binary logic data, 8 channels at 1 MHz.
    if h.query_by_role_and_label(egui::Role::ComboBox, "Format").is_some() {
        h.get_by_role_and_label(egui::Role::ComboBox, "Format").click();
        h.run();
        h.get_by_role_and_label(egui::Role::ListBoxOption, "Raw binary logic data").click();
        h.run();
    }
    let rate = h.get_by_role_and_label(egui::Role::TextInput, "Sample rate (Hz)");
    rate.focus();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.get_by_role_and_label(egui::Role::TextInput, "Sample rate (Hz)").type_text("1M");
    h.run();
    h.get_by_label("Import").click();
    until(&mut h, "bin opened", |a| opened(a, &bin));
    let s = h.state().session().unwrap();
    let rate = s.capture.as_ref().unwrap().read().sample_rate();
    assert_eq!(rate, Some(1_000_000));
    assert_eq!(s.rows.iter().filter(|r| r.channel.kind == oxrok_ui::core::ChannelKind::Digital).count(), 8);
    let level = |s: &oxrok_ui::app::session::Session, k: u16, x: u64| {
        s.capture.as_ref().unwrap().read().channel(ChannelId::digital(k)).and_then(|c| c.level_at(x))
    };
    assert_eq!((level(s, 0, 0), level(s, 0, 1)), (Some(false), Some(true)), "bit 0 of the counter");

    // Export it as sigrok .sr and open that.
    {
        let st = h.state_mut().settings_mut();
        st.last_dirs.export = Some(dir.path().to_owned());
        st.export.format = oxrok_ui::app::settings::ExportFormat::Sigrok;
        st.export.range = oxrok_ui::app::settings::ExportRange::All;
    }
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::E);
    h.run();
    h.key_press(Key::Enter);
    until(&mut h, "sr export", |a| {
        a.toasts().iter().any(|t| t.heading.as_deref() == Some("Export complete") && t.body.contains(".sr"))
    });
    let sr = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|e| e == "sr"))
        .expect("an .sr file");
    open(&mut h, &dialogs, &sr);
    until(&mut h, "sr opened", |a| opened(a, &sr));
    let s = h.state().session().unwrap();
    assert_eq!(s.capture.as_ref().unwrap().read().sample_rate(), Some(1_000_000));
    assert_eq!((level(s, 7, 127), level(s, 7, 128)), (Some(false), Some(true)), "bit 7 survives the round trip");
}
