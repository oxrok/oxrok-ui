#![cfg(feature = "mock")]
//! Performance budgets, measured through the app. Timing tests:
//! `make perf` runs them in release (`--ignored perf`).

use egui::{Event, Key, Modifiers, MouseWheelUnit, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions};
use oxrok_ui::core::capture::CaptureState;
use oxrok_ui::core::{ChannelId, MonotonicClock};
use oxrok_ui::store::DigitalChannel;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn harness(dir: Option<PathBuf>) -> Harness<'static, LaApp> {
    let mut h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        LaApp::with_options(
            cc,
            AppOptions {
                backend: Arc::new(MockBackend::new(MockOptions::default())),
                clock: Arc::new(MonotonicClock::new()),
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
    h
}

fn until(h: &mut Harness<'static, LaApp>, what: &str, cond: impl Fn(&LaApp) -> bool) {
    let t0 = Instant::now();
    while !cond(h.state()) {
        assert!(t0.elapsed() < Duration::from_secs(60), "timed out: {what}");
        h.step();
    }
}

/// Opens the 16 + 4 mock and records a moment, so the session has rows.
fn session(h: &mut Harness<'static, LaApp>) {
    h.get_by_label("Mock 16 digital + 4 analog").click();
    until(h, "device", |a| a.session().is_some_and(|s| s.caps.is_some()));
    h.key_press(Key::Space);
    until(h, "data", |a| a.session().unwrap().has_data());
    h.key_press(Key::Space);
    until(h, "finished", |a| a.session().unwrap().state == CaptureState::Finished);
}

/// 16 channels × `samples`, each with `transitions` edges at
/// pseudo-random gaps (so files don't compress unrealistically).
fn synthetic(samples: u64, transitions: u64) -> Vec<DigitalChannel> {
    (0..16u64)
        .map(|k| {
            let mut ch = DigitalChannel::new();
            ch.start(0, false);
            let gap = samples / transitions.max(1);
            let mut s = 0;
            let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ (k + 1);
            for _ in 0..transitions {
                // xorshift64
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s += 1 + x % (2 * gap).max(1);
                if s >= samples {
                    break;
                }
                ch.push_transition(s);
            }
            ch.extend_to(samples);
            ch
        })
        .collect()
}

/// Replaces the session's samples with `channels` (D0–D15).
fn install(h: &mut Harness<'static, LaApp>, channels: Vec<DigitalChannel>) {
    let s = h.state_mut().session_mut().unwrap();
    let cap = s.capture.as_ref().unwrap().clone();
    let mut d = cap.write();
    let bits: Vec<u16> = (0..16).collect();
    d.ids = (0..16).map(ChannelId::digital).collect();
    d.ingest = Some(oxrok_ui::store::LogicIngest::from_channels(2, bits, channels));
    d.analog.clear();
}

fn report(name: &str, times: &mut [f64], budget_ms: f64) {
    times.sort_by(f64::total_cmp);
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let p95 = times[times.len() * 95 / 100];
    eprintln!("{name}: avg {avg:.2} ms, p95 {p95:.2} ms (budget {budget_ms} ms)");
    assert!(p95 <= budget_ms, "{name}: p95 {p95:.2} ms over {budget_ms} ms");
}

/// Frame time while panning, 16 ch × 1 G samples: ≤ 8 ms UI CPU.
#[test]
#[ignore = "timing; make perf"]
fn perf_pan_frame_time() {
    let mut h = harness(None);
    session(&mut h);
    install(&mut h, synthetic(1 << 30, 2_000_000));
    h.key_press(Key::F);
    h.run_steps(5);
    let plot = Pos2::new(600.0, 300.0);
    h.hover_at(plot);
    for (name, zoom_in) in [("nearly all", 2), ("mid zoom", 12), ("zoomed in", 24)] {
        h.key_press(Key::Home);
        for _ in 0..zoom_in {
            h.key_press(Key::Equals);
        }
        h.run_steps(5);
        let before = h.state().session().unwrap().view.state().left_time;
        let mut times = Vec::new();
        for _ in 0..120 {
            h.event(Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(-40.0, 0.0),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Trackpad,
            });
            let t0 = Instant::now();
            h.step();
            times.push(t0.elapsed().as_secs_f64() * 1e3);
        }
        assert_ne!(h.state().session().unwrap().view.state().left_time, before, "{name}: the view panned");
        report(&format!("pan, {name}"), &mut times, 8.0);
    }
}

