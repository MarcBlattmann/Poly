//! `kernelbase.dll`'s entry point is its `DllMain` (`kernelbase_main`):
//! there is no C runtime to provide `_DllMainCRTStartup` (ADR 0078).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:kernelbase_main");
}
