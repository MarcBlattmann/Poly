//! GetGlyphOutlineW failures and buffer rules: a buffer too small, a size with no
//! buffer, a buffer with size 0, a NULL matrix, unknown formats, a bad DC, a
//! bitmap font, and the last error after each.
#![no_std]
#![no_main]
#![allow(dead_code, non_snake_case, non_camel_case_types, unused_imports, unused_macros, unused_mut, unused_variables, clippy::all)]

// ---------------------------------------------------------------------------
// Glyph and metric test prelude (spec-win-glyphs). The same items are in every
// test of the slice, so each file stands alone. Prototypes from Microsoft
// Learn; constants and structure layouts from the public SDK headers
// wingdi.h, winuser.h, windef.h, winbase.h and commctrl.h. The TrueType
// writer, the helpers and the glyph table are this test's own. The tests draw
// from two fonts only: a font they write themselves ("PolyGlyph ...", added
// with AddFontMemResourceEx) and DejaVu Sans 2.37 (a free font that Poly
// ships; read from C:\Windows\Fonts\DejaVuSans.ttf and added privately with
// AddFontResourceExW(FR_PRIVATE)). No Microsoft font is involved. Written for
// Poly; no code copied from anywhere.

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

/// wingdi.h
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
#[derive(Clone, Copy, Default)]
struct GLYPHMETRICS {
    black_box_x: u32,
    black_box_y: u32,
    origin_x: i32,
    origin_y: i32,
    cell_inc_x: i16,
    cell_inc_y: i16,
}

/// wingdi.h: four FIXED (16.16) values; the memory layout of a FIXED
/// (WORD fract, short value) is the signed 32-bit number times 65536.
#[repr(C)]
#[derive(Clone, Copy)]
struct MAT2 {
    m11: i32,
    m12: i32,
    m21: i32,
    m22: i32,
}
const IDENT: MAT2 = MAT2 { m11: 0x10000, m12: 0, m21: 0, m22: 0x10000 };

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ABC {
    a: i32,
    b: u32,
    c: i32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ABCFLOAT {
    a: f32,
    b: f32,
    c: f32,
}

const GGO_METRICS: u32 = 0;
const GGO_BITMAP: u32 = 1;
const GGO_NATIVE: u32 = 2;
const GGO_BEZIER: u32 = 3;
const GGO_GRAY2_BITMAP: u32 = 4;
const GGO_GRAY4_BITMAP: u32 = 5;
const GGO_GRAY8_BITMAP: u32 = 6;
const GGO_GLYPH_INDEX: u32 = 0x80;
const GGO_UNHINTED: u32 = 0x100;
const GDI_ERROR: u32 = 0xFFFF_FFFF;
const FR_PRIVATE: u32 = 0x10;
const FR_NOT_ENUM: u32 = 0x20;
const FW_NORMAL: i32 = 400;
const FW_BOLD: i32 = 700;
const DEFAULT_CHARSET: u8 = 1;
const NONANTIALIASED_QUALITY: u8 = 3;
const ANTIALIASED_QUALITY: u8 = 4;
const OUT_TT_ONLY_PRECIS: u8 = 7;
const SYSTEM_FONT: i32 = 13;
const OEM_FIXED_FONT: i32 = 10;
const ANSI_FIXED_FONT: i32 = 11;
const ANSI_VAR_FONT: i32 = 12;
const DEFAULT_GUI_FONT: i32 = 17;
const MM_TEXT: i32 = 1;
const MM_LOMETRIC: i32 = 2;
const LOGPIXELSX: i32 = 88;
const LOGPIXELSY: i32 = 90;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_MOD_NOT_FOUND: u32 = 126;
const GENERIC_READ_: u32 = 0x8000_0000;
const OPEN_EXISTING: u32 = 3;
const CREATE_ALWAYS: u32 = 2;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disposition: u32, flags: u32, template: HANDLE) -> HANDLE;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, done: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn GetFileSize(h: HANDLE, high: *mut u32) -> u32;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn CreateDirectoryW(name: *const u16, sa: *mut c_void) -> BOOL;
    fn RemoveDirectoryW(name: *const u16) -> BOOL;
    fn LoadLibraryW(name: *const u16) -> HANDLE;
    fn GetModuleHandleW(name: *const u16) -> HANDLE;
    fn GetProcAddress(m: HANDLE, name: *const u8) -> *mut c_void;
    fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, prot: u32) -> *mut c_void;
    fn VirtualFree(addr: *mut c_void, size: usize, ty: u32) -> BOOL;
    fn VirtualProtect(addr: *mut c_void, size: usize, prot: u32, old: *mut u32) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFontIndirectW(lf: *const LOGFONTW) -> HFONT;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetStockObject(i: i32) -> HGDIOBJ;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn SetMapMode(hdc: HDC, mode: i32) -> i32;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
    fn GetTextMetricsW(hdc: HDC, tm: *mut TEXTMETRICW) -> BOOL;
    fn GetTextFaceW(hdc: HDC, n: i32, out: *mut u16) -> i32;
    fn GetFontData(hdc: HDC, table: u32, offset: u32, buf: *mut c_void, n: u32) -> u32;
    fn GetGlyphOutlineW(hdc: HDC, ch: u32, format: u32, gm: *mut GLYPHMETRICS, n: u32, buf: *mut c_void, mat: *const MAT2) -> u32;
    fn GetOutlineTextMetricsW(hdc: HDC, n: u32, otm: *mut c_void) -> u32;
    fn GetOutlineTextMetricsA(hdc: HDC, n: u32, otm: *mut c_void) -> u32;
    fn GetCharABCWidthsW(hdc: HDC, first: u32, last: u32, abc: *mut ABC) -> BOOL;
    fn GetCharABCWidthsI(hdc: HDC, first: u32, count: u32, glyphs: *const u16, abc: *mut ABC) -> BOOL;
    fn GetCharABCWidthsFloatW(hdc: HDC, first: u32, last: u32, abc: *mut ABCFLOAT) -> BOOL;
    fn GetCharWidthI(hdc: HDC, first: u32, count: u32, glyphs: *const u16, widths: *mut i32) -> BOOL;
    fn GetCharWidthFloatW(hdc: HDC, first: u32, last: u32, widths: *mut f32) -> BOOL;
    fn GetCharWidth32W(hdc: HDC, first: u32, last: u32, out: *mut i32) -> BOOL;
    fn GetCharWidthW(hdc: HDC, first: u32, last: u32, out: *mut i32) -> BOOL;
    fn AddFontMemResourceEx(data: *const c_void, n: u32, reserved: *mut c_void, count: *mut u32) -> HANDLE;
    fn RemoveFontMemResourceEx(h: HANDLE) -> BOOL;
    fn AddFontResourceExW(name: *const u16, flags: u32, reserved: *mut c_void) -> i32;
    fn RemoveFontResourceExW(name: *const u16, flags: u32, reserved: *mut c_void) -> BOOL;
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

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<900>::new();
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

