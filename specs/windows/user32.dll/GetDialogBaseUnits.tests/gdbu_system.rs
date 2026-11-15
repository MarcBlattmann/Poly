//! GetDialogBaseUnits: the result (width in the low word, height in the high
//! word) is the average character width and the height of the system font;
//! it does not change between calls and it is what a dialog without its own
//! font uses for MapDialogRect.
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
type HINSTANCE = *mut c_void;
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HFONT = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
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

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SIZE {
    cx: i32,
    cy: i32,
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

const WM_GETFONT: u32 = 0x0031;
const SYSTEM_FONT: i32 = 13;
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

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

fn rect_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// The UTF-16 text in `buf` up to its first NUL, as ASCII (others as '?').
fn ascii_of(buf: &[u16], out: &mut Buf<300>) {
    for &c in buf {
        if c == 0 {
            break;
        }
        let _ = out.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
}

type DLGPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> isize>;
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
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn MapWindowPoints(from: HWND, to: HWND, pts: *mut POINT, n: u32) -> i32;
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn CreateDialogIndirectParamW(instance: HINSTANCE, template: *const c_void, parent: HWND, proc_: DLGPROC, param: isize) -> HWND;
    fn GetDlgItem(hwnd: HWND, id: i32) -> HWND;
    fn GetDialogBaseUnits() -> i32;
    fn MapDialogRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn GetTextExtentPoint32W(hdc: HDC, text: *const u16, n: i32, size: *mut SIZE) -> BOOL;
    fn GetTextMetricsW(hdc: HDC, tm: *mut TEXTMETRICW) -> BOOL;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
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

// ---- dialog unit helpers of these tests ----

unsafe extern "system" fn dlu_proc(_h: HWND, _m: u32, _w: WPARAM, _l: LPARAM) -> isize {
    0
}

fn w16(s: &str) -> [u16; 80] {
    let mut a = [0u16; 80];
    for (i, u) in s.encode_utf16().enumerate() {
        a[i] = u;
    }
    a
}

/// Make the two test fonts available to this process.
fn dlu_setup(t: &mut Test) {
    let (h, c) = add_mem(&MIXED, 0);
    say!(t, "AddFontMemResourceEx for PolyDlg Mixed: handle {} fonts {}", !h.is_null(), c);
    let (h, c) = add_mem(&EVEN, 1);
    say!(t, "AddFontMemResourceEx for PolyDlg Even: handle {} fonts {}", !h.is_null(), c);
}

/// A dialog made from an in-memory template (not shown).
fn mk(tpl: *const c_void) -> HWND {
    // SAFETY: a template that lives in a static.
    unsafe { CreateDialogIndirectParamW(instance(), tpl, core::ptr::null_mut(), Some(dlu_proc), 0) }
}

/// The base units MapDialogRect uses: the mapped (0, 0, 4, 8).
fn base_units(d: HWND) -> (i32, i32) {
    let mut r = RECT { left: 0, top: 0, right: 4, bottom: 8 };
    // SAFETY: r is writable.
    unsafe { MapDialogRect(d, &mut r) };
    (r.right, r.bottom)
}

fn logfont_of(f: HGDIOBJ) -> LOGFONTW5 {
    let mut lf: LOGFONTW5 = zeroed();
    // SAFETY: lf is a LOGFONTW.
    unsafe { GetObjectW(f, core::mem::size_of::<LOGFONTW5>() as i32, &mut lf as *mut LOGFONTW5 as *mut c_void) };
    lf
}

fn face_text(lf: &LOGFONTW5, out: &mut Buf<300>) {
    ascii_of(&lf.face, out);
}

/// Text metrics and the width of "a..zA..Z" of a font: (tmHeight, tmAveCharWidth, width of the 52 letters).
fn font_numbers(f: HGDIOBJ) -> (i32, i32, i32) {
    // SAFETY: a memory DC of ours, the font selected and restored.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let old = SelectObject(dc, f);
        let mut tm: TEXTMETRICW = zeroed();
        GetTextMetricsW(dc, &mut tm);
        let a = w16("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ");
        let mut sz = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(dc, a.as_ptr(), 52, &mut sz);
        SelectObject(dc, old);
        DeleteDC(dc);
        (tm.height, tm.ave_char_width, sz.cx)
    }
}

/// What a dialog's font is and what the base units are.
fn dlu_report(t: &mut Test, label: &str, d: HWND) {
    // SAFETY: a message to our dialog.
    let f = unsafe { SendMessageW(d, WM_GETFONT, 0, 0) } as HGDIOBJ;
    say!(t, "{label}: dialog font handle {}", if f.is_null() { "NULL" } else { "set" });
    let (bx, by) = base_units(d);
    if !f.is_null() {
        let lf = logfont_of(f);
        let mut fb = Buf::<300>::new();
        face_text(&lf, &mut fb);
        let (h, avg, w52) = font_numbers(f);
        say!(t, "  LOGFONT height {} weight {} charset {} face '{}'", lf.height, lf.weight, lf.char_set, fb.as_str());
        say!(t, "  tmHeight {h} tmAveCharWidth {avg} width of the 52 letters {w52}");
        let fx = (w52 / 26 + 1) / 2;
        say!(t, "  base units by the alphabet rule: x {fx} y {h}");
    }
    say!(t, "  MapDialogRect(0, 0, 4, 8) = ({bx}, {by})");
}

/// The rectangle of each child in client coordinates of the dialog.
fn dlu_kids(t: &mut Test, d: HWND) {
    // SAFETY: calls on our dialog's windows.
    unsafe {
        for id in [1001, 1002, 1003, 1004, 1005, 1006, 1] {
            let c = GetDlgItem(d, id);
            if c.is_null() {
                say!(t, "  child {id}: missing");
                continue;
            }
            let mut r = rect_of(c);
            let mut p = [POINT { x: r.left, y: r.top }, POINT { x: r.right, y: r.bottom }];
            MapWindowPoints(core::ptr::null_mut(), d, p.as_mut_ptr(), 2);
            r.left = p[0].x;
            r.top = p[0].y;
            r.right = p[1].x;
            r.bottom = p[1].y;
            say!(t, "  child {id}: client rect ({}, {}, {}, {}) size {} x {}", r.left, r.top, r.right, r.bottom, r.right - r.left, r.bottom - r.top);
        }
    }
}

/// The template rectangles (left, top, width, height) of the children, in dialog units.
const KID_DLU: [(i32, i32, i32, i32, i32); 7] = [
    (1001, 5, 3, 60, 10),
    (1002, 70, 20, 100, 44),
    (1003, 76, 32, 40, 10),
    (1004, 120, 32, 40, 10),
    (1005, 76, 46, 40, 10),
    (1006, 76, 70, 80, 12),
    (1, 120, 84, 50, 14),
];

/// Whether every child lies where MapDialogRect puts its template rectangle.
fn dlu_kids_match(t: &mut Test, label: &str, d: HWND) {
    let mut all = true;
    let mut bad = 0;
    // SAFETY: calls on our dialog's windows.
    unsafe {
        for (id, x, y, w, h) in KID_DLU {
            let mut m = RECT { left: x, top: y, right: x + w, bottom: y + h };
            MapDialogRect(d, &mut m);
            let c = GetDlgItem(d, id);
            let r = rect_of(c);
            let mut p = [POINT { x: r.left, y: r.top }, POINT { x: r.right, y: r.bottom }];
            MapWindowPoints(core::ptr::null_mut(), d, p.as_mut_ptr(), 2);
            if p[0].x != m.left || p[0].y != m.top || p[1].x != m.right || p[1].y != m.bottom {
                all = false;
                bad += 1;
            }
        }
    }
    say!(t, "{label}: every child lies where MapDialogRect puts its template rectangle: {all} ({bad} differ)");
}

#[repr(C, align(4))]
struct Aligned<const N: usize>([u8; N]);
static T_NOFONT: Aligned<314> = Aligned([128, 0, 200, 128, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 180, 0, 100, 0, 0, 0, 0, 0, 68, 0, 105, 0, 97, 0, 108, 0, 111, 0, 103, 0, 32, 0, 117, 0, 110, 0, 105, 0, 116, 0, 115, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 3, 0, 60, 0, 10, 0, 233, 3, 255, 255, 130, 0, 67, 0, 97, 0, 116, 0, 101, 0, 103, 0, 111, 0, 114, 0, 121, 0, 58, 0, 0, 0, 0, 0, 7, 0, 0, 80, 0, 0, 0, 0, 70, 0, 20, 0, 100, 0, 44, 0, 234, 3, 255, 255, 128, 0, 67, 0, 111, 0, 110, 0, 110, 0, 101, 0, 99, 0, 116, 0, 105, 0, 111, 0, 110, 0, 0, 0, 0, 0, 0, 0, 9, 0, 3, 80, 0, 0, 0, 0, 76, 0, 32, 0, 40, 0, 10, 0, 235, 3, 255, 255, 128, 0, 83, 0, 83, 0, 72, 0, 0, 0, 0, 0, 9, 0, 0, 80, 0, 0, 0, 0, 120, 0, 32, 0, 40, 0, 10, 0, 236, 3, 255, 255, 128, 0, 83, 0, 101, 0, 114, 0, 105, 0, 97, 0, 108, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 80, 0, 0, 0, 0, 76, 0, 46, 0, 40, 0, 10, 0, 237, 3, 255, 255, 128, 0, 79, 0, 116, 0, 104, 0, 101, 0, 114, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 76, 0, 70, 0, 80, 0, 12, 0, 238, 3, 255, 255, 129, 0, 104, 0, 111, 0, 115, 0, 116, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80, 0, 0, 0, 0, 120, 0, 84, 0, 50, 0, 14, 0, 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0]);
static T_MIXED8: Aligned<346> = Aligned([192, 0, 200, 128, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 180, 0, 100, 0, 0, 0, 0, 0, 68, 0, 105, 0, 97, 0, 108, 0, 111, 0, 103, 0, 32, 0, 117, 0, 110, 0, 105, 0, 116, 0, 115, 0, 0, 0, 8, 0, 80, 0, 111, 0, 108, 0, 121, 0, 68, 0, 108, 0, 103, 0, 32, 0, 77, 0, 105, 0, 120, 0, 101, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 3, 0, 60, 0, 10, 0, 233, 3, 255, 255, 130, 0, 67, 0, 97, 0, 116, 0, 101, 0, 103, 0, 111, 0, 114, 0, 121, 0, 58, 0, 0, 0, 0, 0, 7, 0, 0, 80, 0, 0, 0, 0, 70, 0, 20, 0, 100, 0, 44, 0, 234, 3, 255, 255, 128, 0, 67, 0, 111, 0, 110, 0, 110, 0, 101, 0, 99, 0, 116, 0, 105, 0, 111, 0, 110, 0, 0, 0, 0, 0, 0, 0, 9, 0, 3, 80, 0, 0, 0, 0, 76, 0, 32, 0, 40, 0, 10, 0, 235, 3, 255, 255, 128, 0, 83, 0, 83, 0, 72, 0, 0, 0, 0, 0, 9, 0, 0, 80, 0, 0, 0, 0, 120, 0, 32, 0, 40, 0, 10, 0, 236, 3, 255, 255, 128, 0, 83, 0, 101, 0, 114, 0, 105, 0, 97, 0, 108, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 80, 0, 0, 0, 0, 76, 0, 46, 0, 40, 0, 10, 0, 237, 3, 255, 255, 128, 0, 79, 0, 116, 0, 104, 0, 101, 0, 114, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 76, 0, 70, 0, 80, 0, 12, 0, 238, 3, 255, 255, 129, 0, 104, 0, 111, 0, 115, 0, 116, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80, 0, 0, 0, 0, 120, 0, 84, 0, 50, 0, 14, 0, 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0]);

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    dlu_setup(t);
    // SAFETY: GetDialogBaseUnits has no arguments.
    let gb = unsafe { GetDialogBaseUnits() };
    let (x, y) = (gb & 0xFFFF, (gb >> 16) & 0xFFFF);
    say!(t, "GetDialogBaseUnits = ({x}, {y}); high bits of the result are the height: {}", gb as u32 == (x as u32 | ((y as u32) << 16)));
    // SAFETY: GetDialogBaseUnits has no arguments.
    say!(t, "a second call gives the same result: {}", unsafe { GetDialogBaseUnits() } == gb);
    // SAFETY: a stock object.
    let sysf = unsafe { GetStockObject(SYSTEM_FONT) };
    let (h, avg, w52) = font_numbers(sysf);
    say!(t, "system font: tmHeight {h} tmAveCharWidth {avg} width of the 52 letters {w52}");
    say!(t, "x equals tmAveCharWidth {}; x equals (width of the 52 letters / 26 + 1) / 2: {}; y equals tmHeight {}", x == avg, x == (w52 / 26 + 1) / 2, y == h);
    let d = mk(&T_NOFONT as *const _ as *const c_void);
    let (bx, by) = base_units(d);
    say!(t, "a dialog without a font: MapDialogRect base units ({bx}, {by}); equal to GetDialogBaseUnits: {}", bx == x && by == y);
    // SAFETY: our dialog.
    unsafe { DestroyWindow(d) };
    // a dialog with a font does not change the result
    let e = mk(&T_MIXED8 as *const _ as *const c_void);
    // SAFETY: GetDialogBaseUnits has no arguments.
    say!(t, "after making a dialog with its own font the result is unchanged: {}", unsafe { GetDialogBaseUnits() } == gb);
    // SAFETY: our dialog.
    unsafe { DestroyWindow(e) };
    // from another thread
    static OTHER: Global<i32> = Global::new(0);
    unsafe extern "system" fn th(_: *mut c_void) -> u32 {
        // SAFETY: no arguments.
        *OTHER.get() = unsafe { GetDialogBaseUnits() };
        0
    }
    // SAFETY: a thread that is joined at once.
    unsafe {
        let h = CreateThread(core::ptr::null_mut(), 0, th, core::ptr::null_mut(), 0, core::ptr::null_mut());
        WaitForSingleObject(h, 10000);
        CloseHandle(h);
    }
    say!(t, "from another thread: {}", *OTHER.get() == gb);
}
