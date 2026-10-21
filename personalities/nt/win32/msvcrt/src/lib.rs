//! Our `msvcrt.dll` (ADR 0084): the classic system C runtime that programs
//! built with older compilers (and the Windows driver kits) link against.
//! It shares `poly-win32-crt` with `ucrtbase.dll`; this crate adds what is
//! `msvcrt`'s own:
//!
//! - its exported data (`_iob`, `__argc`, `__argv`, `_environ`, `_fmode`,
//!   `_acmdln`, `_pctype` …), which the shared runtime uses through
//!   `startup::bind` and `stdio::bind_iob`;
//! - the classic start-up interface (`__getmainargs`, `__set_app_type`,
//!   `_XcptFilter`, `_onexit`, `__dllonexit`, `_amsg_exit`);
//! - the formatted I/O functions, which keep the classic behaviour
//!   (`poly_crt::printf::MSVCRT_OPTIONS`: three-digit exponents, `1.#INF`,
//!   wide `%s` in wide functions) — the variadic ones through assembly
//!   thunks that hand the arguments on as a `va_list`;
//! - `time` and its family as 64-bit-`time_t` functions, the two-argument
//!   `wcstok`, and the C++-named `terminate` / `set_terminate`.

#![no_std]
#![allow(clippy::missing_safety_doc)]
#![allow(non_upper_case_globals)]

use core::sync::atomic::{AtomicI32, Ordering};
use poly_crt::printf::{self, MSVCRT_OPTIONS, VaList};
use poly_crt::scanf;
use poly_win32_crt::global::{self, Global};
use poly_win32_crt::startup::{self, Cells};
use poly_win32_crt::stdio::{self, BufferRule, File};
use poly_win32_crt::{heap, timefn};
use poly_win32_rt as _;

// ----- exported data --------------------------------------------------------------------

/// `_iob[_IOB_ENTRIES]` (20 in the classic `stdio.h`).
#[unsafe(no_mangle)]
pub static _iob: Global<[File; 20]> = Global::new([const { File::new(-1) }; 20]);
#[unsafe(no_mangle)]
pub static __argc: Global<i32> = Global::new(0);
#[unsafe(no_mangle)]
pub static __argv: Global<*mut *mut u8> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static __wargv: Global<*mut *mut u16> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static _environ: Global<*mut *mut u8> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static _wenviron: Global<*mut *mut u16> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static __initenv: Global<*mut *mut u8> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static __winitenv: Global<*mut *mut u16> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static _acmdln: Global<*mut u8> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static _wcmdln: Global<*mut u16> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static _pgmptr: Global<*mut u8> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static _wpgmptr: Global<*mut u16> = Global::new(core::ptr::null_mut());
#[unsafe(no_mangle)]
pub static _fmode: Global<i32> = Global::new(0);
#[unsafe(no_mangle)]
pub static _commode: Global<i32> = Global::new(0);
/// `_ctype[257]`: the "C" locale's classes from `EOF` on.
#[unsafe(no_mangle)]
pub static _ctype: [u16; 257] = poly_crt::ctype::NARROW_TABLE;
/// `_pctype`: `_ctype + 1`.
#[unsafe(no_mangle)]
pub static _pctype: Global<*const u16> = Global::new(_ctype.as_ptr().wrapping_add(1));
#[unsafe(no_mangle)]
pub static __mb_cur_max: Global<i32> = Global::new(1);
#[unsafe(no_mangle)]
pub static _timezone: Global<i32> = Global::new(timefn::TIMEZONE);
#[unsafe(no_mangle)]
pub static _daylight: Global<i32> = Global::new(timefn::DAYLIGHT);
#[unsafe(no_mangle)]
pub static _dstbias: Global<i32> = Global::new(timefn::DSTBIAS);
#[unsafe(no_mangle)]
pub static _tzname: Global<[*const u8; 2]> = Global::new([c"UTC".as_ptr() as *const u8, c"UTC".as_ptr() as *const u8]);
/// `_sys_errlist[]` (filled at start-up) and `_sys_nerr`.
#[unsafe(no_mangle)]
pub static _sys_errlist: Global<[*const u8; 43]> = Global::new([core::ptr::null(); 43]);
#[unsafe(no_mangle)]
pub static _sys_nerr: Global<i32> = Global::new(43);