/// Prints UTF-16 units as text.
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

fn upto_nul(b: &[u16]) -> &[u16] {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    &b[..n]
}

fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn lei32(b: &[u8], o: usize) -> i32 {
    le32(b, o) as i32
}
fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// SHA-256 of `data`, the first 8 bytes as 16 hex digits.
struct H8([u8; 8]);
impl core::fmt::Display for H8 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}
fn h8(data: &[u8]) -> H8 {
    let mut s = Sha256::new();
    s.update(data);
    let d = s.finish();
    let mut o = [0u8; 8];
    o.copy_from_slice(&d[..8]);
    H8(o)
}

/// A table tag as the DWORD GetFontData takes it: the four characters as they
/// lie in memory (little-endian), so the file's "cmap" is the DWORD 0x70616D63.
fn tag(s: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*s)
}

// ---- the test font ---------------------------------------------------------
//
// A TrueType font the test writes: units per em 1024, every coordinate a
// multiple of 64 units, so at 16 pixels per em (lfHeight -16) every point lies
// on a whole pixel, and at 1024 pixels per em (lfHeight -1024) one pixel is one
// font unit.

#[derive(Clone, Copy)]
struct Pt(i16, i16, bool);
macro_rules! on {
    ($x:expr, $y:expr) => {
        Pt($x, $y, true)
    };
}
macro_rules! off {
    ($x:expr, $y:expr) => {
        Pt($x, $y, false)
    };
}
macro_rules! rect {
    ($x0:expr, $y0:expr, $x1:expr, $y1:expr) => {
        &[on!($x0, $y0), on!($x0, $y1), on!($x1, $y1), on!($x1, $y0)]
    };
}

enum Kind {
    /// no outline
    Empty,
    /// contours
    Simple(&'static [&'static [Pt]]),
    /// components: glyph index, dx, dy
    Composite(&'static [(u16, i16, i16)]),
}

struct G {
    cp: u32,
    adv: u16,
    k: Kind,
}

// glyph index: code point and what it is
//  0 .notdef (frame with a hole)  1 space  2 'A' box  3 'B' box  4 'H' (704 high)
//  5 'M' triangle  6 'O' curve  7 'Q' curve with consecutive off-curve points
//  8 'g' descender box  9 'i' two contours  10 'j' negative left bearing
// 11 'k' ink wider than the advance  12 'x' (448 high)  13 inner (no code point)
// 14 accent (no code point)  15 U+00E9 composite  16 U+20AC box  17 U+0301 advance 0
// 18 'W' wide box
static GLYPHS: [G; 19] = [
    G { cp: 0, adv: 640, k: Kind::Simple(&[&[on!(64, 0), on!(64, 768), on!(576, 768), on!(576, 0)], &[on!(192, 128), on!(448, 128), on!(448, 640), on!(192, 640)]]) },
    G { cp: 0x20, adv: 384, k: Kind::Empty },
    G { cp: 0x41, adv: 768, k: Kind::Simple(&[rect!(128, 0, 640, 768)]) },
    G { cp: 0x42, adv: 512, k: Kind::Simple(&[rect!(64, 0, 448, 512)]) },
    G { cp: 0x48, adv: 768, k: Kind::Simple(&[rect!(64, 0, 704, 704)]) },
    G { cp: 0x4D, adv: 768, k: Kind::Simple(&[&[on!(0, 0), on!(384, 768), on!(768, 0)]]) },
    G { cp: 0x4F, adv: 640, k: Kind::Simple(&[&[on!(64, 0), on!(64, 512), off!(64, 768), on!(320, 768), off!(576, 768), on!(576, 512), on!(576, 0)]]) },
    G { cp: 0x51, adv: 640, k: Kind::Simple(&[&[on!(64, 0), on!(64, 512), off!(64, 768), off!(576, 768), on!(576, 512), on!(576, 0)]]) },
    G { cp: 0x67, adv: 640, k: Kind::Simple(&[rect!(64, -256, 576, 384)]) },
    G { cp: 0x69, adv: 256, k: Kind::Simple(&[rect!(64, 0, 192, 512), rect!(64, 640, 192, 768)]) },
    G { cp: 0x6A, adv: 256, k: Kind::Simple(&[rect!(-128, -256, 128, 768)]) },
    G { cp: 0x6B, adv: 320, k: Kind::Simple(&[rect!(64, 0, 448, 512)]) },
    G { cp: 0x78, adv: 512, k: Kind::Simple(&[rect!(64, 0, 448, 448)]) },
    G { cp: 0, adv: 704, k: Kind::Simple(&[rect!(64, 0, 640, 512)]) },
    G { cp: 0, adv: 0, k: Kind::Simple(&[rect!(192, 576, 384, 768)]) },
    G { cp: 0xE9, adv: 704, k: Kind::Composite(&[(13, 0, 0), (14, 128, 128)]) },
    G { cp: 0x20AC, adv: 896, k: Kind::Simple(&[rect!(64, 0, 832, 768)]) },
    G { cp: 0x301, adv: 0, k: Kind::Simple(&[rect!(-192, 640, 0, 768)]) },
    G { cp: 0x57, adv: 1024, k: Kind::Simple(&[rect!(128, 0, 896, 768)]) },
];

/// The glyph index of a code point of the test font (0 when it has none).
fn pg_index(cp: u32) -> u16 {
    for (i, g) in GLYPHS.iter().enumerate() {
        if g.cp == cp && cp != 0 {
            return i as u16;
        }
    }
    0
}

