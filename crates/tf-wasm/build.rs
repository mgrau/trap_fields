//! Link the WebAssembly engine: static gmsh + OpenCASCADE (from tf-gmsh's build metadata),
//! Emscripten threads, the JS event hook, and the module shape the web worker expects.
use std::path::PathBuf;

fn main() {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("emscripten") {
        // native build (workspace checks): rpath to libgmsh
        if let Ok(dir) = std::env::var("DEP_GMSH_LIB_DIR") {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
        }
        return;
    }
    let libs = std::env::var("DEP_GMSH_WASM_LIBS").expect("tf-gmsh did not report its WebAssembly libraries");
    let arg = |a: &str| println!("cargo:rustc-link-arg={a}");
    arg("-Wl,--start-group");
    for l in libs.split(';') {
        arg(l);
    }
    arg("-Wl,--end-group");
    let js = here.join("src/engine_lib.js");
    println!("cargo:rerun-if-changed={}", js.display());
    for a in [
        // OpenCASCADE reports errors with C++ exceptions (caught inside gmsh); native
        // WebAssembly exceptions, as rustc's Emscripten target uses
        "-fwasm-exceptions",
        // rustc links with emcc: pull in libc++ as em++ would
        "-sDEFAULT_TO_CXX=1",
        "-pthread",
        // the default allocator serializes threads
        "-sMALLOC=mimalloc",
        "-sPTHREAD_POOL_SIZE=navigator.hardwareConcurrency",
        "-sALLOW_MEMORY_GROWTH=1",
        "-sMAXIMUM_MEMORY=4GB",
        "-sINITIAL_MEMORY=128MB",
        "-sSTACK_SIZE=16MB",
        "-sDEFAULT_PTHREAD_STACK_SIZE=4MB",
        // an ES module factory: the worker imports it, and pthread workers locate the
        // script through import.meta.url
        "-sMODULARIZE=1",
        "-sEXPORT_ES6=1",
        "-sEXPORT_NAME=createEngine",
        "-sENVIRONMENT=web,worker",
        "-sINVOKE_RUN=0",
        "-sEXIT_RUNTIME=0",
        "-sEXPORTED_FUNCTIONS=_main,_tf_init,_tf_command,_tf_alloc,_tf_dealloc,_tf_out_ptr,_tf_out_len,_tf_stop_ptr",
        "-sEXPORTED_RUNTIME_METHODS=FS,IDBFS,wasmMemory",
        "-lidbfs.js",
    ] {
        arg(a);
    }
    arg(&format!("--js-library={}", js.display()));
}
