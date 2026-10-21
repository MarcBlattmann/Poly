//! `contest.exe`'s entry point and subsystem: no C runtime, so the entry is
//! our own `contest_main`; a console program, so it gets a console (ADR
//! 0080).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:contest_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