/// What a font written by [`build_font`] looks like.
#[derive(Clone, Copy)]
struct FontDef {
    family: &'static str,
    style: &'static str,
    weight_class: u16,
    bold: bool,
    italic: bool,
    win_asc: u16,
    win_desc: u16,
    hhea_asc: i16,
    hhea_desc: i16,
    line_gap: i16,
    typo: [i16; 3],
    avg_w: i16,
    underline_pos: i16,
    underline_thick: i16,
    strike_pos: i16,
    strike_size: i16,
    /// OS/2 version (0, 1, 2 or 4: 2 and 4 carry sxHeight and sCapHeight)
    os2: u16,
    x_height: i16,
    cap_height: i16,
    /// head.unitsPerEm (the glyph table is in 1024ths; other values only relabel it)
    upem: u16,
    /// post.italicAngle (16.16)
    italic_angle: i32,
}

const PSANS: FontDef = FontDef {
    family: "PolyGlyph Sans",
    style: "Regular",
    weight_class: 400,
    bold: false,
    italic: false,
    win_asc: 768,
    win_desc: 256,
    hhea_asc: 768,
    hhea_desc: -256,
    line_gap: 128,
    typo: [768, -256, 128],
    avg_w: 512,
    underline_pos: -128,
    underline_thick: 64,
    strike_pos: 256,
    strike_size: 64,
    os2: 4,
    x_height: 512,
    cap_height: 768,
    upem: 1024,
    italic_angle: 0,
};

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

