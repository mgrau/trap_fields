fn main() {
    // rpath to libgmsh for the `trapfields` binary
    if let Ok(dir) = std::env::var("DEP_GMSH_LIB_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
