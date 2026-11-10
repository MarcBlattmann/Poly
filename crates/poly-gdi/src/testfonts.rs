//! TrueType fonts for the host tests: the "PolyTest" fonts the spec sheets'
//! tests (`specs/windows/gdi32.dll/*.tests`) write for themselves, so that
//! the metrics and pixels the sheets give can be checked without the
//! machine. Fourteen glyphs on a 1024-unit em; every outline coordinate is
//! a multiple of 64 units, so that at 16 pixels per em every edge lies on a
//! pixel boundary.

use alloc::vec::Vec;

struct Glyph {
    cp: u32,
    adv: u16,
    contours: &'static [&'static [(i16, i16)]],
}

macro_rules! rect {
    ($x0:expr, $y0:expr, $x1:expr, $y1:expr) => {
        &[($x0, $y0), ($x0, $y1), ($x1, $y1), ($x1, $y0)]
    };
}

static GLYPHS: [Glyph; 14] = [
    Glyph {
        cp: 0,
        adv: 640,
        contours: &[&[(64, 0), (64, 768), (576, 768), (576, 0)], &[(192, 128), (448, 128), (448, 640), (192, 640)]],
    },
    Glyph { cp: 0x20, adv: 384, contours: &[] },
    Glyph { cp: 0x26, adv: 576, contours: &[rect!(64, 0, 512, 640)] },
    Glyph { cp: 0x2D, adv: 384, contours: &[rect!(64, 256, 320, 320)] },
    Glyph { cp: 0x2E, adv: 256, contours: &[rect!(64, 0, 192, 128)] },
    Glyph { cp: 0x41, adv: 768, contours: &[rect!(128, 0, 640, 768)] },
    Glyph { cp: 0x42, adv: 512, contours: &[rect!(64, 0, 448, 512)] },
    Glyph { cp: 0x4D, adv: 768, contours: &[&[(0, 0), (384, 768), (768, 0)]] },
    Glyph { cp: 0x57, adv: 1024, contours: &[rect!(128, 0, 896, 768)] },
    Glyph { cp: 0x5C, adv: 384, contours: &[rect!(64, 0, 320, 768)] },
    Glyph { cp: 0x67, adv: 640, contours: &[rect!(64, -256, 576, 384)] },
    Glyph { cp: 0x69, adv: 256, contours: &[rect!(64, 0, 192, 768)] },
    Glyph { cp: 0xE9, adv: 704, contours: &[rect!(64, 0, 640, 512)] },
    Glyph { cp: 0x20AC, adv: 896, contours: &[rect!(64, 0, 832, 768)] },
];

/// What a font looks like.
#[derive(Clone, Copy)]
pub struct Def {
    pub family: &'static str,
    pub style: &'static str,
    pub weight_class: u16,
    pub bold: bool,
    pub italic: bool,
    pub fixed: bool,
    pub win_asc: u16,
    pub win_desc: u16,
    pub line_gap: i16,
    pub avg_w: i16,
    /// The advance of 'A' in units (0 keeps the glyph table's 768).
    pub a_adv: u16,
    pub underline_pos: i16,
    pub underline_thick: i16,
    pub strike_pos: i16,
    pub strike_size: i16,
}

/// "PolyTest Sans": a cell of one em, line gap 128.
pub const SANS: Def = Def {
    family: "PolyTest Sans",
    style: "Regular",
    weight_class: 400,
    bold: false,
    italic: false,
    fixed: false,
    win_asc: 768,
    win_desc: 256,
    line_gap: 128,
    avg_w: 512,
    a_adv: 0,
    underline_pos: -128,
    underline_thick: 64,
    strike_pos: 256,
    strike_size: 64,
};

/// "PolyTest Tall": a cell of 1216 units (19 pixels at 16 per em).
pub const TALL: Def = Def {
    family: "PolyTest Tall",
    win_asc: 896,
    win_desc: 320,
    line_gap: 0,
    avg_w: 640,
    underline_pos: -192,
    underline_thick: 128,
    strike_pos: 320,
    strike_size: 128,
    ..SANS
};

/// "PolyTest Mono": every advance 640.
pub const MONO: Def = Def { family: "PolyTest Mono", fixed: true, avg_w: 640, ..SANS };

/// The Bold face of the "PolyTest Sans" family: 'A' is 13 pixels wide.
pub const SANS_BOLD: Def = Def { style: "Bold", weight_class: 700, bold: true, a_adv: 832, ..SANS };

/// The Italic face of the family: 'A' is 14 pixels wide.
pub const SANS_ITALIC: Def = Def { style: "Italic", italic: true, a_adv: 896, ..SANS };

struct W(Vec<u8>);

impl W {
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn pad4(&mut self) {
        let pad = (4 - self.0.len() % 4) % 4;
        self.0.resize(self.0.len() + pad, 0);
    }
}

fn adv(d: &Def, g: &Glyph) -> u16 {
    if d.fixed {
        640
    } else if g.cp == 0x41 && d.a_adv != 0 {
        d.a_adv
    } else {
        g.adv
    }
}

fn os2(d: &Def) -> Vec<u8> {
    let mut w = W(Vec::new());
    w.u16(4);
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
    w.0.extend_from_slice(&pan);
    for v in [1u32, 0, 0, 0] {
        w.u32(v);
    }
    w.0.extend_from_slice(b"POLY");
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
    w.i16(d.win_asc as i16);
    w.i16(-(d.win_desc as i16));
    w.i16(d.line_gap);
    w.u16(d.win_asc);
    w.u16(d.win_desc);
    w.u32(1);
    w.u32(0);
    w.i16(512);
    w.i16(768);
    w.u16(0);
    w.u16(0x20);
    w.u16(0);
    w.0
}

