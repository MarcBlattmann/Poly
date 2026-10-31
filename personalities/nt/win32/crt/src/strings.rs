//! The string, memory and character-class functions (C11 7.4, 7.24,
//! 7.29.4, 7.30; Learn's pages for the `_s` and `_str*`/`_wcs*` forms), on
//! [`poly_crt::string`] and [`poly_crt::ctype`]. `memcpy`, `memmove`,
//! `memset` and `memcmp` are the compiler's own (`compiler_builtins`),
//! exported by the DLLs' `build.rs`.

use crate::global;
use poly_crt::ctype;
use poly_crt::errno::{EINVAL, ERANGE};
use poly_crt::string::{self as s, Checked};
use poly_crt::unit::{self, Unit};

// ----- generic bodies ------------------------------------------------------------------

/// # Safety
/// `p` is a valid C string.
unsafe fn sl<'a, C: Unit>(p: *const C) -> &'a [C] {
    // SAFETY: the caller's contract.
    unsafe { unit::slice(p) }
}

/// # Safety
/// `dst` has room for `src` and its NUL.
unsafe fn copy<C: Unit>(dst: *mut C, src: *const C) -> *mut C {
    // SAFETY: the caller's contract.
    unsafe {
        let n = unit::len(src);
        core::ptr::copy(src, dst, n + 1);
    }
    dst
}

/// # Safety
/// `dst` holds `n` units; `src` is readable to its NUL or `n` units.
unsafe fn ncopy<C: Unit>(dst: *mut C, src: *const C, n: usize) -> *mut C {
    // SAFETY: the caller's contract.
    unsafe {
        let m = unit::slice_max(src, n).len();
        core::ptr::copy(src, dst, m);
        for i in m..n {
            dst.add(i).write(C::default());
        }
    }
    dst
}

/// # Safety
/// `dst` is a C string with room for `src` appended.
unsafe fn cat<C: Unit>(dst: *mut C, src: *const C) -> *mut C {
    // SAFETY: the caller's contract.
    unsafe { copy(dst.add(unit::len(dst)), src) };
    dst
}

/// # Safety
/// `dst` is a C string with room for `n` more units and a NUL.
unsafe fn ncat<C: Unit>(dst: *mut C, src: *const C, n: usize) -> *mut C {
    // SAFETY: the caller's contract.
    unsafe {
        let end = dst.add(unit::len(dst));
        let m = unit::slice_max(src, n).len();
        core::ptr::copy(src, end, m);
        end.add(m).write(C::default());
    }
    dst
}

/// Apply a [`Checked`] decision of a `_s` copy into `dst` at `at`.
///
/// # Safety
/// The decision was made for these buffers.
unsafe fn apply<C: Unit>(c: Checked, dst: *mut C, at: usize, src: *const C) -> i32 {
    // SAFETY: the decision keeps every write inside `dst`.
    unsafe {
        match c {
            Checked::Copy(n) | Checked::Truncate(n) => {
                core::ptr::copy(src, dst.add(at), n);
                dst.add(at + n).write(C::default());
            }
            Checked::Fail { errno, clear } => {
                if clear {
                    dst.write(C::default());
                }
                global::invalid(errno);
            }
            Checked::Nothing => {}
        }
    }
    s::checked_result(c)
}

fn opt_len<C: Unit>(p: *const C) -> Option<usize> {
    // SAFETY: a non-null argument is a C string by the functions' contracts.
    (!p.is_null()).then(|| unsafe { unit::len(p) })
}

/// The destination's length within `size` units (`None`: unterminated).
///
/// # Safety
/// `dst` holds `size` units.
unsafe fn len_within<C: Unit>(dst: *const C, size: usize) -> Option<usize> {
    // SAFETY: the caller's contract.
    let n = unsafe { unit::slice_max(dst, size) }.len();
    (n < size).then_some(n)
}

/// # Safety
/// As `strcpy_s`.
unsafe fn copy_s<C: Unit>(dst: *mut C, size: usize, src: *const C) -> i32 {
    let c = s::copy_s(dst.is_null(), size, opt_len(src));
    // SAFETY: as decided.
    unsafe { apply(c, dst, 0, src) }
}

