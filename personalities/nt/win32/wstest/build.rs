//! `wstest.exe`'s entry point and subsystem: no C runtime, so the entry is
//! our own `wstest_main` (ADR 0079, 0095).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:wstest_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
