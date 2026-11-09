//! Outline text for logical fonts (ADR 0142): `poly-font` rasterises the
//! DejaVu face the font mapper chose, in grey scale, at whole-pixel pen
//! positions as GDI places glyphs.
//!
//! - [`FontFiles`] holds the font files of Poly's font store (ADR 0105),
//!   read on first use through a loader the DLL supplies (the files are
//!   `C:\Windows\Fonts\DejaVu*.ttf`); a missing file means the bitmap faces
//!   draw instead.
//! - [`Outline`] is what `CreateFontIndirectW` makes of a `LOGFONTW`: the
//!   file, the pixel size and the `TEXTMETRICW` (height conventions of
//!   `LOGFONTW`'s `lfHeight` in the Microsoft Learn reference: positive is
//!   the cell height, negative the character height, zero a default).
//! - [`Typeface`] measures and draws with either an outline or a stock
//!   bitmap [`Face`].

use crate::font::{Face, TextMetric};
use crate::fontmap::{Class, LogFont, Mapping};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_font::{Font, GlyphCache, GlyphId, GlyphKey, RenderMode, render_glyph_scaled};

/// Reads one file of the font store by its file name (`DejaVuSans.ttf`).
pub type Loader = fn(&str) -> Option<Vec<u8>>;

/// The cell height (ascent plus descent) of a default-size font
/// (`lfHeight` 0): that of the 13-pixel `DEFAULT_GUI_FONT`.
pub const DEFAULT_CELL: i32 = 13;

/// The largest pixel size per em a logical font gets.
pub const MAX_PPEM: i32 = 2000;

/// File names of the store, indexed `class * 4 + bold * 2 + italic`.
const FILES: [&str; 12] = [
    "DejaVuSans.ttf",
    "DejaVuSans-Oblique.ttf",
    "DejaVuSans-Bold.ttf",
    "DejaVuSans-BoldOblique.ttf",
    "DejaVuSerif.ttf",
    "DejaVuSerif-Italic.ttf",
    "DejaVuSerif-Bold.ttf",
    "DejaVuSerif-BoldItalic.ttf",
    "DejaVuSansMono.ttf",
    "DejaVuSansMono-Oblique.ttf",
    "DejaVuSansMono-Bold.ttf",
    "DejaVuSansMono-BoldOblique.ttf",
];

fn slot_of(class: Class, bold: bool, italic: bool) -> usize {
    let c = match class {
        Class::Sans => 0,
        Class::Serif => 1,
        Class::Mono => 2,
    };
    c * 4 + usize::from(bold) * 2 + usize::from(italic)
}

enum Slot {
    Unread,
    Missing,
    Loaded(Arc<Vec<u8>>),
}

/// The font store's files, read on first use.
pub struct FontFiles {
    loader: Option<Loader>,
    slots: [Slot; 12],
}

impl core::fmt::Debug for FontFiles {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let loaded = self.slots.iter().filter(|s| matches!(s, Slot::Loaded(_))).count();
        f.debug_struct("FontFiles").field("loaded", &loaded).finish_non_exhaustive()
    }
}

impl Default for FontFiles {
    fn default() -> Self {
        FontFiles::new()
    }
}

impl FontFiles {
    pub fn new() -> FontFiles {
        FontFiles { loader: None, slots: core::array::from_fn(|_| Slot::Unread) }
    }

    /// Where the files come from (`gdi32.dll` reads `C:\Windows\Fonts`).
    /// Files that were missing are tried again.
    pub fn set_loader(&mut self, loader: Loader) {
        self.loader = Some(loader);
        for s in &mut self.slots {
            if matches!(s, Slot::Missing) {
                *s = Slot::Unread;
            }
        }
    }

