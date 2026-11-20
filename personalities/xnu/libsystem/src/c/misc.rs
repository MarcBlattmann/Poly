//! libsystem_c beyond v0 (ADR 0257 addendum, for SDK-linked command-line
//! programs such as Lua and the SQLite shell): character classes, more
//! string functions, `strerror`, `strtod`, calendar time, the C locale,
//! `sscanf`, random numbers, `sysconf`, signals, `assert` and a few
//! wrappers. The logic is in `poly_libsystem` (host-tested). Constants
//! are those of the SDK headers named at each.

use super::*;
use poly_libsystem::ctype::{self, CACHED};
use poly_libsystem::time::{self as ctime, Tm};
use poly_libsystem::{errstr, num, scanf};

unsafe extern "C" {
    fn ioctl(fd: i32, req: u64, arg: *mut u8) -> i32;
    fn __sigaction(sig: i32, new: *const KernelAction, old: *mut Action) -> i32;
    fn getrusage(who: i32, usage: *mut u8) -> i32;
    fn __ulock_wait2(op: u32, addr: *const u32, value: u64, timeout_ns: u64, value2: u64) -> i32;
    fn __sysctlbyname(
        name: *const u8,
        len: usize,
        old: *mut u8,
        oldlen: *mut usize,
        new: *const u8,
        newlen: usize,
    ) -> i32;
}

// ------------------------------------------------------------ ctype

/// `_RuneLocale` (`<runetype.h>`).
#[repr(C)]
pub struct RuneLocale {
    magic: [u8; 8],
    encoding: [u8; 32],
    sgetrune: usize,
    sputrune: usize,
    invalid_rune: i32,
    runetype: [u32; CACHED],
    maplower: [i32; CACHED],
    mapupper: [i32; CACHED],
    ext: [[usize; 2]; 3],
    variable: usize,
    variable_len: i32,
    ncharclasses: i32,
    charclasses: usize,
}

/// `_DefaultRuneLocale`: the C locale's tables, which `<ctype.h>`'s
/// inline functions read.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static _DefaultRuneLocale: RuneLocale = RuneLocale {
    magic: *b"RuneMagi",
    encoding: *b"NONE\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    sgetrune: 0,
    sputrune: 0,
    invalid_rune: 0xfffd,
    runetype: ctype::runetype(),
    maplower: ctype::map(false),
    mapupper: ctype::map(true),
    ext: [[0; 2]; 3],
    variable: 0,
    variable_len: 0,
    ncharclasses: 0,
    charclasses: 0,
};

#[unsafe(no_mangle)]
extern "C" fn __maskrune(c: i32, mask: u64) -> i32 {
    ctype::mask_rune(c, mask)
}

#[unsafe(no_mangle)]
extern "C" fn __tolower(c: i32) -> i32 {
    ctype::map_rune(c, false)
}

#[unsafe(no_mangle)]
extern "C" fn __toupper(c: i32) -> i32 {
    ctype::map_rune(c, true)
}

#[unsafe(no_mangle)]
extern "C" fn tolower(c: i32) -> i32 {
    ctype::map_rune(c, false)
}

#[unsafe(no_mangle)]
extern "C" fn toupper(c: i32) -> i32 {
    ctype::map_rune(c, true)
}

macro_rules! is {
    ($($name:ident = $mask:expr;)*) => {
        $(
            #[unsafe(no_mangle)]
            extern "C" fn $name(c: i32) -> i32 {
                i32::from(ctype::mask_rune(c, u64::from($mask)) != 0)
            }
        )*
    };
}

is! {
    isalnum = ctype::A | ctype::D;
    isalpha = ctype::A;
    iscntrl = ctype::C;
    isdigit = ctype::D;
    isgraph = ctype::G;
    islower = ctype::L;
    isprint = ctype::R;
    ispunct = ctype::P;
    isspace = ctype::S;
    isupper = ctype::U;
    isxdigit = ctype::X;
}

// ---------------------------------------------------------- strings

