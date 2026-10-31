//! The runtime's locales (ADR 0094): the global and per-thread locale and
//! multibyte code page, `setlocale`, `localeconv`, `_create_locale` and
//! the `_locale_t` objects, `_setmbcp`, and the data the headers read.
//!
//! **Objects.** A locale ([`Loc`]) is made once per distinct set of
//! category settings and kept for the life of the process (interned), so
//! every pointer handed out (`setlocale`'s string, `localeconv`'s `lconv`,
//! `__pctype_func`'s table) stays valid whatever another thread sets
//! later. The same for the multibyte code page's tables ([`MbcData`]). A
//! `_locale_t` is a small block of ours (`_free_locale` frees it) whose
//! first two members are what `corecrt.h`'s `__crt_locale_pointers`
//! declares: `locinfo`, `mbcinfo`.
//!
//! **Layouts the headers read.** For `ucrtbase` `locinfo` points at
//! `corecrt.h`'s `__crt_locale_data_public` (`_locale_pctype`,
//! `_locale_mb_cur_max`, `_locale_lc_codepage`), which `ctype.h`'s inline
//! functions read. For `msvcrt` it points at the classic
//! `threadlocaleinfostruct` of the Visual C++ 2005-2013 `crtdefs.h`
//! (x64 offsets checked below), and `mbcinfo` at its
//! `threadmbcinfostruct`.
//!
//! **Conversions** go through `kernel32` (`MultiByteToWideChar`,
//! `WideCharToMultiByte`): the code page tables are `kernelbase`'s
//! (`poly_nls`); the runtime links only their lead and trail byte ranges.

use crate::global::{self, CritSec, Global, Lock};
use crate::sys;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use poly_crt::errno::EINVAL;
use poly_crt::locale::{self as lc, Ctype, LC_ALL, LC_COLLATE, LC_CTYPE, LC_MAX, LC_MONETARY, LC_NUMERIC, Settings};
use poly_crt::mbcs::{self, Chars, Mbc};

/// `MB_ERR_INVALID_CHARS`, `WC_ERR_INVALID_CHARS` (`winnls.h`).
const MB_ERR_INVALID_CHARS: u32 = 0x8;
const WC_ERR_INVALID_CHARS: u32 = 0x80;

// ----- conversions -----------------------------------------------------------------------

/// Bytes of code page `cp` (0: the "C" locale, ISO-8859-1) to UTF-16;
/// `None` if they are not valid characters of it.
pub fn to_wide(cp: u32, bytes: &[u8]) -> Option<Vec<u16>> {
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    if cp == 0 {
        return Some(bytes.iter().map(|&b| u16::from(b)).collect());
    }
    let mut out = vec![0u16; bytes.len()];
    // SAFETY: both buffers have the lengths passed.
    let n = unsafe {
        sys::MultiByteToWideChar(
            cp,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            bytes.len() as i32,
            out.as_mut_ptr(),
            out.len() as i32,
        )
    };
    if n <= 0 {
        return None;
    }
    out.truncate(n as usize);
    Some(out)
}

/// As [`to_wide`], invalid input becoming the code page's default.
pub fn to_wide_lossy(cp: u32, bytes: &[u8]) -> Vec<u16> {
    if cp == 0 || bytes.is_empty() {
        return bytes.iter().map(|&b| u16::from(b)).collect();
    }
    let mut out = vec![0u16; bytes.len()];
    // SAFETY: both buffers have the lengths passed.
    let n = unsafe {
        sys::MultiByteToWideChar(cp, 0, bytes.as_ptr(), bytes.len() as i32, out.as_mut_ptr(), out.len() as i32)
    };
    out.truncate(n.max(0) as usize);
    out
}

/// UTF-16 to code page `cp`; `None` if a character has no form in it.
pub fn from_wide(cp: u32, units: &[u16]) -> Option<Vec<u8>> {
    if units.is_empty() {
        return Some(Vec::new());
    }
    if cp == 0 {
        return units.iter().map(|&u| u8::try_from(u).ok()).collect();
    }
    let mut out = vec![0u8; units.len() * 4];
    let mut used = 0i32;
    let (flags, used_ptr) =
        if cp == lc::UTF8 { (WC_ERR_INVALID_CHARS, core::ptr::null_mut()) } else { (0, &raw mut used) };
    // SAFETY: both buffers have the lengths passed; `used_ptr` is ours or
    // null (UTF-8 requires null).
    let n = unsafe {
        sys::WideCharToMultiByte(
            cp,
            flags,
            units.as_ptr(),
            units.len() as i32,
            out.as_mut_ptr(),
            out.len() as i32,
            core::ptr::null(),
            used_ptr,
        )
    };
    if n <= 0 || used != 0 {
        return None;
    }
    out.truncate(n as usize);
    Some(out)
}

