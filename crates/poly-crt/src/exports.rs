//! What `ucrtbase.dll` and `msvcrt.dll` export beyond their Rust items
//! (ADR 0084): the variadic functions of `msvcrt.dll`, which are assembly
//! thunks (they spill the register arguments next to the stack ones and
//! call the `va_list` form, as the x64 calling convention allows: Learn,
//! "x64 calling convention", "Varargs"), and the memory functions both
//! take from `compiler_builtins`. The DLLs' `build.rs` turn these into
//! `/EXPORT:` lines; `cargo xtask win32` checks every export of each DLL is
//! a name the coverage map lists for it.

/// `msvcrt.dll`'s variadic functions: `(name, fixed arguments)`.
pub const MSVCRT_VARARGS: &[(&str, u8)] = &[
    ("printf", 1),
    ("wprintf", 1),
    ("printf_s", 1),
    ("wprintf_s", 1),
    ("fprintf", 2),
    ("fwprintf", 2),
    ("fprintf_s", 2),
    ("fwprintf_s", 2),
    ("sprintf", 2),
    ("swprintf", 2),
    ("_swprintf", 2),
    ("_scprintf", 1),
    ("_scwprintf", 1),
    ("_snprintf", 3),
    ("_snwprintf", 3),
    ("sprintf_s", 3),
    ("swprintf_s", 3),
    ("_snprintf_s", 4),
    ("_snwprintf_s", 4),
    ("scanf", 1),
    ("wscanf", 1),
    ("scanf_s", 1),
    ("wscanf_s", 1),
    ("sscanf", 2),
    ("swscanf", 2),
    ("sscanf_s", 2),
    ("swscanf_s", 2),
    ("fscanf", 2),
    ("fwscanf", 2),
    ("fscanf_s", 2),
    ("fwscanf_s", 2),
    ("_snscanf", 3),
    ("_snwscanf", 3),
];

/// The memory functions both DLLs export from `compiler_builtins`.
pub const BUILTIN_MEMORY: &[&str] = &["memcpy", "memmove", "memset", "memcmp"];

/// Functions every MSVC-built console program's start-up code imports from
/// the UCRT (Learn, "CRT initialization"; the `corecrt_startup.h` entry
/// points): `cargo xtask win32` checks `ucrtbase.dll` exports them all.
pub const UCRT_STARTUP: &[&str] = &[
    "_initterm",
    "_initterm_e",
    "_get_initial_narrow_environment",
    "_get_initial_wide_environment",
    "_initialize_narrow_environment",
    "_initialize_wide_environment",
    "_configure_narrow_argv",
    "_configure_wide_argv",
    "__p___argc",
    "__p___argv",
    "__p___wargv",
    "__p__environ",
    "__p__wenviron",
    "__p__commode",
    "__p__fmode",
    "_set_fmode",
    "_set_app_type",
    "_crt_atexit",
    "_register_onexit_function",
    "_initialize_onexit_table",
    "_execute_onexit_table",
    "_register_thread_local_exe_atexit_callback",
    "_seh_filter_exe",
    "_set_new_mode",
    "_configthreadlocale",
    "__setusermatherr",
    "_cexit",
    "_c_exit",
    "exit",
    "_exit",
    "terminate",
    "_invalid_parameter_noinfo",
    "_invalid_parameter_noinfo_noreturn",
    "__acrt_iob_func",
    "__stdio_common_vfprintf",
    "__stdio_common_vsprintf",
    "__stdio_common_vswprintf",
    "malloc",
    "free",
    "_callnewh",
];

/// What every program built against `msvcrt.dll` imports at start-up.
pub const MSVCRT_STARTUP: &[&str] = &[
    "__getmainargs",
    "__wgetmainargs",
    "__set_app_type",
    "_initterm",
    "_XcptFilter",
    "_amsg_exit",
    "_cexit",
    "exit",
    "_exit",
    "_onexit",
    "__dllonexit",
    "_iob",
    "__iob_func",
    "_fmode",
    "_commode",
    "_acmdln",
    "_wcmdln",
    "printf",
    "fprintf",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varargs_are_unique_and_shaped() {
        for (i, (a, n)) in MSVCRT_VARARGS.iter().enumerate() {
            assert!((1..=4).contains(n), "{a}");
            assert!(MSVCRT_VARARGS[i + 1..].iter().all(|(b, _)| a != b), "{a} twice");
        }
        assert!(UCRT_STARTUP.contains(&"_initterm") && MSVCRT_STARTUP.contains(&"__getmainargs"));
        assert_eq!(BUILTIN_MEMORY.len(), 4);
    }
}
