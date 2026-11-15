//! `ws2_32.dll`'s entry point is its `DllMain` (`ws2_32_main`), as for the
//! other Win32 modules (ADR 0079). Its exports are its `#[no_mangle]`
//! functions, by name; the ones programs bind by ordinal are pinned to the
//! numbers of `poly_winsock::exports::ORDINALS` (lld-link keeps the first
//! `/EXPORT` of a name, so the ordinal wins over the one rustc passes).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    for ordinal in APPS_ORDINALS {
        println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
    }
    // END generated
    println!("cargo:rerun-if-changed=../../../../crates/poly-winsock/src/exports.rs");
    println!("cargo:rustc-link-arg=/ENTRY:ws2_32_main");
    for &(name, ordinal) in poly_winsock::exports::ORDINALS {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_ORDINALS: &[u32] = &[
    51,
    101,
    103,
    108,
];
// END generated consts