/// `DllMain`: bind the exported data as the runtime's variables, then start
/// the runtime.
#[unsafe(no_mangle)]
pub extern "system" fn msvcrt_main(_module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    if reason == poly_win32_crt::DLL_PROCESS_ATTACH {
        // SAFETY: process attach, before anything reads the cells.
        unsafe { *_fmode.get() = poly_crt::fmode::O_TEXT };
        startup::bind(Cells {
            argc: __argc.get(),
            argv: __argv.get(),
            wargv: __wargv.get(),
            environ: _environ.get(),
            wenviron: _wenviron.get(),
            initenv: __initenv.get(),
            winitenv: __winitenv.get(),
            acmdln: _acmdln.get(),
            wcmdln: _wcmdln.get(),
            pgmptr: _pgmptr.get(),
            wpgmptr: _wpgmptr.get(),
            fmode: _fmode.get(),
            commode: _commode.get(),
        });
        stdio::bind_iob(_iob.get() as *mut File);
        // SAFETY: process attach; one thread.
        let list = unsafe { &mut *_sys_errlist.get() };
        for (i, slot) in list.iter_mut().enumerate() {
            *slot = poly_win32_crt::strings::strerror(i as i32);
        }
    }
    poly_win32_crt::dll_main(reason)
}

// ----- the classic start-up interface ------------------------------------------------------

/// `_startupinfo` (`internal.h` of the classic CRT's public headers):
/// `int newmode`.
#[repr(C)]
pub struct StartupInfo {
    newmode: i32,
}

/// # Safety
/// The out-pointers are writable; `info` NULL or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __getmainargs(
    argc: *mut i32,
    argv: *mut *mut *mut u8,
    env: *mut *mut *mut u8,
    _wildcard: i32,
    info: *const StartupInfo,
) -> i32 {
    if !info.is_null() {
        // SAFETY: the caller's structure.
        heap::_set_new_mode(unsafe { (*info).newmode });
    }
    // SAFETY: the caller's out-pointers.
    unsafe { startup::main_args(argc, argv, env, false) }
}

/// # Safety
/// As `__getmainargs`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __wgetmainargs(
    argc: *mut i32,
    argv: *mut *mut *mut u16,
    env: *mut *mut *mut u16,
    _wildcard: i32,
    info: *const StartupInfo,
) -> i32 {
    if !info.is_null() {
        // SAFETY: the caller's structure.
        heap::_set_new_mode(unsafe { (*info).newmode });
    }
    // SAFETY: the caller's out-pointers.
    unsafe { startup::main_args(argc, argv as *mut *mut *mut u8, env as *mut *mut *mut u8, true) }
}

static APP_TYPE: AtomicI32 = AtomicI32::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn __set_app_type(t: i32) {
    APP_TYPE.store(t, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn __iob_func() -> *mut File {
    _iob.get() as *mut File
}

#[unsafe(no_mangle)]
pub extern "C" fn _XcptFilter(code: u32, _info: *mut u8) -> i32 {
    startup::seh_filter(code)
}

/// `_onexit`: the function, or NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn _onexit(f: usize) -> usize {
    if startup::atexit(f) == 0 { f } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn atexit(f: usize) -> i32 {
    startup::atexit(f)
}

/// `__dllonexit(f, &begin, &end)`: append `f` to a DLL's own table (a heap
/// block of this runtime), growing it; `f`, or NULL on failure.
///
/// # Safety
/// `begin`/`end` delimit a table from this heap (or NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __dllonexit(f: usize, begin: *mut *mut usize, end: *mut *mut usize) -> usize {
    if begin.is_null() || end.is_null() {
        return 0;
    }
    // SAFETY: the caller's table.
    unsafe {
        let (b, e) = (*begin, *end);
        let len = if b.is_null() { 0 } else { e.offset_from(b) as usize };
        let cap = if b.is_null() { 0 } else { heap::_msize(b as *mut u8) / 8 };
        let b = if len + 1 > cap {
            let n = heap::realloc(b as *mut u8, (len + 1).max(cap * 2).max(8) * 8) as *mut usize;
            if n.is_null() {
                return 0;
            }
            n
        } else {
            b
        };
        b.add(len).write(f);
        *begin = b;
        *end = b.add(len + 1);
    }
    f
}

/// `_amsg_exit`: a runtime error; the process ends with code 255.
#[unsafe(no_mangle)]
pub extern "C" fn _amsg_exit(_code: i32) -> ! {
    startup::_exit(255)
}

/// `_invalid_parameter`: the handler with the given details.
///
/// # Safety
/// The strings are NULL or valid wide C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _invalid_parameter(
    expr: *const u16,
    func: *const u16,
    file: *const u16,
    line: u32,
    reserved: usize,
) {
    let h = poly_win32_crt::misc::get_invalid_parameter_handler();
    if h == 0 {
        poly_win32_rt::fail_fast(5);
    }
    // SAFETY: a handler with the documented signature.
    let f: global::InvalidParameterHandler = unsafe { core::mem::transmute(h) };
    // SAFETY: as above.
    unsafe { f(expr, func, file, line, reserved) };
}

