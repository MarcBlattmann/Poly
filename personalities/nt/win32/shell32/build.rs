//! `shell32.dll`'s entry point is its `DllMain` (`shell32_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:shell32_main");
    // Three functions programs bind by ordinal (the pairs are the export table's; ADR 0155, step
    // 9): lld-link keeps the first `/EXPORT` of a name, so the ordinal wins over rustc's own.
    for (name, ordinal) in [("ILFree", 155), ("ILCreateFromPathW", 190), ("IsUserAnAdmin", 680)] {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
}
