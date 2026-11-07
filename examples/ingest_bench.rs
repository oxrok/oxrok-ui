//! Ingest throughput: idle 5-byte samples (a LogicPort with RLE expanded
//! by the driver) in 4096-sample packets.
use oxrok_ui::store::LogicIngest;
use std::time::Instant;

fn main() {
    let us = 5;
    let per = 4096;
    let total: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(200_000_000);
    let mut packet = vec![0u8; per * us];
    // One transition per packet on bit 0, like a slow clock.
    for i in per / 2..per {
        packet[i * us] = 1;
    }
    let mut ing = LogicIngest::new(us, (0..34).collect());
    let t = Instant::now();
    let mut s = 0;
    while s < total {
        ing.push(s, &packet).unwrap();
        s += per as u64;
    }
    let dt = t.elapsed().as_secs_f64();
    println!("{} samples in {dt:.2} s: {:.2} ns/sample", s, dt * 1e9 / s as f64);
}
