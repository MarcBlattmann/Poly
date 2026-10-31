//! `crttest.exe` (ADR 0084): the program the kernel's ring-3 test
//! `nt_c_runtime_program_runs_on_ucrtbase` runs. It imports the C runtime
//! the way an MSVC-built program does — through the `api-ms-win-crt-*`
//! API-set names, which our loader resolves to `ucrtbase.dll` — and does
//! itself what MSVC's start-up code does (`_set_app_type`,
//! `_configure_narrow_argv`, `_initialize_narrow_environment`, the
//! `_initterm` tables, `_crt_atexit`), then checks the runtime: the heap,
//! strings, `printf` and `scanf` formatting, conversions, files and
//! descriptors, the environment, time, sorting, locales, signals, threads,
//! the invalid parameter handler, and `msvcrt.dll` (loaded with
//! `LoadLibraryW`: its variadic functions, data and start-up interface).
//!
//! It ends with `exit(PASS)`, leaving one file unclosed so the kernel test
//! sees that `exit` wrote it; `atexit` functions check their order. A
//! failed check prints `crttest: step N failed` and exits with `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

mod mathtest;

/// The exit status when every check passed.
const PASS: i32 = 0x43;

type File = u8;

#[link(name = "api-ms-win-crt-runtime-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn _initterm(begin: *const usize, end: *const usize);
    fn _initterm_e(begin: *const usize, end: *const usize) -> i32;
    fn _set_app_type(t: i32);
    fn _configure_narrow_argv(mode: i32) -> i32;
    fn _initialize_narrow_environment() -> i32;
    fn _get_initial_narrow_environment() -> *mut *mut u8;
    fn __p___argc() -> *mut i32;
    fn __p___argv() -> *mut *mut *mut u8;
    fn _crt_atexit(f: extern "C" fn()) -> i32;
    fn _initialize_onexit_table(t: *mut [usize; 3]) -> i32;
    fn _register_onexit_function(t: *mut [usize; 3], f: extern "C" fn()) -> i32;
    fn _execute_onexit_table(t: *mut [usize; 3]) -> i32;
    fn _register_thread_local_exe_atexit_callback(f: extern "system" fn(*mut u8, u32, *mut u8));
    fn _set_invalid_parameter_handler(h: extern "C" fn(*const u16, *const u16, *const u16, u32, usize)) -> usize;
    fn _errno() -> *mut i32;
    fn _beginthreadex(
        sa: *const u8,
        stack: u32,
        start: extern "system" fn(*mut u8) -> u32,
        arg: *mut u8,
        flags: u32,
        id: *mut u32,
    ) -> usize;
    fn signal(sig: i32, handler: usize) -> usize;
    fn raise(sig: i32) -> i32;
    fn strerror(e: i32) -> *const u8;
    fn exit(code: i32) -> !;
    fn _seh_filter_exe(code: u32, info: *mut u8) -> i32;
}

#[link(name = "api-ms-win-crt-heap-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn malloc(n: usize) -> *mut u8;
    fn calloc(n: usize, s: usize) -> *mut u8;
    fn realloc(p: *mut u8, n: usize) -> *mut u8;
    fn free(p: *mut u8);
    fn _msize(p: *mut u8) -> usize;
    fn _aligned_malloc(n: usize, a: usize) -> *mut u8;
    fn _aligned_free(p: *mut u8);
    fn _recalloc(p: *mut u8, n: usize, s: usize) -> *mut u8;
    fn _set_new_mode(m: i32) -> i32;
}

#[link(name = "api-ms-win-crt-string-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn strlen(s: *const u8) -> usize;
    fn strcmp(a: *const u8, b: *const u8) -> i32;
    fn _stricmp(a: *const u8, b: *const u8) -> i32;
    fn strncpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8;
    fn strcpy_s(d: *mut u8, n: usize, s: *const u8) -> i32;
    fn strtok(s: *mut u8, d: *const u8) -> *mut u8;
    fn strstr(h: *const u8, n: *const u8) -> *const u8;
    fn strchr(s: *const u8, c: i32) -> *const u8;
    fn wcslen(s: *const u16) -> usize;
    fn _wcsicmp(a: *const u16, b: *const u16) -> i32;
    fn tolower(c: i32) -> i32;
    fn isalpha(c: i32) -> i32;
    fn isdigit(c: i32) -> i32;
    fn memchr(p: *const u8, c: i32, n: usize) -> *const u8;
    fn memcpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8;
    fn memmove(d: *mut u8, s: *const u8, n: usize) -> *mut u8;
    fn memset(d: *mut u8, c: i32, n: usize) -> *mut u8;
    fn _strdup(s: *const u8) -> *mut u8;
}

