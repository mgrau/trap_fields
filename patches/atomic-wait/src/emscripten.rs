// Emscripten backend (added for trapfields): WebAssembly threads block on
// Atomics.wait / Atomics.notify, exposed by Emscripten's futex API when built with -pthread.
use core::sync::atomic::AtomicU32;

extern "C" {
    fn emscripten_futex_wait(addr: *const AtomicU32, val: u32, max_wait_ms: f64) -> i32;
    fn emscripten_futex_wake(addr: *const AtomicU32, count: i32) -> i32;
}

#[inline]
pub fn wait(a: &AtomicU32, expected: u32) {
    unsafe {
        emscripten_futex_wait(a, expected, f64::INFINITY);
    }
}

#[inline]
pub fn wake_one(ptr: *const AtomicU32) {
    unsafe {
        emscripten_futex_wake(ptr, 1);
    }
}

#[inline]
pub fn wake_all(ptr: *const AtomicU32) {
    unsafe {
        emscripten_futex_wake(ptr, i32::MAX);
    }
}