fn cmap() -> Vec<u8> {
    let mut w = W(Vec::new());
    w.u16(0);
    w.u16(1);
    w.u16(3);
    w.u16(1);
    w.u32(12);
    let segs = GLYPHS.len();
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
    w.0
}

fn glyf() -> (Vec<u8>, Vec<u32>) {
    let mut w = W(Vec::new());
    let mut locs = Vec::new();
    for g in GLYPHS.iter() {
        locs.push(w.0.len() as u32);
        if g.contours.is_empty() {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
        for c in g.contours {
            for p in *c {
                x0 = x0.min(p.0);
                x1 = x1.max(p.0);
                y0 = y0.min(p.1);
                y1 = y1.max(p.1);
            }
        }
        w.i16(g.contours.len() as i16);
        for v in [x0, y0, x1, y1] {
            w.i16(v);
        }
        let mut end = 0i16;
        for c in g.contours {
            end += c.len() as i16;
            w.i16(end - 1);
        }
        w.u16(0);
        for c in g.contours {
            for _ in *c {
                w.0.push(1);
            }
        }
        let mut prev = 0i16;
        for c in g.contours {
            for p in *c {
                w.i16(p.0 - prev);
                prev = p.0;
            }
        }
        prev = 0;
        for c in g.contours {
            for p in *c {
                w.i16(p.1 - prev);
                prev = p.1;
            }
        }
        w.pad4();
    }
    locs.push(w.0.len() as u32);
    (w.0, locs)
}

fn name_table(d: &Def) -> Vec<u8> {
    let full = if d.style == "Regular" { d.family.into() } else { alloc::format!("{} {}", d.family, d.style) };
    let ps: alloc::string::String = full.chars().filter(|&c| c != ' ').collect();
    let uid = alloc::format!("{full} 1.0");
    let recs: [(u16, &str); 7] = [
        (0, "PolyTest fonts for Poly's text specs"),
        (1, d.family),
        (2, d.style),
        (3, &uid),
        (4, &full),
        (5, "Version 1.0"),
        (6, &ps),
    ];
    let mut w = W(Vec::new());
    w.u16(0);
    w.u16(recs.len() as u16);
    w.u16((6 + 12 * recs.len()) as u16);
    let mut off = 0u16;
    for (id, s) in &recs {
        let len = (s.encode_utf16().count() * 2) as u16;
        w.u16(3);
        w.u16(1);
        w.u16(0x409);
        w.u16(*id);
        w.u16(len);
        w.u16(off);
        off += len;
    }
    for (_, s) in &recs {
        for u in s.encode_utf16() {
            w.u16(u);
        }
    }
    w.0
}

/// A complete TrueType font for `d`.
pub fn build(d: &Def) -> Vec<u8> {
    let (glyf, locs) = glyf();
    let ng = GLYPHS.len() as u16;
    let mut head = W(Vec::new());
    for v in [0x0001_0000u32, 0x0001_0000, 0, 0x5F0F_3CF5] {
        head.u32(v);
    }
    head.u16(3);
    head.u16(1024);
    for v in [0u32, 0xD000_0000, 0, 0xD000_0000] {
        head.u32(v);
    }
    for v in [0i16, -256, 1024, 768] {
        head.i16(v);
    }
    head.u16(u16::from(d.bold) | (u16::from(d.italic) << 1));
    head.u16(8);
    head.i16(2);
    head.i16(1);
    head.i16(0);
    let mut hhea = W(Vec::new());
    hhea.u32(0x0001_0000);
    hhea.i16(d.win_asc as i16);
    hhea.i16(-(d.win_desc as i16));
    hhea.i16(d.line_gap);
    hhea.u16(1024);
    for v in [0i16, 0, 1024, 1, 0, 0, 0, 0, 0, 0, 0] {
        hhea.i16(v);
    }
    hhea.u16(ng);
    let mut hmtx = W(Vec::new());
    for g in GLYPHS.iter() {
        hmtx.u16(adv(d, g));
        let x0 = g.contours.iter().flat_map(|c| c.iter()).map(|p| p.0).min().unwrap_or(0);
        hmtx.i16(x0);
    }
    let mut loca = W(Vec::new());
    for l in &locs {
        loca.u32(*l);
    }
    let mut maxp = W(Vec::new());
    maxp.u32(0x0001_0000);
    maxp.u16(ng);
    for v in [80u16, 3, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0] {
        maxp.u16(v);
    }
    let mut post = W(Vec::new());
    post.u32(0x0003_0000);
    post.u32(0);
    post.i16(d.underline_pos);
    post.i16(d.underline_thick);
    post.u32(u32::from(d.fixed));
    for _ in 0..4 {
        post.u32(0);
    }
    let tables: [(&[u8; 4], Vec<u8>); 10] = [
        (b"OS/2", os2(d)),
        (b"cmap", cmap()),
        (b"glyf", glyf),
        (b"head", head.0),
        (b"hhea", hhea.0),
        (b"hmtx", hmtx.0),
        (b"loca", loca.0),
        (b"maxp", maxp.0),
        (b"name", name_table(d)),
        (b"post", post.0),
    ];
    let mut out = W(Vec::new());
    out.u32(0x0001_0000);
    out.u16(tables.len() as u16);
    out.u16(128);
    out.u16(3);
    out.u16(32);
    let mut off = 12 + 16 * tables.len();
    let mut dir = W(Vec::new());
    for (tag, data) in &tables {
        dir.0.extend_from_slice(*tag);
        dir.u32(0);
        dir.u32(off as u32);
        dir.u32(data.len() as u32);
        off += data.len().div_ceil(4) * 4;
    }
    out.0.extend(dir.0);
    for (_, data) in &tables {
        out.0.extend_from_slice(data);
        out.pad4();
    }
    out.0
}
