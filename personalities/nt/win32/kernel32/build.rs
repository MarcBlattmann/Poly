//! `kernel32.dll`'s export forwarders (ADR 0078): one `/EXPORT:Name=
//! contract.Name` per row of `poly_kernelbase::exports::KERNEL32_FORWARDS`
//! (lld-link writes an export whose target names another DLL as a
//! forwarder string, as the PE format describes), and its entry point.

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/exports.rs");
    for &(name, contract) in poly_kernelbase::exports::KERNEL32_FORWARDS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}={contract}.{name}");
    }
    println!("cargo:rustc-link-arg=/ENTRY:kernel32_main");
}
