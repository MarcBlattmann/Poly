//! `oleaut32.dll`'s exports with their ordinals (ADR 0087): programs built
//! with Visual C++'s import library import `oleaut32` by ordinal, so each
//! export is `/EXPORT:Name,@ordinal` from
//! `poly_kernelbase::exports::OLEAUT32_EXPORTS`; and its entry point.

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    // The 32-bit modules (ADR 0186) carry none of these: a fail-only stub takes no arguments and `stdcall` would leave them.
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "x86_64") {
        for ordinal in APPS_ORDINALS {
            println!("cargo:rustc-link-arg=/EXPORT:ord_{ordinal},@{ordinal}");
        }
    }
    // END generated
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/exports.rs");
    for &(name, ordinal) in poly_kernelbase::exports::OLEAUT32_EXPORTS {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
    println!("cargo:rustc-link-arg=/ENTRY:oleaut32_main");
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_ORDINALS: &[u32] = &[
    18,
    21,
    22,
    29,
    30,
    32,
    35,
    37,
    40,
    41,
    44,
    94,
    148,
    177,
    178,
    179,
    181,
    182,
    185,
    187,
    188,
    189,
    190,
    191,
    192,
    193,
    194,
    197,
    203,
    204,
    208,
    212,
    216,
    220,
    232,
    240,
    242,
    243,
    269,
    277,
    282,
    283,
    284,
    285,
    286,
    313,
    323,
    327,
    345,
    374,
    375,
    412,
    417,
    419,
    420,
    441,
];
// END generated consts
