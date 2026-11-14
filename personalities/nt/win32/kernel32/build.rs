//! `kernel32.dll`'s export forwarders (ADR 0079): one `/EXPORT:Name=
//! contract.Name` per row of `poly_kernelbase::exports::KERNEL32_FORWARDS`
//! (lld-link writes an export whose target names another DLL as a
//! forwarder string, as the PE format describes), and its entry point.

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/exports.rs");
    // ADR 0087: the `api-ms-win-core-rtlsupport` functions go to `ntdll`.
    let ntdll = poly_kernelbase::exports::KERNEL32_NTDLL_FORWARDS;
    for &(name, contract) in poly_kernelbase::exports::KERNEL32_FORWARDS.iter().chain(ntdll) {
        println!("cargo:rustc-link-arg=/EXPORT:{name}={contract}.{name}");
    }
    println!("cargo:rustc-link-arg=/ENTRY:kernel32_main");
    // The version resource (ADR 0155, step 9): programs read the system's version from this
    // file (Learn, "Getting the System Version"). lld-link takes a `.res` file as an input.
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/verinfo.rs");
    let version = poly_kernelbase::verinfo::system_dll_version_info("kernel32.dll", "Poly Windows base API client DLL");
    let res = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("version.res");
    std::fs::write(&res, poly_kernelbase::verinfo::res_file(&version)).expect("write version.res");
    println!("cargo:rustc-link-arg={}", res.display());
}
