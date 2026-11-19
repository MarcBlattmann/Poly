#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, unused_imports, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type HANDLE = *mut c_void;
type DWORD = u32;

/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; no characters outside the BMP).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<700> {
    let mut b = Buf::<700>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// The units of a NUL-terminated UTF-16 string at `p` (at most `max`).
fn wstr_at<'a>(p: *const u16, max: usize) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string the system returned; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// Whether two UTF-16 strings are equal without regard to ASCII case.
fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(&x, &y)| {
            let l = |c: u16| if (0x41..=0x5a).contains(&c) { c + 32 } else { c };
            l(x) == l(y)
        })
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
}

fn gle() -> i64 {
    // SAFETY: plain call.
    i64::from(unsafe { GetLastError() })
}

fn clear_error() {
    // SAFETY: plain call.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a failed call, else -2 (success leaves it unspecified).
fn gle_if(ok: BOOL) -> i64 {
    if ok == 0 { gle() } else { -2 }
}

// ---- group L helpers ----

#[repr(C)]
#[derive(Clone, Copy)]
struct ST {
    year: u16,
    month: u16,
    dow: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    ms: u16,
}

const fn st(year: u16, month: u16, dow: u16, day: u16, hour: u16, minute: u16, second: u16, ms: u16) -> ST {
    ST { year, month, dow, day, hour, minute, second, ms }
}

// The fixed instants used by the tests (SYSTEMTIME of Learn: year, month, day of week, day, hour, minute, second, ms).
const S_MORNING: ST = st(2024, 3, 2, 5, 7, 8, 9, 10); // Tuesday 2024-03-05 07:08:09.010
const S_NIGHT: ST = st(2023, 12, 0, 31, 23, 59, 58, 999); // Sunday 2023-12-31 23:59:58.999
const S_LEAP: ST = st(2024, 2, 4, 29, 12, 0, 0, 0); // Thursday 2024-02-29 12:00:00 (noon)
const S_MIDNIGHT: ST = st(2024, 7, 0, 14, 0, 0, 0, 0); // Sunday 2024-07-14 00:00:00
const S_EPOCH: ST = st(1601, 1, 1, 1, 0, 5, 0, 0); // Monday 1601-01-01 00:05:00 (first valid year)
const S_AFTERNOON: ST = st(2025, 11, 3, 5, 15, 30, 45, 123); // Wednesday 2025-11-05 15:30:45.123

/// The text of a NUL-terminated (or `n`-long) UTF-16 buffer as printable text.
fn txt(b: &[u16]) -> Buf<700> {
    let mut n = 0;
    while n < b.len() && b[n] != 0 {
        n += 1;
    }
    units_text(&b[..n])
}

/// The length of the NUL-terminated string in `b` (units before the first NUL), or `b.len()` when none.
fn ulen(b: &[u16]) -> usize {
    let mut n = 0;
    while n < b.len() && b[n] != 0 {
        n += 1;
    }
    n
}

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn EnumSystemLocalesEx(cb: extern "system" fn(*const u16, u32, isize) -> BOOL, flags: u32, lparam: isize, reserved: *const c_void) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

//@ use=b:EnumSystemLocalesEx,k:GetCurrentThreadId
poly_wtest::main!(test);

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

static COUNT: AtomicU32 = AtomicU32::new(0);
static STOP_AT: AtomicU32 = AtomicU32::new(0);
static MASK: AtomicU64 = AtomicU64::new(0);
static SORTED: AtomicBool = AtomicBool::new(true);
static BADFMT: AtomicU32 = AtomicU32::new(0);
static BADTHREAD: AtomicU32 = AtomicU32::new(0);
static BADLPARAM: AtomicU32 = AtomicU32::new(0);
static TID: AtomicU32 = AtomicU32::new(0);
static PREVLEN: AtomicU32 = AtomicU32::new(0);
static PREV: [AtomicU32; 100] = [const { AtomicU32::new(0) }; 100];
static FSEEN: [AtomicU32; 8] = [const { AtomicU32::new(0xFFFF_FFFF) }; 8];
static FDISTINCT: AtomicU32 = AtomicU32::new(0);
static FORED: AtomicU32 = AtomicU32::new(0);
static EXPECT_LPARAM: AtomicU32 = AtomicU32::new(0);
static ASCII_ONLY: AtomicBool = AtomicBool::new(true);
static NONEMPTY_SEEN: AtomicU32 = AtomicU32::new(0);
static PREVNUM: AtomicU64 = AtomicU64::new(0);
static NUMSORTED: AtomicBool = AtomicBool::new(true);

fn reset(stop_at: u32, lparam: u32) {
    COUNT.store(0, Relaxed);
    STOP_AT.store(stop_at, Relaxed);
    MASK.store(0, Relaxed);
    SORTED.store(true, Relaxed);
    BADFMT.store(0, Relaxed);
    BADTHREAD.store(0, Relaxed);
    BADLPARAM.store(0, Relaxed);
    PREVLEN.store(0, Relaxed);
    FDISTINCT.store(0, Relaxed);
    FORED.store(0, Relaxed);
    ASCII_ONLY.store(true, Relaxed);
    NONEMPTY_SEEN.store(0, Relaxed);
    PREVNUM.store(0, Relaxed);
    NUMSORTED.store(true, Relaxed);
    EXPECT_LPARAM.store(lparam, Relaxed);
    for f in FSEEN.iter() {
        f.store(0xFFFF_FFFF, Relaxed);
    }
    // SAFETY: plain call.
    TID.store(unsafe { GetCurrentThreadId() }, Relaxed);
}

fn lower(c: u16) -> u16 {
    if (0x41..=0x5a).contains(&c) { c + 32 } else { c }
}

/// Bookkeeping common to every callback call: order (ascending, ASCII case-insensitive), wanted names, thread.
fn note(w: &[u16], want: &[&str]) {
    let pl = PREVLEN.load(Relaxed) as usize;
    let mut cmp = 0i32; // prev vs cur
    let m = pl.min(w.len());
    let mut i = 0;
    while i < m {
        let a = lower(PREV[i].load(Relaxed) as u16);
        let b = lower(w[i]);
        if a != b {
            cmp = if a < b { -1 } else { 1 };
            break;
        }
        i += 1;
    }
    if cmp == 0 && pl > w.len() {
        cmp = 1;
    }
    if COUNT.load(Relaxed) > 1 && cmp > 0 {
        SORTED.store(false, Relaxed);
    }
    for (k, &c) in w.iter().take(100).enumerate() {
        PREV[k].store(u32::from(c), Relaxed);
        if c > 0x7e || c < 0x20 {
            ASCII_ONLY.store(false, Relaxed);
        }
    }
    PREVLEN.store(w.len().min(100) as u32, Relaxed);
    if !w.is_empty() {
        NONEMPTY_SEEN.fetch_add(1, Relaxed);
    }
    for (k, name) in want.iter().enumerate() {
        if name.len() == w.len() && name.bytes().zip(w.iter()).all(|(a, &b)| u16::from(a) == b) {
            MASK.fetch_or(1 << k, Relaxed);
        }
    }
    // SAFETY: plain call.
    if TID.load(Relaxed) != unsafe { GetCurrentThreadId() } {
        BADTHREAD.fetch_add(1, Relaxed);
    }
}

fn present(want: &[&str]) -> Buf<700> {
    let mut b = Buf::<700>::new();
    let m = MASK.load(Relaxed);
    for (k, name) in want.iter().enumerate() {
        if m & (1 << k) != 0 {
            let _ = write!(b, "[{name}]");
        }
    }
    b
}

fn buckets() -> Buf<80> {
    let n = COUNT.load(Relaxed);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "n>=1:{} n>=10:{} n>=100:{} n>=300:{}", n >= 1, n >= 10, n >= 100, n >= 300);
    b
}