/// As [`from_wide`], a missing character becoming `?`.
fn from_wide_lossy(cp: u32, units: &[u16]) -> Vec<u8> {
    from_wide(cp, units).unwrap_or_else(|| {
        units.iter().map(|&u| from_wide(cp, &[u]).and_then(|v| (v.len() == 1).then(|| v[0])).unwrap_or(b'?')).collect()
    })
}

/// A code page's characters through `kernel32`.
pub struct Kernel(pub u32);

impl Chars for Kernel {
    fn to_wide(&self, c: u32) -> Option<u16> {
        let two = [(c >> 8) as u8, c as u8];
        let bytes: &[u8] = if c > 0xFF { &two } else { &two[1..] };
        match to_wide(self.0, bytes)?.as_slice() {
            [w] => Some(*w),
            _ => None,
        }
    }
    fn char_of(&self, w: u16) -> Option<u32> {
        match from_wide(self.0, &[w])?.as_slice() {
            [b] => Some(u32::from(*b)),
            [l, t] => Some(u32::from(*l) << 8 | u32::from(*t)),
            _ => None,
        }
    }
}

/// Whether Poly converts code page `cp` (`IsValidCodePage`).
fn valid_code_page(cp: u32) -> bool {
    // SAFETY: no preconditions.
    unsafe { sys::IsValidCodePage(cp) != 0 }
}

/// The lead byte ranges of code page `cp` (none but for DBCS).
pub fn leads_of(cp: u32) -> &'static [(u8, u8)] {
    poly_nls::codepage::dbcs_ranges(cp).map_or(&[], |(l, _)| l)
}

// ----- the objects -----------------------------------------------------------------------

/// `corecrt.h`'s `__crt_locale_data_public`.
#[repr(C)]
pub struct Public {
    pub pctype: *const u16,
    pub mb_cur_max: i32,
    pub lc_codepage: u32,
}

/// One `lc_category` entry of `threadlocaleinfostruct`.
#[repr(C)]
struct LegacyCategory {
    locale: *const u8,
    wlocale: *const u16,
    refcount: *mut i32,
    wrefcount: *mut i32,
}

/// `crtdefs.h`'s `threadlocaleinfostruct` (Visual C++ 2005-2013).
#[repr(C)]
pub struct LegacyInfo {
    refcount: i32,
    lc_codepage: u32,
    lc_collate_cp: u32,
    lc_handle: [u32; 6],
    lc_id: [[u16; 3]; 6],
    lc_category: [LegacyCategory; 6],
    lc_clike: i32,
    mb_cur_max: i32,
    lconv_intl_refcount: *mut i32,
    lconv_num_refcount: *mut i32,
    lconv_mon_refcount: *mut i32,
    lconv: *const Lconv,
    ctype1_refcount: *mut i32,
    ctype1: *const u16,
    pctype: *const u16,
    pclmap: *const u8,
    pcumap: *const u8,
    lc_time_curr: *const u8,
}

const _: () = assert!(core::mem::offset_of!(LegacyInfo, lc_category) == 72);
const _: () = assert!(core::mem::offset_of!(LegacyInfo, mb_cur_max) == 268);
const _: () = assert!(core::mem::offset_of!(LegacyInfo, pctype) == 320);
const _: () = assert!(core::mem::size_of::<LegacyInfo>() == 352);

/// `struct lconv` (`locale.h`): ten `char *`, eight `char`, and the UCRT's
/// eight `wchar_t *`.
#[repr(C)]
pub struct Lconv {
    strings: [*const u8; 10],
    chars: [u8; 8],
    wide: [*const u16; 8],
}

/// A locale: every category's setting and what they decide.
#[repr(C)]
pub struct Loc {
    pub public: Public,
    legacy: LegacyInfo,
    pub lconv: Lconv,
    pub settings: Settings,
    pub ctype: Ctype,
    /// `LC_CTYPE` is "C".
    pub ctype_c: bool,
    /// `LC_CTYPE`'s and `LC_COLLATE`'s code pages (0 for "C").
    pub ctype_cp: u32,
    pub collate_cp: u32,
    pub collate_c: bool,
    /// `LC_NUMERIC`'s decimal point (its first byte / unit).
    pub point: u8,
    pub wpoint: u16,
    names: [Vec<u8>; 6],
    wnames: [Vec<u16>; 6],
    snames: [Vec<u16>; 6],
    sname_ptrs: [*const u16; 6],
    lc_strings: [Vec<u8>; 10],
    lc_wstrings: [Vec<u16>; 8],
}

