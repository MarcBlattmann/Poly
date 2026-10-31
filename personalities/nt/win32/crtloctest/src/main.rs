//! `crtloctest.exe` (ADR 0094): the program the kernel's ring-3 test
//! `nt_c_runtime_locales_and_code_pages` runs. It imports the UCRT through
//! the `api-ms-win-crt-*` names as an MSVC-built program does and checks
//! locales other than "C" end to end — `setlocale` names, `localeconv`,
//! character classes and case by `LC_CTYPE`, `printf` and `strtod` with a
//! decimal comma, `mbstowcs`/`wcstombs`/`mbrtowc` in single-byte,
//! double-byte and UTF-8 locales (so `kernelbase`'s code page tables), the
//! `_mbs*` functions on code page 932, `_create_locale` and the `_l`
//! forms, collation, a thread's own locale — and `msvcrt.dll`'s locale
//! data. It prints `crtloctest: ok` and ends with [`PASS`]; a failed check
//! prints `crtloctest: step N failed` and ends with `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

/// The exit status when every check passed.
const PASS: u32 = 0x4C;

type Loc = *mut u8;

#[link(name = "api-ms-win-crt-locale-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn setlocale(c: i32, l: *const u8) -> *const u8;
    fn _wsetlocale(c: i32, l: *const u16) -> *const u16;
    fn localeconv() -> *const *const u8;
    fn _create_locale(c: i32, l: *const u8) -> Loc;
    fn _free_locale(l: Loc);
    fn ___lc_codepage_func() -> u32;
    fn ___mb_cur_max_func() -> i32;
    fn _configthreadlocale(m: i32) -> i32;
}

#[link(name = "api-ms-win-crt-string-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn isalpha(c: i32) -> i32;
    fn isupper(c: i32) -> i32;
    fn toupper(c: i32) -> i32;
    fn tolower(c: i32) -> i32;
    fn isleadbyte(c: i32) -> i32;
    fn towupper(c: u16) -> u16;
    fn strcoll(a: *const u8, b: *const u8) -> i32;
    fn strxfrm(d: *mut u8, s: *const u8, n: usize) -> usize;
    fn _stricmp(a: *const u8, b: *const u8) -> i32;
    fn _isalpha_l(c: i32, l: Loc) -> i32;
    fn _toupper_l(c: i32, l: Loc) -> i32;
}

#[link(name = "api-ms-win-crt-convert-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn strtod(s: *const u8, e: *mut *mut u8) -> f64;
    fn _strtod_l(s: *const u8, e: *mut *mut u8, l: Loc) -> f64;
    fn mbstowcs(d: *mut u16, s: *const u8, n: usize) -> usize;
    fn wcstombs(d: *mut u8, s: *const u16, n: usize) -> usize;
    fn mbrtowc(d: *mut u16, s: *const u8, n: usize, st: *mut u64) -> usize;
    fn wctomb(d: *mut u8, c: u16) -> i32;
    fn _mbstrlen(s: *const u8) -> usize;
}

#[link(name = "api-ms-win-crt-multibyte-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn _setmbcp(cp: i32) -> i32;
    fn _getmbcp() -> i32;
    fn _mbslen(s: *const u8) -> usize;
    fn _mbschr(s: *const u8, c: u32) -> *const u8;
    fn _mbsrev(s: *mut u8) -> *mut u8;
    fn _mbsupr(s: *mut u8) -> *mut u8;
    fn _ismbblead(c: u32) -> i32;
    fn _mbctohira(c: u32) -> u32;
    fn _mbcjistojms(c: u32) -> u32;
    fn _mbsnbcnt(s: *const u8, n: usize) -> usize;
    fn _mbsdec(s: *const u8, c: *const u8) -> *const u8;
}

#[link(name = "api-ms-win-crt-stdio-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn __stdio_common_vsprintf(o: u64, b: *mut u8, n: usize, f: *const u8, l: *mut u8, a: *const u64) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetStdHandle(which: u32) -> isize;
    fn WriteFile(h: isize, b: *const u8, n: u32, w: *mut u32, o: *mut u8) -> i32;
    fn LoadLibraryW(n: *const u16) -> *mut u8;
    fn GetProcAddress(m: *mut u8, n: *const u8) -> *mut u8;
    fn CreateThread(
        sa: *const u8,
        st: usize,
        f: extern "system" fn(*mut u8) -> u32,
        a: *mut u8,
        fl: u32,
        id: *mut u32,
    ) -> usize;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn CloseHandle(h: usize) -> i32;
}

