//! Locale-dependent characters and strings (ADR 0094): classification and
//! case by `LC_CTYPE`, collation by `LC_COLLATE` (`poly_nls::collate`, an
//! order of Poly's own, documented there), case-insensitive comparison,
//! numbers with `LC_NUMERIC`'s decimal point, and every function's `_l`
//! form, which takes a `_locale_t` (NULL: the current locale).

use crate::global;
use crate::locale::{LocaleT, loc, loc_of};
use alloc::vec::Vec;
use poly_crt::ctype;
use poly_crt::errno::{EINVAL, ERANGE};
use poly_crt::unit::{self, Unit};

type L = *const LocaleT;

/// `_NLSCMPERROR` (`string.h`).
const NLSCMPERROR: i32 = i32::MAX;

// ----- classification ---------------------------------------------------------------------

macro_rules! classes_l {
    ($($name:ident, $lname:ident => $mask:expr;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub extern "C" fn $name(c: i32) -> i32 {
                i32::from(loc().class(c) & ($mask))
            }
            #[unsafe(no_mangle)]
            pub extern "C" fn $lname(c: i32, l: L) -> i32 {
                i32::from(loc_of(l).class(c) & ($mask))
            }
        )*
    };
}

classes_l! {
    isalpha, _isalpha_l => ctype::ALPHA;
    isupper, _isupper_l => ctype::UPPER;
    islower, _islower_l => ctype::LOWER;
    isdigit, _isdigit_l => ctype::DIGIT;
    isxdigit, _isxdigit_l => ctype::HEX;
    isspace, _isspace_l => ctype::SPACE;
    iscntrl, _iscntrl_l => ctype::CONTROL;
    isalnum, _isalnum_l => ctype::ALNUM;
    isprint, _isprint_l => ctype::PRINT;
    isgraph, _isgraph_l => ctype::GRAPH;
    isleadbyte, _isleadbyte_l => ctype::LEADBYTE;
}

#[unsafe(no_mangle)]
pub extern "C" fn ispunct(c: i32) -> i32 {
    i32::from(loc().class(c) & ctype::PUNCT)
}

/// `_ispunct_l` (`ucrtbase` only).
pub extern "C" fn _ispunct_l(c: i32, l: L) -> i32 {
    i32::from(loc_of(l).class(c) & ctype::PUNCT)
}

/// `isblank`: tab, or `_BLANK`.
pub extern "C" fn isblank(c: i32) -> i32 {
    i32::from(c == 9 || loc().class(c) & ctype::BLANK != 0)
}

