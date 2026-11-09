//! A small TrueType font built in code, for the unit tests: 3 glyphs
//! (`.notdef`, a space, a square), a format 4 `cmap` (and optionally
//! format 12) and the other tables DirectWrite reads.

use alloc::string::String;
use alloc::vec::Vec;

pub struct Spec {
    pub upem: u16,
    pub weight: u16,
    pub width: u16,
    pub fs_selection: u16,
    pub mac_style: u16,
    pub hhea: [i16; 3],
    pub typo: [i16; 3],
    pub win: [u16; 2],
    pub cap: i16,
    pub x: i16,
    pub os2_version: u16,
    pub names: Vec<(u16, u16, String)>,
    pub symbol: bool,
    pub format12: bool,
    pub strike: [i16; 2],
    pub underline: [i16; 2],
}

impl Default for Spec {
    fn default() -> Spec {
        Spec {
            upem: 1000,
            weight: 400,
            width: 5,
            fs_selection: 0x40,
            mac_style: 0,
            hhea: [800, -200, 90],
            typo: [750, -250, 100],
            win: [900, 300],
            cap: 700,
            x: 500,
            os2_version: 4,
            names: Vec::new(),
            symbol: false,
            format12: false,
            strike: [40, 250],
            underline: [-100, 50],
        }
    }
}

