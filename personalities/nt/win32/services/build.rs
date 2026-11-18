//! `services.exe`'s entry point and subsystem: no C runtime, so the entry is our own
//! `services_main`; a Windows program (it has no console).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:services_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
}
