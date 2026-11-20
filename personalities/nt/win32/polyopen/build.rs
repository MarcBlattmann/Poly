//! `polyopen.exe`'s entry point and subsystem: no C runtime, so the entry is our own
//! `polyopen_main` (ADR 0230). A console program: its one line of output (a failure) goes to the
//! standard output its starter gave it.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:polyopen_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
