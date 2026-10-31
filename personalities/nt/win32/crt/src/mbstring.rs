//! The multibyte-string functions (ADR 0094; `mbstring.h`, `mbctype.h`):
//! `_mbs*`, `_mbc*`, `_mbb*`, `_ismb*` and their `_l` forms, on the
//! multibyte code page (`_setmbcp`, or the `_locale_t`'s). The logic is
//! `poly_crt::mbcs`'s; a character's UTF-16 form comes from `kernel32`.
//!
//! Each function is written once as its `_l` form; the plain form passes
//! NULL (the current locale). A `[yes]` mark exports the function from
//! this shared crate (both DLLs export it); `[no]` leaves it a plain
//! function, which `ucrtbase` (or `msvcrt`) exports itself when only it has
//! that name.

use crate::global;
use crate::locale::{Kernel, Loc, LocaleT, resolve};
use poly_crt::errno::{EINVAL, ERANGE, STRUNCATE};
use poly_crt::mbcs::{self, Mbc};
use poly_crt::{ctype, unit};

type L = *const LocaleT;

/// `_NLSCMPERROR` (`string.h`), `_TRUNCATE`.
const NLSCMPERROR: i32 = i32::MAX;
const TRUNCATE: usize = usize::MAX;

macro_rules! export {
    (yes, $item:item) => {
        #[unsafe(no_mangle)]
        $item
    };
    (no, $item:item) => {
        $item
    };
}

/// `fn plain [mark] / name_l [mark] (args) -> ret, |mbc, chars, loc| body`.
macro_rules! mb {
    ($(fn $name:ident [$pe:ident] / $lname:ident [$le:ident] ($($p:ident: $t:ty),*) -> $r:ty, |$m:ident, $ch:ident, $lo:ident| $body:block)*) => {
        $(
            export!($le, pub unsafe extern "C" fn $lname($($p: $t,)* l: L) -> $r {
                let (lo, md) = resolve(l);
                let $m: &Mbc = &md.mbc;
                let $ch = Kernel($m.code_page);
                let $lo: &Loc = lo;
                let _ = (&$ch, &$lo);
                #[allow(unused_unsafe)]
                // SAFETY: the body reads and writes only the caller's
                // strings, as the function's contract (Learn) allows.
                unsafe { $body }
            });
            export!($pe, pub unsafe extern "C" fn $name($($p: $t),*) -> $r {
                // SAFETY: the caller's arguments.
                unsafe { $lname($($p,)* core::ptr::null()) }
            });
        )*
    };
}

/// # Safety
/// `p` a C string.
unsafe fn sl<'a>(p: *const u8) -> &'a [u8] {
    // SAFETY: the caller's string.
    unsafe { unit::slice(p) }
}

/// # Safety
/// `p` a writable C string.
unsafe fn slm<'a>(p: *mut u8) -> &'a mut [u8] {
    // SAFETY: the caller's writable string.
    unsafe { core::slice::from_raw_parts_mut(p, unit::len(p)) }
}

fn byte_class(lo: &Loc, c: u32) -> u16 {
    if c > 0xFF { 0 } else { lo.class(c as i32) }
}

fn null_arg() -> bool {
    global::invalid(EINVAL);
    true
}

// ----- single bytes -----------------------------------------------------------------------

