//! Bench tests: ESP32 signal generator → Intronix LogicPort → our backend
//! and store. Run with `make test-hil` (needs `LA_HIL=1` and a libsigrok
//! with the `intronix-logicport` driver; see `AGENTS.md`).

use oxrok_ui::backend::{HwTrigger, TriggerMatch};
use oxrok_ui::core::ChannelId;
use oxrok_ui::hil::bench::{Bench, CaptureSpec, Captured, MEM_DEPTH};
use oxrok_ui::hil::ch::*;
use oxrok_ui::hil::decode::{self, I2c};
use oxrok_ui::hil::siggen::{DEFAULT_MAP, I2cWave, PROTO_MAP, SigGen, proto};
use parking_lot::{Mutex, MutexGuard};
use std::sync::OnceLock;
use std::time::Duration;

struct Rig {
    bench: Bench,
    siggen: SigGen,
}

/// One libsigrok context and one serial port per process: tests take turns.
fn rig() -> Option<MutexGuard<'static, Rig>> {
    if !oxrok_ui::hil::enabled() {
        eprintln!("skipped: set LA_HIL=1 (make test-hil) to run the bench tests");
        return None;
    }
    static RIG: OnceLock<Mutex<Rig>> = OnceLock::new();
    let rig = RIG.get_or_init(|| {
        let bench = Bench::open().expect("LogicPort");
        let mut siggen = SigGen::open().expect("ESP32 siggen");
        siggen.clk(0, 50, 14).unwrap();
        siggen.map(&DEFAULT_MAP).unwrap();
        siggen.set(0).unwrap();
        eprintln!("bench: {} ({}) + siggen on {}", bench.info.display_name(), bench.info.id, siggen.port_name);
        Mutex::new(Rig { bench, siggen })
    });
    let mut g = rig.lock();
    // Leave the generator in its resting state between tests.
    g.siggen.clk(0, 50, 14).unwrap();
    g.siggen.map(&DEFAULT_MAP).unwrap();
    g.siggen.set(0).unwrap();
    Some(g)
}

fn trigger(matches: &[(ChannelId, TriggerMatch)], pre: u8) -> HwTrigger {
    HwTrigger::one_stage(matches.to_vec(), pre)
}

fn no_stimulus() {}

/// Mean period and duty of a clock, in samples, from all complete periods.
fn period_and_duty(c: &Captured, id: ChannelId) -> (f64, f64) {
    let edges = c.edges(id);
    let ch = c.get(id);
    let rising: Vec<u64> = edges.iter().copied().filter(|&t| ch.level_at(t) == Some(true)).collect();
    // Two rising edges are one full period (a 2 ms window holds 2–3 at 1 kHz).
    assert!(rising.len() >= 2, "too few rising edges: {}", rising.len());
    let periods = (rising.len() - 1) as f64;
    let period = (rising[rising.len() - 1] - rising[0]) as f64 / periods;
    let high: u64 = rising[..rising.len() - 1].iter().map(|&r| ch.next_transition(r).expect("falling edge") - r).sum();
    (period, high as f64 / periods / period * 100.0)
}

#[test]
fn bench_device_is_a_logicport() {
    let Some(r) = rig() else { return };
    let info = &r.bench.info;
    assert!(info.model.contains("LogicPort"), "{info:?}");
    assert_eq!(info.digital_channels().count(), 34);
    assert_eq!(info.channels[32].device_name, "CLK1");
    let caps = r.bench.device.capabilities();
    assert!(caps.hw_trigger.is_some() || !caps.supports_continuous, "{caps:?}");
}

#[test]
fn static_levels_reach_d0_to_d2() {
    let Some(mut r) = rig() else { return };
    for mask in [0b000, 0b001, 0b010, 0b100, 0b101, 0b111] {
        r.siggen.set(mask).unwrap();
        let c = r.bench.capture(&CaptureSpec::new(1_000_000, &[D0, D1, D2]), no_stimulus, Duration::MAX).unwrap();
        assert_eq!(c.samples(), MEM_DEPTH);
        for (bit, id) in [D0, D1, D2].into_iter().enumerate() {
            let want = mask >> bit & 1 == 1;
            let ch = c.get(id);
            assert_eq!(ch.transition_count(), 0, "mask {mask:03b}: {id:?} toggles");
            assert_eq!(ch.level_at(0), Some(want), "mask {mask:03b}: {id:?}");
        }
    }
}

