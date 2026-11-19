fn main() {
    // rpath to libgmsh for this crate's test binaries
    if let Ok(dir) = std::env::var("DEP_GMSH_LIB_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