/// The bounding box of a glyph's own contours (composites: of their components).
fn glyph_box(i: usize) -> (i16, i16, i16, i16) {
    let (mut x0, mut y0, mut x1, mut y1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
    match &GLYPHS[i].k {
        Kind::Empty => return (0, 0, 0, 0),
        Kind::Simple(cs) => {
            for c in cs.iter() {
                for p in c.iter() {
                    x0 = x0.min(p.0);
                    x1 = x1.max(p.0);
                    y0 = y0.min(p.1);
                    y1 = y1.max(p.1);
                }
            }
        }
        Kind::Composite(cs) => {
            for &(g, dx, dy) in cs.iter() {
                let (a, b, c, d) = glyph_box(g as usize);
                x0 = x0.min(a + dx);
                y0 = y0.min(b + dy);
                x1 = x1.max(c + dx);
                y1 = y1.max(d + dy);
            }
        }
    }
    (x0, y0, x1, y1)
}

/// Write a complete TrueType font for `d` into `buf`; its length.
fn build_font(d: &FontDef, buf: &mut [u8]) -> usize {
    let tags: [&[u8; 4]; 9] = [b"OS/2", b"cmap", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"name"];
    let nt = tags.len() + 1; // and post
    let mut all: [&[u8; 4]; 10] = [b"    "; 10];
    for (i, t) in tags.iter().enumerate() {
        all[i] = t;
    }
    all[9] = b"post";
    let mut w = Wr { b: buf, n: 12 + 16 * nt };
    let mut ent = [(0usize, 0usize); 10];
    let mut locs = [0u32; 20];
    let ng = GLYPHS.len();
    let mut head_at = 0usize;
    for i in 0..nt {
        w.pad4();
        let start = w.n;
        match all[i] {
            b"OS/2" => {
                w.u16(d.os2);
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
                pan[3] = 3;
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
                if d.os2 >= 1 {
                    w.u32(1);
                    w.u32(0);
                }
                if d.os2 >= 2 {
                    w.i16(d.x_height);
                    w.i16(d.cap_height);
                    w.u16(0);
                    w.u16(0x20);
                    w.u16(0);
                }
            }
            b"cmap" => {
                // format 4, platform 3 encoding 1; one segment per code point, in code point order
                let mut cps = [0u16; 20];
                let mut order = [0u16; 20];
                let mut n = 0usize;
                for (k, g) in GLYPHS.iter().enumerate() {
                    if g.cp != 0 {
                        cps[n] = g.cp as u16;
                        order[n] = k as u16;
                        n += 1;
                    }
                }
                for a in 0..n {
                    for b in a + 1..n {
                        if cps[b] < cps[a] {
                            cps.swap(a, b);
                            order.swap(a, b);
                        }
                    }
                }
                w.u16(0);
                w.u16(1);
                w.u16(3);
                w.u16(1);
                w.u32(12);
                let segs = n + 1;
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
                for k in 0..n {
                    w.u16(cps[k]);
                }
                w.u16(0xFFFF);
                w.u16(0);
                for k in 0..n {
                    w.u16(cps[k]);
                }
                w.u16(0xFFFF);
                for k in 0..n {
                    w.u16(order[k].wrapping_sub(cps[k]));
                }
                w.u16(1);
                for _ in 0..segs {
                    w.u16(0);
                }
            }
            b"glyf" => {
                for (k, g) in GLYPHS.iter().enumerate() {
                    locs[k] = (w.n - start) as u32;
                    match &g.k {
                        Kind::Empty => continue,
                        Kind::Simple(cs) => {
                            let (x0, y0, x1, y1) = glyph_box(k);
                            w.i16(cs.len() as i16);
                            w.i16(x0);
                            w.i16(y0);
                            w.i16(x1);
                            w.i16(y1);
                            let mut end = 0i16;
                            for c in cs.iter() {
                                end += c.len() as i16;
                                w.i16(end - 1);
                            }
                            w.u16(0);
                            for c in cs.iter() {
                                for p in c.iter() {
                                    w.u8(if p.2 { 1 } else { 0 });
                                }
                            }
                            let mut prev = 0i16;
                            for c in cs.iter() {
                                for p in c.iter() {
                                    w.i16(p.0 - prev);
                                    prev = p.0;
                                }
                            }
                            prev = 0;
                            for c in cs.iter() {
                                for p in c.iter() {
                                    w.i16(p.1 - prev);
                                    prev = p.1;
                                }
                            }
                        }
                        Kind::Composite(cs) => {
                            let (x0, y0, x1, y1) = glyph_box(k);
                            w.i16(-1);
                            w.i16(x0);
                            w.i16(y0);
                            w.i16(x1);
                            w.i16(y1);
                            for (j, &(gi, dx, dy)) in cs.iter().enumerate() {
                                let more = if j + 1 < cs.len() { 0x20 } else { 0 };
                                w.u16(0x0003 | more); // ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES
                                w.u16(gi);
                                w.i16(dx);
                                w.i16(dy);
                            }
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
                w.u16(3);
                w.u16(d.upem);
                w.u32(0);
                w.u32(0xD000_0000);
                w.u32(0);
                w.u32(0xD000_0000);
                w.i16(-192);
                w.i16(-256);
                w.i16(1024);
                w.i16(768);
                w.u16(u16::from(d.bold) | (u16::from(d.italic) << 1));
                w.u16(8);
                w.i16(2);
                w.i16(1);
                w.i16(0);
            }
            b"hhea" => {
                w.u32(0x0001_0000);
                w.i16(d.hhea_asc);
                w.i16(d.hhea_desc);
                w.i16(d.line_gap);
                w.u16(1024);
                w.i16(-192);
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
                for (k, g) in GLYPHS.iter().enumerate() {
                    w.u16(g.adv);
                    let (x0, _, _, _) = glyph_box(k);
                    w.i16(x0);
                }
            }
            b"loca" => {
                for k in 0..=ng {
                    w.u32(locs[k]);
                }
            }
            b"maxp" => {
                w.u32(0x0001_0000);
                w.u16(ng as u16);
                w.u16(80);
                w.u16(3);
                w.u16(0);
                w.u16(0);
                w.u16(1);
                for _ in 0..8 {
                    w.u16(0);
                }
            }
            b"name" => {
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
                let mut uid = Buf::<90>::new();
                let _ = write!(uid, "{} 1.0", full.as_str());
                let recs: [(u16, &str); 7] = [
                    (0, "PolyGlyph test fonts for Poly's glyph specs"),
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
                w.u32(0x0003_0000);
                w.u32(d.italic_angle as u32);
                w.i16(d.underline_pos);
                w.i16(d.underline_thick);
                w.u32(0);
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
    // the table directory, tags in alphabetical order of the byte strings (OS/2 sorts before the lower-case tags)
    let mut order = [0usize; 10];
    for (i, o) in order.iter_mut().enumerate() {
        *o = i;
    }
    for a in 0..nt {
        for b in a + 1..nt {
            if all[order[b]] < all[order[a]] {
                order.swap(a, b);
            }
        }
    }
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
    for &i in order[..nt].iter() {
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
        w.bytes(all[i]);
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

const FONT_SLOT_BYTES: usize = 8192;
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

/// A LOGFONTW with the given height and face name (non-antialiased unless said).
fn logfont(face: &str, height: i32) -> LOGFONTW {
    let mut lf: LOGFONTW = zeroed();
    lf.height = height;
    lf.weight = FW_NORMAL;
    lf.char_set = DEFAULT_CHARSET;
    lf.quality = NONANTIALIASED_QUALITY;
    wide(face, &mut lf.face);
    lf
}

/// A memory DC with `face` at `height` selected (negative: pixels per em); the DC.
fn dc_with(face: &str, height: i32) -> HDC {
    let lf = logfont(face, height);
    // SAFETY: a filled LOGFONTW; the DC and the font are the test's own and live to the end of the run.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let f = CreateFontIndirectW(&lf);
        SelectObject(dc, f);
        dc
    }
}

/// Change the font of `dc`.
fn set_font(dc: HDC, face: &str, height: i32) {
    let lf = logfont(face, height);
    // SAFETY: as in dc_with.
    unsafe {
        let f = CreateFontIndirectW(&lf);
        SelectObject(dc, f);
    }
}

/// The face name selected into `dc`.
fn face_of(dc: HDC) -> ([u16; 80], i32) {
    let mut b = [0u16; 80];
    // SAFETY: a DC and a writable buffer.
    let n = unsafe { GetTextFaceW(dc, 80, b.as_mut_ptr()) };
    (b, n)
}

fn tm_of(dc: HDC) -> TEXTMETRICW {
    let mut tm: TEXTMETRICW = zeroed();
    // SAFETY: a DC and a writable struct.
    unsafe { GetTextMetricsW(dc, &mut tm) };
    tm
}

// ---- DejaVu ------------------------------------------------------------------

const DEJAVU_PATH: &[u16] = wz!("C:\\Windows\\Fonts\\DejaVuSans.ttf");

/// Add DejaVu Sans privately; the count AddFontResourceExW returned.
fn add_dejavu() -> i32 {
    // SAFETY: a NUL-terminated path.
    unsafe { AddFontResourceExW(DEJAVU_PATH.as_ptr(), FR_PRIVATE, core::ptr::null_mut()) }
}

const DV_BYTES: usize = 800_000;
static DV_FILE: Global<[u8; DV_BYTES]> = Global::new([0; DV_BYTES]);

/// The DejaVu Sans file read from disk; its length (0 when it cannot be read).
fn read_dejavu() -> usize {
    // SAFETY: a NUL-terminated path and a buffer of DV_BYTES.
    unsafe {
        let h = CreateFileW(DEJAVU_PATH.as_ptr(), GENERIC_READ_, 1, core::ptr::null_mut(), OPEN_EXISTING, 0x80, core::ptr::null_mut());
        if h as isize == -1 {
            return 0;
        }
        let n = GetFileSize(h, core::ptr::null_mut()) as usize;
        let mut done = 0u32;
        let ok = if n <= DV_BYTES { ReadFile(h, DV_FILE.get().as_mut_ptr(), n as u32, &mut done, core::ptr::null_mut()) } else { 0 };
        CloseHandle(h);
        if ok != 0 && done as usize == n {
            n
        } else {
            0
        }
    }
}

// ---- reading a font's tables with GetFontData -------------------------------------

/// The size of a table (0: the whole file) through GetFontData with no buffer.
fn fd_size(dc: HDC, table: u32) -> u32 {
    // SAFETY: a DC; no buffer.
    unsafe { GetFontData(dc, table, 0, core::ptr::null_mut(), 0) }
}

/// Copy a table into `out`; the byte count (GDI_ERROR on failure).
fn fd_get(dc: HDC, table: u32, offset: u32, out: &mut [u8]) -> u32 {
    // SAFETY: a DC and a writable buffer of out.len() bytes.
    unsafe { GetFontData(dc, table, offset, out.as_mut_ptr() as *mut c_void, out.len() as u32) }
}

/// The glyph index of `cp` found in the 'cmap' of the font selected into `dc`
/// (a format 4 or format 12 subtable for the Unicode BMP or full repertoire); 0 when not found.
fn cmap_glyph(dc: HDC, cp: u32) -> u16 {
    let mut head = [0u8; 4];
    if fd_get(dc, tag(b"cmap"), 0, &mut head) != 4 {
        return 0;
    }
    let n = be16(&head, 2) as usize;
    let mut recs = [0u8; 8 * 12];
    if n > 12 || fd_get(dc, tag(b"cmap"), 4, &mut recs[..8 * n]) != (8 * n) as u32 {
        return 0;
    }
    let mut best: Option<u32> = None;
    for i in 0..n {
        let plat = be16(&recs, 8 * i);
        let enc = be16(&recs, 8 * i + 2);
        if plat == 3 && (enc == 1 || enc == 10) {
            best = Some(be32(&recs, 8 * i + 4));
            if enc == 10 {
                break;
            }
        }
    }
    let Some(off) = best else { return 0 };
    let mut h = [0u8; 16];
    if fd_get(dc, tag(b"cmap"), off, &mut h) != 16 {
        return 0;
    }
    let fmt = be16(&h, 0);
    if fmt == 4 {
        let len = be16(&h, 2) as usize;
        let segx2 = be16(&h, 6) as usize;
        let segs = segx2 / 2;
        let mut sub = [0u8; 4096];
        if len > sub.len() || fd_get(dc, tag(b"cmap"), off, &mut sub[..len]) != len as u32 {
            return 0;
        }
        let ends = 14;
        let starts = ends + segx2 + 2;
        let deltas = starts + segx2;
        let ranges = deltas + segx2;
        for s in 0..segs {
            let e = be16(&sub, ends + 2 * s) as u32;
            let st = be16(&sub, starts + 2 * s) as u32;
            if cp >= st && cp <= e {
                let delta = be16(&sub, deltas + 2 * s);
                let ro = be16(&sub, ranges + 2 * s) as usize;
                if ro == 0 {
                    return (cp as u16).wrapping_add(delta);
                }
                let p = ranges + 2 * s + ro + 2 * (cp - st) as usize;
                if p + 2 > len {
                    return 0;
                }
                let g = be16(&sub, p);
                return if g == 0 { 0 } else { g.wrapping_add(delta) };
            }
        }
        0
    } else if fmt == 12 {
        let groups = be32(&h, 12) as usize;
        let mut g = [0u8; 12];
        for i in 0..groups {
            if fd_get(dc, tag(b"cmap"), off + 16 + 12 * i as u32, &mut g) != 12 {
                return 0;
            }
            let (s, e, gid) = (be32(&g, 0), be32(&g, 4), be32(&g, 8));
            if cp >= s && cp <= e {
                return (gid + cp - s) as u16;
            }
        }
        0
    } else {
        0
    }
}

// ---- printing glyph data ------------------------------------------------------

fn gm_line(gm: &GLYPHMETRICS) -> Buf<120> {
    let mut b = Buf::<120>::new();
    let _ = write!(b, "box {}x{} origin ({},{}) cell ({},{})", gm.black_box_x, gm.black_box_y, gm.origin_x, gm.origin_y, gm.cell_inc_x, gm.cell_inc_y);
    b
}

/// A 16.16 value as text: the integer, then `+n/65536` when it has a fraction.
struct Fx(i32);
impl core::fmt::Display for Fx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let v = self.0 >> 16;
        let fr = (self.0 & 0xFFFF) as u32;
        if fr == 0 {
            write!(f, "{}", v)
        } else {
            write!(f, "{}+{}/65536", v, fr)
        }
    }
}

/// The text of a GGO_NATIVE / GGO_BEZIER buffer: each contour as its start point and its curves.
fn native_text(buf: &[u8], n: usize, out: &mut Buf<900>) {
    let mut p = 0usize;
    while p + 16 <= n {
        let cb = le32(buf, p) as usize;
        let ty = le32(buf, p + 4);
        let (sx, sy) = (lei32(buf, p + 8), lei32(buf, p + 12));
        let _ = write!(out, "[cb {} type {} start ({},{})", cb, ty, Fx(sx), Fx(sy));
        let mut q = p + 16;
        while q + 4 <= p + cb {
            let wt = le16(buf, q);
            let cnt = le16(buf, q + 2) as usize;
            let _ = write!(out, " curve {} n {}:", wt, cnt);
            for i in 0..cnt {
                let o = q + 4 + 8 * i;
                let _ = write!(out, " ({},{})", Fx(lei32(buf, o)), Fx(lei32(buf, o + 4)));
            }
            q += 4 + 8 * cnt;
        }
        let _ = write!(out, "]");
        if cb == 0 {
            break;
        }
        p += cb;
    }
}

static GBUF: Global<[u8; 16384]> = Global::new([0; 16384]);

/// The outline call; (result, metrics, error) with the metrics pre-filled with a marker.
fn ggo(dc: HDC, ch: u32, format: u32, buf: Option<&mut [u8]>, mat: &MAT2) -> (u32, GLYPHMETRICS, u32) {
    let mut gm = GLYPHMETRICS { black_box_x: 0xAAAA, black_box_y: 0xAAAA, origin_x: 0x5555, origin_y: 0x5555, cell_inc_x: 0x2222, cell_inc_y: 0x2222 };
    let (p, n) = match buf {
        Some(b) => (b.as_mut_ptr() as *mut c_void, b.len() as u32),
        None => (core::ptr::null_mut(), 0),
    };
    // SAFETY: a DC, a writable metrics struct, a buffer of n bytes or none.
    unsafe {
        SetLastError(0xDEAD);
        let r = GetGlyphOutlineW(dc, ch, format, &mut gm, n, p, mat);
        (r, gm, GetLastError())
    }
}

fn err_text(e: u32) -> Buf<16> {
    let mut b = Buf::<16>::new();
    if e == 0xDEAD {
        let _ = write!(b, "unchanged");
    } else {
        let _ = write!(b, "{}", e);
    }
    b
}

/// Hex digits of a short byte run.
struct Hx<'a>(&'a [u8]);
impl core::fmt::Display for Hx<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

/// The name of a table tag value for printing.
struct Tg(u32);
impl core::fmt::Display for Tg {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0.to_le_bytes() {
            f.write_char(b as char)?;
        }
        Ok(())
    }
}

// ---- scratch files beside the executable --------------------------------------------

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

// ---- outline test helpers ---------------------------------------------------------

/// One line for a GetGlyphOutlineW result.
fn gline(t: &mut Test, label: &str, r: u32, gm: &GLYPHMETRICS, e: u32) {
    say!(t, "{}: result {} {} err {}", label, r as i32, gm_line(gm), err_text(e));
}

/// The label of a character for printing: printable ASCII as it is, the rest as U+hex.
struct Ch(u32);
impl core::fmt::Display for Ch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if (0x21..0x7F).contains(&self.0) {
            write!(f, "'{}'", self.0 as u8 as char)
        } else {
            write!(f, "U+{:04X}", self.0)
        }
    }
}

/// Metrics only (GGO_METRICS) of `ch` with the identity matrix.
fn metrics_line(t: &mut Test, dc: HDC, ch: u32, extra: u32) {
    let (r, gm, e) = ggo(dc, ch, GGO_METRICS | extra, None, &IDENT);
    let mut l = Buf::<40>::new();
    let _ = write!(l, "metrics {}", Ch(ch));
    mline(t, l.as_str(), r, &gm, e);
}

/// The size a format needs, then the data: its size, the metrics and the records (or a hash when long).
fn outline_line(t: &mut Test, dc: HDC, ch: u32, format: u32, mat: &MAT2, label: &str) {
    let (sz, gm0, e0) = ggo(dc, ch, format, None, mat);
    let buf = GBUF.get();
    if sz == GDI_ERROR || sz as usize > buf.len() {
        say!(t, "{} {}: size query {} {} err {}", label, Ch(ch), sz as i32, gm_line(&gm0), err_text(e0));
        return;
    }
    let n = sz as usize;
    buf[..n].fill(0xEE);
    let (r, gm, e) = ggo(dc, ch, format, Some(&mut buf[..n]), mat);
    let same = gm.black_box_x == gm0.black_box_x && gm.black_box_y == gm0.black_box_y && gm.origin_x == gm0.origin_x && gm.origin_y == gm0.origin_y && gm.cell_inc_x == gm0.cell_inc_x && gm.cell_inc_y == gm0.cell_inc_y;
    let mut text = Buf::<900>::new();
    if (format & 0xF) == GGO_NATIVE || (format & 0xF) == GGO_BEZIER {
        if n <= 400 {
            native_text(&buf[..n], n, &mut text);
        } else {
            let _ = write!(text, "sha256 {}", h8(&buf[..n]));
        }
    } else {
        let _ = write!(text, "sha256 {}", h8(&buf[..n]));
    }
    say!(t, "{} {}: size {} data result {} {} err {} metrics as in the query {} | {}", label, Ch(ch), sz, r as i32, gm_line(&gm), err_text(e), same, text);
}

/// `ok` or `GDI_ERROR` for a GGO_METRICS result (its number is a size that is not specified).
fn mres(r: u32) -> &'static str {
    if r == GDI_ERROR {
        "GDI_ERROR"
    } else {
        "ok"
    }
}

/// One line for a GGO_METRICS result: ok or GDI_ERROR, the metrics, the last error.
fn mline(t: &mut Test, label: &str, r: u32, gm: &GLYPHMETRICS, e: u32) {
    say!(t, "{}: {} {} err {}", label, mres(r), gm_line(gm), err_text(e));
}

/// A line for any format: GGO_METRICS (format & 0xF == 0) prints ok, the others the size.
fn fline(t: &mut Test, label: &str, format: u32, r: u32, gm: &GLYPHMETRICS, e: u32) {
    if format & 0xF == 0 {
        mline(t, label, r, gm, e);
    } else {
        gline(t, label, r, gm, e);
    }
}

// ---- OUTLINETEXTMETRICS --------------------------------------------------------

/// The text of the fixed part (232 bytes on x64) of an OUTLINETEXTMETRICW (`w`) or OUTLINETEXTMETRICA in `b`.
fn otm_fields(t: &mut Test, b: &[u8], w: bool) {
    // the ANSI TEXTMETRIC is 4 bytes shorter, so everything after it moves down by 4
    let s = if w { 0 } else { 4 };
    let u = |o: usize| le32(b, o - if o >= 64 { s } else { 0 });
    let i = |o: usize| lei32(b, o - if o >= 64 { s } else { 0 });
    say!(t, "  otmSize {}", le32(b, 0));
    say!(t, "  tm: height {} ascent {} descent {} internal leading {} external leading {} ave width {} max width {} weight {} overhang {} aspect {},{}", i(4), i(8), i(12), i(16), i(20), i(24), i(28), i(32), i(36), i(40), i(44));
    if w {
        say!(t, "  tm: first {:#x} last {:#x} default {:#x} break {:#x} italic {} underlined {} struck {} pitch/family {:#x} charset {}", le16(b, 48), le16(b, 50), le16(b, 52), le16(b, 54), b[56], b[57], b[58], b[59], b[60]);
    } else {
        say!(t, "  tm: first {:#x} last {:#x} default {:#x} break {:#x} italic {} underlined {} struck {} pitch/family {:#x} charset {}", b[48], b[49], b[50], b[51], b[52], b[53], b[54], b[55], b[56]);
    }
    say!(t, "  filler {} panose {:?} fsSelection {:#x} fsType {:#x}", b[64 - s], &b[65 - s..75 - s], u(76), u(80));
    say!(t, "  slope rise {} run {} italic angle {} em square {}", i(84), i(88), i(92), u(96));
    say!(t, "  ascent {} descent {} line gap {} cap em height {} x height {}", i(100), i(104), u(108), u(112), u(116));
    say!(t, "  font box ({}, {}, {}, {}) mac ascent {} descent {} line gap {} min ppem {}", i(120), i(124), i(128), i(132), i(136), i(140), u(144), u(148));
    say!(t, "  subscript size ({}, {}) offset ({}, {}) superscript size ({}, {}) offset ({}, {})", i(152), i(156), i(160), i(164), i(168), i(172), i(176), i(180));
    say!(t, "  strikeout size {} position {} underscore size {} position {}", u(184), i(188), i(192), i(196));
}

/// The raw OTM call into `buf`; (result, last error).
fn otm_call(dc: HDC, wide_: bool, n: u32, buf: Option<&mut [u8]>) -> (u32, u32) {
    let p = match buf {
        Some(b) => b.as_mut_ptr() as *mut c_void,
        None => core::ptr::null_mut(),
    };
    // SAFETY: a DC and a buffer of n bytes (or none).
    unsafe {
        SetLastError(0xDEAD);
        let r = if wide_ { GetOutlineTextMetricsW(dc, n, p) } else { GetOutlineTextMetricsA(dc, n, p) };
        (r, GetLastError())
    }
}

static OBUF: Global<[u8; 1024]> = Global::new([0; 1024]);

/// The names a call put behind the fixed part: (family, face, style, full) read from the offsets at 200..232.
fn otm_names(t: &mut Test, b: &[u8], total: usize, wide_: bool) {
    let mut offs = [0usize; 4];
    for k in 0..4 {
        offs[k] = le32(b, 200 + 8 * k) as usize;
    }
    say!(t, "  name offsets {:?} (high halves {:?})", offs, [le32(b, 204), le32(b, 212), le32(b, 220), le32(b, 228)]);
    for (k, label) in ["family", "face", "style", "full"].iter().enumerate() {
        let o = offs[k];
        if o >= total {
            say!(t, "  {} name offset beyond the data", label);
            continue;
        }
        if wide_ {
            let mut u = [0u16; 64];
            let mut n = 0;
            while o + 2 * n + 2 <= total && n < 63 {
                let c = le16(b, o + 2 * n);
                if c == 0 {
                    break;
                }
                u[n] = c;
                n += 1;
            }
            say!(t, "  {} name at {}: \"{}\" ({} units)", label, o, W16(&u[..n]), n);
        } else {
            let mut n = 0;
            while o + n < total && b[o + n] != 0 {
                n += 1;
            }
            say!(t, "  {} name at {}: \"{}\" ({} bytes)", label, o, core::str::from_utf8(&b[o..o + n]).unwrap_or("?"), n);
        }
    }
}

// ---- strings, paths, memory --------------------------------------------------------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn lstrcpyW(dst: *mut u16, src: *const u16) -> *mut u16;
    fn lstrcpyA(dst: *mut u8, src: *const u8) -> *mut u8;
    fn lstrcpynW(dst: *mut u16, src: *const u16, n: i32) -> *mut u16;
    fn lstrcpynA(dst: *mut u8, src: *const u8, n: i32) -> *mut u8;
    fn lstrcatW(dst: *mut u16, src: *const u16) -> *mut u16;
    fn lstrcatA(dst: *mut u8, src: *const u8) -> *mut u8;
    fn lstrlenW(s: *const u16) -> i32;
    fn lstrlenA(s: *const u8) -> i32;
    fn GetFileAttributesW(name: *const u16) -> u32;
}

#[link(name = "shlwapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn PathFileExistsW(path: *const u16) -> BOOL;
    fn PathFileExistsA(path: *const u8) -> BOOL;
}

