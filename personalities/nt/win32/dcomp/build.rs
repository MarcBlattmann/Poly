//! `dcomp.dll`'s entry point is its `DllMain` (`dcomp_main`), as for the other Win32 modules
//! (ADR 0079); ordinals 1038 and 1040, which Chrome imports by number, are fail-only exports.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:dcomp_main");
    for ordinal in [1038, 1040] {
        println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
    }
}