// SAFETY: a `Loc` is immutable once interned; its pointers point into
// itself.
unsafe impl Sync for Loc {}
// SAFETY: as above.
unsafe impl Send for Loc {}

impl Loc {
    /// `setlocale(category, NULL)`'s answer.
    pub fn name(&self, category: i32) -> *const u8 {
        self.names[category as usize].as_ptr()
    }
    pub fn wname(&self, category: i32) -> *const u16 {
        self.wnames[category as usize].as_ptr()
    }

    /// The class bits of `c` (`EOF` or an `unsigned char`), else 0.
    pub fn class(&self, c: i32) -> u16 {
        if (-1..=255).contains(&c) { self.ctype.table[(c + 1) as usize] } else { 0 }
    }

    pub fn to_upper(&self, c: i32) -> i32 {
        if (0..=255).contains(&c) { i32::from(self.ctype.upper[c as usize]) } else { c }
    }

    pub fn to_lower(&self, c: i32) -> i32 {
        if (0..=255).contains(&c) { i32::from(self.ctype.lower[c as usize]) } else { c }
    }

    pub fn to_upper_wide(&self, c: u16) -> u16 {
        if self.ctype_c { poly_crt::ctype::to_upper_wide(c) } else { poly_nls::unicode::to_upper(c) }
    }

    pub fn to_lower_wide(&self, c: u16) -> u16 {
        if self.ctype_c { poly_crt::ctype::to_lower_wide(c) } else { poly_nls::unicode::to_lower(c) }
    }

    pub fn is_lead(&self, b: u8) -> bool {
        self.ctype.table[usize::from(b) + 1] & poly_crt::ctype::LEADBYTE != 0
    }
}

fn cstr(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.bytes().collect();
    v.push(0);
    v
}

