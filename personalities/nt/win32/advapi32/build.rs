//! `advapi32.dll`'s export forwarders (ADR 0083): one `/EXPORT:Name=
//! contract.Name` per row of `poly_kernelbase::advapi::ADVAPI32_FORWARDS`
//! (lld-link writes an export whose target names another DLL as a
//! forwarder string), and its entry point. As `kernel32`'s (ADR 0079).

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/advapi.rs");
    for &(name, contract) in poly_kernelbase::advapi::ADVAPI32_FORWARDS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}={contract}.{name}");
    }
    println!("cargo:rustc-link-arg=/ENTRY:advapi32_main");
}