#[link(name = "api-ms-win-crt-stdio-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn __acrt_iob_func(i: u32) -> *mut File;
    fn __stdio_common_vfprintf(o: u64, f: *mut File, fmt: *const u8, l: *mut u8, va: *const u64) -> i32;
    fn __stdio_common_vsprintf(o: u64, b: *mut u8, n: usize, fmt: *const u8, l: *mut u8, va: *const u64) -> i32;
    fn __stdio_common_vsprintf_s(o: u64, b: *mut u8, n: usize, fmt: *const u8, l: *mut u8, va: *const u64) -> i32;
    fn __stdio_common_vswprintf(o: u64, b: *mut u16, n: usize, fmt: *const u16, l: *mut u8, va: *const u64) -> i32;
    fn __stdio_common_vsscanf(o: u64, b: *const u8, n: usize, fmt: *const u8, l: *mut u8, va: *const u64) -> i32;
    fn __p__commode() -> *mut i32;
    fn _set_fmode(m: i32) -> i32;
    fn fopen(n: *const u8, m: *const u8) -> *mut File;
    fn _wfopen(n: *const u16, m: *const u16) -> *mut File;
    fn fclose(f: *mut File) -> i32;
    fn fputs(s: *const u8, f: *mut File) -> i32;
    fn fgets(b: *mut u8, n: i32, f: *mut File) -> *mut u8;
    fn fgetc(f: *mut File) -> i32;
    fn ungetc(c: i32, f: *mut File) -> i32;
    fn fread(b: *mut u8, s: usize, n: usize, f: *mut File) -> usize;
    fn fwrite(b: *const u8, s: usize, n: usize, f: *mut File) -> usize;
    fn fseek(f: *mut File, o: i32, w: i32) -> i32;
    fn ftell(f: *mut File) -> i32;
    fn feof(f: *mut File) -> i32;
    fn fflush(f: *mut File) -> i32;
    fn _fileno(f: *mut File) -> i32;
    fn _get_osfhandle(fd: i32) -> isize;
    fn _open(n: *const u8, flags: i32, pmode: i32) -> i32;
    fn _read(fd: i32, b: *mut u8, n: u32) -> i32;
    fn _close(fd: i32) -> i32;
    fn _setmode(fd: i32, mode: i32) -> i32;
    fn puts(s: *const u8) -> i32;
}

#[link(name = "api-ms-win-crt-filesystem-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn remove(n: *const u8) -> i32;
    fn rename(a: *const u8, b: *const u8) -> i32;
}

#[link(name = "api-ms-win-crt-convert-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn strtol(s: *const u8, e: *mut *mut u8, b: i32) -> i32;
    fn strtoul(s: *const u8, e: *mut *mut u8, b: i32) -> u32;
    fn strtod(s: *const u8, e: *mut *mut u8) -> f64;
    fn atoi(s: *const u8) -> i32;
    fn _atoi64(s: *const u8) -> i64;
    fn _itoa(v: i32, b: *mut u8, r: i32) -> *mut u8;
    fn _ui64toa(v: u64, b: *mut u8, r: i32) -> *mut u8;
    fn _wtoi(s: *const u16) -> i32;
}

#[link(name = "api-ms-win-crt-environment-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn getenv(n: *const u8) -> *const u8;
    fn _putenv(s: *const u8) -> i32;
}

#[link(name = "api-ms-win-crt-time-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn _time64(t: *mut i64) -> i64;
    fn _gmtime64(t: *const i64) -> *const [i32; 9];
    fn _mktime64(tm: *mut [i32; 9]) -> i64;
    fn strftime(b: *mut u8, n: usize, f: *const u8, tm: *const [i32; 9]) -> usize;
    fn asctime(tm: *const [i32; 9]) -> *const u8;
    fn clock() -> i32;
}

#[link(name = "api-ms-win-crt-utility-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn qsort(b: *mut u8, n: usize, s: usize, c: extern "C" fn(*const u8, *const u8) -> i32);
    fn bsearch(
        k: *const u8,
        b: *const u8,
        n: usize,
        s: usize,
        c: extern "C" fn(*const u8, *const u8) -> i32,
    ) -> *const u8;
    fn abs(v: i32) -> i32;
}

#[link(name = "api-ms-win-crt-locale-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn setlocale(c: i32, l: *const u8) -> *const u8;
    fn localeconv() -> *const *const u8;
    fn _configthreadlocale(m: i32) -> i32;
}

