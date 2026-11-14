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

/// The room `c` takes in a line: a line break (inside a single line, or in
/// a line that took over the rest of the text) and a tab that is not
/// expanded have no width and draw nothing, in every font.
fn char_width(tf: &Typeface<'_>, c: u16) -> i32 {
    if matches!(c, TAB | LF | CR) { 0 } else { tf.advance(c) }
}

fn line_width(tf: &Typeface<'_>, s: &[Ch], tabs: Tabs) -> i32 {
    let mut x = 0i32;
    for &c in s {
        if unit(c) == TAB && tabs.expand {
            // Always forward: text that ends on a stop goes to the next.
            x = (x / tabs.stop + 1).saturating_mul(tabs.stop);
        } else {
            x = x.saturating_add(char_width(tf, unit(c)));
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

/// What a format means once the flags that exclude each other are settled
/// (`DrawTextW.yaml`: tabstop, pairs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// The flags, without the tab length where bits 8 to 15 held one.
    pub flags: u32,
    /// The tab stop in average character widths.
    pub tab_chars: i32,
}

impl Format {
    /// `DrawTextW`'s format: with `DT_TABSTOP` bits 8 to 15 are the tab
    /// length and nothing else (not `DT_NOCLIP` ... `DT_END_ELLIPSIS`); a
    /// length of 0 means 8. `ex_tab` is `DrawTextExW`'s `iTabLength`, which
    /// leaves the bits their meaning.
    pub fn new(format: u32, ex_tab: Option<i32>) -> Format {
        let (flags, tab) = if format & DT_TABSTOP == 0 {
            (format, 8)
        } else {
            match ex_tab {
                Some(n) => (format, n),
                None => (format & !0xFF00, ((format >> 8) & 0xFF) as i32),
            }
        };
        Format { flags, tab_chars: if tab <= 0 { 8 } else { tab } }
    }

    fn has(self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    pub fn single(self) -> bool {
        self.has(DT_SINGLELINE)
    }

    /// `DT_WORDBREAK`; a single line is never wrapped.
    pub fn wraps(self) -> bool {
        self.has(DT_WORDBREAK) && !self.single()
    }

    /// Tabs expand only with `DT_EXPANDTABS`; `DT_TABSTOP` alone does not.
    pub fn expand_tabs(self) -> bool {
        self.has(DT_EXPANDTABS)
    }

    /// `DT_CENTER` with `DT_RIGHT` centres.
    pub fn horizontal(self) -> Horizontal {
        if self.has(DT_CENTER) {
            Horizontal::Center
        } else if self.has(DT_RIGHT) {
            Horizontal::Right
        } else {
            Horizontal::Left
        }
    }

    /// `DT_VCENTER` and `DT_BOTTOM` act on a single line only, and both
    /// together put the text at the top.
    pub fn vertical(self) -> Vertical {
        match (self.single(), self.flags & (DT_VCENTER | DT_BOTTOM)) {
            (true, DT_VCENTER) => Vertical::Center,
            (true, DT_BOTTOM) => Vertical::Bottom,
            _ => Vertical::Top,
        }
    }

    /// `&` is a prefix unless `DT_NOPREFIX`.
    pub fn prefix(self) -> bool {
        !self.has(DT_NOPREFIX)
    }

    /// The marked character is underlined: not with `DT_NOPREFIX` or
    /// `DT_HIDEPREFIX`.
    pub fn underline(self) -> bool {
        self.prefix() && !self.has(DT_HIDEPREFIX)
    }

    /// `DT_PREFIXONLY`: only the underline is drawn; with `DT_NOPREFIX` or
    /// `DT_HIDEPREFIX` the text is drawn after all.
    pub fn only_underline(self) -> bool {
        self.underline() && self.has(DT_PREFIXONLY)
    }

    /// `DT_EXTERNALLEADING` adds the font's external leading below each
    /// line. Not with `DT_INTERNAL`, which measures with the system font
    /// (Learn), whose external leading is none.
    pub fn leading(self, external_leading: i32) -> i32 {
        if self.has(DT_EXTERNALLEADING) && !self.has(DT_INTERNAL) { external_leading } else { 0 }
    }

    /// The height of one line (`DrawTextW.yaml`, internal): the selected
    /// font's height and leading, or with `DT_INTERNAL` the stock system
    /// font's height, whatever font is selected (its width stays the
    /// selected font's).
    pub fn line_height(self, font: &crate::font::TextMetric, system: &crate::font::TextMetric) -> i32 {
        if self.has(DT_INTERNAL) {
            system.height + self.leading(system.external_leading)
        } else {
            font.height + self.leading(font.external_leading)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Horizontal {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vertical {
    Top,
    Center,
    Bottom,
}

/// One line of the text: its characters `t[a..b]`, and where the next line
/// starts in `t` (past the line break, or the blank a wrap dropped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ln {
    a: usize,
    b: usize,
    next: usize,
}

/// Where the line break at `t[i]` ends: a carriage return with a line feed
/// after it and a line feed with a carriage return after it are one break,
/// either of them alone is one too. Pairs are taken from the left.
fn break_len(t: &[Ch], i: usize) -> usize {
    let next = t.get(i + 1).map(|&n| unit(n));
    match (unit(t[i]), next) {
        (CR, Some(LF)) | (LF, Some(CR)) => 2,
        _ => 1,
    }
}

/// How `DT_WORDBREAK` wraps.
#[derive(Clone, Copy)]
struct Wrap {
    width: i32,
    /// `DT_EDITCONTROL`: a word wider than the line is broken between
    /// characters.
    break_words: bool,
}

/// The lines of a paragraph `t[s..e]` wrapped at blanks: a blank that fits
/// stays at the end of its line, one that does not is dropped; a word that
/// does not fit behind what the line holds goes to a line of its own. A
/// word wider than the line stays whole, or (`break_words`) is broken
/// between characters, as many to a line as fit and at least one; what
/// follows its last piece goes on behind that piece.
fn wrap(tf: &Typeface<'_>, t: &[Ch], s: usize, e: usize, wr: Wrap, tabs: Tabs, out: &mut Vec<Ln>) {
    let fits = |a: usize, b: usize| line_width(tf, &t[a..b], tabs) <= wr.width;
    let (mut a, mut i) = (s, s);
    while i < e {
        if unit(t[i]) == SPACE {
            if a != i && !fits(a, i + 1) {
                out.push(Ln { a, b: i, next: i + 1 });
                a = i + 1;
            }
            i += 1;
            continue;
        }
        let mut j = i;
        while j < e && unit(t[j]) != SPACE {
            j += 1;
        }
        if fits(a, j) {
            i = j;
        } else if a != i {
            // The word starts a line of its own, and is looked at again.
            out.push(Ln { a, b: i, next: i });
            a = i;
        } else if !wr.break_words {
            i = j;
        } else {
            let mut k = i + 1;
            while k < j && fits(a, k + 1) {
                k += 1;
            }
            if k < j {
                out.push(Ln { a, b: k, next: k });
                a = k;
            }
            i = k;
        }
    }
    out.push(Ln { a, b: e, next: e });
}

/// The lines of `t`: one for a single line, else one for every line break
/// (a break at the very end adds an empty line), wrapped when asked.
fn lines(tf: &Typeface<'_>, t: &[Ch], single: bool, wr: Option<Wrap>, tabs: Tabs) -> Vec<Ln> {
    let mut out = Vec::new();
    if single {
        out.push(Ln { a: 0, b: t.len(), next: t.len() });
        return out;
    }
    let mut start = 0;
    loop {
        let p = (start..t.len()).find(|&i| matches!(unit(t[i]), LF | CR)).unwrap_or(t.len());
        match wr {
            Some(wr) => wrap(tf, t, start, p, wr, tabs, &mut out),
            None => out.push(Ln { a: start, b: p, next: p }),
        }
        if p >= t.len() {
            break;
        }
        let end = p + break_len(t, p);
        if let Some(l) = out.last_mut() {
            l.next = end;
        }
        start = end;
    }
    out
}

/// How many of `total` lines of height `h` are laid out in a rectangle
/// `height` high (`DrawTextW.yaml`, lines and editcontrol): the lines whose
/// top is not below the bottom edge, or (`whole`, `DT_EDITCONTROL`) those
/// that fit whole; the first line always.
fn lines_laid_out(total: usize, h: i32, height: i32, whole: bool) -> usize {
    if h <= 0 {
        return total;
    }
    let (h, height) = (i64::from(h), i64::from(height));
    let room = |i: usize| (i as i64 + i64::from(whole)) * h <= height;
    (0..total).take_while(|&i| room(i)).count().max(1).min(total.max(1))
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

/// The lines as they are shown, after the ellipsis flags.
struct Shown {
    lines: Vec<Vec<Ch>>,
    /// The first line an ellipsis shortened.
    first_cut: Option<usize>,
    /// The last line took over the rest of the text.
    took_rest: bool,
}

/// What the ellipsis flags make of the first `n` lines of `all`
/// (`DrawTextW.yaml`, ellipsis). `DT_WORD_ELLIPSIS` cuts every line that
/// is too wide and never takes over what follows. `DT_END_ELLIPSIS` and
/// `DT_PATH_ELLIPSIS` act on the last line only: the one that holds the
/// end of the text (not the empty line a break at the very end makes: the
/// line before it), or, when the height left lines out, the last line laid
/// out, which then takes all the text that is left, its line breaks as
/// characters without width. `DT_PATH_ELLIPSIS` gives a cut line the path
/// form.
fn ellipsize(tf: &Typeface<'_>, t: &[Ch], all: &[Ln], n: usize, f: Format, width: i32, tabs: Tabs) -> Shown {
    let mut shown: Vec<Vec<Ch>> = all[..n].iter().map(|l| t[l.a..l.b].to_vec()).collect();
    let path = f.has(DT_PATH_ELLIPSIS);
    let fit = |s: &[Ch]| if path { fit_path(tf, s, width, tabs) } else { fit_end(tf, s, width, tabs) };
    let mut cut = alloc::vec![false; n];
    if f.has(DT_WORD_ELLIPSIS) {
        for (i, l) in shown.iter_mut().enumerate() {
            let fitted = fit(l);
            if fitted != *l {
                *l = fitted;
                cut[i] = true;
            }
        }
    }
    let mut took_rest = false;
    if (path || f.has(DT_END_ELLIPSIS)) && n > 0 {
        if n < all.len() {
            let last = n - 1;
            let rest = &t[all[last].a..];
            let fitted = fit(rest);
            cut[last] = fitted != rest;
            shown[last] = fitted;
            took_rest = true;
        } else {
            let ends_in_break = t.last().is_some_and(|&c| matches!(unit(c), LF | CR));
            let last = if n >= 2 && all[n - 1].a == t.len() && ends_in_break { n - 2 } else { n - 1 };
            if !cut[last] {
                let fitted = fit(&shown[last]);
                if fitted != shown[last] {
                    shown[last] = fitted;
                    cut[last] = true;
                }
            }
        }
    }
    Shown { lines: shown, first_cut: cut.iter().position(|&c| c), took_rest }
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

/// What `DrawTextW` answers before it looks at the DC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Early {
    /// The return value; nothing is drawn and the rectangle stays.
    Return(i32),
    /// The text is laid out (an empty one too).
    Layout,
}

/// `DrawTextW`'s checks on the string and the count (`DrawTextW.yaml`,
/// count and bad_dc): a NULL string (or rectangle) and a count below -1
/// give 0; a count of 0 on a text that does not start with a NUL gives 1,
/// whatever the DC and the flags are. A count of 0 on a text that starts
/// with a NUL and a count of -1 on an empty string are the empty string.
pub fn count_check(null_text: bool, null_rect: bool, starts_with_nul: bool, count: i32) -> Early {
    if null_text || null_rect || count < -1 {
        Early::Return(0)
    } else if count == 0 && !starts_with_nul {
        Early::Return(1)
    } else {
        Early::Layout
    }
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

/// Half of the room left over, rounded down: the text sits half a pixel
/// towards the left or the top.
fn half(room: i32) -> i32 {
    room.div_euclid(2)
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
        let f = Format::new(format, if p.tab_in_format { None } else { Some(p.tab_chars) });
        let sel = self.font_sel(dc);
        let tf = sel.typeface();
        let h = f.line_height(&tm, &self.system_font_metrics());
        let prefix = f.prefix();
        let (t, at) = if prefix {
            prefixes(text, f.underline())
        } else {
            (text.iter().map(|&c| Ch::from(c)).collect(), (0..=text.len()).collect())
        };
        let single = f.single();
        let calc = f.has(DT_CALCRECT);
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
        if f.wraps() && width <= 0 {
            return DrawOut { height: 1, modified: None, drawn: 0 };
        }
        let tabs = Tabs { expand: f.expand_tabs(), stop: f.tab_chars.saturating_mul(tm.ave_char_width).max(1) };
        let edit = f.has(DT_EDITCONTROL);
        let wr = f.wraps().then_some(Wrap { width, break_words: edit });
        let all = lines(&tf, &t, single, wr, tabs);
        // The lines laid out; measuring and `DT_NOCLIP` lay out every line.
        let noclip = f.has(DT_NOCLIP);
        let n = if calc || noclip || single {
            all.len()
        } else {
            lines_laid_out(all.len(), h, rect.bottom - rect.top, edit)
        };
        let shown = ellipsize(&tf, &t, &all, n, f, width, tabs);
        // `DT_MODIFYSTRING`: the text up to the end of the first line that
        // was shortened.
        let modified = shown.first_cut.map(|i| {
            let mut s = text[..at[all[i].a]].to_vec();
            s.extend(source_of(&shown.lines[i], prefix));
            s
        });
        let drawn = if shown.took_rest { text.len() } else { at[all[n - 1].next.min(t.len())].min(text.len()) };
        let contents = shown.lines;
        let widths: Vec<i32> = contents.iter().map(|l| line_width(&tf, l, tabs)).collect();
        // A last line that took the rest of the text counts the empty line
        // a break at the very end of the text makes, though nothing of it is
        // shown (`DrawTextW.yaml`, ellipsis).
        let counted =
            contents.len() + usize::from(shown.took_rest && t.last().is_some_and(|&c| matches!(unit(c), LF | CR)));
        let total = h.saturating_mul(counted as i32);
        let room = rect.bottom - rect.top - h;
        let vertical = f.vertical();
        // A single line that is centred or put at the bottom: the offset of
        // the text's bottom from the rectangle's top, drawn or measured.
        let (offset, height) = match vertical {
            Vertical::Top => (0, total),
            Vertical::Center => (half(room), half(room) + h),
            Vertical::Bottom => (room, room + h),
        };
        if calc {
            let w = widths.iter().copied().max().unwrap_or(0);
            rect.right = rect.left + p.left_margin + w + p.right_margin;
            rect.bottom = rect.top + height;
            return DrawOut { height, modified, drawn: text.len() };
        }
        let mut y = rect.top + offset;
        let clip = if noclip { None } else { Some(*rect) };
        let only_underline = f.only_underline();
        let soft = f.wraps();
        let horizontal = f.horizontal();
        for (l, w) in contents.iter().zip(&widths) {
            // A line that wrapped keeps its trailing blank for the left
            // edge only; centred and right aligned lines do not count it.
            let mut l = l.as_slice();
            let mut w = *w;
            if soft && horizontal != Horizontal::Left {
                while let Some((&last, rest)) = l.split_last() {
                    if unit(last) != SPACE {
                        break;
                    }
                    l = rest;
                }
                w = line_width(&tf, l, tabs);
            }
            let x = match horizontal {
                Horizontal::Left => area.left,
                Horizontal::Center => area.left + half(width - w),
                Horizontal::Right => area.right - w,
            };
            let mut cx = 0;
            let segments: Vec<&[Ch]> =
                if tabs.expand { l.split(|&c| unit(c) == TAB).collect() } else { alloc::vec![l] };
            // A line with tabs gets its background in one piece.
            let gaps = segments.len() > 1 && !only_underline;
            if gaps {
                self.fill_text_cell(dc, x, y, w, clip);
            }
            for (i, seg) in segments.iter().enumerate() {
                if i > 0 {
                    cx = (cx / tabs.stop + 1).saturating_mul(tabs.stop);
                }
                // Line breaks and tabs that take no room draw nothing.
                let units: Vec<u16> = seg.iter().map(|&c| unit(c)).filter(|&c| !matches!(c, TAB | LF | CR)).collect();
                if !only_underline {
                    self.draw_text_run(dc, x.saturating_add(cx), y, &units, clip, None, !gaps);
                }
                // Mnemonic underlines.
                let mut px = x.saturating_add(cx);
                for &c in seg.iter() {
                    let cw = char_width(&tf, unit(c));
                    if c & UNDERLINE != 0 {
                        self.draw_underline(dc, px, cw, y, clip);
                    }
                    px += cw;
                }
                cx = cx.saturating_add(line_width(&tf, seg, Tabs { expand: false, ..tabs }));
            }
            y += h;
        }
        DrawOut { height, modified, drawn }
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
        // A line break in a single line has no width.
        let one_two = calc(&mut g, dc, "onetwo", 0, DT_SINGLELINE).1.right;
        assert_eq!(calc(&mut g, dc, "one\ntwo", 0, DT_SINGLELINE).1.right, one_two);
        assert_eq!(calc(&mut g, dc, "the quick brown fox jumps", 60, DT_WORDBREAK), (64, Rect::new(10, 20, 68, 84)));
        assert_eq!(calc(&mut g, dc, "the quick brown fox jumps", 1000, DT_WORDBREAK).1.right, 179);
        assert_eq!(calc(&mut g, dc, "abcdefghij", 20, DT_WORDBREAK), (16, Rect::new(10, 20, 77, 36)));
        // A tab without DT_EXPANDTABS has no width.
        let ab = calc(&mut g, dc, "ab", 0, DT_SINGLELINE).1.right;
        assert_eq!(calc(&mut g, dc, "a\tb", 0, DT_SINGLELINE).1.right, ab);
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

    fn chars(s: &str) -> Vec<Ch> {
        s.encode_utf16().map(Ch::from).collect()
    }

    /// The number of line breaks at the start of `s`, pairs taken from the left.
    fn breaks(s: &str) -> usize {
        let t = chars(s);
        let (mut i, mut n) = (0, 0);
        while i < t.len() {
            i += break_len(&t, i);
            n += 1;
        }
        n
    }

    #[test]
    fn line_break_pairs_are_taken_from_the_left() {
        // dt_pf_breaks / line_breaks.
        for (s, n) in [
            ("\n", 1),
            ("\r", 1),
            ("\r\n", 1),
            ("\n\r", 1),
            ("\n\n", 2),
            ("\r\r", 2),
            ("\r\n\r", 2),
            ("\n\r\n", 2),
            ("\r\r\n", 2),
            ("\n\n\r", 2),
            ("\r\n\n", 2),
            ("\n\r\r", 2),
            ("\r\n\r\n", 2),
            ("\n\r\n\r", 2),
            ("\r\n\n\r", 2),
            ("\n\r\r\n", 2),
            ("\r\r\r", 3),
            ("\n\r\n\r\n", 3),
        ] {
            assert_eq!(breaks(s), n, "{s:?}");
        }
    }

    #[test]
    fn the_format_settles_flags_that_exclude_each_other() {
        // tabstop: bits 8 to 15 are the length and no flags.
        let f = Format::new(DT_TABSTOP | DT_EXPANDTABS | 4 << 8, None);
        assert_eq!((f.flags & DT_CALCRECT, f.tab_chars), (0, 4));
        for (bits, flag) in [
            (1, DT_NOCLIP),
            (2, DT_EXTERNALLEADING),
            (4, DT_CALCRECT),
            (8, DT_NOPREFIX),
            (16, DT_INTERNAL),
            (32, DT_EDITCONTROL),
            (64, DT_PATH_ELLIPSIS),
            (128, DT_END_ELLIPSIS),
        ] {
            let f = Format::new(DT_TABSTOP | bits << 8, None);
            assert_eq!((f.flags & flag, f.tab_chars), (0, bits as i32), "length {bits}");
            // Without DT_TABSTOP the bit is the flag and the length the default.
            let f = Format::new(bits << 8, None);
            assert_eq!((f.flags & flag, f.tab_chars), (flag, 8), "flag {flag:#x}");
        }
        assert_eq!(Format::new(DT_TABSTOP, None).tab_chars, 8, "a length of 0 means 8");
        assert_eq!(Format::new(DT_TABSTOP | 255 << 8, None).tab_chars, 255);
        // Flags above bit 15 keep their meaning.
        let f = Format::new(DT_TABSTOP | 3 << 8 | DT_WORD_ELLIPSIS | DT_HIDEPREFIX, None);
        assert_eq!(f.flags & (DT_WORD_ELLIPSIS | DT_HIDEPREFIX), DT_WORD_ELLIPSIS | DT_HIDEPREFIX);
        // DT_TABSTOP alone does not expand tabs.
        assert!(!Format::new(DT_TABSTOP | 3 << 8, None).expand_tabs());
        assert!(Format::new(DT_TABSTOP | DT_EXPANDTABS | 3 << 8, None).expand_tabs());
        // DrawTextExW's iTabLength leaves the bits their meaning.
        let f = Format::new(DT_TABSTOP | DT_CALCRECT, Some(4));
        assert_eq!((f.flags & DT_CALCRECT, f.tab_chars), (DT_CALCRECT, 4));
        assert_eq!(Format::new(DT_TABSTOP, Some(0)).tab_chars, 8);
        assert_eq!(Format::new(0, Some(4)).tab_chars, 8, "iTabLength without DT_TABSTOP");

        // pairs.
        let f = |flags| Format::new(flags, None);
        assert_eq!(f(DT_CENTER | DT_RIGHT).horizontal(), Horizontal::Center);
        assert_eq!(f(DT_RIGHT).horizontal(), Horizontal::Right);
        assert_eq!(f(0).horizontal(), Horizontal::Left);
        assert_eq!(f(DT_SINGLELINE | DT_VCENTER).vertical(), Vertical::Center);
        assert_eq!(f(DT_SINGLELINE | DT_BOTTOM).vertical(), Vertical::Bottom);
        assert_eq!(f(DT_SINGLELINE | DT_VCENTER | DT_BOTTOM).vertical(), Vertical::Top);
        assert_eq!(f(DT_VCENTER).vertical(), Vertical::Top, "only a single line");
        assert_eq!(f(DT_BOTTOM).vertical(), Vertical::Top);
        assert!(!f(DT_SINGLELINE | DT_WORDBREAK).wraps());
        assert!(f(DT_WORDBREAK).wraps());
        assert!(f(DT_PREFIXONLY).only_underline());
        assert!(!f(DT_HIDEPREFIX | DT_PREFIXONLY).only_underline() && !f(DT_HIDEPREFIX | DT_PREFIXONLY).underline());
        assert!(!f(DT_NOPREFIX | DT_PREFIXONLY).only_underline() && !f(DT_NOPREFIX | DT_PREFIXONLY).prefix());
        assert!(!f(DT_NOPREFIX | DT_HIDEPREFIX).prefix());
        assert_eq!(f(DT_EXTERNALLEADING).leading(2), 2);
        assert_eq!(f(0).leading(2), 0);
        // Bits 22 to 31 have no meaning.
        let odd = f(0xFFC0_0000 | DT_SINGLELINE | DT_BOTTOM | DT_RIGHT);
        assert_eq!((odd.horizontal(), odd.vertical(), odd.wraps()), (Horizontal::Right, Vertical::Bottom, false));
    }

    #[test]
    fn lines_are_laid_out_while_their_top_is_inside() {
        // lines: three lines of 16.
        assert_eq!(lines_laid_out(3, 16, 31, false), 2);
        assert_eq!(lines_laid_out(3, 16, 32, false), 3);
        assert_eq!(lines_laid_out(3, 16, 33, false), 3);
        assert_eq!(lines_laid_out(3, 16, 15, false), 1);
        assert_eq!(lines_laid_out(3, 16, 16, false), 2);
        assert_eq!(lines_laid_out(3, 16, 0, false), 1);
        assert_eq!(lines_laid_out(3, 16, -5, false), 1, "the first line always");
        assert_eq!(lines_laid_out(3, 16, 1000, false), 3);
        // editcontrol: only lines that fit whole, the first one always.
        assert_eq!(lines_laid_out(2, 16, 31, true), 1);
        assert_eq!(lines_laid_out(2, 16, 32, true), 2);
        assert_eq!(lines_laid_out(3, 16, 47, true), 2);
        assert_eq!(lines_laid_out(3, 16, 48, true), 3);
        assert_eq!(lines_laid_out(3, 16, 24, true), 1);
        assert_eq!(lines_laid_out(1, 16, 15, true), 1);
        assert_eq!(lines_laid_out(1, 16, 0, true), 1);
    }

    #[test]
    fn half_rounds_down() {
        assert_eq!((half(52), half(7), half(-7), half(0)), (26, 3, -4, 0));
    }
}
