//! Our `ucrtbase.dll` (ADR 0084): the Universal C Runtime. Programs built
//! by MSVC import it through the `api-ms-win-crt-*` API sets, which our
//! `ntdll`'s loader resolves here (`poly_ldr::apiset`).
//!
//! Most functions are the shared runtime's (`poly-win32-crt`, exported as
//! they are); this crate adds what only the UCRT exports: the
//! `__stdio_common_*` entry points the UCRT headers' inline `printf` and
//! `scanf` call, the `__p_*` accessors and start-up functions of
//! `corecrt_startup.h`, and the handlers' setters.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicI32, Ordering};
use poly_crt::printf::VaList;
use poly_win32_crt::startup::{self, OnExitTable};
use poly_win32_crt::stdio::{self, BufferRule, File};
use poly_win32_crt::{env, global, heap, io, misc, timefn};
use poly_win32_rt as _;

/// `DllMain`.
#[unsafe(no_mangle)]
pub extern "system" fn ucrtbase_main(_module: *mut u8, reason: u32, reserved: *mut u8) -> i32 {
    let r = poly_win32_crt::dll_main(reason, !reserved.is_null());
    if reason == poly_win32_crt::DLL_PROCESS_ATTACH {
        locale::bind();
    }
    r
}

// ----- functions of the shared runtime only the UCRT exports ------------------------------

macro_rules! forward {
    ($($name:ident($($a:ident: $t:ty),*) $(-> $r:ty)? = $path:path;)*) => {$(
        #[unsafe(no_mangle)]
        #[allow(unused_unsafe)]
        pub unsafe extern "C" fn $name($($a: $t),*) $(-> $r)? {
            // SAFETY: the forwarded function's contract, which is this one's.
            unsafe { $path($($a),*) }
        }
    )*};
}

mod locale;

forward! {
    _aligned_msize(p: *mut u8, alignment: usize, offset: usize) -> usize = heap::_aligned_msize;
    _recalloc(p: *mut u8, count: usize, size: usize) -> *mut u8 = heap::_recalloc;
    _set_new_mode(mode: i32) -> i32 = heap::_set_new_mode;
    _query_new_mode() -> i32 = heap::_query_new_mode;
    _byteswap_uint64(v: u64) -> u64 = misc::_byteswap_uint64;
    _byteswap_ulong(v: u32) -> u32 = misc::_byteswap_ulong;
    _byteswap_ushort(v: u16) -> u16 = misc::_byteswap_ushort;
    llabs(v: i64) -> i64 = misc::llabs;
    lldiv(a: i64, b: i64) -> misc::LlDiv = misc::lldiv;
    _get_printf_count_output() -> i32 = misc::_get_printf_count_output;
    _set_printf_count_output(on: i32) -> i32 = misc::_set_printf_count_output;
    _configure_narrow_argv(mode: i32) -> i32 = startup::_configure_narrow_argv;
    _configure_wide_argv(mode: i32) -> i32 = startup::_configure_wide_argv;
    _initialize_narrow_environment() -> i32 = startup::_initialize_narrow_environment;
    _initialize_wide_environment() -> i32 = startup::_initialize_wide_environment;
    _get_initial_narrow_environment() -> *mut *mut u8 = startup::_get_initial_narrow_environment;
    _get_initial_wide_environment() -> *mut *mut u16 = startup::_get_initial_wide_environment;
    _crt_atexit(f: usize) -> i32 = startup::_crt_atexit;
    _crt_at_quick_exit(f: usize) -> i32 = startup::_crt_at_quick_exit;
    _initialize_onexit_table(t: *mut OnExitTable) -> i32 = startup::_initialize_onexit_table;
    _register_onexit_function(t: *mut OnExitTable, f: usize) -> i32 = startup::_register_onexit_function;
    _execute_onexit_table(t: *mut OnExitTable) -> i32 = startup::_execute_onexit_table;
    _register_thread_local_exe_atexit_callback(f: usize) = startup::_register_thread_local_exe_atexit_callback;
    _set_abort_behavior(flags: u32, mask: u32) -> u32 = startup::_set_abort_behavior;
    _dupenv_s(out: *mut *mut u8, len: *mut usize, name: *const u8) -> i32 = env::_dupenv_s;
    _wdupenv_s(out: *mut *mut u16, len: *mut usize, name: *const u16) -> i32 = env::_wdupenv_s;
    _stat64i32(name: *const u8, out: *mut u8) -> i32 = io::_stat64i32;
    _wstat64i32(name: *const u16, out: *mut u8) -> i32 = io::_wstat64i32;
    _fstat64i32(fd: i32, out: *mut u8) -> i32 = io::_fstat64i32;
    _get_daylight(out: *mut i32) -> i32 = timefn::_get_daylight;
    _get_dstbias(out: *mut i32) -> i32 = timefn::_get_dstbias;
    _get_timezone(out: *mut i32) -> i32 = timefn::_get_timezone;
    iswblank(c: u16) -> i32 = poly_win32_crt::strings::iswblank;
    strtof(s: *const u8, end: *mut *mut u8) -> f32 = poly_win32_crt::conv::strtof;
    strtold(s: *const u8, end: *mut *mut u8) -> f64 = poly_win32_crt::conv::strtold;
    wcstof(s: *const u16, end: *mut *mut u16) -> f32 = poly_win32_crt::conv::wcstof;
    strtoll(s: *const u8, end: *mut *mut u8, base: i32) -> i64 = poly_win32_crt::conv::strtoll;
    strtoull(s: *const u8, end: *mut *mut u8, base: i32) -> u64 = poly_win32_crt::conv::strtoull;
    wcstoll(s: *const u16, end: *mut *mut u16, base: i32) -> i64 = poly_win32_crt::conv::wcstoll;
    wcstoull(s: *const u16, end: *mut *mut u16, base: i32) -> u64 = poly_win32_crt::conv::wcstoull;
    atoll(s: *const u8) -> i64 = poly_win32_crt::conv::atoll;
    _wtoll(s: *const u16) -> i64 = poly_win32_crt::conv::_wtoll;
}

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "C" fn _Exit(code: i32) -> ! {
    startup::_Exit(code)
}

