//! Utilities (C11 7.22.5, 7.22.6, 7.22.2; Learn's pages): `qsort`,
//! `bsearch`, `abs`, `div`, `rand`; locales ("C" only); `errno` access; the
//! invalid parameter and pure-call handlers; the floating-point control
//! word; the exact `math.h` functions.

use crate::global;
use crate::startup;
use core::sync::atomic::{AtomicI32, Ordering};
use poly_crt::errno::{EINVAL, ERANGE};
use poly_crt::{fenv, math, sort};

// ----- sorting and searching ---------------------------------------------------------

type Cmp = unsafe extern "C" fn(*const u8, *const u8) -> i32;
type CmpS = unsafe extern "C" fn(*mut u8, *const u8, *const u8) -> i32;

/// # Safety
/// `base` holds `n × size` bytes; `cmp` compares two of them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort(base: *mut u8, n: usize, size: usize, cmp: Option<Cmp>) {
    let Some(cmp) = cmp else {
        global::invalid(EINVAL);
        return;
    };
    if base.is_null() && n != 0 {
        global::invalid(EINVAL);
        return;
    }
    // SAFETY: the program's comparison of two elements.
    let mut c = |a: *const u8, b: *const u8| unsafe { cmp(a, b) };
    // SAFETY: the caller's array.
    unsafe { sort::qsort(base, n, size, &mut c) };
}

/// # Safety
/// As `qsort`, with a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort_s(base: *mut u8, n: usize, size: usize, cmp: Option<CmpS>, ctx: *mut u8) {
    let Some(cmp) = cmp else {
        global::invalid(EINVAL);
        return;
    };
    // SAFETY: the program's comparison.
    let mut c = |a: *const u8, b: *const u8| unsafe { cmp(ctx, a, b) };
    // SAFETY: the caller's array.
    unsafe { sort::qsort(base, n, size, &mut c) };
}

/// # Safety
/// `base` holds `n × size` sorted bytes; `cmp(key, element)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsearch(key: *const u8, base: *const u8, n: usize, size: usize, cmp: Option<Cmp>) -> *mut u8 {
    let Some(cmp) = cmp else {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    };
    // SAFETY: the program's comparison.
    let mut c = |e: *const u8| unsafe { cmp(key, e) };
    // SAFETY: the caller's array.
    unsafe { sort::bsearch(base, n, size, &mut c) }.map_or(core::ptr::null_mut(), |p| p as *mut u8)
}

/// # Safety
/// As `bsearch`, with a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsearch_s(
    key: *const u8,
    base: *const u8,
    n: usize,
    size: usize,
    cmp: Option<CmpS>,
    ctx: *mut u8,
) -> *mut u8 {
    let Some(cmp) = cmp else {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    };
    // SAFETY: the program's comparison.
    let mut c = |e: *const u8| unsafe { cmp(ctx, key, e) };
    // SAFETY: the caller's array.
    unsafe { sort::bsearch(base, n, size, &mut c) }.map_or(core::ptr::null_mut(), |p| p as *mut u8)
}

// ----- integers ------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn abs(v: i32) -> i32 {
    v.wrapping_abs()
}

#[unsafe(no_mangle)]
pub extern "C" fn labs(v: i32) -> i32 {
    v.wrapping_abs()
}

pub extern "C" fn llabs(v: i64) -> i64 {
    v.wrapping_abs()
}

#[unsafe(no_mangle)]
pub extern "C" fn _abs64(v: i64) -> i64 {
    v.wrapping_abs()
}

/// `div_t` (`stdlib.h`): quotient, remainder.
#[repr(C)]
pub struct Div {
    quot: i32,
    rem: i32,
}

/// `lldiv_t`.
#[repr(C)]
pub struct LlDiv {
    quot: i64,
    rem: i64,
}

#[unsafe(no_mangle)]
pub extern "C" fn div(a: i32, b: i32) -> Div {
    Div { quot: a.wrapping_div(b), rem: a.wrapping_rem(b) }
}

#[unsafe(no_mangle)]
pub extern "C" fn ldiv(a: i32, b: i32) -> Div {
    div(a, b)
}

pub extern "C" fn lldiv(a: i64, b: i64) -> LlDiv {
    LlDiv { quot: a.wrapping_div(b), rem: a.wrapping_rem(b) }
}

