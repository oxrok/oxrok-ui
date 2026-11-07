//! Client for the ESP32 `siggen` firmware (921600 baud line protocol).

use serialport::SerialPort;
use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

pub type Result<T> = std::result::Result<T, String>;

/// Default `MAP`: mask bits 0–3 drive GPIO25, 26, 27, 14 (D0, D1, D2, CLK1).
pub const DEFAULT_MAP: [u8; 4] = [25, 26, 27, 14];
/// Map for bit-banged buses: SCL, SDA, SCK, MOSI, CS, MISO.
pub const PROTO_MAP: [u8; 6] = [22, 21, 18, 23, 5, 19];
pub mod proto {
    pub const SCL: u32 = 1;
    pub const SDA: u32 = 2;
    pub const SCK: u32 = 4;
    pub const MOSI: u32 = 8;
    pub const CS: u32 = 16;
    pub const MISO: u32 = 32;
}

pub struct SigGen {
    reader: BufReader<Box<dyn SerialPort>>,
    pub port_name: String,
}

/// Port name fragments of the USB-serial adapters ESP32 boards use, most
/// likely first: a DevKit's CP210x / CH340 bridge shows as `usbserial` (or
/// `SLAB`, `wchusb`), while `usbmodem` is any USB CDC device, so it is a last
/// resort (other instruments on the machine show up as one).
const PORT_KEYS: [&str; 6] = ["usbserial", "SLAB", "wchusb", "ttyUSB", "usbmodem", "ttyACM"];

/// The USB serial port that looks like the ESP32 board, or `LA_SIGGEN_PORT`.
pub fn find_port() -> Result<String> {
    if let Ok(p) = std::env::var("LA_SIGGEN_PORT") {
        return Ok(p);
    }
    let ports = serialport::available_ports().map_err(|e| e.to_string())?;
    // macOS: use the cu.* callout device, not tty.*.
    let names: Vec<String> = ports.into_iter().map(|p| p.port_name).filter(|n| !n.starts_with("/dev/tty.")).collect();
    PORT_KEYS
        .iter()
        .find_map(|k| names.iter().find(|n| n.contains(k)))
        .cloned()
        .ok_or_else(|| "ESP32 serial port not found (set LA_SIGGEN_PORT)".into())
}

impl SigGen {
    pub fn open() -> Result<Self> {
        let name = find_port()?;
        let mut port = serialport::new(&name, 921_600)
            .timeout(Duration::from_millis(200))
            .open()
            .map_err(|e| format!("{name}: {e}"))?;
        // Don't hold the chip in reset or boot mode.
        let _ = port.write_data_terminal_ready(false);
        let _ = port.write_request_to_send(false);
        // Opening may reset the ESP32; give it time to boot.
        std::thread::sleep(Duration::from_millis(1500));
        let _ = port.clear(serialport::ClearBuffer::Input);
        let mut g = Self { reader: BufReader::new(port), port_name: name };
        let pong = g.cmd("PING")?;
        if pong != "PONG" {
            return Err(format!("siggen answered {pong:?} to PING"));
        }
        Ok(g)
    }

    /// Sends one command line without waiting.
    pub fn send(&mut self, line: &str) -> Result<()> {
        let port = self.reader.get_mut();
        port.write_all(format!("{line}\n").as_bytes()).map_err(|e| e.to_string())?;
        port.flush().map_err(|e| e.to_string())
    }

    /// Next non-empty line that isn't the boot banner.
    pub fn read_reply(&mut self, timeout: Duration) -> Result<String> {
        let t0 = Instant::now();
        let mut line = String::new();
        while t0.elapsed() < timeout {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(_) => {
                    let l = line.trim();
                    if !l.is_empty() && !l.starts_with("SIGGEN") {
                        return Ok(l.to_owned());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("siggen: no reply".into())
    }

    pub fn cmd(&mut self, line: &str) -> Result<String> {
        self.send(line)?;
        self.read_reply(Duration::from_secs(30))
    }

    pub fn map(&mut self, gpios: &[u8]) -> Result<String> {
        let list: Vec<String> = gpios.iter().map(u8::to_string).collect();
        self.cmd(&format!("MAP {}", list.join(" ")))
    }

    pub fn set(&mut self, mask: u32) -> Result<String> {
        self.cmd(&format!("SET {mask}"))
    }

    /// Square wave on `gpio` (LEDC). Returns the frequency the ESP32 reports.
    pub fn clk(&mut self, hz: u32, duty_percent: u32, gpio: u8) -> Result<f64> {
        let r = self.cmd(&format!("CLK {hz} {duty_percent} {gpio}"))?;
        if hz == 0 {
            return Ok(0.0);
        }
        // "CLK 1000 Hz (actual 1000) bits 14 gpio 14"
        r.split("actual ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| format!("unexpected CLK reply {r:?}"))
    }

    /// Plays `(mask, ns)` steps `repeat` times; waits for `DONE`.
    pub fn seq(&mut self, repeat: u32, steps: &[(u32, u64)]) -> Result<String> {
        let body: Vec<String> = steps.iter().map(|(m, ns)| format!("{m}:{ns}")).collect();
        self.cmd(&format!("SEQ {repeat} {}", body.join(" ")))
    }

    pub fn spi(&mut self, hz: u32, mode: u8, bytes: &[u8]) -> Result<String> {
        self.cmd(&format!("SPI {hz} {mode} {}", hex(bytes)))
    }

    pub fn i2c(&mut self, hz: u32, addr: u8, bytes: &[u8]) -> Result<String> {
        self.cmd(&format!("I2C {hz} {addr} {}", hex(bytes)))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Bit-banged I2C waveform for [`SigGen::seq`] with [`PROTO_MAP`]; `q` is a
/// quarter bit period in ns. Both sides of the bus are generated.
pub struct I2cWave {
    q: u64,
    pub steps: Vec<(u32, u64)>,
}

impl I2cWave {
    pub fn new(q: u64) -> Self {
        Self { q, steps: vec![(proto::SCL | proto::SDA, 4 * q)] }
    }

    fn put(&mut self, scl: bool, sda: bool, quarters: u64) {
        let m = if scl { proto::SCL } else { 0 } | if sda { proto::SDA } else { 0 };
        self.steps.push((m, quarters * self.q));
    }

    pub fn start(&mut self) {
        self.put(true, false, 2);
        self.put(false, false, 1);
    }

    pub fn repeated_start(&mut self) {
        self.put(false, true, 1);
        self.put(true, true, 2);
        self.start();
    }

    pub fn stop(&mut self) {
        self.put(false, false, 1);
        self.put(true, false, 2);
        self.put(true, true, 4);
    }

    fn bit(&mut self, b: bool) {
        self.put(false, b, 1);
        self.put(true, b, 2);
        self.put(false, b, 1);
    }

    /// Eight data bits, MSB first, then the acknowledge bit (low = ACK).
    pub fn byte(&mut self, v: u8, ack: bool) {
        for i in (0..8).rev() {
            self.bit(v >> i & 1 == 1);
        }
        self.bit(!ack);
    }
}
