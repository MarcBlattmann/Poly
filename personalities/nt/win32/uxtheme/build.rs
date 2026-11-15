//! `uxtheme.dll`'s entry point is its `DllMain` (`uxtheme_main`), as for the
//! other Win32 modules (ADR 0079). `DrawThemeBackgroundEx` is also bound by
//! ordinal 47 (the pair is the export table's; Chrome's `chrome.dll` imports it
//! that way): lld-link keeps the first `/EXPORT` of a name.

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    for ordinal in APPS_ORDINALS {
        println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
    }
    // END generated
    println!("cargo:rustc-link-arg=/ENTRY:uxtheme_main");
    println!("cargo:rustc-link-arg=/EXPORT:DrawThemeBackgroundEx,@47");
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_ORDINALS: &[u32] = &[
    133,
    135,
];
// END generated consts
