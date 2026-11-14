//! COM ABI declarations: GUIDs and the vtable layouts of the DirectWrite
//! interfaces, in declaration order (Microsoft Learn; `dwrite.h`,
//! `dwrite_1.h` .. `dwrite_3.h`). The vtable structs were taken over from
//! the visible tests of `specs/windows/dwrite.dll`, which declare them from
//! the same headers. Pointers are untyped: the glue casts.
#![allow(non_snake_case, non_upper_case_globals, dead_code)]

use core::ffi::c_void;

pub type HRESULT = i32;

/// `guiddef.h`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GUID {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

pub const IID_IDWriteFactory: GUID =
    GUID { d1: 0xB859EE5A, d2: 0xD838, d3: 0x4B5B, d4: [0xA2, 0xE8, 0x1A, 0xDC, 0x7D, 0x93, 0xDB, 0x48] };
pub const IID_IDWriteFactory1: GUID =
    GUID { d1: 0x30572F99, d2: 0xDAC6, d3: 0x41DB, d4: [0xA1, 0x6E, 0x04, 0x86, 0x30, 0x7E, 0x60, 0x6A] };
pub const IID_IDWriteFactory2: GUID =
    GUID { d1: 0x0439FC60, d2: 0xCA44, d3: 0x4994, d4: [0x8D, 0xEE, 0x3A, 0x9A, 0xF7, 0xB7, 0x32, 0xEC] };
pub const IID_IDWriteFactory3: GUID =
    GUID { d1: 0x9A1B41C3, d2: 0xD3BB, d3: 0x466A, d4: [0x87, 0xFC, 0xFE, 0x67, 0x55, 0x6A, 0x3B, 0x65] };
pub const IID_IDWriteFactory4: GUID =
    GUID { d1: 0x4B0B5BD3, d2: 0x0797, d3: 0x4549, d4: [0x8A, 0xC5, 0xFE, 0x91, 0x5C, 0xC5, 0x38, 0x56] };
pub const IID_IDWriteFactory5: GUID =
    GUID { d1: 0x958DB99A, d2: 0xBE2A, d3: 0x4F09, d4: [0xAF, 0x7D, 0x65, 0x18, 0x98, 0x03, 0xD1, 0xD3] };
pub const IID_IDWriteFactory7: GUID =
    GUID { d1: 0x35D0E0B3, d2: 0x9076, d3: 0x4D2E, d4: [0xA0, 0x16, 0xA9, 0x1B, 0x56, 0x8A, 0x06, 0xB4] };
pub const IID_IDWriteFont: GUID =
    GUID { d1: 0xACD16696, d2: 0x8C14, d3: 0x4F5D, d4: [0x87, 0x7E, 0xFE, 0x3F, 0xC1, 0xD3, 0x27, 0x37] };
pub const IID_IDWriteFontCollection: GUID =
    GUID { d1: 0xA84CEE02, d2: 0x3EEA, d3: 0x4EEE, d4: [0xA8, 0x27, 0x87, 0xC1, 0xA0, 0x2A, 0x0F, 0xCC] };
pub const IID_IDWriteFontCollection1: GUID =
    GUID { d1: 0x53585141, d2: 0xD9F8, d3: 0x4095, d4: [0x83, 0x21, 0xD7, 0x3C, 0xF6, 0xBD, 0x11, 0x6C] };
pub const IID_IDWriteFontCollectionLoader: GUID =
    GUID { d1: 0xCCA920E4, d2: 0x52F0, d3: 0x492B, d4: [0xBF, 0xA8, 0x29, 0xC7, 0x2E, 0xE0, 0xA4, 0x68] };
pub const IID_IDWriteFontFace: GUID =
    GUID { d1: 0x5F49804D, d2: 0x7024, d3: 0x4D43, d4: [0xBF, 0xA9, 0xD2, 0x59, 0x84, 0xF5, 0x38, 0x49] };
pub const IID_IDWriteFontFileEnumerator: GUID =
    GUID { d1: 0x72755049, d2: 0x5FF7, d3: 0x435D, d4: [0x83, 0x48, 0x4B, 0xE9, 0x7C, 0xFA, 0x6C, 0x7C] };
pub const IID_IDWriteFontFileLoader: GUID =
    GUID { d1: 0x727CAD4E, d2: 0xD6AF, d3: 0x4C9E, d4: [0x8A, 0x08, 0xD6, 0x95, 0xB1, 0x1C, 0xAA, 0x49] };
