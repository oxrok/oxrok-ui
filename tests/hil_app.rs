//! The app end to end on the bench: open the LogicPort from the start
//! screen, record in Timer mode while the ESP32 drives known signals, and
//! check what the UI shows. Saves `target/hil/app_logicport.png`.
//!
//! Its own test binary: one libsigrok context per process.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::core::MonotonicClock;
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::hil::bench::{DRIVER, MEM_DEPTH};
use oxrok_ui::hil::ch::*;
use oxrok_ui::hil::siggen::{DEFAULT_MAP, SigGen};
use oxrok_ui::oxrok::OxrokBackend;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn until(h: &mut Harness<'_, LaApp>, what: &str, cond: impl Fn(&LaApp) -> bool) {
    let t0 = Instant::now();
    while !cond(h.state()) {
        assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    h.run_steps(3);
}

#[test]
fn record_logicport_in_the_app() {
    if !oxrok_ui::hil::enabled() {
        eprintln!("skipped: set LA_HIL=1 (make test-hil) to run the bench tests");
        return;
    }
    let mut siggen = SigGen::open().expect("ESP32 siggen");
    siggen.map(&DEFAULT_MAP).unwrap();
    siggen.set(0b101).unwrap(); // D0 high, D1 low, D2 high
    let clk = siggen.clk(100_000, 50, 14).unwrap(); // CLK1

    let backend = OxrokBackend::with_drivers(Some(vec![DRIVER.into()]));
    let mut h = Harness::builder().with_size([1280.0, 1000.0]).build_eframe(move |cc| {
        LaApp::with_options(
            cc,
            AppOptions {
                backend: Arc::new(backend),
                clock: Arc::new(MonotonicClock::new()),
                dialogs: Arc::new(FixedDialogs(None)),
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
    // A physical device: the Connect card offers it directly.
    h.get_by_role_and_label(egui::Role::Button, "Intronix LogicPort").click();
    until(&mut h, "device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    for chip in ["D2", "CLK1"] {
        h.get_by_role_and_label(egui::Role::CheckBox, chip).click();
        h.run();
    }
    let rate = h.state().session().unwrap().settings.sample_rate;
    eprintln!("sample rate {rate} S/s");
    // The LogicPort is buffered: Timed is its default, with
    // compression on.
    assert_eq!(h.state().session().unwrap().settings.mode, oxrok_ui::core::capture::CaptureMode::Timed);
    assert!(h.state().session().unwrap().settings.rle);
    h.key_press(Key::Space);
    until(&mut h, "finished", |a| a.session().unwrap().state == CaptureState::Finished);

    // The user learns why.
    let t = h
        .state()
        .toasts()
        .iter()
        .find(|t| t.heading.as_deref() == Some("Capture ended early"))
        .expect("capture ended early");
    assert!(t.body.contains("Lower the sample rate"), "{}", t.body);
    {
        let s = h.state().session().unwrap();
        let cap = s.capture.as_ref().unwrap().read();
        let (b, e) = cap.sample_range().unwrap();
        // The 100 kHz clock fills even the compressed memory long before
        // the 1 s timer: more than 2048 samples, far less than requested.
        let got = e - b;
        eprintln!("captured {got} samples with RLE");
        assert!(got > MEM_DEPTH && got < rate / 10, "{got} samples");
        for (id, level) in [(D0, true), (D1, false), (D2, true)] {
            let ch = cap.channel(id).unwrap();
            assert_eq!((ch.level_at(b), ch.transition_count()), (Some(level), 0), "{id:?}");
        }
        let clk1 = cap.channel(CLK1).unwrap();
        let expected = 2.0 * clk * got as f64 / rate as f64;
        assert!((clk1.transition_count() as f64 - expected).abs() <= 2.0, "{} CLK1 edges", clk1.transition_count());
    }

    // The hover readout on CLK1 reports the generator's frequency.
    h.key_press(Key::F);
    h.run();
    let label = h.get_by_role_and_label(egui::Role::Label, "CLK1").rect();
    h.hover_at(egui::pos2(label.right() + 400.0, label.center().y));
    h.run();
    // One period at 10 MS/s: an edge can land a sample early or late, so
    // allow ±2 % (e.g. "Freq: 99.01 kHz").
    // egui exposes a text label's content as the node's label or value.
    let text = |n: egui_kittest::Node<'_>| {
        let a = n.accesskit_node();
        a.label().or(a.value()).unwrap_or_default().to_owned()
    };
    let freq = text(h.get(egui_kittest::kittest::By::new().label_contains("Freq:")));
    let khz: f64 = freq.trim_start_matches("Freq: ").trim_end_matches(" kHz").parse().unwrap_or(0.0);
    assert!((khz - 100.0).abs() <= 2.0, "{freq}");
    let duty = text(h.get(egui_kittest::kittest::By::new().label_contains("Duty:")));
    let pct: f64 = duty.trim_start_matches("Duty: ").trim_end_matches(" %").parse().unwrap_or(0.0);
    assert!((pct - 50.0).abs() <= 2.0, "{duty}");

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/hil");
    std::fs::create_dir_all(&out).unwrap();
    // The built-in probe profile applies: sections by marker color.
    let (p, _) = h.state().session().unwrap().probe.clone().expect("built-in LogicPort profile");
    assert_eq!(p.name, "Intronix LogicPort");
    for heading in ["D0–D7", "D8–D15", "D16–D23", "D24–D31", "Clocks"] {
        h.get_by_label(heading);
    }
    let path = out.join("app_logicport.png");
    h.render().unwrap().save(&path).unwrap();
    eprintln!("wrote {}", path.display());
    siggen.clk(0, 50, 14).unwrap();
    siggen.set(0).unwrap();
}
