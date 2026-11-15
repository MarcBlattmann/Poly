//! `shcore.dll`'s entry point is its `DllMain` (`shcore_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    // The 32-bit modules (ADR 0186) carry none of these: a fail-only stub takes no arguments and `stdcall` would leave them.
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "x86_64") {
        for (name, target) in APPS_FORWARDS {
            println!("cargo:rustc-link-arg=/EXPORT:{name}={target}");
        }
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
