//! The real protocol decoders on bench captures: SPI, I2C and UART from
//! the ESP32 generator, through the LogicPort.

use oxrok_ui::backend::{HwTrigger, TriggerMatch};
use oxrok_ui::core::ChannelId;
use oxrok_ui::decode::{DecodeSpec, DecoderEngine, OptionValue, RawAnn};
use oxrok_ui::hil::bench::{Bench, CaptureSpec, Captured};
use oxrok_ui::hil::ch::*;
use oxrok_ui::hil::siggen::{DEFAULT_MAP, SigGen};
use oxrok_ui::oxrok::OxrokDecoders;
use std::collections::BTreeMap;
use std::time::Duration;

fn trigger(matches: &[(ChannelId, TriggerMatch)], pre: u8) -> HwTrigger {
    HwTrigger::one_stage(matches.to_vec(), pre)
}

/// Decodes `c` with `decoder`, roles mapped to captured channels.
fn decode(
    e: &mut OxrokDecoders,
    c: &Captured,
    decoder: &str,
    roles: &[(&str, ChannelId)],
    opts: &[(&str, OptionValue)],
) -> Vec<RawAnn> {
    let chans: Vec<&oxrok_ui::store::DigitalChannel> = roles.iter().map(|(_, id)| c.get(*id)).collect();
    let (buf, us) = oxrok_ui::decode::pack::pack(&chans, 0, c.samples()).expect("pack");
    let spec = DecodeSpec {
        decoder: decoder.into(),
        samplerate: c.rate,
        options: opts.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<BTreeMap<_, _>>(),
        channels: roles.iter().enumerate().map(|(k, (r, _))| (r.to_string(), k)).collect(),
    };
    let mut s = e.start(&spec).expect("start");
    let mut anns = s.send(0, &buf, us).expect("send");
    anns.extend(s.finish().expect("finish"));
    anns
}

fn texts(e: &OxrokDecoders, decoder: &str, anns: &[RawAnn], class: &str) -> Vec<String> {
    let info = e.decoders().iter().find(|d| d.id == decoder).unwrap();
    let k = info.annotations.iter().position(|a| a.0 == class).unwrap();
    anns.iter().filter(|a| a.class == k).map(|a| a.texts[0].clone()).collect()
}

#[test]
fn real_decoders_on_the_bench() {
    if !oxrok_ui::hil::enabled() {
        eprintln!("skipped: set LA_HIL=1 (make test-hil) to run the bench tests");
        return;
    }
    let mut bench = Bench::open().expect("LogicPort");
    let mut siggen = SigGen::open().expect("ESP32 siggen");
    siggen.clk(0, 50, 14).unwrap();
    siggen.map(&DEFAULT_MAP).unwrap();
    let mut e = OxrokDecoders::new(None).expect("decoders");
    eprintln!("{}", oxrok_ui::oxrok::library_info());

    // SPI mode 0 at 1 MHz.
    let payload = [0xA5u8, 0x3C, 0x0F, 0xF0, 0x01, 0x80];
    let spec = CaptureSpec::new(10_000_000, &[SCK, MOSI, CS, MISO]).trigger(trigger(&[(CS, TriggerMatch::Falling)], 5));
    let c =
        bench.capture(&spec, || drop(siggen.spi(1_000_000, 0, &payload).unwrap()), Duration::from_millis(700)).unwrap();
    let anns = decode(&mut e, &c, "spi", &[("clk", SCK), ("mosi", MOSI), ("cs", CS), ("miso", MISO)], &[]);
    let mosi = texts(&e, "spi", &anns, "mosi-data");
    eprintln!("SPI MOSI: {mosi:?}");
    assert_eq!(&mosi[..6], ["A5", "3C", "0F", "F0", "01", "80"]);

    // I2C at 100 kHz to 0x3C (no slave: NACK). Retried: the firmware can
    // glitch a START-like edge ahead of the transfer (see the bench tests).
    let spec = CaptureSpec::new(2_000_000, &[SCL, SDA])
        .trigger(trigger(&[(SDA, TriggerMatch::Falling), (SCL, TriggerMatch::One)], 5));
    let mut ok = false;
    for attempt in 1..=3 {
        let c = bench
            .capture(&spec, || drop(siggen.i2c(100_000, 0x3C, &[0x12, 0x34]).unwrap()), Duration::from_millis(700))
            .unwrap();
        let anns = decode(&mut e, &c, "i2c", &[("scl", SCL), ("sda", SDA)], &[]);
        let addr = texts(&e, "i2c", &anns, "address-write");
        let nack = texts(&e, "i2c", &anns, "nack");
        eprintln!("I2C attempt {attempt}: address {addr:?}, nack {nack:?}");
        // The row also carries the R/W bit's "Write", so look at every text.
        if addr.iter().any(|a| a.contains("3C")) && !nack.is_empty() {
            ok = true;
            break;
        }
    }
    assert!(ok, "no I2C address write to 3C decoded");

    // UART: the generator's reply on D21 at 921600 baud.
    let spec = CaptureSpec::new(10_000_000, &[UART_TX]).trigger(trigger(&[(UART_TX, TriggerMatch::Falling)], 5));
    let c =
        bench.capture(&spec, || assert_eq!(siggen.cmd("PING").unwrap(), "PONG"), Duration::from_millis(700)).unwrap();
    let opts = [("baudrate", OptionValue::Int(921_600)), ("format", OptionValue::Str("ascii".into()))];
    let anns = decode(&mut e, &c, "uart", &[("rx", UART_TX)], &opts);
    let text: String = texts(&e, "uart", &anns, "rx-data").concat();
    eprintln!("UART: {text:?}");
    assert!(text.starts_with("PONG[0D][0A]"), "{text:?}");
}
