//! `msimg32.dll`'s entry point is its `DllMain` (`msimg32_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:msimg32_main");
}
