//! `comctl32.dll`'s entry point is its `DllMain` (`comctl32_main`), as for the
//! other Win32 modules (ADR 0079). `InitCommonControls` is also export number 17: the
//! SDK's import library imports it by that ordinal (7-Zip File Manager does).

fn main() {
    println!("cargo:rustc-link-arg=/EXPORT:InitCommonControls,@17");
    println!("cargo:rustc-link-arg=/ENTRY:comctl32_main");
}
