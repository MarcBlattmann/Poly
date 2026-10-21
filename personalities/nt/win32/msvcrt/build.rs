//! `msvcrt.dll`'s entry point and the exports that are not Rust items: the
//! variadic functions' assembly thunks and `compiler_builtins`' memory
//! functions (ADR 0084, `poly_crt::exports`).

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/poly-crt/src/exports.rs");
    println!("cargo:rustc-link-arg=/ENTRY:msvcrt_main");
    for name in poly_crt::exports::BUILTIN_MEMORY {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
    for (name, _) in poly_crt::exports::MSVCRT_VARARGS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
}
