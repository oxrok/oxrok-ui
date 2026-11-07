//! On the bench: hardware-triggered captures sized by
//! `buffered_trigger_plan` for several pre-trigger times. Needs the
//! generator's D0 bursts running (the demo loop, not driven here).

use oxrok_ui::backend::{HwTrigger, TriggerMatch};
use oxrok_ui::hil::bench::{Bench, CaptureSpec};
use oxrok_ui::hil::ch::*;
use std::time::{Duration, Instant};

#[test]
fn pre_trigger_matrix() {
    if !oxrok_ui::hil::enabled() {
        eprintln!("skipped: set LA_HIL=1 (make test-hil) to run the bench tests");
        return;
    }
    let mut bench = Bench::open().expect("LogicPort");
    let only = std::env::var("PRE_CASES").ok();
    for rle in [false, true] {
        for rate in [1_000_000u64, 100_000_000] {
            for (pre, post) in [(0.0, 1.0), (0.1, 1.0), (0.1, 0.9), (1.0, 1.0)] {
                let name = format!("rle={rle} rate={rate} pre={pre} post={post}");
                if only.as_deref().is_some_and(|o| !name.contains(o)) {
                    continue;
                }
                let (limit, pct) = oxrok_ui::app::buffered_trigger_plan(pre, post, rate);
                let mut spec = CaptureSpec::new(rate, &[D0, D1, D2])
                    .trigger(HwTrigger { matches: vec![(D0, TriggerMatch::Rising)], pre_trigger_percent: pct });
                spec.samples = limit;
                spec.rle = rle;
                let t0 = Instant::now();
                match bench.capture(&spec, || {}, Duration::MAX) {
                    Ok(c) => eprintln!(
                        "{name}: limit {limit} pct {pct} -> {} samples, trigger {:?}, {} D0 edges, {:?} in {:.1?}",
                        c.samples(),
                        c.trigger,
                        c.edges(D0).len(),
                        c.end,
                        t0.elapsed()
                    ),
                    Err(e) => eprintln!("{name}: limit {limit} pct {pct} -> ERROR {e} after {:.1?}", t0.elapsed()),
                }
            }
        }
    }
}
