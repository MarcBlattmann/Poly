//! IDWriteFactory::CreateRenderingParams, CreateCustomRenderingParams and the
//! IDWriteRenderingParams getters.
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

/// A float in thousandths, rounded half away from zero (for exact, printable observations).
fn fx(v: f32) -> i64 {
    if v >= 0.0 { (v * 1000.0 + 0.5) as i64 } else { -((-v * 1000.0 + 0.5) as i64) }
}

const DWRITE_FACTORY_TYPE_ISOLATED: u32 = 1;
#[repr(C)]
struct IDWriteRenderingParamsVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetGamma: unsafe extern "system" fn(*mut c_void) -> f32,
    GetEnhancedContrast: unsafe extern "system" fn(*mut c_void) -> f32,
    GetClearTypeLevel: unsafe extern "system" fn(*mut c_void) -> f32,
    GetPixelGeometry: unsafe extern "system" fn(*mut c_void) -> u32,
    GetRenderingMode: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IDWriteRenderingParams {
    vtbl: *const IDWriteRenderingParamsVtbl,
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

fn show(t: &mut Test, tag: &str, hr: HRESULT, p: *mut IDWriteRenderingParams) {
    if p.is_null() {
        say!(t, "{}: {} null", tag, Hr(hr));
        return;
    }
    // SAFETY: p is live.
    unsafe {
        say!(
            t,
            "{}: {} gamma {} contrast {} cleartype level {} pixel geometry {} rendering mode {}",
            tag,
            Hr(hr),
            fx(com!(p, GetGamma())),
            fx(com!(p, GetEnhancedContrast())),
            fx(com!(p, GetClearTypeLevel())),
            com!(p, GetPixelGeometry()),
            com!(p, GetRenderingMode())
        )
    };
    release(p as *mut c_void);
}

fn test(t: &mut Test) {
    let fac = new_factory(DWRITE_FACTORY_TYPE_ISOLATED);
    let mut p: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid factory and out pointers.
    unsafe {
        let hr = com!(fac, CreateRenderingParams(outp!(p)));
        let pp = p as *mut IDWriteRenderingParams;
        let g = com!(pp, GetGamma());
        let c = com!(pp, GetEnhancedContrast());
        let l = com!(pp, GetClearTypeLevel());
        let pg = com!(pp, GetPixelGeometry());
        let rm = com!(pp, GetRenderingMode());
        say!(
            t,
            "CreateRenderingParams {}: gamma in 1..3 {} contrast in 0..2 {} cleartype level 0..1 {} pixel geometry <= 2 {} rendering mode <= 6 {}",
            Hr(hr),
            g >= 1.0 && g <= 3.0,
            c >= 0.0 && c <= 2.0,
            l >= 0.0 && l <= 1.0,
            pg <= 2,
            rm <= 6
        );
        release(p);
        for (tag, gamma, contrast, level, geom, mode) in [
            ("typical", 1.8f32, 0.5f32, 1.0f32, 1u32, 4u32),
            ("gamma 1", 1.0, 0.0, 0.0, 0, 1),
            ("gamma 2.2 flat", 2.2, 0.25, 0.5, 2, 0),
            ("rendering mode 6", 2.0, 1.0, 1.0, 1, 6),
            ("contrast 10", 2.0, 10.0, 1.0, 1, 4),
        ] {
            let mut q: *mut c_void = core::ptr::null_mut();
            let hr = com!(fac, CreateCustomRenderingParams(gamma, contrast, level, geom, mode, outp!(q)));
            let mut b = Buf::<100>::new();
            let _ = write!(b, "custom {}", tag);
            show(t, b.as_str(), hr, q as *mut IDWriteRenderingParams);
        }
        for (tag, gamma, contrast, level, geom, mode) in [
            ("gamma 0", 0.0f32, 0.5f32, 1.0f32, 1u32, 4u32),
            ("gamma negative", -1.0, 0.5, 1.0, 1, 4),
            ("contrast negative", 2.0, -0.5, 1.0, 1, 4),
            ("cleartype level 1.5", 2.0, 0.5, 1.5, 1, 4),
            ("cleartype level -0.1", 2.0, 0.5, -0.1, 1, 4),
            ("pixel geometry 3", 2.0, 0.5, 1.0, 3, 4),
            ("rendering mode 7", 2.0, 0.5, 1.0, 1, 7),
            ("rendering mode 100", 2.0, 0.5, 1.0, 1, 100),
        ] {
            let mut q: *mut c_void = 0x10usize as *mut c_void;
            let hr = com!(fac, CreateCustomRenderingParams(gamma, contrast, level, geom, mode, outp!(q)));
            say!(t, "custom {}: {} out_null {}", tag, Hr(hr), q.is_null());
            if hr == 0 {
                release(q);
            }
        }
    }
    release(fac as *mut c_void);
}
