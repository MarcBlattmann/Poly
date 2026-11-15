//! `vcruntime140_1.dll`'s entry point and its debugger hooks (assembly, ADR 0199).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:vcruntime140_1_main");
    for name in ["__NLG_Dispatch2", "__NLG_Return2"] {
        println!("cargo:rustc-link-arg=/EXPORT:{name}=poly_vcrt1_nlg");
    }
}