mb! {
    fn _ismbbalnum [yes] / _ismbbalnum_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(byte_class(lo, c) & ctype::ALNUM != 0 || c <= 0xFF && m.has(c as u8, mbcs::MS))
    }
    fn _ismbbalpha [yes] / _ismbbalpha_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(byte_class(lo, c) & ctype::ALPHA != 0 || c <= 0xFF && m.has(c as u8, mbcs::MS))
    }
    fn _ismbbblank [no] / _ismbbblank_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(c == 9 || byte_class(lo, c) & ctype::BLANK != 0)
    }
    fn _ismbbgraph [yes] / _ismbbgraph_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(byte_class(lo, c) & ctype::GRAPH != 0 || c <= 0xFF && m.has(c as u8, mbcs::MS | mbcs::MP))
    }
    fn _ismbbprint [yes] / _ismbbprint_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(byte_class(lo, c) & ctype::PRINT != 0 || c <= 0xFF && m.has(c as u8, mbcs::MS | mbcs::MP))
    }
    fn _ismbbpunct [yes] / _ismbbpunct_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(byte_class(lo, c) & ctype::PUNCT != 0 || c <= 0xFF && m.has(c as u8, mbcs::MP))
    }
    fn _ismbbkalnum [yes] / _ismbbkalnum_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(c <= 0xFF && m.has(c as u8, mbcs::MS))
    }
    fn _ismbbkprint [yes] / _ismbbkprint_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(c <= 0xFF && m.has(c as u8, mbcs::MS | mbcs::MP))
    }
    fn _ismbbkpunct [yes] / _ismbbkpunct_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(c <= 0xFF && m.has(c as u8, mbcs::MP))
    }
    fn _ismbbkana [yes] / _ismbbkana_l [yes] (c: u32) -> i32, |m, ch, lo| {
        i32::from(c <= 0xFF && m.is_kana(c as u8))
    }
    fn _ismbblead [yes] / _ismbblead_l [yes] (c: u32) -> i32, |m, ch, lo| {
        i32::from(c <= 0xFF && m.is_lead(c as u8))
    }
    fn _ismbbtrail [yes] / _ismbbtrail_l [yes] (c: u32) -> i32, |m, ch, lo| {
        i32::from(c <= 0xFF && m.is_trail(c as u8))
    }
    fn _mbbtype [yes] / _mbbtype_l [no] (c: u8, previous: i32) -> i32, |m, ch, lo| {
        m.btype(c, previous)
    }
    fn _mbbtombc [yes] / _mbbtombc_l [yes] (c: u32) -> u32, |m, ch, lo| {
        mbcs::to_double(m, c, &ch)
    }
    fn _mbctombb [yes] / _mbctombb_l [yes] (c: u32) -> u32, |m, ch, lo| {
        mbcs::to_single(m, c, &ch)
    }
}

// ----- characters -------------------------------------------------------------------------

fn char_is(c: u32, ch: &Kernel, mask: u16) -> i32 {
    i32::from(mbcs::char_class(c, ch) & mask != 0)
}