const WANT: [&str; 18] = [
    "en-US", "en", "de-CH", "de", "fr-FR", "", "de-DE", "fr", "en-GB", "ja-JP", "zh-CN", "zh-Hans", "de-DE_phoneb", "ja-JP_radstr", "qps-ploc", "!x-sys-default-locale", "en-US_tradnl", "zh-Hans-CN",
];

extern "system" fn cb(s: *const u16, flags: u32, lparam: isize) -> BOOL {
    let n = COUNT.fetch_add(1, Relaxed) + 1;
    let w = wstr_at(s, 99);
    note(w, &WANT);
    if lparam as u32 != EXPECT_LPARAM.load(Relaxed) {
        BADLPARAM.fetch_add(1, Relaxed);
    }
    FORED.fetch_or(flags, Relaxed);
    let d = FDISTINCT.load(Relaxed) as usize;
    if !FSEEN[..d.min(8)].iter().any(|f| f.load(Relaxed) == flags) && d < 8 {
        FSEEN[d].store(flags, Relaxed);
        FDISTINCT.store(d as u32 + 1, Relaxed);
    }
    let stop = STOP_AT.load(Relaxed);
    if stop != 0 && n >= stop { 0 } else { 1 }
}

fn fseen() -> Buf<120> {
    let mut b = Buf::<120>::new();
    let d = (FDISTINCT.load(Relaxed) as usize).min(8);
    let mut v = [0u32; 8];
    for i in 0..d {
        v[i] = FSEEN[i].load(Relaxed);
    }
    v[..d].sort_unstable();
    for x in &v[..d] {
        let _ = write!(b, "{x:#x} ");
    }
    b
}

