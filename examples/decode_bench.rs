//! Times the I2C decoder: oxrok's native one, the Python one, and the
//! adapter the app uses, on the same generated signal.
//!
//! `cargo run -p la-oxrok --features rustpython --release --example decode_bench`

use oxrok::decode::{Config, DecoderDef, Output, Registry, Stack};
use std::time::Instant;

/// I2C-like: SCL bit 0, SDA bit 1, 10 samples per half period.
fn signal(transactions: usize) -> Vec<u8> {
    let half = 10usize;
    let mut bits: Vec<u8> = Vec::new();
    let push = |scl: u8, sda: u8, n: usize, bits: &mut Vec<u8>| {
        for _ in 0..n {
            bits.push(scl | (sda << 1));
        }
    };
    push(1, 1, half, &mut bits);
    for t in 0..transactions {
        push(1, 1, half, &mut bits);
        push(1, 0, half, &mut bits);
        push(0, 0, half, &mut bits);
        let byte = (t % 256) as u8;
        for bit in 0..9u8 {
            let sda = if bit < 8 { (byte >> bit) & 1 } else { 1 };
            push(0, sda, half, &mut bits);
            push(1, sda, half, &mut bits);
        }
        push(0, 0, half, &mut bits);
        push(1, 0, half, &mut bits);
        push(1, 1, half, &mut bits);
        push(1, 1, half * 2, &mut bits);
    }
    bits
}

fn decode(name: &str, def: &'static DecoderDef, data: &[u8], chunk: usize) {
    let cfg = Config { channels: vec![Some(0), Some(1)], ..Config::default() };
    let t = Instant::now();
    let mut stack = Stack::new();
    stack.add(format!("{name}-1"), (def.new)(), &cfg, 1, 1_000_000).unwrap();
    let rx = stack.outputs().unwrap();
    for c in data.chunks(chunk) {
        stack.feed(c);
    }
    stack.end();
    let mut n = 0;
    for out in rx.iter() {
        if let Output::Annotation { .. } = out {
            n += 1;
        }
    }
    println!("{name:8}: {} samples, {n} annotations, {:?}", data.len(), t.elapsed());
}

fn main() {
    let data = signal(30_000);
    println!("generated {} samples", data.len());
    let mut reg = Registry::new();
    reg.register(&oxrok::decoders::ALL);
    reg.register(&oxrok::pydecode::defs());
    let native = reg.find("i2c").unwrap();
    decode("native", native, &data, 4 << 20);

    if let Some(py) = oxrok::pydecode::defs().iter().find(|d| d.info.id == "i2c").copied() {
        decode("python", py, &data, 4 << 20);
    }

    use oxrok_ui::decode::{DecodeSpec, DecoderEngine};
    let mut spec = DecodeSpec {
        decoder: "i2c".into(),
        samplerate: 1_000_000,
        options: Default::default(),
        channels: Default::default(),
    };
    spec.channels.insert("scl".into(), 0);
    spec.channels.insert("sda".into(), 1);
    let chunk: usize = std::env::var("CHUNK").ok().and_then(|v| v.parse().ok()).unwrap_or(1 << 20);
    let mut eng = oxrok_ui::oxrok::OxrokDecoders::new(None).unwrap();
    let t = Instant::now();
    let mut s = eng.start(&spec).unwrap();
    let mut n = 0;
    for c in data.chunks(chunk) {
        let t1 = Instant::now();
        n += s.send(0, c, 1).unwrap().len();
        let d = t1.elapsed();
        if d.as_millis() > 20 {
            println!("  send {} samples took {d:?}", c.len());
        }
    }
    n += s.finish().unwrap().len();
    println!("adapter (chunk {chunk}): {} samples, {n} annotations, {:?}", data.len(), t.elapsed());
}
