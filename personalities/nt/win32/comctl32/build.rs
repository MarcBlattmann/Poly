//! `comctl32.dll`'s entry point is its `DllMain` (`comctl32_main`), as for the
//! other Win32 modules (ADR 0079). `InitCommonControls` is also export number 17: the
//! SDK's import library imports it by that ordinal (7-Zip File Manager does).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    // The 32-bit modules (ADR 0186) carry none of these: a fail-only stub takes no arguments and `stdcall` would leave them.
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "x86_64") {
        for ordinal in APPS_ORDINALS {
            println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
        }
    }
    // END generated
    println!("cargo:rustc-link-arg=/EXPORT:InitCommonControls,@17");
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
