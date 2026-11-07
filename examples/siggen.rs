//! Talks to the ESP32 `siggen` firmware.
//!
//! - `siggen "CLK 1000000" "SET 5"`: each argument is one command line.
//! - `siggen`: reads commands from stdin, one per line.
//! - `siggen --loop [uart] [spi] [i2c] [--period-ms N]`: repeats UART, SPI
//!   and I2C bursts (all three by default) until Ctrl-C, for decoder work.
//!
//! Commands and wiring are in AGENTS.md.

use oxrok_ui::hil::siggen::{DEFAULT_MAP, I2cWave, PROTO_MAP, Result, SigGen, proto};
use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "usage: siggen [COMMAND …] | siggen --loop [uart] [spi] [i2c] [--period-ms N]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let mut g = match SigGen::open() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("siggen: {e}");
            return ExitCode::FAILURE;
        }
    };
    let r = if args.first().is_some_and(|a| a == "--loop") {
        run_loop(&mut g, &args[1..])
    } else {
        run_commands(&mut g, args)
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("siggen: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_commands(g: &mut SigGen, args: Vec<String>) -> Result<()> {
    let interactive = args.is_empty();
    if interactive {
        eprintln!("siggen on {} — one command per line (e.g. CLK 1000 50, SET 5, MAP), Ctrl-D to quit", g.port_name);
    }
    let lines: Box<dyn Iterator<Item = String>> = if interactive {
        Box::new(std::io::stdin().lock().lines().map_while(std::io::Result::ok))
    } else {
        Box::new(args.into_iter())
    };
    let mut failed = false;
    for line in lines.map(|l| l.trim().to_owned()) {
        if line.is_empty() {
            continue;
        }
        match g.cmd(&line) {
            Ok(reply) => println!("{reply}"),
            Err(e) => {
                eprintln!("{line}: {e}");
                failed = true;
            }
        }
        let _ = std::io::stdout().flush();
    }
    if failed { Err("some commands failed".into()) } else { Ok(()) }
}

fn run_loop(g: &mut SigGen, args: &[String]) -> Result<()> {
    let (mut uart, mut spi, mut i2c) = (false, false, false);
    let mut period = Duration::from_millis(100);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "uart" => uart = true,
            "spi" => spi = true,
            "i2c" => i2c = true,
            "--period-ms" => {
                let ms =
                    it.next().and_then(|v| v.parse().ok()).ok_or(format!("--period-ms needs a number\n{USAGE}"))?;
                period = Duration::from_millis(ms);
            }
            _ => return Err(format!("unknown argument {a:?}\n{USAGE}")),
        }
    }
    if !(uart || spi || i2c) {
        (uart, spi, i2c) = (true, true, true);
    }
    eprintln!("siggen on {}, every {} ms until Ctrl-C:", g.port_name, period.as_millis());
    if uart {
        eprintln!("  UART  D21 (ESP32 TX0), 921600 8N1: \"PONG\" (every firmware reply is UART too)");
    }
    if spi {
        eprintln!("  SPI   1 MHz mode 0, MSB first: SCK D28, MOSI D5, CS D25 (active low), MISO D26: \"LA\" + counter");
    }
    if i2c {
        eprintln!("  I2C   100 kHz, bit-banged: SCL D11, SDA D12: write 0x50, register 0x10, counter, all ACKed");
    }
    g.map(&DEFAULT_MAP)?;
    for n in 0u32.. {
        let k = n as u8;
        if i2c {
            g.map(&PROTO_MAP)?;
            g.set(proto::SCL | proto::SDA | proto::CS)?;
            let mut w = I2cWave::new(2_500); // a quarter of a 100 kHz bit
            w.start();
            w.byte(0x50 << 1, true);
            w.byte(0x10, true);
            w.byte(k, true);
            w.stop();
            // CS stays high so the SPI decoder ignores the shared pins.
            let steps: Vec<_> = w.steps.iter().map(|&(m, ns)| (m | proto::CS, ns)).collect();
            g.seq(1, &steps)?;
            g.map(&DEFAULT_MAP)?;
        }
        if spi {
            g.spi(1_000_000, 0, &[b'L', b'A', k])?;
        }
        if uart {
            g.cmd("PING")?;
        }
        eprint!("\rround {}", n + 1);
        std::thread::sleep(period);
    }
    Ok(())
}