pub const IID_IDWriteFontFileStream: GUID =
    GUID { d1: 0x6D4865FE, d2: 0x0AB8, d3: 0x4D91, d4: [0x8F, 0x62, 0x5D, 0xD6, 0xBE, 0x34, 0xA3, 0xE0] };
pub const IID_IDWriteFontSetBuilder1: GUID =
    GUID { d1: 0x3FF7715F, d2: 0x3CDC, d3: 0x4DC6, d4: [0x9B, 0x72, 0xEC, 0x56, 0x21, 0xDC, 0xCA, 0xFD] };
pub const IID_IDWriteLocalFontFileLoader: GUID =
    GUID { d1: 0xB2D9F3EC, d2: 0xC9FE, d3: 0x4A11, d4: [0xA2, 0xEC, 0xD8, 0x62, 0x08, 0xF7, 0xC0, 0xA2] };
pub const IID_IDWriteTextAnalysisSource: GUID =
    GUID { d1: 0x688E1A58, d2: 0x5094, d3: 0x47C8, d4: [0xAD, 0xC8, 0xFB, 0xCE, 0xA6, 0x0A, 0xE9, 0x2B] };
pub const IID_IUnknown: GUID = GUID { d1: 0, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFactory1Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomFontCollection:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomRenderingParams:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextFormat: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    pub CreateGdiCompatibleTextLayout: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> i32,
    pub CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis:
        unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
    pub GetEudcFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomRenderingParams_1:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFactory2Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomFontCollection:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomRenderingParams:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextFormat: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    pub CreateGdiCompatibleTextLayout: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> i32,
    pub CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis:
        unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
    pub GetEudcFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomRenderingParams_1:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontFallback: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFallbackBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub TranslateColorGlyphRun: unsafe extern "system" fn(
        *mut c_void,
        f32,
        f32,
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_2:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis_2: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFactory3Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomFontCollection:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomRenderingParams:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextFormat: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    pub CreateGdiCompatibleTextLayout: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> i32,
    pub CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis:
        unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
    pub GetEudcFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomRenderingParams_1:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontFallback: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFallbackBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub TranslateColorGlyphRun: unsafe extern "system" fn(
        *mut c_void,
        f32,
        f32,
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_2:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis_2: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
    pub CreateGlyphRunAnalysis_3: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_3:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontSetBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontCollectionFromFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetSystemFontCollection_3: unsafe extern "system" fn(*mut c_void, i32, *mut c_void, i32) -> i32,
    pub GetFontDownloadQueue: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFactory4Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomFontCollection:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomRenderingParams:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextFormat: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    pub CreateGdiCompatibleTextLayout: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> i32,
    pub CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis:
        unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
    pub GetEudcFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomRenderingParams_1:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontFallback: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFallbackBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub TranslateColorGlyphRun: unsafe extern "system" fn(
        *mut c_void,
        f32,
        f32,
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_2:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis_2: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
    pub CreateGlyphRunAnalysis_3: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_3:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontSetBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontCollectionFromFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetSystemFontCollection_3: unsafe extern "system" fn(*mut c_void, i32, *mut c_void, i32) -> i32,
    pub GetFontDownloadQueue: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub TranslateColorGlyphRun_4: unsafe extern "system" fn(
        *mut c_void,
        u64,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> i32,
    pub ComputeGlyphOrigins:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u64, *mut c_void, *mut c_void) -> i32,
    pub ComputeGlyphOrigins_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFactory5Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomFontCollection:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomRenderingParams:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextFormat: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    pub CreateGdiCompatibleTextLayout: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> i32,
    pub CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis:
        unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
    pub GetEudcFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomRenderingParams_1:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontFallback: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFallbackBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub TranslateColorGlyphRun: unsafe extern "system" fn(
        *mut c_void,
        f32,
        f32,
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_2:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis_2: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
    pub CreateGlyphRunAnalysis_3: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_3:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontSetBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontCollectionFromFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetSystemFontCollection_3: unsafe extern "system" fn(*mut c_void, i32, *mut c_void, i32) -> i32,
    pub GetFontDownloadQueue: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub TranslateColorGlyphRun_4: unsafe extern "system" fn(
        *mut c_void,
        u64,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> i32,
    pub ComputeGlyphOrigins:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u64, *mut c_void, *mut c_void) -> i32,
    pub ComputeGlyphOrigins_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void) -> i32,
    pub CreateFontSetBuilder_5: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateInMemoryFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateHttpFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AnalyzeContainerType: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> u32,
    pub UnpackFontFile: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFactory7Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomFontCollection:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomRenderingParams:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextFormat: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    pub CreateGdiCompatibleTextLayout: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> i32,
    pub CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis:
        unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
    pub GetEudcFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomRenderingParams_1:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontFallback: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFallbackBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub TranslateColorGlyphRun: unsafe extern "system" fn(
        *mut c_void,
        f32,
        f32,
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_2:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis_2: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
    pub CreateGlyphRunAnalysis_3: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        f32,
        f32,
        *mut c_void,
    ) -> i32,
    pub CreateCustomRenderingParams_3:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub GetSystemFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontSetBuilder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontCollectionFromFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetSystemFontCollection_3: unsafe extern "system" fn(*mut c_void, i32, *mut c_void, i32) -> i32,
    pub GetFontDownloadQueue: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub TranslateColorGlyphRun_4: unsafe extern "system" fn(
        *mut c_void,
        u64,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> i32,
    pub ComputeGlyphOrigins:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u64, *mut c_void, *mut c_void) -> i32,
    pub ComputeGlyphOrigins_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void) -> i32,
    pub CreateFontSetBuilder_5: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateInMemoryFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateHttpFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AnalyzeContainerType: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> u32,
    pub UnpackFontFile: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32, *mut c_void) -> i32,
    pub CreateFontFaceReference_6:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, *mut c_void, u32, *mut c_void) -> i32,
    pub CreateFontResource: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub GetSystemFontSet_6: unsafe extern "system" fn(*mut c_void, i32, *mut c_void) -> i32,
    pub GetSystemFontCollection_6: unsafe extern "system" fn(*mut c_void, i32, u32, *mut c_void) -> i32,
    pub CreateFontCollectionFromFontSet_6: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub CreateFontSetBuilder_6: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextFormat_6: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub GetSystemFontSet_7: unsafe extern "system" fn(*mut c_void, i32, *mut c_void) -> i32,
    pub GetSystemFontCollection_7: unsafe extern "system" fn(*mut c_void, i32, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFactoryVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    pub CreateCustomFontCollection:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    pub CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateCustomRenderingParams:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    pub RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextFormat: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    pub CreateGdiCompatibleTextLayout: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> i32,
    pub CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    pub CreateGlyphRunAnalysis:
        unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontCollection1Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontFamilyCount: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontFamily: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub FindFamilyName: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetFontFromFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetFontFamily_1: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontCollectionLoaderVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub CreateEnumeratorFromKey:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontCollectionVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontFamilyCount: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontFamily: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub FindFamilyName: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetFontFromFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFaceReferenceVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFaceWithSimulations: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub Equals: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetFontFaceIndex: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSimulations: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontFile: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetLocalFileSize: unsafe extern "system" fn(*mut c_void) -> u64,
    pub GetFileSize: unsafe extern "system" fn(*mut c_void) -> u64,
    pub GetFileTime: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetLocality: unsafe extern "system" fn(*mut c_void) -> u32,
    pub EnqueueFontDownloadRequest: unsafe extern "system" fn(*mut c_void) -> i32,
    pub EnqueueCharacterDownloadRequest: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32,
    pub EnqueueGlyphDownloadRequest: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32,
    pub EnqueueFileFragmentDownloadRequest: unsafe extern "system" fn(*mut c_void, u64, u64) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFaceVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetType: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFiles: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetIndex: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetSimulations: unsafe extern "system" fn(*mut c_void) -> u32,
    pub IsSymbolFont: unsafe extern "system" fn(*mut c_void) -> i32,
    pub GetMetrics: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub GetGlyphCount: unsafe extern "system" fn(*mut c_void) -> u16,
    pub GetDesignGlyphMetrics: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, i32) -> i32,
    pub GetGlyphIndices: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub TryGetFontTable:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub ReleaseFontTable: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub GetGlyphRunOutline: unsafe extern "system" fn(
        *mut c_void,
        f32,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        i32,
        i32,
        *mut c_void,
    ) -> i32,
    pub GetRecommendedRenderingMode:
        unsafe extern "system" fn(*mut c_void, f32, f32, u32, *mut c_void, *mut c_void) -> i32,
    pub GetGdiCompatibleMetrics: unsafe extern "system" fn(*mut c_void, f32, f32, *mut c_void, *mut c_void) -> i32,
    pub GetGdiCompatibleGlyphMetrics:
        unsafe extern "system" fn(*mut c_void, f32, f32, *mut c_void, i32, *mut c_void, u32, *mut c_void, i32) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFallbackBuilderVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub AddMapping: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        f32,
    ) -> i32,
    pub AddMappings: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontFallback: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFallbackVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub MapCharacters: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFamilyVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetFontCount: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFont: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub GetFamilyNames: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetFirstMatchingFont: unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32,
    pub GetMatchingFonts: unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFileEnumeratorVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub MoveNext: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetCurrentFontFile: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFileLoaderVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub CreateStreamFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFileStreamVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub ReadFileFragment: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u64, *mut c_void) -> i32,
    pub ReleaseFileFragment: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub GetFileSize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetLastWriteTime: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFileVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetReferenceKey: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub Analyze: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontListVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetFontCount: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFont: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontSetBuilder1Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub AddFontFaceReference: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub AddFontFaceReference_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32) -> i32,
    pub AddFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub CreateFontSet: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub AddFontFile: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontSetVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontCount: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontFaceReference: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub FindFontFaceReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub FindFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetPropertyValues: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, *mut c_void) -> i32,
    pub GetPropertyValues_ov2: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub GetPropertyValues_ov3: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub GetPropertyOccurrenceCount: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetMatchingFonts: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub GetMatchingFonts_ov2: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetFontFamily: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetWeight: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetStretch: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetStyle: unsafe extern "system" fn(*mut c_void) -> u32,
    pub IsSymbolFont: unsafe extern "system" fn(*mut c_void) -> i32,
    pub GetFaceNames: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    pub GetInformationalStrings: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub GetSimulations: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetMetrics: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub HasCharacter: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub CreateFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteGlyphRunAnalysisVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetAlphaTextureBounds: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub CreateAlphaTexture: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void, u32) -> i32,
    pub GetAlphaBlendParams:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteInMemoryFontFileLoaderVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub CreateStreamFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub CreateInMemoryFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub GetFileCount: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteLocalFontFileLoaderVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub CreateStreamFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub GetFilePathLengthFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    pub GetFilePathFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u32) -> i32,
    pub GetLastWriteTimeFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteLocalizedStringsVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetCount: unsafe extern "system" fn(*mut c_void) -> u32,
    pub FindLocaleName: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    pub GetLocaleNameLength: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub GetLocaleName: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32,
    pub GetStringLength: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    pub GetString: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteRenderingParamsVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetGamma: unsafe extern "system" fn(*mut c_void) -> f32,
    pub GetEnhancedContrast: unsafe extern "system" fn(*mut c_void) -> f32,
    pub GetClearTypeLevel: unsafe extern "system" fn(*mut c_void) -> f32,
    pub GetPixelGeometry: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetRenderingMode: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IUnknownVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
}