#[unsafe(no_mangle)]
pub extern "C" fn quick_exit(code: i32) -> ! {
    startup::quick_exit(code)
}

#[unsafe(no_mangle)]
pub extern "C" fn terminate() -> ! {
    startup::terminate()
}

#[unsafe(no_mangle)]
pub extern "C" fn __std_terminate() -> ! {
    startup::terminate()
}

// ----- corecrt_startup.h ---------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn __p___argc() -> *mut i32 {
    startup::cells().argc
}

#[unsafe(no_mangle)]
pub extern "C" fn __p___argv() -> *mut *mut *mut u8 {
    startup::cells().argv
}

#[unsafe(no_mangle)]
pub extern "C" fn __p___wargv() -> *mut *mut *mut u16 {
    startup::cells().wargv
}

#[unsafe(no_mangle)]
pub extern "C" fn __p__environ() -> *mut *mut *mut u8 {
    startup::cells().environ
}

#[unsafe(no_mangle)]
pub extern "C" fn __p__wenviron() -> *mut *mut *mut u16 {
    startup::cells().wenviron
}

#[unsafe(no_mangle)]
pub extern "C" fn __p__acmdln() -> *mut *mut u8 {
    startup::cells().acmdln
}

#[unsafe(no_mangle)]
pub extern "C" fn __p__wcmdln() -> *mut *mut u16 {
    startup::cells().wcmdln
}

#[unsafe(no_mangle)]
pub extern "C" fn __p__pgmptr() -> *mut *mut u8 {
    startup::cells().pgmptr
}

#[unsafe(no_mangle)]
pub extern "C" fn __p__wpgmptr() -> *mut *mut u16 {
    startup::cells().wpgmptr
}

#[unsafe(no_mangle)]
pub extern "C" fn __p__fmode() -> *mut i32 {
    startup::cells().fmode
}

#[unsafe(no_mangle)]
pub extern "C" fn __p__commode() -> *mut i32 {
    startup::cells().commode
}

