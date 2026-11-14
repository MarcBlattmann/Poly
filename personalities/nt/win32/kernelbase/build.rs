//! `kernelbase.dll`'s entry point is its `DllMain` (`kernelbase_main`):
//! there is no C runtime to provide `_DllMainCRTStartup` (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:kernelbase_main");
    // The version resource (ADR 0155, step 9): programs read the system's version from this
    // file (Learn, "Getting the System Version"). lld-link takes a `.res` file as an input.
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/verinfo.rs");
    let version = poly_kernelbase::verinfo::system_dll_version_info("kernelbase.dll", "Poly Windows base API DLL");
    let res = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("version.res");
    std::fs::write(&res, poly_kernelbase::verinfo::res_file(&version)).expect("write version.res");
    println!("cargo:rustc-link-arg={}", res.display());
}