// ----- GUIDs and layouts the tests do not declare

macro_rules! guid {
    ($name:ident, $d1:expr, $d2:expr, $d3:expr, $($d4:expr),*) => {
        pub const $name: GUID = GUID { d1: $d1, d2: $d2, d3: $d3, d4: [$($d4),*] };
    };
}

guid!(IID_IDWriteFontFile, 0x739D886A, 0xCEF5, 0x47DC, 0x87, 0x69, 0x1A, 0x8B, 0x41, 0xBE, 0xBB, 0xB0);
guid!(IID_IDWriteFontFamily, 0xDA20D8EF, 0x812A, 0x4C43, 0x98, 0x02, 0x62, 0xEC, 0x4A, 0xBD, 0x7A, 0xDD);
guid!(IID_IDWriteFontList, 0x1A0D8438, 0x1D97, 0x4EC1, 0xAE, 0xF9, 0xA2, 0xFB, 0x86, 0xED, 0x6A, 0xCB);
guid!(IID_IDWriteLocalizedStrings, 0x08256209, 0x099A, 0x4B34, 0xB8, 0x6D, 0xC2, 0x2B, 0x11, 0x0E, 0x77, 0x71);
guid!(IID_IDWriteNumberSubstitution, 0x14885CC9, 0xBAB0, 0x4F90, 0xB6, 0xED, 0x5C, 0x36, 0x6A, 0x2C, 0xD0, 0x3D);
guid!(IID_IDWriteRenderingParams, 0x2F0DA53A, 0x2ADD, 0x47CD, 0x82, 0xEE, 0xD9, 0xEC, 0x34, 0x68, 0x8E, 0x75);
guid!(IID_IDWriteRenderingParams1, 0x94413CF4, 0xA6FC, 0x4248, 0x8B, 0x50, 0x66, 0x74, 0x34, 0x8F, 0xCA, 0xD3);
guid!(IID_IDWriteRenderingParams2, 0xF9D711C3, 0x9777, 0x40AE, 0x87, 0xE8, 0x3E, 0x5A, 0xF9, 0xBF, 0x09, 0x48);
guid!(IID_IDWriteRenderingParams3, 0xB7924BAA, 0x391B, 0x412A, 0x8C, 0x5C, 0xE4, 0x4C, 0xC2, 0xD8, 0x67, 0xDC);
guid!(IID_IDWriteGlyphRunAnalysis, 0x7D97DBF7, 0xE085, 0x42D4, 0x81, 0xE3, 0x6A, 0x88, 0x3B, 0xDE, 0xD1, 0x18);
guid!(IID_IDWriteFontSet, 0x53585141, 0xD9F8, 0x4095, 0x83, 0x21, 0xD7, 0x3C, 0xF6, 0xBD, 0x11, 0x6B);
guid!(IID_IDWriteFontSetBuilder, 0x2F642AFE, 0x9C68, 0x4F40, 0xB8, 0xBE, 0x45, 0x74, 0x01, 0xAF, 0xCB, 0x3D);
guid!(IID_IDWriteFontFallback, 0xEFA008F9, 0xF7A1, 0x48BF, 0xB0, 0x5C, 0xF2, 0x24, 0x71, 0x3C, 0xC0, 0xFF);
guid!(IID_IDWriteFontFallbackBuilder, 0xFD882D06, 0x8ABA, 0x4FB8, 0xB8, 0x49, 0x8B, 0xE8, 0xB7, 0x3E, 0x14, 0xDE);
guid!(IID_IDWriteFontFaceReference, 0x5E7FA7CA, 0xDDE3, 0x424C, 0x89, 0xF0, 0x9F, 0xCD, 0x6F, 0xED, 0x58, 0xCD);
guid!(IID_IDWriteInMemoryFontFileLoader, 0xDC102F47, 0xA12D, 0x4B1C, 0x82, 0x2D, 0x9E, 0x11, 0x7E, 0x33, 0x04, 0x3F);
guid!(IID_IDWriteFactory6, 0xF3744D80, 0x21F7, 0x42EB, 0xB3, 0x5D, 0x99, 0x5B, 0xC7, 0x2F, 0xC2, 0x23);
guid!(IID_IDWriteFontFace1, 0xA71EFDB4, 0x9FDB, 0x4838, 0xAD, 0x90, 0xCF, 0xC3, 0xBE, 0x8C, 0x3D, 0xAF);
guid!(IID_IDWriteFont1, 0xACD16696, 0x8C14, 0x4F5D, 0x87, 0x7E, 0xFE, 0x3F, 0xC1, 0xD3, 0x27, 0x38);