const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE_: u32 = 0x8000;
const PAGE_RW: u32 = 4;
const PAGE_NA: u32 = 1;
const PAGE_RO: u32 = 2;

/// Two pages from VirtualAlloc, the second one made inaccessible (or read-only):
/// a buffer that ends at a page boundary. The first page's address.
fn two_pages(second: u32) -> *mut u8 {
    // SAFETY: a fresh allocation of two pages; VirtualProtect on its second page.
    unsafe {
        let p = VirtualAlloc(core::ptr::null_mut(), 0x2000, MEM_COMMIT_RESERVE, PAGE_RW) as *mut u8;
        if !p.is_null() {
            let mut old = 0u32;
            VirtualProtect(p.add(0x1000) as *mut c_void, 0x1000, second, &mut old);
        }
        p
    }
}

fn free_pages(p: *mut u8) {
    // SAFETY: p came from two_pages.
    unsafe { VirtualFree(p as *mut c_void, 0, MEM_RELEASE_) };
}

/// ASCII text of a NUL-terminated narrow buffer for printing (non-ASCII bytes as \xNN).
struct A8<'a>(&'a [u8]);
impl core::fmt::Display for A8<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for &b in self.0 {
            if b == 0 {
                break;
            }
            if (0x20..0x7F).contains(&b) && b != 0x5C {
                f.write_char(b as char)?;
            } else {
                write!(f, "\\x{:02x}", b)?;
            }
        }
        Ok(())
    }
}

