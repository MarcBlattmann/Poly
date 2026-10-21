//! Time (C11 7.27; Learn: `time`, `clock`, `gmtime`, `localtime`, `mktime`,
//! `_mkgmtime`, `difftime`, `asctime`, `ctime`, `strftime`, `_ftime`,
//! `_tzset`, `_get_timezone`) on [`poly_crt::time`]. Poly's local time is
//! UTC (ADR 0079): no daylight saving, a bias of 0, the zone "UTC".
//! `gmtime`/`localtime` return the thread's own `struct tm`, `asctime`
//! and `ctime` the thread's own buffer.

use crate::global;
use crate::sys;
use poly_crt::errno::EINVAL;
use poly_crt::time::{self as t, MAX_TIME32, MAX_TIME64, Tm};

/// The time zone variables' values (`_timezone`, `_daylight`, `_dstbias`,
/// `_tzname`): UTC. `_dstbias` is Learn's default, −3600.
pub const TIMEZONE: i32 = 0;
pub const DAYLIGHT: i32 = 0;
pub const DSTBIAS: i32 = -3600;

pub fn now() -> i64 {
    let mut ft = 0u64;
    // SAFETY: `ft` writable.
    unsafe { sys::GetSystemTimeAsFileTime(&mut ft) };
    t::from_filetime(ft)
}

/// # Safety
/// `out` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _time64(out: *mut i64) -> i64 {
    let v = now();
    if !out.is_null() {
        // SAFETY: writable.
        unsafe { *out = v };
    }
    v
}

/// # Safety
/// As `_time64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _time32(out: *mut i32) -> i32 {
    let v = now();
    let v = if v > MAX_TIME32 { -1 } else { v as i32 };
    if !out.is_null() {
        // SAFETY: writable.
        unsafe { *out = v };
    }
    v
}

/// `clock`: milliseconds since the process started (Learn: "the elapsed
/// wall-clock time since the start of the process"; `CLOCKS_PER_SEC` is
/// 1000).
#[unsafe(no_mangle)]
pub extern "C" fn clock() -> i32 {
    let (mut now, mut freq) = (0i64, 0i64);
    // SAFETY: writable.
    unsafe {
        sys::QueryPerformanceCounter(&mut now);
        sys::QueryPerformanceFrequency(&mut freq);
    }
    if freq <= 0 {
        return -1;
    }
    let start = crate::startup::START.load(core::sync::atomic::Ordering::Acquire) as i64;
    ((now - start) as i128 * 1000 / freq as i128) as i32
}

/// The thread's `struct tm` for `time`, or NULL with `EINVAL`.
fn to_thread_tm(time: i64, max: i64) -> *mut Tm {
    match t::gmtime(time, max) {
        Some(tm) => {
            let th = global::thread();
            th.tm = tm;
            &mut th.tm
        }
        None => {
            global::invalid(EINVAL);
            core::ptr::null_mut()
        }
    }
}

/// # Safety
/// `time` is readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _gmtime64(time: *const i64) -> *mut Tm {
    if time.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: readable.
    to_thread_tm(unsafe { *time }, MAX_TIME64)
}

/// # Safety
/// As `_gmtime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _gmtime32(time: *const i32) -> *mut Tm {
    if time.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: readable.
    to_thread_tm(i64::from(unsafe { *time }), MAX_TIME32)
}

/// # Safety
/// As `_gmtime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _localtime64(time: *const i64) -> *mut Tm {
    // SAFETY: the caller's argument.
    unsafe { _gmtime64(time) }
}

/// # Safety
/// As `_gmtime32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _localtime32(time: *const i32) -> *mut Tm {
    // SAFETY: the caller's argument.
    unsafe { _gmtime32(time) }
}

/// `_gmtime64_s`: 0, or `EINVAL` with every field −1.
///
/// # Safety
/// `out` writable, `time` readable.
unsafe fn tm_s(out: *mut Tm, time: Option<i64>, max: i64) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    match time.and_then(|v| t::gmtime(v, max)) {
        Some(tm) => {
            // SAFETY: writable.
            unsafe { *out = tm };
            0
        }
        None => {
            // SAFETY: writable.
            unsafe {
                *out = Tm { sec: -1, min: -1, hour: -1, mday: -1, mon: -1, year: -1, wday: -1, yday: -1, isdst: -1 };
            }
            global::invalid(EINVAL);
            EINVAL
        }
    }
}

/// # Safety
/// `out` writable, `time` NULL or readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _gmtime64_s(out: *mut Tm, time: *const i64) -> i32 {
    // SAFETY: the caller's pointers.
    unsafe { tm_s(out, (!time.is_null()).then(|| *time), MAX_TIME64) }
}

/// # Safety
/// As `_gmtime64_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _gmtime32_s(out: *mut Tm, time: *const i32) -> i32 {
    // SAFETY: the caller's pointers.
    unsafe { tm_s(out, (!time.is_null()).then(|| i64::from(*time)), MAX_TIME32) }
}

/// # Safety
/// As `_gmtime64_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _localtime64_s(out: *mut Tm, time: *const i64) -> i32 {
    // SAFETY: the caller's pointers.
    unsafe { _gmtime64_s(out, time) }
}

