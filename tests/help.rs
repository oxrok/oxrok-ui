#![cfg(feature = "mock")]
//! Device Connection Help, hot-plug polling (05
//! ) and About.

use egui::{Event, Key, Modifiers, OutputCommand};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use oxrok_ui::app::dialogs::FixedDialogs;
use oxrok_ui::app::{AppOptions, LaApp};
use oxrok_ui::backend::{MockBackend, MockOptions};
use oxrok_ui::core::ManualClock;
use std::sync::Arc;
use std::time::Duration;

struct Rig {
    h: Harness<'static, LaApp>,
    mock: Arc<MockBackend>,
    clock: ManualClock,
}

fn rig() -> Rig {
    rig_with(MockOptions::default())
}

fn rig_with(options: MockOptions) -> Rig {
    let clock = ManualClock::new();
    let mock = Arc::new(MockBackend::new(options));
    let (c, m) = (clock.clone(), mock.clone());
    let h = Harness::builder().with_size([1280.0, 800.0]).build_eframe(move |cc| {
        LaApp::with_options(
            cc,
            AppOptions {
                backend: m,
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
    let mut r = Rig { h, mock, clock };
    r.h.run();
    r
}

impl Rig {
    fn menu(&mut self, menu: &str, item: &str) {
        self.h.get_by_label(menu).click();
        self.h.run();
        self.h.get_by_label(item).click();
        self.h.run();
    }

    /// Text copied this frame (`CopyText`), after `click`.
    fn copied_after(&mut self, label: &str) -> String {
        self.h.get_by_label(label).click();
        self.h.step();
        self.h
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|c| match c {
                OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }
}

/// "No device found"; Rescan after the mock adds two devices
/// reports "2 devices found"; Copy diagnostics lists every card.
#[test]
fn connection_help() {
    let mut r = rig();
    r.menu("Help", "Device Connection Help…");
    r.h.get_by_label("Device Connection Help");
    r.h.get_by_label("No device found");
    r.h.get_by_label("Cable and port");
    r.mock.plug("usb-1", "Mock analyzer A");
    r.mock.plug("usb-2", "Mock analyzer B");
    // The open dialog doesn't scan on its own, only on Rescan.
    r.clock.advance(Duration::from_secs(5));
    r.h.run();
    r.h.get_by_label("No device found");
    r.h.get_by_label("Rescan").click();
    r.h.run();
    r.h.get_by_label("2 devices found");
    r.h.snapshot("connection_help");
    let text = r.copied_after("Copy diagnostics");
    assert!(text.contains("[○] Cable and port"), "{text}");
    assert!(text.contains("mock backend"), "{text}");
    assert!(text.contains("Mock analyzer B"), "{text}");
    r.h.key_press(Key::Escape);
    r.h.run();
    assert!(r.h.query_by_label("Device Connection Help").is_none());
}

/// On the start screen the list refreshes every 2 s.
#[test]
fn hot_plug_on_the_start_screen() {
    let mut r = rig();
    r.h.get_by_label("Disconnected");
    r.mock.plug("usb-1", "Mock analyzer A");
    r.clock.advance(Duration::from_millis(1500));
    r.h.run();
    r.h.get_by_label("Disconnected");
    r.clock.advance(Duration::from_millis(600));
    r.h.run();
    r.h.get_by_label("1 Device");
    r.mock.unplug("usb-1");
    r.clock.advance(Duration::from_secs(2));
    r.h.run();
    r.h.get_by_label("Disconnected");
}

/// Driver:conn finds a network instrument, and is remembered.
#[test]
fn connect_by_address() {
    let mut r = rig();
    r.h.get_by_label("Disconnected").click();
    r.h.run();
    r.h.get_by_label("Device Connection Help ›").click();
    r.h.run();
    r.h.get_by_label("Connection").click();
    r.h.event(Event::Text("demo:conn=tcp-raw/10.0.0.5/5555".into()));
    r.h.run();
    r.h.get_by_label("Connect").click();
    let t0 = std::time::Instant::now();
    while r.h.state().toasts().iter().all(|t| t.body != "Found 1 device") {
        assert!(t0.elapsed() < Duration::from_secs(10), "no result");
        r.h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    r.h.run();
    r.h.get_by_label("1 device found");
    assert_eq!(r.h.state().settings().recent_connections, ["demo:conn=tcp-raw/10.0.0.5/5555"]);
}

/// Version, libraries, licenses; Copy version info.
#[test]
fn about_dialog() {
    let mut r = rig();
    r.menu("Help", "About");
    r.h.get_by_label("About Oxrok UI");
    r.h.get_by_label(&format!("Oxrok UI {}", env!("CARGO_PKG_VERSION")));
    r.h.get_by_label_contains("GNU GENERAL PUBLIC LICENSE");
    r.h.run_steps(10);
    r.h.get_by_label("Third-party notices").click();
    r.h.run();
    r.h.get_by_label_contains("Third-party software in this build");
    let text = r.copied_after("Copy version info");
    assert!(text.starts_with(&format!("Oxrok UI {}", env!("CARGO_PKG_VERSION"))), "{text}");
    assert!(text.contains("mock backend"), "{text}");
    r.h.key_press_modifiers(Modifiers::NONE, Key::Escape);
    r.h.run();
    assert!(r.h.query_by_label("About Oxrok UI").is_none());
}

/// A file dropped on the window.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// A missing firmware file: the Devices menu warns; Install… →
/// choose the file → Extract streams two lines and installs one file;
/// Rescan clears the warning.
#[test]
fn install_firmware() {
    let dir = tempfile::tempdir().unwrap();
    let fw = dir.path().join("sigrok-firmware");
    let file = "kingst-la2016a1-fpga.bitstream";
    let mut r = rig_with(MockOptions {
        missing_firmware: Some(("kingst-la2016".into(), file.into())),
        firmware_dir: Some(fw.clone()),
        ..MockOptions::default()
    });
    r.h.get_by_label("Disconnected").click();
    r.h.run();
    r.h.get_by_label("A device needs firmware");
    r.h.get_by_label("Install…").click();
    r.h.run();
    r.h.get_by_label("Install firmware for Kingst LA");
    r.h.get_by_label("The KingstVIS executable");
    // Drop the "vendor download" on the dialog.
    let fixture = dir.path().join("KingstVIS");
    std::fs::write(&fixture, b"fpga").unwrap();
    r.h.input_mut().dropped_files.push(Arc::new(Dropped(fixture)));
    r.h.run();
    r.h.get_by_label("Extract firmware").click();
    let t0 = std::time::Instant::now();
    while r.h.query_by_label_contains("Installed 1 file").is_none() {
        assert!(t0.elapsed() < Duration::from_secs(10), "no result");
        r.h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    r.h.get_by_label_contains(&format!("saving {file} (4 bytes)"));
    assert!(fw.join(file).is_file());
    r.h.run_steps(10);
    r.h.get_by_label("Rescan").click();
    r.h.run();
    assert!(r.h.state().firmware_missing().is_empty());
    r.h.key_press(Key::Escape);
    r.h.run();
    r.h.get_by_label("Disconnected").click();
    r.h.run();
    assert!(r.h.query_by_label("A device needs firmware").is_none());
}
