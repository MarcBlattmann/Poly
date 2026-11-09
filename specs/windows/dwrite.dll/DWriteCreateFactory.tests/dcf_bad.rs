//! DWriteCreateFactory: invalid arguments. What comes back and whether the
//! output pointer is written.
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

/// HRESULT as `0x........`.
struct Hr(HRESULT);

impl core::fmt::Display for Hr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{:08x}", self.0 as u32)
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

const IID_IDWriteFontCollection: GUID =
    GUID { d1: 0xA84CEE02, d2: 0x3EEA, d3: 0x4EEE, d4: [0xA8, 0x27, 0x87, 0xC1, 0xA0, 0x2A, 0x0F, 0xCC] };
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
const IID_IDWriteFactory: GUID = GUID { d1: 0xB859EE5A, d2: 0xD838, d3: 0x4B5B, d4: [0xA2, 0xE8, 0x1A, 0xDC, 0x7D, 0x93, 0xDB, 0x48] };

#[link(name = "dwrite", kind = "raw-dylib")]
unsafe extern "system" {
    fn DWriteCreateFactory(kind: u32, iid: *const GUID, factory: *mut *mut c_void) -> HRESULT;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let sentinel = 0x10usize as *mut c_void;
    for kind in [2u32, 3, 0x8000_0000, u32::MAX] {
        let mut f = sentinel;
        // SAFETY: valid IID and out pointer.
        let hr = unsafe { DWriteCreateFactory(kind, &IID_IDWriteFactory, &mut f) };
        say!(t, "kind {:#x}: {} out_untouched={} out_null={}", kind, Hr(hr), f == sentinel, f.is_null());
        if hr == 0 {
            release(f);
        }
    }
    let unrelated = [
        ("IDWriteFont", IID_IDWriteFont),
        ("IDWriteFontFace", IID_IDWriteFontFace),
        ("IDWriteFontCollection", IID_IDWriteFontCollection),
        ("random", GUID { d1: 0x1234_5678, d2: 0x9abc, d3: 0xdef0, d4: [1, 2, 3, 4, 5, 6, 7, 8] }),
    ];
    for kind in [DWRITE_FACTORY_TYPE_SHARED, DWRITE_FACTORY_TYPE_ISOLATED] {
        for (name, iid) in unrelated.iter() {
            let mut f = sentinel;
            // SAFETY: valid IID and out pointer.
            let hr = unsafe { DWriteCreateFactory(kind, iid, &mut f) };
            say!(t, "kind {} asks {}: {} out_untouched={} out_null={}", kind, name, Hr(hr), f == sentinel, f.is_null());
            if hr == 0 {
                release(f);
            }
        }
    }
}
