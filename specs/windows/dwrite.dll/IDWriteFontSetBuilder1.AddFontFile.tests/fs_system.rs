//! IDWriteFactory3::GetSystemFontSet, IDWriteFontSet::GetFontCount,
//! GetFontFaceReference and IDWriteFontCollection1::GetFontSet for the system
//! fonts.
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

/// HRESULT as `0x........`.
struct Hr(HRESULT);

impl core::fmt::Display for Hr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{:08x}", self.0 as u32)
    }
}

/// QueryInterface on any COM object.
fn query(obj: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: `obj` is a live COM object; `out` is a valid out pointer.
    let hr = unsafe {
        let u = obj as *mut IUnknown;
        ((*(*u).vtbl).QueryInterface)(obj, iid as *const GUID as *mut c_void, &mut out as *mut *mut c_void as *mut c_void)
    };
    (hr, out)
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
#[repr(C)]
struct IDWriteFactory3Vtbl {
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
    GetEudcFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    CreateCustomRenderingParams_1: unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    GetSystemFontFallback: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateFontFallbackBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    TranslateColorGlyphRun:
        unsafe extern "system" fn(*mut c_void, f32, f32, *mut c_void, *mut c_void, u32, *mut c_void, u32, *mut c_void) -> i32,
    CreateCustomRenderingParams_2: unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    CreateGlyphRunAnalysis_2:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, u32, u32, u32, f32, f32, *mut c_void) -> i32,
    CreateGlyphRunAnalysis_3:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, u32, u32, u32, f32, f32, *mut c_void) -> i32,
    CreateCustomRenderingParams_3: unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    CreateFontFaceReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    CreateFontFaceReference_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    GetSystemFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateFontSetBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateFontCollectionFromFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    GetSystemFontCollection_3: unsafe extern "system" fn(*mut c_void, i32, *mut c_void, i32) -> i32,
    GetFontDownloadQueue: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFactory3 {
    vtbl: *const IDWriteFactory3Vtbl,
}

const IID_IDWriteFactory3: GUID = GUID { d1: 0x9A1B41C3, d2: 0xD3BB, d3: 0x466A, d4: [0x87, 0xFC, 0xFE, 0x67, 0x55, 0x6A, 0x3B, 0x65] };
#[repr(C)]
struct IDWriteFontSetVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontCount: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFaceReference: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    FindFontFaceReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    FindFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    GetPropertyValues: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, *mut c_void) -> i32,
    GetPropertyValues_ov2: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    GetPropertyValues_ov3: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    GetPropertyOccurrenceCount: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    GetMatchingFonts: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    GetMatchingFonts_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFontSet {
    vtbl: *const IDWriteFontSetVtbl,
}

#[repr(C)]
struct IDWriteFontCollection1Vtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFamilyCount: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFamily: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    FindFamilyName: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    GetFontFromFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    GetFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetFontFamily_1: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFontCollection1 {
    vtbl: *const IDWriteFontCollection1Vtbl,
}

const IID_IDWriteFontCollection1: GUID =
    GUID { d1: 0x53585141, d2: 0xD9F8, d3: 0x4095, d4: [0x83, 0x21, 0xD7, 0x3C, 0xF6, 0xBD, 0x11, 0x6C] };
#[repr(C)]
struct IDWriteFontFaceReferenceVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    CreateFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateFontFaceWithSimulations: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    Equals: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetFontFaceIndex: unsafe extern "system" fn(*mut c_void) -> u32,
    GetSimulations: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFile: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetLocalFileSize: unsafe extern "system" fn(*mut c_void) -> u64,
    GetFileSize: unsafe extern "system" fn(*mut c_void) -> u64,
    GetFileTime: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetLocality: unsafe extern "system" fn(*mut c_void) -> u32,
    EnqueueFontDownloadRequest: unsafe extern "system" fn(*mut c_void) -> i32,
    EnqueueCharacterDownloadRequest: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32,
    EnqueueGlyphDownloadRequest: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32,
    EnqueueFileFragmentDownloadRequest: unsafe extern "system" fn(*mut c_void, u64, u64) -> i32,
}

#[repr(C)]
struct IDWriteFontFaceReference {
    vtbl: *const IDWriteFontFaceReferenceVtbl,
}

#[link(name = "dwrite", kind = "raw-dylib")]
unsafe extern "system" {
    fn DWriteCreateFactory(kind: u32, iid: *const GUID, factory: *mut *mut c_void) -> HRESULT;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let fac = new_factory(DWRITE_FACTORY_TYPE_SHARED);
    let f3 = query(fac as *mut c_void, &IID_IDWriteFactory3).1 as *mut IDWriteFactory3;
    let mut set: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid objects.
    unsafe {
        let hr = com!(f3, GetSystemFontSet(outp!(set)));
        let fs = set as *mut IDWriteFontSet;
        let n = com!(fs, GetFontCount());
        say!(t, "GetSystemFontSet {} font count >= 50 {}", Hr(hr), n >= 50);
        let (mut ok, mut local) = (0u32, 0u32);
        for i in 0..n.min(40) {
            let mut r: *mut c_void = core::ptr::null_mut();
            if com!(fs, GetFontFaceReference(i, outp!(r))) == 0 {
                let rr = r as *mut IDWriteFontFaceReference;
                let mut face: *mut c_void = core::ptr::null_mut();
                if com!(rr, CreateFontFace(outp!(face))) == 0 {
                    ok += 1;
                    release(face);
                }
                if com!(rr, GetLocality()) == 2 {
                    local += 1;
                }
                release(r);
            }
        }
        say!(
            t,
            "the first {} font face references all make faces {} and are local (locality 2) {}",
            n.min(40),
            ok == n.min(40),
            local == n.min(40)
        );
        let mut r = 0x10usize as *mut c_void;
        let hr = com!(fs, GetFontFaceReference(n, outp!(r)));
        say!(t, "GetFontFaceReference(count): {} out_null {}", Hr(hr), r.is_null());
        let mut c: *mut c_void = core::ptr::null_mut();
        let f1 = query(fac as *mut c_void, &IID_IDWriteFactory).1 as *mut IDWriteFactory;
        let coll = system_collection(f1);
        let (hrq, c1) = query(coll as *mut c_void, &IID_IDWriteFontCollection1);
        say!(t, "system collection QI IDWriteFontCollection1 {}", Hr(hrq));
        let mut fs2: *mut c_void = core::ptr::null_mut();
        let hr = com!(c1 as *mut IDWriteFontCollection1, GetFontSet(outp!(fs2)));
        say!(t, "GetFontSet {} at least 50 fonts {}", Hr(hr), com!(fs2 as *mut IDWriteFontSet, GetFontCount()) >= 50);
        release(fs2);
        release(c1);
        release(coll as *mut c_void);
        release(f1 as *mut c_void);
        let _ = &mut c;
    }
    release(set);
    release(f3 as *mut c_void);
    release(fac as *mut c_void);
}