fn wstr(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

fn with_nul<T: Copy + Default>(v: &[T]) -> Vec<T> {
    let mut o = v.to_vec();
    o.push(T::default());
    o
}

fn build(settings: &Settings) -> &'static Loc {
    let ct = settings[(LC_CTYPE - 1) as usize];
    let (ctype, ctype_cp) = if ct.is_c() {
        (Ctype::c(), 0)
    } else {
        (Ctype::build(ct.code_page, leads_of(ct.code_page), &Kernel(ct.code_page)), ct.code_page)
    };
    let co = settings[(LC_COLLATE - 1) as usize];
    let text = lc::lconv(settings);
    let num_cp = settings[(LC_NUMERIC - 1) as usize].code_page;
    let mon_cp = settings[(LC_MONETARY - 1) as usize].code_page;
    let n = |u: &[u16]| with_nul(&from_wide_lossy(num_cp, u));
    let m = |u: &[u16]| with_nul(&from_wide_lossy(mon_cp, u));
    let lc_strings = [
        n(&text.decimal_point),
        n(&text.thousands_sep),
        with_nul(&text.grouping),
        m(&text.int_curr_symbol),
        m(&text.currency_symbol),
        m(&text.mon_decimal_point),
        m(&text.mon_thousands_sep),
        with_nul(&text.mon_grouping),
        m(&text.positive_sign),
        m(&text.negative_sign),
    ];
    let lc_wstrings = [
        with_nul(&text.decimal_point),
        with_nul(&text.thousands_sep),
        with_nul(&text.int_curr_symbol),
        with_nul(&text.currency_symbol),
        with_nul(&text.mon_decimal_point),
        with_nul(&text.mon_thousands_sep),
        with_nul(&text.positive_sign),
        with_nul(&text.negative_sign),
    ];
    let names: [String; 6] = core::array::from_fn(|i| lc::name_of(settings, i as i32));
    let snames: [Vec<u16>; 6] = core::array::from_fn(|i| {
        if i == 0 {
            return Vec::new();
        }
        settings[i - 1].locale.map_or_else(Vec::new, |l| wstr(l.name))
    });
    let point = lc_strings[0].first().copied().filter(|&b| b != 0).unwrap_or(b'.');
    let wpoint = lc_wstrings[0].first().copied().filter(|&u| u != 0).unwrap_or(u16::from(b'.'));
    let mut loc = Box::new(Loc {
        public: Public { pctype: core::ptr::null(), mb_cur_max: ctype.mb_cur_max, lc_codepage: ctype_cp },
        legacy: LegacyInfo {
            refcount: 1 << 30,
            lc_codepage: ctype_cp,
            lc_collate_cp: co.code_page,
            lc_handle: [0; 6],
            lc_id: [[0; 3]; 6],
            lc_category: core::array::from_fn(|_| LegacyCategory {
                locale: core::ptr::null(),
                wlocale: core::ptr::null(),
                refcount: core::ptr::null_mut(),
                wrefcount: core::ptr::null_mut(),
            }),
            lc_clike: i32::from(ct.is_c()),
            mb_cur_max: ctype.mb_cur_max,
            lconv_intl_refcount: core::ptr::null_mut(),
            lconv_num_refcount: core::ptr::null_mut(),
            lconv_mon_refcount: core::ptr::null_mut(),
            lconv: core::ptr::null(),
            ctype1_refcount: core::ptr::null_mut(),
            ctype1: core::ptr::null(),
            pctype: core::ptr::null(),
            pclmap: core::ptr::null(),
            pcumap: core::ptr::null(),
            lc_time_curr: core::ptr::null(),
        },
        lconv: Lconv { strings: [core::ptr::null(); 10], chars: text.chars, wide: [core::ptr::null(); 8] },
        settings: *settings,
        ctype,
        ctype_c: ct.is_c(),
        ctype_cp,
        collate_cp: co.code_page,
        collate_c: co.is_c(),
        point,
        wpoint,
        names: core::array::from_fn(|i| cstr(&names[i])),
        wnames: core::array::from_fn(|i| wstr(&names[i])),
        snames,
        sname_ptrs: [core::ptr::null(); 6],
        lc_strings,
        lc_wstrings,
    });
    // Point the members at the boxed data (it never moves again).
    let l = &mut *loc;
    l.public.pctype = l.ctype.table[1..].as_ptr();
    for i in 0..10 {
        l.lconv.strings[i] = l.lc_strings[i].as_ptr();
    }
    for i in 0..8 {
        l.lconv.wide[i] = l.lc_wstrings[i].as_ptr();
    }
    for i in 0..6 {
        l.sname_ptrs[i] = if l.snames[i].is_empty() { core::ptr::null() } else { l.snames[i].as_ptr() };
        let s = if i == 0 { None } else { Some(settings[i - 1]) };
        l.legacy.lc_handle[i] = s.map_or(0, |s| s.lcid());
        l.legacy.lc_id[i] = s.map_or([0; 3], |s| [s.lcid() as u16, s.lcid() as u16, s.code_page as u16]);
        l.legacy.lc_category[i].locale = l.names[i].as_ptr();
        l.legacy.lc_category[i].wlocale = l.wnames[i].as_ptr();
    }
    l.legacy.lconv = &raw const l.lconv;
    l.legacy.pctype = l.public.pctype;
    l.legacy.ctype1 = l.ctype.table.as_ptr();
    l.legacy.pclmap = l.ctype.lower.as_ptr();
    l.legacy.pcumap = l.ctype.upper.as_ptr();
    Box::leak(loc)
}

/// `crtdefs.h`'s `threadmbcinfostruct`.
#[repr(C)]
pub struct LegacyMbc {
    refcount: i32,
    mbcodepage: i32,
    ismbcodepage: i32,
    mbulinfo: [u16; 6],
    pub mbctype: [u8; 257],
    pub mbcasemap: [u8; 256],
    mblocalename: *const u16,
}

/// The multibyte code page's tables.
#[repr(C)]
pub struct MbcData {
    pub legacy: LegacyMbc,
    pub mbc: Mbc,
}

// SAFETY: immutable once interned.
unsafe impl Sync for MbcData {}
// SAFETY: as above.
unsafe impl Send for MbcData {}

fn build_mbc(cp: u32) -> &'static MbcData {
    let mbc = if cp == 0 {
        Mbc::sbcs()
    } else {
        let (leads, trails) = poly_nls::codepage::dbcs_ranges(cp).unwrap_or((&[], &[]));
        Mbc::build(cp, leads, trails, &Kernel(cp))
    };
    Box::leak(Box::new(MbcData {
        legacy: LegacyMbc {
            refcount: 1 << 30,
            mbcodepage: cp as i32,
            ismbcodepage: i32::from(mbc.is_mb),
            mbulinfo: [0; 6],
            mbctype: mbc.ctype,
            mbcasemap: mbc.casemap,
            mblocalename: core::ptr::null(),
        },
        mbc,
    }))
}

