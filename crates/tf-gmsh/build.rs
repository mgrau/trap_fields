//! Locate gmsh.
//!
//! Native: the gmsh SDK (shared library) from $GMSH_DIR, else <workspace>/third_party/gmsh
//! (populated by scripts/fetch-gmsh.sh).
//!
//! WebAssembly (target_os = "emscripten"): static gmsh + OpenCASCADE built with
//! -pthread by scripts/build-wasm-gmsh.sh, from $GMSH_WASM_LIB and $OCCT_WASM_LIB, else
//! <workspace>/wasm-build/{gmsh-occ-mt, occt-install-mt/lib} (…-mt64 for wasm64). The archives are exported as
//! DEP_GMSH_WASM_LIBS for the final binary to link (see crates/tf-wasm/build.rs).
use std::path::PathBuf;

/// OpenCASCADE toolkits gmsh needs for STEP import and meshing, in link order.
const OCC_TOOLKITS: &[&str] = &[
    "TKDESTEP", "TKDEIGES", "TKDE", "TKXCAF", "TKVCAF", "TKCAF", "TKLCAF", "TKCDF", "TKV3d", "TKService", "TKXSBase", "TKOffset", "TKFeat",
    "TKFillet", "TKBool", "TKMesh", "TKHLR", "TKBO", "TKPrim", "TKShHealing", "TKTopAlgo", "TKGeomAlgo", "TKBRep", "TKGeomBase", "TKG3d",
    "TKG2d", "TKMath", "TKernel",
];

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("emscripten") {
        println!("cargo:rerun-if-env-changed=GMSH_WASM_LIB");
        println!("cargo:rerun-if-env-changed=OCCT_WASM_LIB");
        // wasm64 (Memory64) links the libraries built with WASM64=1 scripts/build-wasm-gmsh.sh
        let sfx = if std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").as_deref() == Ok("64") { "mt64" } else { "mt" };
        let gmsh = std::env::var_os("GMSH_WASM_LIB").map(PathBuf::from).unwrap_or_else(|| root.join(format!("wasm-build/gmsh-occ-{sfx}")));
        let occt = std::env::var_os("OCCT_WASM_LIB").map(PathBuf::from).unwrap_or_else(|| root.join(format!("wasm-build/occt-install-{sfx}/lib")));
        let libgmsh = gmsh.join("libgmsh.a");
        assert!(libgmsh.exists(), "{} missing: run scripts/build-wasm-gmsh.sh", libgmsh.display());
        // Link arguments do not propagate from a dependency's build script, so the final
        // binary's build script links these (as one group) from DEP_GMSH_WASM_LIBS.
        let mut libs = vec![libgmsh.display().to_string()];
        libs.extend(OCC_TOOLKITS.iter().map(|t| occt.join(format!("lib{t}.a")).display().to_string()));
        println!("cargo:wasm_libs={}", libs.join(";"));
        return;
    }

    println!("cargo:rerun-if-env-changed=GMSH_DIR");
    let dir = std::env::var_os("GMSH_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("third_party/gmsh"));
    let dir = dir
        .canonicalize()
        .unwrap_or_else(|_| panic!("gmsh SDK not found at {}. Run scripts/fetch-gmsh.sh or set GMSH_DIR.", dir.display()));
    let lib = dir.join("lib");
    assert!(dir.join("include/gmshc.h").exists(), "gmshc.h missing in {}", dir.display());
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=gmsh");
    // exported to dependents as DEP_GMSH_LIB_DIR
    println!("cargo:lib_dir={}", lib.display());
    // rpath for this crate's own tests
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
}
