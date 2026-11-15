//! `comctl32.dll`'s entry point is its `DllMain` (`comctl32_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    for ordinal in APPS_ORDINALS {
        println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
    }
    // END generated
    println!("cargo:rustc-link-arg=/ENTRY:comctl32_main");
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_ORDINALS: &[u32] = &[
    17,
    345,
    381,
    410,
    411,
    412,
    413,
];
// END generated consts