static INTERN_LOCK: Lock = Lock::new();
static LOCS: Global<Vec<&'static Loc>> = Global::new(Vec::new());
static MBCS: Global<Vec<&'static MbcData>> = Global::new(Vec::new());

fn intern(settings: &Settings) -> &'static Loc {
    let _g = INTERN_LOCK.lock();
    // SAFETY: under INTERN_LOCK.
    let all = unsafe { &mut *LOCS.get() };
    if let Some(l) = all.iter().find(|l| l.settings == *settings) {
        return l;
    }
    let l = build(settings);
    all.push(l);
    l
}

fn intern_mbc(cp: u32) -> &'static MbcData {
    let _g = INTERN_LOCK.lock();
    // SAFETY: under INTERN_LOCK.
    let all = unsafe { &mut *MBCS.get() };
    if let Some(m) = all.iter().find(|m| m.mbc.code_page == cp) {
        return m;
    }
    let m = build_mbc(cp);
    all.push(m);
    m
}

// ----- the current locale ----------------------------------------------------------------

static GLOBAL_LOC: AtomicPtr<Loc> = AtomicPtr::new(core::ptr::null_mut());
static GLOBAL_MBC: AtomicPtr<MbcData> = AtomicPtr::new(core::ptr::null_mut());
/// Serialises changes of the global locale (`_lock_locales` too).
static CHANGE: CritSec = CritSec::new();
/// `msvcrt`'s layouts (set by `msvcrt.dll` at attach).
static LEGACY: AtomicBool = AtomicBool::new(false);

/// A thread's own locale (`_configthreadlocale(_ENABLE_PER_THREAD_LOCALE)`).
struct ThreadLocale {
    on: bool,
    loc: *const Loc,
    mbc: *const MbcData,
}

const NO_INDEX: u32 = u32::MAX;
static TLS: AtomicU32 = AtomicU32::new(NO_INDEX);
/// Threads with their own locale on: while 0, no thread looks at TLS.
static PER_THREAD: AtomicUsize = AtomicUsize::new(0);

fn c_settings() -> Settings {
    [lc::Setting::C; 5]
}

fn global_loc() -> &'static Loc {
    let p = GLOBAL_LOC.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: interned, never freed.
        return unsafe { &*p };
    }
    let c = intern(&c_settings());
    let _ = GLOBAL_LOC.compare_exchange(
        core::ptr::null_mut(),
        c as *const Loc as *mut Loc,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    // SAFETY: interned, never freed.
    unsafe { &*GLOBAL_LOC.load(Ordering::Acquire) }
}

fn global_mbc() -> &'static MbcData {
    let p = GLOBAL_MBC.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: interned, never freed.
        return unsafe { &*p };
    }
    let m = intern_mbc(0);
    let _ = GLOBAL_MBC.compare_exchange(
        core::ptr::null_mut(),
        m as *const MbcData as *mut MbcData,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    // SAFETY: interned, never freed.
    unsafe { &*GLOBAL_MBC.load(Ordering::Acquire) }
}

/// The calling thread's own locale record, made on first use.
fn thread_locale(make: bool) -> Option<&'static mut ThreadLocale> {
    let mut index = TLS.load(Ordering::Acquire);
    if index == NO_INDEX {
        if !make {
            return None;
        }
        // SAFETY: no preconditions.
        let i = unsafe { sys::TlsAlloc() };
        if i == NO_INDEX {
            return None;
        }
        if let Err(other) = TLS.compare_exchange(NO_INDEX, i, Ordering::AcqRel, Ordering::Acquire) {
            index = other;
        } else {
            index = i;
        }
    }
    // SAFETY: no preconditions.
    let saved = unsafe { sys::GetLastError() };
    // SAFETY: a valid index.
    let mut p = unsafe { sys::TlsGetValue(index) } as *mut ThreadLocale;
    if p.is_null() && make {
        p = Box::into_raw(Box::new(ThreadLocale { on: false, loc: global_loc(), mbc: global_mbc() }));
        // SAFETY: a valid index.
        unsafe { sys::TlsSetValue(index, p as *mut u8) };
    }
    // SAFETY: as above.
    unsafe { sys::SetLastError(saved) };
    // SAFETY: this thread's own allocation, freed at its detach.
    (!p.is_null()).then(|| unsafe { &mut *p })
}

