//! `ucrtbase.dll`'s entry point (`DllMain`, no C runtime below it) and the
//! memory functions it exports from the compiler's own `compiler_builtins`
//! (ADR 0084).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:ucrtbase_main");
    for name in ["memcpy", "memmove", "memset", "memcmp"] {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
    // setjmp/longjmp: the shared runtime's assembly (ADR 0092), under the
    // names the UCRT lists.
    for name in ["setjmp", "__intrinsic_setjmp", "__intrinsic_setjmpex"] {
        println!("cargo:rustc-link-arg=/EXPORT:{name}=poly_crt_setjmp");
    }
    println!("cargo:rustc-link-arg=/EXPORT:longjmp=poly_crt_longjmp");
}
