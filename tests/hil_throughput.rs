//! A long RLE capture's download and ingest time (run on demand:
//! `make test-hil ARGS="--test throughput -- --ignored"`).

use oxrok_ui::hil::bench::{Bench, CaptureSpec};
use oxrok_ui::hil::ch::CLK1;
use std::time::{Duration, Instant};

#[test]
#[ignore = "takes ~10 s of bench time"]
fn rle_200msps_5s() {
    if !oxrok_ui::hil::enabled() {
        return;
    }
    let mut bench = Bench::open().expect("LogicPort");
    let rate = 200_000_000;
    let spec = CaptureSpec { samples: 5 * rate, rle: true, ..CaptureSpec::new(rate, &[CLK1]) };
    let t = Instant::now();
    let c = bench.capture(&spec, || {}, Duration::MAX).unwrap();
    eprintln!("{} samples in {:.1} s", c.samples(), t.elapsed().as_secs_f64());
}