/// `DLL_THREAD_DETACH`: free the thread's locale record.
pub fn thread_detach() {
    if let Some(t) = thread_locale(false) {
        if t.on {
            PER_THREAD.fetch_sub(1, Ordering::AcqRel);
        }
        let index = TLS.load(Ordering::Acquire);
        // SAFETY: made by `thread_locale` with Box; nothing uses it after
        // detach.
        drop(unsafe { Box::from_raw(t as *mut ThreadLocale) });
        // SAFETY: a valid index.
        unsafe { sys::TlsSetValue(index, core::ptr::null_mut()) };
    }
}

/// The calling thread's locale and multibyte code page.
pub fn current() -> (&'static Loc, &'static MbcData) {
    if PER_THREAD.load(Ordering::Acquire) != 0
        && let Some(t) = thread_locale(false)
        && t.on
    {
        // SAFETY: interned, never freed.
        return unsafe { (&*t.loc, &*t.mbc) };
    }
    (global_loc(), global_mbc())
}

/// The calling thread's locale.
pub fn loc() -> &'static Loc {
    current().0
}

/// A program's `_locale_t`: ours, `locinfo` and `mbcinfo` first.
#[repr(C)]
pub struct LocaleT {
    locinfo: *const u8,
    mbcinfo: *const u8,
    loc: *const Loc,
    mbc: *const MbcData,
}

/// The locale a `_locale_t` argument names (NULL: the current one). Not
/// `unsafe`: every `_l` function takes the program's `_locale_t` as C
/// does, and its contract is that it came from this runtime.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn resolve(l: *const LocaleT) -> (&'static Loc, &'static MbcData) {
    if l.is_null() {
        return current();
    }
    // SAFETY: a `_locale_t` this runtime made (`_create_locale`,
    // `_get_current_locale`); its locale objects are interned.
    unsafe { (&*(*l).loc, &*(*l).mbc) }
}

/// The locale of a `_locale_t` argument.
pub fn loc_of(l: *const LocaleT) -> &'static Loc {
    resolve(l).0
}

fn new_handle(loc: &'static Loc, mbc: &'static MbcData) -> *mut LocaleT {
    let legacy = LEGACY.load(Ordering::Relaxed);
    let locinfo = if legacy { &raw const loc.legacy as *const u8 } else { &raw const loc.public as *const u8 };
    let mbcinfo = &raw const mbc.legacy as *const u8;
    Box::into_raw(Box::new(LocaleT { locinfo, mbcinfo, loc, mbc }))
}

// ----- the data the headers read (msvcrt's exported variables) -----------------------------

/// Where a DLL keeps the variables a global change updates.
#[derive(Clone, Copy)]
pub struct Cells {
    pub pctype: *mut *const u16,
    pub mb_cur_max: *mut i32,
    pub lc_codepage: *mut u32,
    pub lc_collate_cp: *mut u32,
    pub lc_handle: *mut [u32; 6],
    pub mbctype: *mut [u8; 257],
    pub mbcasemap: *mut [u8; 256],
}

static CELLS: Global<Option<Cells>> = Global::new(None);

/// At `DLL_PROCESS_ATTACH`: the DLL's variables (null members for those it
/// lacks) and whether it has `msvcrt`'s layouts.
pub fn bind(cells: Cells, legacy: bool) {
    // SAFETY: process attach, one thread.
    unsafe { *CELLS.get() = Some(cells) };
    LEGACY.store(legacy, Ordering::Relaxed);
    publish();
}

/// Copy the global locale into the bound variables.
fn publish() {
    // SAFETY: written only at attach.
    let Some(c) = (unsafe { *CELLS.get() }) else { return };
    let (l, m) = (global_loc(), global_mbc());
    // SAFETY: the DLL's own variables (or null), written under CHANGE.
    unsafe {
        if !c.pctype.is_null() {
            *c.pctype = l.public.pctype;
        }
        if !c.mb_cur_max.is_null() {
            *c.mb_cur_max = l.public.mb_cur_max;
        }
        if !c.lc_codepage.is_null() {
            *c.lc_codepage = l.ctype_cp;
        }
        if !c.lc_collate_cp.is_null() {
            *c.lc_collate_cp = l.collate_cp;
        }
        if !c.lc_handle.is_null() {
            *c.lc_handle = l.legacy.lc_handle;
        }
        if !c.mbctype.is_null() {
            *c.mbctype = m.legacy.mbctype;
        }
        if !c.mbcasemap.is_null() {
            *c.mbcasemap = m.legacy.mbcasemap;
        }
    }
}

