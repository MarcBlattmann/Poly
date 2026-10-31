//! The locale functions only the UCRT exports (ADR 0094): the shared
//! runtime's (`poly_win32_crt::{locale, lctype, mbconv, mbstring}`), under
//! their names, and the UCRT's accessors of the multibyte tables.

use poly_win32_crt::lctype as lt;
use poly_win32_crt::locale::{self as lc, LocaleT};
use poly_win32_crt::mbconv as mc;
use poly_win32_crt::mbstring as ms;

type L = *const LocaleT;

forward! {
    _wcreate_locale(category: i32, name: *const u16) -> *mut LocaleT = lc::_wcreate_locale;
    ___lc_locale_name_func() -> *const *const u16 = lc::___lc_locale_name_func;
    _lock_locales() = lc::_lock_locales;
    _unlock_locales() = lc::_unlock_locales;
    isblank(c: i32) -> i32 = lt::isblank;
    _ispunct_l(c: i32, l: L) -> i32 = lt::_ispunct_l;
    _isblank_l(c: i32, l: L) -> i32 = lt::_isblank_l;
    _iswblank_l(c: u16, l: L) -> i32 = lt::_iswblank_l;
    __iswcsym(c: u16) -> i32 = lt::__iswcsym;
    __iswcsymf(c: u16) -> i32 = lt::__iswcsymf;
    _iswcsym_l(c: u16, l: L) -> i32 = lt::_iswcsym_l;
    _iswcsymf_l(c: u16, l: L) -> i32 = lt::_iswcsymf_l;
    wctype(name: *const u8) -> u16 = lt::wctype;
    wctrans(name: *const u8) -> u16 = lt::wctrans;
    towctrans(c: u16, t: u16) -> u16 = lt::towctrans;
    ___mb_cur_max_l_func(l: L) -> i32 = lt::___mb_cur_max_l_func;
    wcstold(s: *const u16, end: *mut *mut u16) -> f64 = lt::wcstold;
    _atoflt(out: *mut u8, s: *const u8) -> i32 = lt::_atoflt;
    wcstoimax(s: *const u16, end: *mut *mut u16, base: i32) -> i64 = lt::wcstoimax;
    wcstoumax(s: *const u16, end: *mut *mut u16, base: i32) -> u64 = lt::wcstoumax;
    _strftime_l(buf: *mut u8, size: usize, fmt: *const u8, tm: *const poly_crt::time::Tm, l: L) -> usize = lt::_strftime_l;
    _strtof_l(s: *const u8, end: *mut *mut u8, l: L) -> f32 = lt::_strtof_l;
    _strtold_l(s: *const u8, end: *mut *mut u8, l: L) -> f64 = lt::_strtold_l;
    _wcstof_l(s: *const u16, end: *mut *mut u16, l: L) -> f32 = lt::_wcstof_l;
    _wcstold_l(s: *const u16, end: *mut *mut u16, l: L) -> f64 = lt::_wcstold_l;
    _strtoll_l(s: *const u8, end: *mut *mut u8, base: i32, l: L) -> i64 = lt::_strtoll_l;
    _strtoull_l(s: *const u8, end: *mut *mut u8, base: i32, l: L) -> u64 = lt::_strtoull_l;
    _strtoimax_l(s: *const u8, end: *mut *mut u8, base: i32, l: L) -> i64 = lt::_strtoimax_l;
    _strtoumax_l(s: *const u8, end: *mut *mut u8, base: i32, l: L) -> u64 = lt::_strtoumax_l;
    _wcstoll_l(s: *const u16, end: *mut *mut u16, base: i32, l: L) -> i64 = lt::_wcstoll_l;
    _wcstoull_l(s: *const u16, end: *mut *mut u16, base: i32, l: L) -> u64 = lt::_wcstoull_l;
    _wcstoimax_l(s: *const u16, end: *mut *mut u16, base: i32, l: L) -> i64 = lt::_wcstoimax_l;
    _wcstoumax_l(s: *const u16, end: *mut *mut u16, base: i32, l: L) -> u64 = lt::_wcstoumax_l;
    _atoll_l(s: *const u8, l: L) -> i64 = lt::_atoll_l;
    _wtoll_l(s: *const u16, l: L) -> i64 = lt::_wtoll_l;
    _mbstrnlen(s: *const u8, max: usize) -> usize = mc::_mbstrnlen;
    _mbstrnlen_l(s: *const u8, max: usize, l: L) -> usize = mc::_mbstrnlen_l;
    _ismbbalnum_l(c: u32, l: L) -> i32 = ms::_ismbbalnum_l;
    _ismbbalpha_l(c: u32, l: L) -> i32 = ms::_ismbbalpha_l;
    _ismbbblank(c: u32) -> i32 = ms::_ismbbblank;
    _ismbbblank_l(c: u32, l: L) -> i32 = ms::_ismbbblank_l;
    _ismbbgraph_l(c: u32, l: L) -> i32 = ms::_ismbbgraph_l;
    _ismbbprint_l(c: u32, l: L) -> i32 = ms::_ismbbprint_l;
    _ismbbpunct_l(c: u32, l: L) -> i32 = ms::_ismbbpunct_l;
    _ismbbkalnum_l(c: u32, l: L) -> i32 = ms::_ismbbkalnum_l;
    _ismbbkprint_l(c: u32, l: L) -> i32 = ms::_ismbbkprint_l;
    _ismbbkpunct_l(c: u32, l: L) -> i32 = ms::_ismbbkpunct_l;
    _mbbtype_l(c: u8, previous: i32, l: L) -> i32 = ms::_mbbtype_l;
    _ismbcblank(c: u32) -> i32 = ms::_ismbcblank;
    _ismbcblank_l(c: u32, l: L) -> i32 = ms::_ismbcblank_l;
    _mbsninc_l(p: *const u8, n: usize, l: L) -> *const u8 = ms::_mbsninc_l;
    _mbsstr_l(s: *const u8, needle: *const u8, l: L) -> *const u8 = ms::_mbsstr_l;
    _mbsncoll_l(a: *const u8, b: *const u8, n: usize, l: L) -> i32 = ms::_mbsncoll_l;
    _mbsnicoll_l(a: *const u8, b: *const u8, n: usize, l: L) -> i32 = ms::_mbsnicoll_l;
    _mbsset_s(s: *mut u8, size: usize, c: u32) -> i32 = ms::_mbsset_s;
    _mbsset_s_l(s: *mut u8, size: usize, c: u32, l: L) -> i32 = ms::_mbsset_s_l;
    _mbsnset_s(s: *mut u8, size: usize, c: u32, n: usize) -> i32 = ms::_mbsnset_s;
    _mbsnset_s_l(s: *mut u8, size: usize, c: u32, n: usize, l: L) -> i32 = ms::_mbsnset_s_l;
    _mbsnbset_s(s: *mut u8, size: usize, c: u32, n: usize) -> i32 = ms::_mbsnbset_s;
    _mbsnbset_s_l(s: *mut u8, size: usize, c: u32, n: usize, l: L) -> i32 = ms::_mbsnbset_s_l;
    _mbsncpy_s(dst: *mut u8, size: usize, src: *const u8, n: usize) -> i32 = ms::_mbsncpy_s;
    _mbsncpy_s_l(dst: *mut u8, size: usize, src: *const u8, n: usize, l: L) -> i32 = ms::_mbsncpy_s_l;
    _mbsncat_s(dst: *mut u8, size: usize, src: *const u8, n: usize) -> i32 = ms::_mbsncat_s;
    _mbsncat_s_l(dst: *mut u8, size: usize, src: *const u8, n: usize, l: L) -> i32 = ms::_mbsncat_s_l;
}