#[unsafe(export_name = "?terminate@@YAXXZ")]
pub extern "C" fn cxx_terminate() -> ! {
    startup::terminate()
}

#[unsafe(export_name = "?set_terminate@@YAP6AXXZP6AXXZ@Z")]
pub extern "C" fn cxx_set_terminate(h: usize) -> usize {
    poly_win32_crt::misc::set_terminate(h)
}

#[unsafe(export_name = "?_set_new_handler@@YAP6AH_K@ZP6AH0@Z@Z")]
pub extern "C" fn cxx_set_new_handler(h: usize) -> usize {
    heap::set_new_handler(h)
}

#[unsafe(export_name = "?_set_new_mode@@YAHH@Z")]
pub extern "C" fn cxx_set_new_mode(mode: i32) -> i32 {
    heap::_set_new_mode(mode)
}

// ----- time with a 64-bit time_t ----------------------------------------------------------

/// # Safety
/// As `_time64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time(out: *mut i64) -> i64 {
    // SAFETY: the caller's pointer.
    unsafe { timefn::_time64(out) }
}

/// # Safety
/// As `_gmtime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gmtime(t: *const i64) -> *mut poly_crt::time::Tm {
    // SAFETY: the caller's pointer.
    unsafe { timefn::_gmtime64(t) }
}

/// # Safety
/// As `_localtime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn localtime(t: *const i64) -> *mut poly_crt::time::Tm {
    // SAFETY: the caller's pointer.
    unsafe { timefn::_localtime64(t) }
}

/// # Safety
/// As `_mktime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mktime(tm: *mut poly_crt::time::Tm) -> i64 {
    // SAFETY: the caller's structure.
    unsafe { timefn::_mktime64(tm) }
}

/// # Safety
/// As `_ctime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ctime(t: *const i64) -> *mut u8 {
    // SAFETY: the caller's pointer.
    unsafe { timefn::_ctime64(t) }
}

#[unsafe(no_mangle)]
pub extern "C" fn difftime(a: i64, b: i64) -> f64 {
    timefn::_difftime64(a, b)
}

/// The classic two-argument `wcstok` (the thread's context).
///
/// # Safety
/// As `strtok`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstok(text: *mut u16, delims: *const u16) -> *mut u16 {
    // SAFETY: the caller's strings.
    unsafe { poly_win32_crt::strings::wcstok_thread(text, delims) }
}

// ----- formatted I/O ------------------------------------------------------------------------

/// What every classic formatted output function passes.
const OPT: u64 = MSVCRT_OPTIONS;
/// The classic `scanf` options (wide `%s` in wide functions).
const SCAN: u64 = scanf::LEGACY_WIDE_SPECIFIERS | scanf::LEGACY_MSVCRT_COMPATIBILITY;
/// No limit (`sprintf`).
const UNBOUNDED: usize = i32::MAX as usize;
/// `_snprintf`'s rule.
const LEGACY_N: u64 = OPT | printf::LEGACY_VSPRINTF_NULL_TERMINATION;

fn stdout() -> *mut File {
    stdio::iob(1)
}

fn stdin() -> *mut File {
    stdio::iob(0)
}

macro_rules! va_fns {
    ($($name:ident($($a:ident: $t:ty),*) -> $r:ty = $body:expr;)*) => {$(
        /// # Safety
        /// The C contract of the function of the same name: the format and
        /// its arguments (`va`), the buffers' sizes.
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name($($a: $t,)* va: *const u64) -> $r {
            let mut va = VaList(va);
            let _ = &mut va;
            // SAFETY: as documented above.
            unsafe { $body(&mut va) }
        }
    )*};
}

/// The `va_list` forms the thunks call (and the `v*` exports wrap).
pub mod va {
    use super::*;

