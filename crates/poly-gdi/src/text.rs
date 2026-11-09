//! `DrawTextW`'s layout (`DrawTextW.yaml`, and the Microsoft Learn reference
//! of `DrawText`/`DrawTextEx` for the flags the sheet does not cover):
//! prefixes, lines, word wrap, tabs, ellipses, alignment and the rectangle
//! `DT_CALCRECT` reports.

use crate::gdi::{Gdi, Rect};
use crate::outline::Typeface;
use alloc::vec::Vec;

pub const DT_TOP: u32 = 0x0000;
pub const DT_LEFT: u32 = 0x0000;
pub const DT_CENTER: u32 = 0x0001;
pub const DT_RIGHT: u32 = 0x0002;
pub const DT_VCENTER: u32 = 0x0004;
pub const DT_BOTTOM: u32 = 0x0008;
pub const DT_WORDBREAK: u32 = 0x0010;
pub const DT_SINGLELINE: u32 = 0x0020;
pub const DT_EXPANDTABS: u32 = 0x0040;
pub const DT_TABSTOP: u32 = 0x0080;
pub const DT_NOCLIP: u32 = 0x0100;
pub const DT_EXTERNALLEADING: u32 = 0x0200;
pub const DT_CALCRECT: u32 = 0x0400;
pub const DT_NOPREFIX: u32 = 0x0800;
pub const DT_INTERNAL: u32 = 0x1000;
pub const DT_EDITCONTROL: u32 = 0x2000;
pub const DT_PATH_ELLIPSIS: u32 = 0x4000;
pub const DT_END_ELLIPSIS: u32 = 0x8000;
pub const DT_MODIFYSTRING: u32 = 0x1_0000;
pub const DT_RTLREADING: u32 = 0x2_0000;
pub const DT_WORD_ELLIPSIS: u32 = 0x4_0000;
pub const DT_NOFULLWIDTHCHARBREAK: u32 = 0x8_0000;
pub const DT_HIDEPREFIX: u32 = 0x10_0000;
pub const DT_PREFIXONLY: u32 = 0x20_0000;

const ELLIPSES: u32 = DT_PATH_ELLIPSIS | DT_END_ELLIPSIS | DT_WORD_ELLIPSIS;

const AMP: u16 = b'&' as u16;
const TAB: u16 = 9;
const LF: u16 = 10;
const CR: u16 = 13;
const SPACE: u16 = 32;
const DOT: u16 = b'.' as u16;
const BACKSLASH: u16 = b'\\' as u16;

/// A character of the text being laid out: the UTF-16 unit in the low 16
/// bits, and [`UNDERLINE`] when a mnemonic prefix marked it.
type Ch = u32;
const UNDERLINE: Ch = 1 << 16;

fn unit(c: Ch) -> u16 {
    c as u16
}

/// `"&x"` shows `x` underlined, `"&&"` shows `&`; a lone `&` at the end
/// shows nothing.
fn prefixes(s: &[u16], mark: bool) -> Vec<Ch> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == AMP {
            match s.get(i + 1) {
                Some(&AMP) => out.push(Ch::from(AMP)),
                Some(&n) => out.push(Ch::from(n) | if mark { UNDERLINE } else { 0 }),
                None => {}
            }
            i += 2;
        } else {
            out.push(Ch::from(s[i]));
            i += 1;
        }
    }
    out
}

/// The text `DrawTextW` shows for `s` with its prefixes processed (no
/// underline).
pub fn strip_prefixes(s: &[u16]) -> Vec<u16> {
    prefixes(s, false).into_iter().map(unit).collect()
}

/// How wide tab stops are, and whether tabs expand at all.
#[derive(Clone, Copy)]
struct Tabs {
    expand: bool,
    stop: i32,
}

fn line_width(tf: &Typeface<'_>, s: &[Ch], tabs: Tabs) -> i32 {
    let mut x = 0;
    for &c in s {
        if unit(c) == TAB && tabs.expand {
            x = (x / tabs.stop + 1) * tabs.stop;
        } else {
            x += tf.width(unit(c));
        }
    }
    x
}