fn run(t: &mut Test, label: &str, flags: u32, stop_at: u32, lparam: u32) {
    reset(stop_at, lparam);
    clear_error();
    // SAFETY: a callback with the signature of Learn's LOCALE_ENUMPROCEX.
    let r = unsafe { EnumSystemLocalesEx(cb, flags, lparam as isize, core::ptr::null()) };
    let e = if r == 0 { gle() } else { -2 };
    say!(
        t,
        "{label}: ret={r} err={e} {} sorted={} ascii={} other_thread={} lparam_wrong={} callback_flags={} present={}",
        buckets(),
        SORTED.load(Relaxed),
        ASCII_ONLY.load(Relaxed),
        BADTHREAD.load(Relaxed),
        BADLPARAM.load(Relaxed),
        fseen(),
        present(&WANT)
    );
}

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    #[link_name = "EnumSystemLocalesEx"]
    fn EnumSystemLocalesEx_raw(cb: *const c_void, flags: u32, lparam: isize, reserved: *const c_void) -> BOOL;
}

fn test(t: &mut Test) {
    for stop in [1u32, 2, 3, 10] {
        reset(stop, 7);
        clear_error();
        // SAFETY: valid callback.
        let r = unsafe { EnumSystemLocalesEx(cb, 1, 7, core::ptr::null()) };
        say!(t, "stop after {stop} (WINDOWS): ret={r} err={} calls={} lparam_wrong={}", gle_if(r), COUNT.load(Relaxed), BADLPARAM.load(Relaxed));
    }
    // lparam is passed through unchanged
    for lp in [0u32, 1, 0x1234_5678, 0xFFFF_FFFF] {
        reset(3, lp);
        // SAFETY: valid callback.
        let r = unsafe { EnumSystemLocalesEx(cb, 1, lp as isize, core::ptr::null()) };
        say!(t, "lparam {lp:#x}: ret={r} lparam_wrong={} calls={}", BADLPARAM.load(Relaxed), COUNT.load(Relaxed));
    }
    // a 64-bit lparam survives whole
    {
        reset(1, 0);
        static BIG: AtomicU64 = AtomicU64::new(0);
        extern "system" fn big(_s: *const u16, _f: u32, lp: isize) -> BOOL {
            BIG.store(lp as u64, Relaxed);
            0
        }
        // SAFETY: valid callback.
        let r = unsafe { EnumSystemLocalesEx(big, 1, 0x1122_3344_5566_7788u64 as isize, core::ptr::null()) };
        say!(t, "64-bit lparam intact: {} (ret={r})", BIG.load(Relaxed) == 0x1122_3344_5566_7788);
    }
    // reserved must be NULL
    {
        let x = 0u8;
        reset(0, 0);
        clear_error();
        // SAFETY: valid callback; the reserved pointer is not used when it is refused.
        let r = unsafe { EnumSystemLocalesEx(cb, 1, 0, &x as *const u8 as *const c_void) };
        say!(t, "reserved non-NULL: ret={r} err={} calls>0={}", gle_if(r), COUNT.load(Relaxed) > 0);
    }
    // flags that name nothing
    for f in [0u32, 0x40, 0xFFFF_FFFF] {
        reset(0, 0);
        clear_error();
        // SAFETY: valid callback.
        let r = unsafe { EnumSystemLocalesEx(cb, f, 0, core::ptr::null()) };
        say!(t, "flags {f:#x}: ret={r} err={} calls>0={}", gle_if(r), COUNT.load(Relaxed) > 0);
    }
    // NULL callback
    for f in [1u32, 0] {
        clear_error();
        // SAFETY: deliberately invalid callback pointer; the call must fail before using it.
        let r = unsafe { EnumSystemLocalesEx_raw(core::ptr::null(), f, 0, core::ptr::null()) };
        say!(t, "NULL callback flags {f}: ret={r} err={}", gle_if(r));
    }
    reset(0, 0);
    // SAFETY: valid callback.
    unsafe { EnumSystemLocalesEx(cb, 0x31, 0, core::ptr::null()) };
    let a = COUNT.load(Relaxed);
    let am = MASK.load(Relaxed);
    reset(0, 0);
    // SAFETY: valid callback.
    unsafe { EnumSystemLocalesEx(cb, 0x31, 0, core::ptr::null()) };
    say!(t, "two enumerations agree: {}", a == COUNT.load(Relaxed) && am == MASK.load(Relaxed));
}
