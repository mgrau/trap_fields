fn main() {
    // rpath to libgmsh for tests and examples of this crate
    if let Ok(dir) = std::env::var("DEP_GMSH_LIB_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