/// The width of a line of plain text, tabs expanded to multiples of eight
/// average character widths when asked.
pub fn text_width(tf: &Typeface<'_>, s: &[u16], expand_tabs: bool) -> i32 {
    let chars: Vec<Ch> = s.iter().map(|&c| Ch::from(c)).collect();
    line_width(tf, &chars, Tabs { expand: expand_tabs, stop: (8 * tf.metrics().ave_char_width).max(1) })
}

/// The lines of `s` for `format` in a rectangle `width` wide.
fn lines(tf: &Typeface<'_>, s: &[Ch], format: u32, width: i32, tabs: Tabs) -> Vec<Vec<Ch>> {
    if format & DT_SINGLELINE != 0 {
        return alloc::vec![s.to_vec()];
    }
    let mut out = Vec::new();
    for para in s.split(|&c| unit(c) == LF) {
        let para = para.strip_suffix(&[Ch::from(CR)]).unwrap_or(para);
        if format & DT_WORDBREAK == 0 {
            out.push(para.to_vec());
            continue;
        }
        let mut line: Vec<Ch> = Vec::new();
        for word in para.split(|&c| unit(c) == SPACE).filter(|w| !w.is_empty()) {
            if line.is_empty() {
                line.extend_from_slice(word);
                continue;
            }
            let mut candidate = line.clone();
            candidate.push(Ch::from(SPACE));
            candidate.extend_from_slice(word);
            if line_width(tf, &candidate, tabs) <= width {
                line = candidate;
            } else {
                out.push(core::mem::replace(&mut line, word.to_vec()));
            }
        }
        out.push(line);
    }
    out
}

fn dots(n: usize) -> Vec<Ch> {
    core::iter::repeat_n(Ch::from(DOT), n).collect()
}

/// `s` cut at the end to fit `width` with "..." after it (as many dots as
/// fit when not even one character does).
fn fit_end(tf: &Typeface<'_>, s: &[Ch], width: i32, tabs: Tabs) -> Vec<Ch> {
    if line_width(tf, s, tabs) <= width {
        return s.to_vec();
    }
    let dot_w = 3 * tf.width(DOT);
    if dot_w > width {
        let n = (0..=3).rev().find(|&n| n as i32 * tf.width(DOT) <= width).unwrap_or(0);
        return dots(n);
    }
    let n = (0..s.len()).rev().find(|&n| line_width(tf, &s[..n], tabs) + dot_w <= width).unwrap_or(0);
    let mut out = s[..n].to_vec();
    out.extend(dots(3));
    out
}

/// `s` with its middle replaced by "..." so that what follows the last
/// backslash stays where it fits (a string without one is cut at the end).
fn fit_path(tf: &Typeface<'_>, s: &[Ch], width: i32, tabs: Tabs) -> Vec<Ch> {
    if line_width(tf, s, tabs) <= width {
        return s.to_vec();
    }
    let Some(cut) = s.iter().rposition(|&c| unit(c) == BACKSLASH) else {
        return fit_end(tf, s, width, tabs);
    };
    let (head, tail) = s.split_at(cut);
    let dot_w = 3 * tf.width(DOT);
    let tail_w = line_width(tf, tail, tabs);
    if dot_w + tail_w > width {
        // Not even the tail fits: keep the end of the tail.
        let keep = (0..=tail.len()).find(|&k| line_width(tf, &tail[k..], tabs) + dot_w <= width).unwrap_or(tail.len());
        let mut out = dots(3);
        out.extend_from_slice(&tail[keep..]);
        return out;
    }
    let n = (0..head.len()).rev().find(|&n| line_width(tf, &head[..n], tabs) + dot_w + tail_w <= width).unwrap_or(0);
    let mut out = head[..n].to_vec();
    out.extend(dots(3));
    out.extend_from_slice(tail);
    out
}

/// `DT_WORD_ELLIPSIS`: every word wider than `width` is cut with "...".
fn fit_words(tf: &Typeface<'_>, s: &[Ch], width: i32, tabs: Tabs) -> Vec<Ch> {
    let mut out = Vec::with_capacity(s.len());
    for (i, word) in s.split(|&c| unit(c) == SPACE).enumerate() {
        if i > 0 {
            out.push(Ch::from(SPACE));
        }
        out.extend(fit_end(tf, word, width, tabs));
    }
    out
}

