//! FrameStore::memory_bytes matches what it allocates,
//! within 10 %. One test per binary: the counting allocator sees every
//! allocation of the process.

use oxrok_ui::decode::{Frame, FrameStore};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
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
fn frame_store_reports_what_it_allocates() {
    let before = LIVE.load(Ordering::SeqCst);
    let mut f = FrameStore::default();
    for i in 0..500_000u64 {
        let texts: Arc<[String]> = Arc::from(vec![format!("Data {i:#04x}"), format!("{i:02X}")]);
        f.push(Frame { start: i * 10, end: i * 10 + 8, class: 0, row: (i % 3) as usize, texts, error: false });
    }
    within("decoder results, 500 k frames", f.memory_bytes(), LIVE.load(Ordering::SeqCst) - before);
}
