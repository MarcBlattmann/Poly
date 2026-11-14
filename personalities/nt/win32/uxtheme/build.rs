//! `uxtheme.dll`'s entry point is its `DllMain` (`uxtheme_main`), as for the
//! other Win32 modules (ADR 0079). `DrawThemeBackgroundEx` is also bound by
//! ordinal 47 (the pair is the export table's; Chrome's `chrome.dll` imports it
//! that way): lld-link keeps the first `/EXPORT` of a name.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:uxtheme_main");
    println!("cargo:rustc-link-arg=/EXPORT:DrawThemeBackgroundEx,@47");
}