impl Gdi {
    /// `DrawTextW`: the height drawn (see the sheet for `DT_VCENTER` and
    /// `DT_BOTTOM`); `rect` changes only with `DT_CALCRECT`.
    pub fn draw_text(&mut self, dc: u64, text: &[u16], rect: &mut Rect, format: u32) -> i32 {
        self.draw_text_ex(dc, text, rect, format, 0).0
    }

    /// `DrawTextExW`'s work: `tab_chars` is the tab stop in average
    /// character widths (0: from `DT_TABSTOP`, else 8). The second result
    /// is the string `DT_MODIFYSTRING` writes back, when an ellipsis
    /// changed it.
    pub fn draw_text_ex(
        &mut self,
        dc: u64,
        text: &[u16],
        rect: &mut Rect,
        format: u32,
        tab_chars: i32,
    ) -> (i32, Option<Vec<u16>>) {
        let Some(tm) = self.text_metrics(dc) else { return (0, None) };
        let sel = self.font_sel(dc);
        let tf = sel.typeface();
        let h = tm.height + if format & DT_EXTERNALLEADING != 0 { tm.external_leading } else { 0 };
        let t = if format & DT_NOPREFIX == 0 {
            prefixes(text, format & (DT_HIDEPREFIX | DT_CALCRECT) == 0)
        } else {
            text.iter().map(|&c| Ch::from(c)).collect()
        };
        let single = format & DT_SINGLELINE != 0;
        if t.is_empty() && !single {
            if format & DT_CALCRECT != 0 {
                rect.right = rect.left;
                rect.bottom = rect.top;
            }
            return (1, None);
        }
        let per_tab = if tab_chars > 0 {
            tab_chars
        } else if format & DT_TABSTOP != 0 {
            ((format >> 8) & 0xFF) as i32
        } else {
            8
        };
        let per_tab = if per_tab == 0 { 8 } else { per_tab };
        let tabs =
            Tabs { expand: format & (DT_EXPANDTABS | DT_TABSTOP) != 0, stop: (per_tab * tm.ave_char_width).max(1) };
        let width = rect.right - rect.left;
        let mut ls = lines(&tf, &t, format, width, tabs);
        if format & DT_CALCRECT != 0 {
            let widths: Vec<i32> = ls.iter().map(|l| line_width(&tf, l, tabs)).collect();
            let total = h * ls.len() as i32;
            rect.right = rect.left + widths.iter().copied().max().unwrap_or(0);
            rect.bottom = rect.top + total;
            return (total, None);
        }
        let mut modified = None;
        if format & ELLIPSES != 0 {
            let before = ls.clone();
            self.fit_ellipses(&tf, &mut ls, format, rect, h, tabs);
            if before != ls && format & DT_MODIFYSTRING != 0 {
                let mut s = Vec::new();
                for (i, l) in ls.iter().enumerate() {
                    if i > 0 {
                        s.push(LF);
                    }
                    s.extend(l.iter().map(|&c| unit(c)));
                }
                modified = Some(s);
            }
        }
        let widths: Vec<i32> = ls.iter().map(|l| line_width(&tf, l, tabs)).collect();
        let total = h * ls.len() as i32;
        let mut y = rect.top;
        if single && format & DT_VCENTER != 0 {
            y = rect.top + (rect.bottom - rect.top - h) / 2;
        } else if single && format & DT_BOTTOM != 0 {
            y = rect.bottom - h;
        }
        let clip = if format & DT_NOCLIP != 0 { None } else { Some(*rect) };
        let only_underline = format & DT_PREFIXONLY != 0;
        for (l, w) in ls.iter().zip(&widths) {
            let x = if format & DT_RIGHT != 0 {
                rect.right - w
            } else if format & DT_CENTER != 0 {
                rect.left + (rect.right - rect.left - w) / 2
            } else {
                rect.left
            };
            let mut cx = 0;
            let segments: Vec<&[Ch]> =
                if tabs.expand { l.split(|&c| unit(c) == TAB).collect() } else { alloc::vec![l.as_slice()] };
            for (i, seg) in segments.iter().enumerate() {
                if i > 0 {
                    cx = (cx / tabs.stop + 1) * tabs.stop;
                }
                let units: Vec<u16> = seg.iter().map(|&c| unit(c)).collect();
                if !only_underline {
                    self.draw_text_run(dc, x + cx, y, &units, clip, None);
                }
                // Mnemonic underlines.
                let mut px = x + cx;
                for &c in seg.iter() {
                    let cw = tf.width(unit(c));
                    if c & UNDERLINE != 0 {
                        self.draw_underline(dc, px, cw, y, clip);
                    }
                    px += cw;
                }
                cx += line_width(&tf, seg, Tabs { expand: false, ..tabs });
            }
            y += h;
        }
        (if single && format & (DT_VCENTER | DT_BOTTOM) != 0 { y - rect.top } else { total }, modified)
    }