/// A pointer for printing: NULL, the base itself or its offset from `base` in units of `unit`.
fn rel(p: *const u8, base: *const u8) -> Buf<24> {
    let mut b = Buf::<24>::new();
    if p.is_null() {
        let _ = write!(b, "NULL");
    } else {
        let _ = write!(b, "base{:+}", p as isize - base as isize);
    }
    b
}

// ---- path test helpers ---------------------------------------------------------------

/// The scratch directory `dir` beside the executable plus `name` (which starts with a backslash or is empty), NUL-terminated.
fn pf_path(dir: &str, name: &str) -> [u16; 700] {
    let mut dw = [0u16; 32];
    wide(dir, &mut dw);
    let base = scratch_path(&dw);
    let mut out = [0u16; 700];
    let mut n = 0;
    for &c in upto_nul(&base.buf[..base.len]) {
        out[n] = c;
        n += 1;
    }
    for u in name.encode_utf16() {
        out[n] = u;
        n += 1;
    }
    out
}

fn pf_file(path: &[u16], data: &[u8]) {
    // SAFETY: a NUL-terminated path and a buffer.
    unsafe {
        let h = CreateFileW(path.as_ptr(), 0x4000_0000, 0, core::ptr::null_mut(), 2, 0x80, core::ptr::null_mut());
        if h as isize != -1 {
            let mut done = 0u32;
            WriteFile(h, data.as_ptr(), data.len() as u32, &mut done, core::ptr::null_mut());
            CloseHandle(h);
        }
    }
}

