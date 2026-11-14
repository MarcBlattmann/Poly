//! `uiautomationcore.dll`'s entry point is its `DllMain` (`uiautomationcore_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:uiautomationcore_main");
}
