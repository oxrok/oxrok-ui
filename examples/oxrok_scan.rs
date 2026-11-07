//! Prints the oxrok in use, its drivers, and the devices it finds.
//!
//! `cargo run -p la-oxrok --example oxrok_scan [-- --info | --options | [driver …]]`
//! or `make oxrok-scan` / `make oxrok-info`.

use oxrok_ui::backend::Backend;
use oxrok_ui::oxrok::OxrokBackend;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let info = args.iter().any(|a| a == "--info");
    let options = args.iter().any(|a| a == "--options");
    let only: Vec<String> = args.into_iter().filter(|a| !a.starts_with("--")).collect();

    println!("{}", oxrok_ui::oxrok::library_info());
    let backend = OxrokBackend::with_drivers((!only.is_empty()).then_some(only.clone()));
    if info || only.is_empty() {
        let drivers = backend.drivers();
        println!("{} drivers:", drivers.len());
        for (id, long) in drivers.iter().filter(|(id, _)| only.is_empty() || only.contains(id)) {
            println!("  {id:24} {long}");
        }
        if info {
            let decoders = oxrok_ui::oxrok::OxrokDecoders::new(None).expect("decoders");
            use oxrok_ui::decode::DecoderEngine;
            println!("{} decoders", decoders.decoders().len());
            return;
        }
    }
    let devices = backend.scan_devices();
    println!("{} devices:", devices.len());
    for d in &devices {
        println!("  {}  {} {} ({})", d.id, d.display_name(), d.version, d.serial_or_conn);
    }
    if options {
        for d in &devices {
            let Ok(mut dev) = backend.open(&d.id) else { continue };
            println!("{}:", d.id);
            for g in dev.read_options() {
                println!("  {:?}:", g.scope);
                for o in g.options {
                    println!("    {:24} = {:?} ({:?})", o.id, o.value, o.kind);
                }
            }
        }
    }
}
