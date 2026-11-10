//! Poly's built-in scalable font (ADR 0142, extended): a TrueType font made
//! in memory from Spleen 8x16's pixels (`poly-gfx`, BSD 2-Clause), one
//! square of outline per ink pixel. It stands in for the fonts of the font
//! store where those files are not there (a bare root disk, the test
//! images of the diff runner), so that a logical font always has the
//! metrics and scaling of a TrueType face: pixel-exact at 16 pixels per em
//! and at its multiples, blocky in between.
//!
//! Units per em 1024, one Spleen pixel 64 units; Spleen's baseline lies
//! below row 11, so ascent 768 and descent 256, like the test fonts of the
//! spec sheets. Advances: a glyph's last ink column plus two pixels (the
//! proportional face), or 8 pixels (the fixed-pitch one).

use alloc::vec::Vec;
use poly_gfx::SPLEEN_8X16;

const UNIT: i16 = 64;

/// The code point ranges the font covers (those of them Spleen has).
const RANGES: &[(u32, u32)] = &[
    (0x20, 0x7E),
    (0xA0, 0x24F),
    (0x370, 0x3FF),
    (0x400, 0x4FF),
    (0x2010, 0x205E),
    (0x20A0, 0x20BF),
    (0x2100, 0x214F),
    (0x2190, 0x21FF),
    (0x2200, 0x22FF),
    (0x2500, 0x259F),
    (0x25A0, 0x25FF),
    (0xFFFD, 0xFFFD),
];

struct Glyph {
    cp: u32,
    adv: u16,
    /// Rectangles (x0, y0, x1, y1) in font units.
    rects: Vec<(i16, i16, i16, i16)>,
}

struct Wr(Vec<u8>);

