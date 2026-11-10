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

/// The text after its prefixes: `"&x"` shows `x` underlined (when `mark`),
/// `"&&"` shows `&`, a lone `&` at the end shows nothing. `at[i]` is where
/// the `i`-th character begins in `s` (its `&` included), the last entry
/// `s.len()`.
fn prefixes(s: &[u16], mark: bool) -> (Vec<Ch>, Vec<usize>) {
    let mut out = Vec::with_capacity(s.len());
    let mut at = Vec::with_capacity(s.len() + 1);
    let mut i = 0;
    let mut marked: Option<usize> = None;
    while i < s.len() {
        if s[i] == AMP {
            match s.get(i + 1) {
                Some(&AMP) => {
                    out.push(Ch::from(AMP));
                    at.push(i);
                }
                Some(&n) => {
                    if mark {
                        marked = Some(out.len());
                    }
                    out.push(Ch::from(n));
                    at.push(i);
                }
                None => {}
            }
            i += 2;
        } else {
            out.push(Ch::from(s[i]));
            at.push(i);
            i += 1;
        }
    }
    at.push(s.len());
    // Only the last marked character is underlined.
    if let Some(m) = marked {
        out[m] |= UNDERLINE;
    }
    (out, at)
}

/// The text `DrawTextW` shows for `s` with its prefixes processed (no
/// underline).
pub fn strip_prefixes(s: &[u16]) -> Vec<u16> {
    prefixes(s, false).0.into_iter().map(unit).collect()
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
            x += tf.advance(unit(c));
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

/// One line of the text: its characters `t[a..b]`, and where the next line
/// starts in `t` (past the line break, or the blank a wrap dropped).
#[derive(Clone, Copy, Debug)]
struct Ln {
    a: usize,
    b: usize,
    next: usize,
}

/// Where the line break at `t[i]` ends: LF, CR LF or CR.
fn break_len(t: &[Ch], i: usize) -> usize {
    if unit(t[i]) == CR && t.get(i + 1).is_some_and(|&n| unit(n) == LF) { 2 } else { 1 }
}

/// The lines of a paragraph `t[s..e]` wrapped at blanks in `width` pixels:
/// a blank that fits stays at the end of its line, one that does not is
/// dropped; a word wider than the line has a line of its own.
fn wrap(tf: &Typeface<'_>, t: &[Ch], s: usize, e: usize, width: i32, tabs: Tabs, out: &mut Vec<Ln>) {
    let (mut a, mut w, mut i) = (s, 0, s);
    while i < e {
        if unit(t[i]) == SPACE {
            let sw = tf.advance(SPACE);
            if w + sw <= width || a == i {
                w += sw;
            } else {
                out.push(Ln { a, b: i, next: i + 1 });
                a = i + 1;
                w = 0;
            }
            i += 1;
        } else {
            let mut j = i;
            while j < e && unit(t[j]) != SPACE {
                j += 1;
            }
            let ww = line_width(tf, &t[i..j], tabs);
            if w + ww <= width || a == i {
                w += ww;
            } else {
                out.push(Ln { a, b: i, next: i });
                a = i;
                w = ww;
            }
            i = j;
        }
    }
    out.push(Ln { a, b: e, next: e });
}

/// The lines of `t` for `format` in `width` pixels.
fn lines(tf: &Typeface<'_>, t: &[Ch], format: u32, width: i32, tabs: Tabs) -> Vec<Ln> {
    let mut out = Vec::new();
    if format & DT_SINGLELINE != 0 {
        out.push(Ln { a: 0, b: t.len(), next: t.len() });
        return out;
    }
    let mut start = 0;
    loop {
        let p = (start..t.len()).find(|&i| matches!(unit(t[i]), LF | CR)).unwrap_or(t.len());
        let first = out.len();
        if format & DT_WORDBREAK != 0 {
            wrap(tf, t, start, p, width, tabs, &mut out);
        } else {
            out.push(Ln { a: start, b: p, next: p });
        }
        if p >= t.len() {
            break;
        }
        let end = p + break_len(t, p);
        if let Some(l) = out[first..].last_mut() {
            l.next = end;
        }
        start = end;
    }
    out
}

fn dots(n: usize) -> Vec<Ch> {
    core::iter::repeat_n(Ch::from(DOT), n).collect()
}

/// `s` cut at the end to fit `width` with "..." after it: as many
/// characters as fit with the dots, and at least one, wider than `width`
/// or not (`DrawTextW.yaml`).
fn fit_end(tf: &Typeface<'_>, s: &[Ch], width: i32, tabs: Tabs) -> Vec<Ch> {
    if line_width(tf, s, tabs) <= width {
        return s.to_vec();
    }
    let dot_w = 3 * tf.advance(DOT);
    let n = (1..s.len()).rev().find(|&n| line_width(tf, &s[..n], tabs) + dot_w <= width).unwrap_or(1).min(s.len());
    let mut out = s[..n].to_vec();
    out.extend(dots(3));
    out
}

/// `s` with its middle replaced by "..." so that what follows the last
/// backslash (the backslash too) stays: the head keeps as many characters
/// as fit, none if need be. A string without a backslash, or with
/// nothing after the last one, is left alone.
fn fit_path(tf: &Typeface<'_>, s: &[Ch], width: i32, tabs: Tabs) -> Vec<Ch> {
    if line_width(tf, s, tabs) <= width {
        return s.to_vec();
    }
    let Some(cut) = s.iter().rposition(|&c| unit(c) == BACKSLASH) else { return s.to_vec() };
    if cut + 1 >= s.len() {
        return s.to_vec();
    }
    let (head, tail) = s.split_at(cut);
    let dot_w = 3 * tf.advance(DOT);
    let tail_w = line_width(tf, tail, tabs);
    let n = (0..head.len()).rev().find(|&n| line_width(tf, &head[..n], tabs) + dot_w + tail_w <= width).unwrap_or(0);
    let mut out = head[..n].to_vec();
    out.extend(dots(3));
    out.extend_from_slice(tail);
    out
}

/// What [`Gdi::draw_text_ex`] reports.
#[derive(Debug, Default)]
pub struct DrawOut {
    /// `DrawText`'s return value.
    pub height: i32,
    /// The text `DT_MODIFYSTRING` writes back, when an ellipsis changed it.
    pub modified: Option<Vec<u16>>,
    /// `uiLengthDrawn`: the characters of the text laid out.
    pub drawn: usize,
}

/// `DrawTextExW`'s extras: the margins and the tab length.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawParams {
    /// The tab stop in average character widths (0: the default; only with
    /// `DT_TABSTOP`).
    pub tab_chars: i32,
    /// `DrawTextW`, or `DrawTextExW` without a `DRAWTEXTPARAMS`: with
    /// `DT_TABSTOP` bits 8 to 15 of the format are the tab stop, not the
    /// flags that have those bits (Learn: `DT_TABSTOP`).
    pub tab_in_format: bool,
    pub left_margin: i32,
    pub right_margin: i32,
}

