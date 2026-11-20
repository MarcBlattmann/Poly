//! `comctl32.dll`'s entry point is its `DllMain` (`comctl32_main`), as for the
//! other Win32 modules (ADR 0079). `InitCommonControls` is also export number 17: the
//! SDK's import library imports it by that ordinal (7-Zip File Manager does).

fn main() {
    println!("cargo:rustc-link-arg=/EXPORT:InitCommonControls,@17");
    // Window subclassing: by name and by these ordinals (`commctrl.h`'s functions; Google's
    // updater imports them by ordinal), in both widths.
    for (name, ordinal) in [
        ("SetWindowSubclass", 410),
        ("GetWindowSubclass", 411),
        ("RemoveWindowSubclass", 412),
        ("DefSubclassProc", 413),
    ] {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
    // The icon loaders by name and by the ordinals of Windows' own comctl32.dll (Notepad++ imports `LoadIconWithScaleDown` by 381).
    println!("cargo:rustc-link-arg=/EXPORT:LoadIconWithScaleDown,@381");
    println!("cargo:rustc-link-arg=/EXPORT:LoadIconMetric,@380");
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    // The 32-bit modules (ADR 0186) carry none of these: a fail-only stub takes no arguments and `stdcall` would leave them.
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "x86_64") {
        for ordinal in APPS_ORDINALS {
            println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
        }
    }
    // END generated
    println!("cargo:rustc-link-arg=/ENTRY:comctl32_main");
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_ORDINALS: &[u32] = &[
    345,
];
// END generated consts