/// `IDWriteRenderingParams3`: the three levels extend one table.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteRenderingParams3Vtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetGamma: unsafe extern "system" fn(*mut c_void) -> f32,
    pub GetEnhancedContrast: unsafe extern "system" fn(*mut c_void) -> f32,
    pub GetClearTypeLevel: unsafe extern "system" fn(*mut c_void) -> f32,
    pub GetPixelGeometry: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetRenderingMode: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetGrayscaleEnhancedContrast: unsafe extern "system" fn(*mut c_void) -> f32,
    pub GetGridFitMode: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetRenderingMode1: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// `IDWriteFontFace1` (`dwrite_1.h`): `IDWriteFontFace` plus these.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IDWriteFontFace1Vtbl {
    pub base: IDWriteFontFaceVtbl,
    pub GetMetrics_1: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub GetGdiCompatibleMetrics_1: unsafe extern "system" fn(*mut c_void, f32, f32, *mut c_void, *mut c_void) -> i32,
    pub GetCaretMetrics: unsafe extern "system" fn(*mut c_void, *mut c_void),
    pub GetUnicodeRanges: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub IsMonospacedFont: unsafe extern "system" fn(*mut c_void) -> i32,
    pub GetDesignGlyphAdvances: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void, i32) -> i32,
    pub GetGdiCompatibleGlyphAdvances:
        unsafe extern "system" fn(*mut c_void, f32, f32, *mut c_void, i32, i32, u32, *mut c_void, *mut c_void) -> i32,
    pub GetKerningPairAdjustments: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub HasKerningPairs: unsafe extern "system" fn(*mut c_void) -> i32,
    pub GetRecommendedRenderingMode_1: unsafe extern "system" fn(
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        u32,
        u32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    pub GetVerticalGlyphVariants: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    pub HasVerticalGlyphVariants: unsafe extern "system" fn(*mut c_void) -> i32,
}