/// `_crt_app_type` (`corecrt_startup.h`): kept for `_get_app_type`'s sake.
static APP_TYPE: AtomicI32 = AtomicI32::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn _set_app_type(t: i32) {
    APP_TYPE.store(t, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn _seh_filter_exe(code: u32, _info: *mut u8) -> i32 {
    startup::seh_filter(code)
}

#[unsafe(no_mangle)]
pub extern "C" fn _seh_filter_dll(code: u32, _info: *mut u8) -> i32 {
    startup::seh_filter(code)
}

/// The command line after the program name and the blanks that follow it
/// (Learn, `WinMain`'s `lpCmdLine`).
fn after_program<C: poly_crt::Unit>(line: &[C]) -> usize {
    let mut i = 0;
    let mut quoted = false;
    while i < line.len() {
        let c = line[i].to_u32();
        if c == u32::from(b'"') {
            quoted = !quoted;
        } else if (c == u32::from(b' ') || c == u32::from(b'\t')) && !quoted {
            break;
        }
        i += 1;
    }
    while i < line.len() && (line[i].to_u32() == u32::from(b' ') || line[i].to_u32() == u32::from(b'\t')) {
        i += 1;
    }
    i
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_narrow_winmain_command_line() -> *mut u8 {
    // SAFETY: a valid cell holding the command line.
    let p = unsafe { *startup::cells().acmdln };
    // SAFETY: the command line is a C string.
    let line = unsafe { poly_crt::unit::slice(p) };
    p.wrapping_add(after_program(line))
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_wide_winmain_command_line() -> *mut u16 {
    // SAFETY: a valid cell holding the command line.
    let p = unsafe { *startup::cells().wcmdln };
    // SAFETY: the command line is a C string.
    let line = unsafe { poly_crt::unit::slice(p) };
    p.wrapping_add(after_program(line))
}

// ----- handlers ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn _invalid_parameter_noinfo() {
    global::invalid_parameter();
}

#[unsafe(no_mangle)]
pub extern "C" fn _invalid_parameter_noinfo_noreturn() -> ! {
    global::invalid_parameter();
    // FAST_FAIL_INVALID_ARG (winnt.h).
    poly_win32_rt::fail_fast(5)
}

#[unsafe(no_mangle)]
pub extern "C" fn _set_invalid_parameter_handler(h: usize) -> usize {
    misc::set_invalid_parameter_handler(h)
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_invalid_parameter_handler() -> usize {
    misc::get_invalid_parameter_handler()
}

#[unsafe(no_mangle)]
pub extern "C" fn _set_thread_local_invalid_parameter_handler(h: usize) -> usize {
    misc::set_thread_invalid_parameter_handler(h)
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_thread_local_invalid_parameter_handler() -> usize {
    misc::get_thread_invalid_parameter_handler()
}

#[unsafe(no_mangle)]
pub extern "C" fn set_terminate(h: usize) -> usize {
    misc::set_terminate(h)
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_terminate() -> usize {
    startup::TERMINATE.load(Ordering::Acquire)
}

#[unsafe(no_mangle)]
pub extern "C" fn _set_purecall_handler(h: usize) -> usize {
    misc::set_purecall_handler(h)
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_purecall_handler() -> usize {
    startup::PURECALL.load(Ordering::Acquire)
}

#[unsafe(no_mangle)]
pub extern "C" fn _set_new_handler(h: usize) -> usize {
    heap::set_new_handler(h)
}

// ----- stdio ------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn __acrt_iob_func(i: u32) -> *mut File {
    stdio::iob(i as usize)
}

/// # Safety
/// The UCRT headers' contract: `format` and its `va_list`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfprintf(
    options: u64,
    f: *mut File,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vfprintf(f, format, &mut VaList(args), options) }
}

/// # Safety
/// As `__stdio_common_vfprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfprintf_s(
    options: u64,
    f: *mut File,
    format: *const u8,
    locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { __stdio_common_vfprintf(options, f, format, locale, args) }
}

/// # Safety
/// As `__stdio_common_vfprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfwprintf(
    options: u64,
    f: *mut File,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vfprintf(f, format, &mut VaList(args), options) }
}

/// # Safety
/// As `__stdio_common_vfprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfwprintf_s(
    options: u64,
    f: *mut File,
    format: *const u16,
    locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { __stdio_common_vfwprintf(options, f, format, locale, args) }
}

/// # Safety
/// `buf` holds `count` units (or is NULL with 0); as above otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsprintf(
    options: u64,
    buf: *mut u8,
    count: usize,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsprintf(buf, count, format, &mut VaList(args), options, BufferRule::Options) }
}

/// # Safety
/// As `__stdio_common_vsprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vswprintf(
    options: u64,
    buf: *mut u16,
    count: usize,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsprintf(buf, count, format, &mut VaList(args), options, BufferRule::Options) }
}

/// # Safety
/// As `__stdio_common_vsprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsprintf_s(
    options: u64,
    buf: *mut u8,
    count: usize,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsprintf(buf, count, format, &mut VaList(args), options, BufferRule::Secure) }
}

/// # Safety
/// As `__stdio_common_vsprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vswprintf_s(
    options: u64,
    buf: *mut u16,
    count: usize,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsprintf(buf, count, format, &mut VaList(args), options, BufferRule::Secure) }
}

