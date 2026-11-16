//! `vcruntime140_1.dll`'s entry point, its debugger hooks (assembly) and `__CxxFrameHandler4`,
//! a forwarder to Poly's `vcruntime140.dll`, which keeps the C++ exception state of the process
//! (ADR 0199).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:vcruntime140_1_main");
    for name in ["__NLG_Dispatch2", "__NLG_Return2"] {
        println!("cargo:rustc-link-arg=/EXPORT:{name}=poly_vcrt1_nlg");
    }
    println!("cargo:rustc-link-arg=/EXPORT:__CxxFrameHandler4=vcruntime140.__CxxFrameHandler4");
}
