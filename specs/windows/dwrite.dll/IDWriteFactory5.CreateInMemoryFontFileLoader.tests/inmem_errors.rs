//! IDWriteInMemoryFontFileLoader::CreateInMemoryFontFileReference: data that
//! is not a font, empty data, and a reference used with the loader not
//! registered.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_snake_case, non_upper_case_globals, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers dwrite.h, dwrite_1.h, dwrite_2.h, dwrite_3.h and winerror.h
// (vtable layouts, constants, structure layouts); the font writer, the loaders
// and the helpers are this test's own.

use core::cell::UnsafeCell;
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

/// A global the test and the loader callbacks share (one thread).
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

/// One entry of the `name` table (platform 3, encoding 1).
struct NameRec {
    id: u16,
    lang: u16,
    text: &'static str,
}

/// What a font written by [`build_font`] looks like. The outlines are fixed
/// (eight glyphs: .notdef, space, a square, a triangle, a rounded shape, a
/// small box, a box below the baseline and a box for a non-BMP code point);
/// the tables' values come from here.
struct FontSpec {
    names: &'static [NameRec],
    /// (code point, glyph index) pairs, ascending; BMP ones go into the
    /// format 4 subtable, all into the format 12 one.
    cmap: &'static [(u32, u16)],
    format12: bool,
    /// platform 3, encoding 0 cmap (a symbol font), no format 12 subtable
    symbol_cmap: bool,
    weight_class: u16,
    width_class: u16,
    italic: bool,
    bold: bool,
    use_typo_metrics: bool,
    upem: u16,
    hhea: [i16; 3],
    typo: [i16; 3],
    win: [u16; 2],
    x_height: i16,
    cap_height: i16,
    underline_pos: i16,
    underline_thick: i16,
    strike_pos: i16,
    strike_size: i16,
    italic_angle_fixed: i32,
}

const STD_CMAP: &[(u32, u16)] = &[(0x20, 1), (0x41, 2), (0x42, 3), (0x43, 4), (0xE9, 5), (0x20AC, 6), (0x1F600, 7)];
const BASE_FONT: FontSpec = FontSpec {
    names: &[],
    cmap: STD_CMAP,
    format12: true,
    symbol_cmap: false,
    weight_class: 400,
    width_class: 5,
    italic: false,
    bold: false,
    use_typo_metrics: false,
    upem: 1000,
    hhea: [800, -200, 90],
    typo: [750, -250, 100],
    win: [900, 300],
    x_height: 500,
    cap_height: 700,
    underline_pos: -100,
    underline_thick: 50,
    strike_pos: 250,
    strike_size: 40,
    italic_angle_fixed: 0,
};

/// A glyph outline: points (x, y, on-curve) per contour, in a 1000-unit grid.
struct Outline {
    adv: u16,
    contours: &'static [&'static [(i16, i16, bool)]],
}

static GLYPHS: [Outline; 8] = [
    Outline { adv: 500, contours: &[&[(50, 0, true), (50, 700, true), (450, 700, true), (450, 0, true)]] },
    Outline { adv: 300, contours: &[] },
    Outline { adv: 1000, contours: &[&[(100, 0, true), (100, 800, true), (900, 800, true), (900, 0, true)]] },
    Outline { adv: 1000, contours: &[&[(0, 0, true), (500, 800, true), (1000, 0, true)]] },
    Outline {
        adv: 1000,
        contours: &[&[
            (500, 0, true),
            (0, 0, false),
            (0, 400, true),
            (0, 800, false),
            (500, 800, true),
            (1000, 800, false),
            (1000, 400, true),
            (1000, 0, false),
        ]],
    },
    Outline { adv: 700, contours: &[&[(200, 0, true), (200, 500, true), (600, 500, true), (600, 0, true)]] },
    Outline { adv: 600, contours: &[&[(50, -200, true), (50, 300, true), (550, 300, true), (550, -200, true)]] },
    Outline { adv: 1000, contours: &[&[(100, 100, true), (100, 700, true), (900, 700, true), (900, 100, true)]] },
];

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

