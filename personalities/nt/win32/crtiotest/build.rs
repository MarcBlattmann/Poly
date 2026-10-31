//! `crtiotest.exe`'s entry point and subsystem (ADR 0093): its own
//! `crtiotest_main` does what MSVC's start-up code does with the UCRT.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:crtiotest_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
