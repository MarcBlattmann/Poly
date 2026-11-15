//! `vcruntime140.dll`'s entry point and the exports that are not Rust items of this crate
//! (ADR 0199): `_CxxThrowException` (the shared C++ runtime's assembly), the debugger hooks
//! (assembly here) and the forwarders to the DLLs that hold the function.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:vcruntime140_main");
    println!("cargo:rustc-link-arg=/EXPORT:_CxxThrowException");
    for name in ["_NLG_Dispatch2", "_NLG_Return", "_NLG_Return2", "__NLG_Dispatch2", "__NLG_Return2"] {
        println!("cargo:rustc-link-arg=/EXPORT:{name}=poly_vcrt_nlg");
    }
    for (name, target) in FORWARDS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}={target}");
    }
}

/// Functions another Poly DLL already has, under the same contract (ADR 0199): the C library's
/// memory and string functions, `setjmp`/`longjmp` and the pure-call handler of the Universal
/// C runtime, `ntdll`'s language handler for `__try`, and the `kernel32` functions the
/// `__vcrt_` names wrap.
const FORWARDS: &[(&str, &str)] = &[
    ("memcpy", "ucrtbase.memcpy"),
    ("memmove", "ucrtbase.memmove"),
    ("memset", "ucrtbase.memset"),
    ("memcmp", "ucrtbase.memcmp"),
    ("memchr", "ucrtbase.memchr"),
    ("strchr", "ucrtbase.strchr"),
    ("strrchr", "ucrtbase.strrchr"),
    ("strstr", "ucrtbase.strstr"),
    ("wcschr", "ucrtbase.wcschr"),
    ("wcsrchr", "ucrtbase.wcsrchr"),
    ("wcsstr", "ucrtbase.wcsstr"),
    ("__intrinsic_setjmp", "ucrtbase.__intrinsic_setjmp"),
    ("__intrinsic_setjmpex", "ucrtbase.__intrinsic_setjmpex"),
    ("longjmp", "ucrtbase.longjmp"),
    ("__std_terminate", "ucrtbase.terminate"),
    ("_purecall", "ucrtbase._purecall"),
    ("_set_purecall_handler", "ucrtbase._set_purecall_handler"),
    ("_get_purecall_handler", "ucrtbase._get_purecall_handler"),
    ("__C_specific_handler", "ntdll.__C_specific_handler"),
    ("__C_specific_handler_noexcept", "ntdll.__C_specific_handler"),
    ("__vcrt_GetModuleFileNameW", "kernel32.GetModuleFileNameW"),
    ("__vcrt_GetModuleHandleW", "kernel32.GetModuleHandleW"),
    ("__vcrt_LoadLibraryExW", "kernel32.LoadLibraryExW"),
    ("__vcrt_InitializeCriticalSectionEx", "kernel32.InitializeCriticalSectionEx"),
];
