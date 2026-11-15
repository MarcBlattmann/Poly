//! `user32.dll`'s entry point is its `DllMain` (`user32_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    for ordinal in APPS_ORDINALS {
        println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
    }
    // END generated
    println!("cargo:rustc-link-arg=/ENTRY:user32_main");
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_ORDINALS: &[u32] = &[
    244,
    2503,
    2505,
    2557,
];
// END generated consts