mb! {
    fn _ismbcalnum [yes] / _ismbcalnum_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::ALNUM) }
    fn _ismbcalpha [yes] / _ismbcalpha_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::ALPHA) }
    fn _ismbcdigit [yes] / _ismbcdigit_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::DIGIT) }
    fn _ismbcgraph [yes] / _ismbcgraph_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::GRAPH) }
    fn _ismbclower [yes] / _ismbclower_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::LOWER) }
    fn _ismbcupper [yes] / _ismbcupper_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::UPPER) }
    fn _ismbcprint [yes] / _ismbcprint_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::PRINT) }
    fn _ismbcpunct [yes] / _ismbcpunct_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::PUNCT) }
    fn _ismbcspace [yes] / _ismbcspace_l [yes] (c: u32) -> i32, |m, ch, lo| { char_is(c, &ch, ctype::SPACE) }
    fn _ismbcblank [no] / _ismbcblank_l [no] (c: u32) -> i32, |m, ch, lo| {
        i32::from(c == 9 || char_is(c, &ch, ctype::BLANK) != 0)
    }
    fn _ismbclegal [yes] / _ismbclegal_l [yes] (c: u32) -> i32, |m, ch, lo| { i32::from(mbcs::is_legal(m, c)) }
    fn _ismbchira [yes] / _ismbchira_l [yes] (c: u32) -> i32, |m, ch, lo| {
        i32::from(m.code_page == 932 && mbcs::is_hiragana(c))
    }
    fn _ismbckata [yes] / _ismbckata_l [yes] (c: u32) -> i32, |m, ch, lo| {
        i32::from(m.code_page == 932 && mbcs::is_katakana(c))
    }
    fn _ismbcsymbol [yes] / _ismbcsymbol_l [yes] (c: u32) -> i32, |m, ch, lo| {
        i32::from(m.code_page == 932 && mbcs::is_symbol(c))
    }
    fn _ismbcl0 [yes] / _ismbcl0_l [yes] (c: u32) -> i32, |m, ch, lo| { i32::from(mbcs::jis_level(c, m) == Some(0)) }
    fn _ismbcl1 [yes] / _ismbcl1_l [yes] (c: u32) -> i32, |m, ch, lo| { i32::from(mbcs::jis_level(c, m) == Some(1)) }
    fn _ismbcl2 [yes] / _ismbcl2_l [yes] (c: u32) -> i32, |m, ch, lo| { i32::from(mbcs::jis_level(c, m) == Some(2)) }
    fn _mbctolower [yes] / _mbctolower_l [yes] (c: u32) -> u32, |m, ch, lo| { m.to_case(c, false, &ch) }
    fn _mbctoupper [yes] / _mbctoupper_l [yes] (c: u32) -> u32, |m, ch, lo| { m.to_case(c, true, &ch) }
    fn _mbctohira [yes] / _mbctohira_l [yes] (c: u32) -> u32, |m, ch, lo| {
        if m.code_page == 932 { mbcs::to_hiragana(c) } else { c }
    }
    fn _mbctokata [yes] / _mbctokata_l [yes] (c: u32) -> u32, |m, ch, lo| {
        if m.code_page == 932 { mbcs::to_katakana(c) } else { c }
    }
    fn _mbcjistojms [yes] / _mbcjistojms_l [yes] (c: u32) -> u32, |m, ch, lo| {
        if m.code_page == 932 { mbcs::jis_to_sjis(c) } else { c }
    }
    fn _mbcjmstojis [yes] / _mbcjmstojis_l [yes] (c: u32) -> u32, |m, ch, lo| {
        if m.code_page == 932 { mbcs::sjis_to_jis(c) } else { c }
    }
    fn _mbclen [yes] / _mbclen_l [yes] (p: *const u8) -> usize, |m, ch, lo| {
        if p.is_null() && null_arg() { return 0; }
        m.clen(*p)
    }
    fn _mbccpy [yes] / _mbccpy_l [yes] (dst: *mut u8, src: *const u8) -> (), |m, ch, lo| {
        if (dst.is_null() || src.is_null()) && null_arg() { return; }
        *dst = *src;
        if m.is_lead(*src) && *src.add(1) != 0 {
            *dst.add(1) = *src.add(1);
        }
    }
    fn _mbccpy_s [yes] / _mbccpy_s_l [yes] (dst: *mut u8, size: usize, copied: *mut i32, src: *const u8) -> i32, |m, ch, lo| {
        if dst.is_null() || src.is_null() || size == 0 {
            global::invalid(EINVAL);
            return EINVAL;
        }
        let n = if m.is_lead(*src) && *src.add(1) != 0 { 2 } else { 1 };
        if n > size {
            *dst = 0;
            global::invalid(ERANGE);
            return ERANGE;
        }
        core::ptr::copy_nonoverlapping(src, dst, n);
        if !copied.is_null() {
            *copied = n as i32;
        }
        0
    }
}

// ----- walking and counting ----------------------------------------------------------------