/// Make the directory `dir` with a file, an empty file, a mixed-case file and a sub directory with a file in it.
fn pf_setup(dir: &str) {
    // SAFETY: NUL-terminated paths.
    unsafe {
        CreateDirectoryW(pf_path(dir, "").as_ptr(), core::ptr::null_mut());
        CreateDirectoryW(pf_path(dir, "\\subdir").as_ptr(), core::ptr::null_mut());
    }
    pf_file(&pf_path(dir, "\\file.txt"), b"hello");
    pf_file(&pf_path(dir, "\\Mixed Case.TXT"), b"x");
    pf_file(&pf_path(dir, "\\empty"), b"");
    pf_file(&pf_path(dir, "\\subdir\\inner.dat"), b"data");
}

fn pf_cleanup(dir: &str) {
    // SAFETY: NUL-terminated paths.
    unsafe {
        for n in ["\\subdir\\inner.dat", "\\file.txt", "\\Mixed Case.TXT", "\\empty"] {
            DeleteFileW(pf_path(dir, n).as_ptr());
        }
        RemoveDirectoryW(pf_path(dir, "\\subdir").as_ptr());
        RemoveDirectoryW(pf_path(dir, "").as_ptr());
    }
}

fn narrow(w: &[u16]) -> [u8; 700] {
    let mut o = [0u8; 700];
    let mut i = 0;
    while w[i] != 0 && i < 690 {
        o[i] = w[i] as u8;
        i += 1;
    }
    o
}

