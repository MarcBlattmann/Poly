//! A small TrueType font built in code for the tests (unit tests, and
//! `tests/golden.rs` through `#[path]`), so that parsing,
//! rasterising and matching are tested without any third-party font.
//!
//! Units per em 1000, ascender 800, descender -200, line gap 100. Glyphs:
//!
//! | id | char | outline | advance |
//! |---|---|---|---|
//! | 0 | `.notdef` | a box with a box-shaped hole | 500 |
//! | 1 | `A` | a triangle (50,0) (300,700) (550,0) with a hole (200,150) (400,150) (300,450) | 600 |
//! | 2 | `O`, U+1F600 | outer contour of four off-curve points only (a rounded square), inner contour mixed | 700 |
//! | 3 | `Á` | composite: glyph 1, and glyph 5 scaled (1, 0.5) at (200, 750) | 600 |
//! | 4 | space | none | 250 |
//! | 5 | (accent) | a 200 × 100 rectangle | 250 (past `numberOfHMetrics`) |
//!
//! `kern` table: A then O is -40.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Style fields of a built font.
#[derive(Clone, Debug)]
pub struct Spec {
    pub family: &'static str,
    pub style: &'static str,
    /// Name ID 16/17 (typographic family and style), if any.
    pub typographic: Option<(&'static str, &'static str)>,
    pub weight: u16,
    pub width: u16,
    pub fs_selection: u16,
    pub mono: bool,
}

impl Default for Spec {
    fn default() -> Spec {
        Spec {
            family: "Poly Test",
            style: "Regular",
            typographic: None,
            weight: 400,
            width: 5,
            fs_selection: 0x40,
            mono: false,
        }
    }
}

fn be16(out: &mut Vec<u8>, v: &[i32]) {
    for &x in v {
        out.extend_from_slice(&(x as u16).to_be_bytes());
    }
}

fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// A simple glyph from contours of `(x, y, on_curve)`.
pub fn simple_glyph(contours: &[&[(i16, i16, bool)]]) -> Vec<u8> {
    let all: Vec<(i16, i16, bool)> = contours.iter().flat_map(|c| c.iter().copied()).collect();
    let xmin = all.iter().map(|p| p.0).min().unwrap_or(0);
    let xmax = all.iter().map(|p| p.0).max().unwrap_or(0);
    let ymin = all.iter().map(|p| p.1).min().unwrap_or(0);
    let ymax = all.iter().map(|p| p.1).max().unwrap_or(0);
    let mut out = Vec::new();
    be16(&mut out, &[contours.len() as i32, xmin.into(), ymin.into(), xmax.into(), ymax.into()]);
    let mut end = -1i32;
    for c in contours {
        end += c.len() as i32;
        be16(&mut out, &[end]);
    }
    be16(&mut out, &[0]); // no instructions
    let mut flags = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let (mut px, mut py) = (0i32, 0i32);
    for &(x, y, on) in &all {
        let (dx, dy) = (i32::from(x) - px, i32::from(y) - py);
        (px, py) = (x.into(), y.into());
        let mut f = u8::from(on);
        for (d, short, same, buf) in [(dx, 0x02, 0x10, &mut xs), (dy, 0x04, 0x20, &mut ys)] {
            if d == 0 {
                f |= same;
            } else if d.abs() <= 255 {
                f |= short;
                if d > 0 {
                    f |= same;
                }
                buf.push(d.unsigned_abs() as u8);
            } else {
                buf.extend_from_slice(&(d as i16).to_be_bytes());
            }
        }
        flags.push(f);
    }
    // Runs of equal flags use the repeat bit.
    let mut i = 0;
    while i < flags.len() {
        let f = flags[i];
        let mut run = 1;
        while i + run < flags.len() && flags[i + run] == f && run < 256 {
            run += 1;
        }
        if run > 1 {
            out.push(f | 0x08);
            out.push((run - 1) as u8);
        } else {
            out.push(f);
        }
        i += run;
    }
    out.extend(xs);
    out.extend(ys);
    out
}

/// The composite 'Á' (or, with `self_ref`, a glyph that includes itself).
fn composite(self_ref: bool) -> Vec<u8> {
    let mut out = Vec::new();
    be16(&mut out, &[-1, 50, 0, 550, 800]);
    // glyph 1 at (0, 0): words, xy values, more components.
    be16(&mut out, &[0x0001 | 0x0002 | 0x0020, 1, 0, 0]);
    // glyph 5 (or 3) at (200, 750), x scale 1.0, y scale 0.5.
    be16(&mut out, &[0x0001 | 0x0002 | 0x0040, if self_ref { 3 } else { 5 }, 200, 750, 0x4000, 0x2000]);
    out
}