/// Start → first data on screen: ≤ 300 ms after the device streams.
#[test]
#[ignore = "timing; make perf"]
fn perf_first_data() {
    let mut h = harness(None);
    h.get_by_label("Mock 16 digital + 4 analog").click();
    until(&mut h, "device", |a| a.session().is_some_and(|s| s.caps.is_some()));
    let mut times = Vec::new();
    for _ in 0..10 {
        let t0 = Instant::now();
        h.key_press(Key::Space);
        until(&mut h, "data", |a| a.session().unwrap().has_data());
        times.push(t0.elapsed().as_secs_f64() * 1e3);
        h.key_press(Key::Space);
        until(&mut h, "finished", |a| a.session().unwrap().state == CaptureState::Finished);
    }
    times.push(times[times.len() - 1]);
    report("start → first data", &mut times, 300.0);
}

/// Open a 1 GB capture (16 ch × 512 M samples) to an interactive view: ≤ 2 s.
#[test]
#[ignore = "timing; make perf"]
fn perf_open_capture() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(Some(dir.path().to_path_buf()));
    session(&mut h);
    install(&mut h, synthetic(512 << 20, 1_000_000));
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    let path = dir.path().join("Session 0.oxrok");
    let t0 = Instant::now();
    while !path.exists() || h.state().toasts().iter().all(|t| !t.body.starts_with("Saved")) {
        assert!(t0.elapsed() < Duration::from_secs(120), "save");
        h.step();
    }
    let mb = std::fs::metadata(&path).unwrap().len() as f64 / 1e6;
    let mut h2 = harness(Some(dir.path().to_path_buf()));
    let t0 = Instant::now();
    h2.input_mut().dropped_files.push(Arc::new(Dropped(path)));
    until(&mut h2, "opened", |a| {
        a.session().is_some_and(|s| s.state == CaptureState::Finished && s.has_data()) && !a.is_loading()
    });
    h2.step();
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    eprintln!("open 1 GB capture ({mb:.1} MB file): {ms:.0} ms (budget 2000 ms)");
    assert!(ms <= 2000.0, "open took {ms:.0} ms");
}

/// Data table scroll to an arbitrary row: ≤ 50 ms (10⁷ rows here; the
/// table lays out only the visible rows, so the time doesn't grow).
#[test]
#[ignore = "timing; make perf"]
fn perf_table_scroll() {
    let mut h = harness(None);
    session(&mut h);
    h.get_by_label("Analyzers").click();
    h.run();
    h.get_by_label("Async Serial").click();
    h.run_steps(5);
    h.get_by_label("RX channel").click();
    h.run();
    h.get_by_label("01.  D1").click();
    h.run();
    h.get_by_label("Save").click();
    h.run_steps(5);
    until(&mut h, "decoded", |a| {
        *a.session().unwrap().analyzers[0].out.status.lock() == oxrok_ui::decode::runner::Status::Complete
    });
    {
        let s = h.state_mut().session_mut().unwrap();
        let mut f = s.analyzers[0].out.frames.write();
        let text: Arc<[String]> = Arc::from(vec!["A".to_owned()]);
        for i in 0..10_000_000u64 {
            f.push(oxrok_ui::decode::Frame {
                start: i * 10,
                end: i * 10 + 8,
                class: 0,
                row: 0,
                texts: text.clone(),
                error: false,
            });
        }
    }
    h.run_steps(3);
    let table = h.get_by_label("Search table").rect().center() + vec2(0.0, 200.0);
    h.hover_at(table);
    let mut times = Vec::new();
    for k in 0..60 {
        h.event(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, if k % 2 == 0 { -5_000_000.0 } else { 2_000_000.0 }),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Trackpad,
        });
        let t0 = Instant::now();
        h.step();
        times.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    report("table scroll (10⁷ rows)", &mut times, 50.0);
}

/// A file dropped on the window.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}