/// `LC_ALL` … (`locale.h`).
const LC_ALL: i32 = 0;
const LC_COLLATE: i32 = 1;
const LC_CTYPE: i32 = 2;
const LC_NUMERIC: i32 = 4;

static STEP: AtomicU32 = AtomicU32::new(0);
static THREAD_OK: AtomicU32 = AtomicU32::new(0);

fn say(text: &[u8]) {
    let mut n = 0;
    // SAFETY: kernel32 with a valid buffer.
    unsafe {
        WriteFile(GetStdHandle((-11i32) as u32), text.as_ptr(), text.len() as u32, &mut n, core::ptr::null_mut())
    };
}

fn fail(step: u32) -> ! {
    let mut msg = *b"crtloctest: step 000 failed\r\n";
    msg[17] = b'0' + (step / 100 % 10) as u8;
    msg[18] = b'0' + (step / 10 % 10) as u8;
    msg[19] = b'0' + (step % 10) as u8;
    say(&msg);
    // SAFETY: no preconditions.
    unsafe { ExitProcess(0x80 + step) }
}

/// The next step: fail it unless `ok`.
fn check(ok: bool) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        fail(step);
    }
}

/// A C string's bytes (without the NUL).
fn cs<'a, T: Copy + Default + PartialEq>(p: *const T) -> &'a [T] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: a C string from the runtime.
    while unsafe { *p.add(n) } != T::default() {
        n += 1;
    }
    // SAFETY: n units are readable.
    unsafe { core::slice::from_raw_parts(p, n) }
}

fn set(c: i32, name: &core::ffi::CStr) -> &'static [u8] {
    // SAFETY: a C string.
    cs(unsafe { setlocale(c, name.as_ptr() as *const u8) })
}

/// `__stdio_common_vsprintf` (the UCRT headers' default options) into a
/// 64-byte buffer.
fn fmt(format: &core::ffi::CStr, args: &[u64]) -> ([u8; 64], usize) {
    let mut buf = [0u8; 64];
    // SAFETY: a valid buffer, a C format and its arguments.
    let n = unsafe {
        __stdio_common_vsprintf(
            4 | 32,
            buf.as_mut_ptr(),
            64,
            format.as_ptr() as *const u8,
            core::ptr::null_mut(),
            args.as_ptr(),
        )
    };
    (buf, n.max(0) as usize)
}