mb! {
    fn _ismbslead [yes] / _ismbslead_l [yes] (s: *const u8, cur: *const u8) -> i32, |m, ch, lo| {
        if (s.is_null() || cur.is_null()) && null_arg() { return 0; }
        let t = sl(s);
        let i = cur.offset_from(s);
        if i < 0 { 0 } else if m.is_lead_at(t, i as usize) { -1 } else { 0 }
    }
    fn _ismbstrail [yes] / _ismbstrail_l [yes] (s: *const u8, cur: *const u8) -> i32, |m, ch, lo| {
        if (s.is_null() || cur.is_null()) && null_arg() { return 0; }
        let t = sl(s);
        let i = cur.offset_from(s);
        if i < 0 { 0 } else if m.is_trail_at(t, i as usize) { -1 } else { 0 }
    }
    fn _mbsbtype [yes] / _mbsbtype_l [yes] (s: *const u8, count: usize) -> i32, |m, ch, lo| {
        if s.is_null() && null_arg() { return mbcs::MBC_ILLEGAL; }
        m.byte_type(sl(s), count)
    }
    fn _mbsinc [yes] / _mbsinc_l [yes] (p: *const u8) -> *const u8, |m, ch, lo| {
        if p.is_null() && null_arg() { return p; }
        let n = if *p != 0 && m.is_lead(*p) && *p.add(1) != 0 { 2 } else { 1 };
        p.add(n)
    }
    fn _mbsninc [yes] / _mbsninc_l [no] (p: *const u8, n: usize) -> *const u8, |m, ch, lo| {
        if p.is_null() { return p; }
        p.add(m.ninc(sl(p), 0, n))
    }
    fn _mbsdec [yes] / _mbsdec_l [yes] (start: *const u8, cur: *const u8) -> *const u8, |m, ch, lo| {
        if (start.is_null() || cur.is_null()) && null_arg() { return core::ptr::null(); }
        let i = cur.offset_from(start);
        if i <= 0 { return core::ptr::null(); }
        let t = unit::slice_max(start, i as usize);
        match m.dec(t, i as usize) {
            Some(k) => start.add(k),
            None => core::ptr::null(),
        }
    }
    fn _mbsnextc [yes] / _mbsnextc_l [yes] (p: *const u8) -> u32, |m, ch, lo| {
        if p.is_null() && null_arg() { return 0; }
        m.next_char(sl(p))
    }
    fn _mbslen [yes] / _mbslen_l [yes] (s: *const u8) -> usize, |m, ch, lo| {
        if s.is_null() && null_arg() { return 0; }
        m.len(sl(s))
    }
    fn _mbsnlen [yes] / _mbsnlen_l [yes] (s: *const u8, max: usize) -> usize, |m, ch, lo| {
        if s.is_null() && null_arg() { return 0; }
        m.chars_in_bytes(unit::slice_max(s, max), max)
    }
    fn _mbsnbcnt [yes] / _mbsnbcnt_l [yes] (s: *const u8, n: usize) -> usize, |m, ch, lo| {
        if s.is_null() && null_arg() { return 0; }
        m.bytes_of_chars(sl(s), n)
    }
    fn _mbsnccnt [yes] / _mbsnccnt_l [yes] (s: *const u8, n: usize) -> usize, |m, ch, lo| {
        if s.is_null() && null_arg() { return 0; }
        m.chars_in_bytes(sl(s), n)
    }
}

// ----- searching --------------------------------------------------------------------------

fn at(s: *const u8, i: Option<usize>) -> *const u8 {
    // SAFETY: an index inside the caller's string (or its NUL).
    i.map_or(core::ptr::null(), |i| unsafe { s.add(i) })
}

mb! {
    fn _mbschr [yes] / _mbschr_l [yes] (s: *const u8, c: u32) -> *const u8, |m, ch, lo| {
        if s.is_null() && null_arg() { return s; }
        at(s, m.find_char(sl(s), c))
    }
    fn _mbsrchr [yes] / _mbsrchr_l [yes] (s: *const u8, c: u32) -> *const u8, |m, ch, lo| {
        if s.is_null() && null_arg() { return s; }
        at(s, m.rfind_char(sl(s), c))
    }
    fn _mbsstr [yes] / _mbsstr_l [no] (s: *const u8, needle: *const u8) -> *const u8, |m, ch, lo| {
        if (s.is_null() || needle.is_null()) && null_arg() { return core::ptr::null(); }
        at(s, m.find_str(sl(s), sl(needle)))
    }
    fn _mbspbrk [yes] / _mbspbrk_l [yes] (s: *const u8, set: *const u8) -> *const u8, |m, ch, lo| {
        if (s.is_null() || set.is_null()) && null_arg() { return core::ptr::null(); }
        at(s, m.pbrk(sl(s), sl(set)))
    }
    fn _mbsspnp [yes] / _mbsspnp_l [yes] (s: *const u8, set: *const u8) -> *const u8, |m, ch, lo| {
        if (s.is_null() || set.is_null()) && null_arg() { return core::ptr::null(); }
        at(s, m.spnp(sl(s), sl(set)))
    }
    fn _mbscspn [yes] / _mbscspn_l [yes] (s: *const u8, set: *const u8) -> usize, |m, ch, lo| {
        if (s.is_null() || set.is_null()) && null_arg() { return 0; }
        m.span(sl(s), sl(set), false)
    }
    fn _mbsspn [yes] / _mbsspn_l [yes] (s: *const u8, set: *const u8) -> usize, |m, ch, lo| {
        if (s.is_null() || set.is_null()) && null_arg() { return 0; }
        m.span(sl(s), sl(set), true)
    }
}