pub extern "C" fn _isblank_l(c: i32, l: L) -> i32 {
    i32::from(c == 9 || loc_of(l).class(c) & ctype::BLANK != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn _isctype(c: i32, mask: i32) -> i32 {
    i32::from(loc().class(c)) & mask
}

#[unsafe(no_mangle)]
pub extern "C" fn _isctype_l(c: i32, mask: i32, l: L) -> i32 {
    i32::from(loc_of(l).class(c)) & mask
}

#[unsafe(no_mangle)]
pub extern "C" fn __iscsym(c: i32) -> i32 {
    i32::from(c == 0x5F || loc().class(c) & ctype::ALNUM != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn __iscsymf(c: i32) -> i32 {
    i32::from(c == 0x5F || loc().class(c) & ctype::ALPHA != 0)
}

/// Wide classes do not depend on the locale (`_wctype`, `poly_nls`).
macro_rules! wide_classes_l {
    ($($lname:ident => $mask:expr;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub extern "C" fn $lname(c: u16, _l: L) -> i32 {
                i32::from(ctype::wide_class(u32::from(c)) & ($mask))
            }
        )*
    };
}

wide_classes_l! {
    _iswalpha_l => ctype::ALPHA;
    _iswupper_l => ctype::UPPER;
    _iswlower_l => ctype::LOWER;
    _iswdigit_l => ctype::DIGIT;
    _iswxdigit_l => ctype::HEX;
    _iswspace_l => ctype::SPACE;
    _iswpunct_l => ctype::PUNCT;
    _iswcntrl_l => ctype::CONTROL;
    _iswalnum_l => ctype::ALNUM;
    _iswprint_l => ctype::PRINT;
    _iswgraph_l => ctype::GRAPH;
}

pub extern "C" fn _iswblank_l(c: u16, _l: L) -> i32 {
    i32::from(c == 9 || ctype::wide_class(u32::from(c)) & ctype::BLANK != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn _iswctype_l(c: u16, mask: u16, _l: L) -> i32 {
    i32::from(ctype::wide_class(u32::from(c)) & mask)
}

/// `is_wctype`: the old name of `iswctype`.
#[unsafe(no_mangle)]
pub extern "C" fn is_wctype(c: u16, mask: u16) -> i32 {
    i32::from(ctype::wide_class(u32::from(c)) & mask)
}

#[unsafe(no_mangle)]
pub extern "C" fn iswascii(c: u16) -> i32 {
    i32::from(c < 0x80)
}

pub extern "C" fn __iswcsym(c: u16) -> i32 {
    i32::from(c == 0x5F || ctype::wide_class(u32::from(c)) & ctype::ALNUM != 0)
}

pub extern "C" fn __iswcsymf(c: u16) -> i32 {
    i32::from(c == 0x5F || ctype::wide_class(u32::from(c)) & ctype::ALPHA != 0)
}

pub extern "C" fn _iswcsym_l(c: u16, _l: L) -> i32 {
    __iswcsym(c)
}

pub extern "C" fn _iswcsymf_l(c: u16, _l: L) -> i32 {
    __iswcsymf(c)
}

/// `wctype(name)`: the class mask C11 7.30.2.2.2 names (`ctype.h`'s bits),
/// 0 for an unknown name.
///
/// # Safety
/// `name` a C string.
pub unsafe extern "C" fn wctype(name: *const u8) -> u16 {
    if name.is_null() {
        return 0;
    }
    // SAFETY: the caller's string.
    match unsafe { unit::slice(name) } {
        b"alnum" => ctype::ALNUM,
        b"alpha" => ctype::ALPHA,
        b"blank" => ctype::BLANK,
        b"cntrl" => ctype::CONTROL,
        b"digit" => ctype::DIGIT,
        b"graph" => ctype::GRAPH,
        b"lower" => ctype::LOWER,
        b"print" => ctype::PRINT,
        b"punct" => ctype::PUNCT,
        b"space" => ctype::SPACE,
        b"upper" => ctype::UPPER,
        b"xdigit" => ctype::HEX,
        _ => 0,
    }
}

/// `wctrans(name)`: 1 for "toupper", 2 for "tolower" (Poly's values), else 0.
///
/// # Safety
/// `name` a C string.
pub unsafe extern "C" fn wctrans(name: *const u8) -> u16 {
    if name.is_null() {
        return 0;
    }
    // SAFETY: the caller's string.
    match unsafe { unit::slice(name) } {
        b"toupper" => 1,
        b"tolower" => 2,
        _ => 0,
    }
}

pub extern "C" fn towctrans(c: u16, t: u16) -> u16 {
    match t {
        1 => towupper(c),
        2 => towlower(c),
        _ => c,
    }
}

// ----- case ------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn toupper(c: i32) -> i32 {
    loc().to_upper(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn tolower(c: i32) -> i32 {
    loc().to_lower(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn _toupper_l(c: i32, l: L) -> i32 {
    loc_of(l).to_upper(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn _tolower_l(c: i32, l: L) -> i32 {
    loc_of(l).to_lower(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn towupper(c: u16) -> u16 {
    loc().to_upper_wide(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn towlower(c: u16) -> u16 {
    loc().to_lower_wide(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn _towupper_l(c: u16, l: L) -> u16 {
    loc_of(l).to_upper_wide(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn _towlower_l(c: u16, l: L) -> u16 {
    loc_of(l).to_lower_wide(c)
}

/// `__pctype_func`: `LC_CTYPE`'s table, indexable from −1.
#[unsafe(no_mangle)]
pub extern "C" fn __pctype_func() -> *const u16 {
    loc().public.pctype
}

/// `__pwctype_func`: the wide table for U+0000-U+00FF (`_wctype + 1`).
#[unsafe(no_mangle)]
pub extern "C" fn __pwctype_func() -> *const u16 {
    ctype::WIDE[1..].as_ptr()
}

/// `___mb_cur_max_func`: `MB_CUR_MAX`.
#[unsafe(no_mangle)]
pub extern "C" fn ___mb_cur_max_func() -> i32 {
    loc().public.mb_cur_max
}

pub extern "C" fn ___mb_cur_max_l_func(l: L) -> i32 {
    loc_of(l).public.mb_cur_max
}

// ----- case-insensitive comparison and case conversion -----------------------------------

fn fold_narrow(l: &crate::locale::Loc, b: u8) -> u8 {
    l.ctype.lower[usize::from(b)]
}

fn fold_wide(l: &crate::locale::Loc, c: u16) -> u16 {
    if l.ctype_c { if (0x41..=0x5A).contains(&c) { c + 32 } else { c } } else { poly_nls::unicode::to_lower(c) }
}

fn icmp<C: Copy + Into<u32>>(a: &[C], b: &[C], fold: impl Fn(C) -> u32) -> i32 {
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).map_or(0, |&c| fold(c));
        let y = b.get(i).map_or(0, |&c| fold(c));
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    0
}

/// # Safety
/// `p` a C string (or NULL), read up to `n` units.
unsafe fn prefix<'a, C: Unit>(p: *const C, n: usize) -> &'a [C] {
    // SAFETY: the caller's string.
    unsafe { unit::slice_max(p, n) }
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _stricmp_l(a: *const u8, b: *const u8, l: L) -> i32 {
    if a.is_null() || b.is_null() {
        global::invalid(EINVAL);
        return NLSCMPERROR;
    }
    let l = loc_of(l);
    // SAFETY: the caller's strings.
    icmp(unsafe { unit::slice(a) }, unsafe { unit::slice(b) }, |c| u32::from(fold_narrow(l, c)))
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _stricmp(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { _stricmp_l(a, b, core::ptr::null()) }
}

/// # Safety
/// Readable to their NULs or `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strnicmp_l(a: *const u8, b: *const u8, n: usize, l: L) -> i32 {
    if n == 0 {
        return 0;
    }
    if a.is_null() || b.is_null() {
        global::invalid(EINVAL);
        return NLSCMPERROR;
    }
    let l = loc_of(l);
    // SAFETY: the caller's strings.
    icmp(unsafe { prefix(a, n) }, unsafe { prefix(b, n) }, |c| u32::from(fold_narrow(l, c)))
}

/// # Safety
/// As `_strnicmp_l`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strnicmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { _strnicmp_l(a, b, n, core::ptr::null()) }
}

/// # Safety
/// `n` readable bytes each.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _memicmp_l(a: *const u8, b: *const u8, n: usize, l: L) -> i32 {
    if n == 0 {
        return 0;
    }
    if a.is_null() || b.is_null() {
        global::invalid(EINVAL);
        return NLSCMPERROR;
    }
    let l = loc_of(l);
    // SAFETY: the caller's buffers.
    let (x, y) = unsafe { (core::slice::from_raw_parts(a, n), core::slice::from_raw_parts(b, n)) };
    for i in 0..n {
        let (p, q) = (fold_narrow(l, x[i]), fold_narrow(l, y[i]));
        if p != q {
            return if p < q { -1 } else { 1 };
        }
    }
    0
}

/// # Safety
/// As `_memicmp_l`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _memicmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { _memicmp_l(a, b, n, core::ptr::null()) }
}

/// # Safety
/// Valid wide C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsicmp_l(a: *const u16, b: *const u16, l: L) -> i32 {
    if a.is_null() || b.is_null() {
        global::invalid(EINVAL);
        return NLSCMPERROR;
    }
    let l = loc_of(l);
    // SAFETY: the caller's strings.
    icmp(unsafe { unit::slice(a) }, unsafe { unit::slice(b) }, |c| u32::from(fold_wide(l, c)))
}

/// # Safety
/// Valid wide C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsicmp(a: *const u16, b: *const u16) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { _wcsicmp_l(a, b, core::ptr::null()) }
}

/// # Safety
/// Readable to their NULs or `n` units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsnicmp_l(a: *const u16, b: *const u16, n: usize, l: L) -> i32 {
    if n == 0 {
        return 0;
    }
    if a.is_null() || b.is_null() {
        global::invalid(EINVAL);
        return NLSCMPERROR;
    }
    let l = loc_of(l);
    // SAFETY: the caller's strings.
    icmp(unsafe { prefix(a, n) }, unsafe { prefix(b, n) }, |c| u32::from(fold_wide(l, c)))
}

/// # Safety
/// As `_wcsnicmp_l`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsnicmp(a: *const u16, b: *const u16, n: usize) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { _wcsnicmp_l(a, b, n, core::ptr::null()) }
}

/// Map a string in place.
///
/// # Safety
/// `p` NULL or a writable C string.
unsafe fn map_in_place<C: Unit>(p: *mut C, f: impl Fn(C) -> C) -> *mut C {
    if p.is_null() {
        global::invalid(EINVAL);
        return p;
    }
    // SAFETY: the caller's writable string.
    let s = unsafe { core::slice::from_raw_parts_mut(p, unit::len(p)) };
    for c in s {
        *c = f(*c);
    }
    p
}

/// The `_s` form: `size` must hold the string and its NUL.
///
/// # Safety
/// `p` NULL or `size` writable units holding a C string.
unsafe fn map_in_place_s<C: Unit>(p: *mut C, size: usize, f: impl Fn(C) -> C) -> i32 {
    if p.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: up to `size` units are the caller's.
    let n = unsafe { unit::slice_max(p, size) }.len();
    if n >= size {
        if size > 0 {
            // SAFETY: at least one writable unit.
            unsafe { p.write(C::default()) };
        }
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: n units of the caller's string.
    let s = unsafe { core::slice::from_raw_parts_mut(p, n) };
    for c in s {
        *c = f(*c);
    }
    0
}

macro_rules! case_maps {
    ($($(#[$m:meta])* $name:ident, $lname:ident, $sname:ident, $slname:ident: $unit:ty, $f:ident;)*) => {
        $(
            /// # Safety
            /// `p` NULL or a writable C string.
            $(#[$m])*
            pub unsafe extern "C" fn $lname(p: *mut $unit, l: L) -> *mut $unit {
                let l = loc_of(l);
                // SAFETY: the caller's string.
                unsafe { map_in_place(p, |c| $f(l, c)) }
            }
            /// # Safety
            /// As above.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(p: *mut $unit) -> *mut $unit {
                // SAFETY: the caller's string.
                unsafe { $lname(p, core::ptr::null()) }
            }
            /// # Safety
            /// `p` NULL or `size` writable units.
            $(#[$m])*
            pub unsafe extern "C" fn $slname(p: *mut $unit, size: usize, l: L) -> i32 {
                let l = loc_of(l);
                // SAFETY: the caller's buffer.
                unsafe { map_in_place_s(p, size, |c| $f(l, c)) }
            }
            /// # Safety
            /// As above.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $sname(p: *mut $unit, size: usize) -> i32 {
                // SAFETY: the caller's buffer.
                unsafe { $slname(p, size, core::ptr::null()) }
            }
        )*
    };
}

fn lower_n(l: &crate::locale::Loc, c: u8) -> u8 {
    l.ctype.lower[usize::from(c)]
}
fn upper_n(l: &crate::locale::Loc, c: u8) -> u8 {
    l.ctype.upper[usize::from(c)]
}
fn lower_w(l: &crate::locale::Loc, c: u16) -> u16 {
    l.to_lower_wide(c)
}
fn upper_w(l: &crate::locale::Loc, c: u16) -> u16 {
    l.to_upper_wide(c)
}

case_maps! {
    #[unsafe(no_mangle)] _strlwr, _strlwr_l, _strlwr_s, _strlwr_s_l: u8, lower_n;
    #[unsafe(no_mangle)] _strupr, _strupr_l, _strupr_s, _strupr_s_l: u8, upper_n;
    #[unsafe(no_mangle)] _wcslwr, _wcslwr_l, _wcslwr_s, _wcslwr_s_l: u16, lower_w;
    #[unsafe(no_mangle)] _wcsupr, _wcsupr_l, _wcsupr_s, _wcsupr_s_l: u16, upper_w;
}

// ----- collation -------------------------------------------------------------------------

fn order(o: core::cmp::Ordering) -> i32 {
    match o {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// Collate two narrow strings in `l`'s `LC_COLLATE`.
fn coll_narrow(l: &crate::locale::Loc, a: &[u8], b: &[u8], ignore_case: bool) -> i32 {
    if l.collate_c {
        return if ignore_case {
            icmp(a, b, |c| u32::from(if c.is_ascii_uppercase() { c + 32 } else { c }))
        } else {
            order(a.cmp(b))
        };
    }
    let (x, y) = (crate::locale::to_wide_lossy(l.collate_cp, a), crate::locale::to_wide_lossy(l.collate_cp, b));
    order(poly_nls::collate::compare(&x, &y, ignore_case))
}

fn coll_wide(l: &crate::locale::Loc, a: &[u16], b: &[u16], ignore_case: bool) -> i32 {
    if l.collate_c {
        return if ignore_case {
            icmp(a, b, |c| u32::from(if (0x41..=0x5A).contains(&c) { c + 32 } else { c }))
        } else {
            order(a.cmp(b))
        };
    }
    order(poly_nls::collate::compare(a, b, ignore_case))
}

macro_rules! collations {
    ($($(#[$m:meta])* $name:ident, $lname:ident: $unit:ty, $f:ident, $ic:expr, $n:tt;)*) => {
        $( collations!(@one $(#[$m])* $name, $lname, $unit, $f, $ic, $n); )*
    };
    (@one $(#[$m:meta])* $name:ident, $lname:ident, $unit:ty, $f:ident, $ic:expr, full) => {
        /// # Safety
        /// Valid C strings.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $lname(a: *const $unit, b: *const $unit, l: L) -> i32 {
            if a.is_null() || b.is_null() {
                global::invalid(EINVAL);
                return NLSCMPERROR;
            }
            // SAFETY: the caller's strings.
            $f(loc_of(l), unsafe { unit::slice(a) }, unsafe { unit::slice(b) }, $ic)
        }
        /// # Safety
        /// Valid C strings.
        $(#[$m])*
        pub unsafe extern "C" fn $name(a: *const $unit, b: *const $unit) -> i32 {
            // SAFETY: the caller's strings.
            unsafe { $lname(a, b, core::ptr::null()) }
        }
    };
    (@one $(#[$m:meta])* $name:ident, $lname:ident, $unit:ty, $f:ident, $ic:expr, count) => {
        /// # Safety
        /// Readable to their NULs or `n` units.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $lname(a: *const $unit, b: *const $unit, n: usize, l: L) -> i32 {
            if n == 0 {
                return 0;
            }
            if a.is_null() || b.is_null() {
                global::invalid(EINVAL);
                return NLSCMPERROR;
            }
            // SAFETY: the caller's strings.
            $f(loc_of(l), unsafe { prefix(a, n) }, unsafe { prefix(b, n) }, $ic)
        }
        /// # Safety
        /// As above.
        $(#[$m])*
        pub unsafe extern "C" fn $name(a: *const $unit, b: *const $unit, n: usize) -> i32 {
            // SAFETY: the caller's strings.
            unsafe { $lname(a, b, n, core::ptr::null()) }
        }
    };
}

collations! {
    #[unsafe(no_mangle)] strcoll, _strcoll_l: u8, coll_narrow, false, full;
    #[unsafe(no_mangle)] wcscoll, _wcscoll_l: u16, coll_wide, false, full;
    #[unsafe(no_mangle)] _stricoll, _stricoll_l: u8, coll_narrow, true, full;
    #[unsafe(no_mangle)] _wcsicoll, _wcsicoll_l: u16, coll_wide, true, full;
    #[unsafe(no_mangle)] _strncoll, _strncoll_l: u8, coll_narrow, false, count;
    #[unsafe(no_mangle)] _wcsncoll, _wcsncoll_l: u16, coll_wide, false, count;
    #[unsafe(no_mangle)] _strnicoll, _strnicoll_l: u8, coll_narrow, true, count;
    #[unsafe(no_mangle)] _wcsnicoll, _wcsnicoll_l: u16, coll_wide, true, count;
}

/// The transformed string of `strxfrm` / `wcsxfrm` (without its NUL): in
/// "C" the string itself, else `poly_nls::collate::sort_key`.
fn xfrm<C: Unit>(l: &crate::locale::Loc, s: &[C]) -> Vec<u32> {
    if l.collate_c {
        return s.iter().map(|c| c.to_u32()).collect();
    }
    let wide: Vec<u16> = if C::WIDE {
        s.iter().map(|c| c.to_u32() as u16).collect()
    } else {
        let bytes: Vec<u8> = s.iter().map(|c| c.to_u32() as u8).collect();
        crate::locale::to_wide_lossy(l.collate_cp, &bytes)
    };
    let mut key = poly_nls::collate::sort_key(&wide);
    key.pop();
    key.into_iter().map(u32::from).collect()
}

/// # Safety
/// `src` a C string; `dst` NULL (with `n` 0) or `n` writable units.
unsafe fn strxfrm_in<C: Unit>(dst: *mut C, src: *const C, n: usize, l: L) -> usize {
    if src.is_null() || (dst.is_null() && n > 0) {
        global::invalid(EINVAL);
        return usize::MAX;
    }
    // SAFETY: the caller's string.
    let key = xfrm(loc_of(l), unsafe { unit::slice(src) });
    if key.len() < n {
        for (i, &k) in key.iter().enumerate() {
            // SAFETY: i < n.
            unsafe { dst.add(i).write(C::from_u32(k)) };
        }
        // SAFETY: key.len() < n.
        unsafe { dst.add(key.len()).write(C::default()) };
    } else if n > 0 {
        global::set_errno(ERANGE);
    }
    key.len()
}

/// # Safety
/// As `strxfrm_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strxfrm_l(dst: *mut u8, src: *const u8, n: usize, l: L) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { strxfrm_in(dst, src, n, l) }
}

/// # Safety
/// As `strxfrm_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strxfrm(dst: *mut u8, src: *const u8, n: usize) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { strxfrm_in(dst, src, n, core::ptr::null()) }
}

/// # Safety
/// As `strxfrm_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsxfrm_l(dst: *mut u16, src: *const u16, n: usize, l: L) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { strxfrm_in(dst, src, n, l) }
}

/// # Safety
/// As `strxfrm_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsxfrm(dst: *mut u16, src: *const u16, n: usize) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { strxfrm_in(dst, src, n, core::ptr::null()) }
}

// ----- numbers ----------------------------------------------------------------------------

macro_rules! float_l {
    ($($(#[$m:meta])* $name:ident: $unit:ty => $ret:ty, $fmt:expr, $conv:expr;)*) => {
        $(
            /// # Safety
            /// `s` a valid C string; `end` NULL or writable.
            $(#[$m])*
            pub unsafe extern "C" fn $name(s: *const $unit, end: *mut *mut $unit, l: L) -> $ret {
                // SAFETY: the caller's arguments.
                let bits = unsafe { crate::conv::strtofloat(s, end, $fmt, loc_of(l).point) };
                $conv(bits)
            }
        )*
    };
}

fn f64_of(b: u64) -> f64 {
    f64::from_bits(b)
}
fn f32_of(b: u64) -> f32 {
    f32::from_bits(b as u32)
}

float_l! {
    #[unsafe(no_mangle)] _strtod_l: u8 => f64, poly_crt::float::DOUBLE, f64_of;
    _strtof_l: u8 => f32, poly_crt::float::SINGLE, f32_of;
    _strtold_l: u8 => f64, poly_crt::float::DOUBLE, f64_of;
    #[unsafe(no_mangle)] _wcstod_l: u16 => f64, poly_crt::float::DOUBLE, f64_of;
    _wcstof_l: u16 => f32, poly_crt::float::SINGLE, f32_of;
    _wcstold_l: u16 => f64, poly_crt::float::DOUBLE, f64_of;
}

/// `wcstold`: `long double` is `double`.
///
/// # Safety
/// As `wcstod`.
pub unsafe extern "C" fn wcstold(s: *const u16, end: *mut *mut u16) -> f64 {
    // SAFETY: the caller's arguments.
    unsafe { _wcstod_l(s, end, core::ptr::null()) }
}

/// # Safety
/// `s` a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _atof_l(s: *const u8, l: L) -> f64 {
    // SAFETY: the caller's string.
    unsafe { _strtod_l(s, core::ptr::null_mut(), l) }
}

/// # Safety
/// `s` a valid wide C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wtof_l(s: *const u16, l: L) -> f64 {
    // SAFETY: the caller's string.
    unsafe { _wcstod_l(s, core::ptr::null_mut(), l) }
}

/// `_atodbl` / `_atoldbl` / `_atoflt` (`_CRT_DOUBLE` …: the value is
/// stored at `out`): 0, or `_UNDERFLOW` (4) / `_OVERFLOW` (3) (`stdlib.h`).
///
/// # Safety
/// `out` writable; `s` a C string.
unsafe fn atodbl(out: *mut u8, s: *const u8, l: L, single: bool) -> i32 {
    if out.is_null() || s.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    let fmt = if single { poly_crt::float::SINGLE } else { poly_crt::float::DOUBLE };
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    let lead = t.iter().take_while(|&&c| ctype::is_space_unit(u32::from(c), false)).count();
    let p = poly_crt::float::parse_point(&t[lead..], fmt, loc_of(l).point);
    // SAFETY: the caller's _CRT_DOUBLE / _CRT_FLOAT.
    unsafe {
        if single {
            (out as *mut u32).write_unaligned(p.bits as u32);
        } else {
            (out as *mut u64).write_unaligned(p.bits);
        }
    }
    if !p.range_error {
        return 0;
    }
    let mag = if single { p.bits as u32 & 0x7FFF_FFFF == 0x7F80_0000 } else { p.bits & !(1 << 63) == 0x7FF0 << 48 };
    if mag { 3 } else { 4 }
}

/// # Safety
/// As `atodbl`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _atodbl_l(out: *mut u8, s: *const u8, l: L) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { atodbl(out, s, l, false) }
}

/// # Safety
/// As `atodbl`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _atoldbl_l(out: *mut u8, s: *const u8, l: L) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { atodbl(out, s, l, false) }
}

/// # Safety
/// As `atodbl`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _atoflt_l(out: *mut u8, s: *const u8, l: L) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { atodbl(out, s, l, true) }
}

/// # Safety
/// As `atodbl`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _atodbl(out: *mut u8, s: *const u8) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { atodbl(out, s, core::ptr::null(), false) }
}

/// # Safety
/// As `atodbl`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _atoldbl(out: *mut u8, s: *const u8) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { atodbl(out, s, core::ptr::null(), false) }
}

/// # Safety
/// As `atodbl`.
pub unsafe extern "C" fn _atoflt(out: *mut u8, s: *const u8) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { atodbl(out, s, core::ptr::null(), true) }
}

/// Integer conversions do not depend on the locale: the `_l` forms are
/// the plain ones.
macro_rules! int_l {
    ($($(#[$m:meta])* $name:ident: $unit:ty => $ret:ty, $kind:expr;)*) => {
        $(
            /// # Safety
            /// `s` a valid C string; `end` NULL or writable.
            $(#[$m])*
            pub unsafe extern "C" fn $name(s: *const $unit, end: *mut *mut $unit, base: i32, _l: L) -> $ret {
                // SAFETY: the caller's arguments.
                unsafe { crate::conv::strto(s, end, base, $kind) as $ret }
            }
        )*
    };
}

use poly_crt::convert::Kind;

int_l! {
    #[unsafe(no_mangle)] _strtol_l: u8 => i32, Kind::I32;
    #[unsafe(no_mangle)] _strtoul_l: u8 => u32, Kind::U32;
    _strtoll_l: u8 => i64, Kind::I64;
    _strtoull_l: u8 => u64, Kind::U64;
    #[unsafe(no_mangle)] _strtoi64_l: u8 => i64, Kind::I64;
    #[unsafe(no_mangle)] _strtoui64_l: u8 => u64, Kind::U64;
    _strtoimax_l: u8 => i64, Kind::I64;
    _strtoumax_l: u8 => u64, Kind::U64;
    #[unsafe(no_mangle)] _wcstol_l: u16 => i32, Kind::I32;
    #[unsafe(no_mangle)] _wcstoul_l: u16 => u32, Kind::U32;
    _wcstoll_l: u16 => i64, Kind::I64;
    _wcstoull_l: u16 => u64, Kind::U64;
    #[unsafe(no_mangle)] _wcstoi64_l: u16 => i64, Kind::I64;
    #[unsafe(no_mangle)] _wcstoui64_l: u16 => u64, Kind::U64;
    _wcstoimax_l: u16 => i64, Kind::I64;
    _wcstoumax_l: u16 => u64, Kind::U64;
}

/// # Safety
/// `s` a valid wide C string; `end` NULL or writable.
pub unsafe extern "C" fn wcstoimax(s: *const u16, end: *mut *mut u16, base: i32) -> i64 {
    // SAFETY: the caller's arguments.
    unsafe { crate::conv::strto(s, end, base, Kind::I64) as i64 }
}

/// # Safety
/// As `wcstoimax`.
pub unsafe extern "C" fn wcstoumax(s: *const u16, end: *mut *mut u16, base: i32) -> u64 {
    // SAFETY: the caller's arguments.
    unsafe { crate::conv::strto(s, end, base, Kind::U64) }
}

macro_rules! ato_l {
    ($($(#[$m:meta])* $name:ident: $unit:ty => $ret:ty, $wide:expr;)*) => {
        $(
            /// # Safety
            /// `s` a valid C string.
            $(#[$m])*
            pub unsafe extern "C" fn $name(s: *const $unit, _l: L) -> $ret {
                // SAFETY: the caller's string.
                unsafe { crate::conv::ato(s, $wide) as $ret }
            }
        )*
    };
}

ato_l! {
    #[unsafe(no_mangle)] _atoi_l: u8 => i32, false;
    #[unsafe(no_mangle)] _atol_l: u8 => i32, false;
    _atoll_l: u8 => i64, true;
    #[unsafe(no_mangle)] _atoi64_l: u8 => i64, true;
    #[unsafe(no_mangle)] _wtoi_l: u16 => i32, false;
    #[unsafe(no_mangle)] _wtol_l: u16 => i32, false;
    _wtoll_l: u16 => i64, true;
    #[unsafe(no_mangle)] _wtoi64_l: u16 => i64, true;
}

// ----- time -------------------------------------------------------------------------------

/// `_strftime_l`: `LC_TIME` is not localised (ADR 0094), as `strftime`.
///
/// # Safety
/// As `strftime`.
pub unsafe extern "C" fn _strftime_l(
    buf: *mut u8,
    size: usize,
    fmt: *const u8,
    tm: *const poly_crt::time::Tm,
    _l: L,
) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { crate::timefn::strftime(buf, size, fmt, tm) }
}

/// `_wcsftime_l`.
///
/// # Safety
/// As `wcsftime`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsftime_l(
    buf: *mut u16,
    size: usize,
    fmt: *const u16,
    tm: *const poly_crt::time::Tm,
    _l: L,
) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { crate::timefn::wcsftime(buf, size, fmt, tm) }
}
