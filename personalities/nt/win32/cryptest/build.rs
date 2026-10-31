//! `cryptest.exe`'s entry point and subsystem: no C runtime, so the entry
//! is our own `cryptest_main` (ADR 0079, 0096).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:cryptest_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