fn pfw(t: &mut Test, label: &str, path: &[u16]) {
    unsafe { SetLastError(0xDEAD) };
    let r = unsafe { PathFileExistsW(path.as_ptr()) };
    let e = unsafe { GetLastError() };
    say!(t, "{}: {} err {}", label, r, err_text(e));
}

fn pfa(t: &mut Test, label: &str, path: &[u16]) {
    let n = narrow(path);
    unsafe { SetLastError(0xDEAD) };
    let r = unsafe { PathFileExistsA(n.as_ptr()) };
    let e = unsafe { GetLastError() };
    say!(t, "{}: {} err {}", label, r, err_text(e));
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    add_mem(&PSANS, 0);
    let dc = dc_with("PolyGlyph Sans", -16);
    let buf = GBUF.get();
    let (sz, _, _) = ggo(dc, 0x4D, GGO_NATIVE, None, &IDENT);
    say!(t, "'M' native size {}", sz);
    // buffers of every size up to the needed one, and above
    for n in [0usize, 1, 4, (sz as usize) - 1, sz as usize, sz as usize + 1, sz as usize + 100] {
        buf[..n.max(1)].fill(0xEE);
        let (r, gm, e) = ggo(dc, 0x4D, GGO_NATIVE, Some(&mut buf[..n]), &IDENT);
        say!(t, "native 'M' buffer {} bytes (pointer given): result {} {} err {} first bytes {}", n, r as i32, gm_line(&gm), err_text(e), Hx(&buf[..8]));
    }
    // a size but no buffer
    {
        let mut gm = GLYPHMETRICS::default();
        unsafe { SetLastError(0xDEAD) };
        let r = unsafe { GetGlyphOutlineW(dc, 0x4D, GGO_NATIVE, &mut gm, 100, core::ptr::null_mut(), &IDENT) };
        let e = unsafe { GetLastError() };
        say!(t, "native 'M' size 100 but no buffer: result {} err {}", r as i32, err_text(e));
        unsafe { SetLastError(0xDEAD) };
        let r = unsafe { GetGlyphOutlineW(dc, 0x4D, GGO_METRICS, &mut gm, 100, core::ptr::null_mut(), &IDENT) };
        let e = unsafe { GetLastError() };
        say!(t, "metrics 'M' size 100 but no buffer: {} err {}", mres(r), err_text(e));
        // GGO_METRICS with a buffer: what is written
        for n in [16u32, 64, 200] {
            buf[..256].fill(0xEE);
            unsafe { SetLastError(0xDEAD) };
            let r = unsafe { GetGlyphOutlineW(dc, 0x4D, GGO_METRICS, &mut gm, n, buf.as_mut_ptr() as *mut c_void, &IDENT) };
            let e = unsafe { GetLastError() };
            let written = buf[..256].iter().filter(|&&b| b != 0xEE).count();
            say!(t, "metrics 'M' with a {} byte buffer: {} err {} bytes changed {} first bytes {}", n, mres(r), err_text(e), written, Hx(&buf[..16]));
        }
    }
    // formats
    for f in [4u32, 5, 6, 7, 8, 9, 0xF, 0x10, 0x100, 0x101, 0x102, 0x80, 0x82, 0x1000] {
        let (r, gm, e) = ggo(dc, 0x4D, f, None, &IDENT);
        let mut l = Buf::<60>::new();
        let _ = write!(l, "format {:#x} size query", f);
        fline(t, l.as_str(), f, r, &gm, e);
    }
    // GGO_UNHINTED with the formats
    for f in [GGO_NATIVE | GGO_UNHINTED, GGO_BEZIER | GGO_UNHINTED, GGO_METRICS | GGO_UNHINTED, GGO_GRAY8_BITMAP | GGO_UNHINTED] {
        let (r, gm, e) = ggo(dc, 0x4D, f, None, &IDENT);
        let mut l = Buf::<60>::new();
        let _ = write!(l, "format {:#x} size query", f);
        fline(t, l.as_str(), f, r, &gm, e);
    }
    // NULL matrix
    {
        let mut gm = GLYPHMETRICS::default();
        unsafe { SetLastError(0xDEAD) };
        let r = unsafe { GetGlyphOutlineW(dc, 0x4D, GGO_METRICS, &mut gm, 0, core::ptr::null_mut(), core::ptr::null()) };
        let e = unsafe { GetLastError() };
        say!(t, "NULL matrix: result {} err {}", r as i32, err_text(e));
    }
    // DCs
    for (label, d) in [("NULL dc", core::ptr::null_mut()), ("made-up dc", 0x1234usize as HDC)] {
        let (r, gm, e) = ggo(d, 0x4D, GGO_METRICS, None, &IDENT);
        mline(t, label, r, &gm, e);
    }
    let dead = dc_with("PolyGlyph Sans", -16);
    unsafe { DeleteDC(dead) };
    let (r, gm, e) = ggo(dead, 0x4D, GGO_METRICS, None, &IDENT);
    mline(t, "deleted dc", r, &gm, e);
    // bitmap stock fonts have no outlines
    let mdc = unsafe { CreateCompatibleDC(core::ptr::null_mut()) };
    for (label, id) in [("OEM_FIXED_FONT", OEM_FIXED_FONT), ("SYSTEM_FONT", SYSTEM_FONT)] {
        unsafe { SelectObject(mdc, GetStockObject(id)) };
        let (r, gm, e) = ggo(mdc, 0x41, GGO_NATIVE, None, &IDENT);
        say!(t, "stock {} native 'A' size query: {} err {}", label, r as i32, err_text(e));
        let (r, gm, e) = ggo(mdc, 0x41, GGO_METRICS, None, &IDENT);
        say!(t, "stock {} metrics 'A': {} err {}", label, if r == GDI_ERROR { "GDI_ERROR" } else { "ok" }, err_text(e));
    }
    // a character that is not in the font maps to .notdef; a code point beyond the BMP
    for ch in [0x7Au32, 0x10000, 0x1F600, 0xFFFF_FFFF] {
        let (r, gm, e) = ggo(dc, ch, GGO_METRICS, None, &IDENT);
        let mut l = Buf::<60>::new();
        let _ = write!(l, "metrics of {:#x}", ch);
        mline(t, l.as_str(), r, &gm, e);
    }
}