// ----- comparing --------------------------------------------------------------------------

/// Collate in `lo`'s `LC_COLLATE`, the bytes read in the multibyte code
/// page (Poly's collation, `poly_nls::collate`).
fn mb_coll(m: &Mbc, lo: &Loc, a: &[u8], b: &[u8], ignore_case: bool) -> i32 {
    if lo.collate_c {
        return m.compare_bytes(a, b, usize::MAX, ignore_case);
    }
    let cp = if m.code_page == 0 { lo.collate_cp } else { m.code_page };
    let (x, y) = (crate::locale::to_wide_lossy(cp, a), crate::locale::to_wide_lossy(cp, b));
    match poly_nls::collate::compare(&x, &y, ignore_case) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

mb! {
    fn _mbscmp [yes] / _mbscmp_l [yes] (a: *const u8, b: *const u8) -> i32, |m, ch, lo| {
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        m.compare_bytes(sl(a), sl(b), usize::MAX, false)
    }
    fn _mbsicmp [yes] / _mbsicmp_l [yes] (a: *const u8, b: *const u8) -> i32, |m, ch, lo| {
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        m.compare_bytes(sl(a), sl(b), usize::MAX, true)
    }
    fn _mbsncmp [yes] / _mbsncmp_l [yes] (a: *const u8, b: *const u8, n: usize) -> i32, |m, ch, lo| {
        if n == 0 { return 0; }
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        m.compare_chars(sl(a), sl(b), n, false)
    }
    fn _mbsnicmp [yes] / _mbsnicmp_l [yes] (a: *const u8, b: *const u8, n: usize) -> i32, |m, ch, lo| {
        if n == 0 { return 0; }
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        m.compare_chars(sl(a), sl(b), n, true)
    }
    fn _mbsnbcmp [yes] / _mbsnbcmp_l [yes] (a: *const u8, b: *const u8, n: usize) -> i32, |m, ch, lo| {
        if n == 0 { return 0; }
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        m.compare_bytes(sl(a), sl(b), n, false)
    }
    fn _mbsnbicmp [yes] / _mbsnbicmp_l [yes] (a: *const u8, b: *const u8, n: usize) -> i32, |m, ch, lo| {
        if n == 0 { return 0; }
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        m.compare_bytes(sl(a), sl(b), n, true)
    }
    fn _mbscoll [yes] / _mbscoll_l [yes] (a: *const u8, b: *const u8) -> i32, |m, ch, lo| {
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        mb_coll(m, lo, sl(a), sl(b), false)
    }
    fn _mbsicoll [yes] / _mbsicoll_l [yes] (a: *const u8, b: *const u8) -> i32, |m, ch, lo| {
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        mb_coll(m, lo, sl(a), sl(b), true)
    }
    fn _mbsncoll [yes] / _mbsncoll_l [no] (a: *const u8, b: *const u8, n: usize) -> i32, |m, ch, lo| {
        if n == 0 { return 0; }
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        let (x, y) = (sl(a), sl(b));
        mb_coll(m, lo, &x[..m.bytes_of_chars(x, n)], &y[..m.bytes_of_chars(y, n)], false)
    }
    fn _mbsnicoll [yes] / _mbsnicoll_l [no] (a: *const u8, b: *const u8, n: usize) -> i32, |m, ch, lo| {
        if n == 0 { return 0; }
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        let (x, y) = (sl(a), sl(b));
        mb_coll(m, lo, &x[..m.bytes_of_chars(x, n)], &y[..m.bytes_of_chars(y, n)], true)
    }
    fn _mbsnbcoll [yes] / _mbsnbcoll_l [yes] (a: *const u8, b: *const u8, n: usize) -> i32, |m, ch, lo| {
        if n == 0 { return 0; }
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        mb_coll(m, lo, unit::slice_max(a, n), unit::slice_max(b, n), false)
    }
    fn _mbsnbicoll [yes] / _mbsnbicoll_l [yes] (a: *const u8, b: *const u8, n: usize) -> i32, |m, ch, lo| {
        if n == 0 { return 0; }
        if (a.is_null() || b.is_null()) && null_arg() { return NLSCMPERROR; }
        mb_coll(m, lo, unit::slice_max(a, n), unit::slice_max(b, n), true)
    }
}

// ----- changing in place ------------------------------------------------------------------

/// The `_s` check of an in-place function: `size` holds the string and
/// its NUL.
///
/// # Safety
/// `s` NULL or `size` readable bytes.
unsafe fn fits_s(s: *mut u8, size: usize) -> bool {
    if s.is_null() || size == 0 {
        global::invalid(EINVAL);
        return false;
    }
    // SAFETY: up to `size` bytes are the caller's.
    if unsafe { unit::slice_max(s, size) }.len() >= size {
        // SAFETY: at least one byte.
        unsafe { s.write(0) };
        global::invalid(EINVAL);
        return false;
    }
    true
}

mb! {
    fn _mbslwr [yes] / _mbslwr_l [yes] (s: *mut u8) -> *mut u8, |m, ch, lo| {
        if s.is_null() && null_arg() { return s; }
        m.convert_case(slm(s), false, &ch);
        s
    }
    fn _mbsupr [yes] / _mbsupr_l [yes] (s: *mut u8) -> *mut u8, |m, ch, lo| {
        if s.is_null() && null_arg() { return s; }
        m.convert_case(slm(s), true, &ch);
        s
    }
    fn _mbslwr_s [yes] / _mbslwr_s_l [yes] (s: *mut u8, size: usize) -> i32, |m, ch, lo| {
        if !fits_s(s, size) { return EINVAL; }
        m.convert_case(slm(s), false, &ch);
        0
    }
    fn _mbsupr_s [yes] / _mbsupr_s_l [yes] (s: *mut u8, size: usize) -> i32, |m, ch, lo| {
        if !fits_s(s, size) { return EINVAL; }
        m.convert_case(slm(s), true, &ch);
        0
    }
    fn _mbsrev [yes] / _mbsrev_l [yes] (s: *mut u8) -> *mut u8, |m, ch, lo| {
        if s.is_null() && null_arg() { return s; }
        m.reverse(slm(s));
        s
    }
    fn _mbsset [yes] / _mbsset_l [yes] (s: *mut u8, c: u32) -> *mut u8, |m, ch, lo| {
        if s.is_null() && null_arg() { return s; }
        let t = slm(s);
        let n = t.len();
        m.fill(t, c, n);
        s
    }
    fn _mbsnset [yes] / _mbsnset_l [yes] (s: *mut u8, c: u32, n: usize) -> *mut u8, |m, ch, lo| {
        if s.is_null() && null_arg() { return s; }
        let t = slm(s);
        let bytes = n.saturating_mul(if c > 0xFF { 2 } else { 1 });
        m.fill(t, c, bytes);
        s
    }
    fn _mbsnbset [yes] / _mbsnbset_l [yes] (s: *mut u8, c: u32, n: usize) -> *mut u8, |m, ch, lo| {
        if s.is_null() && null_arg() { return s; }
        m.fill(slm(s), c, n);
        s
    }
    fn _mbsset_s [no] / _mbsset_s_l [no] (s: *mut u8, size: usize, c: u32) -> i32, |m, ch, lo| {
        if !fits_s(s, size) { return EINVAL; }
        let t = slm(s);
        let n = t.len();
        m.fill(t, c, n);
        0
    }
    fn _mbsnset_s [no] / _mbsnset_s_l [no] (s: *mut u8, size: usize, c: u32, n: usize) -> i32, |m, ch, lo| {
        if !fits_s(s, size) { return EINVAL; }
        m.fill(slm(s), c, n.saturating_mul(if c > 0xFF { 2 } else { 1 }));
        0
    }
    fn _mbsnbset_s [no] / _mbsnbset_s_l [no] (s: *mut u8, size: usize, c: u32, n: usize) -> i32, |m, ch, lo| {
        if !fits_s(s, size) { return EINVAL; }
        m.fill(slm(s), c, n);
        0
    }
}

// ----- copying and appending ---------------------------------------------------------------

/// Copy `n` bytes of `src` to `dst`, then `pad` NULs.
///
/// # Safety
/// The caller's buffers hold them.
unsafe fn put(dst: *mut u8, src: &[u8], n: usize, pad: usize) {
    // SAFETY: the caller's contract.
    unsafe {
        core::ptr::copy(src.as_ptr(), dst, n);
        core::ptr::write_bytes(dst.add(n), 0, pad);
    }
}

/// The `_s` rule of the copying functions: `add` bytes (or, with
/// `_TRUNCATE`, as many whole characters as fit) after `at` bytes of a
/// `size`-byte buffer, then the NUL.
///
/// # Safety
/// `dst` holds `size` bytes; `src` the caller's string.
unsafe fn append_s(m: &Mbc, dst: *mut u8, size: usize, at: usize, src: &[u8], add: usize, truncate: bool) -> i32 {
    if at + add < size {
        // SAFETY: at + add + 1 <= size.
        unsafe { put(dst.add(at), src, add, 1) };
        return 0;
    }
    if truncate {
        let keep = m.whole_bytes(&src[..add], size - 1 - at.min(size - 1));
        // SAFETY: at + keep + 1 <= size.
        unsafe { put(dst.add(at), src, keep, 1) };
        return STRUNCATE;
    }
    // SAFETY: size >= 1.
    unsafe { dst.write(0) };
    global::invalid(ERANGE);
    ERANGE
}

mb! {
    fn _mbsncpy [yes] / _mbsncpy_l [yes] (dst: *mut u8, src: *const u8, n: usize) -> *mut u8, |m, ch, lo| {
        if (dst.is_null() || src.is_null()) && n > 0 && null_arg() { return dst; }
        if n == 0 { return dst; }
        let s = sl(src);
        let bytes = m.bytes_of_chars(s, n);
        let chars = m.len(&s[..bytes]);
        put(dst, s, bytes, n - chars);
        dst
    }
    fn _mbsnbcpy [yes] / _mbsnbcpy_l [yes] (dst: *mut u8, src: *const u8, n: usize) -> *mut u8, |m, ch, lo| {
        if (dst.is_null() || src.is_null()) && n > 0 && null_arg() { return dst; }
        if n == 0 { return dst; }
        let s = sl(src);
        let bytes = m.whole_bytes(s, n);
        put(dst, s, bytes, n - bytes);
        dst
    }
    fn _mbsncat [yes] / _mbsncat_l [yes] (dst: *mut u8, src: *const u8, n: usize) -> *mut u8, |m, ch, lo| {
        if (dst.is_null() || src.is_null()) && n > 0 && null_arg() { return dst; }
        if n == 0 { return dst; }
        let s = sl(src);
        put(dst.add(unit::len(dst)), s, m.bytes_of_chars(s, n), 1);
        dst
    }
    fn _mbsnbcat [yes] / _mbsnbcat_l [yes] (dst: *mut u8, src: *const u8, n: usize) -> *mut u8, |m, ch, lo| {
        if (dst.is_null() || src.is_null()) && n > 0 && null_arg() { return dst; }
        if n == 0 { return dst; }
        let s = sl(src);
        put(dst.add(unit::len(dst)), s, m.whole_bytes(s, n), 1);
        dst
    }
    fn _mbsncpy_s [no] / _mbsncpy_s_l [no] (dst: *mut u8, size: usize, src: *const u8, n: usize) -> i32, |m, ch, lo| {
        if dst.is_null() || size == 0 || src.is_null() {
            global::invalid(EINVAL);
            return EINVAL;
        }
        let s = sl(src);
        let add = m.bytes_of_chars(s, n);
        append_s(m, dst, size, 0, s, add, n == TRUNCATE)
    }
    fn _mbsnbcpy_s [yes] / _mbsnbcpy_s_l [yes] (dst: *mut u8, size: usize, src: *const u8, n: usize) -> i32, |m, ch, lo| {
        if dst.is_null() || size == 0 || src.is_null() {
            global::invalid(EINVAL);
            return EINVAL;
        }
        let s = sl(src);
        let add = m.whole_bytes(s, n);
        append_s(m, dst, size, 0, s, add, n == TRUNCATE)
    }
    fn _mbsncat_s [no] / _mbsncat_s_l [no] (dst: *mut u8, size: usize, src: *const u8, n: usize) -> i32, |m, ch, lo| {
        if dst.is_null() || size == 0 || src.is_null() {
            global::invalid(EINVAL);
            return EINVAL;
        }
        let s = sl(src);
        let at = unit::slice_max(dst, size).len();
        if at >= size {
            *dst = 0;
            global::invalid(EINVAL);
            return EINVAL;
        }
        append_s(m, dst, size, at, s, m.bytes_of_chars(s, n), n == TRUNCATE)
    }
    fn _mbsnbcat_s [yes] / _mbsnbcat_s_l [yes] (dst: *mut u8, size: usize, src: *const u8, n: usize) -> i32, |m, ch, lo| {
        if dst.is_null() || size == 0 || src.is_null() {
            global::invalid(EINVAL);
            return EINVAL;
        }
        let s = sl(src);
        let at = unit::slice_max(dst, size).len();
        if at >= size {
            *dst = 0;
            global::invalid(EINVAL);
            return EINVAL;
        }
        append_s(m, dst, size, at, s, m.whole_bytes(s, n), n == TRUNCATE)
    }
    fn _mbscpy_s [yes] / _mbscpy_s_l [yes] (dst: *mut u8, size: usize, src: *const u8) -> i32, |m, ch, lo| {
        crate::strings::strcpy_s(dst, size, src)
    }
    fn _mbscat_s [yes] / _mbscat_s_l [yes] (dst: *mut u8, size: usize, src: *const u8) -> i32, |m, ch, lo| {
        crate::strings::strcat_s(dst, size, src)
    }
}

/// `_mbsdup` is `_strdup`.
///
/// # Safety
/// `p` NULL or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mbsdup(p: *const u8) -> *mut u8 {
    // SAFETY: the caller's string.
    unsafe { crate::strings::_strdup(p) }
}

