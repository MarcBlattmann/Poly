//! `schedsvc.exe`'s entry point and subsystem: no C runtime, so the entry is our own
//! `schedsvc_main` (ADR 0226). A Windows program with no console: it serves a pipe and runs tasks.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:schedsvc_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