/// # Safety
/// As `_gmtime64_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _localtime32_s(out: *mut Tm, time: *const i32) -> i32 {
    // SAFETY: the caller's pointers.
    unsafe { _gmtime32_s(out, time) }
}

/// # Safety
/// `tm` is a writable `struct tm`.
unsafe fn mk(tm: *mut Tm, max: i64) -> i64 {
    if tm.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's structure.
    match t::mktime(unsafe { &mut *tm }, max) {
        Some(v) => v,
        None => {
            global::set_errno(EINVAL);
            -1
        }
    }
}

/// # Safety
/// `tm` is a writable `struct tm`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mktime64(tm: *mut Tm) -> i64 {
    // SAFETY: the caller's structure.
    unsafe { mk(tm, MAX_TIME64) }
}

/// # Safety
/// As `_mktime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mktime32(tm: *mut Tm) -> i32 {
    // SAFETY: the caller's structure.
    unsafe { mk(tm, MAX_TIME32) as i32 }
}

/// # Safety
/// As `_mktime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mkgmtime64(tm: *mut Tm) -> i64 {
    // SAFETY: the caller's structure.
    unsafe { mk(tm, MAX_TIME64) }
}

/// # Safety
/// As `_mktime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mkgmtime32(tm: *mut Tm) -> i32 {
    // SAFETY: the caller's structure.
    unsafe { mk(tm, MAX_TIME32) as i32 }
}

#[unsafe(no_mangle)]
pub extern "C" fn _difftime64(a: i64, b: i64) -> f64 {
    (a - b) as f64
}

#[unsafe(no_mangle)]
pub extern "C" fn _difftime32(a: i32, b: i32) -> f64 {
    f64::from(a) - f64::from(b)
}

/// `asctime` into the thread's buffer.
fn asc(tm: &Tm) -> *mut u8 {
    match t::asctime(tm) {
        Some(s) => {
            let th = global::thread();
            th.asctime[..s.len()].copy_from_slice(&s);
            th.asctime[s.len()] = 0;
            th.asctime.as_mut_ptr()
        }
        None => {
            global::invalid(EINVAL);
            core::ptr::null_mut()
        }
    }
}

fn wasc(tm: &Tm) -> *mut u16 {
    match t::asctime(tm) {
        Some(s) => {
            let th = global::thread();
            for (i, &b) in s.iter().enumerate() {
                th.wasctime[i] = u16::from(b);
            }
            th.wasctime[s.len()] = 0;
            th.wasctime.as_mut_ptr()
        }
        None => {
            global::invalid(EINVAL);
            core::ptr::null_mut()
        }
    }
}

/// # Safety
/// `tm` is a valid `struct tm`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn asctime(tm: *const Tm) -> *mut u8 {
    if tm.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's structure.
    asc(unsafe { &*tm })
}

/// # Safety
/// As `asctime`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wasctime(tm: *const Tm) -> *mut u16 {
    if tm.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's structure.
    wasc(unsafe { &*tm })
}

/// # Safety
/// `buf` holds `size` bytes; `tm` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn asctime_s(buf: *mut u8, size: usize, tm: *const Tm) -> i32 {
    if buf.is_null() || tm.is_null() || size < 26 {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's structure.
    let Some(s) = t::asctime(unsafe { &*tm }) else {
        global::invalid(EINVAL);
        return EINVAL;
    };
    // SAFETY: 26 bytes fit.
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
        buf.add(s.len()).write(0);
    }
    0
}

fn ctime_of(time: Option<i64>, max: i64) -> Option<Tm> {
    match time.and_then(|v| t::gmtime(v, max)) {
        Some(tm) => Some(tm),
        None => {
            global::invalid(EINVAL);
            None
        }
    }
}

/// # Safety
/// `time` is readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ctime64(time: *const i64) -> *mut u8 {
    // SAFETY: the caller's pointer.
    ctime_of((!time.is_null()).then(|| unsafe { *time }), MAX_TIME64).map_or(core::ptr::null_mut(), |tm| asc(&tm))
}

/// # Safety
/// As `_ctime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ctime32(time: *const i32) -> *mut u8 {
    // SAFETY: the caller's pointer.
    ctime_of((!time.is_null()).then(|| i64::from(unsafe { *time })), MAX_TIME32)
        .map_or(core::ptr::null_mut(), |tm| asc(&tm))
}

/// # Safety
/// As `_ctime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wctime64(time: *const i64) -> *mut u16 {
    // SAFETY: the caller's pointer.
    ctime_of((!time.is_null()).then(|| unsafe { *time }), MAX_TIME64).map_or(core::ptr::null_mut(), |tm| wasc(&tm))
}

/// # Safety
/// As `_ctime64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wctime32(time: *const i32) -> *mut u16 {
    // SAFETY: the caller's pointer.
    ctime_of((!time.is_null()).then(|| i64::from(unsafe { *time })), MAX_TIME32)
        .map_or(core::ptr::null_mut(), |tm| wasc(&tm))
}

