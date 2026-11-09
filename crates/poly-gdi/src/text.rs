//! `DrawTextW`'s layout (`DrawTextW.yaml`): prefixes, lines, word wrap,
//! tabs, alignment and the rectangle `DT_CALCRECT` reports.

use crate::font::Face;
use crate::gdi::{Gdi, Rect};
use alloc::vec::Vec;

pub const DT_CENTER: u32 = 0x0001;
pub const DT_RIGHT: u32 = 0x0002;
pub const DT_VCENTER: u32 = 0x0004;
pub const DT_BOTTOM: u32 = 0x0008;
pub const DT_WORDBREAK: u32 = 0x0010;
pub const DT_SINGLELINE: u32 = 0x0020;
pub const DT_EXPANDTABS: u32 = 0x0040;
pub const DT_NOCLIP: u32 = 0x0100;
pub const DT_CALCRECT: u32 = 0x0400;
pub const DT_NOPREFIX: u32 = 0x0800;

const AMP: u16 = b'&' as u16;
const TAB: u16 = 9;
const LF: u16 = 10;
const CR: u16 = 13;
const SPACE: u16 = 32;

/// `"&x"` shows `x`, `"&&"` shows `&` (no underline: keyboard cues are
/// hidden).
pub fn strip_prefixes(s: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == AMP && i + 1 < s.len() {
            out.push(s[i + 1]);
            i += 2;
        } else if s[i] == AMP {
            i += 1;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// The width of a line, tabs expanded to multiples of eight average
/// character widths when asked.
pub fn line_width(face: Face, s: &[u16], expand_tabs: bool) -> i32 {
    let stop = 8 * face.metrics().ave_char_width;
    let mut x = 0;
    for &c in s {
        if c == TAB && expand_tabs {
            x = (x / stop + 1) * stop;
        } else {
            x += face.width(c);
        }
    }
    x
}

/// The lines of `s` for `format` in a rectangle `width` wide.
pub fn lines(face: Face, s: &[u16], format: u32, width: i32) -> Vec<Vec<u16>> {
    let tabs = format & DT_EXPANDTABS != 0;
    if format & DT_SINGLELINE != 0 {
        return alloc::vec![s.to_vec()];
    }
    let mut out = Vec::new();
    for para in s.split(|&c| c == LF) {
        let para = para.strip_suffix(&[CR]).unwrap_or(para);
        if format & DT_WORDBREAK == 0 {
            out.push(para.to_vec());
            continue;
        }
        let mut line: Vec<u16> = Vec::new();
        for word in para.split(|&c| c == SPACE).filter(|w| !w.is_empty()) {
            if line.is_empty() {
                line.extend_from_slice(word);
                continue;
            }
            let mut candidate = line.clone();
            candidate.push(SPACE);
            candidate.extend_from_slice(word);
            if line_width(face, &candidate, tabs) <= width {
                line = candidate;
            } else {
                out.push(core::mem::replace(&mut line, word.to_vec()));
            }
        }
        out.push(line);
    }
    out
}

impl Gdi {
    /// `DrawTextW`: the height drawn (see the sheet for `DT_VCENTER` and
    /// `DT_BOTTOM`); `rect` changes only with `DT_CALCRECT`.
    pub fn draw_text(&mut self, dc: u64, text: &[u16], rect: &mut Rect, format: u32) -> i32 {
        let Some(tm) = self.text_metrics(dc) else { return 0 };
        let face = self.face_of(dc);
        let h = tm.height;
        let t = if format & DT_NOPREFIX == 0 { strip_prefixes(text) } else { text.to_vec() };
        let single = format & DT_SINGLELINE != 0;
        if t.is_empty() && !single {
            if format & DT_CALCRECT != 0 {
                rect.right = rect.left;
                rect.bottom = rect.top;
            }
            return 1;
        }
        let tabs = format & DT_EXPANDTABS != 0;
        let ls = lines(face, &t, format, rect.right - rect.left);
        let widths: Vec<i32> = ls.iter().map(|l| line_width(face, l, tabs)).collect();
        let total = h * ls.len() as i32;
        if format & DT_CALCRECT != 0 {
            rect.right = rect.left + widths.iter().copied().max().unwrap_or(0);
            rect.bottom = rect.top + total;
            return total;
        }
        let mut y = rect.top;
        if single && format & DT_VCENTER != 0 {
            y = rect.top + (rect.bottom - rect.top - h) / 2;
        } else if single && format & DT_BOTTOM != 0 {
            y = rect.bottom - h;
        }
        let clip = if format & DT_NOCLIP != 0 { None } else { Some(*rect) };
        for (l, w) in ls.iter().zip(&widths) {
            let x = if format & DT_RIGHT != 0 {
                rect.right - w
            } else if format & DT_CENTER != 0 {
                rect.left + (rect.right - rect.left - w) / 2
            } else {
                rect.left
            };
            if tabs && l.contains(&TAB) {
                let stop = 8 * tm.ave_char_width;
                let mut cx = 0;
                for seg in l.split(|&c| c == TAB) {
                    self.draw_text_run(dc, x + cx, y, seg, clip);
                    cx += line_width(face, seg, false);
                    cx = (cx / stop + 1) * stop;
                }
            } else {
                self.draw_text_run(dc, x, y, l, clip);
            }
            y += h;
        }
        if single && format & (DT_VCENTER | DT_BOTTOM) != 0 { y - rect.top } else { total }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn calc(g: &mut Gdi, dc: u64, s: &str, width: i32, fmt: u32) -> (i32, Rect) {
        let mut r = Rect::new(10, 20, 10 + width, 20);
        let n = g.draw_text(dc, &w(s), &mut r, fmt | DT_CALCRECT);
        (n, r)
    }

    #[test]
    fn calcrect_matches_the_sheet() {
        let mut g = Gdi::new();
        let dc = u64::from(g.create_compatible_dc(0));
        assert_eq!(calc(&mut g, dc, "Hello", 0, DT_SINGLELINE), (16, Rect::new(10, 20, 44, 36)));
        assert_eq!(calc(&mut g, dc, "&File", 0, DT_SINGLELINE).1.right, 34);
        assert_eq!(calc(&mut g, dc, "&File", 0, DT_SINGLELINE | DT_NOPREFIX).1.right, 43);
        assert_eq!(calc(&mut g, dc, "A&&B", 0, DT_SINGLELINE).1.right, 37);
        assert_eq!(calc(&mut g, dc, "one\ntwo", 0, 0), (32, Rect::new(10, 20, 34, 52)));
        assert_eq!(calc(&mut g, dc, "one\ntwo", 0, DT_SINGLELINE).1.right, 56);
        assert_eq!(calc(&mut g, dc, "the quick brown fox jumps", 60, DT_WORDBREAK), (64, Rect::new(10, 20, 68, 84)));
        assert_eq!(calc(&mut g, dc, "the quick brown fox jumps", 1000, DT_WORDBREAK).1.right, 179);
        assert_eq!(calc(&mut g, dc, "abcdefghij", 20, DT_WORDBREAK), (16, Rect::new(10, 20, 77, 36)));
        assert_eq!(calc(&mut g, dc, "a\tb", 0, DT_SINGLELINE).1.right, 26);
        assert_eq!(calc(&mut g, dc, "a\tb", 0, DT_SINGLELINE | DT_EXPANDTABS).1.right, 74);
        assert_eq!(calc(&mut g, dc, "", 0, DT_SINGLELINE), (16, Rect::new(10, 20, 10, 36)));
        assert_eq!(calc(&mut g, dc, "", 0, 0), (1, Rect::new(10, 20, 10, 20)));
    }
}
