//! `ws2_32.dll`'s entry point is its `DllMain` (`ws2_32_main`), as for the
//! other Win32 modules (ADR 0079). Its exports are its `#[no_mangle]`
//! functions, by name; the ones programs bind by ordinal are pinned to the
//! numbers of `poly_winsock::exports::ORDINALS` (lld-link keeps the first
//! `/EXPORT` of a name, so the ordinal wins over the one rustc passes).

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/poly-winsock/src/exports.rs");
    println!("cargo:rustc-link-arg=/ENTRY:ws2_32_main");
    for &(name, ordinal) in poly_winsock::exports::ORDINALS {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
}