    va_fns! {
        printf(f: *const u8) -> i32 = |va: &mut VaList| stdio::vfprintf(stdout(), f, va, OPT);
        wprintf(f: *const u16) -> i32 = |va: &mut VaList| stdio::vfprintf(stdout(), f, va, OPT);
        fprintf(s: *mut File, f: *const u8) -> i32 = |va: &mut VaList| stdio::vfprintf(s, f, va, OPT);
        fwprintf(s: *mut File, f: *const u16) -> i32 = |va: &mut VaList| stdio::vfprintf(s, f, va, OPT);
        sprintf(b: *mut u8, f: *const u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, UNBOUNDED, f, va, LEGACY_N, BufferRule::Options);
        swprintf(b: *mut u16, f: *const u16) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, UNBOUNDED, f, va, LEGACY_N, BufferRule::Options);
        scprintf(f: *const u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(core::ptr::null_mut::<u8>(), 0, f, va, LEGACY_N, BufferRule::Options);
        scwprintf(f: *const u16) -> i32 =
            |va: &mut VaList| stdio::vsprintf(core::ptr::null_mut::<u16>(), 0, f, va, LEGACY_N, BufferRule::Options);
        snprintf(b: *mut u8, n: usize, f: *const u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, LEGACY_N, BufferRule::Options);
        snwprintf(b: *mut u16, n: usize, f: *const u16) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, LEGACY_N, BufferRule::Options);
        sprintf_s(b: *mut u8, n: usize, f: *const u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, OPT, BufferRule::Secure);
        swprintf_s(b: *mut u16, n: usize, f: *const u16) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, OPT, BufferRule::Secure);
        snprintf_s(b: *mut u8, n: usize, c: usize, f: *const u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, OPT, BufferRule::SecureCount(c));
        snwprintf_s(b: *mut u16, n: usize, c: usize, f: *const u16) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, OPT, BufferRule::SecureCount(c));
        scanf(f: *const u8) -> i32 = |va: &mut VaList| stdio::vfscanf(stdin(), f, va, SCAN);
        wscanf(f: *const u16) -> i32 = |va: &mut VaList| stdio::vfscanf(stdin(), f, va, SCAN);
        scanf_s(f: *const u8) -> i32 = |va: &mut VaList| stdio::vfscanf(stdin(), f, va, SCAN | scanf::SECURECRT);
        wscanf_s(f: *const u16) -> i32 = |va: &mut VaList| stdio::vfscanf(stdin(), f, va, SCAN | scanf::SECURECRT);
        sscanf(s: *const u8, f: *const u8) -> i32 = |va: &mut VaList| stdio::vsscanf(s, usize::MAX, f, va, SCAN);
        swscanf(s: *const u16, f: *const u16) -> i32 = |va: &mut VaList| stdio::vsscanf(s, usize::MAX, f, va, SCAN);
        sscanf_s(s: *const u8, f: *const u8) -> i32 =
            |va: &mut VaList| stdio::vsscanf(s, usize::MAX, f, va, SCAN | scanf::SECURECRT);
        swscanf_s(s: *const u16, f: *const u16) -> i32 =
            |va: &mut VaList| stdio::vsscanf(s, usize::MAX, f, va, SCAN | scanf::SECURECRT);
        snscanf(s: *const u8, n: usize, f: *const u8) -> i32 = |va: &mut VaList| stdio::vsscanf(s, n, f, va, SCAN);
        snwscanf(s: *const u16, n: usize, f: *const u16) -> i32 = |va: &mut VaList| stdio::vsscanf(s, n, f, va, SCAN);
        fscanf(st: *mut File, f: *const u8) -> i32 = |va: &mut VaList| stdio::vfscanf(st, f, va, SCAN);
        fwscanf(st: *mut File, f: *const u16) -> i32 = |va: &mut VaList| stdio::vfscanf(st, f, va, SCAN);
        fscanf_s(st: *mut File, f: *const u8) -> i32 =
            |va: &mut VaList| stdio::vfscanf(st, f, va, SCAN | scanf::SECURECRT);
        fwscanf_s(st: *mut File, f: *const u16) -> i32 =
            |va: &mut VaList| stdio::vfscanf(st, f, va, SCAN | scanf::SECURECRT);
    }
}

/// An exported variadic function: spill the register arguments into their
/// home slots (so all arguments form one array), point a `va_list` at the
/// first variadic one, and call the `va_list` form with it as the argument
/// after the fixed ones (Learn, "x64 calling convention": "the caller
/// reserves ... space for the four register parameters"; varargs are
/// passed as for an unprototyped function). The thunk has unwind data.
macro_rules! thunk {
    ($name:literal, 1, $t:path) => {
        thunk!(@ $name, "72", "mov rdx, rax", $t);
    };
    ($name:literal, 2, $t:path) => {
        thunk!(@ $name, "80", "mov r8, rax", $t);
    };
    ($name:literal, 3, $t:path) => {
        thunk!(@ $name, "88", "mov r9, rax", $t);
    };
    ($name:literal, 4, $t:path) => {
        thunk!(@ $name, "96", "mov qword ptr [rsp + 32], rax", $t);
    };
    (@ $name:literal, $off:literal, $store:literal, $t:path) => {
        core::arch::global_asm!(
            ".text",
            concat!(".globl ", $name),
            ".p2align 4",
            concat!(".seh_proc ", $name),
            concat!($name, ":"),
            "mov qword ptr [rsp + 8], rcx",
            "mov qword ptr [rsp + 16], rdx",
            "mov qword ptr [rsp + 24], r8",
            "mov qword ptr [rsp + 32], r9",
            "sub rsp, 56",
            ".seh_stackalloc 56",
            ".seh_endprologue",
            concat!("lea rax, [rsp + ", $off, "]"),
            $store,
            "call {t}",
            "add rsp, 56",
            "ret",
            ".seh_endproc",
            t = sym $t,
        );
    };
}

