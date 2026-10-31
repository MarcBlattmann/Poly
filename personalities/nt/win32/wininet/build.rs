//! `wininet.dll`'s entry point is its `DllMain` (`wininet_main`), as for
//! the other Win32 modules (ADR 0079). Its exports are its `#[no_mangle]`
//! functions, by name.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:wininet_main");
}
