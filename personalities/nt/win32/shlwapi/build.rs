//! `shlwapi.dll`'s entry point is its `DllMain` (`shlwapi_main`), as for the
//! other Win32 modules (ADR 0079). Three of its functions are imported by
//! ordinal (Learn: `SHCreateMemStream` is ordinal 12, `QISearch` 219, `IsOS`
//! 437; before Windows Vista they were exported by ordinal only, and
//! programs still bind them that way): lld-link keeps the first `/EXPORT`
//! of a name, so the ordinal wins over the one rustc passes.

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    // The 32-bit modules (ADR 0186) carry none of these: a fail-only stub takes no arguments and `stdcall` would leave them.
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "x86_64") {
        for ordinal in APPS_ORDINALS {
            println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
        }
    }
    // END generated
    println!("cargo:rustc-link-arg=/ENTRY:shlwapi_main");
    for (name, ordinal) in [("SHCreateMemStream", 12), ("QISearch", 219), ("IsOS", 437)] {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_ORDINALS: &[u32] = &[
    2,
    176,
    213,
];
// END generated consts