    /// Put a file in the store by hand (tests, or a caller that has the
    /// bytes already). `None` if the bytes are not a font with outlines.
    pub fn add(&mut self, class: Class, bold: bool, italic: bool, data: Vec<u8>) -> bool {
        let ok = Font::parse(&data).is_ok_and(|f| f.outline_format() != poly_font::OutlineFormat::None);
        self.slots[slot_of(class, bold, italic)] = if ok { Slot::Loaded(Arc::new(data)) } else { Slot::Missing };
        ok
    }

    fn load(&mut self, i: usize) -> bool {
        if matches!(self.slots[i], Slot::Unread) {
            let data = self.loader.and_then(|l| l(FILES[i]));
            let ok = data
                .as_ref()
                .is_some_and(|d| Font::parse(d).is_ok_and(|f| f.outline_format() != poly_font::OutlineFormat::None));
            self.slots[i] = match data {
                Some(d) if ok => Slot::Loaded(Arc::new(d)),
                _ => Slot::Missing,
            };
        }
        matches!(self.slots[i], Slot::Loaded(_))
    }

    /// The file for a class and style: the exact one, else the same weight
    /// upright (Poly does not slant letters itself), else the regular one.
    /// The slot number identifies the file for [`FontFiles::data`].
    pub fn find(&mut self, class: Class, bold: bool, italic: bool) -> Option<u8> {
        let tries = [(bold, italic), (bold, false), (false, false)];
        for (b, i) in tries {
            let slot = slot_of(class, b, i);
            if self.load(slot) {
                return Some(slot as u8);
            }
        }
        None
    }

    /// The bytes of a slot [`FontFiles::find`] returned.
    pub fn data(&self, slot: u8) -> Option<Arc<Vec<u8>>> {
        match self.slots.get(usize::from(slot))? {
            Slot::Loaded(d) => Some(Arc::clone(d)),
            _ => None,
        }
    }
}

/// A logical font that is drawn from a font file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outline {
    /// Which file ([`FontFiles::data`]).
    pub slot: u8,
    /// Pixels per em, vertically and horizontally (`lfWidth` makes them
    /// differ).
    pub ppem: i32,
    pub ppem_x: i32,
    pub tm: TextMetric,
}

/// A font metric rounded from font units at `ppem` pixels per em.
fn scaled(units: i32, ppem: i32, upem: i32) -> i32 {
    let n = i64::from(units) * i64::from(ppem) * 2 + i64::from(upem) * i64::from(units.signum());
    (n / (2 * i64::from(upem))) as i32
}

/// Ascent and descent in pixels at `ppem`.
fn cell(font: &Font<'_>, ppem: i32) -> (i32, i32) {
    let v = font.vmetrics();
    let upem = i32::from(v.units_per_em).max(1);
    let a = scaled(i32::from(v.ascender), ppem, upem).max(0);
    let d = scaled(-i32::from(v.descender), ppem, upem).max(0);
    (a, d)
}

