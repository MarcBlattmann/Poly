//! IDWriteFontFamily::GetFirstMatchingFont on a family that has Regular,
//! Bold, Italic and Bold Italic (weights 400 and 700).
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
type BOOL = i32;
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
const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
const E_FAIL: HRESULT = 0x8000_4005u32 as i32;
const E_INVALIDARG: HRESULT = 0x8007_0057u32 as i32;
const E_FILE_NOT_FOUND: HRESULT = 0x8007_0002u32 as i32;
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

/// Prints UTF-16 units as text: printable ASCII as is, everything else
/// as `\u{hex}`.
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

/// AddRef on any COM object; the new reference count.
fn add_ref(obj: *mut c_void) -> u32 {
    // SAFETY: `obj` is a live COM object.
    unsafe {
        let u = obj as *mut IUnknown;
        ((*(*u).vtbl).AddRef)(obj)
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

/// Counters of what DirectWrite asked of the loaders.
struct LoaderCalls {
    streams_created: u32,
    sizes: u32,
    reads: u32,
    releases: u32,
    enumerators: u32,
    last_key_len: u32,
    last_key0: u32,
    enum_key_len: u32,
}

static CALLS: Global<LoaderCalls> = Global::new(LoaderCalls {
    streams_created: 0,
    sizes: 0,
    reads: 0,
    releases: 0,
    enumerators: 0,
    last_key_len: 0,
    last_key0: 0,
    enum_key_len: 0,
});

unsafe extern "system" fn obj_addref(_: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn obj_release(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn stream_qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> i32 {
    // SAFETY: DirectWrite passes a valid IID pointer and out pointer.
    unsafe {
        let iid = *(riid as *const GUID);
        let out = out as *mut *mut c_void;
        if iid == IID_IUnknown || iid == IID_IDWriteFontFileStream {
            *out = this;
            0
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn stream_read(this: *mut c_void, start: *mut c_void, offset: u64, size: u64, ctx: *mut c_void) -> i32 {
    // SAFETY: `this` is one of the STREAMS objects; `start` and `ctx` are valid out pointers.
    unsafe {
        let s = &*(this as *const FontStream);
        let f = &FONTS.get()[s.slot];
        CALLS.get().reads += 1;
        if offset.checked_add(size).is_none_or(|e| e > f.len as u64) {
            *(start as *mut *const c_void) = core::ptr::null();
            *(ctx as *mut *mut c_void) = core::ptr::null_mut();
            return E_FAIL;
        }
        *(start as *mut *const u8) = f.data.as_ptr().add(offset as usize);
        *(ctx as *mut *mut c_void) = core::ptr::null_mut();
    }
    0
}

unsafe extern "system" fn stream_release_fragment(_: *mut c_void, _: *mut c_void) {
    CALLS.get().releases += 1;
}

unsafe extern "system" fn stream_size(this: *mut c_void, size: *mut c_void) -> i32 {
    // SAFETY: `this` is one of the STREAMS objects; `size` is a valid out pointer.
    unsafe {
        let s = &*(this as *const FontStream);
        CALLS.get().sizes += 1;
        *(size as *mut u64) = FONTS.get()[s.slot].len as u64;
    }
    0
}

unsafe extern "system" fn stream_time(_: *mut c_void, time: *mut c_void) -> i32 {
    // SAFETY: `time` is a valid out pointer.
    unsafe { *(time as *mut u64) = 0 };
    0
}

static STREAM_VTBL: IDWriteFontFileStreamVtbl = IDWriteFontFileStreamVtbl {
    QueryInterface: stream_qi,
    AddRef: obj_addref,
    Release: obj_release,
    ReadFileFragment: stream_read,
    ReleaseFileFragment: stream_release_fragment,
    GetFileSize: stream_size,
    GetLastWriteTime: stream_time,
};

#[repr(C)]
struct FontStream {
    vtbl: *const IDWriteFontFileStreamVtbl,
    slot: usize,
}

static STREAMS: Global<[FontStream; 8]> = Global::new([
    FontStream { vtbl: &STREAM_VTBL, slot: 0 },
    FontStream { vtbl: &STREAM_VTBL, slot: 1 },
    FontStream { vtbl: &STREAM_VTBL, slot: 2 },
    FontStream { vtbl: &STREAM_VTBL, slot: 3 },
    FontStream { vtbl: &STREAM_VTBL, slot: 4 },
    FontStream { vtbl: &STREAM_VTBL, slot: 5 },
    FontStream { vtbl: &STREAM_VTBL, slot: 6 },
    FontStream { vtbl: &STREAM_VTBL, slot: 7 },
]);

unsafe extern "system" fn loader_qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> i32 {
    // SAFETY: DirectWrite passes a valid IID pointer and out pointer.
    unsafe {
        let iid = *(riid as *const GUID);
        let out = out as *mut *mut c_void;
        if iid == IID_IUnknown || iid == IID_IDWriteFontFileLoader {
            *out = this;
            0
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

/// The key is a 4-byte little-endian font slot number.
unsafe extern "system" fn loader_create_stream(_: *mut c_void, key: *mut c_void, key_size: u32, stream: *mut c_void) -> i32 {
    // SAFETY: DirectWrite passes `key_size` readable bytes at `key` and a valid out pointer.
    unsafe {
        let calls = CALLS.get();
        calls.streams_created += 1;
        calls.last_key_len = key_size;
        let out = stream as *mut *mut c_void;
        if key_size != 4 {
            *out = core::ptr::null_mut();
            return E_INVALIDARG;
        }
        let slot = u32::from_le_bytes(*(key as *const [u8; 4])) as usize;
        calls.last_key0 = slot as u32;
        if slot >= 8 || FONTS.get()[slot].len == 0 {
            *out = core::ptr::null_mut();
            return E_FILE_NOT_FOUND;
        }
        *out = &mut STREAMS.get()[slot] as *mut FontStream as *mut c_void;
    }
    0
}

static LOADER_VTBL: IDWriteFontFileLoaderVtbl = IDWriteFontFileLoaderVtbl {
    QueryInterface: loader_qi,
    AddRef: obj_addref,
    Release: obj_release,
    CreateStreamFromKey: loader_create_stream,
};

#[repr(C)]
struct FileLoader {
    vtbl: *const IDWriteFontFileLoaderVtbl,
}

static FILE_LOADER: Global<FileLoader> = Global::new(FileLoader { vtbl: &LOADER_VTBL });
fn file_loader() -> *mut c_void {
    FILE_LOADER.get() as *mut FileLoader as *mut c_void
}

/// Key bytes for font slot `slot`.
fn slot_key(slot: u32) -> [u8; 4] {
    slot.to_le_bytes()
}

#[repr(C)]
struct FontEnum {
    vtbl: *const IDWriteFontFileEnumeratorVtbl,
    factory: *mut c_void,
    slots: [u8; 8],
    count: usize,
    pos: usize,
    current: *mut c_void,
}

unsafe extern "system" fn enum_qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> i32 {
    // SAFETY: DirectWrite passes a valid IID pointer and out pointer.
    unsafe {
        let iid = *(riid as *const GUID);
        let out = out as *mut *mut c_void;
        if iid == IID_IUnknown || iid == IID_IDWriteFontFileEnumerator {
            *out = this;
            0
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn enum_move_next(this: *mut c_void, has: *mut c_void) -> i32 {
    // SAFETY: `this` is one of the ENUMS objects; `has` is a valid out pointer.
    unsafe {
        let e = &mut *(this as *mut FontEnum);
        if e.pos < e.count {
            let key = slot_key(e.slots[e.pos] as u32);
            let mut file: *mut c_void = core::ptr::null_mut();
            let f = e.factory as *mut IDWriteFactory;
            let hr = com!(
                f,
                CreateCustomFontFileReference(key.as_ptr() as *mut c_void, 4, file_loader(), &mut file as *mut *mut c_void as *mut c_void)
            );
            if hr < 0 {
                return hr;
            }
            e.current = file;
            e.pos += 1;
            *(has as *mut BOOL) = 1;
        } else {
            e.current = core::ptr::null_mut();
            *(has as *mut BOOL) = 0;
        }
    }
    0
}

unsafe extern "system" fn enum_current(this: *mut c_void, file: *mut c_void) -> i32 {
    // SAFETY: `this` is one of the ENUMS objects; `file` is a valid out pointer.
    unsafe {
        let e = &mut *(this as *mut FontEnum);
        if e.current.is_null() {
            *(file as *mut *mut c_void) = core::ptr::null_mut();
            return E_FAIL;
        }
        add_ref(e.current);
        *(file as *mut *mut c_void) = e.current;
    }
    0
}

static ENUM_VTBL: IDWriteFontFileEnumeratorVtbl = IDWriteFontFileEnumeratorVtbl {
    QueryInterface: enum_qi,
    AddRef: obj_addref,
    Release: obj_release,
    MoveNext: enum_move_next,
    GetCurrentFontFile: enum_current,
};

static ENUMS: Global<[FontEnum; 4]> = Global::new(
    [const { FontEnum { vtbl: &ENUM_VTBL, factory: core::ptr::null_mut(), slots: [0; 8], count: 0, pos: 0, current: core::ptr::null_mut() } };
        4],
);
static NEXT_ENUM: Global<usize> = Global::new(0);
unsafe extern "system" fn coll_loader_qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> i32 {
    // SAFETY: DirectWrite passes a valid IID pointer and out pointer.
    unsafe {
        let iid = *(riid as *const GUID);
        let out = out as *mut *mut c_void;
        if iid == IID_IUnknown || iid == IID_IDWriteFontCollectionLoader {
            *out = this;
            0
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn coll_loader_create(
    _: *mut c_void,
    factory: *mut c_void,
    key: *mut c_void,
    key_size: u32,
    out: *mut c_void,
) -> i32 {
    // SAFETY: DirectWrite passes `key_size` readable bytes at `key` and a valid out pointer.
    unsafe {
        let calls = CALLS.get();
        calls.enumerators += 1;
        calls.enum_key_len = key_size;
        let out = out as *mut *mut c_void;
        if key_size > 8 {
            *out = core::ptr::null_mut();
            return E_INVALIDARG;
        }
        let i = *NEXT_ENUM.get() % 4;
        *NEXT_ENUM.get() += 1;
        let e = &mut ENUMS.get()[i];
        e.factory = factory;
        e.count = key_size as usize;
        e.pos = 0;
        e.current = core::ptr::null_mut();
        for k in 0..key_size as usize {
            e.slots[k] = *(key as *const u8).add(k);
        }
        *out = e as *mut FontEnum as *mut c_void;
    }
    0
}

static COLL_LOADER_VTBL: IDWriteFontCollectionLoaderVtbl = IDWriteFontCollectionLoaderVtbl {
    QueryInterface: coll_loader_qi,
    AddRef: obj_addref,
    Release: obj_release,
    CreateEnumeratorFromKey: coll_loader_create,
};

#[repr(C)]
struct CollLoader {
    vtbl: *const IDWriteFontCollectionLoaderVtbl,
}

static COLL_LOADER: Global<CollLoader> = Global::new(CollLoader { vtbl: &COLL_LOADER_VTBL });
fn coll_loader() -> *mut c_void {
    COLL_LOADER.get() as *mut CollLoader as *mut c_void
}

/// A NUL-terminated UTF-16 literal (ASCII): `wz!("Arial")` is a `&[u16; 6]`.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W[..]
    }};
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

/// Register the test's file loader with `fac`.
fn register_file_loader(fac: *mut IDWriteFactory) -> HRESULT {
    // SAFETY: the factory is live and the loader object is static.
    unsafe { com!(fac, RegisterFontFileLoader(file_loader())) }
}

/// Register the test's collection loader with `fac`.
fn register_coll_loader(fac: *mut IDWriteFactory) -> HRESULT {
    // SAFETY: the factory is live and the loader object is static.
    unsafe { com!(fac, RegisterFontCollectionLoader(coll_loader())) }
}

/// A custom font collection of the listed font slots (needs both loaders registered).
fn custom_collection(fac: *mut IDWriteFactory, slots: &[u8]) -> (HRESULT, *mut IDWriteFontCollection) {
    let mut c: *mut c_void = core::ptr::null_mut();
    // SAFETY: the factory is live; the key outlives the call.
    let hr = unsafe { com!(fac, CreateCustomFontCollection(coll_loader(), slots.as_ptr() as *mut c_void, slots.len() as u32, outp!(c))) };
    (hr, c as *mut IDWriteFontCollection)
}

/// The text of locale-name or localized-string entry `i` of `s` into `out`.
fn lstr(s: *mut IDWriteLocalizedStrings, i: u32, out: &mut Buf<200>) {
    let mut len = 0u32;
    let mut buf = [0u16; 128];
    // SAFETY: `s` is live; the buffer is large enough for the strings of these tests.
    unsafe {
        com!(s, GetStringLength(i, outp!(len)));
        let hr = com!(s, GetString(i, buf.as_mut_ptr() as *mut c_void, 128));
        if hr != 0 {
            let _ = write!(out, "<GetString {}>", Hr(hr));
            return;
        }
    }
    let _ = write!(out, "{}", W16(&buf[..(len as usize).min(127)]));
}

fn lloc(s: *mut IDWriteLocalizedStrings, i: u32, out: &mut Buf<200>) {
    let mut len = 0u32;
    let mut buf = [0u16; 64];
    // SAFETY: as `lstr`.
    unsafe {
        com!(s, GetLocaleNameLength(i, outp!(len)));
        let hr = com!(s, GetLocaleName(i, buf.as_mut_ptr() as *mut c_void, 64));
        if hr != 0 {
            let _ = write!(out, "<GetLocaleName {}>", Hr(hr));
            return;
        }
    }
    let _ = write!(out, "{}", W16(&buf[..(len as usize).min(63)]));
}

/// "locale=text" pairs of a localized strings object, `; `-separated.
fn dump_strings(s: *mut IDWriteLocalizedStrings, out: &mut Buf<200>) {
    if s.is_null() {
        let _ = write!(out, "null");
        return;
    }
    // SAFETY: `s` is live.
    let n = unsafe { com!(s, GetCount()) };
    for i in 0..n {
        if i > 0 {
            let _ = write!(out, "; ");
        }
        lloc(s, i, out);
        let _ = write!(out, "=");
        lstr(s, i, out);
    }
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

/// An isolated factory with both loaders registered and the custom
/// collection of font slots 0..7 (see [`setup_poly_fonts`]).
fn poly_env() -> (*mut IDWriteFactory, *mut IDWriteFontCollection) {
    setup_poly_fonts();
    let fac = new_factory(DWRITE_FACTORY_TYPE_ISOLATED);
    register_file_loader(fac);
    register_coll_loader(fac);
    let (hr, c) = custom_collection(fac, &[0, 1, 2, 3, 4, 5, 6, 7]);
    if hr != 0 {
        exit(9);
    }
    (fac, c)
}

/// The family with the given name (NUL-terminated UTF-16) in `c`, or null.
fn family_named(c: *mut IDWriteFontCollection, name: &[u16]) -> *mut IDWriteFontFamily {
    let mut idx = 0u32;
    let mut ex = 0i32;
    let mut fam: *mut c_void = core::ptr::null_mut();
    // SAFETY: `c` is live; the pointers are valid for the calls.
    unsafe {
        com!(c, FindFamilyName(name.as_ptr() as *mut c_void, outp!(idx), outp!(ex)));
        if ex != 0 {
            com!(c, GetFontFamily(idx, outp!(fam)));
        }
    }
    fam as *mut IDWriteFontFamily
}

/// "weight/stretch/style/simulations face-name(en-us)" of a font.
fn font_text(ft: *mut IDWriteFont) -> Buf<200> {
    let mut b = Buf::<200>::new();
    if ft.is_null() {
        let _ = write!(b, "null");
        return b;
    }
    // SAFETY: `ft` is a live font.
    unsafe {
        let mut fnm: *mut c_void = core::ptr::null_mut();
        com!(ft, GetFaceNames(outp!(fnm)));
        let _ = write!(
            b,
            "w{} s{} st{} sim{} ",
            com!(ft, GetWeight()),
            com!(ft, GetStretch()),
            com!(ft, GetStyle()),
            com!(ft, GetSimulations())
        );
        let mut names = Buf::<200>::new();
        dump_strings(fnm as *mut IDWriteLocalizedStrings, &mut names);
        let _ = write!(b, "{}", names.as_str());
        release(fnm);
    }
    b
}

const DWRITE_FONT_STRETCH_NORMAL: u32 = 5;
const DWRITE_FONT_STYLE_NORMAL: u32 = 0;
const DWRITE_FONT_STYLE_OBLIQUE: u32 = 1;
const DWRITE_FONT_STYLE_ITALIC: u32 = 2;
const DWRITE_FACTORY_TYPE_ISOLATED: u32 = 1;
#[repr(C)]
struct IDWriteFontFileLoaderVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    CreateStreamFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void) -> i32,
}

const IID_IDWriteFontFileLoader: GUID =
    GUID { d1: 0x727CAD4E, d2: 0xD6AF, d3: 0x4C9E, d4: [0x8A, 0x08, 0xD6, 0x95, 0xB1, 0x1C, 0xAA, 0x49] };
#[repr(C)]
struct IDWriteFontFileStreamVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    ReadFileFragment: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u64, *mut c_void) -> i32,
    ReleaseFileFragment: unsafe extern "system" fn(*mut c_void, *mut c_void),
    GetFileSize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetLastWriteTime: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

const IID_IDWriteFontFileStream: GUID =
    GUID { d1: 0x6D4865FE, d2: 0x0AB8, d3: 0x4D91, d4: [0x8F, 0x62, 0x5D, 0xD6, 0xBE, 0x34, 0xA3, 0xE0] };
#[repr(C)]
struct IDWriteFontCollectionLoaderVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    CreateEnumeratorFromKey: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
}

const IID_IDWriteFontCollectionLoader: GUID =
    GUID { d1: 0xCCA920E4, d2: 0x52F0, d3: 0x492B, d4: [0xBF, 0xA8, 0x29, 0xC7, 0x2E, 0xE0, 0xA4, 0x68] };
#[repr(C)]
struct IDWriteFontFileEnumeratorVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    MoveNext: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetCurrentFontFile: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

const IID_IDWriteFontFileEnumerator: GUID =
    GUID { d1: 0x72755049, d2: 0x5FF7, d3: 0x435D, d4: [0x83, 0x48, 0x4B, 0xE9, 0x7C, 0xFA, 0x6C, 0x7C] };
#[repr(C)]
struct IDWriteLocalizedStringsVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetCount: unsafe extern "system" fn(*mut c_void) -> u32,
    FindLocaleName: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    GetLocaleNameLength: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    GetLocaleName: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32,
    GetStringLength: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    GetString: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32,
}

#[repr(C)]
struct IDWriteLocalizedStrings {
    vtbl: *const IDWriteLocalizedStringsVtbl,
}

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
struct IDWriteFontFamilyVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetFontCount: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFont: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    GetFamilyNames: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetFirstMatchingFont: unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32,
    GetMatchingFonts: unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32,
}

#[repr(C)]
struct IDWriteFontFamily {
    vtbl: *const IDWriteFontFamilyVtbl,
}

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

fn test(t: &mut Test) {
    let (fac, c) = poly_env();
    let fm = family_named(c, wz!("PolyTest Alpha"));
    let weights = [
        0u32, 1, 100, 200, 300, 350, 399, 400, 401, 450, 499, 500, 501, 550, 599, 600, 649, 650, 651, 700, 800, 900, 950, 999, 1000, 1001,
        5000,
    ];
    for style in [DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STYLE_OBLIQUE, DWRITE_FONT_STYLE_ITALIC] {
        for &w in weights.iter() {
            let mut font: *mut c_void = core::ptr::null_mut();
            // SAFETY: the family is live.
            let hr = unsafe { com!(fm, GetFirstMatchingFont(w, DWRITE_FONT_STRETCH_NORMAL, style, outp!(font))) };
            let txt = font_text(font as *mut IDWriteFont);
            say!(t, "style {} weight {}: {} {}", style, w, Hr(hr), txt.as_str());
            release(font);
        }
    }
    for (st, sy) in [(0u32, 0u32), (1, 0), (3, 0), (7, 0), (9, 0), (10, 0), (5, 3), (5, 4), (5, 100)] {
        let mut font = 0x10usize as *mut c_void;
        // SAFETY: the family is live.
        let hr = unsafe { com!(fm, GetFirstMatchingFont(400, st, sy, outp!(font))) };
        let txt = font_text(if hr == 0 { font as *mut IDWriteFont } else { core::ptr::null_mut() });
        say!(t, "stretch {} style {}: {} {} out_untouched={}", st, sy, Hr(hr), txt.as_str(), font as usize == 0x10);
        if hr == 0 {
            release(font);
        }
    }
    release(fm as *mut c_void);
    release(c as *mut c_void);
    release(fac as *mut c_void);
}