/// The bytes of a C string.
///
/// # Safety
/// `s` is a string.
unsafe fn bytes<'a>(s: *const u8) -> &'a [u8] {
    // SAFETY: the caller's contract.
    unsafe { core::slice::from_raw_parts(s, cstr::strlen(s)) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strspn(s: *const u8, accept: *const u8) -> usize {
    // SAFETY: the C contract: two strings.
    let (s, a) = unsafe { (bytes(s), bytes(accept)) };
    s.iter().take_while(|c| a.contains(c)).count()
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strcspn(s: *const u8, reject: *const u8) -> usize {
    // SAFETY: the C contract: two strings.
    let (s, r) = unsafe { (bytes(s), bytes(reject)) };
    s.iter().take_while(|c| !r.contains(c)).count()
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strpbrk(s: *const u8, accept: *const u8) -> *mut u8 {
    // SAFETY: the C contract: two strings.
    let (b, a) = unsafe { (bytes(s), bytes(accept)) };
    match b.iter().position(|c| a.contains(c)) {
        // SAFETY: inside the string.
        Some(i) => unsafe { s.add(i) as *mut u8 },
        None => core::ptr::null_mut(),
    }
}

/// `strcoll`: the C locale collates by byte.
#[unsafe(no_mangle)]
unsafe extern "C" fn strcoll(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the C contract.
    unsafe { strcmp(a, b) }
}

unsafe extern "C" {
    fn strcmp(a: *const u8, b: *const u8) -> i32;
}

static ERRBUF: crate::Global<[u8; 64]> = crate::Global::new([0; 64]);

#[unsafe(no_mangle)]
extern "C" fn strerror(e: i32) -> *mut u8 {
    // SAFETY: one use per call (one thread uses the buffer at a time, as
    // C allows strerror to share one).
    let buf = unsafe { ERRBUF.get() };
    errstr::format(e, buf);
    buf.as_mut_ptr()
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strerror_r(e: i32, buf: *mut u8, n: usize) -> i32 {
    // SAFETY: the C contract: `buf` holds `n` bytes.
    let out = unsafe { core::slice::from_raw_parts_mut(buf, n) };
    let len = errstr::format(e, out);
    if errstr::message(e).is_none() {
        EINVAL
    } else if n == 0 || len < errstr::message(e).map_or(0, str::len) {
        ERANGE
    } else {
        0
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memset_pattern16(b: *mut u8, pattern: *const u8, len: usize) {
    // SAFETY: the C contract: `b` holds `len` bytes, `pattern` 16.
    unsafe {
        for i in 0..len {
            *b.add(i) = *pattern.add(i % 16);
        }
    }
}

// ---------------------------------------------------------- numbers

#[unsafe(no_mangle)]
unsafe extern "C" fn strtod(s: *const u8, end: *mut *mut u8) -> f64 {
    // SAFETY: the C contract: a string; `end` null or writable.
    unsafe {
        let p = num::strtod(bytes(s));
        if p.range {
            crate::set_errno(ERANGE);
        }
        if !end.is_null() {
            *end = s.add(p.len) as *mut u8;
        }
        p.value
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strtof(s: *const u8, end: *mut *mut u8) -> f32 {
    // SAFETY: the C contract.
    unsafe { strtod(s, end) as f32 }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strtold(s: *const u8, end: *mut *mut u8) -> f64 {
    // SAFETY: the C contract (long double is double on arm64; on x86-64 a
    // v0 limit).
    unsafe { strtod(s, end) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn atof(s: *const u8) -> f64 {
    // SAFETY: the C contract.
    unsafe { strtod(s, core::ptr::null_mut()) }
}

static SEED: crate::Global<u64> = crate::Global::new(1);

fn next_random() -> u64 {
    // SAFETY: one use per call.
    let s = unsafe { SEED.get() };
    // A 64-bit LCG (Knuth's MMIX constants); the top bits are returned.
    *s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
    *s >> 33
}

#[unsafe(no_mangle)]
extern "C" fn random() -> i64 {
    (next_random() & 0x7fff_ffff) as i64
}

#[unsafe(no_mangle)]
extern "C" fn srandom(seed: u32) {
    // SAFETY: one use per call.
    unsafe { *SEED.get() = u64::from(seed) };
}

#[unsafe(no_mangle)]
extern "C" fn srandomdev() {
    let mut s = 0u64;
    // SAFETY: eight bytes of our own.
    unsafe {
        if getentropy((&raw mut s).cast(), 8) != 0 {
            s = now_us();
        }
        *SEED.get() = s;
    }
}

#[unsafe(no_mangle)]
extern "C" fn rand() -> i32 {
    (next_random() & 0x7fff_ffff) as i32
}

#[unsafe(no_mangle)]
extern "C" fn srand(seed: u32) {
    srandom(seed);
}

#[unsafe(no_mangle)]
extern "C" fn arc4random() -> u32 {
    let mut v = 0u32;
    // SAFETY: four bytes of our own.
    if unsafe { getentropy((&raw mut v).cast(), 4) } != 0 {
        v = next_random() as u32;
    }
    v
}

#[unsafe(no_mangle)]
unsafe extern "C" fn arc4random_buf(buf: *mut u8, n: usize) {
    // SAFETY: the C contract: `buf` holds `n` bytes.
    unsafe {
        let mut done = 0;
        while done < n {
            let chunk = (n - done).min(256);
            if getentropy(buf.add(done), chunk) != 0 {
                for i in 0..chunk {
                    *buf.add(done + i) = next_random() as u8;
                }
            }
            done += chunk;
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn arc4random_uniform(bound: u32) -> u32 {
    if bound < 2 {
        return 0;
    }
    // Reject the values that would make some results likelier.
    let min = bound.wrapping_neg() % bound;
    loop {
        let r = arc4random();
        if r >= min {
            return r % bound;
        }
    }
}

// -------------------------------------------------------------- scanf

/// `vsscanf(s, fmt, ap)`.
#[unsafe(no_mangle)]
pub(super) unsafe extern "C" fn vsscanf(s: *const u8, fmt: *const u8, ap: CVaList) -> i32 {
    #[cfg(target_arch = "x86_64")]
    let mut args = VaList(ap);
    #[cfg(target_arch = "aarch64")]
    let mut args = printf::StackSlots(ap);
    // SAFETY: the C contract: two strings, and the pointers fmt names.
    unsafe { scanf::sscanf(bytes(s), bytes(fmt), &mut args) }
}

// --------------------------------------------------------------- time

/// `struct timeval` (`<sys/_types/_timeval.h>`).
#[repr(C)]
#[derive(Default)]
struct Timeval {
    sec: i64,
    usec: i32,
}

/// `gettimeofday(tv, tz)`: the kernel's call (BSD 116) writes the time
/// into `tv`; the zone is UTC.
#[unsafe(no_mangle)]
unsafe extern "C" fn gettimeofday(tv: *mut u8, tz: *mut u8) -> i32 {
    match crate::raw3(116, tv as u64, 0, 0) {
        Ok(_) => {
            if !tz.is_null() {
                // SAFETY: the C contract: a struct timezone (two ints).
                unsafe { core::ptr::write_bytes(tz, 0, 8) };
            }
            0
        }
        Err(e) => {
            crate::set_errno(e as i32);
            -1
        }
    }
}

/// Microseconds since the Epoch (0 if the kernel cannot tell).
fn now_us() -> u64 {
    let mut tv = Timeval::default();
    // SAFETY: our own timeval.
    if unsafe { gettimeofday((&raw mut tv).cast(), core::ptr::null_mut()) } != 0 {
        return 0;
    }
    (tv.sec as u64).wrapping_mul(1_000_000).wrapping_add(tv.usec as u64)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn time(out: *mut i64) -> i64 {
    let t = (now_us() / 1_000_000) as i64;
    if !out.is_null() {
        // SAFETY: the C contract.
        unsafe { *out = t };
    }
    t
}

#[unsafe(no_mangle)]
extern "C" fn difftime(a: i64, b: i64) -> f64 {
    a as f64 - b as f64
}

/// The process's start, for `clock` when the kernel has no `getrusage`.
static STARTED: crate::Global<u64> = crate::Global::new(0);

/// `clock`: processor time in `CLOCKS_PER_SEC` (1,000,000, `<time.h>`)
/// units: `getrusage`'s user and system time, or the time since the
/// program started.
#[unsafe(no_mangle)]
extern "C" fn clock() -> u64 {
    let mut ru = [0u8; 144];
    // SAFETY: a struct rusage is 144 bytes (<sys/resource.h>); RUSAGE_SELF.
    if unsafe { getrusage(0, ru.as_mut_ptr()) } == 0 {
        let word = |o: usize| i64::from_le_bytes(ru[o..o + 8].try_into().unwrap_or([0; 8]));
        let micros = |o: usize| {
            word(o) * 1_000_000 + i64::from(i32::from_le_bytes(ru[o + 8..o + 12].try_into().unwrap_or([0; 4])))
        };
        return (micros(0) + micros(16)) as u64;
    }
    // SAFETY: one use per call.
    let start = unsafe { STARTED.get() };
    let now = now_us();
    if *start == 0 {
        *start = now;
    }
    now - *start
}

static UTC: &[u8] = b"UTC\0";
static TM: crate::Global<Tm> = crate::Global::new(Tm {
    sec: 0,
    min: 0,
    hour: 0,
    mday: 0,
    mon: 0,
    year: 0,
    wday: 0,
    yday: 0,
    isdst: 0,
    gmtoff: 0,
    zone: core::ptr::null(),
});

#[unsafe(no_mangle)]
unsafe extern "C" fn gmtime_r(t: *const i64, out: *mut Tm) -> *mut Tm {
    // SAFETY: the C contract.
    unsafe {
        match ctime::gmtime(*t) {
            Some(tm) => {
                *out = Tm { zone: UTC.as_ptr(), ..tm };
                out
            }
            None => {
                crate::set_errno(EOVERFLOW);
                core::ptr::null_mut()
            }
        }
    }
}

/// Local time is UTC until Poly has a time zone database.
#[unsafe(no_mangle)]
unsafe extern "C" fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm {
    // SAFETY: the C contract.
    unsafe { gmtime_r(t, out) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn gmtime(t: *const i64) -> *mut Tm {
    // SAFETY: the C contract; the shared result as C allows.
    unsafe { gmtime_r(t, TM.get()) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn localtime(t: *const i64) -> *mut Tm {
    // SAFETY: as above.
    unsafe { gmtime_r(t, TM.get()) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn timegm(tm: *mut Tm) -> i64 {
    // SAFETY: the C contract.
    unsafe {
        let r = ctime::timegm(&mut *tm);
        (*tm).zone = UTC.as_ptr();
        r.unwrap_or_else(|| {
            crate::set_errno(EOVERFLOW);
            -1
        })
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn mktime(tm: *mut Tm) -> i64 {
    // SAFETY: the C contract.
    unsafe { timegm(tm) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strftime(buf: *mut u8, n: usize, fmt: *const u8, tm: *const Tm) -> usize {
    // SAFETY: the C contract: `buf` holds `n` bytes, a format and a tm.
    unsafe { ctime::strftime(core::slice::from_raw_parts_mut(buf, n), bytes(fmt), &*tm, b"UTC") }
}

/// `nanosleep`: a wait on a word nobody wakes, with the time as its
/// timeout (`__ulock_wait2`, UL_COMPARE_AND_WAIT).
#[unsafe(no_mangle)]
unsafe extern "C" fn nanosleep(req: *const [i64; 2], rem: *mut [i64; 2]) -> i32 {
    // SAFETY: the C contract: a struct timespec.
    let (sec, nsec) = unsafe { ((*req)[0], (*req)[1]) };
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        crate::set_errno(EINVAL);
        return -1;
    }
    let ns = (sec as u64).saturating_mul(1_000_000_000).saturating_add(nsec as u64);
    if ns > 0 {
        let word = 0u32;
        // SAFETY: our own word; the call returns at the timeout.
        unsafe { __ulock_wait2(1, &word, 0, ns, 0) };
    }
    if !rem.is_null() {
        // SAFETY: the C contract.
        unsafe { *rem = [0, 0] };
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn usleep(us: u32) -> i32 {
    let req = [i64::from(us / 1_000_000), i64::from(us % 1_000_000) * 1000];
    // SAFETY: our own timespec.
    unsafe { nanosleep(&req, core::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn sleep(s: u32) -> u32 {
    let req = [i64::from(s), 0];
    // SAFETY: our own timespec.
    unsafe { nanosleep(&req, core::ptr::null_mut()) };
    0
}

// ------------------------------------------------------------- locale

#[unsafe(no_mangle)]
unsafe extern "C" fn setlocale(_category: i32, locale: *const u8) -> *const u8 {
    // SAFETY: the C contract: null or a string.
    unsafe {
        if locale.is_null() || *locale == 0 {
            return c"C".as_ptr() as *const u8;
        }
        match bytes(locale) {
            b"C" | b"POSIX" => c"C".as_ptr() as *const u8,
            _ => core::ptr::null(),
        }
    }
}

/// `struct lconv` (`<_locale.h>`).
#[repr(C)]
pub struct Lconv {
    strings: [*const u8; 10],
    chars: [u8; 14],
}

// SAFETY: read-only data.
unsafe impl Sync for Lconv {}

static LCONV: Lconv = Lconv {
    strings: [
        c".".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
        c"".as_ptr() as *const u8,
    ],
    // CHAR_MAX: "not available in this locale".
    chars: [127; 14],
};

#[unsafe(no_mangle)]
extern "C" fn localeconv() -> *const Lconv {
    &LCONV
}

// ------------------------------------------------------------- system

/// `sysconf` (`<unistd.h>` names).
#[unsafe(no_mangle)]
extern "C" fn sysconf(name: i32) -> i64 {
    match name {
        1 => 1 << 20,    // _SC_ARG_MAX
        3 => 100,        // _SC_CLK_TCK
        5 => 256,        // _SC_OPEN_MAX
        29 => PAGE_SIZE, // _SC_PAGESIZE
        57 | 58 => 1,    // _SC_NPROCESSORS_CONF, _ONLN
        200 => 65_536,   // _SC_PHYS_PAGES
        _ => {
            crate::set_errno(EINVAL);
            -1
        }
    }
}

#[cfg(target_arch = "x86_64")]
const PAGE_SIZE: i64 = 4096;
#[cfg(target_arch = "aarch64")]
const PAGE_SIZE: i64 = 16384;

#[unsafe(no_mangle)]
extern "C" fn getpagesize() -> i32 {
    PAGE_SIZE as i32
}

/// `confstr` (`<unistd.h>` names): the length the value needs with its NUL.
#[unsafe(no_mangle)]
unsafe extern "C" fn confstr(name: i32, buf: *mut u8, len: usize) -> usize {
    let value: &[u8] = match name {
        1 => b"/usr/bin:/bin:/usr/sbin:/sbin", // _CS_PATH
        65537 | 65538 => b"/tmp/",             // _CS_DARWIN_USER_TEMP_DIR, _CACHE_DIR
        _ => {
            crate::set_errno(EINVAL);
            return 0;
        }
    };
    if !buf.is_null() && len > 0 {
        let n = value.len().min(len - 1);
        // SAFETY: the C contract: `buf` holds `len` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(value.as_ptr(), buf, n);
            *buf.add(n) = 0;
        }
    }
    value.len() + 1
}

#[unsafe(no_mangle)]
unsafe extern "C" fn sysctlbyname(
    name: *const u8,
    old: *mut u8,
    oldlen: *mut usize,
    new: *const u8,
    newlen: usize,
) -> i32 {
    // SAFETY: the C contract; the kernel checks the pointers.
    unsafe { __sysctlbyname(name, cstr::strlen(name), old, oldlen, new, newlen) }
}

/// `isatty`: whether `TIOCGETA` (`<sys/ttycom.h>`) reads a terminal's
/// settings.
#[unsafe(no_mangle)]
extern "C" fn isatty(fd: i32) -> i32 {
    let mut termios = [0u8; 72];
    // SAFETY: a struct termios is 72 bytes on 64-bit Darwin.
    if unsafe { ioctl(fd, 0x4048_7413, termios.as_mut_ptr()) } == 0 {
        return 1;
    }
    if crate::errno() != EBADF {
        crate::set_errno(ENOTTY);
    }
    0
}

const ENOTTY: i32 = 25;

/// `__assert_rtn(function, file, line, expression)` (`<assert.h>`): the
/// message of `assert(3)`, then `abort`.
#[unsafe(no_mangle)]
unsafe extern "C" fn __assert_rtn(func: *const u8, file: *const u8, line: i32, expr: *const u8) -> ! {
    let mut msg = [0u8; 512];
    let mut sink = printf::Buffer { buf: &mut msg[..511], len: 0 };
    let nothing: [u64; 0] = [];
    // SAFETY: four strings or nulls from the macro; the format is ours.
    unsafe {
        let parts: [&[u8]; 9] = [
            b"Assertion failed: (",
            if expr.is_null() { b"?" } else { bytes(expr) },
            b"), function ",
            if func.is_null() { b"?" } else { bytes(func) },
            b", file ",
            if file.is_null() { b"?" } else { bytes(file) },
            b", line ",
            b"",
            b"",
        ];
        for p in &parts[..7] {
            sink.put(p);
        }
        let _ = printf::format(b"%d.\n", &mut printf::Slots { ints: &[line as u64], doubles: &[] }, &mut sink);
        let _ = nothing;
        let n = sink.len;
        flush_all();
        write(2, msg.as_ptr(), n);
        abort()
    }
}

unsafe extern "C" {
    fn abort() -> !;
}

// ------------------------------------------------------------ signals

/// The kernel's `struct __sigaction` (the sigaction sheet: handler,
/// trampoline, mask, flags; 24 bytes).
#[repr(C)]
struct KernelAction {
    handler: usize,
    tramp: usize,
    mask: u32,
    flags: i32,
}

/// `struct sigaction` (`<sys/signal.h>`: handler, mask, flags; 16 bytes).
#[repr(C)]
pub struct Action {
    handler: usize,
    mask: u32,
    flags: i32,
}

// The trampoline the kernel (or the translator) enters a handler through:
// (handler, info style, signal, siginfo, ucontext); it calls
// handler(signal, siginfo, ucontext), then sigreturn(ucontext, info style)
// (BSD 184), which never returns.
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".p2align 4",
    "_poly_sigtramp:",
    "    mov rbx, r8",
    "    mov r12, rsi",
    "    mov rax, rdi",
    "    mov edi, edx",
    "    mov rsi, rcx",
    "    mov rdx, r8",
    "    call rax",
    "    mov rdi, rbx",
    "    mov esi, r12d",
    "    mov eax, 0x2000000 + 184",
    "    syscall",
    "    ud2",
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".p2align 2",
    "_poly_sigtramp:",
    "    mov x19, x4",
    "    mov x20, x1",
    "    mov x9, x0",
    "    mov w0, w2",
    "    mov x1, x3",
    "    mov x2, x4",
    "    blr x9",
    "    mov x0, x19",
    "    mov w1, w20",
    "    mov x16, #184",
    "    svc #0x80",
    "    brk #1",
);

unsafe extern "C" {
    fn poly_sigtramp();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn sigaction(sig: i32, new: *const Action, old: *mut Action) -> i32 {
    // SAFETY: the C contract: null or a struct sigaction each.
    unsafe {
        if new.is_null() {
            return __sigaction(sig, core::ptr::null(), old);
        }
        let k = KernelAction {
            handler: (*new).handler,
            tramp: poly_sigtramp as *const () as usize,
            mask: (*new).mask,
            flags: (*new).flags,
        };
        __sigaction(sig, &k, old)
    }
}

/// `signal`: `sigaction` with `SA_RESTART` (2), the old handler back
/// (`SIG_ERR`, -1, on failure).
#[unsafe(no_mangle)]
unsafe extern "C" fn signal(sig: i32, handler: usize) -> usize {
    let new = Action { handler, mask: 0, flags: 2 };
    let mut old = Action { handler: 0, mask: 0, flags: 0 };
    // SAFETY: our own structures.
    if unsafe { sigaction(sig, &new, &mut old) } != 0 {
        return usize::MAX;
    }
    old.handler
}

#[unsafe(no_mangle)]
extern "C" fn raise(sig: i32) -> i32 {
    // SAFETY: plain calls.
    unsafe { kill(getpid(), sig) }
}
