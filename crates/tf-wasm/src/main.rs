//! WebAssembly engine: [`tf_app::App`] behind a C ABI, driven from a web worker
//! (frontend/public/engine/engine-worker.js).
//!
//! Calls run on the worker's thread, one at a time; rayon spreads the heavy loops over
//! the pthread pool. Results go into an output buffer that JS reads with tf_out_ptr /
//! tf_out_len; events from long commands are pushed to JS through `tf_emit`.

use std::cell::RefCell;
use std::path::Path;
use tf_app::{App, Event, Reply};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[cfg(target_os = "emscripten")]
extern "C" {
    fn tf_emit(ptr: *const u8, len: usize);
}

fn emit(e: Event) {
    #[cfg(target_os = "emscripten")]
    if let Ok(s) = serde_json::to_vec(&e) {
        unsafe { tf_emit(s.as_ptr(), s.len()) };
    }
    #[cfg(not(target_os = "emscripten"))]
    let _ = e;
}

fn set_out(bytes: Vec<u8>) {
    OUT.with(|o| *o.borrow_mut() = bytes);
}

unsafe fn slice<'a>(p: *const u8, n: usize) -> &'a [u8] {
    if n == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(p, n)
    }
}

/// Allocate `n` bytes for JS to write an argument into.
#[no_mangle]
pub extern "C" fn tf_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `p` must come from `tf_alloc(n)` with the same `n`.
#[no_mangle]
pub unsafe extern "C" fn tf_dealloc(p: *mut u8, n: usize) {
    drop(Vec::from_raw_parts(p, 0, n.max(1)));
}

#[no_mangle]
pub extern "C" fn tf_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn tf_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// Create the app on the project directory `dir` with `threads` worker threads.
/// Returns 0, or 1 with an error message in the output buffer.
///
/// # Safety
/// `dir` must point to `len` bytes of UTF-8.
#[no_mangle]
pub unsafe extern "C" fn tf_init(dir: *const u8, len: usize, threads: usize) -> i32 {
    let _ = rayon::ThreadPoolBuilder::new().num_threads(threads.max(1)).build_global();
    let dir = String::from_utf8_lossy(slice(dir, len)).into_owned();
    match App::new(Path::new(&dir)) {
        Ok(app) => {
            APP.with(|a| *a.borrow_mut() = Some(app));
            set_out(Vec::new());
            0
        }
        Err(e) => {
            set_out(format!("{e:#}").into_bytes());
            1
        }
    }
}

/// Run a command. Returns 0 (JSON in the output buffer), 1 (binary), or 2 (error JSON
/// {status, message}).
///
/// # Safety
/// The pointers must reference the given number of bytes (UTF-8 for `cmd` and `args`).
#[no_mangle]
pub unsafe extern "C" fn tf_command(cmd: *const u8, cmd_len: usize, args: *const u8, args_len: usize, data: *const u8, data_len: usize) -> i32 {
    let cmd = String::from_utf8_lossy(slice(cmd, cmd_len)).into_owned();
    let args: serde_json::Value = serde_json::from_slice(slice(args, args_len)).unwrap_or(serde_json::Value::Null);
    let data = (data_len > 0).then(|| slice(data, data_len));
    let r = APP.with(|a| {
        let mut a = a.borrow_mut();
        match a.as_mut() {
            Some(app) => app.command(&cmd, args, data, &emit),
            None => Err(tf_app::AppError::new(500, "engine not initialized")),
        }
    });
    match r {
        Ok(Reply::Json(v)) => {
            set_out(serde_json::to_vec(&v).unwrap_or_default());
            0
        }
        Ok(Reply::Binary(b)) => {
            set_out(b);
            1
        }
        Err(e) => {
            set_out(serde_json::to_vec(&serde_json::json!({"status": e.status, "message": e.message})).unwrap_or_default());
            2
        }
    }
}

/// OpenCASCADE's memory statistics (OSD_MemInfo) call mallinfo, which mimalloc does not
/// provide; nothing we use reads those statistics.
#[cfg(target_os = "emscripten")]
#[repr(C)]
pub struct MallInfo {
    fields: [i32; 10],
}

#[cfg(target_os = "emscripten")]
#[no_mangle]
pub extern "C" fn mallinfo() -> MallInfo {
    MallInfo { fields: [0; 10] }
}

fn main() {}