/// The glyphs' data.
fn glyphs(self_ref: bool) -> Vec<Vec<u8>> {
    vec![
        simple_glyph(&[
            &[(50, 0, true), (50, 700, true), (450, 700, true), (450, 0, true)],
            &[(100, 50, true), (400, 50, true), (400, 650, true), (100, 650, true)],
        ]),
        simple_glyph(&[
            &[(50, 0, true), (300, 700, true), (550, 0, true)],
            &[(200, 150, true), (400, 150, true), (300, 450, true)],
        ]),
        simple_glyph(&[
            &[(50, 0, false), (50, 700, false), (650, 700, false), (650, 0, false)],
            &[(350, 100, true), (550, 100, false), (550, 600, false), (150, 600, false), (150, 100, false)],
        ]),
        composite(self_ref),
        Vec::new(),
        simple_glyph(&[&[(0, 0, true), (0, 100, true), (200, 100, true), (200, 0, true)]]),
    ]
}

/// The characters mapped, sorted.
const MAP: [(u32, u16); 5] = [(0x20, 4), (0x41, 1), (0x4f, 2), (0xc1, 3), (0x1f600, 2)];

/// A format 4 subtable for the BMP part of [`MAP`]: one segment per
/// character, alternately mapped by `idDelta` and through the glyph array.
pub fn cmap_format4() -> Vec<u8> {
    let bmp: Vec<(u32, u16)> = MAP.iter().copied().filter(|m| m.0 <= 0xffff).collect();
    let seg = bmp.len() + 1;
    let mut ends = Vec::new();
    let mut starts = Vec::new();
    let mut deltas = Vec::new();
    let mut ranges = Vec::new();
    let mut array = Vec::new();
    for (i, &(c, g)) in bmp.iter().enumerate() {
        ends.push(c as i32);
        starts.push(c as i32);
        if i % 2 == 0 {
            deltas.push((i32::from(g) - c as i32) & 0xffff);
            ranges.push(0);
        } else {
            deltas.push(0);
            ranges.push((2 * (seg - i) + 2 * array.len()) as i32);
            array.push(i32::from(g));
        }
    }
    ends.push(0xffff);
    starts.push(0xffff);
    deltas.push(1);
    ranges.push(0);
    let mut body = Vec::new();
    be16(&mut body, &[(seg * 2) as i32, 0, 0, 0]);
    be16(&mut body, &ends);
    be16(&mut body, &[0]);
    be16(&mut body, &starts);
    be16(&mut body, &deltas);
    be16(&mut body, &ranges);
    be16(&mut body, &array);
    let mut out = Vec::new();
    be16(&mut out, &[4, (6 + body.len()) as i32, 0]);
    out.extend(body);
    out
}

fn cmap() -> Vec<u8> {
    let f4 = cmap_format4();
    let mut f12 = Vec::new();
    be16(&mut f12, &[12, 0]);
    be32(&mut f12, 16 + 12 * MAP.len() as u32);
    be32(&mut f12, 0);
    be32(&mut f12, MAP.len() as u32);
    for (c, g) in MAP {
        be32(&mut f12, c);
        be32(&mut f12, c);
        be32(&mut f12, u32::from(g));
    }
    let mut out = Vec::new();
    be16(&mut out, &[0, 2, 3, 1]);
    be32(&mut out, 20);
    be16(&mut out, &[3, 10]);
    be32(&mut out, 20 + f4.len() as u32);
    out.extend(f4);
    out.extend(f12);
    out
}

fn name(spec: &Spec) -> Vec<u8> {
    let full = alloc::format!("{} {}", spec.family, spec.style);
    let ps: String = alloc::format!("{}-{}", spec.family, spec.style).chars().filter(|c| *c != ' ').collect();
    let mut names: Vec<(u16, u16, u16, u16, Vec<u8>)> = vec![
        (1, 0, 0, 1, b"PolyMac".to_vec()),
        (3, 1, 0x409, 1, utf16(spec.family)),
        (3, 1, 0x409, 2, utf16(spec.style)),
        (3, 1, 0x409, 4, utf16(&full)),
        (3, 1, 0x409, 6, utf16(&ps)),
    ];
    if let Some((f, s)) = spec.typographic {
        names.push((3, 1, 0x409, 16, utf16(f)));
        names.push((3, 1, 0x409, 17, utf16(s)));
    }
    let mut out = Vec::new();
    be16(&mut out, &[0, names.len() as i32, 6 + 12 * names.len() as i32]);
    let mut storage = Vec::new();
    for (p, e, l, id, s) in &names {
        be16(&mut out, &[(*p).into(), (*e).into(), (*l).into(), (*id).into(), s.len() as i32, storage.len() as i32]);
        storage.extend_from_slice(s);
    }
    out.extend(storage);
    out
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

fn os2(spec: &Spec) -> Vec<u8> {
    let mut o = vec![0u8; 96];
    o[0..2].copy_from_slice(&4u16.to_be_bytes());
    o[2..4].copy_from_slice(&500u16.to_be_bytes());
    o[4..6].copy_from_slice(&spec.weight.to_be_bytes());
    o[6..8].copy_from_slice(&spec.width.to_be_bytes());
    o[58..62].copy_from_slice(b"POLY");
    o[62..64].copy_from_slice(&(spec.fs_selection | 0x80).to_be_bytes());
    o[68..70].copy_from_slice(&800i16.to_be_bytes());
    o[70..72].copy_from_slice(&(-200i16).to_be_bytes());
    o[72..74].copy_from_slice(&100i16.to_be_bytes());
    o
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |acc, c| {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        acc.wrapping_add(u32::from_be_bytes(b))
    })
}

