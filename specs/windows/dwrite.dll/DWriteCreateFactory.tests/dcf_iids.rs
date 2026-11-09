//! DWriteCreateFactory: every factory interface can be asked for, in both
//! factory types, and the objects answer QueryInterface for the whole chain.
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

const IID_IUnknown: GUID = GUID { d1: 0, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
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

const DWRITE_FACTORY_TYPE_SHARED: u32 = 0;
const DWRITE_FACTORY_TYPE_ISOLATED: u32 = 1;
#[repr(C)]
struct IDWriteFontFaceVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetType: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFiles: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    GetIndex: unsafe extern "system" fn(*mut c_void) -> u32,
    GetSimulations: unsafe extern "system" fn(*mut c_void) -> u32,
    IsSymbolFont: unsafe extern "system" fn(*mut c_void) -> i32,
    GetMetrics: unsafe extern "system" fn(*mut c_void, *mut c_void),
    GetGlyphCount: unsafe extern "system" fn(*mut c_void) -> u16,
    GetDesignGlyphMetrics: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, i32) -> i32,
    GetGlyphIndices: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    TryGetFontTable: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    ReleaseFontTable: unsafe extern "system" fn(*mut c_void, *mut c_void),
    GetGlyphRunOutline:
        unsafe extern "system" fn(*mut c_void, f32, *mut c_void, *mut c_void, *mut c_void, u32, i32, i32, *mut c_void) -> i32,
    GetRecommendedRenderingMode: unsafe extern "system" fn(*mut c_void, f32, f32, u32, *mut c_void, *mut c_void) -> i32,
    GetGdiCompatibleMetrics: unsafe extern "system" fn(*mut c_void, f32, f32, *mut c_void, *mut c_void) -> i32,
    GetGdiCompatibleGlyphMetrics:
        unsafe extern "system" fn(*mut c_void, f32, f32, *mut c_void, i32, *mut c_void, u32, *mut c_void, i32) -> i32,
}

#[repr(C)]
struct IDWriteFontFace {
    vtbl: *const IDWriteFontFaceVtbl,
}

const IID_IDWriteFontFace: GUID = GUID { d1: 0x5F49804D, d2: 0x7024, d3: 0x4D43, d4: [0xBF, 0xA9, 0xD2, 0x59, 0x84, 0xF5, 0x38, 0x49] };
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

const IID_IDWriteFont: GUID = GUID { d1: 0xACD16696, d2: 0x8C14, d3: 0x4F5D, d4: [0x87, 0x7E, 0xFE, 0x3F, 0xC1, 0xD3, 0x27, 0x37] };
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
struct IDWriteFactory1Vtbl {
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
}

#[repr(C)]
struct IDWriteFactory1 {
    vtbl: *const IDWriteFactory1Vtbl,
}

const IID_IDWriteFactory1: GUID = GUID { d1: 0x30572F99, d2: 0xDAC6, d3: 0x41DB, d4: [0xA1, 0x6E, 0x04, 0x86, 0x30, 0x7E, 0x60, 0x6A] };
#[repr(C)]
struct IDWriteFactory2Vtbl {
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
}

#[repr(C)]
struct IDWriteFactory2 {
    vtbl: *const IDWriteFactory2Vtbl,
}

const IID_IDWriteFactory2: GUID = GUID { d1: 0x0439FC60, d2: 0xCA44, d3: 0x4994, d4: [0x8D, 0xEE, 0x3A, 0x9A, 0xF7, 0xB7, 0x32, 0xEC] };
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
struct IDWriteFactory4Vtbl {
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
    TranslateColorGlyphRun_4:
        unsafe extern "system" fn(*mut c_void, u64, *mut c_void, *mut c_void, u32, u32, *mut c_void, u32, *mut c_void) -> i32,
    ComputeGlyphOrigins: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u64, *mut c_void, *mut c_void) -> i32,
    ComputeGlyphOrigins_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFactory4 {
    vtbl: *const IDWriteFactory4Vtbl,
}

const IID_IDWriteFactory4: GUID = GUID { d1: 0x4B0B5BD3, d2: 0x0797, d3: 0x4549, d4: [0x8A, 0xC5, 0xFE, 0x91, 0x5C, 0xC5, 0x38, 0x56] };
#[repr(C)]
struct IDWriteFactory5Vtbl {
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
    TranslateColorGlyphRun_4:
        unsafe extern "system" fn(*mut c_void, u64, *mut c_void, *mut c_void, u32, u32, *mut c_void, u32, *mut c_void) -> i32,
    ComputeGlyphOrigins: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u64, *mut c_void, *mut c_void) -> i32,
    ComputeGlyphOrigins_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void) -> i32,
    CreateFontSetBuilder_5: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateInMemoryFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateHttpFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    AnalyzeContainerType: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> u32,
    UnpackFontFile: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFactory5 {
    vtbl: *const IDWriteFactory5Vtbl,
}

