//! `msvcrt.dll`'s entry point and the exports that are not Rust items: the
//! variadic functions' assembly thunks and `compiler_builtins`' memory
//! functions (ADR 0084, `poly_crt::exports`).

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/poly-crt/src/exports.rs");
    println!("cargo:rustc-link-arg=/ENTRY:msvcrt_main");
    for name in poly_crt::exports::BUILTIN_MEMORY {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
    // `__C_specific_handler` is `ntdll`'s (ADR 0087; the coverage map's
    // export list has it in both).
    println!("cargo:rustc-link-arg=/EXPORT:__C_specific_handler=ntdll.__C_specific_handler");
    // `_CxxThrowException` is assembly: its frame has an unwind handler
    // (ADR 0099).
    println!("cargo:rustc-link-arg=/EXPORT:_CxxThrowException");
    for (name, _) in poly_crt::exports::MSVCRT_VARARGS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
    // setjmp/longjmp: the shared runtime's assembly (ADR 0092).
    for name in ["setjmp", "_setjmp", "_setjmpex"] {
        println!("cargo:rustc-link-arg=/EXPORT:{name}=poly_crt_setjmp");
    }
    println!("cargo:rustc-link-arg=/EXPORT:longjmp=poly_crt_longjmp");
}