impl Wr {
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

/// One glyph's squares: runs of ink in a row, merged with the rows below
/// that have the same run.
fn rects_of(rows: &[u8; 16]) -> Vec<(i16, i16, i16, i16)> {
    let mut runs: Vec<(u8, u8, usize, usize)> = Vec::new(); // x0, x1, first row, last row
    for (y, &r) in rows.iter().enumerate() {
        let mut x = 0u8;
        while x < 8 {
            if r & (0x80 >> x) == 0 {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < 8 && r & (0x80 >> x) != 0 {
                x += 1;
            }
            match runs.iter_mut().find(|(a, b, _, last)| *a == x0 && *b == x && *last + 1 == y) {
                Some(run) => run.3 = y,
                None => runs.push((x0, x, y, y)),
            }
        }
    }
    runs.into_iter()
        .map(|(x0, x1, r0, r1)| {
            (i16::from(x0) * UNIT, (11 - r1 as i16) * UNIT, i16::from(x1) * UNIT, (12 - r0 as i16) * UNIT)
        })
        .collect()
}

fn glyphs(mono: bool) -> Vec<Glyph> {
    // .notdef: a hollow box.
    let notdef_rows = [0, 0, 0x7E, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x7E, 0, 0, 0];
    let mut out = alloc::vec![Glyph { cp: 0, adv: 8 * UNIT as u16, rects: rects_of(&notdef_rows) }];
    for &(lo, hi) in RANGES {
        for cp in lo..=hi {
            let Some(ch) = char::from_u32(cp) else { continue };
            let Some(g) = SPLEEN_8X16.lookup(ch) else { continue };
            let mut rows = [0u8; 16];
            for (y, r) in rows.iter_mut().enumerate() {
                *r = (g.row(y as u32) >> 24) as u8;
            }
            let any = rows.iter().fold(0u8, |a, &r| a | r);
            let adv_px: u16 = if mono {
                8
            } else if any == 0 {
                4
            } else {
                // The last ink column plus the gap, at least 3.
                (8 - any.trailing_zeros() as u16 + 1).max(3)
            };
            out.push(Glyph { cp, adv: adv_px * UNIT as u16, rects: rects_of(&rows) });
        }
    }
    out
}

/// The font file: `mono` gives the fixed-pitch face.
pub fn build(mono: bool) -> Vec<u8> {
    let gl = glyphs(mono);
    let ng = gl.len();
    // glyf and loca.
    let mut glyf = Wr(Vec::new());
    let mut locs: Vec<u32> = Vec::new();
    let (mut bx0, mut by0, mut bx1, mut by1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
    for g in &gl {
        locs.push(glyf.0.len() as u32);
        if g.rects.is_empty() {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
        for &(a, b, c, d) in &g.rects {
            x0 = x0.min(a);
            y0 = y0.min(b);
            x1 = x1.max(c);
            y1 = y1.max(d);
        }
        bx0 = bx0.min(x0);
        by0 = by0.min(y0);
        bx1 = bx1.max(x1);
        by1 = by1.max(y1);
        glyf.i16(g.rects.len() as i16);
        for v in [x0, y0, x1, y1] {
            glyf.i16(v);
        }
        for i in 0..g.rects.len() {
            glyf.u16((4 * i + 3) as u16);
        }
        glyf.u16(0);
        let n = 4 * g.rects.len();
        glyf.0.extend(core::iter::repeat_n(1u8, n));
        // Each rectangle clockwise: (x0,y0) (x0,y1) (x1,y1) (x1,y0).
        let pts: Vec<(i16, i16)> = g.rects.iter().flat_map(|&(a, b, c, d)| [(a, b), (a, d), (c, d), (c, b)]).collect();
        let mut prev = 0i16;
        for p in &pts {
            glyf.i16(p.0 - prev);
            prev = p.0;
        }
        prev = 0;
        for p in &pts {
            glyf.i16(p.1 - prev);
            prev = p.1;
        }
        glyf.pad4();
    }
    locs.push(glyf.0.len() as u32);
    if bx0 > bx1 {
        (bx0, by0, bx1, by1) = (0, 0, 0, 0);
    }
    let mut loca = Wr(Vec::new());
    for l in &locs {
        loca.u32(*l);
    }
    let mut hmtx = Wr(Vec::new());
    for g in &gl {
        hmtx.u16(g.adv);
        hmtx.i16(g.rects.iter().map(|r| r.0).min().unwrap_or(0));
    }
    // cmap format 4: one segment per run of consecutive code points.
    let mut segs: Vec<(u16, u16, u16)> = Vec::new(); // start, end, first glyph
    for (i, g) in gl.iter().enumerate().skip(1) {
        let cp = g.cp as u16;
        match segs.last_mut() {
            Some(s) if s.1.wrapping_add(1) == cp && s.2 as usize + (s.1 - s.0) as usize + 1 == i => s.1 = cp,
            _ => segs.push((cp, cp, i as u16)),
        }
    }
    segs.push((0xFFFF, 0xFFFF, 0));
    let nseg = segs.len();
    let mut cmap = Wr(Vec::new());
    for v in [0u16, 1, 3, 1] {
        cmap.u16(v);
    }
    cmap.u32(12);
    cmap.u16(4);
    cmap.u16((16 + 8 * nseg) as u16);
    cmap.u16(0);
    cmap.u16((nseg * 2) as u16);
    let mut pow = 1usize;
    let mut log = 0u16;
    while pow * 2 <= nseg {
        pow *= 2;
        log += 1;
    }
    cmap.u16((pow * 2) as u16);
    cmap.u16(log);
    cmap.u16((nseg * 2 - pow * 2) as u16);
    for s in &segs {
        cmap.u16(s.1);
    }
    cmap.u16(0);
    for s in &segs {
        cmap.u16(s.0);
    }
    for s in &segs {
        // idDelta maps the code point onto its glyph (the final segment 0xFFFF to glyph 0).
        cmap.u16(if s.0 == 0xFFFF { 1 } else { s.2.wrapping_sub(s.0) });
    }
    for _ in &segs {
        cmap.u16(0);
    }
    let avg = if mono { 8 * UNIT } else { 6 * UNIT };
    let mut os2 = Wr(Vec::new());
    os2.u16(4);
    os2.i16(avg);
    os2.u16(400);
    os2.u16(5);
    os2.u16(0);
    for v in [650i16, 600, 0, 75, 650, 600, 0, 350] {
        os2.i16(v);
    }
    os2.i16(UNIT);
    os2.i16(4 * UNIT);
    os2.i16(0);
    let mut pan = [0u8; 10];
    pan[0] = 2;
    pan[3] = if mono { 9 } else { 3 };
    os2.0.extend_from_slice(&pan);
    for v in [1u32, 0, 0, 0] {
        os2.u32(v);
    }
    os2.0.extend_from_slice(b"POLY");
    os2.u16(0x40);
    os2.u16(0x20);
    os2.u16(0xFFFF);
    os2.i16(12 * UNIT);
    os2.i16(-4 * UNIT);
    os2.i16(0);
    os2.u16(12 * UNIT as u16);
    os2.u16(4 * UNIT as u16);
    os2.u32(1);
    os2.u32(0);
    os2.i16(8 * UNIT);
    os2.i16(12 * UNIT);
    os2.u16(0);
    os2.u16(0x20);
    os2.u16(0);
    let mut head = Wr(Vec::new());
    for v in [0x0001_0000u32, 0x0001_0000, 0, 0x5F0F_3CF5] {
        head.u32(v);
    }
    head.u16(3);
    head.u16(1024);
    for v in [0u32, 0xD000_0000, 0, 0xD000_0000] {
        head.u32(v);
    }
    for v in [bx0, by0, bx1, by1] {
        head.i16(v);
    }
    head.u16(0);
    head.u16(8);
    head.i16(2);
    head.i16(1);
    head.i16(0);
    let mut hhea = Wr(Vec::new());
    hhea.u32(0x0001_0000);
    hhea.i16(12 * UNIT);
    hhea.i16(-4 * UNIT);
    hhea.i16(0);
    hhea.u16(gl.iter().map(|g| g.adv).max().unwrap_or(0));
    for v in [0i16, 0, 1024, 1, 0, 0, 0, 0, 0, 0, 0] {
        hhea.i16(v);
    }
    hhea.u16(ng as u16);
    let mut maxp = Wr(Vec::new());
    maxp.u32(0x0001_0000);
    maxp.u16(ng as u16);
    for v in [400u16, 100, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0] {
        maxp.u16(v);
    }
    let mut post = Wr(Vec::new());
    post.u32(0x0003_0000);
    post.u32(0);
    post.i16(-2 * UNIT);
    post.i16(UNIT);
    post.u32(u32::from(mono));
    for _ in 0..4 {
        post.u32(0);
    }
    let tables: [(&[u8; 4], Vec<u8>); 9] = [
        (b"OS/2", os2.0),
        (b"cmap", cmap.0),
        (b"glyf", glyf.0),
        (b"head", head.0),
        (b"hhea", hhea.0),
        (b"hmtx", hmtx.0),
        (b"loca", loca.0),
        (b"maxp", maxp.0),
        (b"post", post.0),
    ];
    let mut out = Wr(Vec::new());
    out.u32(0x0001_0000);
    out.u16(tables.len() as u16);
    out.u16(128);
    out.u16(3);
    out.u16(16);
    let mut off = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.0.extend_from_slice(*tag);
        out.u32(0);
        out.u32(off as u32);
        out.u32(data.len() as u32);
        off += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.0.extend_from_slice(data);
        out.pad4();
    }
    out.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_font::{Font, OutlineFormat};

    #[test]
    fn the_fonts_parse_and_have_spleens_shapes() {
        for mono in [false, true] {
            let data = build(mono);
            let f = Font::parse(&data).unwrap();
            assert_eq!(f.outline_format(), OutlineFormat::TrueType);
            assert_eq!(f.units_per_em(), 1024);
            let a = f.glyph_index('A').unwrap();
            assert!(f.outline(a).is_some());
            assert!(f.glyph_index(' ').is_some() && f.outline(f.glyph_index(' ').unwrap()).is_none());
            let gm = f.gdi_metrics();
            assert_eq!((gm.win_ascent, gm.win_descent), (768, 256));
            let (lo, hi) = f.cmap_range().unwrap();
            assert_eq!(lo, 0x20);
            assert!(hi >= 0x2500);
            let (i, w) = (f.glyph_index('i').unwrap(), f.glyph_index('W').unwrap());
            if mono {
                assert_eq!(f.advance(i), f.advance(w));
                assert!(f.style().monospace);
            } else {
                assert!(f.advance(i) < f.advance(w));
            }
        }
    }
}
