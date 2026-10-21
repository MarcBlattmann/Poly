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
}