/// `strftime`: the units written without the NUL, or 0 when they and the
/// NUL do not fit (C11 7.27.3.5).
///
/// # Safety
/// `buf` holds `size` units; `fmt` a valid C string; `tm` valid.
unsafe fn ftime<C: poly_crt::Unit>(buf: *mut C, size: usize, fmt: *const C, tm: *const Tm) -> usize {
    if buf.is_null() || size == 0 || fmt.is_null() || tm.is_null() {
        global::invalid(EINVAL);
        return 0;
    }
    // SAFETY: the caller's arguments.
    let (f, tm) = unsafe { (poly_crt::unit::slice(fmt), &*tm) };
    let Some(out) = t::strftime(f, tm) else {
        // SAFETY: at least one unit.
        unsafe { buf.write(C::default()) };
        global::invalid(EINVAL);
        return 0;
    };
    if out.len() >= size {
        // SAFETY: as above.
        unsafe { buf.write(C::default()) };
        global::set_errno(poly_crt::errno::ERANGE);
        return 0;
    }
    // SAFETY: out.len() + 1 <= size.
    unsafe {
        core::ptr::copy_nonoverlapping(out.as_ptr(), buf, out.len());
        buf.add(out.len()).write(C::default());
    }
    out.len()
}

/// # Safety
/// As `ftime`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strftime(buf: *mut u8, size: usize, fmt: *const u8, tm: *const Tm) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { ftime(buf, size, fmt, tm) }
}

/// # Safety
/// As `ftime`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsftime(buf: *mut u16, size: usize, fmt: *const u16, tm: *const Tm) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { ftime(buf, size, fmt, tm) }
}

#[unsafe(no_mangle)]
pub extern "C" fn _tzset() {}

/// # Safety
/// `out` writable.
pub unsafe extern "C" fn _get_timezone(out: *mut i32) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable.
    unsafe { *out = TIMEZONE };
    0
}

/// # Safety
/// `out` writable.
pub unsafe extern "C" fn _get_daylight(out: *mut i32) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable.
    unsafe { *out = DAYLIGHT };
    0
}

/// # Safety
/// `out` writable.
pub unsafe extern "C" fn _get_dstbias(out: *mut i32) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable.
    unsafe { *out = DSTBIAS };
    0
}

/// `_get_tzname(&len, buf, size, index)`: the zone name (both are "UTC").
///
/// # Safety
/// `len` writable; `buf` NULL or `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _get_tzname(len: *mut usize, buf: *mut u8, size: usize, index: i32) -> i32 {
    let name = t::ZONE.as_bytes();
    if len.is_null() || !(0..=1).contains(&index) || (buf.is_null() && size != 0) {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable.
    unsafe { *len = name.len() + 1 };
    if buf.is_null() {
        return 0;
    }
    if size < name.len() + 1 {
        global::invalid(poly_crt::errno::ERANGE);
        return poly_crt::errno::ERANGE;
    }
    // SAFETY: the buffer holds the name and NUL.
    unsafe {
        core::ptr::copy_nonoverlapping(name.as_ptr(), buf, name.len());
        buf.add(name.len()).write(0);
    }
    0
}

/// `_ftime64(struct __timeb64 *)`: time (8), millitm (2), timezone (2, in
/// minutes), dstflag (2) (`sys/timeb.h`).
///
/// # Safety
/// `out` is a `struct __timeb64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ftime64(out: *mut u8) {
    let mut ft = 0u64;
    // SAFETY: `ft` writable.
    unsafe { sys::GetSystemTimeAsFileTime(&mut ft) };
    if out.is_null() {
        global::invalid(EINVAL);
        return;
    }
    let secs = t::from_filetime(ft);
    let ms = ((ft / 10_000) % 1000) as u16;
    // SAFETY: the caller's structure.
    unsafe {
        (out as *mut i64).write_unaligned(secs);
        (out.add(8) as *mut u16).write_unaligned(ms);
        (out.add(10) as *mut i16).write_unaligned(0);
        (out.add(12) as *mut i16).write_unaligned(0);
    }
}

/// `_ftime32(struct __timeb32 *)`: time (4), millitm, timezone, dstflag.
///
/// # Safety
/// `out` is a `struct __timeb32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ftime32(out: *mut u8) {
    let mut b = [0u8; 16];
    // SAFETY: a local structure.
    unsafe { _ftime64(b.as_mut_ptr()) };
    if out.is_null() {
        return;
    }
    let secs = i64::from_le_bytes(b[..8].try_into().unwrap_or_default());
    // SAFETY: the caller's structure.
    unsafe {
        (out as *mut i32).write_unaligned(secs as i32);
        core::ptr::copy_nonoverlapping(b.as_ptr().add(8), out.add(4), 6);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __daylight() -> *mut i32 {
    static V: crate::global::Global<i32> = crate::global::Global::new(DAYLIGHT);
    V.get()
}

#[unsafe(no_mangle)]
pub extern "C" fn __dstbias() -> *mut i32 {
    static V: crate::global::Global<i32> = crate::global::Global::new(DSTBIAS);
    V.get()
}
