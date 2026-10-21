//! `crttest.exe`'s entry point and subsystem (ADR 0084): its own
//! `crttest_main` does what MSVC's start-up code does with the UCRT.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:crttest_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