/// # Safety
/// As `__stdio_common_vsprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsnprintf_s(
    options: u64,
    buf: *mut u8,
    count: usize,
    max: usize,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsprintf(buf, count, format, &mut VaList(args), options, BufferRule::SecureCount(max)) }
}

/// # Safety
/// As `__stdio_common_vsprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsnwprintf_s(
    options: u64,
    buf: *mut u16,
    count: usize,
    max: usize,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsprintf(buf, count, format, &mut VaList(args), options, BufferRule::SecureCount(max)) }
}

/// # Safety
/// `buf` holds `count` units or a NUL before; `format` and its pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsscanf(
    options: u64,
    buf: *const u8,
    count: usize,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsscanf(buf, count, format, &mut VaList(args), options) }
}

/// # Safety
/// As `__stdio_common_vsscanf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vswscanf(
    options: u64,
    buf: *const u16,
    count: usize,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsscanf(buf, count, format, &mut VaList(args), options) }
}

/// # Safety
/// `format` and its pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfscanf(
    options: u64,
    f: *mut File,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vfscanf(f, format, &mut VaList(args), options) }
}

/// # Safety
/// As `__stdio_common_vfscanf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfwscanf(
    options: u64,
    f: *mut File,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vfscanf(f, format, &mut VaList(args), options) }
}

// ----- the rest -----------------------------------------------------------------------------

/// The `strerror` texts as `char *[]` (`__sys_errlist`).
#[unsafe(no_mangle)]
pub extern "C" fn __sys_errlist() -> *mut *mut u8 {
    static LIST: poly_win32_crt::global::Global<[usize; 43]> = poly_win32_crt::global::Global::new([0; 43]);
    // SAFETY: filled on first use; a race writes the same pointers.
    unsafe {
        let l = &mut *LIST.get();
        if l[0] == 0 {
            for (i, slot) in l.iter_mut().enumerate() {
                *slot = poly_win32_crt::strings::strerror(i as i32) as usize;
            }
        }
        l.as_mut_ptr() as *mut *mut u8
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_nerr() -> *mut i32 {
    static N: poly_win32_crt::global::Global<i32> = poly_win32_crt::global::Global::new(43);
    N.get()
}

#[unsafe(no_mangle)]
pub extern "C" fn __timezone() -> *mut i32 {
    static V: poly_win32_crt::global::Global<i32> = poly_win32_crt::global::Global::new(timefn::TIMEZONE);
    V.get()
}

#[unsafe(no_mangle)]
pub extern "C" fn __tzname() -> *mut *const u8 {
    static V: poly_win32_crt::global::Global<[*const u8; 2]> =
        poly_win32_crt::global::Global::new([c"UTC".as_ptr() as *const u8, c"UTC".as_ptr() as *const u8]);
    V.get() as *mut *const u8
}

#[unsafe(no_mangle)]
pub extern "C" fn __initialize_lconv_for_unsigned_char() -> i32 {
    0
}

/// # Safety
/// As `memcpy_s`, in units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wmemcpy_s(dst: *mut u16, size: usize, src: *const u16, n: usize) -> i32 {
    // SAFETY: the caller's buffers, in bytes.
    unsafe {
        poly_win32_crt::strings::memcpy_s(dst as *mut u8, size.saturating_mul(2), src as *const u8, n.saturating_mul(2))
    }
}

/// The UCRT's `wcstok` takes a context (C11); NULL uses the thread's.
///
/// # Safety
/// As `wcstok_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstok(text: *mut u16, delims: *const u16, context: *mut *mut u16) -> *mut u16 {
    if context.is_null() {
        // SAFETY: the caller's strings.
        unsafe { poly_win32_crt::strings::wcstok_thread(text, delims) }
    } else {
        // SAFETY: the caller's arguments.
        unsafe { poly_win32_crt::strings::wcstok_s(text, delims, context) }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn round(x: f64) -> f64 {
    poly_crt::math::round(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn trunc(x: f64) -> f64 {
    poly_crt::math::trunc(x)
}

/// `_dclass`: `FP_ZERO`, `FP_SUBNORMAL`, `FP_NORMAL`, `FP_INFINITE`,
/// `FP_NAN` (`math.h`).
#[unsafe(no_mangle)]
pub extern "C" fn _dclass(x: f64) -> i16 {
    poly_crt::math::class(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn _dsign(x: f64) -> i32 {
    i32::from(x.is_sign_negative())
}

mod math_p;