/// Assemble an sfnt from tables (tag, data).
pub fn sfnt(mut tables: Vec<([u8; 4], Vec<u8>)>, version: u32) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    let n = tables.len();
    let mut out = Vec::new();
    be32(&mut out, version);
    be16(&mut out, &[n as i32, 0, 0, 0]);
    let mut offset = 12 + 16 * n;
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        be32(&mut out, checksum(data));
        be32(&mut out, offset as u32);
        be32(&mut out, data.len() as u32);
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * n + body.len();
    }
    out.extend(body);
    out
}

fn build_spec(spec: &Spec, self_ref: bool) -> Vec<u8> {
    let glyphs = glyphs(self_ref);
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        be16(&mut loca, &[(glyf.len() / 2) as i32]);
        glyf.extend_from_slice(g);
        if glyf.len() % 2 == 1 {
            glyf.push(0);
        }
    }
    be16(&mut loca, &[(glyf.len() / 2) as i32]);
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, &[0, 1000]);
    head.extend_from_slice(&[0; 16]);
    let mac_style = i32::from(spec.fs_selection & 0x20 != 0) | (i32::from(spec.fs_selection & 1 != 0) << 1);
    be16(&mut head, &[0, -200, 700, 800, mac_style, 8, 2, 0, 0]);
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    be16(&mut hhea, &[800, -200, 100, 700, 0, 0, 650, 1, 0, 0, 0, 0, 0, 0, 0, 5]);
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0000_5000);
    be16(&mut maxp, &[glyphs.len() as i32]);
    let mut hmtx = Vec::new();
    be16(&mut hmtx, &[500, 50, 600, 50, 700, 50, 600, 50, 250, 0, 0]);
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, if spec.fs_selection & 1 != 0 { (-12i32 << 16) as u32 } else { 0 });
    be16(&mut post, &[-100, 50]);
    be32(&mut post, u32::from(spec.mono));
    post.extend_from_slice(&[0; 16]);
    let mut kern = Vec::new();
    be16(&mut kern, &[0, 1, 0, 20, 0x0001, 1, 6, 0, 0, 1, 2, -40]);
    sfnt(
        vec![
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"maxp", maxp),
            (*b"OS/2", os2(spec)),
            (*b"hmtx", hmtx),
            (*b"cmap", cmap()),
            (*b"loca", loca),
            (*b"glyf", glyf),
            (*b"name", name(spec)),
            (*b"post", post),
            (*b"kern", kern),
        ],
        0x0001_0000,
    )
}

/// The test font.
pub fn build() -> Vec<u8> {
    build_spec(&Spec::default(), false)
}

/// The test font with other names and style fields.
pub fn build_styled(spec: &Spec) -> Vec<u8> {
    build_spec(spec, false)
}

/// The test font whose glyph 3 contains itself.
pub fn build_with_self_reference() -> Vec<u8> {
    build_spec(&Spec::default(), true)
}

/// A collection (`.ttc`) of the given fonts.
pub fn collection(fonts: &[Vec<u8>]) -> Vec<u8> {
    let header = 12 + 4 * fonts.len();
    let mut out = Vec::from(&b"ttcf"[..]);
    be32(&mut out, 0x0001_0000);
    be32(&mut out, fonts.len() as u32);
    let mut offset = header;
    for f in fonts {
        be32(&mut out, offset as u32);
        offset += f.len();
    }
    // Each font's table offsets are relative to its own start in this
    // builder, so shift them.
    for f in fonts {
        let base = out.len() as u32;
        let mut f = f.clone();
        let n = usize::from(u16::from_be_bytes([f[4], f[5]]));
        for i in 0..n {
            let at = 12 + 16 * i + 8;
            let off = u32::from_be_bytes([f[at], f[at + 1], f[at + 2], f[at + 3]]) + base;
            f[at..at + 4].copy_from_slice(&off.to_be_bytes());
        }
        out.extend(f);
    }
    out
}
