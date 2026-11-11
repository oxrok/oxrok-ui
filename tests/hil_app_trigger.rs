//! Trigger mode in the app on the LogicPort: records with
//! several pre-trigger times. Needs the generator's demo loop running (D0
//! bursts); it doesn't drive the generator itself.
//!
//! Its own test binary: one libsigrok context per process.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::core::MonotonicClock;
use oxrok_ui::core::capture::{CaptureMode, CaptureState};
use oxrok_ui::hil::bench::DRIVER;
use oxrok_ui::oxrok::OxrokBackend;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Picks a capture mode from the toolbar dropdown.
fn pick_mode(h: &mut Harness<'_, LaApp>, mode: &str) {
    h.get_by_role_and_label(egui::Role::Button, "Capture Mode").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::Role::MenuItemCheckBox, mode).click();
    h.run_steps(3);
}

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
fn trigger_mode_pre_trigger_in_the_app() {
    if !oxrok_ui::hil::enabled() {
        eprintln!("skipped: set LA_HIL=1 (make test-hil) to run the bench tests");
        return;
    }
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
    h.get_by_role_and_label(egui::Role::Button, "Intronix LogicPort").click();
    until(&mut h, "device opened", |a| a.session().is_some_and(|s| s.caps.is_some()));
    pick_mode(&mut h, "Trigger");
    h.run_steps(3);
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/hil");
    std::fs::create_dir_all(&out).unwrap();
    for rle in [true, false] {
        for pre in [0.0, 0.1, 1.0] {
            {
                let s = h.state_mut().session_mut().unwrap();
                assert_eq!(s.settings.mode, CaptureMode::Triggered);
                s.settings.rle = rle;
                s.settings.trigger.pre_trigger_seconds = pre;
                eprintln!(
                    "--- rle={rle} pre={pre}: rate {} trigger {:?} caps.hw {:?}",
                    s.settings.sample_rate,
                    s.settings.trigger,
                    s.caps.as_ref().unwrap().hw_trigger
                );
            }
            h.key_press(Key::Space);
            until(&mut h, "started", |a| {
                a.session().unwrap().state != CaptureState::Empty || a.toasts().iter().next().is_some()
            });
            let t0 = Instant::now();
            until(&mut h, "finished", |a| {
                matches!(a.session().unwrap().state, CaptureState::Finished | CaptureState::Empty)
            });
            eprintln!("took {:.1?}", t0.elapsed());
            let s = h.state().session().unwrap();
            let range = s.capture.as_ref().and_then(|c| c.read().sample_range());
            let d0 = s
                .capture
                .as_ref()
                .and_then(|c| c.read().channel(oxrok_ui::core::ChannelId::digital(0)).map(|c| c.transition_count()));
            eprintln!(
                "state {:?} range {range:?} trigger {:?} D0 edges {d0:?} origin {:?} snapshot {:?}",
                s.state,
                s.trigger,
                s.origin,
                s.snapshot.as_ref().map(|x| (x.mode, x.trigger.clone())),
            );
            // T is the device's trigger (refined to the edge).
            let cap = s.capture.as_ref().unwrap().read();
            let (hw, t) = (cap.hw_trigger.expect("device trigger reported"), cap.trigger.expect("T"));
            eprintln!("device trigger at {hw}, T at {t}");
            assert!(t.abs_diff(hw) <= 64, "T {t} vs device {hw}");
            drop(cap);
            for t in h.state().toasts().iter() {
                eprintln!("toast: {:?} {}", t.heading, t.body);
            }
            let path = out.join(format!("trigger_rle{rle}_pre{pre}.png"));
            h.render().unwrap().save(&path).unwrap();
        }
    }
}
