//! CFF outlines (OpenType fonts whose `sfntVersion` is `OTTO`): the
//! Compact Font Format (Adobe Technical Note #5176) and its Type 2
//! charstrings (Technical Note #5177), as the OpenType specification's
//! "CFF — Compact Font Format table" chapter references them.
//!
//! Only what drawing a glyph needs: the header, the Name, Top DICT,
//! String and Global Subr INDEXes, the Top DICT's `CharStrings`,
//! `Private`, and for CID-keyed fonts `FDArray` and `FDSelect` (formats 0
//! and 3), the Private DICT's `Subrs`, and every Type 2 path, subroutine
//! and hint operator (hints are skipped: the rasteriser is hinting-free).
//! CFF2 (variable fonts) is not read.

use crate::font::GlyphId;
use crate::path::{Path, Point};
use crate::reader::{Reader, u16_at};
use alloc::vec::Vec;

/// An INDEX: `count` objects whose bytes follow an offset array.
#[derive(Clone, Copy, Debug, Default)]
struct Index<'a> {
    count: usize,
    off_size: u8,
    offsets: &'a [u8],
    data: &'a [u8],
}

impl<'a> Index<'a> {
    /// Parse the INDEX at the reader; leaves the reader after it.
    fn parse(r: &mut Reader<'a>) -> Option<Index<'a>> {
        let count = usize::from(r.u16()?);
        if count == 0 {
            return Some(Index::default());
        }
        let off_size = r.u8()?;
        if !(1..=4).contains(&off_size) {
            return None;
        }
        let offsets = r.bytes((count + 1) * usize::from(off_size))?;
        let mut idx = Index { count, off_size, offsets, data: &[] };
        let last = idx.offset(count)?;
        idx.data = r.bytes(last.checked_sub(1)?)?;
        Some(idx)
    }

    fn offset(&self, i: usize) -> Option<usize> {
        let size = usize::from(self.off_size);
        let b = self.offsets.get(i * size..(i + 1) * size)?;
        Some(b.iter().fold(0usize, |acc, &x| (acc << 8) | usize::from(x)))
    }

    fn get(&self, i: usize) -> Option<&'a [u8]> {
        if i >= self.count {
            return None;
        }
        let (start, end) = (self.offset(i)?.checked_sub(1)?, self.offset(i + 1)?.checked_sub(1)?);
        self.data.get(start..end)
    }
}

/// A DICT operand.
fn dict_operand(r: &mut Reader<'_>, b0: u8) -> Option<f32> {
    Some(match b0 {
        28 => f32::from(r.i16()?),
        29 => r.i32()? as f32,
        30 => {
            // A real number in nibbles; its value is not needed by any
            // operator read here, so it is skipped.
            loop {
                let b = r.u8()?;
                if b & 0x0f == 0x0f || b >> 4 == 0x0f {
                    break;
                }
            }
            0.0
        }
        32..=246 => f32::from(i16::from(b0) - 139),
        247..=250 => f32::from((i16::from(b0) - 247) * 256 + i16::from(r.u8()?) + 108),
        251..=254 => f32::from(-(i16::from(b0) - 251) * 256 - i16::from(r.u8()?) - 108),
        _ => return None,
    })
}

/// The operands of `op` (12 xx is `0x0c00 | xx`) in a DICT, if present.
fn dict_get(dict: &[u8], op: u16) -> Option<Vec<f32>> {
    let mut r = Reader::new(dict);
    let mut operands = Vec::new();
    while let Some(b0) = r.u8() {
        if b0 <= 21 {
            let key = if b0 == 12 { 0x0c00 | u16::from(r.u8()?) } else { u16::from(b0) };
            if key == op {
                return Some(operands);
            }
            operands.clear();
        } else {
            operands.push(dict_operand(&mut r, b0)?);
            if operands.len() > 48 {
                return None;
            }
        }
    }
    None
}

const OP_CHARSTRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_SUBRS: u16 = 19;
const OP_CHARSTRING_TYPE: u16 = 0x0c06;
const OP_FDARRAY: u16 = 0x0c24;
const OP_FDSELECT: u16 = 0x0c25;

/// The pieces of a CFF table a glyph needs.
struct Cff<'a> {
    charstrings: Index<'a>,
    global_subrs: Index<'a>,
    /// The local subroutines of each Font DICT (one for a non-CID font).
    local_subrs: Vec<Index<'a>>,
    /// CID fonts: `FDSelect`.
    fd_select: Option<&'a [u8]>,
}

