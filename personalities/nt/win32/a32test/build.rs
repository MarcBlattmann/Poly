//! `a32test.exe`'s entry point and subsystem: no C runtime, so the entry is
//! our own `a32test_main` (ADR 0079, 0083).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:a32test_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