/// LEDC clock on CLK1 at several frequencies and sample rates.
#[test]
fn clock_frequency_and_duty() {
    let Some(mut r) = rig() else { return };
    for (hz, rate) in
        [(1_000u32, 1_000_000u64), (100_000, 10_000_000), (1_000_000, 100_000_000), (5_000_000, 200_000_000)]
    {
        let actual = r.siggen.clk(hz, 50, 14).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let c = r.bench.capture(&CaptureSpec::new(rate, &[CLK1]), no_stimulus, Duration::MAX).unwrap();
        let (period, duty) = period_and_duty(&c, CLK1);
        let measured = rate as f64 / period;
        eprintln!("CLK {hz} Hz (ESP32: {actual}) at {rate} S/s → {measured:.1} Hz, duty {duty:.2} %");
        // One sample of quantization over the measured span, plus 0.1 %.
        let tol = actual * (1.0 / (MEM_DEPTH as f64 - 2.0 * period) + 1e-3);
        assert!((measured - actual).abs() <= tol, "{hz} Hz: measured {measured}, ESP32 says {actual}");
        let duty_tol = 100.0 / period + 1.0;
        assert!((duty - 50.0).abs() <= duty_tol, "{hz} Hz: duty {duty}");
    }
    r.siggen.clk(0, 50, 14).unwrap();
}

/// With run-length compression a slow signal fills far more
/// time than the 2048-entry memory holds; without it the memory caps it.
#[test]
fn rle_records_seconds_of_a_slow_clock() {
    let Some(mut r) = rig() else { return };
    let actual = r.siggen.clk(100, 50, 14).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let rate = 1_000_000;
    let wanted = 2 * rate; // 2 s

    let plain = CaptureSpec { samples: wanted, ..CaptureSpec::new(rate, &[CLK1]) };
    let c = r.bench.capture(&plain, no_stimulus, Duration::MAX).unwrap();
    eprintln!("without RLE: {} samples", c.samples());
    assert!(c.samples() <= MEM_DEPTH, "the memory caps an uncompressed capture");

    let rle = CaptureSpec { rle: true, ..plain };
    let c = r.bench.capture(&rle, no_stimulus, Duration::MAX).unwrap();
    let (period, duty) = period_and_duty(&c, CLK1);
    let measured = rate as f64 / period;
    let rising = (c.edges(CLK1).len() / 2) as f64;
    eprintln!(
        "with RLE: {} samples, {rising} periods, {measured:.2} Hz (ESP32: {actual}), duty {duty:.2} %",
        c.samples()
    );
    r.siggen.clk(0, 50, 14).unwrap();
    assert!(c.samples() as f64 >= wanted as f64 * 0.99, "{} of {wanted} samples", c.samples());
    // 2 s of 100 Hz: about 200 periods, all of them seen.
    assert!((rising - 200.0).abs() <= 2.0, "{rising} periods");
    assert!((measured - actual).abs() <= actual * 2e-3, "measured {measured}, ESP32 says {actual}");
    assert!((duty - 50.0).abs() <= 1.0, "duty {duty}");
}

/// A cycle-timed pulse train on D0, triggered on its rising edge.
#[test]
fn seq_pulse_widths_with_hw_trigger() {
    let Some(mut r) = rig() else { return };
    let rate = 10_000_000;
    let (high_ns, low_ns) = (10_000u64, 30_000u64);
    let spec = CaptureSpec::new(rate, &[D0, D1]).trigger(trigger(&[(D0, TriggerMatch::Rising)], 10));
    let Rig { bench, siggen } = &mut *r;
    let fire = || drop(siggen.seq(200, &[(1, high_ns), (0, low_ns)]).unwrap());
    let c = bench.capture(&spec, fire, Duration::from_millis(700)).unwrap();
    let t = c.trigger.expect("trigger position");
    let d0 = c.get(D0);
    eprintln!("trigger at sample {t} of {}", c.samples());
    // The trigger lands on (or within 2 samples of) a rising edge.
    let edge = d0.next_transition(t.saturating_sub(3)).unwrap();
    assert!(edge.abs_diff(t) <= 2 && d0.level_at(edge) == Some(true), "edge {edge} vs trigger {t}");
    let (period, duty) = period_and_duty(&c, D0);
    let want_period = (high_ns + low_ns) as f64 * rate as f64 / 1e9;
    eprintln!("SEQ period {period:.2} samples (want {want_period}), duty {duty:.2} %");
    // SEQ is a software loop: each step runs a little long (~0.4 % measured),
    // unlike the LEDC clock, which is exact.
    assert!((period - want_period).abs() <= want_period * 0.01, "period {period} samples, want {want_period}");
    assert!((duty - 25.0).abs() <= 1.0, "duty {duty}");
    assert_eq!(c.get(D1).transition_count(), 0, "D1 stays low");
}

