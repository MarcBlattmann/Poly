//! `shlwapi.dll`'s entry point is its `DllMain` (`shlwapi_main`), as for the
//! other Win32 modules (ADR 0079). Three of its functions are imported by
//! ordinal (Learn: `SHCreateMemStream` is ordinal 12, `QISearch` 219, `IsOS`
//! 437; before Windows Vista they were exported by ordinal only, and
//! programs still bind them that way): lld-link keeps the first `/EXPORT`
//! of a name, so the ordinal wins over the one rustc passes.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:shlwapi_main");
    for (name, ordinal) in [("SHCreateMemStream", 12), ("QISearch", 219), ("IsOS", 437)] {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
}