/// Write a complete TrueType font for `spec` into `buf`; its length.
fn build_font(spec: &FontSpec, buf: &mut [u8]) -> usize {
    let tags: [&[u8; 4]; 10] = [b"OS/2", b"cmap", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"name", b"post"];
    let sc = |v: i16| -> i16 { ((v as i32) * (spec.upem as i32) / 1000) as i16 };
    let mut w = Wr { b: buf, n: 12 + 16 * 10 };
    let mut ent = [(0usize, 0usize); 10];
    let mut locs = [0u32; 9];
    let bmp_count = spec.cmap.iter().filter(|c| c.0 < 0x10000).count();
    for i in 0..10 {
        w.pad4();
        let start = w.n;
        match i {
            0 => {
                // OS/2, version 4
                w.u16(4);
                w.i16(sc(600));
                w.u16(spec.weight_class);
                w.u16(spec.width_class);
                w.u16(0); // fsType
                for v in [650i16, 600, 0, 75, 650, 600, 0, 350] {
                    w.i16(sc(v)); // subscript and superscript sizes and offsets
                }
                w.i16(spec.strike_size);
                w.i16(spec.strike_pos);
                w.i16(0); // sFamilyClass
                w.bytes(&[0; 10]); // panose
                w.u32(1); // unicode ranges
                w.u32(0);
                w.u32(0);
                w.u32(0);
                w.bytes(b"POLY");
                let mut sel = 0u16;
                if spec.italic {
                    sel |= 1;
                }
                if spec.bold {
                    sel |= 1 << 5;
                }
                if !spec.italic && !spec.bold {
                    sel |= 1 << 6;
                }
                if spec.use_typo_metrics {
                    sel |= 1 << 7;
                }
                w.u16(sel);
                w.u16(0x20);
                w.u16(0xFFFF);
                w.i16(spec.typo[0]);
                w.i16(spec.typo[1]);
                w.i16(spec.typo[2]);
                w.u16(spec.win[0]);
                w.u16(spec.win[1]);
                w.u32(1); // code page ranges
                w.u32(0);
                w.i16(spec.x_height);
                w.i16(spec.cap_height);
                w.u16(0);
                w.u16(0x20);
                w.u16(0);
            }
            1 => {
                // cmap
                let f12 = spec.format12 && !spec.symbol_cmap;
                let tables: u16 = if f12 { 2 } else { 1 };
                w.u16(0);
                w.u16(tables);
                let f4_len = 16 + 8 * (bmp_count + 1);
                let off4 = 4 + 8 * tables as usize;
                w.u16(3);
                w.u16(if spec.symbol_cmap { 0 } else { 1 });
                w.u32(off4 as u32);
                if f12 {
                    w.u16(3);
                    w.u16(10);
                    w.u32((off4 + f4_len) as u32);
                }
                // format 4: one segment per code point, plus the end mark
                let segs = bmp_count + 1;
                w.u16(4);
                w.u16(f4_len as u16);
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
                for c in spec.cmap.iter().filter(|c| c.0 < 0x10000) {
                    w.u16(c.0 as u16);
                }
                w.u16(0xFFFF);
                w.u16(0); // reserved pad
                for c in spec.cmap.iter().filter(|c| c.0 < 0x10000) {
                    w.u16(c.0 as u16);
                }
                w.u16(0xFFFF);
                for c in spec.cmap.iter().filter(|c| c.0 < 0x10000) {
                    w.u16(c.1.wrapping_sub(c.0 as u16));
                }
                w.u16(1);
                for _ in 0..segs {
                    w.u16(0);
                }
                if f12 {
                    w.u16(12);
                    w.u16(0);
                    w.u32((16 + 12 * spec.cmap.len()) as u32);
                    w.u32(0);
                    w.u32(spec.cmap.len() as u32);
                    for c in spec.cmap {
                        w.u32(c.0);
                        w.u32(c.0);
                        w.u32(c.1 as u32);
                    }
                }
            }
            2 => {
                // glyf
                for (g, o) in GLYPHS.iter().enumerate() {
                    locs[g] = (w.n - start) as u32;
                    if o.contours.is_empty() {
                        continue;
                    }
                    let (mut x0, mut y0, mut x1, mut y1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
                    for c in o.contours {
                        for p in *c {
                            x0 = x0.min(sc(p.0));
                            x1 = x1.max(sc(p.0));
                            y0 = y0.min(sc(p.1));
                            y1 = y1.max(sc(p.1));
                        }
                    }
                    w.i16(o.contours.len() as i16);
                    w.i16(x0);
                    w.i16(y0);
                    w.i16(x1);
                    w.i16(y1);
                    let mut end = 0i16;
                    for c in o.contours {
                        end += c.len() as i16;
                        w.i16(end - 1);
                    }
                    w.u16(0);
                    for c in o.contours {
                        for p in *c {
                            w.u8(u8::from(p.2));
                        }
                    }
                    let mut prev = 0i16;
                    for c in o.contours {
                        for p in *c {
                            w.i16(sc(p.0) - prev);
                            prev = sc(p.0);
                        }
                    }
                    prev = 0;
                    for c in o.contours {
                        for p in *c {
                            w.i16(sc(p.1) - prev);
                            prev = sc(p.1);
                        }
                    }
                    w.pad4();
                }
                locs[8] = (w.n - start) as u32;
            }
            3 => {
                // head
                w.u32(0x0001_0000);
                w.u32(0x0001_0000);
                w.u32(0); // checkSumAdjustment, patched below
                w.u32(0x5F0F_3CF5);
                w.u16(3);
                w.u16(spec.upem);
                w.u32(0);
                w.u32(0xD000_0000); // created
                w.u32(0);
                w.u32(0xD000_0000); // modified
                w.i16(0);
                w.i16(sc(-200));
                w.i16(sc(1000));
                w.i16(sc(800));
                w.u16(u16::from(spec.bold) | (u16::from(spec.italic) << 1));
                w.u16(8);
                w.i16(2);
                w.i16(1);
                w.i16(0);
            }
            4 => {
                // hhea
                w.u32(0x0001_0000);
                w.i16(spec.hhea[0]);
                w.i16(spec.hhea[1]);
                w.i16(spec.hhea[2]);
                w.u16(sc(1000) as u16);
                w.i16(0);
                w.i16(0);
                w.i16(sc(1000));
                w.i16(1);
                w.i16(0);
                w.i16(0);
                for _ in 0..4 {
                    w.i16(0);
                }
                w.i16(0);
                w.u16(8);
            }
            5 => {
                // hmtx
                for o in GLYPHS.iter() {
                    w.u16(sc(o.adv as i16) as u16);
                    let mut x0 = i16::MAX;
                    for c in o.contours {
                        for p in *c {
                            x0 = x0.min(sc(p.0));
                        }
                    }
                    w.i16(if o.contours.is_empty() { 0 } else { x0 });
                }
            }
            6 => {
                // loca (long)
                for l in locs {
                    w.u32(l);
                }
            }
            7 => {
                // maxp 1.0
                w.u32(0x0001_0000);
                w.u16(8);
                w.u16(8);
                w.u16(1);
                w.u16(0);
                w.u16(0);
                w.u16(1);
                for _ in 0..8 {
                    w.u16(0);
                }
            }
            8 => {
                // name
                let n = spec.names.len();
                let mut order = [0usize; 32];
                for (k, o) in order.iter_mut().enumerate().take(n) {
                    *o = k;
                }
                for a in 1..n {
                    let mut b = a;
                    while b > 0 {
                        let (p, q) = (&spec.names[order[b - 1]], &spec.names[order[b]]);
                        if (p.lang, p.id) > (q.lang, q.id) {
                            order.swap(b - 1, b);
                            b -= 1;
                        } else {
                            break;
                        }
                    }
                }
                w.u16(0);
                w.u16(n as u16);
                w.u16((6 + 12 * n) as u16);
                let mut off = 0u16;
                for &k in order.iter().take(n) {
                    let r = &spec.names[k];
                    let len = (r.text.encode_utf16().count() * 2) as u16;
                    w.u16(3);
                    w.u16(1);
                    w.u16(r.lang);
                    w.u16(r.id);
                    w.u16(len);
                    w.u16(off);
                    off += len;
                }
                for &k in order.iter().take(n) {
                    for u in spec.names[k].text.encode_utf16() {
                        w.u16(u);
                    }
                }
            }
            _ => {
                // post 3.0
                w.u32(0x0003_0000);
                w.u32(spec.italic_angle_fixed as u32);
                w.i16(spec.underline_pos);
                w.i16(spec.underline_thick as i16);
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
    // table directory
    w.n = 0;
    w.u32(0x0001_0000);
    w.u16(10);
    w.u16(128);
    w.u16(3);
    w.u16(32);
    for (i, tag) in tags.iter().enumerate() {
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
        w.bytes(*tag);
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
    let head = ent[3].0 + 8;
    w.b[head..head + 4].copy_from_slice(&adj.to_be_bytes());
    total
}

const FONT_SLOT_BYTES: usize = 6144;
struct FontSlot {
    data: [u8; FONT_SLOT_BYTES],
    len: usize,
}

static FONTS: Global<[FontSlot; 8]> = Global::new([const { FontSlot { data: [0; FONT_SLOT_BYTES], len: 0 } }; 8]);
/// Build `spec` into font slot `slot`.
fn install_font(slot: usize, spec: &FontSpec) {
    let f = &mut FONTS.get()[slot];
    f.len = build_font(spec, &mut f.data);
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

const LANG_EN_US: u16 = 0x409;
static N_ALPHA_R: [NameRec; 5] = [
    NameRec { id: 1, lang: LANG_EN_US, text: "PolyTest Alpha" },
    NameRec { id: 2, lang: LANG_EN_US, text: "Regular" },
    NameRec { id: 4, lang: LANG_EN_US, text: "PolyTest Alpha" },
    NameRec { id: 6, lang: LANG_EN_US, text: "PolyTestAlpha-Regular" },
    NameRec { id: 3, lang: LANG_EN_US, text: "PolyTestAlpha;1.0;POLY" },
];

static N_ALPHA_B: [NameRec; 3] = [
    NameRec { id: 1, lang: LANG_EN_US, text: "PolyTest Alpha" },
    NameRec { id: 2, lang: LANG_EN_US, text: "Bold" },
    NameRec { id: 4, lang: LANG_EN_US, text: "PolyTest Alpha Bold" },
];

static N_ALPHA_I: [NameRec; 3] = [
    NameRec { id: 1, lang: LANG_EN_US, text: "PolyTest Alpha" },
    NameRec { id: 2, lang: LANG_EN_US, text: "Italic" },
    NameRec { id: 4, lang: LANG_EN_US, text: "PolyTest Alpha Italic" },
];

static N_ALPHA_BI: [NameRec; 3] = [
    NameRec { id: 1, lang: LANG_EN_US, text: "PolyTest Alpha" },
    NameRec { id: 2, lang: LANG_EN_US, text: "Bold Italic" },
    NameRec { id: 4, lang: LANG_EN_US, text: "PolyTest Alpha Bold Italic" },
];

static N_BETA_L: [NameRec; 5] = [
    NameRec { id: 1, lang: LANG_EN_US, text: "PolyTest Beta Light" },
    NameRec { id: 2, lang: LANG_EN_US, text: "Regular" },
    NameRec { id: 16, lang: LANG_EN_US, text: "PolyTest Beta" },
    NameRec { id: 17, lang: LANG_EN_US, text: "Light" },
    NameRec { id: 4, lang: LANG_EN_US, text: "PolyTest Beta Light" },
];

static N_BETA_R: [NameRec; 3] = [
    NameRec { id: 1, lang: LANG_EN_US, text: "PolyTest Beta" },
    NameRec { id: 2, lang: LANG_EN_US, text: "Regular" },
    NameRec { id: 4, lang: LANG_EN_US, text: "PolyTest Beta" },
];

static N_BETA_K: [NameRec; 5] = [
    NameRec { id: 1, lang: LANG_EN_US, text: "PolyTest Beta Black" },
    NameRec { id: 2, lang: LANG_EN_US, text: "Regular" },
    NameRec { id: 16, lang: LANG_EN_US, text: "PolyTest Beta" },
    NameRec { id: 17, lang: LANG_EN_US, text: "Black" },
    NameRec { id: 4, lang: LANG_EN_US, text: "PolyTest Beta Black" },
];

static N_BETA_C: [NameRec; 5] = [
    NameRec { id: 1, lang: LANG_EN_US, text: "PolyTest Beta Condensed" },
    NameRec { id: 2, lang: LANG_EN_US, text: "Regular" },
    NameRec { id: 16, lang: LANG_EN_US, text: "PolyTest Beta" },
    NameRec { id: 17, lang: LANG_EN_US, text: "Condensed" },
    NameRec { id: 4, lang: LANG_EN_US, text: "PolyTest Beta Condensed" },
];

/// Eight fonts in the font slots 0..7: family "PolyTest Alpha" (Regular,
/// Bold, Italic, Bold Italic) and family "PolyTest Beta", whose faces are
/// named through name IDs 16 and 17 (Light 300, Regular 400, Black 900,
/// Condensed 400 at stretch 3).
fn setup_poly_fonts() {
    install_font(0, &FontSpec { names: &N_ALPHA_R, ..BASE_FONT });
    install_font(1, &FontSpec { names: &N_ALPHA_B, weight_class: 700, bold: true, ..BASE_FONT });
    install_font(2, &FontSpec { names: &N_ALPHA_I, italic: true, ..BASE_FONT });
    install_font(3, &FontSpec { names: &N_ALPHA_BI, weight_class: 700, bold: true, italic: true, ..BASE_FONT });
    install_font(4, &FontSpec { names: &N_BETA_L, weight_class: 300, ..BASE_FONT });
    install_font(5, &FontSpec { names: &N_BETA_R, ..BASE_FONT });
    install_font(6, &FontSpec { names: &N_BETA_K, weight_class: 900, ..BASE_FONT });
    install_font(7, &FontSpec { names: &N_BETA_C, width_class: 3, ..BASE_FONT });
}

const DWRITE_FACTORY_TYPE_ISOLATED: u32 = 1;
#[repr(C)]
struct IDWriteFontFileVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetReferenceKey: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    GetLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    Analyze: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFontFile {
    vtbl: *const IDWriteFontFileVtbl,
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
struct IDWriteInMemoryFontFileLoaderVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    CreateStreamFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    CreateInMemoryFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    GetFileCount: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IDWriteInMemoryFontFileLoader {
    vtbl: *const IDWriteInMemoryFontFileLoaderVtbl,
}

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

#[link(name = "dwrite", kind = "raw-dylib")]
unsafe extern "system" {
    fn DWriteCreateFactory(kind: u32, iid: *const GUID, factory: *mut *mut c_void) -> HRESULT;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    setup_poly_fonts();
    let fac = new_factory(DWRITE_FACTORY_TYPE_ISOLATED);
    let f5 = query(fac as *mut c_void, &IID_IDWriteFactory5).1;
    let mut ld: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid objects.
    unsafe {
        com!(f5 as *mut IDWriteFactory5, CreateInMemoryFontFileLoader(outp!(ld)));
        let mem = ld as *mut IDWriteInMemoryFontFileLoader;
        let font = &FONTS.get()[0];
        let garbage = [0x41u8; 64];
        for (tag, data, len) in [
            ("a font", font.data.as_ptr(), font.len as u32),
            ("64 bytes of text", garbage.as_ptr(), 64u32),
            ("size 0", font.data.as_ptr(), 0),
            ("a font cut after 100 bytes", font.data.as_ptr(), 100),
        ] {
            let mut file: *mut c_void = 0x10usize as *mut c_void;
            let hr = com!(
                mem,
                CreateInMemoryFontFileReference(fac as *mut c_void, data as *mut c_void, len, core::ptr::null_mut(), outp!(file))
            );
            let (mut s, mut a, mut b, mut c) = (7i32, 77u32, 77u32, 77u32);
            let hr2 = if hr == 0 { com!(file as *mut IDWriteFontFile, Analyze(outp!(s), outp!(a), outp!(b), outp!(c))) } else { -1 };
            say!(t, "{} (loader not registered): reference {} Analyze {} supported {}", tag, Hr(hr), Hr(hr2), s);
            if hr == 0 {
                release(file);
            }
        }
        com!(fac, RegisterFontFileLoader(ld));
        for (tag, data, len) in [
            ("a font", font.data.as_ptr(), font.len as u32),
            ("64 bytes of text", garbage.as_ptr(), 64u32),
            ("size 0", font.data.as_ptr(), 0),
            ("a font cut after 100 bytes", font.data.as_ptr(), 100),
        ] {
            let mut file: *mut c_void = 0x10usize as *mut c_void;
            let hr = com!(
                mem,
                CreateInMemoryFontFileReference(fac as *mut c_void, data as *mut c_void, len, core::ptr::null_mut(), outp!(file))
            );
            let (mut s, mut a, mut b, mut c) = (7i32, 77u32, 77u32, 77u32);
            let hr2 = if hr == 0 { com!(file as *mut IDWriteFontFile, Analyze(outp!(s), outp!(a), outp!(b), outp!(c))) } else { -1 };
            say!(t, "{} (registered): reference {} Analyze {} supported {}", tag, Hr(hr), Hr(hr2), s);
            if hr == 0 {
                release(file);
            }
        }
        say!(t, "file count {}", com!(mem, GetFileCount()));
    }
    release(ld);
    release(f5);
    release(fac as *mut c_void);
}