#[link(name = "api-ms-win-crt-math-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn __setusermatherr(f: usize);
    fn floor(x: f64) -> f64;
    fn sqrt(x: f64) -> f64;
    fn fmod(x: f64, y: f64) -> f64;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetStdHandle(which: u32) -> isize;
    fn WriteFile(h: isize, b: *const u8, n: u32, w: *mut u32, o: *mut u8) -> i32;
    fn LoadLibraryW(n: *const u16) -> *mut u8;
    fn GetProcAddress(m: *mut u8, n: *const u8) -> *mut u8;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn CloseHandle(h: usize) -> i32;
    fn GetEnvironmentVariableW(n: *const u16, b: *mut u16, s: u32) -> u32;
}

/// The UCRT headers' default options (`corecrt_stdio_config.h`):
/// legacy wide specifiers, standard rounding.
const OPTS: u64 = 4 | 32;
const STANDARD_SNPRINTF: u64 = 2;
const LEGACY_VSPRINTF: u64 = 1;
const ERANGE: i32 = 34;
const ENOENT: i32 = 2;
const EBADF: i32 = 9;
const SIGTERM: i32 = 15;

static STEP: AtomicU32 = AtomicU32::new(0);
static INVALID: AtomicU32 = AtomicU32::new(0);
static INIT: AtomicU32 = AtomicU32::new(0);
static ORDER: AtomicU32 = AtomicU32::new(0);
static SIGNALLED: AtomicU32 = AtomicU32::new(0);

fn say(text: &[u8]) {
    let mut n = 0;
    // SAFETY: kernel32 with a valid buffer.
    unsafe {
        WriteFile(GetStdHandle((-11i32) as u32), text.as_ptr(), text.len() as u32, &mut n, core::ptr::null_mut())
    };
}

fn fail(step: u32) -> ! {
    let mut msg = *b"crttest: step 000 failed\r\n";
    msg[14] = b'0' + (step / 100 % 10) as u8;
    msg[15] = b'0' + (step / 10 % 10) as u8;
    msg[16] = b'0' + (step % 10) as u8;
    say(&msg);
    // SAFETY: no preconditions.
    unsafe { ExitProcess(0x80 + step) }
}

/// The next step: fail it unless `ok`.
fn check(ok: bool) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        fail(step);
    }
}

/// A C string's bytes (without the NUL).
fn cs<'a>(p: *const u8) -> &'a [u8] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: a C string from the runtime.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n bytes are readable.
    unsafe { core::slice::from_raw_parts(p, n) }
}

fn ws<const N: usize>(s: &[u8; N]) -> [u16; N] {
    let mut w = [0u16; N];
    for (i, &b) in s.iter().enumerate() {
        w[i] = u16::from(b);
    }
    w
}

/// `__stdio_common_vsprintf` into a 128-byte buffer.
fn fmt(options: u64, format: &[u8], args: &[u64]) -> ([u8; 128], i32) {
    let mut buf = [0xAAu8; 128];
    // SAFETY: a valid buffer, a NUL-terminated format and its arguments.
    let n = unsafe {
        __stdio_common_vsprintf(options, buf.as_mut_ptr(), 128, format.as_ptr(), core::ptr::null_mut(), args.as_ptr())
    };
    (buf, n)
}

fn fmt_is(format: &[u8], args: &[u64], want: &[u8]) -> bool {
    let (buf, n) = fmt(OPTS, format, args);
    n == want.len() as i32 && &buf[..want.len()] == want && buf[want.len()] == 0
}

extern "C" fn on_invalid(_e: *const u16, _f: *const u16, _file: *const u16, _l: u32, _r: usize) {
    INVALID.fetch_add(1, Ordering::Relaxed);
}

extern "C" fn init_v() {
    INIT.fetch_or(1, Ordering::Relaxed);
}

extern "C" fn init_i() -> i32 {
    INIT.fetch_or(2, Ordering::Relaxed);
    0
}

extern "C" fn cmp_i32(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: elements of the test's arrays.
    let (a, b) = unsafe { (*(a as *const i32), *(b as *const i32)) };
    a.cmp(&b) as i32
}

extern "C" fn on_sigterm(sig: i32) {
    SIGNALLED.store(sig as u32, Ordering::Relaxed);
}

extern "system" fn thread(arg: *mut u8) -> u32 {
    // SAFETY: the runtime's errno for this thread; `arg` the main thread's
    // u32.
    unsafe {
        *_errno() = 77;
        *(arg as *mut u32) = *_errno() as u32;
    }
    7
}

extern "C" fn onexit_a() {
    ORDER.fetch_add(if ORDER.load(Ordering::Relaxed) == 10 { 1 } else { 100 }, Ordering::Relaxed);
}

extern "C" fn onexit_b() {
    ORDER.store(10, Ordering::Relaxed);
}