/// `__p__mbctype`: the multibyte code page's `_mbctype` (257 bytes, `EOF`
/// first).
#[unsafe(no_mangle)]
pub extern "C" fn __p__mbctype() -> *const u8 {
    lc::mbctype_ptr()
}

/// `__p__mbcasemap`.
#[unsafe(no_mangle)]
pub extern "C" fn __p__mbcasemap() -> *const u8 {
    lc::mbcasemap_ptr()
}

/// `_mbcasemap`: the global multibyte code page's case map (kept current
/// by `poly_win32_crt::locale::bind`).
#[unsafe(no_mangle)]
pub static _mbcasemap: poly_win32_crt::global::Global<[u8; 256]> = poly_win32_crt::global::Global::new([0; 256]);

/// `_configthreadlocale`.
#[unsafe(no_mangle)]
pub extern "C" fn _configthreadlocale(mode: i32) -> i32 {
    lc::configthreadlocale(mode)
}

/// At attach: the variables a global locale change updates.
pub fn bind() {
    lc::bind(
        lc::Cells {
            pctype: core::ptr::null_mut(),
            mb_cur_max: core::ptr::null_mut(),
            lc_codepage: core::ptr::null_mut(),
            lc_collate_cp: core::ptr::null_mut(),
            lc_handle: core::ptr::null_mut(),
            mbctype: core::ptr::null_mut(),
            mbcasemap: _mbcasemap.get(),
        },
        false,
    );
}
