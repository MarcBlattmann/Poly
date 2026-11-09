//! IDWriteFont::GetStretch on system fonts: the ordinary faces are normal
//! (5).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_snake_case, non_upper_case_globals, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers dwrite.h, dwrite_1.h, dwrite_2.h, dwrite_3.h and winerror.h
// (vtable layouts, constants, structure layouts); the font writer, the loaders
// and the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HRESULT = i32;
/// guiddef.h
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

#[repr(C)]
struct IUnknownVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IUnknown {
    vtbl: *const IUnknownVtbl,
}

/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<240>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A call through a COM interface pointer: `com!(p, Method(args))` calls the
/// method of the vtable of `*p`, passing `p` as the interface pointer.
macro_rules! com {
    ($p:expr, $m:ident($($a:expr),* $(,)?)) => {{
        let p = $p;
        ((*(*p).vtbl).$m)(p as *mut c_void $(, $a)*)
    }};
}

/// Prints UTF-16 units as text: printable ASCII as is, everything else
/// as `\u{hex}`.
struct W16<'a>(&'a [u16]);

impl core::fmt::Display for W16<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for r in core::char::decode_utf16(self.0.iter().copied()) {
            match r {
                Ok(c) if (' '..='~').contains(&c) && c != '\\' => f.write_char(c)?,
                Ok(c) => write!(f, "\\u{{{:x}}}", c as u32)?,
                Err(e) => write!(f, "\\u{{{:x}}}", e.unpaired_surrogate())?,
            }
        }
        Ok(())
    }
}

/// Release on any COM object; the new reference count.
fn release(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: `obj` is a live COM object this test holds a reference to.
    unsafe {
        let u = obj as *mut IUnknown;
        ((*(*u).vtbl).Release)(obj)
    }
}

/// DWriteCreateFactory; the HRESULT and the factory pointer.
fn create_factory(kind: u32, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut f: *mut c_void = core::ptr::null_mut();
    // SAFETY: `iid` and `f` are valid for the call.
    let hr = unsafe { DWriteCreateFactory(kind, iid, &mut f) };
    (hr, f)
}

/// A NUL-terminated UTF-16 literal (ASCII): `wz!("Arial")` is a `&[u16; 6]`.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W[..]
    }};
}

/// `&mut x` as the `*mut c_void` of an out parameter.
macro_rules! outp {
    ($e:expr) => {
        &mut $e as *mut _ as *mut c_void
    };
}

/// A factory of the given type and interface (`IDWriteFactory`).
fn new_factory(kind: u32) -> *mut IDWriteFactory {
    let (hr, f) = create_factory(kind, &IID_IDWriteFactory);
    if hr != 0 {
        exit(9);
    }
    f as *mut IDWriteFactory
}

/// The system font collection.
fn system_collection(fac: *mut IDWriteFactory) -> *mut IDWriteFontCollection {
    let mut c: *mut c_void = core::ptr::null_mut();
    // SAFETY: the factory is live.
    let hr = unsafe { com!(fac, GetSystemFontCollection(outp!(c), 0)) };
    if hr != 0 {
        exit(9);
    }
    c as *mut IDWriteFontCollection
}

/// The family with the given name (NUL-terminated UTF-16) in `c`, or null.
fn family_named(c: *mut IDWriteFontCollection, name: &[u16]) -> *mut IDWriteFontFamily {
    let mut idx = 0u32;
    let mut ex = 0i32;
    let mut fam: *mut c_void = core::ptr::null_mut();
    // SAFETY: `c` is live; the pointers are valid for the calls.
    unsafe {
        com!(c, FindFamilyName(name.as_ptr() as *mut c_void, outp!(idx), outp!(ex)));
        if ex != 0 {
            com!(c, GetFontFamily(idx, outp!(fam)));
        }
    }
    fam as *mut IDWriteFontFamily
}

/// The best-matching font of the family `name` in `c` (null if the family is missing).
fn match_font(c: *mut IDWriteFontCollection, name: &[u16], w: u32, st: u32, sy: u32) -> *mut IDWriteFont {
    let fm = family_named(c, name);
    if fm.is_null() {
        return core::ptr::null_mut();
    }
    let mut font: *mut c_void = core::ptr::null_mut();
    // SAFETY: the family is live.
    unsafe { com!(fm, GetFirstMatchingFont(w, st, sy, outp!(font))) };
    release(fm as *mut c_void);
    font as *mut IDWriteFont
}

const DWRITE_FACTORY_TYPE_SHARED: u32 = 0;
#[repr(C)]
struct IDWriteFontCollectionVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFamilyCount: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFamily: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    FindFamilyName: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    GetFontFromFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFontCollection {
    vtbl: *const IDWriteFontCollectionVtbl,
}

#[repr(C)]
struct IDWriteFontFamilyVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetFontCount: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFont: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    GetFamilyNames: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetFirstMatchingFont: unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32,
    GetMatchingFonts: unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFontFamily {
    vtbl: *const IDWriteFontFamilyVtbl,
}

#[repr(C)]
struct IDWriteFontVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFamily: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetWeight: unsafe extern "system" fn(*mut c_void) -> u32,
    GetStretch: unsafe extern "system" fn(*mut c_void) -> u32,
    GetStyle: unsafe extern "system" fn(*mut c_void) -> u32,
    IsSymbolFont: unsafe extern "system" fn(*mut c_void) -> i32,
    GetFaceNames: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetInformationalStrings: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    GetSimulations: unsafe extern "system" fn(*mut c_void) -> u32,
    GetMetrics: unsafe extern "system" fn(*mut c_void, *mut c_void),
    HasCharacter: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    CreateFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFont {
    vtbl: *const IDWriteFontVtbl,
}

#[repr(C)]
struct IDWriteFactoryVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    CreateCustomFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    CreateCustomFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    CreateCustomRenderingParams: unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateTextFormat: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, u32, u32, f32, *mut c_void, *mut c_void) -> i32,
    CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateTextLayout: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    CreateGdiCompatibleTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, f32, *mut c_void, i32, *mut c_void) -> i32,
    CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    CreateGlyphRunAnalysis: unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFactory {
    vtbl: *const IDWriteFactoryVtbl,
}

const IID_IDWriteFactory: GUID = GUID { d1: 0xB859EE5A, d2: 0xD838, d3: 0x4B5B, d4: [0xA2, 0xE8, 0x1A, 0xDC, 0x7D, 0x93, 0xDB, 0x48] };

#[link(name = "dwrite", kind = "raw-dylib")]
unsafe extern "system" {
    fn DWriteCreateFactory(kind: u32, iid: *const GUID, factory: *mut *mut c_void) -> HRESULT;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let fac = new_factory(DWRITE_FACTORY_TYPE_SHARED);
    let c = system_collection(fac);
    for name in [wz!("Arial"), wz!("Segoe UI"), wz!("Times New Roman"), wz!("Courier New"), wz!("Verdana")] {
        let ft = match_font(c, name, 400, 5, 0);
        // SAFETY: the font is live.
        let s = unsafe { com!(ft, GetStretch()) };
        say!(t, "{}: stretch {}", W16(&name[..name.len() - 1]), s);
        release(ft as *mut c_void);
    }
    release(c as *mut c_void);
    release(fac as *mut c_void);
}
