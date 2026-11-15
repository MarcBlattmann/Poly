//! `dxgi.dll`'s entry point is its `DllMain` (`dxgi_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:dxgi_main");
}
