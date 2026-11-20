//! GetOutlineTextMetricsA: buffer sizes (NULL buffer asks for the size, too
//! small a buffer fails, the exact size works), a font that is not an outline
//! font (the stock bitmap fonts) gives 0, NULL and invalid device contexts,
//! the last error, and that the strings are laid out after the fixed part.
//! (The stock bitmap fonts are not used: what they report depends on the
//! installed fonts.)
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HFONT = *mut c_void;
type BOOL = i32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// A global the window procedures and the test share (one thread at a
/// time unless a test says otherwise).
struct Global<T>(UnsafeCell<T>);

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
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

/// wingdi.h (the ANSI layout, 4-byte aligned fields of the structure)
#[repr(C)]
#[derive(Clone, Copy)]
struct PANOSE {
    b: [u8; 10],
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct TEXTMETRICA {
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
    first_char: u8,
    last_char: u8,
    default_char: u8,
    break_char: u8,
    italic: u8,
    underlined: u8,
    struck_out: u8,
    pitch_and_family: u8,
    char_set: u8,
}

/// wingdi.h, OUTLINETEXTMETRICA (x64 layout: four-byte aligned, the string
/// members are offsets (PSTR is a pointer-sized member holding an offset))
#[repr(C)]
#[derive(Clone, Copy)]
struct OUTLINETEXTMETRICA {
    size: u32,
    tm: TEXTMETRICA,
    filler: u8,
    panose: PANOSE,
    fs_selection: u32,
    fs_type: u32,
    char_slope_rise: i32,
    char_slope_run: i32,
    italic_angle: i32,
    em_square: u32,
    ascent: i32,
    descent: i32,
    line_gap: u32,
    cap_em_height: u32,
    x_height: u32,
    font_box: RECT,
    mac_ascent: i32,
    mac_descent: i32,
    mac_line_gap: u32,
    min_ppem: u32,
    subscript_size: POINT,
    subscript_offset: POINT,
    superscript_size: POINT,
    superscript_offset: POINT,
    strikeout_size: u32,
    strikeout_position: i32,
    underscore_size: i32,
    underscore_position: i32,
    family_name: usize,
    face_name: usize,
    style_name: usize,
    full_name: usize,
}

/// wingdi.h LOGFONTW
#[repr(C)]
#[derive(Clone, Copy)]
struct LOGFONTW5 {
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

const FW_NORMAL5: i32 = 400;
const DEFAULT_CHARSET5: u8 = 1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn GetOutlineTextMetricsA(dc: HDC, cb: u32, otm: *mut OUTLINETEXTMETRICA) -> u32;
    fn AddFontMemResourceEx(data: *const c_void, n: u32, reserved: *mut c_void, count: *mut u32) -> HANDLE;
    fn CreateFontIndirectW(lf: *const LOGFONTW5) -> HFONT;
}

// ---- a test font (generated by lib/mkfont.py) ----

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

static GLYPHS: [G; 70] = [
    G { cp: 0, adv: 640, c: &[&[(64, 0), (64, 768), (576, 768), (576, 0)], &[(192, 128), (448, 128), (448, 640), (192, 640)]] },
    G { cp: 0x20, adv: 384, c: &[] },
    G { cp: 0x26, adv: 576, c: &[r!(64, 0, 512, 640)] },
    G { cp: 0x2d, adv: 576, c: &[r!(64, 0, 512, 640)] },
    G { cp: 0x2e, adv: 576, c: &[r!(64, 0, 512, 640)] },
    G { cp: 0x30, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x31, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x32, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x33, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x34, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x35, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x36, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x37, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x38, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x39, adv: 512, c: &[r!(64, 0, 448, 768)] },
    G { cp: 0x41, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x42, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x43, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x44, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x45, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x46, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x47, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x48, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x49, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x4a, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x4b, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x4c, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x4d, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x4e, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x4f, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x50, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x51, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x52, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x53, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x54, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x55, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x56, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x57, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x58, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x59, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x5a, adv: 704, c: &[r!(64, 0, 640, 768)] },
    G { cp: 0x5c, adv: 576, c: &[r!(64, 0, 512, 640)] },
    G { cp: 0x61, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x62, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x63, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x64, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x65, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x66, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x67, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x68, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x69, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x6a, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x6b, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x6c, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x6d, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x6e, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x6f, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x70, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x71, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x72, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x73, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x74, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x75, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x76, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x77, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x78, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x79, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0x7a, adv: 448, c: &[r!(64, 0, 384, 512)] },
    G { cp: 0xe9, adv: 576, c: &[r!(64, 0, 512, 640)] },
    G { cp: 0x20ac, adv: 576, c: &[r!(64, 0, 512, 640)] },
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
    /// advance of lower case and upper case letters in units (0: the glyph table)
    low_adv: u16,
    up_adv: u16,
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
    low_adv: 0,
    up_adv: 0,
    opt: 0,
};

/// "PolyDlg Mixed": lower case 448 units, upper case 704, digits 512 (average of the 52 letters 576 units).
const MIXED: FontDef = FontDef { family: "PolyDlg Mixed", low_adv: 448, up_adv: 704, avg_w: 500, ..SANS };

/// "PolyDlg Even": every glyph 640 units, fixed pitch flag set.
const EVEN: FontDef = FontDef { family: "PolyDlg Even", fixed: true, avg_w: 640, ..SANS };

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
    } else if g.cp >= 0x61 && g.cp <= 0x7A && d.low_adv != 0 {
        d.low_adv
    } else if g.cp >= 0x41 && g.cp <= 0x5A && d.up_adv != 0 {
        d.up_adv
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
    let mut locs = [0u32; 96];
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

const FONT_SLOT_BYTES: usize = 16384;
static FONTS: Global<[[u8; FONT_SLOT_BYTES]; 4]> = Global::new([[0; FONT_SLOT_BYTES]; 4]);

/// Build `d` into slot `slot`; the file bytes.
fn font_file(d: &FontDef, slot: usize) -> &'static [u8] {
    let b = &mut FONTS.get()[slot];
    let n = build_font(d, b);
    &b[..n]
}