/// A Private DICT's local subroutines (`Subrs`, relative to the DICT).
fn private_subrs<'a>(cff: &'a [u8], font_dict: &[u8]) -> Option<Index<'a>> {
    let Some(p) = dict_get(font_dict, OP_PRIVATE) else { return Some(Index::default()) };
    let (size, offset) = (*p.first()? as usize, *p.get(1)? as usize);
    let private = cff.get(offset..offset.checked_add(size)?)?;
    match dict_get(private, OP_SUBRS) {
        Some(s) => Index::parse(&mut Reader::at(cff, offset.checked_add(*s.first()? as usize)?)?),
        None => Some(Index::default()),
    }
}

impl<'a> Cff<'a> {
    fn parse(cff: &'a [u8]) -> Option<Cff<'a>> {
        let header_size = usize::from(*cff.get(2)?);
        let mut r = Reader::at(cff, header_size)?;
        let _names = Index::parse(&mut r)?;
        let top_dicts = Index::parse(&mut r)?;
        let _strings = Index::parse(&mut r)?;
        let global_subrs = Index::parse(&mut r)?;
        let top = top_dicts.get(0)?;
        if dict_get(top, OP_CHARSTRING_TYPE).is_some_and(|t| t.first() != Some(&2.0)) {
            return None;
        }
        let cs_offset = *dict_get(top, OP_CHARSTRINGS)?.first()? as usize;
        let charstrings = Index::parse(&mut Reader::at(cff, cs_offset)?)?;
        let mut local_subrs = Vec::new();
        let mut fd_select = None;
        if let Some(fd) = dict_get(top, OP_FDARRAY) {
            let fd_array = Index::parse(&mut Reader::at(cff, *fd.first()? as usize)?)?;
            for i in 0..fd_array.count.min(256) {
                local_subrs.push(private_subrs(cff, fd_array.get(i)?)?);
            }
            fd_select = Some(cff.get(*dict_get(top, OP_FDSELECT)?.first()? as usize..)?);
        } else {
            local_subrs.push(private_subrs(cff, top)?);
        }
        Some(Cff { charstrings, global_subrs, local_subrs, fd_select })
    }

    /// Which Font DICT a glyph uses.
    fn font_dict(&self, glyph: u16) -> Option<usize> {
        let Some(sel) = self.fd_select else { return Some(0) };
        match *sel.first()? {
            0 => sel.get(1 + usize::from(glyph)).map(|&fd| usize::from(fd)),
            3 => {
                let n = usize::from(u16_at(sel, 1)?);
                for i in 0..n {
                    let at = 3 + 3 * i;
                    let (first, fd, next) = (u16_at(sel, at)?, *sel.get(at + 2)?, u16_at(sel, at + 3)?);
                    if glyph >= first && glyph < next {
                        return Some(usize::from(fd));
                    }
                }
                None
            }
            _ => None,
        }
    }
}

/// The bias added to a subroutine number (it depends on how many there are).
fn bias(subrs: &Index<'_>) -> i32 {
    match subrs.count {
        0..1240 => 107,
        1240..33900 => 1131,
        _ => 32768,
    }
}

/// A glyph's outline from a `CFF ` table, in font units.
pub(crate) fn outline(cff: &[u8], glyph: GlyphId) -> Option<Path> {
    let c = Cff::parse(cff)?;
    let cs = c.charstrings.get(usize::from(glyph.0))?;
    let local = *c.local_subrs.get(c.font_dict(glyph.0)?)?;
    let mut m =
        Machine { path: Path::new(), stack: Vec::new(), x: 0.0, y: 0.0, open: false, stems: 0, width_done: false };
    m.run(cs, &c.global_subrs, &local, 0)?;
    if m.open {
        m.path.close();
    }
    Some(m.path)
}

/// Type 2 charstring interpreter state.
struct Machine {
    path: Path,
    stack: Vec<f32>,
    x: f32,
    y: f32,
    open: bool,
    stems: usize,
    /// Whether the optional width operand before the first stack-clearing
    /// operator has been dealt with.
    width_done: bool,
}

/// The argument stack limit and the subroutine nesting limit (TN #5177).
const MAX_STACK: usize = 48;
const MAX_CALL_DEPTH: u32 = 10;

/// What [`Machine::run`] stopped at.
#[derive(PartialEq)]
enum Stop {
    Return,
    EndChar,
}

impl Machine {
    /// Drop the width operand if the first stack-clearing operator got one
    /// more operand than it takes (`even`: whether it takes an even count).
    fn take_width(&mut self, even: bool) {
        if !self.width_done {
            self.width_done = true;
            if (self.stack.len() % 2 == 1) == even && !self.stack.is_empty() {
                self.stack.remove(0);
            }
        }
    }