impl Outline {
    /// Make the outline font a `LOGFONTW` asks for, if the file exists.
    ///
    /// Size: a negative `lfHeight` is the character height (the em); a
    /// positive one the cell height (ascent plus descent), for which the
    /// largest em whose cell fits is taken; 0 is a cell of
    /// [`DEFAULT_CELL`]. Ascent and descent are the font's `hhea` values
    /// scaled and rounded; internal leading is the cell less the em.
    pub fn new(files: &mut FontFiles, lf: &LogFont, m: &Mapping) -> Option<Outline> {
        let slot = files.find(m.class, m.bold, m.italic)?;
        let data = files.data(slot)?;
        let font = Font::parse(&data).ok()?;
        let ppem = pick_ppem(&font, lf.height);
        let upem = i32::from(font.units_per_em()).max(1);
        let (ascent, descent) = cell(&font, ppem);
        let height = ascent + descent;
        let a_gid = |c: char| font.glyph_index(c).unwrap_or(GlyphId(0));
        // The average width of the lowercase letters, in font units.
        let (mut sum, mut n) = (0i64, 0i64);
        for c in 'a'..='z' {
            sum += i64::from(font.advance(a_gid(c)));
            n += 1;
        }
        let avg_units = ((sum + n / 2) / n.max(1)).max(1) as i32;
        let ppem_x = if lf.width > 0 {
            scaled(lf.width.saturating_mul(upem), 1, avg_units).clamp((ppem / 4).max(1), (ppem * 4).min(MAX_PPEM))
        } else {
            ppem
        };
        let bb = font.bounding_box();
        let max_units = i32::from(bb[2]) - i32::from(bb[0]);
        let line_gap = i32::from(font.vmetrics().line_gap);
        let bold = lf.weight >= 600;
        let family = match m.class {
            Class::Sans => 0x20,
            Class::Serif => 0x10,
            Class::Mono => 0x30,
        };
        // TMPF_FIXED_PITCH (1) means *variable* pitch, TMPF_VECTOR 2,
        // TMPF_TRUETYPE 4 (`wingdi.h`).
        let pitch = if m.class == Class::Mono { 0 } else { 1 };
        let tm = TextMetric {
            height,
            ascent,
            descent,
            internal_leading: (height - ppem).max(0),
            external_leading: scaled(line_gap, ppem, upem).max(0),
            ave_char_width: scaled(avg_units, ppem_x, upem).max(1),
            max_char_width: scaled(max_units, ppem_x, upem).max(1),
            weight: if bold { 700 } else { 400 },
            overhang: 0,
            aspect_x: 96,
            aspect_y: 96,
            first_char: 0x20,
            last_char: 0xFFFD,
            default_char: 0xFFFD,
            break_char: 0x20,
            italic: u8::from(lf.italic),
            underlined: u8::from(lf.underline),
            struck_out: u8::from(lf.strike_out),
            pitch_and_family: family | 4 | 2 | pitch,
            char_set: if lf.char_set == 1 { 0 } else { lf.char_set },
        };
        Some(Outline { slot, ppem, ppem_x, tm })
    }
}

fn pick_ppem(font: &Font<'_>, height: i32) -> i32 {
    if height < 0 {
        return height.saturating_neg().clamp(1, MAX_PPEM);
    }
    let want = if height == 0 { DEFAULT_CELL } else { height };
    let v = font.vmetrics();
    let upem = i32::from(v.units_per_em).max(1);
    let total = (i32::from(v.ascender) - i32::from(v.descender)).max(1);
    let fits = |p: i32| {
        let (a, d) = cell(font, p);
        a + d <= want
    };
    let mut p = (i64::from(want) * i64::from(upem) / i64::from(total)).clamp(1, i64::from(MAX_PPEM)) as i32;
    while p > 1 && !fits(p) {
        p -= 1;
    }
    while p < MAX_PPEM && fits(p + 1) {
        p += 1;
    }
    p
}

/// A font ready to measure and draw with: a stock bitmap face or an
/// outline with its parsed file.
pub struct Typeface<'a> {
    pub face: Face,
    outline: Option<(Outline, Font<'a>)>,
}

impl core::fmt::Debug for Typeface<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Typeface")
            .field("face", &self.face)
            .field("outline", &self.outline.as_ref().map(|(o, _)| o))
            .finish()
    }
}

/// The advance of a control character (`DrawTextW.yaml`: one pixel).
const CONTROL_ADVANCE: i32 = 1;

