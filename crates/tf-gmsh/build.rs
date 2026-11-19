//! Locate the gmsh SDK. Order: $GMSH_DIR, then <workspace>/third_party/gmsh
//! (populated by scripts/fetch-gmsh.sh).
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=GMSH_DIR");
    let dir = std::env::var_os("GMSH_DIR").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/gmsh")
    });
    let dir = dir.canonicalize().unwrap_or_else(|_| {
        panic!(
            "gmsh SDK not found at {}. Run scripts/fetch-gmsh.sh or set GMSH_DIR.",
            dir.display()
        )
    });
    let lib = dir.join("lib");
    assert!(
        dir.join("include/gmshc.h").exists(),
        "gmshc.h missing in {}",
        dir.display()
    );
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=gmsh");
    // exported to dependents as DEP_GMSH_LIB_DIR
    println!("cargo:lib_dir={}", lib.display());
    // rpath for this crate's own tests
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
}
