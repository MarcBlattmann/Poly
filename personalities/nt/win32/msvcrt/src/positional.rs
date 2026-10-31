//! `msvcrt.dll`'s positional-parameter `printf` functions (ADR 0092;
//! Learn, "printf_p Positional Parameters"): the classic options plus
//! `poly_crt::printf::POSITIONAL`. `_sprintf_p` is `sprintf_s` with
//! positional parameters (Learn), so it follows the secure buffer rule;
//! `_scprintf_p_l` counts like `_scprintf`. The `_l` forms ignore their
//! locale (only "C", ADR 0084). Also `frexpf`, which only `msvcrt.dll`
//! exports of the slice.

use super::{LEGACY_N, OPT};
use poly_crt::printf::{POSITIONAL, VaList};
use poly_win32_crt::stdio::{self, BufferRule, File};

const P: u64 = OPT | POSITIONAL;
const P_COUNT: u64 = LEGACY_N | POSITIONAL;

/// The `va_list` forms the thunks call (and the `_v*` exports wrap).
pub mod va {
    use super::*;

    va_fns! {
        fprintf_p(s: *mut File, f: *const u8) -> i32 = |va: &mut VaList| stdio::vfprintf(s, f, va, P);
        fprintf_p_l(s: *mut File, f: *const u8, _l: *mut u8) -> i32 = |va: &mut VaList| stdio::vfprintf(s, f, va, P);
        fwprintf_p(s: *mut File, f: *const u16) -> i32 = |va: &mut VaList| stdio::vfprintf(s, f, va, P);
        fwprintf_p_l(s: *mut File, f: *const u16, _l: *mut u8) -> i32 =
            |va: &mut VaList| stdio::vfprintf(s, f, va, P);
        scprintf_p_l(f: *const u8, _l: *mut u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(core::ptr::null_mut::<u8>(), 0, f, va, P_COUNT, BufferRule::Options);
        scwprintf_p_l(f: *const u16, _l: *mut u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(core::ptr::null_mut::<u16>(), 0, f, va, P_COUNT, BufferRule::Options);
        sprintf_p_l(b: *mut u8, n: usize, f: *const u8, _l: *mut u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, P, BufferRule::Secure);
        swprintf_p_l(b: *mut u16, n: usize, f: *const u16, _l: *mut u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, P, BufferRule::Secure);
        sprintf_p(b: *mut u8, n: usize, f: *const u8) -> i32 =
            |va: &mut VaList| stdio::vsprintf(b, n, f, va, P, BufferRule::Secure);
    }
}

thunk!("_fprintf_p", 2, va::fprintf_p);
thunk!("_fprintf_p_l", 3, va::fprintf_p_l);
thunk!("_fwprintf_p", 2, va::fwprintf_p);
thunk!("_fwprintf_p_l", 3, va::fwprintf_p_l);
thunk!("_scprintf_p_l", 2, va::scprintf_p_l);
thunk!("_scwprintf_p_l", 2, va::scwprintf_p_l);
thunk!("_sprintf_p_l", 4, va::sprintf_p_l);
thunk!("_swprintf_p_l", 4, va::swprintf_p_l);

v_exports! {
    _vfprintf_p = va::fprintf_p, (s: *mut File, f: *const u8);
    _vfprintf_p_l = va::fprintf_p_l, (s: *mut File, f: *const u8, l: *mut u8);
    _vfwprintf_p = va::fwprintf_p, (s: *mut File, f: *const u16);
    _vfwprintf_p_l = va::fwprintf_p_l, (s: *mut File, f: *const u16, l: *mut u8);
    _vscprintf_p_l = va::scprintf_p_l, (f: *const u8, l: *mut u8);
    _vscwprintf_p_l = va::scwprintf_p_l, (f: *const u16, l: *mut u8);
    _vsprintf_p = va::sprintf_p, (b: *mut u8, n: usize, f: *const u8);
    _vsprintf_p_l = va::sprintf_p_l, (b: *mut u8, n: usize, f: *const u8, l: *mut u8);
    _vswprintf_p_l = va::swprintf_p_l, (b: *mut u16, n: usize, f: *const u16, l: *mut u8);
}

/// `frexpf`.
///
/// # Safety
/// `exp` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn frexpf(x: f32, exp: *mut i32) -> f32 {
    // SAFETY: the caller's pointer.
    unsafe { poly_win32_crt::mathfn::frexpf(x, exp) }
}