extern "system" fn own_locale(_: *mut u8) -> u32 {
    // SAFETY: the runtime's functions with valid arguments.
    unsafe {
        let mut ok = _configthreadlocale(1) == 2;
        ok &= set(LC_ALL, c"de-DE") == b"de-DE";
        ok &= cs(*localeconv()) == b",";
        THREAD_OK.store(u32::from(ok), Ordering::Release);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn crtloctest_main() -> u32 {
    // SAFETY: the runtime's documented functions, each with valid arguments.
    unsafe { run() }
}

unsafe fn run() -> ! {
    // SAFETY: the whole body calls the runtime with valid arguments.
    unsafe {
        // ----- names -----
        check(cs(setlocale(LC_ALL, core::ptr::null())) == b"C"); // 1
        check(set(LC_ALL, c"") == b"English_United States.1252");
        check(set(LC_ALL, c"German_Switzerland.850") == b"German_Switzerland.850");
        check(___lc_codepage_func() == 850);
        check(set(LC_ALL, c"german") == b"German_Germany.1252"); // 5
        check(set(LC_ALL, c".OCP") == b"English_United States.437");
        check(set(LC_ALL, c"de-CH") == b"de-CH" && ___lc_codepage_func() == 1252);
        check(setlocale(LC_ALL, c"Klingon_Qo'noS".as_ptr() as *const u8).is_null());
        check(set(LC_ALL, c"") == b"English_United States.1252");
        let w: [u16; 6] = [b'f' as u16, b'r' as u16, b'-' as u16, b'F' as u16, b'R' as u16, 0];
        check(*cs(_wsetlocale(LC_ALL, w.as_ptr())) == w[..5]); // 10

        // ----- LC_NUMERIC: localeconv, printf, strtod -----
        check(set(LC_ALL, c"C") == b"C");
        check(set(LC_NUMERIC, c"de-DE") == b"de-DE");
        check(set(LC_ALL, c"") == b"English_United States.1252");
        check(set(LC_NUMERIC, c"German_Germany.1252") == b"German_Germany.1252");
        check(cs(setlocale(LC_ALL, core::ptr::null())).starts_with(b"LC_COLLATE=English_United States.1252;")); // 15
        let lc = localeconv();
        check(cs(*lc) == b"," && cs(*lc.add(1)) == b".");
        let (b, n) = fmt(c"%.2f|%g", &[1.5f64.to_bits(), 0.25f64.to_bits()]);
        check(&b[..n] == b"1,50|0,25");
        check(strtod(c"3,25".as_ptr() as *const u8, core::ptr::null_mut()) == 3.25);
        check(set(LC_NUMERIC, c"C") == b"C");
        let (b, n) = fmt(c"%.1f", &[2.5f64.to_bits()]);
        check(&b[..n] == b"2.5"); // 20

        // ----- LC_CTYPE: classes and case -----
        check(set(LC_ALL, c"C") == b"C");
        check(isalpha(0xE9) == 0 && toupper(0xE9) == 0xE9);
        check(set(LC_ALL, c".1252") == b"English_United States.1252");
        check(isalpha(0xE9) != 0 && toupper(0xE9) == 0xC9 && tolower(0xC9) == 0xE9 && isupper(0xC9) != 0);
        check(_stricmp(c"\xC9t\xC9".as_ptr() as *const u8, c"\xE9T\xE9".as_ptr() as *const u8) == 0); // 25
        check(towupper(0x3B1) == 0x391);
        check(set(LC_ALL, c"Russian_Russia.1251") == b"Russian_Russia.1251");
        check(toupper(0xE0) == 0xC0 && isalpha(0xE0) != 0);

        // ----- conversions in single-byte, double-byte and UTF-8 locales -----
        check(set(LC_ALL, c".1252") == b"English_United States.1252");
        let mut wbuf = [0u16; 8];
        check(mbstowcs(wbuf.as_mut_ptr(), c"\xE9\x80".as_ptr() as *const u8, 8) == 2 && wbuf[..2] == [0xE9, 0x20AC]); // 30
        let mut nbuf = [0u8; 8];
        let euro: [u16; 2] = [0x20AC, 0];
        check(wcstombs(nbuf.as_mut_ptr(), euro.as_ptr(), 8) == 1 && nbuf[0] == 0x80);
        check(wctomb(nbuf.as_mut_ptr(), 0x4E2D) == -1);
        check(set(LC_ALL, c"ja-JP") == b"ja-JP" && ___mb_cur_max_func() == 2 && ___lc_codepage_func() == 932);
        check(isleadbyte(0x93) != 0 && isleadbyte(b'A' as i32) == 0);
        let nihon = c"\x93\xFA\x96\x7B";
        check(mbstowcs(wbuf.as_mut_ptr(), nihon.as_ptr() as *const u8, 8) == 2 && wbuf[..2] == [0x65E5, 0x672C]); // 35
        let units: [u16; 3] = [0x65E5, 0x672C, 0];
        check(wcstombs(nbuf.as_mut_ptr(), units.as_ptr(), 8) == 4 && nbuf[..4] == *b"\x93\xFA\x96\x7B");
        let mut state = 0u64;
        let mut wc = 0u16;
        check(mbrtowc(&mut wc, b"\x93".as_ptr(), 1, &mut state) == usize::MAX - 1);
        check(mbrtowc(&mut wc, b"\xFA".as_ptr(), 1, &mut state) == 1 && wc == 0x65E5);
        check(_mbstrlen(c"a\x93\xFA".as_ptr() as *const u8) == 2);
        check(set(LC_ALL, c".utf8") == b"English_United States.utf8" && ___mb_cur_max_func() == 4); // 40
        check(
            mbstowcs(wbuf.as_mut_ptr(), c"\xE2\x82\xACx".as_ptr() as *const u8, 8) == 2 && wbuf[..2] == [0x20AC, 0x78],
        );
        check(wcstombs(nbuf.as_mut_ptr(), euro.as_ptr(), 8) == 3 && nbuf[..3] == *b"\xE2\x82\xAC");

        // ----- the multibyte code page and the _mbs functions -----
        check(set(LC_ALL, c"C") == b"C" && _getmbcp() == 0);
        check(_setmbcp(932) == 0 && _getmbcp() == 932);
        check(_ismbblead(0x81) != 0 && _ismbblead(b'a' as u32) == 0); // 45
        check(_mbslen(c"a\x93\xFAb".as_ptr() as *const u8) == 3);
        // 0x955C (表) has '\\' as its trail byte: not a match.
        let hyo = c"\x95\x5Cx\\";
        check(_mbschr(hyo.as_ptr() as *const u8, u32::from(b'\\')) == (hyo.as_ptr() as *const u8).add(3));
        let mut rev = *b"a\x93\xFAb\0";
        _mbsrev(rev.as_mut_ptr());
        check(rev == *b"b\x93\xFAa\0");
        let mut up = *b"ab\x82\x81\0";
        _mbsupr(up.as_mut_ptr());
        check(up == *b"AB\x82\x60\0");
        check(_mbctohira(0x8341) == 0x82A0 && _mbcjistojms(0x2422) == 0x82A0); // 50
        let s = c"a\x93\xFAb";
        let p = s.as_ptr() as *const u8;
        check(_mbsnbcnt(p, 2) == 3 && _mbsdec(p, p.add(3)) == p.add(1));
        check(_setmbcp(12345) == -1 && _getmbcp() == 932);
        check(_setmbcp(0) == 0 && _mbslen(c"a\x93\xFAb".as_ptr() as *const u8) == 4);

        // ----- _create_locale and the _l forms -----
        let fr = _create_locale(LC_ALL, c"French_France.1252".as_ptr() as *const u8);
        check(!fr.is_null());
        check(_strtod_l(c"2,5".as_ptr() as *const u8, core::ptr::null_mut(), fr) == 2.5); // 55
        check(_isalpha_l(0xE9, fr) != 0 && _toupper_l(0xE9, fr) == 0xC9 && isalpha(0xE9) == 0);
        _free_locale(fr);
        check(_create_locale(LC_ALL, c"xx".as_ptr() as *const u8).is_null());

        // ----- collation -----
        check(strcoll(c"a".as_ptr() as *const u8, c"B".as_ptr() as *const u8) > 0); // "C": byte order
        check(set(LC_COLLATE, c"English_United States.1252") == b"English_United States.1252");
        check(strcoll(c"a".as_ptr() as *const u8, c"B".as_ptr() as *const u8) < 0); // 60
        check(strcoll(c"resume".as_ptr() as *const u8, c"r\xE9sum\xE9".as_ptr() as *const u8) < 0);
        let (mut k1, mut k2) = ([0u8; 64], [0u8; 64]);
        let n1 = strxfrm(k1.as_mut_ptr(), c"a".as_ptr() as *const u8, 64);
        let n2 = strxfrm(k2.as_mut_ptr(), c"B".as_ptr() as *const u8, 64);
        check(n1 < 64 && n2 < 64 && cs(k1.as_ptr()) < cs(k2.as_ptr()));

        // ----- a thread's own locale -----
        check(set(LC_ALL, c"C") == b"C");
        // A committed 256 KiB stack: a second thread's stack does not grow
        // past its first guard page on Poly yet (seen here; not this test's
        // subject).
        let h = CreateThread(core::ptr::null(), 0x4_0000, own_locale, core::ptr::null_mut(), 0, core::ptr::null_mut());
        check(h != 0 && WaitForSingleObject(h, 10_000) == 0);
        CloseHandle(h);
        check(THREAD_OK.load(Ordering::Acquire) == 1 && cs(setlocale(LC_ALL, core::ptr::null())) == b"C"); // 65
        check(cs(*localeconv()) == b".");

        // ----- msvcrt.dll -----
        let m =
            LoadLibraryW([b'm' as u16, b's' as u16, b'v' as u16, b'c' as u16, b'r' as u16, b't' as u16, 0].as_ptr());
        check(!m.is_null());
        let get = |n: &core::ffi::CStr| GetProcAddress(m, n.as_ptr() as *const u8);
        type SetLocale = unsafe extern "C" fn(i32, *const u8) -> *const u8;
        let msetlocale: SetLocale = core::mem::transmute(get(c"setlocale"));
        let mb_cur_max = get(c"__mb_cur_max") as *const i32;
        let pctype = get(c"_pctype") as *const *const u16;
        let mbctype = get(c"_mbctype") as *const u8;
        check(!mb_cur_max.is_null() && !pctype.is_null() && !mbctype.is_null());
        check(cs(msetlocale(LC_ALL, c"Japanese_Japan.932".as_ptr() as *const u8)) == b"Japanese_Japan.932");
        check(*mb_cur_max == 2 && *(*pctype).add(0x93) & 0x8000 != 0); // 70
        check(*mbctype.add(0x93 + 1) & 0x04 != 0);
        check(msetlocale(LC_CTYPE, c".utf8".as_ptr() as *const u8).is_null());

        say(b"crtloctest: ok\r\n");
        ExitProcess(PASS)
    }
}