/// # Safety
/// As `strncpy_s`.
unsafe fn ncopy_s<C: Unit>(dst: *mut C, size: usize, src: *const C, count: usize) -> i32 {
    // SAFETY: a non-null source is readable up to its NUL or `count`.
    let len = (!src.is_null()).then(|| unsafe { unit::slice_max(src, count) }.len());
    let c = s::ncopy_s(dst.is_null(), size, len, count);
    // SAFETY: as decided.
    unsafe { apply(c, dst, 0, src) }
}

/// # Safety
/// As `strcat_s`.
unsafe fn cat_s<C: Unit>(dst: *mut C, size: usize, src: *const C) -> i32 {
    // SAFETY: a non-null destination holds `size` units.
    let d = if dst.is_null() { None } else { unsafe { len_within(dst, size) } };
    let c = s::cat_s(dst.is_null(), size, d, opt_len(src));
    // SAFETY: as decided.
    unsafe { apply(c, dst, d.unwrap_or(0), src) }
}

/// # Safety
/// As `strncat_s`.
unsafe fn ncat_s<C: Unit>(dst: *mut C, size: usize, src: *const C, count: usize) -> i32 {
    // SAFETY: as `cat_s`.
    let d = if dst.is_null() { None } else { unsafe { len_within(dst, size) } };
    // SAFETY: as `ncopy_s`.
    let len = (!src.is_null()).then(|| unsafe { unit::slice_max(src, count) }.len());
    let c = s::ncat_s(dst.is_null(), size, d, len, count);
    // SAFETY: as decided.
    unsafe { apply(c, dst, d.unwrap_or(0), src) }
}

/// # Safety
/// `p` is a C string; `c` a unit value.
unsafe fn chr<C: Unit>(p: *const C, c: u32, last: bool) -> *mut C {
    // SAFETY: the caller's string, its NUL included in the search.
    let s = unsafe { core::slice::from_raw_parts(p, unit::len(p) + 1) };
    let c = C::from_u32(c);
    let found = if last { s.iter().rposition(|&x| x == c) } else { s.iter().position(|&x| x == c) };
    found.map_or(core::ptr::null_mut(), |i| p.wrapping_add(i) as *mut C)
}

/// One `strtok` step with a context pointer.
///
/// # Safety
/// `text` is NULL or a writable C string; `context` the saved position.
unsafe fn tok<C: Unit>(text: *mut C, delims: *const C, context: &mut *mut C) -> *mut C {
    let start = if text.is_null() { *context } else { text };
    if start.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    let (hay, set) = unsafe { (unit::slice(start), sl(delims)) };
    match s::token(hay, set) {
        Some((a, b, next)) => {
            // SAFETY: inside the string.
            unsafe {
                if b < hay.len() {
                    start.add(b).write(C::default());
                }
                *context = start.add(next);
                start.add(a)
            }
        }
        None => {
            // SAFETY: the end of the string.
            *context = unsafe { start.add(hay.len()) };
            core::ptr::null_mut()
        }
    }
}

// ----- lengths, comparisons -----------------------------------------------------------

/// # Safety
/// `p` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(p: *const u8) -> usize {
    // SAFETY: the caller's string.
    unsafe { unit::len(p) }
}

/// # Safety
/// As `strlen`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcslen(p: *const u16) -> usize {
    // SAFETY: the caller's string.
    unsafe { unit::len(p) }
}

/// # Safety
/// `p` is readable to its NUL or `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strnlen(p: *const u8, n: usize) -> usize {
    // SAFETY: the caller's contract.
    if p.is_null() { 0 } else { unsafe { unit::slice_max(p, n) }.len() }
}