const IID_IDWriteFactory5: GUID = GUID { d1: 0x958DB99A, d2: 0xBE2A, d3: 0x4F09, d4: [0xAF, 0x7D, 0x65, 0x18, 0x98, 0x03, 0xD1, 0xD3] };
#[repr(C)]
struct IDWriteFactory7Vtbl {
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
    TranslateColorGlyphRun_4:
        unsafe extern "system" fn(*mut c_void, u64, *mut c_void, *mut c_void, u32, u32, *mut c_void, u32, *mut c_void) -> i32,
    ComputeGlyphOrigins: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u64, *mut c_void, *mut c_void) -> i32,
    ComputeGlyphOrigins_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void) -> i32,
    CreateFontSetBuilder_5: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateInMemoryFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateHttpFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    AnalyzeContainerType: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> u32,
    UnpackFontFile: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32, *mut c_void) -> i32,
    CreateFontFaceReference_6: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, *mut c_void, u32, *mut c_void) -> i32,
    CreateFontResource: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    GetSystemFontSet_6: unsafe extern "system" fn(*mut c_void, i32, *mut c_void) -> i32,
    GetSystemFontCollection_6: unsafe extern "system" fn(*mut c_void, i32, u32, *mut c_void) -> i32,
    CreateFontCollectionFromFontSet_6: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    CreateFontSetBuilder_6: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateTextFormat_6:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, u32, f32, *mut c_void, *mut c_void) -> i32,
    GetSystemFontSet_7: unsafe extern "system" fn(*mut c_void, i32, *mut c_void) -> i32,
    GetSystemFontCollection_7: unsafe extern "system" fn(*mut c_void, i32, u32, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFactory7 {
    vtbl: *const IDWriteFactory7Vtbl,
}

const IID_IDWriteFactory7: GUID = GUID { d1: 0x35D0E0B3, d2: 0x9076, d3: 0x4D2E, d4: [0xA0, 0x16, 0xA9, 0x1B, 0x56, 0x8A, 0x06, 0xB4] };

#[link(name = "dwrite", kind = "raw-dylib")]
unsafe extern "system" {
    fn DWriteCreateFactory(kind: u32, iid: *const GUID, factory: *mut *mut c_void) -> HRESULT;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let iids: [(&str, GUID); 8] = [
        ("IDWriteFactory", IID_IDWriteFactory),
        ("IDWriteFactory1", IID_IDWriteFactory1),
        ("IDWriteFactory2", IID_IDWriteFactory2),
        ("IDWriteFactory3", IID_IDWriteFactory3),
        ("IDWriteFactory4", IID_IDWriteFactory4),
        ("IDWriteFactory5", IID_IDWriteFactory5),
        ("IDWriteFactory7", IID_IDWriteFactory7),
        ("IUnknown", IID_IUnknown),
    ];
    for kind in [DWRITE_FACTORY_TYPE_SHARED, DWRITE_FACTORY_TYPE_ISOLATED] {
        for (name, iid) in iids.iter() {
            let (hr, f) = create_factory(kind, iid);
            say!(t, "kind {} asks {}: {} null={}", kind, name, Hr(hr), f.is_null());
            if f.is_null() {
                continue;
            }
            for (name2, iid2) in iids.iter() {
                let (hr2, g) = query(f, iid2);
                say!(t, "  QI {}: {} null={} same_pointer={}", name2, Hr(hr2), g.is_null(), g == f);
                release(g);
            }
            let (hr3, g) = query(f, &IID_IDWriteFont);
            say!(t, "  QI IDWriteFont: {} null={}", Hr(hr3), g.is_null());
            let (hr4, g) = query(f, &IID_IDWriteFontFace);
            say!(t, "  QI IDWriteFontFace: {} null={}", Hr(hr4), g.is_null());
            release(f);
        }
    }
    let (hr, f) = create_factory(DWRITE_FACTORY_TYPE_SHARED, &IID_IDWriteFactory7);
    t.check("factory7 created", hr == 0 && !f.is_null(), i64::from(hr as u32));
    release(f);
}
