//! `shell32.dll`'s entry point is its `DllMain` (`shell32_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    for ordinal in APPS_ORDINALS {
        println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
    }
    // END generated
    println!("cargo:rustc-link-arg=/ENTRY:shell32_main");
    // Three functions programs bind by ordinal (the pairs are the export table's; ADR 0155, step
    // 9): lld-link keeps the first `/EXPORT` of a name, so the ordinal wins over rustc's own.
    for (name, ordinal) in [("ILFree", 155), ("ILCreateFromPathW", 190), ("IsUserAnAdmin", 680)] {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_ORDINALS: &[u32] = &[
    2,
    4,
    6,
    16,
    17,
    18,
    19,
    25,
    51,
    102,
    153,
    162,
    165,
    195,
    644,
    645,
    701,
    716,
    727,
];
// END generated consts
