//! Times packing: `cargo run [--release] -p la-decode --example pack_bench`.
use oxrok_ui::store::DigitalChannel;
use std::time::Instant;

fn main() {
    let n: u64 = 16 << 20;
    let mut chans = Vec::new();
    for k in 0..3u64 {
        let mut c = DigitalChannel::new();
        c.start(0, true);
        // Sparse traffic: a 1 µs-ish toggle burst every ~1 ms at 100 MHz.
        let mut t = 1000 + k * 7;
        while t < n - 2000 {
            for j in 0..20 {
                c.push_transition(t + j * 100);
            }
            t += 100_000;
        }
        c.extend_to(n);
        chans.push(c);
    }
    let refs: Vec<&DigitalChannel> = chans.iter().collect();
    let t0 = Instant::now();
    let mut s = 0;
    while s < n {
        let e = (s + (1 << 20)).min(n);
        std::hint::black_box(oxrok_ui::decode::pack::pack(&refs, s, e));
        s = e;
    }
    let dt = t0.elapsed().as_secs_f64();
    println!("pack 3 ch × 16 Mi samples: {dt:.2} s = {:.0} Msamples/s", n as f64 / dt / 1e6);
}
