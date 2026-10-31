//! `crtloctest.exe`'s entry point and subsystem (ADR 0094): its own
//! `crtloctest_main` is the entry point.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:crtloctest_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