/// # Safety
/// As `strnlen`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsnlen(p: *const u16, n: usize) -> usize {
    // SAFETY: the caller's contract.
    if p.is_null() { 0 } else { unsafe { unit::slice_max(p, n) }.len() }
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's strings.
    s::compare(unsafe { sl(a) }, unsafe { sl(b) })
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscmp(a: *const u16, b: *const u16) -> i32 {
    // SAFETY: the caller's strings.
    s::compare(unsafe { sl(a) }, unsafe { sl(b) })
}

/// # Safety
/// Readable to their NULs or `n` units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    // SAFETY: the caller's strings.
    s::compare(unsafe { s::prefix(a, n) }, unsafe { s::prefix(b, n) })
}

/// # Safety
/// As `strncmp`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsncmp(a: *const u16, b: *const u16, n: usize) -> i32 {
    // SAFETY: the caller's strings.
    s::compare(unsafe { s::prefix(a, n) }, unsafe { s::prefix(b, n) })
}

// ----- copies ---------------------------------------------------------------------------

/// # Safety
/// `dst` has room for `src`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcpy(dst: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: the caller's buffers.
    unsafe { copy(dst, src) }
}

/// # Safety
/// As `strcpy`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscpy(dst: *mut u16, src: *const u16) -> *mut u16 {
    // SAFETY: the caller's buffers.
    unsafe { copy(dst, src) }
}

/// # Safety
/// `dst` holds `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the caller's buffers.
    unsafe { ncopy(dst, src, n) }
}

/// # Safety
/// As `strncpy`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsncpy(dst: *mut u16, src: *const u16, n: usize) -> *mut u16 {
    // SAFETY: the caller's buffers.
    unsafe { ncopy(dst, src, n) }
}

/// # Safety
/// `dst` has room for `src` appended.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcat(dst: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: the caller's buffers.
    unsafe { cat(dst, src) }
}

/// # Safety
/// As `strcat`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscat(dst: *mut u16, src: *const u16) -> *mut u16 {
    // SAFETY: the caller's buffers.
    unsafe { cat(dst, src) }
}

/// # Safety
/// `dst` has room for `n` more bytes and a NUL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncat(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the caller's buffers.
    unsafe { ncat(dst, src, n) }
}

/// # Safety
/// As `strncat`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsncat(dst: *mut u16, src: *const u16, n: usize) -> *mut u16 {
    // SAFETY: the caller's buffers.
    unsafe { ncat(dst, src, n) }
}

/// # Safety
/// `dst` holds `size` bytes; `src` NULL or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcpy_s(dst: *mut u8, size: usize, src: *const u8) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { copy_s(dst, size, src) }
}

/// # Safety
/// As `strcpy_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscpy_s(dst: *mut u16, size: usize, src: *const u16) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { copy_s(dst, size, src) }
}

/// # Safety
/// As `strcpy_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncpy_s(dst: *mut u8, size: usize, src: *const u8, count: usize) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { ncopy_s(dst, size, src, count) }
}

/// # Safety
/// As `strcpy_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsncpy_s(dst: *mut u16, size: usize, src: *const u16, count: usize) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { ncopy_s(dst, size, src, count) }
}

/// # Safety
/// As `strcpy_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcat_s(dst: *mut u8, size: usize, src: *const u8) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { cat_s(dst, size, src) }
}

/// # Safety
/// As `strcpy_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscat_s(dst: *mut u16, size: usize, src: *const u16) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { cat_s(dst, size, src) }
}

/// # Safety
/// As `strcpy_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncat_s(dst: *mut u8, size: usize, src: *const u8, count: usize) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { ncat_s(dst, size, src, count) }
}

/// # Safety
/// As `strcpy_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsncat_s(dst: *mut u16, size: usize, src: *const u16, count: usize) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { ncat_s(dst, size, src, count) }
}

/// # Safety
/// `p` is NULL or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strdup(p: *const u8) -> *mut u8 {
    // SAFETY: the caller's string.
    if p.is_null() { core::ptr::null_mut() } else { crate::heap::dup(unsafe { sl(p) }) }
}

