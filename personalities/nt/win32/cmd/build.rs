//! `cmd.exe`'s entry point, subsystem and version resource (ADR 0240): no C runtime, so the
//! entry is our own `cmd_main`; a console program. Programs read its version resource
//! (`AssocQueryStringW`'s `ASSOCSTR_FRIENDLYAPPNAME` is its `FileDescription`, recorded as
//! "Windows Command Processor" in `specs/windows/shlwapi.dll/AssocQueryStringW.yaml`).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:cmd_main");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rustc-link-arg=/MANIFEST:NO");
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/verinfo.rs");
    let version = poly_kernelbase::verinfo::system_exe_version_info("cmd.exe", "Windows Command Processor");
    let res = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("version.res");
    std::fs::write(&res, poly_kernelbase::verinfo::res_file(&version)).expect("write version.res");
    println!("cargo:rustc-link-arg={}", res.display());
}