    /// Apply `DT_END_ELLIPSIS`, `DT_PATH_ELLIPSIS` and `DT_WORD_ELLIPSIS`
    /// to the lines (Learn, `DrawText`).
    fn fit_ellipses(&self, tf: &Typeface<'_>, ls: &mut Vec<Vec<Ch>>, format: u32, rect: &Rect, h: i32, tabs: Tabs) {
        let width = rect.right - rect.left;
        // A paragraph that runs on past the rectangle's bottom ends in an
        // ellipsis on its last visible line.
        if format & DT_END_ELLIPSIS != 0 && format & DT_WORDBREAK != 0 && format & DT_SINGLELINE == 0 && h > 0 {
            let fit = ((rect.bottom - rect.top) / h).max(1) as usize;
            if ls.len() > fit {
                let mut last = ls[fit - 1].clone();
                for l in &ls[fit..] {
                    last.push(Ch::from(SPACE));
                    last.extend_from_slice(l);
                }
                ls.truncate(fit);
                ls[fit - 1] = fit_end(tf, &last, width, tabs);
                // The earlier lines only lose what does not fit.
                for l in &mut ls[..fit - 1] {
                    *l = fit_end(tf, l, width, tabs);
                }
                return;
            }
        }
        for l in ls.iter_mut() {
            *l = if format & DT_PATH_ELLIPSIS != 0 {
                fit_path(tf, l, width, tabs)
            } else if format & DT_WORD_ELLIPSIS != 0 {
                fit_words(tf, l, width, tabs)
            } else {
                fit_end(tf, l, width, tabs)
            };
        }
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

    #[test]
    fn prefixes_mark_the_character_after_the_ampersand() {
        let p = prefixes(&w("&File && a&"), true);
        let shown: Vec<u16> = p.iter().map(|&c| unit(c)).collect();
        assert_eq!(shown, w("File & a"));
        assert_eq!(p.iter().filter(|&&c| c & UNDERLINE != 0).count(), 1);
        assert!(p[0] & UNDERLINE != 0);
        let hidden = prefixes(&w("&File"), false);
        assert!(hidden.iter().all(|&c| c & UNDERLINE == 0));
        assert_eq!(strip_prefixes(&w("&File")), w("File"));
    }

    #[test]
    fn ellipses_fit_the_width() {
        let sel = crate::gdi::Gdi::new().font_sel(0);
        let tf = sel.typeface();
        let tabs = Tabs { expand: false, stop: 56 };
        let s: Vec<Ch> = w("a long file name.txt").iter().map(|&c| Ch::from(c)).collect();
        let full = line_width(&tf, &s, tabs);
        // Fits: unchanged.
        assert_eq!(fit_end(&tf, &s, full, tabs), s);
        let cut = fit_end(&tf, &s, 80, tabs);
        assert!(line_width(&tf, &cut, tabs) <= 80);
        let text: Vec<u16> = cut.iter().map(|&c| unit(c)).collect();
        assert!(text.ends_with(&w("...")) && text.len() > 3);
        // Too narrow for three dots: fewer.
        let tiny = fit_end(&tf, &s, 2 * tf.width(DOT), tabs);
        assert_eq!(tiny.len(), 2);
        // Paths keep the file name.
        let path: Vec<Ch> = w("C:\\Users\\someone\\Documents\\report.txt").iter().map(|&c| Ch::from(c)).collect();
        let p = fit_path(&tf, &path, 150, tabs);
        let pt: Vec<u16> = p.iter().map(|&c| unit(c)).collect();
        assert!(line_width(&tf, &p, tabs) <= 150);
        assert!(pt.ends_with(&w("\\report.txt")), "{pt:?}");
        assert!(pt.windows(3).any(|x| x == w("...").as_slice()));
    }
}