#[unsafe(no_mangle)]
pub extern "C" fn _rotl(v: u32, n: i32) -> u32 {
    v.rotate_left((n & 31) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn _rotr(v: u32, n: i32) -> u32 {
    v.rotate_right((n & 31) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn _lrotl(v: u32, n: i32) -> u32 {
    _rotl(v, n)
}

#[unsafe(no_mangle)]
pub extern "C" fn _lrotr(v: u32, n: i32) -> u32 {
    _rotr(v, n)
}

#[unsafe(no_mangle)]
pub extern "C" fn _rotl64(v: u64, n: i32) -> u64 {
    v.rotate_left((n & 63) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn _rotr64(v: u64, n: i32) -> u64 {
    v.rotate_right((n & 63) as u32)
}

pub extern "C" fn _byteswap_ushort(v: u16) -> u16 {
    v.swap_bytes()
}

pub extern "C" fn _byteswap_ulong(v: u32) -> u32 {
    v.swap_bytes()
}

pub extern "C" fn _byteswap_uint64(v: u64) -> u64 {
    v.swap_bytes()
}

/// `srand`: the thread's seed (Learn: `rand` "is thread safe ... each
/// thread has its own seed").
#[unsafe(no_mangle)]
pub extern "C" fn srand(seed: u32) {
    global::thread().rand = seed;
}

/// `rand`: 0 … `RAND_MAX` (32767). The generator is the linear
/// congruential one published for Microsoft's C runtime (Wikipedia,
/// "Linear congruential generator", parameters table: a = 214013,
/// c = 2531011, bits 30..16), to be confirmed by the spec agent.
#[unsafe(no_mangle)]
pub extern "C" fn rand() -> i32 {
    let t = global::thread();
    t.rand = t.rand.wrapping_mul(214_013).wrapping_add(2_531_011);
    ((t.rand >> 16) & 0x7FFF) as i32
}

// ----- errno and handlers ----------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn _errno() -> *mut i32 {
    &mut global::thread().errno
}

#[unsafe(no_mangle)]
pub extern "C" fn __doserrno() -> *mut u32 {
    &mut global::thread().doserrno
}

/// # Safety
/// `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _get_errno(out: *mut i32) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable.
    unsafe { *out = global::thread().errno };
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn _set_errno(v: i32) -> i32 {
    global::thread().errno = v;
    0
}

/// # Safety
/// `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _get_doserrno(out: *mut u32) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable.
    unsafe { *out = global::thread().doserrno };
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn _set_doserrno(v: u32) -> i32 {
    global::thread().doserrno = v;
    0
}

/// The handler setters' shared body: the previous handler.
pub fn set_invalid_parameter_handler(h: usize) -> usize {
    global::INVALID_PARAMETER.swap(h, Ordering::AcqRel)
}

pub fn get_invalid_parameter_handler() -> usize {
    global::INVALID_PARAMETER.load(Ordering::Acquire)
}

pub fn set_thread_invalid_parameter_handler(h: usize) -> usize {
    core::mem::replace(&mut global::thread().invalid_parameter, h)
}

pub fn get_thread_invalid_parameter_handler() -> usize {
    global::thread().invalid_parameter
}

pub fn set_purecall_handler(h: usize) -> usize {
    startup::PURECALL.swap(h, Ordering::AcqRel)
}

pub fn set_terminate(h: usize) -> usize {
    startup::TERMINATE.swap(h, Ordering::AcqRel)
}

static COUNT_OUTPUT: AtomicI32 = AtomicI32::new(0);

/// Whether `%n` is enabled.
pub fn count_output() -> bool {
    COUNT_OUTPUT.load(Ordering::Acquire) != 0
}

pub extern "C" fn _set_printf_count_output(on: i32) -> i32 {
    COUNT_OUTPUT.swap(i32::from(on != 0), Ordering::AcqRel)
}

pub extern "C" fn _get_printf_count_output() -> i32 {
    COUNT_OUTPUT.load(Ordering::Acquire)
}

static ERROR_MODE: AtomicI32 = AtomicI32::new(0);

/// `_set_error_mode`: `_OUT_TO_DEFAULT` (0), `_OUT_TO_STDERR` (1),
/// `_OUT_TO_MSGBOX` (2); `_REPORT_ERRMODE` (3) asks. The previous mode.
#[unsafe(no_mangle)]
pub extern "C" fn _set_error_mode(mode: i32) -> i32 {
    match mode {
        0..=2 => ERROR_MODE.swap(mode, Ordering::AcqRel),
        3 => ERROR_MODE.load(Ordering::Acquire),
        _ => {
            global::invalid(EINVAL);
            -1
        }
    }
}

/// The handler `__setusermatherr` installed (the math functions call it,
/// `mathfn::finish`).
pub(crate) static MATHERR: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// `__setusermatherr`: the program's `_matherr` (Learn, `_matherr`).
#[unsafe(no_mangle)]
pub extern "C" fn __setusermatherr(f: usize) {
    MATHERR.store(f, Ordering::Release);
}

/// `_set_fmode`: `_O_TEXT`, `_O_BINARY`, `_O_WTEXT`, `_O_U8TEXT`,
/// `_O_U16TEXT`.
#[unsafe(no_mangle)]
pub extern "C" fn _set_fmode(mode: i32) -> i32 {
    use poly_crt::fmode::{O_BINARY, O_TEXT, O_U8TEXT, O_U16TEXT, O_WTEXT};
    if ![O_TEXT, O_BINARY, O_WTEXT, O_U8TEXT, O_U16TEXT].contains(&mode) {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: a valid cell.
    unsafe { *startup::cells().fmode = mode };
    0
}

/// # Safety
/// `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _get_fmode(out: *mut i32) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable; a valid cell.
    unsafe { *out = startup::fmode() };
    0
}

// ----- the floating-point environment ---------------------------------------------------

fn mxcsr() -> u32 {
    let mut v: u32 = 0;
    // SAFETY: stmxcsr stores the register to the local.
    unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut v, options(nostack)) };
    v
}

fn set_mxcsr(v: u32) {
    // SAFETY: ldmxcsr of a value built from the current one.
    unsafe { core::arch::asm!("ldmxcsr [{}]", in(reg) &v, options(nostack)) };
}

#[unsafe(no_mangle)]
pub extern "C" fn _controlfp(new: u32, mask: u32) -> u32 {
    let m = fenv::apply(mxcsr(), new, mask);
    set_mxcsr(m);
    fenv::control(m)
}

#[unsafe(no_mangle)]
pub extern "C" fn _control87(new: u32, mask: u32) -> u32 {
    _controlfp(new, mask)
}

/// `_controlfp_s`: `_MCW_PC` and `_MCW_IC` are refused on x64.
///
/// # Safety
/// `current` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _controlfp_s(current: *mut u32, new: u32, mask: u32) -> i32 {
    if mask & (fenv::MCW_PC | fenv::MCW_IC) != 0 && new & mask & (fenv::MCW_PC | fenv::MCW_IC) != 0 {
        global::invalid(EINVAL);
        return EINVAL;
    }
    let cw = _controlfp(new, mask & !(fenv::MCW_PC | fenv::MCW_IC));
    if !current.is_null() {
        // SAFETY: writable.
        unsafe { *current = cw };
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn _clearfp() -> u32 {
    let m = mxcsr();
    set_mxcsr(fenv::clear(m));
    fenv::status(m)
}

#[unsafe(no_mangle)]
pub extern "C" fn _statusfp() -> u32 {
    fenv::status(mxcsr())
}

#[unsafe(no_mangle)]
pub extern "C" fn _fpreset() {
    set_mxcsr(fenv::MXCSR_DEFAULT);
}

// ----- math ----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn fabs(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & !(1 << 63))
}

/// `sqrt`: the SSE2 instruction, correctly rounded (IEEE 754).
#[unsafe(no_mangle)]
pub extern "C" fn sqrt(x: f64) -> f64 {
    let r: f64;
    // SAFETY: sqrtsd on registers.
    unsafe { core::arch::asm!("sqrtsd {0}, {1}", out(xmm_reg) r, in(xmm_reg) x, options(nomem, nostack, pure)) };
    if x < 0.0 {
        global::set_errno(poly_crt::errno::EDOM);
    }
    r
}

#[unsafe(no_mangle)]
pub extern "C" fn floor(x: f64) -> f64 {
    math::floor(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn ceil(x: f64) -> f64 {
    math::ceil(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn fmod(x: f64, y: f64) -> f64 {
    math::fmod(x, y)
}

/// # Safety
/// `int_part` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn modf(x: f64, int_part: *mut f64) -> f64 {
    let (f, i) = math::modf(x);
    if !int_part.is_null() {
        // SAFETY: writable.
        unsafe { *int_part = i };
    }
    f
}

/// # Safety
/// `exp` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn frexp(x: f64, exp: *mut i32) -> f64 {
    let (m, e) = math::frexp(x);
    if !exp.is_null() {
        // SAFETY: writable.
        unsafe { *exp = e };
    }
    m
}

#[unsafe(no_mangle)]
pub extern "C" fn ldexp(x: f64, n: i32) -> f64 {
    let r = math::ldexp(x, n);
    if r.is_infinite() && x.is_finite() {
        global::set_errno(ERANGE);
    }
    r
}

#[unsafe(no_mangle)]
pub extern "C" fn _copysign(x: f64, y: f64) -> f64 {
    math::copysign(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn _finite(x: f64) -> i32 {
    i32::from(x.is_finite())
}

#[unsafe(no_mangle)]
pub extern "C" fn _isnan(x: f64) -> i32 {
    i32::from(x.is_nan())
}