/// The firmware's serial replies appear on D21 (ESP32 TX0) at 921600 baud.
#[test]
fn uart_reply_on_d21() {
    let Some(mut r) = rig() else { return };
    let spec = CaptureSpec::new(10_000_000, &[UART_TX]).trigger(trigger(&[(UART_TX, TriggerMatch::Falling)], 5));
    let Rig { bench, siggen } = &mut *r;
    let fire = || assert_eq!(siggen.cmd("PING").unwrap(), "PONG");
    let c = bench.capture(&spec, fire, Duration::from_millis(700)).unwrap();
    let bytes = decode::uart(c.get(UART_TX), c.rate, 921_600);
    let text = String::from_utf8_lossy(&bytes);
    eprintln!("UART: {text:?}");
    assert!(text.starts_with("PONG\r\n"), "{text:?}");
}

/// ESP32 hardware SPI (VSPI), mode 0 at 1 MHz.
#[test]
fn hardware_spi_mosi() {
    let Some(mut r) = rig() else { return };
    let payload = [0xA5, 0x3C, 0x0F, 0xF0, 0x01, 0x80];
    let spec = CaptureSpec::new(10_000_000, &[SCK, MOSI, CS, MISO]).trigger(trigger(&[(CS, TriggerMatch::Falling)], 5));
    let Rig { bench, siggen } = &mut *r;
    let fire = || drop(siggen.spi(1_000_000, 0, &payload).unwrap());
    let c = bench.capture(&spec, fire, Duration::from_millis(700)).unwrap();
    let frames = decode::spi_mode0(c.get(SCK), c.get(MOSI), c.get(CS));
    eprintln!("SPI frames: {frames:02X?}");
    assert_eq!(frames.first().map(Vec::as_slice), Some(&payload[..]));
}

/// ESP32 hardware I2C with no slave: START, address + W, NACK, STOP.
///
/// The firmware re-initializes the I2C pins for every command, which can
/// glitch a START-like edge ahead of the real transfer and win the trigger;
/// such captures are retried.
#[test]
fn hardware_i2c_address_nack() {
    let Some(mut r) = rig() else { return };
    let spec = CaptureSpec::new(2_000_000, &[SCL, SDA])
        .trigger(trigger(&[(SDA, TriggerMatch::Falling), (SCL, TriggerMatch::One)], 5));
    let Rig { bench, siggen } = &mut *r;
    let want = [I2c::Start, I2c::Byte(0x3C << 1, false)];
    let mut seen = Vec::new();
    for attempt in 1..=3 {
        let fire = || drop(siggen.i2c(100_000, 0x3C, &[0x12, 0x34]).unwrap());
        let c = bench.capture(&spec, fire, Duration::from_millis(700)).unwrap();
        let ev = decode::i2c(c.get(SCL), c.get(SDA));
        eprintln!("I2C attempt {attempt}: {ev:?}");
        if ev.starts_with(&want) {
            assert!(ev.contains(&I2c::Stop), "{ev:?}");
            return;
        }
        seen.push(ev);
    }
    panic!("no address byte in 3 captures: {seen:?}");
}

/// A full bit-banged I2C transaction, both sides generated: write, repeated
/// start, read with ACK/NACK.
#[test]
fn bitbanged_i2c_transaction() {
    let Some(mut r) = rig() else { return };
    r.siggen.map(&PROTO_MAP).unwrap();
    r.siggen.set(proto::SCL | proto::SDA | proto::CS).unwrap();
    let mut w = I2cWave::new(2_500); // 100 kHz
    w.start();
    w.byte(0x50 << 1, true);
    w.byte(0x10, true);
    w.repeated_start();
    w.byte(0x50 << 1 | 1, true);
    w.byte(0xDE, true);
    w.byte(0xAD, false);
    w.stop();
    let steps = w.steps.iter().map(|&(m, ns)| (m | proto::CS, ns)).collect::<Vec<_>>();
    let spec = CaptureSpec::new(2_000_000, &[SCL, SDA])
        .trigger(trigger(&[(SDA, TriggerMatch::Falling), (SCL, TriggerMatch::One)], 5));
    let Rig { bench, siggen } = &mut *r;
    let fire = || drop(siggen.seq(1, &steps).unwrap());
    let c = bench.capture(&spec, fire, Duration::from_millis(700)).unwrap();
    let ev = decode::i2c(c.get(SCL), c.get(SDA));
    eprintln!("I2C: {ev:?}");
    let want = [
        I2c::Start,
        I2c::Byte(0xA0, true),
        I2c::Byte(0x10, true),
        I2c::Start,
        I2c::Byte(0xA1, true),
        I2c::Byte(0xDE, true),
        I2c::Byte(0xAD, false),
        I2c::Stop,
    ];
    assert_eq!(ev[..want.len()], want, "{ev:?}");
}
