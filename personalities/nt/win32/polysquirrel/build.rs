//! `polysquirrel.exe`'s entry point and subsystem: no C runtime, so the entry is our own
//! `polysquirrel_main` (ADR 0253). A console program: its lines go to the standard output its
//! starter gave it.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:polysquirrel_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