/// `_mbscpy` / `_mbscat` (`msvcrt`) are `strcpy` / `strcat`.
///
/// # Safety
/// As `strcpy`.
pub unsafe extern "C" fn _mbscpy(dst: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: the caller's buffers.
    unsafe { crate::strings::strcpy(dst, src) }
}

/// # Safety
/// As `strcat`.
pub unsafe extern "C" fn _mbscat(dst: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: the caller's buffers.
    unsafe { crate::strings::strcat(dst, src) }
}

// ----- tokens -----------------------------------------------------------------------------

/// The work of `_mbstok_s`: `context` holds where the next call goes on.
///
/// # Safety
/// `s` NULL or a writable C string; `delims` a C string; `context`
/// writable.
unsafe fn tok(m: &Mbc, s: *mut u8, delims: *const u8, context: *mut *mut u8) -> *mut u8 {
    if delims.is_null() || context.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's context.
    let base = if s.is_null() { unsafe { *context } } else { s };
    if base.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    let (t, d) = unsafe { (sl(base), sl(delims)) };
    let Some((start, end)) = m.token(t, 0, d) else {
        // SAFETY: the caller's context; the NUL.
        unsafe { *context = base.add(t.len()) };
        return core::ptr::null_mut();
    };
    // SAFETY: indices inside the string; the delimiter becomes a NUL (a
    // double-byte delimiter: both bytes).
    unsafe {
        if end < t.len() {
            let n = m.char_at(t, end).1;
            core::ptr::write_bytes(base.add(end), 0, n);
            *context = base.add(end + n);
        } else {
            *context = base.add(end);
        }
        base.add(start)
    }
}

mb! {
    fn _mbstok_s [yes] / _mbstok_s_l [yes] (s: *mut u8, delims: *const u8, context: *mut *mut u8) -> *mut u8, |m, ch, lo| {
        tok(m, s, delims, context)
    }
    fn _mbstok [yes] / _mbstok_l [yes] (s: *mut u8, delims: *const u8) -> *mut u8, |m, ch, lo| {
        let t = global::thread();
        tok(m, s, delims, &raw mut t.strtok)
    }
}
