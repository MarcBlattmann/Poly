//! `s1test.exe`'s entry point and subsystem: no C runtime, so the entry is
//! our own `s1test_main` (ADR 0079, 0155).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:s1test_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
