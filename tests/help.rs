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
                mock_decoders: cfg!(feature = "mock"),
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

/// The app with `file` missing for `driver`, installs going to a
/// temporary folder, and Install Firmware open from the Devices menu.
fn firmware_rig(driver: &str, file: &str, stalls: bool) -> (Rig, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let fw = dir.path().join("oxrok-firmware");
    let mut r = rig_with(MockOptions {
        missing_firmware: Some((driver.into(), file.into())),
        firmware_dir: Some(fw.clone()),
        firmware_download_stalls: stalls,
        ..MockOptions::default()
    });
    r.h.get_by_label("Disconnected").click();
    r.h.run();
    r.h.get_by_label("A device needs firmware");
    r.h.get_by_label("Install…").click();
    r.h.run();
    r.h.get_by_label("Install firmware");
    r.h.get_by_label(&format!("{file} is missing."));
    (r, dir, fw)
}

/// Steps until a label containing `text` shows (a worker thread is busy),
/// and the layout settles.
fn wait_for(r: &mut Rig, text: &str) {
    let t0 = std::time::Instant::now();
    while r.h.query_by_label_contains(text).is_none() {
        assert!(t0.elapsed() < Duration::from_secs(10), "no {text:?}");
        r.h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    // The dialog grew, and is centred again a frame later.
    r.h.run_steps(4);
}

/// Rescan in the dialog finds nothing missing, and the Devices menu
/// stops warning.
fn rescan_clears_the_warning(r: &mut Rig) {
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

/// A missing file the firmware registry can download: the dialog names
/// the package, the size and the hosts; Download and install puts every
/// file of the package in the install folder; Rescan clears the warning.
#[test]
fn install_firmware_downloads_the_package() {
    let file = "kingst-la2016a1-fpga.bitstream";
    let (mut r, _dir, fw) = firmware_rig("kingst-la2016", file, false);
    let pkg = oxrok_ui::backend::fetch::package_for(file).unwrap();
    r.h.get_by_label(pkg.title);
    r.h.get_by_label("27.8 MB from res.kingst.site, www.qdkingst.com");
    // The vendor's program, for those who have it already.
    r.h.get_by_label("Or use files you have");
    r.h.get_by_label_contains("the KingstVIS program");
    r.h.get_by_label("Download and install").click();
    wait_for(&mut r, &format!("Installed {} files", pkg.files.len()));
    r.h.get_by_label_contains(&format!("Downloading {}", pkg.downloads[0].name));
    assert!(pkg.files.iter().all(|f| fw.join(f).is_file()));
    rescan_clears_the_warning(&mut r);
}

/// Cancel stops a download, and nothing is installed.
#[test]
fn install_firmware_download_cancels() {
    let file = "kingst-la2016a1-fpga.bitstream";
    let (mut r, _dir, fw) = firmware_rig("kingst-la2016", file, true);
    r.h.get_by_label("Download and install").click();
    wait_for(&mut r, "27.8 MB / 27.8 MB");
    r.h.get_by_label("Cancel").click();
    wait_for(&mut r, "Cancelled. Nothing was installed.");
    assert!(!fw.exists());
    // The download can start again.
    r.h.get_by_label("Download and install");
    r.h.key_press(Key::Escape);
    r.h.run();
    assert_eq!(r.h.state().firmware_missing(), [file]);
}

/// A package with no download (fx2lafw from git): the dialog says what
/// to get; a dropped folder with the files is imported by the registry's
/// own importer.
#[test]
fn install_firmware_from_the_users_files() {
    let file = "fx2lafw-usb-c-grok.fw";
    let (mut r, dir, fw) = firmware_rig("fx2lafw", file, false);
    let pkg = oxrok_ui::backend::fetch::package_for(file).unwrap();
    r.h.get_by_label(pkg.title);
    r.h.get_by_label("Use files you have");
    r.h.get_by_label_contains("built from fx2lafw's git");
    assert!(r.h.query_by_label("Download and install").is_none());
    let built = dir.path().join("sigrok-firmware-fx2lafw");
    std::fs::create_dir(&built).unwrap();
    for f in pkg.files {
        std::fs::write(built.join(f), b"fw").unwrap();
    }
    r.h.input_mut().dropped_files.push(Arc::new(Dropped(built.clone())));
    r.h.run();
    r.h.get_by_label(&built.display().to_string());
    r.h.get_by_label("Install from these files").click();
    wait_for(&mut r, "Installed 2 files");
    assert_eq!(std::fs::read(fw.join(file)).unwrap(), b"fw");
    rescan_clears_the_warning(&mut r);
}

/// A device that loads its firmware when the capture starts (as the
/// LogicPort does) and doesn't find it: the error toast offers the
/// install.
#[test]
fn start_failure_offers_the_firmware() {
    let file = "kingst-la2016a1-fpga.bitstream";
    let dir = tempfile::tempdir().unwrap();
    let mut r = rig_with(MockOptions {
        missing_firmware: Some(("kingst-la2016".into(), file.into())),
        firmware_dir: Some(dir.path().join("oxrok-firmware")),
        fail_start: true,
        ..MockOptions::default()
    });
    r.h.get_by_label("Mock 16 digital + 4 analog").click();
    let t0 = std::time::Instant::now();
    while !r.h.state().session().is_some_and(|s| s.caps.is_some()) {
        assert!(t0.elapsed() < Duration::from_secs(10), "the device didn't open");
        r.h.step();
    }
    r.h.key_press(Key::Space);
    wait_for(&mut r, &format!("firmware not found: {file}"));
    r.h.get_by_label("Could not begin the capture.");
    r.h.get_by_label("Install…").click();
    r.h.run();
    r.h.get_by_label("Install firmware");
    r.h.get_by_label("Download and install");
    assert!(r.h.query_by_label("Could not begin the capture.").is_none(), "the toast is gone");
}

/// A file no package provides: the dialog lists the firmware folders.
#[test]
fn install_firmware_without_a_known_source() {
    let (r, _dir, fw) = firmware_rig("odd-driver", "odd.bin", false);
    r.h.get_by_label_contains("No source is known for this file.");
    r.h.get_by_label(&format!("○ {}", fw.display()));
    assert!(r.h.query_by_label("Install from these files").is_none());
}
