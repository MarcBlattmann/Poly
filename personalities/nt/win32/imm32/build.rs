//! `imm32.dll`'s entry point is its `DllMain` (`imm32_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:imm32_main");
}
