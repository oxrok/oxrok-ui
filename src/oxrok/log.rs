//! oxrok's log, kept on stderr, watched for firmware files a
//! driver failed to load.
//!
//! oxrok reports a missing file as `firmware file not found: NAME (hint)`;
//! the driver that was scanning or opening at the time owns it, as with
//! libsigrok's messages.

use crate::backend::MissingFirmware;
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
        if !self.enabled(record.metadata()) {
            return;
        }
        let msg = record.args().to_string();
        eprintln!("oxrok: {msg}");
        if let (Some(file), Some(driver)) = (missing_file(&msg), CURRENT.lock().clone()) {
            let m = MissingFirmware { driver, file };
            let mut list = MISSING.lock();
            if !list.contains(&m) {
                list.push(m);
            }
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

pub(crate) fn missing() -> Vec<MissingFirmware> {
    MISSING.lock().clone()
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
}
