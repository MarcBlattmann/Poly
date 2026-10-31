//! `msvcrt.dll`'s own locale data and functions (ADR 0094): the classic
//! exported variables a global locale change keeps current (`_pctype`,
//! `__mb_cur_max`, `_mbctype`, `__lc_codepage`, `__lc_collate_cp`,
//! `__lc_handle`), and the names only `msvcrt` has.

use poly_win32_crt::global::Global;
use poly_win32_crt::locale as lc;

/// `_mbctype[257]` (`mbctype.h`): the multibyte code page's table.
#[unsafe(no_mangle)]
pub static _mbctype: Global<[u8; 257]> = Global::new([0; 257]);
/// `__lc_codepage`, `__lc_collate_cp`, `__lc_handle[6]`.
#[unsafe(no_mangle)]
pub static __lc_codepage: Global<u32> = Global::new(0);
#[unsafe(no_mangle)]
pub static __lc_collate_cp: Global<u32> = Global::new(0);
#[unsafe(no_mangle)]
pub static __lc_handle: Global<[u32; 6]> = Global::new([0; 6]);

/// At attach: bind the variables (with `_pctype` and `__mb_cur_max` from
/// the crate root) and choose `msvcrt`'s `_locale_t` layouts.
pub fn bind(pctype: *mut *const u16, mb_cur_max: *mut i32) {
    lc::bind(
        lc::Cells {
            pctype,
            mb_cur_max,
            lc_codepage: __lc_codepage.get(),
            lc_collate_cp: __lc_collate_cp.get(),
            lc_handle: __lc_handle.get(),
            mbctype: _mbctype.get(),
            mbcasemap: core::ptr::null_mut(),
        },
        true,
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn ___lc_handle_func() -> *const u32 {
    lc::___lc_handle_func()
}

/// `__lconv_init`: nothing to prepare (the `lconv` is built with the
/// locale).
#[unsafe(no_mangle)]
pub extern "C" fn __lconv_init() -> i32 {
    0
}

/// `__p___mb_cur_max`.
#[unsafe(no_mangle)]
pub extern "C" fn __p___mb_cur_max() -> *mut i32 {
    super::__mb_cur_max.get()
}

/// # Safety
/// As `strcpy`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mbscpy(dst: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: the caller's buffers.
    unsafe { poly_win32_crt::mbstring::_mbscpy(dst, src) }
}

/// # Safety
/// As `strcat`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mbscat(dst: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: the caller's buffers.
    unsafe { poly_win32_crt::mbstring::_mbscat(dst, src) }
}
