//! memory_bytes matches what the stores really allocate,
//! within 10 %. One test per binary: the counting allocator sees every
//! allocation of the process.

use oxrok_ui::store::{AnalogChannel, DigitalChannel};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Ordering::SeqCst);
        unsafe { System.alloc(l) }
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Ordering::SeqCst);
        unsafe { System.dealloc(p, l) }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        LIVE.fetch_add(new as isize - l.size() as isize, Ordering::SeqCst);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn within(name: &str, reported: usize, actual: isize) {
    let ratio = reported as f64 / actual as f64;
    eprintln!("{name}: reported {reported} B, allocated {actual} B ({ratio:.3})");
    assert!((0.9..=1.1).contains(&ratio), "{name}: {ratio:.3}");
}

#[test]
fn stores_report_what_they_allocate() {
    let before = LIVE.load(Ordering::SeqCst);
    let mut d = DigitalChannel::new();
    d.start(0, false);
    let mut s = 0u64;
    for i in 0..2_000_000u64 {
        s += 1 + (i * 2_654_435_761) % 997;
        d.push_transition(s);
    }
    d.extend_to(s + 1);
    within("digital, 2 M transitions", d.memory_bytes(), LIVE.load(Ordering::SeqCst) - before);
    drop(d);

    let before = LIVE.load(Ordering::SeqCst);
    let mut a = AnalogChannel::new();
    let block: Vec<f32> = (0..100_000).map(|i| (i as f32 * 0.01).sin()).collect();
    let block_bytes = LIVE.load(Ordering::SeqCst) - before;
    for k in 0..50u64 {
        a.push(k * 100_000, &block).unwrap();
    }
    within("analog, 5 M samples", a.memory_bytes(), LIVE.load(Ordering::SeqCst) - before - block_bytes);
}
