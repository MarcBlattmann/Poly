//! DrawTextW and the mnemonic prefix '&' with the private font (PolyTest Sans, lfHeight -16;
//! the font's underline sits 2 pixels below the baseline and is 1 pixel thick): what is
//! measured and what is drawn for "&A", "A&B", "A&&B", a trailing '&', and the flags
//! DT_NOPREFIX, DT_HIDEPREFIX and DT_PREFIXONLY; the pixels of the underline under the marked
//! letter. The oracle session shows the keyboard cues the way a fresh desktop does.
#![no_std]
#![no_main]
#![allow(dead_code, non_snake_case, non_camel_case_types, unused_imports, unused_macros, unused_mut, clippy::all)]

// ---------------------------------------------------------------------------
// GDI text test prelude (spec-win-text). The same items are in every test of
// the slice, so each file stands alone. Prototypes from Microsoft Learn;
// constants and structure layouts from the public SDK headers wingdi.h,
// winuser.h, windef.h and winbase.h. The TrueType writer, the helpers and the
// font's glyph table are this test's own: the tests never depend on the pixels
// or metrics of a Microsoft font, they draw with a font they make themselves
// (AddFontMemResourceEx / AddFontResourceExW) or check relations that hold for
// every font. Written for Poly; no code copied from anywhere.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type HFONT = *mut c_void;
type BOOL = i32;
type COLORREF = u32;

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct SIZE {
    cx: i32,
    cy: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// wingdi.h (92 bytes)
#[repr(C)]
#[derive(Clone, Copy)]
struct LOGFONTW {
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u8,
    underline: u8,
    strike_out: u8,
    char_set: u8,
    out_precision: u8,
    clip_precision: u8,
    quality: u8,
    pitch_and_family: u8,
    face: [u16; 32],
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct TEXTMETRICW {
    height: i32,
    ascent: i32,
    descent: i32,
    internal_leading: i32,
    external_leading: i32,
    ave_char_width: i32,
    max_char_width: i32,
    weight: i32,
    overhang: i32,
    digitized_aspect_x: i32,
    digitized_aspect_y: i32,
    first_char: u16,
    last_char: u16,
    default_char: u16,
    break_char: u16,
    italic: u8,
    underlined: u8,
    struck_out: u8,
    pitch_and_family: u8,
    char_set: u8,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct ENUMLOGFONTEXW {
    lf: LOGFONTW,
    full_name: [u16; 64],
    style: [u16; 32],
    script: [u16; 32],
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct DRAWTEXTPARAMS {
    cb_size: u32,
    tab_length: i32,
    left_margin: i32,
    right_margin: i32,
    length_drawn: u32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAPINFOHEADER {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

/// wingdi.h (one RGBQUAD as a u32)
#[repr(C)]
struct BITMAPINFO {
    header: BITMAPINFOHEADER,
    colors: [u32; 1],
}

// wingdi.h / winuser.h constants
const TA_NOUPDATECP: u32 = 0;
const TA_UPDATECP: u32 = 1;
const TA_LEFT: u32 = 0;
const TA_RIGHT: u32 = 2;
const TA_CENTER: u32 = 6;
const TA_TOP: u32 = 0;
const TA_BOTTOM: u32 = 8;
const TA_BASELINE: u32 = 24;
const TRANSPARENT: i32 = 1;
const OPAQUE: i32 = 2;
const ETO_OPAQUE: u32 = 0x2;
const ETO_CLIPPED: u32 = 0x4;
const ETO_GLYPH_INDEX: u32 = 0x10;
const FW_DONTCARE: i32 = 0;
const FW_NORMAL: i32 = 400;
const FW_BOLD: i32 = 700;
const DEFAULT_CHARSET: u8 = 1;
const ANSI_CHARSET: u8 = 0;
const SYMBOL_CHARSET: u8 = 2;
const OUT_DEFAULT_PRECIS: u8 = 0;
const OUT_TT_PRECIS: u8 = 4;
const OUT_TT_ONLY_PRECIS: u8 = 7;
const CLIP_DEFAULT_PRECIS: u8 = 0;
const DEFAULT_QUALITY: u8 = 0;
const DRAFT_QUALITY: u8 = 1;
const PROOF_QUALITY: u8 = 2;
const NONANTIALIASED_QUALITY: u8 = 3;
const ANTIALIASED_QUALITY: u8 = 4;
const CLEARTYPE_QUALITY: u8 = 5;
const DEFAULT_PITCH: u8 = 0;
const FIXED_PITCH: u8 = 1;
const VARIABLE_PITCH: u8 = 2;
const FF_DONTCARE: u8 = 0x00;
const FF_ROMAN: u8 = 0x10;
const FF_SWISS: u8 = 0x20;
const FF_MODERN: u8 = 0x30;
const TMPF_FIXED_PITCH: u8 = 1;
const TMPF_VECTOR: u8 = 2;
const TMPF_TRUETYPE: u8 = 4;
const TMPF_DEVICE: u8 = 8;
const OBJ_FONT: u32 = 6;
const SYSTEM_FONT: i32 = 13;
const DEFAULT_GUI_FONT: i32 = 17;
const ANSI_FIXED_FONT: i32 = 11;
const ANSI_VAR_FONT: i32 = 12;
const OEM_FIXED_FONT: i32 = 10;
const FR_PRIVATE: u32 = 0x10;
const FR_NOT_ENUM: u32 = 0x20;
const RASTER_FONTTYPE: u32 = 1;
const DEVICE_FONTTYPE: u32 = 2;
const TRUETYPE_FONTTYPE: u32 = 4;
const GDI_ERROR: u32 = 0xFFFF_FFFF;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;

const DT_TOP: u32 = 0x0;
const DT_LEFT: u32 = 0x0;
const DT_CENTER: u32 = 0x1;
const DT_RIGHT: u32 = 0x2;
const DT_VCENTER: u32 = 0x4;
const DT_BOTTOM: u32 = 0x8;
const DT_WORDBREAK: u32 = 0x10;
const DT_SINGLELINE: u32 = 0x20;
const DT_EXPANDTABS: u32 = 0x40;
const DT_TABSTOP: u32 = 0x80;
const DT_NOCLIP: u32 = 0x100;
const DT_EXTERNALLEADING: u32 = 0x200;
const DT_CALCRECT: u32 = 0x400;
const DT_NOPREFIX: u32 = 0x800;
const DT_INTERNAL: u32 = 0x1000;
const DT_EDITCONTROL: u32 = 0x2000;
const DT_PATH_ELLIPSIS: u32 = 0x4000;
const DT_END_ELLIPSIS: u32 = 0x8000;
const DT_MODIFYSTRING: u32 = 0x10000;
const DT_RTLREADING: u32 = 0x20000;
const DT_WORD_ELLIPSIS: u32 = 0x40000;
const DT_NOFULLWIDTHCHARBREAK: u32 = 0x80000;
const DT_HIDEPREFIX: u32 = 0x100000;
const DT_PREFIXONLY: u32 = 0x200000;

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disposition: u32, flags: u32, template: HANDLE) -> HANDLE;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFontW(
        h: i32,
        w: i32,
        esc: i32,
        orient: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        out_prec: u32,
        clip_prec: u32,
        quality: u32,
        pitch_family: u32,
        face: *const u16,
    ) -> HFONT;
    fn CreateFontIndirectW(lf: *const LOGFONTW) -> HFONT;
    fn GetObjectW(h: HGDIOBJ, n: i32, out: *mut c_void) -> i32;
    fn GetObjectType(h: HGDIOBJ) -> u32;
    fn GetStockObject(i: i32) -> HGDIOBJ;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateDIBSection(hdc: HDC, info: *const BITMAPINFO, usage: u32, bits: *mut *mut c_void, section: HANDLE, offset: u32) -> HBITMAP;
    fn GdiFlush() -> BOOL;
    fn SetTextColor(hdc: HDC, c: COLORREF) -> COLORREF;
    fn GetTextColor(hdc: HDC) -> COLORREF;
    fn SetBkColor(hdc: HDC, c: COLORREF) -> COLORREF;
    fn SetBkMode(hdc: HDC, m: i32) -> i32;
    fn SetTextAlign(hdc: HDC, a: u32) -> u32;
    fn GetTextAlign(hdc: HDC) -> u32;
    fn MoveToEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn GetCurrentPositionEx(hdc: HDC, p: *mut POINT) -> BOOL;
    fn TextOutW(hdc: HDC, x: i32, y: i32, text: *const u16, n: i32) -> BOOL;
    fn ExtTextOutW(hdc: HDC, x: i32, y: i32, options: u32, rc: *const RECT, text: *const u16, n: u32, dx: *const i32) -> BOOL;
    fn GetTextMetricsW(hdc: HDC, tm: *mut TEXTMETRICW) -> BOOL;
    fn GetTextFaceW(hdc: HDC, n: i32, out: *mut u16) -> i32;
    fn GetTextExtentPoint32W(hdc: HDC, text: *const u16, n: i32, size: *mut SIZE) -> BOOL;
    fn GetTextExtentPointW(hdc: HDC, text: *const u16, n: i32, size: *mut SIZE) -> BOOL;
    fn GetTextExtentExPointW(hdc: HDC, text: *const u16, n: i32, max_extent: i32, fit: *mut i32, dx: *mut i32, size: *mut SIZE) -> BOOL;
    fn GetCharWidth32W(hdc: HDC, first: u32, last: u32, out: *mut i32) -> BOOL;
    fn GetCharWidthW(hdc: HDC, first: u32, last: u32, out: *mut i32) -> BOOL;
    fn AddFontMemResourceEx(data: *const c_void, n: u32, reserved: *mut c_void, count: *mut u32) -> HANDLE;
    fn RemoveFontMemResourceEx(h: HANDLE) -> BOOL;
    fn AddFontResourceExW(name: *const u16, flags: u32, reserved: *mut c_void) -> i32;
    fn RemoveFontResourceExW(name: *const u16, flags: u32, reserved: *mut c_void) -> BOOL;
    fn EnumFontFamiliesExW(
        hdc: HDC,
        lf: *const LOGFONTW,
        proc_: unsafe extern "system" fn(*const ENUMLOGFONTEXW, *const c_void, u32, isize) -> i32,
        lparam: isize,
        flags: u32,
    ) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DrawTextW(hdc: HDC, text: *mut u16, n: i32, rc: *mut RECT, format: u32) -> i32;
    fn DrawTextExW(hdc: HDC, text: *mut u16, n: i32, rc: *mut RECT, format: u32, params: *mut DRAWTEXTPARAMS) -> i32;
}

/// A global shared with callbacks (one thread).
struct Global<T>(UnsafeCell<T>);
// SAFETY: every test of this slice uses its globals from one thread.
unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: see the Sync impl: no two live references at once.
        unsafe { &mut *self.0.get() }
    }
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
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// Prints UTF-16 units as text: printable ASCII as is, the rest as `\u{hex}`.
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

/// UTF-16 units of a NUL-terminated buffer, without the NUL.
fn upto_nul(b: &[u16]) -> &[u16] {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    &b[..n]
}

/// UTF-16 text (NUL-terminated) of a `&str`, into `out`; the count without the NUL.
fn wide(s: &str, out: &mut [u16]) -> usize {
    let mut n = 0;
    for u in s.encode_utf16() {
        if n + 1 < out.len() {
            out[n] = u;
            n += 1;
        }
    }
    out[n] = 0;
    n
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// The DIB value (0x00RRGGBB) of a COLORREF.
fn dibv(c: COLORREF) -> u32 {
    (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
}

fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

// ---- drawing surface --------------------------------------------------------

/// A memory DC with a 32-bit top-down DIB section selected. Pixels are
/// 0x00RRGGBB. Black text on white unless the test says otherwise.
struct Surf {
    dc: HDC,
    bmp: HBITMAP,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Surf {
    fn new(w: i32, h: i32) -> Surf {
        let info = BITMAPINFO {
            header: BITMAPINFOHEADER {
                size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                width: w,
                height: -h,
                planes: 1,
                bit_count: 32,
                compression: BI_RGB,
                size_image: 0,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0],
        };
        let mut bits: *mut c_void = core::ptr::null_mut();
        // SAFETY: a filled BITMAPINFO; bits receives the pixel pointer.
        unsafe {
            let dc = CreateCompatibleDC(core::ptr::null_mut());
            let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
            SelectObject(dc, bmp);
            let s = Surf { dc, bmp, bits: bits as *mut u32, w, h };
            s.clear(0x00FF_FFFF);
            s
        }
    }
    fn clear(&self, v: u32) {
        for i in 0..(self.w * self.h) as usize {
            // SAFETY: inside the DIB.
            unsafe { *self.bits.add(i) = v };
        }
    }
    fn px(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0xDEAD_BEEF;
        }
        // SAFETY: inside the DIB (checked above).
        unsafe { *self.bits.add((y * self.w + x) as usize) }
    }
    fn set(&self, x: i32, y: i32, v: u32) {
        // SAFETY: callers stay inside the DIB.
        unsafe { *self.bits.add((y * self.w + x) as usize) = v };
    }
    /// GdiFlush, then the bounding box and count of pixels not `bg`
    /// inside the rectangle (all of the surface if `r` is None).
    fn ink_in(&self, r: Option<RECT>, bg: u32) -> (i32, i32, i32, i32, u32) {
        // SAFETY: flushes batched GDI calls.
        unsafe { GdiFlush() };
        let r = r.unwrap_or(RECT { left: 0, top: 0, right: self.w, bottom: self.h });
        let (mut l, mut tp, mut rr, mut b, mut n) = (i32::MAX, i32::MAX, -1, -1, 0u32);
        for y in r.top.max(0)..r.bottom.min(self.h) {
            for x in r.left.max(0)..r.right.min(self.w) {
                if self.px(x, y) != bg {
                    l = l.min(x);
                    tp = tp.min(y);
                    rr = rr.max(x + 1);
                    b = b.max(y + 1);
                    n += 1;
                }
            }
        }
        if n == 0 { (0, 0, 0, 0, 0) } else { (l, tp, rr, b, n) }
    }
    fn ink(&self, bg: u32) -> (i32, i32, i32, i32, u32) {
        self.ink_in(None, bg)
    }
    /// The number of pixels with value `v` in `r`.
    fn count(&self, r: RECT, v: u32) -> u32 {
        // SAFETY: flushes batched GDI calls.
        unsafe { GdiFlush() };
        let mut n = 0;
        for y in r.top.max(0)..r.bottom.min(self.h) {
            for x in r.left.max(0)..r.right.min(self.w) {
                if self.px(x, y) == v {
                    n += 1;
                }
            }
        }
        n
    }
    /// The rows of the rectangle as text: `.` for `bg`, `#` for `fg`, `+` for anything else.
    fn rows(&self, t: &mut Test, r: RECT, bg: u32, fg: u32) {
        // SAFETY: flushes batched GDI calls.
        unsafe { GdiFlush() };
        for y in r.top..r.bottom {
            let mut b = Buf::<300>::new();
            let _ = write!(b, "  |");
            for x in r.left..r.right {
                let v = self.px(x, y);
                let _ = b.write_char(if v == bg {
                    '.'
                } else if v == fg {
                    '#'
                } else {
                    '+'
                });
            }
            let _ = b.write_str("|");
            t.line(b.as_str());
        }
    }
    fn free(&self) {
        // SAFETY: our DC and bitmap.
        unsafe {
            DeleteDC(self.dc);
            DeleteObject(self.bmp);
        }
    }
}

// ---- a TrueType font written by the test -------------------------------------
//
// Fourteen glyphs on a 1024-unit em; every outline coordinate is a multiple
// of 64 units, so at 16 pixels per em (lfHeight -16) every edge lies on a
// pixel boundary and the glyphs are the same boxes in every rasteriser.

/// One glyph: code point (0 for .notdef), advance width, contours (points
/// on the curve only, a straight-edged outline).
struct G {
    cp: u32,
    adv: u16,
    c: &'static [&'static [(i16, i16)]],
}

macro_rules! r {
    ($x0:expr, $y0:expr, $x1:expr, $y1:expr) => {
        &[($x0, $y0), ($x0, $y1), ($x1, $y1), ($x1, $y0)]
    };
}

/// Advances in pixels at 16 per em (units / 64): notdef 10, space 6, '&' 9,
/// '-' 6, '.' 4, 'A' 12, 'B' 8, 'M' 12, 'W' 16, backslash 6, 'g' 10, 'i' 4,
/// U+00E9 11, U+20AC 14.
static GLYPHS: [G; 14] = [
    G {
        cp: 0,
        adv: 640,
        c: &[&[(64, 0), (64, 768), (576, 768), (576, 0)], &[(192, 128), (448, 128), (448, 640), (192, 640)]],
    },
    G { cp: 0x20, adv: 384, c: &[] },
    G { cp: 0x26, adv: 576, c: &[r!(64, 0, 512, 640)] },
    G { cp: 0x2D, adv: 384, c: &[r!(64, 256, 320, 320)] },
    G { cp: 0x2E, adv: 256, c: &[r!(64, 0, 192, 128)] },
    G { cp: 0x41, adv: 768, c: &[r!(128, 0, 640, 768)] },
    G { cp: 0x42, adv: 512, c: &[r!(64, 0, 448, 512)] },
    G { cp: 0x4D, adv: 768, c: &[&[(0, 0), (384, 768), (768, 0)]] },
    G { cp: 0x57, adv: 1024, c: &[r!(128, 0, 896, 768)] },
    G { cp: 0x5C, adv: 384, c: &[r!(64, 0, 320, 768)] },
    G { cp: 0x67, adv: 640, c: &[r!(64, -256, 576, 384)] },
    G { cp: 0x69, adv: 256, c: &[r!(64, 0, 192, 768)] },
    G { cp: 0xE9, adv: 704, c: &[r!(64, 0, 640, 512)] },
    G { cp: 0x20AC, adv: 896, c: &[r!(64, 0, 832, 768)] },
];

/// What a font written by [`build_font`] looks like.
#[derive(Clone, Copy)]
struct FontDef {
    family: &'static str,
    style: &'static str,
    weight_class: u16,
    bold: bool,
    italic: bool,
    /// every advance 640 and the post table says fixed pitch
    fixed: bool,
    win_asc: u16,
    win_desc: u16,
    hhea_asc: i16,
    hhea_desc: i16,
    line_gap: i16,
    typo: [i16; 3],
    avg_w: i16,
    /// the advance of 'A' in units (0 keeps the glyph table's 768)
    a_adv: u16,
    underline_pos: i16,
    underline_thick: i16,
    strike_pos: i16,
    strike_size: i16,
    italic_angle_fixed: i32,
    /// builder switches (probing which table layouts GDI accepts)
    opt: u32,
}

/// "PolyTest Sans": cell 1024 units (one em), no internal leading; line gap 128.
const SANS: FontDef = FontDef {
    family: "PolyTest Sans",
    style: "Regular",
    weight_class: 400,
    bold: false,
    italic: false,
    fixed: false,
    win_asc: 768,
    win_desc: 256,
    hhea_asc: 768,
    hhea_desc: -256,
    line_gap: 128,
    typo: [768, -256, 128],
    avg_w: 512,
    a_adv: 0,
    underline_pos: -128,
    underline_thick: 64,
    strike_pos: 256,
    strike_size: 64,
    italic_angle_fixed: 0,
    opt: 0,
};

/// "PolyTest Tall": cell 1216 units (19 pixels at 16 per em), internal leading 3; no line gap.
const TALL: FontDef = FontDef {
    family: "PolyTest Tall",
    style: "Regular",
    weight_class: 400,
    bold: false,
    italic: false,
    fixed: false,
    win_asc: 896,
    win_desc: 320,
    hhea_asc: 896,
    hhea_desc: -320,
    line_gap: 0,
    typo: [896, -320, 0],
    avg_w: 640,
    a_adv: 0,
    underline_pos: -192,
    underline_thick: 128,
    strike_pos: 320,
    strike_size: 128,
    italic_angle_fixed: 0,
    opt: 0,
};

/// "PolyTest Mono": every advance 640 (10 pixels at 16 per em).
const MONO: FontDef = FontDef { family: "PolyTest Mono", fixed: true, avg_w: 640, ..SANS };

/// The Bold face of the "PolyTest Sans" family: 'A' is 13 pixels wide.
const SANS_BOLD: FontDef = FontDef { style: "Bold", weight_class: 700, bold: true, a_adv: 832, ..SANS };

/// The Italic face of the "PolyTest Sans" family: 'A' is 14 pixels wide.
const SANS_ITALIC: FontDef = FontDef { style: "Italic", italic: true, a_adv: 896, italic_angle_fixed: -10 << 16, ..SANS };

/// Big-endian writer over a byte buffer.
struct Wr<'a> {
    b: &'a mut [u8],
    n: usize,
}
impl Wr<'_> {
    fn u8(&mut self, v: u8) {
        self.b[self.n] = v;
        self.n += 1;
    }
    fn u16(&mut self, v: u16) {
        self.u8((v >> 8) as u8);
        self.u8(v as u8);
    }
    fn i16(&mut self, v: i16) {
        self.u16(v as u16);
    }
    fn u32(&mut self, v: u32) {
        self.u16((v >> 16) as u16);
        self.u16(v as u16);
    }
    fn bytes(&mut self, v: &[u8]) {
        for &c in v {
            self.u8(c);
        }
    }
    fn pad4(&mut self) {
        while self.n % 4 != 0 {
            self.u8(0);
        }
    }
}

fn glyph_adv(d: &FontDef, g: &G) -> u16 {
    if d.fixed {
        640
    } else if g.cp == 0x41 && d.a_adv != 0 {
        d.a_adv
    } else {
        g.adv
    }
}

/// builder options (probing what GDI accepts)
const O_OS2_V1: u32 = 1;
const O_REAL_MAXP: u32 = 2;
const O_MAC_NAMES: u32 = 4;
const O_SHORT_LOCA: u32 = 8;
const O_HINT_TABLES: u32 = 16;
const O_HEAD_FLAGS: u32 = 32;

/// Write a complete TrueType font for `d` into `buf`; its length.
fn build_font(d: &FontDef, buf: &mut [u8]) -> usize {
    let all: [&[u8; 4]; 13] = [b"OS/2", b"cmap", b"cvt ", b"fpgm", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"name", b"post", b"prep"];
    let mut tags: [&[u8; 4]; 13] = [b"    "; 13];
    let mut nt = 0usize;
    for tg in all.iter() {
        let hint = *tg == b"cvt " || *tg == b"fpgm" || *tg == b"prep";
        if hint && d.opt & O_HINT_TABLES == 0 {
            continue;
        }
        tags[nt] = tg;
        nt += 1;
    }
    let mut w = Wr { b: buf, n: 12 + 16 * nt };
    let mut ent = [(0usize, 0usize); 13];
    let mut locs = [0u32; 15];
    let ng = GLYPHS.len();
    let bmp = ng - 1; // code points of the cmap: all glyphs but .notdef
    let mut head_at = 0usize;
    for i in 0..nt {
        w.pad4();
        let start = w.n;
        match tags[i] {
            b"OS/2" => {
                w.u16(if d.opt & O_OS2_V1 != 0 { 1 } else { 4 });
                w.i16(d.avg_w);
                w.u16(d.weight_class);
                w.u16(5);
                w.u16(0);
                for v in [650i16, 600, 0, 75, 650, 600, 0, 350] {
                    w.i16(v);
                }
                w.i16(d.strike_size);
                w.i16(d.strike_pos);
                w.i16(0);
                let mut pan = [0u8; 10];
                pan[0] = 2;
                pan[3] = if d.fixed { 9 } else { 3 };
                w.bytes(&pan);
                w.u32(1);
                w.u32(0);
                w.u32(0);
                w.u32(0);
                w.bytes(b"POLY");
                let mut sel = 0u16;
                if d.italic {
                    sel |= 1;
                }
                if d.bold {
                    sel |= 1 << 5;
                }
                if !d.italic && !d.bold {
                    sel |= 1 << 6;
                }
                w.u16(sel);
                w.u16(0x20);
                w.u16(0xFFFF);
                w.i16(d.typo[0]);
                w.i16(d.typo[1]);
                w.i16(d.typo[2]);
                w.u16(d.win_asc);
                w.u16(d.win_desc);
                w.u32(1);
                w.u32(0);
                if d.opt & O_OS2_V1 == 0 {
                    w.i16(512);
                    w.i16(768);
                    w.u16(0);
                    w.u16(0x20);
                    w.u16(0);
                }
            }
            b"cmap" => {
                // format 4, platform 3 encoding 1; one segment per code point
                w.u16(0);
                w.u16(1);
                w.u16(3);
                w.u16(1);
                w.u32(12);
                let segs = bmp + 1;
                w.u16(4);
                w.u16((16 + 8 * segs) as u16);
                w.u16(0);
                w.u16((segs * 2) as u16);
                let mut pow = 1usize;
                let mut log = 0u16;
                while pow * 2 <= segs {
                    pow *= 2;
                    log += 1;
                }
                w.u16((pow * 2) as u16);
                w.u16(log);
                w.u16((segs * 2 - pow * 2) as u16);
                for g in GLYPHS.iter().skip(1) {
                    w.u16(g.cp as u16);
                }
                w.u16(0xFFFF);
                w.u16(0);
                for g in GLYPHS.iter().skip(1) {
                    w.u16(g.cp as u16);
                }
                w.u16(0xFFFF);
                for (k, g) in GLYPHS.iter().enumerate().skip(1) {
                    w.u16((k as u16).wrapping_sub(g.cp as u16));
                }
                w.u16(1);
                for _ in 0..segs {
                    w.u16(0);
                }
            }
            b"cvt " => {
                w.u16(0);
            }
            b"fpgm" | b"prep" => {
                w.u8(0xB0); // PUSHB[1]
                w.u8(0);
                w.u8(0x21); // POP
                w.u8(0);
            }
            b"glyf" => {
                for (k, g) in GLYPHS.iter().enumerate() {
                    locs[k] = (w.n - start) as u32;
                    if g.c.is_empty() {
                        continue;
                    }
                    let (mut x0, mut y0, mut x1, mut y1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
                    for c in g.c {
                        for p in *c {
                            x0 = x0.min(p.0);
                            x1 = x1.max(p.0);
                            y0 = y0.min(p.1);
                            y1 = y1.max(p.1);
                        }
                    }
                    w.i16(g.c.len() as i16);
                    w.i16(x0);
                    w.i16(y0);
                    w.i16(x1);
                    w.i16(y1);
                    let mut end = 0i16;
                    for c in g.c {
                        end += c.len() as i16;
                        w.i16(end - 1);
                    }
                    w.u16(0);
                    for c in g.c {
                        for _ in *c {
                            w.u8(1);
                        }
                    }
                    let mut prev = 0i16;
                    for c in g.c {
                        for p in *c {
                            w.i16(p.0 - prev);
                            prev = p.0;
                        }
                    }
                    prev = 0;
                    for c in g.c {
                        for p in *c {
                            w.i16(p.1 - prev);
                            prev = p.1;
                        }
                    }
                    w.pad4();
                }
                locs[ng] = (w.n - start) as u32;
            }
            b"head" => {
                head_at = start;
                w.u32(0x0001_0000);
                w.u32(0x0001_0000);
                w.u32(0);
                w.u32(0x5F0F_3CF5);
                w.u16(if d.opt & O_HEAD_FLAGS != 0 { 0x000B } else { 3 });
                w.u16(1024);
                w.u32(0);
                w.u32(0xD000_0000);
                w.u32(0);
                w.u32(0xD000_0000);
                w.i16(0);
                w.i16(-256);
                w.i16(1024);
                w.i16(768);
                w.u16(u16::from(d.bold) | (u16::from(d.italic) << 1));
                w.u16(8);
                w.i16(2);
                w.i16(if d.opt & O_SHORT_LOCA != 0 { 0 } else { 1 });
                w.i16(0);
            }
            b"hhea" => {
                w.u32(0x0001_0000);
                w.i16(d.hhea_asc);
                w.i16(d.hhea_desc);
                w.i16(d.line_gap);
                w.u16(1024);
                w.i16(0);
                w.i16(0);
                w.i16(1024);
                w.i16(1);
                w.i16(0);
                w.i16(0);
                for _ in 0..4 {
                    w.i16(0);
                }
                w.i16(0);
                w.u16(ng as u16);
            }
            b"hmtx" => {
                for g in GLYPHS.iter() {
                    w.u16(glyph_adv(d, g));
                    let mut x0 = i16::MAX;
                    for c in g.c {
                        for p in *c {
                            x0 = x0.min(p.0);
                        }
                    }
                    w.i16(if g.c.is_empty() { 0 } else { x0 });
                }
            }
            b"loca" => {
                for k in 0..=ng {
                    if d.opt & O_SHORT_LOCA != 0 {
                        w.u16((locs[k] / 2) as u16);
                    } else {
                        w.u32(locs[k]);
                    }
                }
            }
            b"maxp" => {
                w.u32(0x0001_0000);
                w.u16(ng as u16);
                w.u16(80);
                w.u16(3);
                w.u16(0);
                w.u16(0);
                if d.opt & O_REAL_MAXP != 0 {
                    w.u16(2);
                    w.u16(16);
                    w.u16(32);
                    w.u16(64);
                    w.u16(0);
                    w.u16(256);
                    w.u16(128);
                    w.u16(0);
                    w.u16(0);
                } else {
                    w.u16(1);
                    for _ in 0..8 {
                        w.u16(0);
                    }
                }
            }
            b"name" => {
                // family (1), subfamily (2), full name (4), PostScript name (6); platform 3, encoding 1, English (US)
                let mut full = Buf::<80>::new();
                let _ = write!(full, "{}", d.family);
                if d.style != "Regular" {
                    let _ = write!(full, " {}", d.style);
                }
                let mut ps = Buf::<80>::new();
                for ch in full.as_str().chars() {
                    if ch != ' ' {
                        let _ = ps.write_char(ch);
                    }
                }
                let uid = {
                    let mut b = Buf::<90>::new();
                    let _ = write!(b, "{} 1.0", full.as_str());
                    b
                };
                let recs: [(u16, &str); 7] = [
                    (0, "PolyTest fonts for Poly's text specs"),
                    (1, d.family),
                    (2, d.style),
                    (3, uid.as_str()),
                    (4, full.as_str()),
                    (5, "Version 1.0"),
                    (6, ps.as_str()),
                ];
                let nrec = recs.len();
                let str_off = 6 + 12 * nrec;
                w.u16(0);
                w.u16(nrec as u16);
                w.u16(str_off as u16);
                let mut off = 0u16;
                for (id, s) in recs.iter() {
                    let len = (s.encode_utf16().count() * 2) as u16;
                    w.u16(3);
                    w.u16(1);
                    w.u16(0x409);
                    w.u16(*id);
                    w.u16(len);
                    w.u16(off);
                    off += len;
                }
                for (_, s) in recs.iter() {
                    for u in s.encode_utf16() {
                        w.u16(u);
                    }
                }
            }
            _ => {
                // post 3.0
                w.u32(0x0003_0000);
                w.u32(d.italic_angle_fixed as u32);
                w.i16(d.underline_pos);
                w.i16(d.underline_thick);
                w.u32(u32::from(d.fixed));
                w.u32(0);
                w.u32(0);
                w.u32(0);
                w.u32(0);
            }
        }
        ent[i] = (start, w.n - start);
    }
    w.pad4();
    let total = w.n;
    w.n = 0;
    let mut pow = 1usize;
    let mut log = 0u16;
    while pow * 2 <= nt {
        pow *= 2;
        log += 1;
    }
    w.u32(0x0001_0000);
    w.u16(nt as u16);
    w.u16((pow * 16) as u16);
    w.u16(log);
    w.u16((nt * 16 - pow * 16) as u16);
    for i in 0..nt {
        let (o, l) = ent[i];
        let mut sum = 0u32;
        let mut k = o;
        while k < o + l {
            let mut word = 0u32;
            for j in 0..4 {
                word = (word << 8) | u32::from(if k + j < o + l { w.b[k + j] } else { 0 });
            }
            sum = sum.wrapping_add(word);
            k += 4;
        }
        w.bytes(tags[i]);
        w.u32(sum);
        w.u32(o as u32);
        w.u32(l as u32);
    }
    let mut file_sum = 0u32;
    let mut k = 0;
    while k < total {
        file_sum = file_sum.wrapping_add(u32::from_be_bytes([w.b[k], w.b[k + 1], w.b[k + 2], w.b[k + 3]]));
        k += 4;
    }
    let adj = 0xB1B0_AFBAu32.wrapping_sub(file_sum);
    let head = head_at + 8;
    w.b[head..head + 4].copy_from_slice(&adj.to_be_bytes());
    total
}

const FONT_SLOT_BYTES: usize = 4096;
static FONTS: Global<[[u8; FONT_SLOT_BYTES]; 8]> = Global::new([[0; FONT_SLOT_BYTES]; 8]);

/// Build `d` into slot `slot`; the file bytes.
fn font_file(d: &FontDef, slot: usize) -> &'static [u8] {
    let b = &mut FONTS.get()[slot];
    let n = build_font(d, b);
    &b[..n]
}

/// Make the font of `d` available with AddFontMemResourceEx; the handle and the font count.
fn add_mem(d: &FontDef, slot: usize) -> (HANDLE, u32) {
    let f = font_file(d, slot);
    let mut count = 0xAAAAu32;
    // SAFETY: the buffer lives in a static; count is writable.
    let h = unsafe { AddFontMemResourceEx(f.as_ptr() as *const c_void, f.len() as u32, core::ptr::null_mut(), &mut count) };
    (h, count)
}

/// Install SANS, TALL and MONO privately; they stay for the whole run.
fn setup_fonts() {
    add_mem(&SANS, 0);
    add_mem(&TALL, 1);
    add_mem(&MONO, 2);
}

/// A NUL-terminated copy of `s` as UTF-16 in a fixed array.
fn wchars(s: &str) -> [u16; 64] {
    let mut a = [0u16; 64];
    wide(s, &mut a);
    a
}

/// A LOGFONTW with the given height, quality and face name.
fn logfont(face: &str, height: i32, quality: u8) -> LOGFONTW {
    let mut lf: LOGFONTW = zeroed();
    lf.height = height;
    lf.weight = FW_NORMAL;
    lf.char_set = DEFAULT_CHARSET;
    lf.quality = quality;
    wide(face, &mut lf.face);
    lf
}

/// CreateFontIndirectW of a LOGFONTW with the face, height (negative: em) and quality.
fn mkfont(face: &str, height: i32, quality: u8) -> HFONT {
    let lf = logfont(face, height, quality);
    // SAFETY: a filled LOGFONTW.
    unsafe { CreateFontIndirectW(&lf) }
}

/// Select `f` into `dc`; the previous object.
fn sel(dc: HDC, f: HFONT) -> HGDIOBJ {
    // SAFETY: a DC and a font of ours.
    unsafe { SelectObject(dc, f) }
}

fn tm_of(dc: HDC) -> TEXTMETRICW {
    let mut tm: TEXTMETRICW = zeroed();
    // SAFETY: a DC of ours and a writable struct.
    unsafe { GetTextMetricsW(dc, &mut tm) };
    tm
}

/// The extent of the NUL-less units `s` in the font of `dc`.
fn ext(dc: HDC, s: &[u16]) -> (i32, i32) {
    let mut z = SIZE { cx: -1, cy: -1 };
    // SAFETY: a DC of ours; s is valid for its length.
    unsafe { GetTextExtentPoint32W(dc, s.as_ptr(), s.len() as i32, &mut z) };
    (z.cx, z.cy)
}

/// The extent of an ASCII string in the font of `dc`.
fn exta(dc: HDC, s: &str) -> (i32, i32) {
    let mut a = [0u16; 64];
    let n = wide(s, &mut a);
    ext(dc, &a[..n])
}

/// The face name GetTextFaceW reports for the font of `dc`.
fn face_of(dc: HDC) -> ([u16; 80], i32) {
    let mut b = [0u16; 80];
    // SAFETY: a DC of ours; b holds 80 units.
    let n = unsafe { GetTextFaceW(dc, 80, b.as_mut_ptr()) };
    (b, n)
}

/// Print every field of a TEXTMETRICW on one line.
fn say_tm(t: &mut Test, label: &str, m: &TEXTMETRICW) {
    say!(
        t,
        "{label}: height={} ascent={} descent={} intlead={} extlead={} avew={} maxw={} weight={} overhang={} aspect={}x{} first={} last={} default={} break={} italic={} underlined={} struck={} pitchfamily=0x{:02x} charset={}",
        m.height,
        m.ascent,
        m.descent,
        m.internal_leading,
        m.external_leading,
        m.ave_char_width,
        m.max_char_width,
        m.weight,
        m.overhang,
        m.digitized_aspect_x,
        m.digitized_aspect_y,
        m.first_char,
        m.last_char,
        m.default_char,
        m.break_char,
        m.italic,
        m.underlined,
        m.struck_out,
        m.pitch_and_family,
        m.char_set
    );
}

/// A path (UTF-16, NUL-terminated) for a file beside the executable.
struct DosPath {
    buf: [u16; 640],
    len: usize,
}
impl DosPath {
    fn ptr(&self) -> *const u16 {
        self.buf.as_ptr()
    }
}

fn scratch_path(name: &[u16]) -> DosPath {
    let mut d = DosPath { buf: [0; 640], len: 0 };
    if let Some(p) = path_beside_image(name) {
        let u = p.units();
        let skip = if u.len() > 4 && u[0] == 0x5c && u[1] == 0x3f && u[2] == 0x3f && u[3] == 0x5c { 4 } else { 0 };
        for &c in &u[skip..] {
            if d.len < 638 {
                d.buf[d.len] = c;
                d.len += 1;
            }
        }
    }
    d
}

/// Write `data` to the file `name` beside the executable; its path.
fn write_scratch(name: &[u16], data: &[u8]) -> DosPath {
    let d = scratch_path(name);
    // SAFETY: the path is NUL-terminated; the buffer outlives the calls.
    unsafe {
        let h = CreateFileW(d.ptr(), 0x4000_0000, 0, core::ptr::null_mut(), 2, 0x80, core::ptr::null_mut());
        if h as isize != -1 {
            let mut done = 0u32;
            WriteFile(h, data.as_ptr(), data.len() as u32, &mut done, core::ptr::null_mut());
            CloseHandle(h);
        }
    }
    d
}

fn delete_scratch(name: &[u16]) {
    let d = scratch_path(name);
    // SAFETY: the path is NUL-terminated.
    unsafe { DeleteFileW(d.ptr()) };
}


// ---- drawing helpers --------------------------------------------------------

/// text colour (white), background colour (dark blue) and the surface colour (grey) of the drawing tests
const C_FG: COLORREF = 0x00FF_FFFF;
const C_BK: COLORREF = 0x0080_0000;
const C_SURF: COLORREF = 0x0080_8080;

fn v_fg() -> u32 {
    dibv(C_FG)
}
fn v_bk() -> u32 {
    dibv(C_BK)
}
fn v_surf() -> u32 {
    dibv(C_SURF)
}

/// Fill the surface with the surface colour, select `f`, set the colours, the background mode and the alignment.
fn prep(s: &Surf, f: HFONT, mode: i32, align: u32) -> HGDIOBJ {
    s.clear(v_surf());
    // SAFETY: GDI calls on our DC.
    unsafe {
        SetTextColor(s.dc, C_FG);
        SetBkColor(s.dc, C_BK);
        SetBkMode(s.dc, mode);
        SetTextAlign(s.dc, align);
        MoveToEx(s.dc, 0, 0, core::ptr::null_mut());
    }
    sel(s.dc, f)
}

/// (text colour pixels, background colour pixels, other pixels that are not the surface colour) in `r`.
fn counts(s: &Surf, r: RECT) -> (u32, u32, u32) {
    // SAFETY: flushes batched GDI calls.
    unsafe { GdiFlush() };
    let (mut f, mut b, mut o) = (0, 0, 0);
    for y in r.top.max(0)..r.bottom.min(s.h) {
        for x in r.left.max(0)..r.right.min(s.w) {
            let v = s.px(x, y);
            if v == v_fg() {
                f += 1;
            } else if v == v_bk() {
                b += 1;
            } else if v != v_surf() {
                o += 1;
            }
        }
    }
    (f, b, o)
}

fn whole(s: &Surf) -> RECT {
    RECT { left: 0, top: 0, right: s.w, bottom: s.h }
}

/// bbox of pixels that differ from the surface colour, and the number of them
fn changed(s: &Surf) -> (i32, i32, i32, i32, u32) {
    s.ink(v_surf())
}

/// bbox of background-colour pixels
fn bk_box(s: &Surf) -> (i32, i32, i32, i32, u32) {
    // SAFETY: flushes batched GDI calls.
    unsafe { GdiFlush() };
    let (mut l, mut tp, mut rr, mut b, mut n) = (i32::MAX, i32::MAX, -1, -1, 0u32);
    for y in 0..s.h {
        for x in 0..s.w {
            if s.px(x, y) == v_bk() {
                l = l.min(x);
                tp = tp.min(y);
                rr = rr.max(x + 1);
                b = b.max(y + 1);
                n += 1;
            }
        }
    }
    if n == 0 { (0, 0, 0, 0, 0) } else { (l, tp, rr, b, n) }
}

/// bbox of text-colour pixels
fn fg_box(s: &Surf) -> (i32, i32, i32, i32, u32) {
    // SAFETY: flushes batched GDI calls.
    unsafe { GdiFlush() };
    let (mut l, mut tp, mut rr, mut b, mut n) = (i32::MAX, i32::MAX, -1, -1, 0u32);
    for y in 0..s.h {
        for x in 0..s.w {
            if s.px(x, y) == v_fg() {
                l = l.min(x);
                tp = tp.min(y);
                rr = rr.max(x + 1);
                b = b.max(y + 1);
                n += 1;
            }
        }
    }
    if n == 0 { (0, 0, 0, 0, 0) } else { (l, tp, rr, b, n) }
}

/// rows of the surface: `.` surface colour, `#` text colour, `o` background colour, `+` other
fn rows3(s: &Surf, t: &mut Test, r: RECT) {
    // SAFETY: flushes batched GDI calls.
    unsafe { GdiFlush() };
    for y in r.top..r.bottom {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "  |");
        for x in r.left..r.right {
            let v = s.px(x, y);
            let _ = b.write_char(if v == v_surf() {
                '.'
            } else if v == v_fg() {
                '#'
            } else if v == v_bk() {
                'o'
            } else {
                '+'
            });
        }
        let _ = b.write_str("|");
        t.line(b.as_str());
    }
}

/// A copy of the surface's pixels.
fn copy_to(a: &Surf, b: &Surf) {
    for i in 0..(a.w * a.h) as usize {
        // SAFETY: both surfaces have the same size.
        unsafe { *b.bits.add(i) = *a.bits.add(i) };
    }
}

/// The text units of an ASCII or Unicode `&str` (without NUL) in a fixed array.
struct Txt {
    u: [u16; 128],
    n: usize,
}
fn txt(s: &str) -> Txt {
    let mut t = Txt { u: [0; 128], n: 0 };
    t.n = wide(s, &mut t.u);
    t
}
impl Txt {
    fn units(&self) -> &[u16] {
        &self.u[..self.n]
    }
}

poly_wtest::main!(test);


fn show_in(t: &mut Test, s: &Surf, face: &str, h: i32, label: &str, text: &str, flags: u32) {
    let f = mkfont(face, h, NONANTIALIASED_QUALITY);
    let mut tx = txt(text);
    let old = prep(s, f, TRANSPARENT, 0);
    let mut r = RECT { left: 4, top: 4, right: 100, bottom: 30 };
    // SAFETY: the buffer holds the text.
    let ret = unsafe { DrawTextW(s.dc, tx.u.as_mut_ptr(), tx.n as i32, &mut r, flags) };
    let (l, tp, rr, b, n) = changed(s);
    let mut rc = RECT { left: 4, top: 4, right: 100, bottom: 30 };
    prep(s, f, TRANSPARENT, 0);
    // SAFETY: as above.
    unsafe { DrawTextW(s.dc, tx.u.as_mut_ptr(), tx.n as i32, &mut rc, flags | DT_CALCRECT) };
    say!(t, "{label}: returned {ret}; ink {l},{tp},{rr},{b} ({n} px); DT_CALCRECT width {}", rc.right - rc.left);
    // redraw for the rows
    prep(s, f, TRANSPARENT, 0);
    let mut r3 = RECT { left: 4, top: 4, right: 100, bottom: 30 };
    // SAFETY: as above.
    unsafe { DrawTextW(s.dc, tx.u.as_mut_ptr(), tx.n as i32, &mut r3, flags) };
    rows3(s, t, RECT { left: 2, top: 2, right: 42, bottom: 34 });
    sel(s.dc, old);
    // SAFETY: our font.
    unsafe { DeleteObject(f) };
}

fn show(t: &mut Test, s: &Surf, label: &str, text: &str, flags: u32) {
    show_in(t, s, "PolyTest Sans", -16, label, text, flags);
}

fn test(t: &mut Test) {
    setup_fonts();
    let s = Surf::new(110, 60);
    show(t, &s, "A&B default", "A&B", 0);
    show(t, &s, "A&B DT_NOPREFIX", "A&B", DT_NOPREFIX);
    show(t, &s, "A&B DT_HIDEPREFIX", "A&B", DT_HIDEPREFIX);
    show(t, &s, "A&B DT_PREFIXONLY", "A&B", DT_PREFIXONLY);
    show(t, &s, "&Ai default", "&Ai", 0);
    show(t, &s, "A&&B default", "A&&B", 0);
    show(t, &s, "A&&B DT_PREFIXONLY", "A&&B", DT_PREFIXONLY);
    show(t, &s, "AB& default", "AB&", 0);
    show(t, &s, "AB&g default", "AB&g", 0);
    show(t, &s, "A&B&i two marks", "A&B&i", 0);
    show(t, &s, "&&&A", "&&&A", 0);
    show(t, &s, "& alone", "&", 0);
    show(t, &s, "&g descender letter, DT_PREFIXONLY", "&g", DT_PREFIXONLY);
    show(t, &s, "A&B DT_SINGLELINE", "A&B", DT_SINGLELINE);
    // the underline's place in fonts of other proportions
    show_in(t, &s, "PolyTest Tall", -16, "Tall -16: A&B default", "A&B", 0);
    show_in(t, &s, "PolyTest Sans", -24, "Sans -24: A&B default", "A&B", 0);
    show_in(t, &s, "PolyTest Sans", -12, "Sans -12: A&B default", "A&B", 0);
    show_in(t, &s, "PolyTest Mono", -16, "Mono -16: A&B default", "A&B", 0);
    s.free();
}
