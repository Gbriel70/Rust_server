//! Counting allocator is restricted to this standalone measurement executable.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
#[path = "support/phase12/baseline_parser.rs"]
mod baseline;
static ENABLED: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
struct Counting;
// SAFETY: every operation delegates unchanged pointer/layout contracts to System.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) }
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(size, Ordering::Relaxed);
        }
        unsafe { System.realloc(p, layout, size) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;
fn measure(name: &str, bytes: &[u8], borrowed: bool) {
    const N: usize = 10000;
    CALLS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::Relaxed);
    for _ in 0..N {
        if borrowed {
            black_box(rust_serv::http::parser::parse_borrowed(black_box(bytes)))
                .unwrap()
                .unwrap();
        } else {
            black_box(baseline::parse_request(black_box(bytes)))
                .unwrap()
                .unwrap();
        }
    }
    ENABLED.store(false, Ordering::Relaxed);
    println!(
        "{name}: calls/request={} requested_bytes/request={}",
        CALLS.load(Ordering::Relaxed) / N,
        BYTES.load(Ordering::Relaxed) / N
    );
}
fn main() {
    measure(
        "get/before",
        b"GET /health HTTP/1.1\r\nHost: x\r\nUser-Agent: benchmark\r\nAccept: */*\r\n\r\n",
        false,
    );
    measure(
        "get/after",
        b"GET /health HTTP/1.1\r\nHost: x\r\nUser-Agent: benchmark\r\nAccept: */*\r\n\r\n",
        true,
    );
    let mut fixed = b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 4096\r\n\r\n".to_vec();
    fixed.resize(fixed.len() + 4096, 42);
    measure("fixed4096/before", &fixed, false);
    measure("fixed4096/after", &fixed, true);
    let mut chunked =
        b"POST /echo HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n1000\r\n".to_vec();
    chunked.resize(chunked.len() + 4096, 42);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    measure("chunked4096/before", &chunked, false);
    measure("chunked4096/after", &chunked, true);
}