    fn move_to(&mut self, dx: f32, dy: f32) {
        if self.open {
            self.path.close();
        }
        self.x += dx;
        self.y += dy;
        self.path.move_to(Point::new(self.x, self.y));
        self.open = true;
    }

    fn line_to(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
        self.path.line_to(Point::new(self.x, self.y));
    }

    #[allow(clippy::too_many_arguments)]
    fn curve_to(&mut self, dx1: f32, dy1: f32, dx2: f32, dy2: f32, dx3: f32, dy3: f32) {
        let c1 = Point::new(self.x + dx1, self.y + dy1);
        let c2 = Point::new(c1.x + dx2, c1.y + dy2);
        let p = Point::new(c2.x + dx3, c2.y + dy3);
        self.path.cubic_to(c1, c2, p);
        self.x = p.x;
        self.y = p.y;
    }

    fn run(&mut self, code: &[u8], global: &Index<'_>, local: &Index<'_>, depth: u32) -> Option<Stop> {
        if depth > MAX_CALL_DEPTH {
            return None;
        }
        let mut r = Reader::new(code);
        while let Some(b0) = r.u8() {
            match b0 {
                32..=246 => self.push(f32::from(i16::from(b0) - 139))?,
                247..=250 => self.push(f32::from((i16::from(b0) - 247) * 256 + i16::from(r.u8()?) + 108))?,
                251..=254 => self.push(f32::from(-(i16::from(b0) - 251) * 256 - i16::from(r.u8()?) - 108))?,
                28 => self.push(f32::from(r.i16()?))?,
                255 => self.push(r.fixed()?)?,
                // Stem hints: counted (hintmask's size depends on them), skipped.
                1 | 3 | 18 | 23 => {
                    self.take_width(true);
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                }
                19 | 20 => {
                    // hintmask/cntrmask: leftover operands are vstems.
                    self.take_width(true);
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    r.skip(self.stems.div_ceil(8))?;
                }
                21 => {
                    self.take_width(true);
                    let s = self.args(2)?;
                    self.move_to(s[0], s[1]);
                }
                22 => {
                    self.take_width(false);
                    let s = self.args(1)?;
                    self.move_to(s[0], 0.0);
                }
                4 => {
                    self.take_width(false);
                    let s = self.args(1)?;
                    self.move_to(0.0, s[0]);
                }
                5 => {
                    let s = core::mem::take(&mut self.stack);
                    for p in s.chunks_exact(2) {
                        self.line_to(p[0], p[1]);
                    }
                }
                6 | 7 => {
                    let s = core::mem::take(&mut self.stack);
                    let mut horizontal = b0 == 6;
                    for &d in &s {
                        if horizontal {
                            self.line_to(d, 0.0)
                        } else {
                            self.line_to(0.0, d)
                        }
                        horizontal = !horizontal;
                    }
                }
                8 => {
                    let s = core::mem::take(&mut self.stack);
                    for p in s.chunks_exact(6) {
                        self.curve_to(p[0], p[1], p[2], p[3], p[4], p[5]);
                    }
                }
                24 => {
                    // rcurveline: curves, then a line.
                    let s = core::mem::take(&mut self.stack);
                    if s.len() < 8 {
                        return None;
                    }
                    let (curves, line) = s.split_at(s.len() - 2);
                    for p in curves.chunks_exact(6) {
                        self.curve_to(p[0], p[1], p[2], p[3], p[4], p[5]);
                    }
                    self.line_to(line[0], line[1]);
                }
                25 => {
                    // rlinecurve: lines, then a curve.
                    let s = core::mem::take(&mut self.stack);
                    if s.len() < 8 {
                        return None;
                    }
                    let (lines, p) = s.split_at(s.len() - 6);
                    for l in lines.chunks_exact(2) {
                        self.line_to(l[0], l[1]);
                    }
                    self.curve_to(p[0], p[1], p[2], p[3], p[4], p[5]);
                }
                26 | 27 => {
                    // vvcurveto / hhcurveto: an optional odd first operand.
                    let s = core::mem::take(&mut self.stack);
                    let (mut extra, rest) = if s.len() % 4 == 1 { (s[0], &s[1..]) } else { (0.0, &s[..]) };
                    for p in rest.chunks_exact(4) {
                        if b0 == 26 {
                            self.curve_to(extra, p[0], p[1], p[2], 0.0, p[3]);
                        } else {
                            self.curve_to(p[0], extra, p[1], p[2], p[3], 0.0);
                        }
                        extra = 0.0;
                    }
                }
                30 | 31 => {
                    // vhcurveto / hvcurveto: alternating start tangents, an
                    // optional last operand for the final curve's end.
                    let s = core::mem::take(&mut self.stack);
                    let mut horizontal = b0 == 31;
                    let mut i = 0;
                    while i + 4 <= s.len() {
                        let last = if s.len() - (i + 4) == 1 { s[i + 4] } else { 0.0 };
                        if horizontal {
                            self.curve_to(s[i], 0.0, s[i + 1], s[i + 2], last, s[i + 3]);
                        } else {
                            self.curve_to(0.0, s[i], s[i + 1], s[i + 2], s[i + 3], last);
                        }
                        horizontal = !horizontal;
                        i += 4;
                    }
                }
                10 | 29 => {
                    let n = self.stack.pop()? as i32;
                    let subrs = if b0 == 10 { local } else { global };
                    let sub = subrs.get(usize::try_from(n.checked_add(bias(subrs))?).ok()?)?;
                    if self.run(sub, global, local, depth + 1)? == Stop::EndChar {
                        return Some(Stop::EndChar);
                    }
                }
                11 => return Some(Stop::Return),
                14 => {
                    // endchar (the seac accent form with 4 operands is not
                    // supported: it draws the base glyph only).
                    self.width_done = true;
                    self.stack.clear();
                    return Some(Stop::EndChar);
                }
                12 => {
                    let op = r.u8()?;
                    let s = core::mem::take(&mut self.stack);
                    match op {
                        // flex: two curves.
                        35 if s.len() >= 13 => {
                            self.curve_to(s[0], s[1], s[2], s[3], s[4], s[5]);
                            self.curve_to(s[6], s[7], s[8], s[9], s[10], s[11]);
                        }
                        // hflex
                        34 if s.len() >= 7 => {
                            self.curve_to(s[0], 0.0, s[1], s[2], s[3], 0.0);
                            self.curve_to(s[4], 0.0, s[5], -s[2], s[6], 0.0);
                        }
                        // hflex1
                        36 if s.len() >= 9 => {
                            let y0 = self.y;
                            self.curve_to(s[0], s[1], s[2], s[3], s[4], 0.0);
                            self.curve_to(s[5], 0.0, s[6], s[7], s[8], y0 - (self.y + s[7]));
                        }
                        // flex1: the last point moves along the longer axis.
                        37 if s.len() >= 11 => {
                            let (x0, y0) = (self.x, self.y);
                            let dx: f32 = s[0] + s[2] + s[4] + s[6] + s[8];
                            let dy: f32 = s[1] + s[3] + s[5] + s[7] + s[9];
                            self.curve_to(s[0], s[1], s[2], s[3], s[4], s[5]);
                            let (lx, ly) = if dx.abs() > dy.abs() {
                                (s[10], y0 - (self.y + s[7] + s[9]))
                            } else {
                                (x0 - (self.x + s[6] + s[8]), s[10])
                            };
                            self.curve_to(s[6], s[7], s[8], s[9], lx, ly);
                        }
                        // Arithmetic and storage operators are deprecated
                        // or rare in fonts; their operands are dropped.
                        _ => {}
                    }
                }
                _ => return None,
            }
        }
        Some(Stop::Return)
    }