/// The text of `t[a..b]` as source text: the characters with their
/// prefixes (`&x`, and `&&` for a literal `&` when prefixes are on).
fn source_of(t: &[Ch], prefix: bool) -> Vec<u16> {
    let mut out = Vec::new();
    for &c in t {
        if prefix && (c & UNDERLINE != 0 || unit(c) == AMP) {
            out.push(AMP);
        }
        out.push(unit(c));
    }
    out
}

impl Gdi {
    /// `DrawTextW`: the height drawn (see the sheet for `DT_VCENTER` and
    /// `DT_BOTTOM`); `rect` changes only with `DT_CALCRECT`.
    pub fn draw_text(&mut self, dc: u64, text: &[u16], rect: &mut Rect, format: u32) -> i32 {
        self.draw_text_ex(dc, text, rect, format, DrawParams { tab_in_format: true, ..DrawParams::default() }).height
    }

    /// `DrawTextExW`'s work (`DrawTextW.yaml`, `DrawTextExW.yaml`).
    pub fn draw_text_ex(&mut self, dc: u64, text: &[u16], rect: &mut Rect, format: u32, p: DrawParams) -> DrawOut {
        let Some(tm) = self.text_metrics(dc) else { return DrawOut::default() };
        // The tab stop of `DT_TABSTOP`: `iTabLength`, or the format's second
        // byte, which then holds no flags (a tab stop of 4 is not
        // `DT_CALCRECT`).
        let (format, per_tab) = if format & DT_TABSTOP == 0 {
            (format, 8)
        } else if p.tab_in_format {
            (format & !0xFF00, ((format >> 8) & 0xFF) as i32)
        } else {
            (format, p.tab_chars)
        };
        let sel = self.font_sel(dc);
        let tf = sel.typeface();
        let h = tm.height + if format & DT_EXTERNALLEADING != 0 { tm.external_leading } else { 0 };
        let prefix = format & DT_NOPREFIX == 0;
        let (t, at) = if prefix {
            prefixes(text, format & (DT_HIDEPREFIX | DT_CALCRECT) == 0)
        } else {
            (text.iter().map(|&c| Ch::from(c)).collect(), (0..=text.len()).collect())
        };
        let single = format & DT_SINGLELINE != 0;
        let calc = format & DT_CALCRECT != 0;
        if text.is_empty() && !single {
            if calc {
                rect.right = rect.left;
                rect.bottom = rect.top;
            }
            return DrawOut { height: 1, modified: None, drawn: 0 };
        }
        // The rectangle the text is laid out and aligned in.
        let area = Rect::new(rect.left + p.left_margin, rect.top, rect.right - p.right_margin, rect.bottom);
        let width = area.right - area.left;
        if format & DT_WORDBREAK != 0 && !single && width <= 0 {
            return DrawOut { height: 1, modified: None, drawn: 0 };
        }
        let per_tab = if per_tab <= 0 { 8 } else { per_tab };
        let tabs =
            Tabs { expand: format & (DT_EXPANDTABS | DT_TABSTOP) != 0, stop: (per_tab * tm.ave_char_width).max(1) };
        let all = lines(&tf, &t, format, width, tabs);
        // The lines laid out: those that start inside the rectangle (the
        // first line always); an edit control leaves out a last line that
        // is only partly inside.
        let noclip = format & DT_NOCLIP != 0;
        let processed = if calc || noclip || single {
            all.len()
        } else if format & DT_EDITCONTROL != 0 {
            (0..all.len()).take_while(|&i| rect.top + (i as i32 + 1) * h <= rect.bottom).count().max(1)
        } else {
            (0..all.len()).take_while(|&i| rect.top + i as i32 * h <= rect.bottom).count().max(1)
        };
        let mut contents: Vec<Vec<Ch>> = all[..processed].iter().map(|l| t[l.a..l.b].to_vec()).collect();
        let mut cut = alloc::vec![false; contents.len()];
        let drawn_of = |n: usize| -> usize { all[n - 1].next.min(t.len()) };
        let mut drawn = if processed == 0 { 0 } else { at[drawn_of(processed)] };
        // What the ellipsis flags cut (Learn): `DT_PATH_ELLIPSIS` the middle
        // of every line that is too wide; `DT_WORD_ELLIPSIS` the end of every
        // such line (with `DT_WORDBREAK` those are the words wider than the
        // rectangle); `DT_END_ELLIPSIS` only the end of the text, which is
        // its last line laid out: a line before it that is too wide is
        // clipped without dots.
        let wraps = format & DT_WORDBREAK != 0 && !single;
        let path_style = format & DT_PATH_ELLIPSIS != 0;
        let end_of_text = !path_style && format & DT_END_ELLIPSIS != 0;
        let last = processed - 1;
        if path_style || format & DT_WORD_ELLIPSIS != 0 {
            for (i, c) in contents.iter_mut().enumerate() {
                if end_of_text && i == last {
                    continue;
                }
                let fitted = if path_style { fit_path(&tf, c, width, tabs) } else { fit_end(&tf, c, width, tabs) };
                if fitted != *c {
                    *c = fitted;
                    cut[i] = true;
                }
            }
        }
        // With `DT_WORDBREAK` the last line laid out takes what the rectangle
        // has no room for, and is cut to fit.
        let swallow = end_of_text && wraps && processed < all.len();
        if end_of_text {
            let l = if swallow { t[all[last].a..].to_vec() } else { contents[last].clone() };
            let fitted = fit_end(&tf, &l, width, tabs);
            if fitted != contents[last] {
                contents[last] = fitted;
                cut[last] = true;
                if swallow {
                    drawn = text.len();
                }
            }
        }
        let modified = if cut.iter().any(|&c| c) {
            // The text with each cut line replaced; what follows a swallowing
            // last line is in that line.
            let mut s = Vec::new();
            for (i, l) in all[..processed].iter().enumerate() {
                let next = at[l.next.min(t.len())];
                if cut[i] {
                    s.extend(source_of(&contents[i], prefix));
                    if swallow && i == last {
                        break;
                    }
                    s.extend_from_slice(&text[at[l.b]..next]);
                } else {
                    s.extend_from_slice(&text[at[l.a]..next]);
                }
            }
            Some(s)
        } else {
            None
        };
        let widths: Vec<i32> = contents.iter().map(|l| line_width(&tf, l, tabs)).collect();
        if calc {
            let w = widths.iter().copied().max().unwrap_or(0);
            let mut total = h * contents.len() as i32;
            // A single line that would be centred or put at the bottom: the
            // height is the offset of its bottom, as when drawing.
            if single && format & DT_VCENTER != 0 {
                total = (rect.bottom - rect.top - h) / 2 + h;
            } else if single && format & DT_BOTTOM != 0 {
                total = rect.bottom - rect.top;
            }
            rect.right = rect.left + p.left_margin + w + p.right_margin;
            rect.bottom = rect.top + total;
            return DrawOut { height: total, modified, drawn: text.len() };
        }
        let total = h * contents.len() as i32;
        let mut y = rect.top;
        if single && format & DT_VCENTER != 0 {
            y = rect.top + (rect.bottom - rect.top - h) / 2;
        } else if single && format & DT_BOTTOM != 0 {
            y = rect.bottom - h;
        }
        let clip = if noclip { None } else { Some(*rect) };
        let only_underline = format & DT_PREFIXONLY != 0;
        let soft = format & DT_WORDBREAK != 0 && !single;
        for (l, w) in contents.iter().zip(&widths) {
            // A line that wrapped keeps its trailing blank for the left
            // edge only; centred and right aligned lines do not count it.
            let mut l = l.as_slice();
            let mut w = *w;
            if soft && format & (DT_CENTER | DT_RIGHT) != 0 {
                while let Some((&last, rest)) = l.split_last() {
                    if unit(last) != SPACE {
                        break;
                    }
                    l = rest;
                }
                w = line_width(&tf, l, tabs);
            }
            let x = if format & DT_RIGHT != 0 {
                area.right - w
            } else if format & DT_CENTER != 0 {
                area.left + (width - w) / 2
            } else {
                area.left
            };
            let mut cx = 0;
            let segments: Vec<&[Ch]> =
                if tabs.expand { l.split(|&c| unit(c) == TAB).collect() } else { alloc::vec![l] };
            // A line with tabs gets its background in one piece.
            let gaps = segments.len() > 1 && !only_underline;
            if gaps {
                self.fill_text_cell(dc, x, y, line_width(&tf, l, tabs), clip);
            }
            for (i, seg) in segments.iter().enumerate() {
                if i > 0 {
                    cx = (cx / tabs.stop + 1) * tabs.stop;
                }
                let units: Vec<u16> = seg.iter().map(|&c| unit(c)).collect();
                if !only_underline {
                    self.draw_text_run(dc, x + cx, y, &units, clip, None, !gaps);
                }
                // Mnemonic underlines.
                let mut px = x + cx;
                for &c in seg.iter() {
                    let cw = tf.advance(unit(c));
                    if c & UNDERLINE != 0 {
                        self.draw_underline(dc, px, cw, y, clip);
                    }
                    px += cw;
                }
                cx += line_width(&tf, seg, Tabs { expand: false, ..tabs });
            }
            y += h;
        }
        let height = if single && format & (DT_VCENTER | DT_BOTTOM) != 0 { y - rect.top } else { total };
        DrawOut { height, modified, drawn: drawn.min(text.len()) }
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
        let p = prefixes(&w("&File && a&"), true).0;
        let shown: Vec<u16> = p.iter().map(|&c| unit(c)).collect();
        assert_eq!(shown, w("File & a"));
        assert_eq!(p.iter().filter(|&&c| c & UNDERLINE != 0).count(), 1);
        assert!(p[0] & UNDERLINE != 0);
        let hidden = prefixes(&w("&File"), false).0;
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
        // Too narrow: one character and the dots, wider than the width.
        let tiny = fit_end(&tf, &s, 2 * tf.width(DOT), tabs);
        assert_eq!(tiny.len(), 4);
        // Paths keep the file name.
        let path: Vec<Ch> = w("C:\\Users\\someone\\Documents\\report.txt").iter().map(|&c| Ch::from(c)).collect();
        let p = fit_path(&tf, &path, 150, tabs);
        let pt: Vec<u16> = p.iter().map(|&c| unit(c)).collect();
        assert!(line_width(&tf, &p, tabs) <= 150);
        assert!(pt.ends_with(&w("\\report.txt")), "{pt:?}");
        assert!(pt.windows(3).any(|x| x == w("...").as_slice()));
    }
}
