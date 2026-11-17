//! `comlaunch.exe`'s entry point and subsystem: no C runtime, so the entry is our own
//! `comlaunch_main` (ADR 0220). A Windows program with no console: it serves pipes only.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:comlaunch_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
