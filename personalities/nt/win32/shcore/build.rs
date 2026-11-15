//! `shcore.dll`'s entry point is its `DllMain` (`shcore_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    for (name, target) in APPS_FORWARDS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}={target}");
    }
    // END generated
    println!("cargo:rustc-link-arg=/ENTRY:shcore_main");
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_FORWARDS: &[(&str, &str)] = &[
    ("CommandLineToArgvW", "shell32.CommandLineToArgvW"),
];
// END generated consts
