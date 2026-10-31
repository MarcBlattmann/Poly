//! `httptest.exe`'s entry point and subsystem: no C runtime, so the entry
//! is our own `httptest_main` (ADR 0079, 0095).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:httptest_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