/// Make the font of `d` available to this process (AddFontMemResourceEx); the handle and the font count.
fn add_mem(d: &FontDef, slot: usize) -> (HANDLE, u32) {
    let f = font_file(d, slot);
    let mut count = 0xAAAAu32;
    // SAFETY: the buffer lives in a static; count is writable.
    let h = unsafe { AddFontMemResourceEx(f.as_ptr() as *const c_void, f.len() as u32, core::ptr::null_mut(), &mut count) };
    (h, count)
}

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

/// A LOGFONTW5 for a face at a height (negative: em height in pixels).
fn lf5(face: &str, height: i32) -> LOGFONTW5 {
    let mut lf: LOGFONTW5 = zeroed();
    lf.height = height;
    lf.weight = FW_NORMAL5;
    lf.char_set = DEFAULT_CHARSET5;
    wide(face, &mut lf.face);
    lf
}

fn mkfont5(face: &str, height: i32) -> HFONT {
    let lf = lf5(face, height);
    // SAFETY: a filled LOGFONTW.
    unsafe { CreateFontIndirectW(&lf) }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    add_mem(&MIXED, 0);
    let f = mkfont5("PolyDlg Mixed", -16);
    // SAFETY: a memory DC of ours with the font selected; buffers sized as stated.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let old = SelectObject(dc, f);
        let need = GetOutlineTextMetricsA(dc, 0, core::ptr::null_mut());
        say!(t, "size asked for with cb 0 and a NULL buffer: {}", need);
        let mut raw = [0u8; 1024];
        for cb in [0u32, 1, 4, 8, 100, need - 1, need, need + 1, 1024] {
            raw.fill(0xA5);
            set_error(0xDEAD);
            let r = GetOutlineTextMetricsA(dc, cb, raw.as_mut_ptr() as *mut OUTLINETEXTMETRICA);
            let e = last_error();
            let touched = raw.iter().filter(|&&b| b != 0xA5).count();
            say!(t, "cb {cb}: result {r} err {} bytes written {touched}", if e == 0xDEAD { -1 } else { e as i64 });
        }
        set_error(0xDEAD);
        let r = GetOutlineTextMetricsA(dc, need, core::ptr::null_mut());
        say!(t, "exact size with a NULL buffer: {r} err {}", last_error());
        // the exact buffer: string area
        let mut exact = [0u8; 600];
        raw.fill(0);
        let r = GetOutlineTextMetricsA(dc, need, exact.as_mut_ptr() as *mut OUTLINETEXTMETRICA);
        let o = &*(exact.as_ptr() as *const OUTLINETEXTMETRICA);
        say!(t, "exact buffer: result {r}, otmSize {}", o.size);
        let hdr = core::mem::size_of::<OUTLINETEXTMETRICA>();
        let last = o.full_name;
        let mut e = last;
        while e < exact.len() && exact[e] != 0 {
            e += 1;
        }
        say!(t, "the last string ends at {} of {} (header {hdr}): the structure ends with the last string's NUL: {}", e + 1, need, e + 1 == need as usize);
        SelectObject(dc, old);
        DeleteDC(dc);
        // invalid DCs
        set_error(0xDEAD);
        let r = GetOutlineTextMetricsA(core::ptr::null_mut(), 0, core::ptr::null_mut());
        say!(t, "NULL DC: {r} err {}", if last_error() == 0xDEAD { -1 } else { last_error() as i64 });
        set_error(0xDEAD);
        let r = GetOutlineTextMetricsA(0x1234 as HDC, 1024, raw.as_mut_ptr() as *mut OUTLINETEXTMETRICA);
        say!(t, "made-up DC: {r} err {}", if last_error() == 0xDEAD { -1 } else { last_error() as i64 });
        set_error(0xDEAD);
        let r = GetOutlineTextMetricsA(dc, 1024, raw.as_mut_ptr() as *mut OUTLINETEXTMETRICA);
        say!(t, "deleted DC: {r} err {}", if last_error() == 0xDEAD { -1 } else { last_error() as i64 });
        // a screen DC with the font
        let sdc = GetDC(core::ptr::null_mut());
        let old3 = SelectObject(sdc, f);
        let r = GetOutlineTextMetricsA(sdc, 0, core::ptr::null_mut());
        say!(t, "screen DC with the test font: size asked for {r}, same as the memory DC: {}", r == need);
        SelectObject(sdc, old3);
        ReleaseDC(core::ptr::null_mut(), sdc);
    }
}