impl<'a> Typeface<'a> {
    /// `data` is the file of `outline`; a file that does not parse, or no
    /// outline, gives the bitmap `face`.
    pub fn new(face: Face, outline: Option<Outline>, data: Option<&'a [u8]>) -> Typeface<'a> {
        let outline = outline.zip(data).and_then(|(o, d)| Font::parse(d).ok().map(|f| (o, f)));
        Typeface { face, outline }
    }

    pub fn is_outline(&self) -> bool {
        self.outline.is_some()
    }

    pub fn metrics(&self) -> TextMetric {
        self.outline.as_ref().map_or_else(|| self.face.metrics(), |(o, _)| o.tm)
    }

    fn glyph(font: &Font<'_>, c: u16) -> GlyphId {
        char::from_u32(u32::from(c)).and_then(|ch| font.glyph_index(ch)).unwrap_or(GlyphId(0))
    }

    /// The advance width of `c` in pixels: the font's advance scaled to the
    /// em and rounded, as GDI's unhinted TrueType advances are whole pixels.
    pub fn width(&self, c: u16) -> i32 {
        match &self.outline {
            None => self.face.width(c),
            Some((o, font)) => {
                if c < 0x20 {
                    return CONTROL_ADVANCE;
                }
                let adv = i32::from(font.advance(Self::glyph(font, c)));
                scaled(adv, o.ppem_x, i32::from(font.units_per_em()).max(1))
            }
        }
    }

    /// The extent of `text`: (width, height); an empty string is 0x0.
    pub fn extent(&self, text: &[u16]) -> (i32, i32) {
        if text.is_empty() {
            return (0, 0);
        }
        (text.iter().map(|&c| self.width(c)).sum(), self.metrics().height)
    }

    /// Draw `text` with its first pen position at x `x0`, the baseline at
    /// y `baseline`. `put(x, y, coverage)` gets every pixel with ink, from
    /// 1 to 255. `advances` (`ExtTextOutW`'s `lpDx`) gives each
    /// character's advance instead of the font's. Returns the pen's x at
    /// the end.
    pub fn draw_outline(
        &self,
        cache: &mut GlyphCache,
        text: &[u16],
        x0: i32,
        baseline: i32,
        advances: Option<&[i32]>,
        mut put: impl FnMut(i32, i32, u8),
    ) -> i32 {
        let Some((o, font)) = &self.outline else { return x0 };
        let mut x = x0;
        for (i, &c) in text.iter().enumerate() {
            let gid = Self::glyph(font, c);
            if c >= 0x20 && !(c == 0x20 || c == 0xA0) {
                let rendered;
                let bmp = if o.ppem_x == o.ppem {
                    cache.get(font, GlyphKey::new(u32::from(o.slot), o.ppem as f32, gid, 0, RenderMode::Gray))
                } else {
                    rendered = render_glyph_scaled(font, gid, o.ppem_x as f32, o.ppem as f32, 0.0, RenderMode::Gray);
                    &rendered
                };
                for row in 0..bmp.height {
                    for col in 0..bmp.width {
                        let v = bmp.data[(row * bmp.width + col) as usize];
                        if v != 0 {
                            put(x + bmp.left + col as i32, baseline + bmp.top + row as i32, v);
                        }
                    }
                }
            }
            x += match advances.and_then(|a| a.get(i)) {
                Some(&dx) => dx,
                None => self.width(c),
            };
        }
        x
    }

    /// The underline's top row and thickness, relative to the baseline
    /// (rows below it), and the strike-out's: a line a twelfth of the em
    /// thick (at least one pixel), the underline just below the baseline,
    /// the strike-out through the middle of the lowercase.
    pub fn line_metrics(&self) -> ((i32, i32), (i32, i32)) {
        let tm = self.metrics();
        let em = tm.height - tm.internal_leading;
        let thick = (em / 12).max(1);
        let under = ((tm.descent / 3).max(1), thick);
        let strike = (-(tm.ascent * 3 / 10 + thick / 2).max(1), thick);
        (under, strike)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fontmap::{LOGFONT_SIZE, map};

    /// DejaVu from the font store, when it is fetched (`POLY_FONT_DIR`, as
    /// for `poly-font`'s tests, or `target/fonts/dejavu-2.37`).
    pub(crate) fn dejavu(file: &str) -> Option<Vec<u8>> {
        let dir = std::env::var("POLY_FONT_DIR").ok()?;
        std::fs::read(std::path::Path::new(&dir).join(file)).ok()
    }

    pub(crate) fn store() -> Option<FontFiles> {
        let mut f = FontFiles::new();
        f.set_loader(dejavu);
        f.find(Class::Sans, false, false)?;
        Some(f)
    }

    fn lf(name: &str, height: i32, weight: i32) -> LogFont {
        let mut b = [0u8; LOGFONT_SIZE];
        b[0..4].copy_from_slice(&height.to_le_bytes());
        b[16..20].copy_from_slice(&weight.to_le_bytes());
        for (i, c) in name.encode_utf16().take(31).enumerate() {
            b[28 + 2 * i..30 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        LogFont::parse(&b).unwrap()
    }

    #[test]
    fn scaling_rounds_to_the_nearest_pixel() {
        assert_eq!(scaled(1901, 12, 2048), 11);
        assert_eq!(scaled(483, 12, 2048), 3);
        assert_eq!(scaled(1024, 16, 2048), 8);
        assert_eq!(scaled(-1024, 16, 2048), -8);
        assert_eq!(scaled(0, 16, 2048), 0);
    }

    #[test]
    fn heights_follow_the_lfheight_convention() {
        let Some(mut files) = store() else {
            std::eprintln!("skipped: no DejaVu fonts (POLY_FONT_DIR)");
            return;
        };
        let mut o = |h: i32| {
            let l = lf("Segoe UI", h, 400);
            Outline::new(&mut files, &l, &map(&l)).unwrap()
        };
        // Negative: the em.
        assert_eq!(o(-16).ppem, 16);
        assert_eq!(o(-12).ppem, 12);
        // Positive: the cell, never taller than asked.
        for h in [8, 13, 16, 20, 33] {
            let m = o(h);
            assert!(m.tm.height <= h && m.tm.height >= h - 1, "cell {h}: {}", m.tm.height);
            assert_eq!(m.tm.ascent + m.tm.descent, m.tm.height);
        }
        // Zero: the default cell.
        assert_eq!(o(0).tm.height, DEFAULT_CELL);
        let t = o(-12).tm;
        assert_eq!((t.ascent, t.descent, t.height), (11, 3, 14));
        assert_eq!(t.internal_leading, 2);
        assert_eq!(t.pitch_and_family, 0x20 | 4 | 2 | 1);
        assert_eq!((t.weight, t.aspect_x, t.aspect_y), (400, 96, 96));
    }

    #[test]
    fn a_wider_lfwidth_widens_the_advances() {
        let Some(mut files) = store() else { return };
        let mut b = [0u8; LOGFONT_SIZE];
        b[0..4].copy_from_slice(&(-16i32).to_le_bytes());
        for (i, c) in "Arial".encode_utf16().enumerate() {
            b[28 + 2 * i..30 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        let natural = LogFont::parse(&b).unwrap();
        b[4..8].copy_from_slice(&16i32.to_le_bytes());
        let wide = LogFont::parse(&b).unwrap();
        let on = Outline::new(&mut files, &natural, &map(&natural)).unwrap();
        let ow = Outline::new(&mut files, &wide, &map(&wide)).unwrap();
        assert!(ow.ppem_x > on.ppem_x && ow.tm.ave_char_width > on.tm.ave_char_width);
        assert_eq!(ow.ppem, on.ppem);
    }

    #[test]
    fn a_missing_file_falls_back_to_the_regular_one() {
        let Some(regular) = dejavu("DejaVuSans.ttf") else { return };
        let mut files = FontFiles::new();
        assert!(files.add(Class::Sans, false, false, regular));
        let slot = files.find(Class::Sans, true, true).unwrap();
        assert_eq!(slot, 0);
        assert!(files.find(Class::Mono, false, false).is_none());
        assert!(!files.add(Class::Mono, false, false, alloc::vec![0; 40]));
    }
}
