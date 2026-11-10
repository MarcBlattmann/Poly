//! `dwrite.dll`'s entry point is its `DllMain` (`dwrite_main`), as for the
//! other Win32 modules (ADR 0079). Its export is `DWriteCreateFactory`.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:dwrite_main");
}