fn u16b(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn u32b(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// Build the font file.
pub fn build(s: &Spec) -> Vec<u8> {
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    // head
    let mut t = Vec::new();
    u32b(&mut t, 0x0001_0000);
    u32b(&mut t, 0x0001_0000);
    u32b(&mut t, 0);
    u32b(&mut t, 0x5F0F_3CF5);
    u16b(&mut t, 3);
    u16b(&mut t, s.upem);
    t.extend_from_slice(&[0; 16]);
    for v in [0i16, -200, 1000, 800] {
        u16b(&mut t, v as u16);
    }
    u16b(&mut t, s.mac_style);
    u16b(&mut t, 8);
    u16b(&mut t, 2);
    u16b(&mut t, 1); // long loca
    u16b(&mut t, 0);
    tables.push((*b"head", t));
    // hhea
    let mut t = Vec::new();
    u32b(&mut t, 0x0001_0000);
    for v in s.hhea {
        u16b(&mut t, v as u16);
    }
    t.extend_from_slice(&[0; 24]);
    u16b(&mut t, 3);
    tables.push((*b"hhea", t));
    // maxp
    let mut t = Vec::new();
    u32b(&mut t, 0x0001_0000);
    u16b(&mut t, 3);
    t.extend_from_slice(&[0; 26]);
    tables.push((*b"maxp", t));
    // hmtx: advances 500, 300, 1000
    let mut t = Vec::new();
    for (a, l) in [(500u16, 50i16), (300, 0), (1000, 100)] {
        u16b(&mut t, a);
        u16b(&mut t, l as u16);
    }
    tables.push((*b"hmtx", t));
    // glyf: glyph 0 empty, glyph 1 empty, glyph 2 a square 100..900 x 0..800
    let mut g = Vec::new();
    u16b(&mut g, 1);
    for v in [100i16, 0, 900, 800] {
        u16b(&mut g, v as u16);
    }
    u16b(&mut g, 3); // end point
    u16b(&mut g, 0); // no instructions
    g.extend_from_slice(&[1, 1, 1, 1]);
    for dx in [100i16, 0, 800, 0] {
        u16b(&mut g, dx as u16);
    }
    for dy in [0i16, 800, 0, -800] {
        u16b(&mut g, dy as u16);
    }
    while g.len() % 4 != 0 {
        g.push(0);
    }
    let mut loca = Vec::new();
    for off in [0u32, 0, 0, g.len() as u32] {
        u32b(&mut loca, off);
    }
    tables.push((*b"loca", loca));
    tables.push((*b"glyf", g));
    // cmap
    let mut sub4 = Vec::new();
    let codes: &[(u16, u16)] = if s.symbol { &[(0xF020, 1), (0xF041, 2)] } else { &[(0x20, 1), (0x41, 2)] };
    let segs = codes.len() + 1;
    u16b(&mut sub4, 4);
    u16b(&mut sub4, (16 + 8 * segs) as u16);
    u16b(&mut sub4, 0);
    u16b(&mut sub4, (segs * 2) as u16);
    u16b(&mut sub4, 4);
    u16b(&mut sub4, 1);
    u16b(&mut sub4, 0);
    for c in codes {
        u16b(&mut sub4, c.0);
    }
    u16b(&mut sub4, 0xFFFF);
    u16b(&mut sub4, 0);
    for c in codes {
        u16b(&mut sub4, c.0);
    }
    u16b(&mut sub4, 0xFFFF);
    for c in codes {
        u16b(&mut sub4, c.1.wrapping_sub(c.0));
    }
    u16b(&mut sub4, 1);
    for _ in 0..segs {
        u16b(&mut sub4, 0);
    }
    let mut sub12 = Vec::new();
    u16b(&mut sub12, 12);
    u16b(&mut sub12, 0);
    u32b(&mut sub12, 16 + 36);
    u32b(&mut sub12, 0);
    u32b(&mut sub12, 3);
    for (a, g) in [(0x20u32, 1u32), (0x41, 2), (0x1F600, 2)] {
        u32b(&mut sub12, a);
        u32b(&mut sub12, a);
        u32b(&mut sub12, g);
    }
    let mut t = Vec::new();
    u16b(&mut t, 0);
    u16b(&mut t, if s.format12 { 2 } else { 1 });
    let off4 = 4 + 8 * if s.format12 { 2 } else { 1 };
    u16b(&mut t, 3);
    u16b(&mut t, if s.symbol { 0 } else { 1 });
    u32b(&mut t, off4 as u32);
    if s.format12 {
        u16b(&mut t, 3);
        u16b(&mut t, 10);
        u32b(&mut t, (off4 + sub4.len()) as u32);
    }
    t.extend_from_slice(&sub4);
    if s.format12 {
        t.extend_from_slice(&sub12);
    }
    tables.push((*b"cmap", t));
    // OS/2 (version 4, 96 bytes)
    let mut t = Vec::new();
    u16b(&mut t, s.os2_version);
    u16b(&mut t, 500);
    u16b(&mut t, s.weight);
    u16b(&mut t, s.width);
    t.extend_from_slice(&[0; 2 + 16]);
    u16b(&mut t, s.strike[0] as u16);
    u16b(&mut t, s.strike[1] as u16);
    t.extend_from_slice(&[0; 2 + 10 + 16 + 4]);
    // now at offset 62: fsSelection
    u16b(&mut t, s.fs_selection);
    u16b(&mut t, 0x20);
    u16b(&mut t, 0xFFFF);
    for v in s.typo {
        u16b(&mut t, v as u16);
    }
    u16b(&mut t, s.win[0]);
    u16b(&mut t, s.win[1]);
    u32b(&mut t, 1);
    u32b(&mut t, 0);
    u16b(&mut t, s.x as u16);
    u16b(&mut t, s.cap as u16);
    u16b(&mut t, 0);
    u16b(&mut t, 0x20);
    u16b(&mut t, 0);
    tables.push((*b"OS/2", t));
    // post 3.0
    let mut t = Vec::new();
    u32b(&mut t, 0x0003_0000);
    u32b(&mut t, 0);
    u16b(&mut t, s.underline[0] as u16);
    u16b(&mut t, s.underline[1] as u16);
    t.extend_from_slice(&[0; 20]);
    tables.push((*b"post", t));
    // name (Windows, Unicode BMP)
    let mut strings = Vec::new();
    let mut t = Vec::new();
    u16b(&mut t, 0);
    u16b(&mut t, s.names.len() as u16);
    u16b(&mut t, (6 + 12 * s.names.len()) as u16);
    for (lang, id, text) in &s.names {
        let bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        for v in [3u16, 1, *lang, *id, bytes.len() as u16, strings.len() as u16] {
            u16b(&mut t, v);
        }
        strings.extend_from_slice(&bytes);
    }
    t.extend_from_slice(&strings);
    tables.push((*b"name", t));

    tables.sort_by_key(|t| t.0);
    let mut out = Vec::new();
    u32b(&mut out, 0x0001_0000);
    u16b(&mut out, tables.len() as u16);
    u16b(&mut out, 128);
    u16b(&mut out, 3);
    u16b(&mut out, 32);
    let mut off = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        u32b(&mut out, 0);
        u32b(&mut out, off as u32);
        u32b(&mut out, data.len() as u32);
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        off = 12 + 16 * tables.len() + body.len();
    }
    out.extend_from_slice(&body);
    out
}