extern "system" fn tls_exit(_m: *mut u8, reason: u32, _r: *mut u8) {
    if reason == 0 {
        ORDER.store(20, Ordering::Relaxed);
    }
}

extern "C" fn atexit_last_registered() {
    // Runs first: after the thread-local callback.
    if ORDER.load(Ordering::Relaxed) != 20 {
        fail(250);
    }
    ORDER.store(21, Ordering::Relaxed);
}

extern "C" fn atexit_first_registered() {
    if ORDER.load(Ordering::Relaxed) != 21 {
        fail(251);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn crttest_main() -> u32 {
    // SAFETY: the runtime's documented functions, each with valid arguments.
    unsafe { run() }
}

unsafe fn run() -> ! {
    // SAFETY: the whole body calls the runtime with valid arguments.
    unsafe {
        // ----- what MSVC's start-up code does -----
        _set_app_type(1);
        __setusermatherr(0);
        check(_configure_narrow_argv(1) == 0); // 1
        check(_initialize_narrow_environment() == 0);
        check(_set_fmode(0x4000) == 0);
        *__p__commode() = 0;
        check(_configthreadlocale(-1) == 2);
        check(_set_new_mode(0) == 0);
        let tv: [usize; 3] = [0, init_v as extern "C" fn() as usize, 0];
        let te: [usize; 2] = [init_i as extern "C" fn() -> i32 as usize, 0];
        check(_initterm_e(te.as_ptr(), te.as_ptr().add(2)) == 0);
        _initterm(tv.as_ptr(), tv.as_ptr().add(3));
        check(INIT.load(Ordering::Relaxed) == 3);
        _set_invalid_parameter_handler(on_invalid);

        // ----- arguments and environment -----
        let argc = *__p___argc();
        let argv = *__p___argv();
        check(argc == 4); // 8
        check(cs(*argv).ends_with(b"crttest.exe"));
        check(cs(*argv.add(1)) == b"--poly");
        check(cs(*argv.add(2)) == b"two words");
        check(cs(*argv.add(3)) == b"q\"uote");
        check((*argv.add(4)).is_null());
        check(!_get_initial_narrow_environment().is_null());
        check(_stricmp(getenv(c"SYSTEMROOT".as_ptr() as *const u8), c"C:\\Windows".as_ptr() as *const u8) == 0); // 15
        check(_putenv(c"POLY_CRT=yes".as_ptr() as *const u8) == 0);
        check(cs(getenv(c"poly_crt".as_ptr() as *const u8)) == b"yes");
        let mut w = [0u16; 8];
        check(GetEnvironmentVariableW(ws(b"POLY_CRT\0").as_ptr(), w.as_mut_ptr(), 8) == 3);
        check(_putenv(c"POLY_CRT=".as_ptr() as *const u8) == 0);
        check(getenv(c"POLY_CRT".as_ptr() as *const u8).is_null()); // 20

        // ----- the heap -----
        let p = malloc(100);
        check(!p.is_null() && _msize(p) >= 100);
        memset(p, 0x5A, 100);
        let p = realloc(p, 1000);
        check(!p.is_null() && *p == 0x5A && *p.add(99) == 0x5A);
        free(p);
        let z = calloc(10, 10);
        check(!z.is_null() && core::slice::from_raw_parts(z, 100).iter().all(|&b| b == 0));
        let z = _recalloc(z, 20, 10);
        check(!z.is_null() && core::slice::from_raw_parts(z, 200).iter().all(|&b| b == 0)); // 24
        free(z);
        let a = _aligned_malloc(100, 64);
        check(!a.is_null() && (a as usize).is_multiple_of(64));
        _aligned_free(a);
        let e = malloc(0);
        check(!e.is_null());
        free(e);
        free(core::ptr::null_mut());

        // ----- strings -----
        check(strlen(c"hello".as_ptr() as *const u8) == 5); // 27
        check(strcmp(c"a".as_ptr() as *const u8, c"b".as_ptr() as *const u8) < 0);
        check(_stricmp(c"HeLLo".as_ptr() as *const u8, c"hello".as_ptr() as *const u8) == 0);
        let mut t = *b"a,b,,c\0";
        let d = c",".as_ptr() as *const u8;
        check(cs(strtok(t.as_mut_ptr(), d)) == b"a");
        check(cs(strtok(core::ptr::null_mut(), d)) == b"b");
        check(cs(strtok(core::ptr::null_mut(), d)) == b"c");
        check(strtok(core::ptr::null_mut(), d).is_null());
        check(cs(strstr(c"haystack".as_ptr() as *const u8, c"st".as_ptr() as *const u8)) == b"stack");
        check(cs(strchr(c"a=b".as_ptr() as *const u8, i32::from(b'='))) == b"=b"); // 35
        check(wcslen(ws(b"wide\0").as_ptr()) == 4);
        check(_wcsicmp(ws(b"ABC\0").as_ptr(), ws(b"abc\0").as_ptr()) == 0);
        let mut n = [0xFFu8; 6];
        strncpy(n.as_mut_ptr(), c"ab".as_ptr() as *const u8, 5);
        check(n == [b'a', b'b', 0, 0, 0, 0xFF]);
        check(
            tolower(i32::from(b'Q')) == i32::from(b'q')
                && isalpha(i32::from(b'1')) == 0
                && isdigit(i32::from(b'7')) != 0,
        );
        let hay = b"0123456789";
        check(memchr(hay.as_ptr(), i32::from(b'7'), 10) == hay.as_ptr().add(7)); // 40
        let mut m = *b"abcdef";
        memmove(m.as_mut_ptr().add(1), m.as_ptr(), 4);
        check(&m == b"aabcdf");
        let mut c4 = [0u8; 4];
        memcpy(c4.as_mut_ptr(), c"xyz".as_ptr() as *const u8, 4);
        check(&c4 == b"xyz\0");
        let before = INVALID.load(Ordering::Relaxed);
        let mut small = [0xEEu8; 4];
        check(strcpy_s(small.as_mut_ptr(), 4, c"toolong".as_ptr() as *const u8) == ERANGE);
        check(small[0] == 0 && INVALID.load(Ordering::Relaxed) == before + 1);
        let dup = _strdup(c"copy".as_ptr() as *const u8);
        check(cs(dup) == b"copy"); // 45
        free(dup);

        // ----- printf -----
        let hi = c"hi".as_ptr() as u64;
        check(fmt_is(
            b"%d|%5.2f|%e|%g|%x|%s\0",
            &[(-42i64) as u64, 2.5f64.to_bits(), 12345.678f64.to_bits(), 0.0001f64.to_bits(), 255, hi],
            b"-42| 2.50|1.234568e+04|0.0001|ff|hi",
        ));
        let wide = ws(b"wide\0");
        check(fmt_is(
            b"%s|%ls|%I64d|%zu|%p\0",
            &[0, wide.as_ptr() as u64, (-5i64) as u64, 7, 0x1000],
            b"(null)|wide|-5|7|0000000000001000",
        ));
        let ind = 0xFFF8_0000_0000_0000u64;
        check(fmt_is(
            b"%f %F %f\0",
            &[f64::INFINITY.to_bits(), f64::NEG_INFINITY.to_bits(), ind],
            b"inf -INF -nan(ind)",
        ));
        check(fmt_is(
            b"%.0f %.0f %+.3d %-4c|\0",
            &[0.5f64.to_bits(), 1.5f64.to_bits(), 7, u64::from(b'z')],
            b"0 2 +007 z   |",
        ));
        // snprintf into 5 bytes: cut, terminated, the full length returned.
        let mut small = [0xEEu8; 8];
        let r = __stdio_common_vsprintf(
            OPTS | STANDARD_SNPRINTF,
            small.as_mut_ptr(),
            5,
            c"abcdefgh".as_ptr() as *const u8,
            core::ptr::null_mut(),
            [0u64].as_ptr(),
        );
        check(r == 8 && &small[..6] == b"abcd\0\xEE"); // 50
        let r = __stdio_common_vsprintf(
            OPTS | STANDARD_SNPRINTF,
            core::ptr::null_mut(),
            0,
            c"%d".as_ptr() as *const u8,
            core::ptr::null_mut(),
            [12345u64].as_ptr(),
        );
        check(r == 5);
        let mut small = [0xEEu8; 8];
        let r = __stdio_common_vsprintf(
            OPTS | LEGACY_VSPRINTF,
            small.as_mut_ptr(),
            3,
            c"abc".as_ptr() as *const u8,
            core::ptr::null_mut(),
            [0u64].as_ptr(),
        );
        check(r == 3 && &small[..4] == b"abc\xEE");
        let before = INVALID.load(Ordering::Relaxed);
        *_errno() = 0;
        let r = __stdio_common_vsprintf_s(
            OPTS,
            small.as_mut_ptr(),
            3,
            c"abcdef".as_ptr() as *const u8,
            core::ptr::null_mut(),
            [0u64].as_ptr(),
        );
        check(r == -1 && small[0] == 0 && *_errno() == ERANGE && INVALID.load(Ordering::Relaxed) == before + 1);
        let mut wbuf = [0u16; 16];
        let wfmt = ws(b"%s %hs %d\0");
        let wa = ws(b"w\0");
        let r = __stdio_common_vswprintf(
            OPTS,
            wbuf.as_mut_ptr(),
            16,
            wfmt.as_ptr(),
            core::ptr::null_mut(),
            [wa.as_ptr() as u64, c"n".as_ptr() as u64, 9].as_ptr(),
        );
        check(r == 5 && wbuf[..6] == ws(b"w n 9\0")); // 54
        let (mut i, mut dv, mut word) = (0i32, 0f64, [0u8; 8]);
        let r = __stdio_common_vsscanf(
            2,
            c"42 3.5 word".as_ptr() as *const u8,
            usize::MAX,
            c"%d %lf %s".as_ptr() as *const u8,
            core::ptr::null_mut(),
            [&mut i as *mut i32 as u64, &mut dv as *mut f64 as u64, word.as_mut_ptr() as u64].as_ptr(),
        );
        check(r == 3 && i == 42 && dv == 3.5 && &word[..5] == b"word\0");
        let out = __acrt_iob_func(1);
        let r = __stdio_common_vfprintf(
            OPTS,
            out,
            c"crttest: %s\n".as_ptr() as *const u8,
            core::ptr::null_mut(),
            [c"printing through the UCRT".as_ptr() as u64].as_ptr(),
        );
        check(r == 35);
        check(fflush(out) == 0 && puts(c"crttest: puts".as_ptr() as *const u8) >= 0);

        // ----- conversions -----
        let mut end: *mut u8 = core::ptr::null_mut();
        let s = c"  0x1fZ".as_ptr() as *const u8;
        check(strtol(s, &mut end, 0) == 31 && core::ptr::eq(end, s.add(6))); // 58
        *_errno() = 0;
        check(
            strtoul(c"99999999999".as_ptr() as *const u8, core::ptr::null_mut(), 10) == u32::MAX && *_errno() == ERANGE,
        );
        let s = c"1e-2xyz".as_ptr() as *const u8;
        check(strtod(s, &mut end) == 0.01 && core::ptr::eq(end, s.add(4))); // 60
        check(strtod(c"0x1.8p1".as_ptr() as *const u8, core::ptr::null_mut()) == 3.0);
        check(
            atoi(c"  -17abc".as_ptr() as *const u8) == -17
                && _atoi64(c"9000000000".as_ptr() as *const u8) == 9_000_000_000,
        );
        let mut nb = [0u8; 40];
        check(cs(_itoa(-255, nb.as_mut_ptr(), 16)) == b"ffffff01");
        check(cs(_itoa(-255, nb.as_mut_ptr(), 10)) == b"-255");
        check(cs(_ui64toa(0xDEAD_BEEF, nb.as_mut_ptr(), 16)) == b"deadbeef");
        check(_wtoi(ws(b"123\0").as_ptr()) == 123); // 66

        // ----- files and descriptors -----
        let name = c"crt.txt".as_ptr() as *const u8;
        let f = fopen(name, c"w".as_ptr() as *const u8);
        check(!f.is_null());
        check(fputs(c"line one\n".as_ptr() as *const u8, f) >= 0);
        check(
            __stdio_common_vfprintf(OPTS, f, c"n=%d\n".as_ptr() as *const u8, core::ptr::null_mut(), [7u64].as_ptr())
                == 4,
        );
        check(fclose(f) == 0); // 70
        let f = fopen(name, c"rb".as_ptr() as *const u8);
        let mut raw = [0u8; 64];
        check(!f.is_null() && fread(raw.as_mut_ptr(), 1, 64, f) == 15 && &raw[..15] == b"line one\r\nn=7\r\n");
        check(feof(f) != 0 && fclose(f) == 0);
        let f = fopen(name, c"r".as_ptr() as *const u8);
        let mut line = [0u8; 32];
        check(!f.is_null() && cs(fgets(line.as_mut_ptr(), 32, f)) == b"line one\n");
        check(cs(fgets(line.as_mut_ptr(), 32, f)) == b"n=7\n");
        check(fgets(line.as_mut_ptr(), 32, f).is_null() && feof(f) != 0); // 75
        fclose(f);
        let f = fopen(name, c"rb".as_ptr() as *const u8);
        check(fseek(f, 5, 0) == 0 && fgetc(f) == i32::from(b'o') && ftell(f) == 6);
        check(ungetc(i32::from(b'X'), f) == i32::from(b'X') && fgetc(f) == i32::from(b'X') && ftell(f) == 6);
        check(fseek(f, 0, 2) == 0 && ftell(f) == 15);
        check(_get_osfhandle(_fileno(f)) != -1);
        fclose(f);
        let f = fopen(name, c"a".as_ptr() as *const u8);
        check(fwrite(b"z".as_ptr(), 1, 1, f) == 1 && fclose(f) == 0); // 80
        let fd = _open(name, 0x8000, 0);
        let mut four = [0u8; 4];
        check(fd >= 3 && _read(fd, four.as_mut_ptr(), 4) == 4 && &four == b"line");
        check(_setmode(fd, 0x4000) == 0x8000);
        check(_close(fd) == 0);
        let before = INVALID.load(Ordering::Relaxed);
        check(_close(fd) == -1 && *_errno() == EBADF && INVALID.load(Ordering::Relaxed) == before + 1);
        *_errno() = 0;
        check(fopen(c"missing.txt".as_ptr() as *const u8, c"r".as_ptr() as *const u8).is_null() && *_errno() == ENOENT); // 85
        check(rename(name, c"crt2.txt".as_ptr() as *const u8) == 0);
        check(remove(c"crt2.txt".as_ptr() as *const u8) == 0);
        check(fopen(c"crt2.txt".as_ptr() as *const u8, c"r".as_ptr() as *const u8).is_null());
        let wf = _wfopen(ws(b"wide.txt\0").as_ptr(), ws(b"w+\0").as_ptr());
        check(!wf.is_null() && fputs(c"via _wfopen".as_ptr() as *const u8, wf) >= 0 && fseek(wf, 0, 0) == 0);
        check(cs(fgets(line.as_mut_ptr(), 32, wf)) == b"via _wfopen" && fclose(wf) == 0); // 90
        check(remove(c"wide.txt".as_ptr() as *const u8) == 0);

        // ----- time -----
        let t: i64 = 1_700_000_000;
        let tm = _gmtime64(&t);
        check(!tm.is_null() && *tm == [20, 13, 22, 14, 10, 123, 2, 317, 0]);
        let mut copy = *tm;
        check(_mktime64(&mut copy) == t);
        let mut tb = [0u8; 64];
        check(
            strftime(tb.as_mut_ptr(), 64, c"%Y-%m-%d %H:%M:%S".as_ptr() as *const u8, tm) == 19
                && cs(tb.as_ptr()) == b"2023-11-14 22:13:20",
        );
        check(cs(asctime(tm)) == b"Tue Nov 14 22:13:20 2023\n"); // 95
        check(_time64(core::ptr::null_mut()) > 0 && clock() >= 0);

        // ----- sorting, numbers -----
        let mut v = [5i32, 3, 9, 1, 7];
        qsort(v.as_mut_ptr() as *mut u8, 5, 4, cmp_i32);
        check(v == [1, 3, 5, 7, 9]);
        let key = 7i32;
        let hit = bsearch(&key as *const i32 as *const u8, v.as_ptr() as *const u8, 5, 4, cmp_i32);
        check(hit == v.as_ptr().add(3) as *const u8);
        check(abs(-3) == 3 && floor(-1.5) == -2.0 && sqrt(2.25) == 1.5 && fmod(5.5, 2.0) == 1.5);

        // ----- locale -----
        check(cs(setlocale(0, c"".as_ptr() as *const u8)) == b"C"); // 100
        check(setlocale(0, c"xx-YY".as_ptr() as *const u8).is_null());
        check(cs(*localeconv()) == b"." && _configthreadlocale(0) == 2);

        // ----- signals, errors, threads -----
        check(signal(SIGTERM, on_sigterm as extern "C" fn(i32) as usize) == 0);
        check(raise(SIGTERM) == 0 && SIGNALLED.load(Ordering::Relaxed) == 15);
        // 105: the action was reset to SIG_DFL before the handler ran.
        check(signal(SIGTERM, 1) == 0);
        check(cs(strerror(ENOENT)) == b"No such file or directory");
        check(_seh_filter_exe(0xC000_0005, core::ptr::null_mut()) == 0);
        *_errno() = 11;
        let mut seen = 0u32;
        let h =
            _beginthreadex(core::ptr::null(), 0, thread, &mut seen as *mut u32 as *mut u8, 0, core::ptr::null_mut());
        check(h != 0 && WaitForSingleObject(h, 0xFFFF_FFFF) == 0);
        check(seen == 77 && *_errno() == 11);
        CloseHandle(h);

        // ----- msvcrt.dll -----
        msvcrt(); // 110 …

        // ----- math, positional printf, setjmp (ADR 0092) -----
        mathtest::run(); // 300 …

        // ----- exit -----
        let mut table = [0usize; 3];
        check(_initialize_onexit_table(&mut table) == 0);
        check(
            _register_onexit_function(&mut table, onexit_a) == 0
                && _register_onexit_function(&mut table, onexit_b) == 0,
        );
        check(_execute_onexit_table(&mut table) == 0 && ORDER.load(Ordering::Relaxed) == 11);
        _register_thread_local_exe_atexit_callback(tls_exit);
        check(_crt_atexit(atexit_first_registered) == 0 && _crt_atexit(atexit_last_registered) == 0);
        // Left open: `exit` must write it.
        let f = fopen(c"crtexit.txt".as_ptr() as *const u8, c"w".as_ptr() as *const u8);
        check(!f.is_null() && fputs(c"flushed at exit\n".as_ptr() as *const u8, f) >= 0);
        exit(PASS)
    }
}

/// `msvcrt.dll`'s classic interface, through `GetProcAddress`.
unsafe fn msvcrt() {
    // SAFETY: the runtime's documented functions with valid arguments.
    unsafe {
        let m = LoadLibraryW(ws(b"msvcrt.dll\0").as_ptr());
        check(!m.is_null()); // 110
        let get = |n: &core::ffi::CStr| GetProcAddress(m, n.as_ptr() as *const u8);
        type Sprintf = unsafe extern "C" fn(*mut u8, *const u8, ...) -> i32;
        type Snprintf = unsafe extern "C" fn(*mut u8, usize, *const u8, ...) -> i32;
        type Swprintf = unsafe extern "C" fn(*mut u16, *const u16, ...) -> i32;
        type Sscanf = unsafe extern "C" fn(*const u8, *const u8, ...) -> i32;
        type GetMainArgs = unsafe extern "C" fn(*mut i32, *mut *mut *mut u8, *mut *mut *mut u8, i32, *const i32) -> i32;
        type Fprintf = unsafe extern "C" fn(*mut u8, *const u8, ...) -> i32;
        let sprintf: Sprintf = core::mem::transmute(get(c"sprintf"));
        let snprintf: Snprintf = core::mem::transmute(get(c"_snprintf"));
        let swprintf: Swprintf = core::mem::transmute(get(c"swprintf"));
        let sscanf: Sscanf = core::mem::transmute(get(c"sscanf"));
        let getmainargs: GetMainArgs = core::mem::transmute(get(c"__getmainargs"));
        let fprintf: Fprintf = core::mem::transmute(get(c"fprintf"));
        let iob_func: unsafe extern "C" fn() -> *mut u8 = core::mem::transmute(get(c"__iob_func"));
        let iob = get(c"_iob");
        let mut buf = [0xEEu8; 64];
        let r = sprintf(
            buf.as_mut_ptr(),
            c"%d %s %.2f %e %d %d %d".as_ptr() as *const u8,
            42,
            c"x".as_ptr(),
            1.5f64,
            1e10f64,
            5,
            6,
            7,
        );
        let want = b"42 x 1.50 1.000000e+010 5 6 7";
        check(r == want.len() as i32 && &buf[..want.len()] == want && buf[want.len()] == 0);
        let mut small = [0xEEu8; 4];
        check(snprintf(small.as_mut_ptr(), 3, c"abcdef".as_ptr() as *const u8) == -1 && &small == b"abc\xEE");
        let mut wbuf = [0u16; 8];
        check(
            swprintf(wbuf.as_mut_ptr(), ws(b"%s-%S\0").as_ptr(), ws(b"w\0").as_ptr(), c"n".as_ptr()) == 3
                && wbuf[..4] == ws(b"w-n\0"),
        );
        let (mut i, mut word) = (0i32, [0u8; 8]);
        check(
            sscanf(
                c"12 ab".as_ptr() as *const u8,
                c"%d %s".as_ptr() as *const u8,
                &mut i as *mut i32,
                word.as_mut_ptr(),
            ) == 2
                && i == 12
                && &word[..3] == b"ab\0",
        );
        check(!iob.is_null() && iob_func() == iob); // 115
        let (mut argc, mut argv, mut env) = (0i32, core::ptr::null_mut(), core::ptr::null_mut());
        let newmode = 0i32;
        check(
            getmainargs(&mut argc, &mut argv, &mut env, 0, &newmode) == 0
                && argc == 4
                && cs(*argv.add(2)) == b"two words"
                && !env.is_null(),
        );
        // stdout is _iob[1] (48-byte FILEs).
        check(fprintf(iob.add(48), c"crttest: msvcrt %s %d\n".as_ptr() as *const u8, c"fprintf".as_ptr(), 1) == 26);
        // msvcrt's own buffer (a file is fully buffered): write it now.
        let mflush: unsafe extern "C" fn(*mut u8) -> i32 = core::mem::transmute(get(c"fflush"));
        check(mflush(iob.add(48)) == 0);
    }
}
