//! `k32test.exe`'s entry point and subsystem: no C runtime, so the entry is
//! our own `k32test_main` (ADR 0076).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:k32test_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    // No manifest: the test checks the version Learn documents for an
    // unmanifested program.
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