    fn push(&mut self, v: f32) -> Option<()> {
        if self.stack.len() >= MAX_STACK {
            return None;
        }
        self.stack.push(v);
        Some(())
    }

    /// The last `n` operands (an operator taking `n` operands uses the top
    /// of the stack); clears the stack.
    fn args(&mut self, n: usize) -> Option<Vec<f32>> {
        let s = core::mem::take(&mut self.stack);
        (s.len() >= n).then(|| s[s.len() - n..].to_vec())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::path::Segment;
    use alloc::vec;

    /// A CFF INDEX of `items` with 1- or 2-byte offsets.
    pub fn index(items: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(items.len() as u16).to_be_bytes());
        if items.is_empty() {
            return out;
        }
        let total: usize = items.iter().map(|i| i.len()).sum::<usize>() + 1;
        let off_size: u8 = if total < 256 { 1 } else { 2 };
        out.push(off_size);
        let mut off = 1usize;
        let push = |out: &mut Vec<u8>, v: usize| {
            if off_size == 1 { out.push(v as u8) } else { out.extend_from_slice(&(v as u16).to_be_bytes()) }
        };
        push(&mut out, off);
        for i in items {
            off += i.len();
            push(&mut out, off);
        }
        for i in items {
            out.extend_from_slice(i);
        }
        out
    }