thunk!("printf", 1, va::printf);
thunk!("wprintf", 1, va::wprintf);
thunk!("printf_s", 1, va::printf);
thunk!("wprintf_s", 1, va::wprintf);
thunk!("fprintf", 2, va::fprintf);
thunk!("fwprintf", 2, va::fwprintf);
thunk!("fprintf_s", 2, va::fprintf);
thunk!("fwprintf_s", 2, va::fwprintf);
thunk!("sprintf", 2, va::sprintf);
thunk!("swprintf", 2, va::swprintf);
thunk!("_swprintf", 2, va::swprintf);
thunk!("_scprintf", 1, va::scprintf);
thunk!("_scwprintf", 1, va::scwprintf);
thunk!("_snprintf", 3, va::snprintf);
thunk!("_snwprintf", 3, va::snwprintf);
thunk!("sprintf_s", 3, va::sprintf_s);
thunk!("swprintf_s", 3, va::swprintf_s);
thunk!("_snprintf_s", 4, va::snprintf_s);
thunk!("_snwprintf_s", 4, va::snwprintf_s);
thunk!("scanf", 1, va::scanf);
thunk!("wscanf", 1, va::wscanf);
thunk!("scanf_s", 1, va::scanf_s);
thunk!("wscanf_s", 1, va::wscanf_s);
thunk!("sscanf", 2, va::sscanf);
thunk!("swscanf", 2, va::swscanf);
thunk!("sscanf_s", 2, va::sscanf_s);
thunk!("swscanf_s", 2, va::swscanf_s);
thunk!("fscanf", 2, va::fscanf);
thunk!("fwscanf", 2, va::fwscanf);
thunk!("fscanf_s", 2, va::fscanf_s);
thunk!("fwscanf_s", 2, va::fwscanf_s);
thunk!("_snscanf", 3, va::snscanf);
thunk!("_snwscanf", 3, va::snwscanf);

// The `va_list` exports.

macro_rules! v_exports {
    ($($name:ident = $target:path, ($($a:ident: $t:ty),*);)*) => {$(
        /// # Safety
        /// As the function's C contract.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name($($a: $t,)* va: *const u64) -> i32 {
            // SAFETY: the caller's arguments.
            unsafe { $target($($a,)* va) }
        }
    )*};
}

v_exports! {
    vprintf = va::printf, (f: *const u8);
    vwprintf = va::wprintf, (f: *const u16);
    vprintf_s = va::printf, (f: *const u8);
    vwprintf_s = va::wprintf, (f: *const u16);
    vfprintf = va::fprintf, (s: *mut File, f: *const u8);
    vfwprintf = va::fwprintf, (s: *mut File, f: *const u16);
    vfprintf_s = va::fprintf, (s: *mut File, f: *const u8);
    vfwprintf_s = va::fwprintf, (s: *mut File, f: *const u16);
    vsprintf = va::sprintf, (b: *mut u8, f: *const u8);
    vswprintf = va::swprintf, (b: *mut u16, f: *const u16);
    _vswprintf = va::swprintf, (b: *mut u16, f: *const u16);
    _vscprintf = va::scprintf, (f: *const u8);
    _vscwprintf = va::scwprintf, (f: *const u16);
    _vsnprintf = va::snprintf, (b: *mut u8, n: usize, f: *const u8);
    _vsnwprintf = va::snwprintf, (b: *mut u16, n: usize, f: *const u16);
    vsprintf_s = va::sprintf_s, (b: *mut u8, n: usize, f: *const u8);
    vswprintf_s = va::swprintf_s, (b: *mut u16, n: usize, f: *const u16);
    _vsnprintf_s = va::snprintf_s, (b: *mut u8, n: usize, c: usize, f: *const u8);
    _vsnwprintf_s = va::snwprintf_s, (b: *mut u16, n: usize, c: usize, f: *const u16);
}
