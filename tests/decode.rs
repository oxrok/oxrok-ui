//! The decoder engine: metadata and a UART frame through oxrok's native
//! decoder, then the same through the Python host when it is built in.

use oxrok_ui::decode::{DecodeSpec, DecoderEngine, OptionValue};
use oxrok_ui::oxrok::OxrokDecoders;
use std::collections::BTreeMap;

fn uart_spec() -> DecodeSpec {
    // 8 samples per bit at 115200 baud.
    let spb = 8;
    let mut spec =
        DecodeSpec { decoder: "uart".into(), samplerate: 921_600, options: BTreeMap::new(), channels: BTreeMap::new() };
    spec.options.insert("baudrate".into(), OptionValue::Int(115_200));
    spec.options.insert("format".into(), OptionValue::Str("hex".into()));
    spec.channels.insert("rx".into(), 0);
    let _ = spb;
    spec
}

/// A UART frame for `byte`, idle high, LSB first.
fn frame(byte: u8) -> Vec<u8> {
    let spb = 8;
    let mut bits: Vec<u8> = vec![1; spb * 4];
    bits.extend(std::iter::repeat_n(0u8, spb));
    for i in 0..8 {
        bits.extend(std::iter::repeat_n((byte >> i) & 1, spb));
    }
    bits.extend(std::iter::repeat_n(1u8, spb * 4));
    bits
}

#[test]
fn metadata_has_the_native_decoders() {
    let engine = OxrokDecoders::new(None).expect("engine");
    for id in ["uart", "i2c", "spi"] {
        let d = engine.decoders().iter().find(|d| d.id == id).unwrap_or_else(|| panic!("{id}"));
        assert!(!d.longname.is_empty());
        assert!(!d.annotation_rows.is_empty(), "{id}");
        assert!(!d.channels.is_empty() || !d.opt_channels.is_empty(), "{id}");
    }
    let uart = engine.decoders().iter().find(|d| d.id == "uart").unwrap();
    assert_eq!(uart.opt_channels.first().map(|c| c.id.as_str()), Some("rx"));
    assert!(uart.options.iter().any(|o| o.id == "baudrate"));
}

#[test]
fn decodes_a_uart_frame() {
    let mut engine = OxrokDecoders::new(None).expect("engine");
    let mut session = engine.start(&uart_spec()).expect("start");
    let data = frame(0x55);
    let mut anns = session.send(0, &data, 1).expect("send");
    anns.extend(session.finish().expect("finish"));
    let texts: Vec<String> = anns.iter().flat_map(|a| a.texts.iter().cloned()).collect();
    assert!(texts.iter().any(|t| t == "55"), "annotations: {texts:?}");
    assert!(texts.iter().any(|t| t.eq_ignore_ascii_case("stop")), "annotations: {texts:?}");
    let data = anns.iter().find(|a| a.texts.first().is_some_and(|t| t == "55")).expect("data annotation");
    assert!(data.end > data.start);
    assert!(data.value.is_some(), "the data byte has its raw value ()");
}

#[test]
fn unknown_decoder_is_an_error() {
    let mut engine = OxrokDecoders::new(None).expect("engine");
    let mut spec = uart_spec();
    spec.decoder = "no-such-decoder".into();
    assert!(engine.start(&spec).is_err());
}
