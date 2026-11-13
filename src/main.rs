use oxrok_ui::backend::Backend;
#[cfg(feature = "mock")]
use oxrok_ui::backend::{MockBackend, MockOptions, Streaming};
use std::sync::Arc;

fn backend() -> Arc<dyn Backend> {
    #[cfg(feature = "mock")]
    {
        // Real-time streaming so Continuous mode behaves like hardware.
        let clock = Arc::new(oxrok_ui::core::MonotonicClock::new());
        Arc::new(MockBackend::new(MockOptions { streaming: Streaming::RealTime(clock), ..MockOptions::default() }))
    }
    #[cfg(not(feature = "mock"))]
    {
        eprintln!("{}", oxrok_ui::oxrok::library_info());
        Arc::new(oxrok_ui::oxrok::OxrokBackend::new())
    }
}

/// `--self-test`: the libraries and resources a bundle finds, then exit
/// (packaging smoke tests). Fails without oxrok or its decoders.
fn self_test() -> i32 {
    let mut ok = true;
    println!("{} {}", oxrok_ui::app::APP_NAME, env!("CARGO_PKG_VERSION"));
    println!("resources: {:?}", oxrok_ui::app::firmware::resource_dir());
    for key in ["OXROK_FIRMWARE_DIR", "SIGROK_FIRMWARE_DIR", "OXROK_DECODERS_DIR", "SIGROKDECODE_DIR"] {
        println!("{key}={}", std::env::var(key).unwrap_or_default());
    }
    use oxrok_ui::decode::DecoderEngine;
    let b = oxrok_ui::oxrok::OxrokBackend::new();
    println!("{}", oxrok_ui::oxrok::library_info());
    let devices = b.scan_devices();
    println!("{} devices: {:?}", devices.len(), devices.iter().map(|d| d.id.0.clone()).collect::<Vec<_>>());
    let firmware = b.firmware_status();
    for (p, exists) in firmware.paths {
        println!("firmware path {} {}", if exists { "✓" } else { "○" }, p.display());
    }
    println!("firmware installs into {:?}", firmware.install_dir);
    ok &= devices.iter().any(|d| d.driver == "demo");
    match oxrok_ui::oxrok::OxrokDecoders::new(None) {
        Ok(e) => {
            println!("{}: {} decoders", oxrok_ui::oxrok::library_info(), e.decoders().len());
            ok &= ["uart", "i2c", "spi"].iter().all(|id| e.decoders().iter().any(|d| d.id == *id));
        }
        Err(e) => {
            println!("oxrok decoders: {e}");
            ok = false;
        }
    }
    println!("{}", if ok { "self-test passed" } else { "self-test FAILED" });
    if ok { 0 } else { 1 }
}

fn main() -> eframe::Result {
    // Packaging: the bundled firmware, decoders and Python.
    // SAFETY: no other thread runs yet.
    unsafe { oxrok_ui::app::firmware::apply_bundle_env() };
    if std::env::args().any(|a| a == "--self-test") {
        std::process::exit(self_test());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(oxrok_ui::app::APP_NAME)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0])
            .with_icon(std::sync::Arc::new(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("bundled icon"),
            )),
        persist_window: true,
        // winit's default macOS menu quits on Cmd+Q without
        // asking the window, so unsaved captures would be lost. Without it
        // Cmd+Q is a key press the app turns into a close request, which
        // the quit prompt can cancel. (The app's own menus come in 0.8.0.)
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_default_menu(false);
        })),
        ..Default::default()
    };
    eframe::run_native(
        oxrok_ui::app::APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(oxrok_ui::app::LaApp::new(cc, backend())))),
    )
}
