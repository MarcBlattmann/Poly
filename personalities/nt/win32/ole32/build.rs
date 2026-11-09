//! `ole32.dll`'s entry point is its `DllMain` (`ole32_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:ole32_main");
}
