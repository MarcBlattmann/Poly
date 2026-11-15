//! `msvcp140.dll`'s entry point, its forwarder, and the generated parts (ADR 0199): the
//! exception classes' RTTI and throw records (assembly, `poly_msvcp::classes`) and the
//! fail-only exports of `fail-only.txt` (Rust, `poly_msvcp::exports`).

use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:msvcp140_main");
    // The Concurrency Runtime's thread id is kernel32's.
    println!(
        "cargo:rustc-link-arg=/EXPORT:?GetCurrentThreadId@platform@details@Concurrency@@YAJXZ=kernel32.GetCurrentThreadId"
    );
    println!("cargo:rerun-if-changed=fail-only.txt");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let list = std::fs::read_to_string("fail-only.txt").expect("fail-only.txt");
    let names = poly_msvcp::exports::parse_list(&list);
    std::fs::write(out.join("stubs.rs"), poly_msvcp::exports::stubs_rust("msvcp140", &names)).expect("stubs.rs");
    std::fs::write(out.join("records.s"), poly_msvcp::classes::records_asm()).expect("records.s");
}