// ----- setlocale -------------------------------------------------------------------------

/// Make the change `f` on the calling thread's locale (its own, or the
/// global one): `(locale, multibyte code page)` → the new pair.
fn change(
    f: impl FnOnce(&'static Loc, &'static MbcData) -> Option<(&'static Loc, &'static MbcData)>,
) -> Option<&'static Loc> {
    CHANGE.enter();
    let (l, m) = current();
    let r = f(l, m);
    if let Some((nl, nm)) = r {
        match thread_locale(false).filter(|t| t.on && PER_THREAD.load(Ordering::Acquire) != 0) {
            Some(t) => {
                t.loc = nl;
                t.mbc = nm;
            }
            None => {
                GLOBAL_LOC.store(nl as *const Loc as *mut Loc, Ordering::Release);
                GLOBAL_MBC.store(nm as *const MbcData as *mut MbcData, Ordering::Release);
                publish();
            }
        }
    }
    CHANGE.leave();
    r.map(|(l, _)| l)
}

/// `setlocale`'s work on a name already read: the new locale, or `None`.
pub fn set(category: i32, name: &str) -> Option<&'static Loc> {
    let utf8 = !LEGACY.load(Ordering::Relaxed);
    change(|l, m| {
        let settings = lc::apply(&l.settings, category, name, &valid_code_page, utf8)?;
        let nl = intern(&settings);
        // The multibyte code page follows LC_CTYPE (Poly's reading of
        // "_setmbcp(_MB_CP_LOCALE)" as what setlocale does).
        let nm = if (category == LC_ALL || category == LC_CTYPE) && nl.ctype_cp != m.mbc.code_page {
            intern_mbc(nl.ctype_cp)
        } else {
            m
        };
        Some((nl, nm))
    })
}

fn category_ok(category: i32) -> bool {
    if (0..=LC_MAX).contains(&category) {
        return true;
    }
    global::invalid(EINVAL);
    false
}

fn name_from_bytes(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn name_from_units(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

/// `setlocale`.
///
/// # Safety
/// `locale` NULL or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setlocale(category: i32, locale: *const u8) -> *const u8 {
    if !category_ok(category) {
        return core::ptr::null();
    }
    if locale.is_null() {
        return loc().name(category);
    }
    // SAFETY: the caller's string.
    let name = name_from_bytes(unsafe { poly_crt::unit::slice(locale) });
    set(category, &name).map_or(core::ptr::null(), |l| l.name(category))
}

/// `_wsetlocale`.
///
/// # Safety
/// `locale` NULL or a wide C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wsetlocale(category: i32, locale: *const u16) -> *const u16 {
    if !category_ok(category) {
        return core::ptr::null();
    }
    if locale.is_null() {
        return loc().wname(category);
    }
    // SAFETY: the caller's string.
    let name = name_from_units(unsafe { poly_crt::unit::slice(locale) });
    set(category, &name).map_or(core::ptr::null(), |l| l.wname(category))
}

/// `localeconv`: the current locale's `lconv` (kept for the process's
/// life, so the pointer stays valid).
#[unsafe(no_mangle)]
pub extern "C" fn localeconv() -> *const Lconv {
    &loc().lconv
}

/// `_create_locale` / `_wcreate_locale`'s work: the "C" locale with
/// `category` set to `name`.
pub fn create(category: i32, name: &str) -> *mut LocaleT {
    if !(0..=LC_MAX).contains(&category) {
        return core::ptr::null_mut();
    }
    let utf8 = !LEGACY.load(Ordering::Relaxed);
    let Some(settings) = lc::apply(&c_settings(), category, name, &valid_code_page, utf8) else {
        return core::ptr::null_mut();
    };
    let l = intern(&settings);
    new_handle(l, intern_mbc(l.ctype_cp))
}

/// # Safety
/// `locale` a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _create_locale(category: i32, locale: *const u8) -> *mut LocaleT {
    if locale.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's string.
    create(category, &name_from_bytes(unsafe { poly_crt::unit::slice(locale) }))
}

/// `_wcreate_locale` (`ucrtbase` only).
///
/// # Safety
/// `locale` a wide C string.
pub unsafe extern "C" fn _wcreate_locale(category: i32, locale: *const u16) -> *mut LocaleT {
    if locale.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's string.
    create(category, &name_from_units(unsafe { poly_crt::unit::slice(locale) }))
}