/// # Safety
/// As `_strdup`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsdup(p: *const u16) -> *mut u16 {
    // SAFETY: the caller's string.
    if p.is_null() { core::ptr::null_mut() } else { crate::heap::dup(unsafe { sl(p) }) }
}

/// # Safety
/// `dst` and `src` hold `count` and `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy_s(dst: *mut u8, size: usize, src: *const u8, n: usize) -> i32 {
    if n == 0 {
        return 0;
    }
    if dst.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    if src.is_null() || size < n {
        // SAFETY: the destination holds `size` bytes.
        unsafe { core::ptr::write_bytes(dst, 0, size) };
        let e = if src.is_null() { EINVAL } else { ERANGE };
        global::invalid(e);
        return e;
    }
    // SAFETY: the caller's buffers.
    unsafe { core::ptr::copy(src, dst, n) };
    0
}

/// # Safety
/// As `memcpy_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove_s(dst: *mut u8, size: usize, src: *const u8, n: usize) -> i32 {
    if n == 0 {
        return 0;
    }
    if dst.is_null() || src.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    if size < n {
        global::invalid(ERANGE);
        return ERANGE;
    }
    // SAFETY: the caller's buffers.
    unsafe { core::ptr::copy(src, dst, n) };
    0
}

/// # Safety
/// `n` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memchr(p: *const u8, c: i32, n: usize) -> *mut u8 {
    if n == 0 {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's buffer.
    let b = unsafe { core::slice::from_raw_parts(p, n) };
    b.iter().position(|&x| x == c as u8).map_or(core::ptr::null_mut(), |i| p.wrapping_add(i) as *mut u8)
}

/// `_memccpy`: copy through the first `c`; the byte after it, or NULL.
///
/// # Safety
/// Both hold `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _memccpy(dst: *mut u8, src: *const u8, c: i32, n: usize) -> *mut u8 {
    for i in 0..n {
        // SAFETY: i < n.
        unsafe {
            let b = *src.add(i);
            *dst.add(i) = b;
            if b == c as u8 {
                return dst.add(i + 1);
            }
        }
    }
    core::ptr::null_mut()
}

// ----- searching ---------------------------------------------------------------------------

/// # Safety
/// `p` is a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchr(p: *const u8, c: i32) -> *mut u8 {
    // SAFETY: the caller's string.
    unsafe { chr(p, u32::from(c as u8), false) }
}

/// # Safety
/// As `strchr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcschr(p: *const u16, c: u16) -> *mut u16 {
    // SAFETY: the caller's string.
    unsafe { chr(p, u32::from(c), false) }
}

/// # Safety
/// As `strchr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strrchr(p: *const u8, c: i32) -> *mut u8 {
    // SAFETY: the caller's string.
    unsafe { chr(p, u32::from(c as u8), true) }
}

