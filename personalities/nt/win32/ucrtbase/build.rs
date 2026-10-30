//! `ucrtbase.dll`'s entry point (`DllMain`, no C runtime below it), the
//! memory functions it exports from the compiler's own `compiler_builtins`
//! (ADR 0084) and the shared runtime's variadic thunks (ADR 0093).

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/poly-crt/src/exports.rs");
    println!("cargo:rustc-link-arg=/ENTRY:ucrtbase_main");
    for name in poly_crt::exports::BUILTIN_MEMORY {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
    for (name, _) in poly_crt::exports::SHARED_VARARGS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
}