/// # Safety
/// `l` NULL or from `_create_locale` / `_get_current_locale`, not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _free_locale(l: *mut LocaleT) {
    if !l.is_null() {
        // SAFETY: made by `new_handle` with Box (the locale it names is
        // interned and stays).
        drop(unsafe { Box::from_raw(l) });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_current_locale() -> *mut LocaleT {
    let (l, m) = current();
    new_handle(l, m)
}

/// `___lc_codepage_func`: `LC_CTYPE`'s code page (0 in "C").
#[unsafe(no_mangle)]
pub extern "C" fn ___lc_codepage_func() -> u32 {
    loc().ctype_cp
}

/// `___lc_collate_cp_func`.
#[unsafe(no_mangle)]
pub extern "C" fn ___lc_collate_cp_func() -> u32 {
    loc().collate_cp
}

/// `___lc_locale_name_func` (`ucrtbase`): the categories' locale names
/// (`wchar_t *[6]`, NULL for "C").
pub extern "C" fn ___lc_locale_name_func() -> *const *const u16 {
    loc().sname_ptrs.as_ptr()
}

/// `___lc_handle_func` (`msvcrt`): the categories' LCIDs.
pub extern "C" fn ___lc_handle_func() -> *const u32 {
    loc().legacy.lc_handle.as_ptr()
}

/// `_lock_locales` / `_unlock_locales` (`ucrtbase`).
pub extern "C" fn _lock_locales() {
    CHANGE.enter();
}
pub extern "C" fn _unlock_locales() {
    CHANGE.leave();
}

/// `_configthreadlocale`: 0 asks, `_ENABLE_PER_THREAD_LOCALE` (1) and
/// `_DISABLE_PER_THREAD_LOCALE` (2) set the calling thread's, −1 is 2;
/// the previous setting. A thread that turns its own locale on starts with
/// a copy of the global one.
pub fn configthreadlocale(mode: i32) -> i32 {
    let on = thread_locale(false).is_some_and(|t| t.on);
    let previous = if on { 1 } else { 2 };
    match mode {
        0 => {}
        1 if !on => {
            let (l, m) = (global_loc(), global_mbc());
            if let Some(t) = thread_locale(true) {
                t.loc = l;
                t.mbc = m;
                t.on = true;
                PER_THREAD.fetch_add(1, Ordering::AcqRel);
            }
        }
        2 | -1 if on => {
            if let Some(t) = thread_locale(false) {
                t.on = false;
                PER_THREAD.fetch_sub(1, Ordering::AcqRel);
            }
        }
        1 | 2 | -1 => {}
        _ => {
            global::invalid(EINVAL);
            return -1;
        }
    }
    previous
}

// ----- the multibyte code page -----------------------------------------------------------

/// `_getmbcp`.
#[unsafe(no_mangle)]
pub extern "C" fn _getmbcp() -> i32 {
    current().1.mbc.code_page as i32
}

/// `_setmbcp`: `_MB_CP_SBCS`, `_MB_CP_OEM`, `_MB_CP_ANSI`, `_MB_CP_LOCALE`
/// or a code page Poly converts (not UTF-8: the `_mbs` functions handle
/// one- and two-byte characters); −1 and `EINVAL` otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn _setmbcp(cp: i32) -> i32 {
    let user = poly_nls::locale::user_default();
    let resolved = match cp {
        mbcs::MB_CP_SBCS => 0,
        mbcs::MB_CP_OEM => u32::from(user.oem),
        mbcs::MB_CP_ANSI => u32::from(user.ansi),
        mbcs::MB_CP_LOCALE => loc().ctype_cp,
        c if c > 0 && c as u32 != lc::UTF8 && valid_code_page(c as u32) => c as u32,
        _ => {
            global::set_errno(EINVAL);
            return -1;
        }
    };
    let m = intern_mbc(resolved);
    change(|l, _| Some((l, m)));
    0
}

/// The multibyte code page's tables (`_mbctype`, `_mbcasemap`).
pub fn mbctype_ptr() -> *const u8 {
    current().1.legacy.mbctype.as_ptr()
}
pub fn mbcasemap_ptr() -> *const u8 {
    current().1.legacy.mbcasemap.as_ptr()
}

// ----- formatting ------------------------------------------------------------------------

/// A `printf`/`scanf` option word with the current locale's decimal point
/// (`poly_crt::locale::point_option`; the program's own bits there
/// cleared).
pub fn with_point(options: u64) -> u64 {
    (options & !lc::POINT_MASK) | lc::point_option(loc().point)
}
