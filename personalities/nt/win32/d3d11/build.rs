//! `d3d11.dll`'s entry point is its `DllMain` (`d3d11_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:d3d11_main");
}
