//! `ucrtbase.dll`'s entry point (`DllMain`, no C runtime below it), the
//! memory functions it exports from the compiler's own `compiler_builtins`
//! (ADR 0084) and the shared runtime's variadic thunks (ADR 0093).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    for (name, target) in APPS_FORWARDS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}={target}");
    }
    // The "optimized" `_o_` exports are the function of the name without the prefix.
    for name in APPS_ALIASES {
        println!("cargo:rustc-link-arg=/EXPORT:{name}={}", &name[3..]);
    }
    // END generated
    println!("cargo:rerun-if-changed=../../../../crates/poly-crt/src/exports.rs");
    println!("cargo:rustc-link-arg=/ENTRY:ucrtbase_main");
    for name in poly_crt::exports::BUILTIN_MEMORY {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
    for (name, _) in poly_crt::exports::SHARED_VARARGS {
        println!("cargo:rustc-link-arg=/EXPORT:{name}");
    }
    // setjmp/longjmp: the shared runtime's assembly (ADR 0092), under the
    // names the UCRT lists.
    for name in ["setjmp", "__intrinsic_setjmp", "__intrinsic_setjmpex"] {
        println!("cargo:rustc-link-arg=/EXPORT:{name}=poly_crt_setjmp");
    }
    println!("cargo:rustc-link-arg=/EXPORT:longjmp=poly_crt_longjmp");
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_FORWARDS: &[(&str, &str)] = &[
    ("__C_specific_handler", "ntdll.__C_specific_handler"),
];
const APPS_ALIASES: &[&str] = &[
    "_o____lc_codepage_func",
    "_o____lc_collate_cp_func",
    "_o____lc_locale_name_func",
    "_o____mb_cur_max_func",
    "_o___pctype_func",
    "_o___stdio_common_vswprintf",
    "_o___stdio_common_vswprintf_s",
    "_o___stdio_common_vswscanf",
    "_o__callnewh",
    "_o__cexit",
    "_o__configure_narrow_argv",
    "_o__create_locale",
    "_o__crt_atexit",
    "_o__dsign",
    "_o__errno",
    "_o__execute_onexit_table",
    "_o__initialize_narrow_environment",
    "_o__initialize_onexit_table",
    "_o__invalid_parameter_noinfo",
    "_o__localtime64_s",
    "_o__purecall",
    "_o__register_onexit_function",
    "_o__seh_filter_dll",
    "_o__wcsdup",
    "_o__wcsicmp_l",
    "_o_abort",
    "_o_calloc",
    "_o_ceilf",
    "_o_free",
    "_o_iswspace",
    "_o_localeconv",
    "_o_malloc",
    "_o_realloc",
    "_o_setlocale",
    "_o_terminate",
    "_o_towlower",
    "_o_towupper",
    "_o_wcstok_s",
    "_o_wcstoul",
    "_o_wcstoull",
];
// END generated consts