    /// A DICT integer operand (always the 5-byte form, so sizes are fixed).
    fn int(v: i32) -> Vec<u8> {
        let mut o = vec![29];
        o.extend_from_slice(&v.to_be_bytes());
        o
    }

    /// A Type 2 number (always the 3-byte `28` form).
    fn n(v: i16) -> Vec<u8> {
        let mut o = vec![28];
        o.extend_from_slice(&v.to_be_bytes());
        o
    }

    /// A small CFF table: glyph 0 empty, glyph 1 a square drawn with a
    /// width operand, rlineto and a local subroutine, glyph 2 a curve
    /// through hvcurveto and a global subroutine.
    pub fn build() -> Vec<u8> {
        let mut g1 = Vec::new();
        // width 500, then rmoveto 100 0.
        g1.extend(n(500));
        g1.extend(n(100));
        g1.extend(n(0));
        g1.push(21);
        // callsubr 0 (biased: -107).
        g1.extend(n(-107));
        g1.push(10);
        g1.push(14);
        let local0 = {
            let mut s = Vec::new();
            for (dx, dy) in [(400, 0), (0, 400), (-400, 0)] {
                s.extend(n(dx));
                s.extend(n(dy));
            }
            s.push(5);
            s.push(11);
            s
        };
        let mut g2 = Vec::new();
        g2.extend(n(0));
        g2.extend(n(0));
        g2.push(21);
        g2.extend(n(-107));
        g2.push(29);
        g2.push(14);
        let global0 = {
            let mut s = Vec::new();
            // hvcurveto dx1 dx2 dy2 dy3: from (0,0) right then up.
            for v in [100, 100, 100, 100] {
                s.extend(n(v));
            }
            s.push(31);
            s.push(11);
            s
        };
        let charstrings = index(&[&[14], &g1, &g2]);
        let gsubrs = index(&[&global0]);
        let lsubrs = index(&[&local0]);
        // Layout: header(4) names top strings gsubrs charstrings private lsubrs.
        let names = index(&[b"PolyCff"]);
        let strings = index(&[]);
        // The Top DICT has a fixed size: CharStrings (6 bytes), Private (11).
        let top_len = 6 + 11;
        let top_index_len = 2 + 1 + 2 + top_len;
        let cs_off = 4 + names.len() + top_index_len + strings.len() + gsubrs.len();
        let private = {
            let mut p = int(6); // Subrs offset, relative to the Private DICT
            p.push(19);
            p
        };
        let private_off = cs_off + charstrings.len();
        let mut top = int(cs_off as i32);
        top.push(17);
        top.extend(int(private.len() as i32));
        top.extend(int(private_off as i32));
        top.push(18);
        assert_eq!(top.len(), top_len);
        let mut out = vec![1, 0, 4, 1];
        out.extend(names);
        out.extend(index(&[&top]));
        out.extend(strings);
        out.extend(gsubrs);
        assert_eq!(out.len(), cs_off);
        out.extend(charstrings);
        out.extend(private);
        out.extend(lsubrs);
        out
    }

    #[test]
    fn draws_type2_charstrings() {
        let cff = build();
        let p = outline(&cff, GlyphId(1)).unwrap();
        assert_eq!(
            p.segments,
            vec![
                Segment::MoveTo(Point::new(100.0, 0.0)),
                Segment::LineTo(Point::new(500.0, 0.0)),
                Segment::LineTo(Point::new(500.0, 400.0)),
                Segment::LineTo(Point::new(100.0, 400.0)),
                Segment::Close,
            ]
        );
        let p = outline(&cff, GlyphId(2)).unwrap();
        assert_eq!(
            p.segments,
            vec![
                Segment::MoveTo(Point::new(0.0, 0.0)),
                Segment::CubicTo(Point::new(100.0, 0.0), Point::new(200.0, 100.0), Point::new(200.0, 200.0)),
                Segment::Close,
            ]
        );
        assert_eq!(outline(&cff, GlyphId(0)).unwrap().segments, vec![]);
        assert!(outline(&cff, GlyphId(3)).is_none());
        // Truncated tables fail cleanly.
        for len in 0..cff.len() {
            let _ = outline(&cff[..len], GlyphId(1));
            let _ = outline(&cff[..len], GlyphId(2));
        }
    }
}
