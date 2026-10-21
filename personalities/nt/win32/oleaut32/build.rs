//! `oleaut32.dll`'s exports with their ordinals (ADR 0087): programs built
//! with Visual C++'s import library import `oleaut32` by ordinal, so each
//! export is `/EXPORT:Name,@ordinal` from
//! `poly_kernelbase::exports::OLEAUT32_EXPORTS`; and its entry point.

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/exports.rs");
    for &(name, ordinal) in poly_kernelbase::exports::OLEAUT32_EXPORTS {
        println!("cargo:rustc-link-arg=/EXPORT:{name},@{ordinal}");
    }
    println!("cargo:rustc-link-arg=/ENTRY:oleaut32_main");
}