/// # Safety
/// As `strchr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsrchr(p: *const u16, c: u16) -> *mut u16 {
    // SAFETY: the caller's string.
    unsafe { chr(p, u32::from(c), true) }
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strstr(hay: *const u8, needle: *const u8) -> *mut u8 {
    // SAFETY: the caller's strings.
    s::find(unsafe { sl(hay) }, unsafe { sl(needle) }).map_or(core::ptr::null_mut(), |i| hay.wrapping_add(i) as *mut u8)
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsstr(hay: *const u16, needle: *const u16) -> *mut u16 {
    // SAFETY: the caller's strings.
    s::find(unsafe { sl(hay) }, unsafe { sl(needle) })
        .map_or(core::ptr::null_mut(), |i| hay.wrapping_add(i) as *mut u16)
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strspn(p: *const u8, set: *const u8) -> usize {
    // SAFETY: the caller's strings.
    s::span(unsafe { sl(p) }, unsafe { sl(set) }, true)
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsspn(p: *const u16, set: *const u16) -> usize {
    // SAFETY: the caller's strings.
    s::span(unsafe { sl(p) }, unsafe { sl(set) }, true)
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcspn(p: *const u8, set: *const u8) -> usize {
    // SAFETY: the caller's strings.
    s::span(unsafe { sl(p) }, unsafe { sl(set) }, false)
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscspn(p: *const u16, set: *const u16) -> usize {
    // SAFETY: the caller's strings.
    s::span(unsafe { sl(p) }, unsafe { sl(set) }, false)
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strpbrk(p: *const u8, set: *const u8) -> *mut u8 {
    // SAFETY: the caller's strings.
    let (t, d) = unsafe { (sl(p), sl(set)) };
    let i = s::span(t, d, false);
    if i < t.len() { p.wrapping_add(i) as *mut u8 } else { core::ptr::null_mut() }
}

/// # Safety
/// Valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcspbrk(p: *const u16, set: *const u16) -> *mut u16 {
    // SAFETY: the caller's strings.
    let (t, d) = unsafe { (sl(p), sl(set)) };
    let i = s::span(t, d, false);
    if i < t.len() { p.wrapping_add(i) as *mut u16 } else { core::ptr::null_mut() }
}

/// `strtok`: the context is the thread's (Learn: "each thread has its own
/// strtok state").
///
/// # Safety
/// `text` NULL or a writable C string; `delims` a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok(text: *mut u8, delims: *const u8) -> *mut u8 {
    let t = global::thread();
    // SAFETY: the caller's strings.
    unsafe { tok(text, delims, &mut t.strtok) }
}

/// # Safety
/// As `strtok`; `context` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok_s(text: *mut u8, delims: *const u8, context: *mut *mut u8) -> *mut u8 {
    // SAFETY: a non-null context is the caller's.
    if context.is_null() || delims.is_null() || (text.is_null() && unsafe { (*context).is_null() }) {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's arguments.
    unsafe { tok(text, delims, &mut *context) }
}

/// # Safety
/// As `strtok_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstok_s(text: *mut u16, delims: *const u16, context: *mut *mut u16) -> *mut u16 {
    // SAFETY: as `strtok_s`.
    if context.is_null() || delims.is_null() || (text.is_null() && unsafe { (*context).is_null() }) {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's arguments.
    unsafe { tok(text, delims, &mut *context) }
}

/// The two-argument `wcstok` (`msvcrt.dll`, and the UCRT's `_wcstok`).
///
/// # Safety
/// As `strtok`.
pub unsafe fn wcstok_thread(text: *mut u16, delims: *const u16) -> *mut u16 {
    let t = global::thread();
    // SAFETY: the caller's strings.
    unsafe { tok(text, delims, &mut t.wcstok) }
}

// ----- in place -------------------------------------------------------------------------

/// # Safety
/// As `_strlwr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strrev(p: *mut u8) -> *mut u8 {
    if !p.is_null() {
        // SAFETY: the caller's string.
        unsafe { core::slice::from_raw_parts_mut(p, unit::len(p)) }.reverse();
    }
    p
}

/// # Safety
/// As `_strlwr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcsrev(p: *mut u16) -> *mut u16 {
    if !p.is_null() {
        // SAFETY: the caller's string.
        unsafe { core::slice::from_raw_parts_mut(p, unit::len(p)) }.reverse();
    }
    p
}

// ----- character classes -----------------------------------------------------------------

// The narrow classes and case depend on `LC_CTYPE`: `lctype.rs`.

macro_rules! classes {
    ($($wname:ident => $mask:expr;)*) => {
        $(
            #[unsafe(no_mangle)]
            pub extern "C" fn $wname(c: u16) -> i32 {
                i32::from(ctype::wide_class(u32::from(c)) & ($mask))
            }
        )*
    };
}

classes! {
    iswalpha => ctype::ALPHA;
    iswupper => ctype::UPPER;
    iswlower => ctype::LOWER;
    iswdigit => ctype::DIGIT;
    iswxdigit => ctype::HEX;
    iswspace => ctype::SPACE;
    iswpunct => ctype::PUNCT;
    iswcntrl => ctype::CONTROL;
    iswalnum => ctype::ALNUM;
    iswprint => ctype::PRINT;
    iswgraph => ctype::GRAPH;
}

pub extern "C" fn iswblank(c: u16) -> i32 {
    i32::from(c == 9 || ctype::wide_class(u32::from(c)) & ctype::BLANK != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn iswctype(c: u16, mask: u16) -> i32 {
    i32::from(ctype::wide_class(u32::from(c)) & mask)
}

#[unsafe(no_mangle)]
pub extern "C" fn __isascii(c: i32) -> i32 {
    i32::from((0..0x80).contains(&c))
}

#[unsafe(no_mangle)]
pub extern "C" fn __toascii(c: i32) -> i32 {
    c & 0x7F
}

/// `_toupper`: no check that `c` is lower case (Learn: "only ... if c is a
/// lowercase letter ... otherwise undefined"); Poly maps letters only.
#[unsafe(no_mangle)]
pub extern "C" fn _toupper(c: i32) -> i32 {
    ctype::to_upper(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn _tolower(c: i32) -> i32 {
    ctype::to_lower(c)
}

/// `_wctype`: the wide table for U+0000–U+00FF (with the `WEOF` entry
/// first, as `_ctype`).
#[unsafe(no_mangle)]
pub static _wctype: [u16; 257] = ctype::WIDE_TABLE;

// ----- errors --------------------------------------------------------------------------

/// The `strerror` texts as C strings (built once, kept).
fn message_cstr(e: i32) -> *mut u8 {
    static TABLE: crate::global::Global<[usize; 44]> = crate::global::Global::new([0; 44]);
    let i = if (0..=42).contains(&e) {
        e as usize
    } else if e == poly_crt::errno::STRUNCATE {
        43
    } else {
        15
    };
    // SAFETY: the table is filled once per slot; a race makes two equal
    // copies, one of which leaks.
    unsafe {
        let slot = &mut (*TABLE.get())[i];
        if *slot == 0 {
            *slot = crate::heap::dup(poly_crt::errno::message(e).as_bytes()) as usize;
        }
        *slot as *mut u8
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn strerror(e: i32) -> *mut u8 {
    message_cstr(e)
}

/// # Safety
/// `buf` holds `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strerror_s(buf: *mut u8, size: usize, e: i32) -> i32 {
    if buf.is_null() || size == 0 {
        global::invalid(EINVAL);
        return EINVAL;
    }
    let m = poly_crt::errno::message(e).as_bytes();
    let n = m.len().min(size - 1);
    // SAFETY: n + 1 <= size.
    unsafe {
        core::ptr::copy_nonoverlapping(m.as_ptr(), buf, n);
        buf.add(n).write(0);
    }
    if n < m.len() { poly_crt::errno::STRUNCATE } else { 0 }
}

/// `_wcserror`: a thread-shared wide copy (Poly keeps one per value).
#[unsafe(no_mangle)]
pub extern "C" fn _wcserror(e: i32) -> *mut u16 {
    static TABLE: crate::global::Global<[usize; 44]> = crate::global::Global::new([0; 44]);
    let i = if (0..=42).contains(&e) {
        e as usize
    } else if e == poly_crt::errno::STRUNCATE {
        43
    } else {
        15
    };
    // SAFETY: as in `message_cstr`.
    unsafe {
        let slot = &mut (*TABLE.get())[i];
        if *slot == 0 {
            let w: alloc::vec::Vec<u16> = poly_crt::errno::message(e).encode_utf16().collect();
            *slot = crate::heap::dup(&w) as usize;
        }
        *slot as *mut u16
    }
}

/// `_strerror(s)`: `"s: message\n"` for the current `errno` (or just the
/// message and a newline), in the thread's buffer.
///
/// # Safety
/// `s` is NULL or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strerror(s: *const u8) -> *mut u8 {
    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if !s.is_null() {
        // SAFETY: the caller's string.
        out.extend_from_slice(unsafe { sl(s) });
        out.extend_from_slice(b": ");
    }
    out.extend_from_slice(poly_crt::errno::message(global::thread().errno).as_bytes());
    out.push(b'\n');
    crate::heap::dup(&out)
}
