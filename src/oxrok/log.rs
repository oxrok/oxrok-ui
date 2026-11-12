//! oxrok's log, kept on stderr, watched for firmware files a
//! driver failed to load.
//!
//! oxrok reports a missing file as `firmware file not found: NAME (hint)`.
//! The driver that was scanning or opening at the time owns the miss, as
//! with libsigrok's messages; one from elsewhere (a device that loads its
//! firmware when a capture starts) has no driver.

use crate::backend::MissingFirmware;
use oxrok::core::firmware;
use parking_lot::Mutex;
use std::sync::Once;

/// The driver being scanned or opened, whose failures are recorded.
static CURRENT: Mutex<Option<String>> = Mutex::new(None);
static MISSING: Mutex<Vec<MissingFirmware>> = Mutex::new(Vec::new());

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let msg = record.args().to_string();
            eprintln!("oxrok: {msg}");
            note(&msg);
        }
    }

    fn flush(&self) {}
}

/// Installs the logger once; later calls do nothing.
pub(crate) fn install() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = log::set_boxed_logger(Box::new(Logger));
        log::set_max_level(log::LevelFilter::Warn);
    });
}

/// Adds `files` to `list` for `driver` (empty: unknown). A file with no
/// driver and the same file with one are the same miss.
fn record(list: &mut Vec<MissingFirmware>, driver: &str, files: Vec<String>) {
    for file in files {
        let same = |m: &&mut MissingFirmware| {
            m.file == file && (m.driver == driver || m.driver.is_empty() || driver.is_empty())
        };
        match list.iter_mut().find(same) {
            Some(m) if m.driver.is_empty() => driver.clone_into(&mut m.driver),
            Some(_) => {}
            None => list.push(MissingFirmware { driver: driver.to_owned(), file }),
        }
    }
}

/// Records the file `msg` reports as missing, if it is such a message,
/// for the current driver.
pub(crate) fn note(msg: &str) {
    if let Some(file) = missing_file(msg) {
        let driver = CURRENT.lock().clone().unwrap_or_default();
        record(&mut MISSING.lock(), &driver, vec![file]);
    }
}

/// Records failures for `driver` until [`end`]; its earlier ones go.
pub(crate) fn begin(driver: &str) {
    MISSING.lock().retain(|m| m.driver != driver);
    *CURRENT.lock() = Some(driver.to_owned());
}

pub(crate) fn end() {
    *CURRENT.lock() = None;
}

/// The same, but keeps the earlier failures (a device open).
pub(crate) fn watch(driver: &str) {
    *CURRENT.lock() = Some(driver.to_owned());
}

/// The files still missing: one installed since is no longer.
pub(crate) fn missing() -> Vec<MissingFirmware> {
    let mut list = MISSING.lock();
    list.retain(|m| firmware::find_file(&m.file).is_none());
    list.clone()
}

/// The file a firmware failure names, if `msg` is one.
fn missing_file(msg: &str) -> Option<String> {
    let rest = msg.split_once("firmware file not found: ")?.1;
    let end = rest.find([' ', '(', ',']).unwrap_or(rest.len());
    let name = rest[..end].trim();
    (!name.is_empty()).then(|| name.rsplit(['/', '\\']).next().unwrap_or(name).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmware_messages() {
        assert_eq!(
            missing_file(
                "Firmware upload failed for device 1.2, name fx2lafw-cwav-usbeeax.fw: firmware file not found: fx2lafw-cwav-usbeeax.fw (install the sigrok fx2lafw firmware)"
            ),
            Some("fx2lafw-cwav-usbeeax.fw".into())
        );
        assert_eq!(missing_file("Scan found 1 device"), None);
    }

    #[test]
    fn misses_by_driver() {
        let m = |driver: &str, file: &str| MissingFirmware { driver: driver.into(), file: file.into() };
        let mut list = Vec::new();
        record(&mut list, "fx2lafw", vec!["a.fw".into(), "b.fw".into()]);
        record(&mut list, "fx2lafw", vec!["a.fw".into()]);
        assert_eq!(list, [m("fx2lafw", "a.fw"), m("fx2lafw", "b.fw")]);
        // Two drivers may need the same file.
        record(&mut list, "saleae-logic16", vec!["a.fw".into()]);
        assert_eq!(list.len(), 3);
        // A miss outside a scan has no driver, until a scan names it.
        record(&mut list, "", vec!["c.rbf".into(), "b.fw".into()]);
        assert_eq!(list[3..], [m("", "c.rbf")]);
        record(&mut list, "sysclk-lwla", vec!["c.rbf".into()]);
        assert_eq!(list[3..], [m("sysclk-lwla", "c.rbf")]);
    }
}
