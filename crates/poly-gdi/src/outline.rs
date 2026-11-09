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
use crate::fontmap::{Class, LogFont};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_font::{Font, GlyphCache, GlyphId, GlyphKey, RenderMode};

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
    /// The built-in proportional and fixed-pitch fonts ([`crate::builtin`]),
    /// made on first use.
    builtin: [Option<Arc<Vec<u8>>>; 2],
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
        FontFiles { loader: None, slots: core::array::from_fn(|_| Slot::Unread), builtin: [None, None] }
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

    /// The built-in font of a pitch.
    pub fn builtin(&mut self, mono: bool) -> Arc<Vec<u8>> {
        let slot = &mut self.builtin[usize::from(mono)];
        Arc::clone(slot.get_or_insert_with(|| Arc::new(crate::builtin::build(mono))))
    }

    /// The bytes of a slot [`FontFiles::find`] returned.
    pub fn data(&self, slot: u8) -> Option<Arc<Vec<u8>>> {
        match self.slots.get(usize::from(slot))? {
            Slot::Loaded(d) => Some(Arc::clone(d)),
            _ => None,
        }
    }
}

/// Which font file an [`Outline`] draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    /// A file of Poly's font store ([`FontFiles`] slot).
    Installed(u8),
    /// The built-in font ([`crate::builtin`]): `true` the fixed-pitch one.
    Builtin(bool),
    /// A font the program added to its process (`AddFontMemResourceEx`,
    /// `AddFontResourceExW`): [`crate::fonttab::FontTable`] id.
    Private(u32),
}

impl Src {
    /// A number that tells the font apart in the glyph cache.
    pub fn key(self) -> u32 {
        match self {
            Src::Installed(s) => u32::from(s),
            Src::Builtin(mono) => 0x80 + u32::from(mono),
            Src::Private(id) => 0x100 + id,
        }
    }
}

/// A pixel size per em as the fraction `n / d` (a positive `lfHeight` asks
/// for a cell of that height, which makes the em fractional).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ratio {
    pub n: i64,
    pub d: i64,
}

impl Ratio {
    /// `v` times this size per em, divided by `upem`, to the nearest pixel
    /// (halves up): what GDI makes of a font-unit value.
    pub fn px(self, v: i64, upem: i64) -> i32 {
        round_div(v * self.n, upem * self.d) as i32
    }

    /// The size in pixels per em.
    pub fn em(self) -> f32 {
        self.n as f32 / self.d as f32
    }
}

/// `a / b` to the nearest integer, halves up (`b` > 0).
fn round_div(a: i64, b: i64) -> i64 {
    (2 * a + b).div_euclid(2 * b)
}

/// What the face a logical font got says about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceStyle {
    pub bold: bool,
    pub italic: bool,
    /// The face's weight (`OS/2.usWeightClass`).
    pub weight: i32,
}

/// A logical font that is drawn from a font file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outline {
    pub src: Src,
    /// Pixels per em, vertically and horizontally (`lfWidth` makes the
    /// horizontal one differ).
    pub ey: Ratio,
    pub ex: Ratio,
    pub upem: i32,
    pub tm: TextMetric,
    /// The face has no bold (italic) style: the glyphs are widened by a
    /// pixel (slanted).
    pub synth_bold: bool,
    pub synth_italic: bool,
    /// `lfQuality` NONANTIALIASED or ANTIALIASED (the oracle drew both as
    /// pixels that are all or nothing, `TextOutW.yaml`).
    pub bilevel: bool,
    /// `lfQuality` DEFAULT, CLEARTYPE or CLEARTYPE_NATURAL: the three
    /// colour channels of a pixel get their own coverage (the mean of three
    /// subpixels, three per pixel).
    pub lcd: bool,
    /// `lfEscapement` in tenths of a degree, 0 to 3599.
    pub escapement: i32,
    /// Underline: the row of its top below the baseline, and its
    /// thickness; strike-out: its top above the baseline, and thickness.
    pub underline: (i32, i32),
    pub strike: (i32, i32),
}

/// The default size of `lfHeight` 0: 16 pixels per em.
pub const DEFAULT_EM: i64 = 16;
/// The smallest em: GDI makes a 1 pixel font 2 pixels.
pub const MIN_EM: i64 = 2;

impl Outline {
    /// The logical font a `LOGFONTW` asks for, drawn from `font`.
    ///
    /// Size (`LOGFONTW`, Microsoft Learn; `CreateFontIndirectW.yaml`): a
    /// negative `lfHeight` is the em in pixels, a positive one the cell
    /// height (the ascent plus the descent, scaled together so that they add
    /// up to it), 0 is 16 pixels per em; an em below 2 pixels is 2. Metrics
    /// are the font's `OS/2` `usWinAscent` and `usWinDescent`, `hhea` line
    /// gap and `OS/2` average width, scaled and rounded to the nearest
    /// pixel (halves up); the advance of a glyph likewise; `lfWidth` scales
    /// the horizontal size so that the average width is that many pixels.
    pub fn new(font: &Font<'_>, src: Src, lf: &LogFont, face: FaceStyle, class: Option<Class>) -> Outline {
        let upem = i64::from(font.units_per_em()).max(1);
        let gm = font.gdi_metrics();
        let (mut wa, mut wd) = (i64::from(gm.win_ascent), i64::from(gm.win_descent));
        if wa + wd == 0 {
            let v = font.vmetrics();
            wa = i64::from(v.ascender).max(0);
            wd = i64::from(v.descender).abs();
            if wa + wd == 0 {
                wa = upem * 3 / 4;
                wd = upem / 4;
            }
        }
        let mut ey = match lf.height {
            h if h < 0 => Ratio { n: i64::from(h).abs(), d: 1 },
            0 => Ratio { n: DEFAULT_EM, d: 1 },
            h => Ratio { n: i64::from(h) * upem, d: wa + wd },
        };
        let mut em = round_div(ey.n, ey.d);
        if em < MIN_EM {
            ey = Ratio { n: MIN_EM, d: 1 };
            em = MIN_EM;
        }
        if em > i64::from(MAX_PPEM) {
            ey = Ratio { n: i64::from(MAX_PPEM), d: 1 };
            em = i64::from(MAX_PPEM);
        }
        let ascent = ey.px(wa, upem);
        let descent = ey.px(wd, upem);
        let height = ascent + descent;
        // The average width: OS/2 `xAvgCharWidth`, else the mean advance of
        // the lowercase letters.
        let avg_units = if gm.avg_char_width > 0 {
            i64::from(gm.avg_char_width)
        } else {
            let (mut sum, mut n) = (0i64, 0i64);
            for c in 'a'..='z' {
                sum += i64::from(font.advance(font.glyph_index(c).unwrap_or(GlyphId(0))));
                n += 1;
            }
            ((sum + n / 2) / n.max(1)).max(1)
        };
        let natural_avg = i64::from(ey.px(avg_units, upem)).max(1);
        let ex = if lf.width != 0 {
            let w = i64::from(lf.width).abs().min(1 << 20);
            Ratio { n: ey.n * w, d: ey.d * natural_avg }
        } else {
            ey
        };
        let synth_bold = lf.weight >= 600 && !face.bold;
        let synth_italic = lf.italic && !face.italic;
        let bold_px = i32::from(synth_bold);
        let max_units = if gm.advance_width_max > 0 {
            i64::from(gm.advance_width_max)
        } else {
            let bb = font.bounding_box();
            i64::from(bb[2]) - i64::from(bb[0])
        };
        let (lo, hi) = font.cmap_range().unwrap_or((0x20, 0xFFFD));
        let first = lo.clamp(0x20, 0xFFFF) as u16;
        let last = hi.min(0xFFFF).max(u32::from(first)) as u16;
        let (fixed, family) = match class {
            Some(Class::Sans) => (false, 0x20),
            Some(Class::Serif) => (false, 0x10),
            Some(Class::Mono) => (true, 0x30),
            None => {
                let fixed = gm.panose_proportion == 9 || font.style().monospace;
                (fixed, if fixed { 0x30 } else { 0 })
            }
        };
        // TMPF_FIXED_PITCH (1) means *variable* pitch, TMPF_VECTOR 2,
        // TMPF_TRUETYPE 4 (`wingdi.h`).
        let tm = TextMetric {
            height,
            ascent,
            descent,
            internal_leading: (height - em as i32).max(0),
            external_leading: ey.px(i64::from(gm.line_gap), upem).max(0),
            ave_char_width: ex.px(avg_units, upem).max(1) + bold_px,
            max_char_width: ex.px(max_units, upem).max(1) + bold_px,
            weight: if synth_bold || face.bold { face.weight.max(700) } else { face.weight.clamp(1, 599) },
            overhang: 0,
            aspect_x: 96,
            aspect_y: 96,
            first_char: first,
            last_char: last,
            default_char: first.saturating_sub(1),
            break_char: 0x20,
            italic: if lf.italic || face.italic { 255 } else { 0 },
            underlined: if lf.underline { 255 } else { 0 },
            struck_out: if lf.strike_out { 255 } else { 0 },
            pitch_and_family: family | 4 | 2 | u8::from(!fixed),
            char_set: if lf.char_set == 1 { 0 } else { lf.char_set },
        };
        // The font's own underline and strike-out (the tops of the lines).
        let fallback_thick = (em as i32 / 12).max(1);
        let thick = |units: i16| if units > 0 { ey.px(i64::from(units), upem).max(1) } else { fallback_thick };
        let under_top = if gm.underline_position < 0 || gm.underline_thickness > 0 {
            ey.px(-i64::from(gm.underline_position), upem)
        } else {
            (descent / 3).max(1)
        };
        let strike_top = if gm.strikeout_size > 0 || gm.strikeout_position > 0 {
            ey.px(i64::from(gm.strikeout_position), upem).max(1)
        } else {
            (ascent * 3 / 10).max(1)
        };
        Outline {
            src,
            ey,
            ex,
            upem: upem as i32,
            tm,
            synth_bold,
            synth_italic,
            bilevel: matches!(lf.quality, 3 | 4),
            lcd: matches!(lf.quality, 0 | 5 | 6),
            escapement: lf.escapement.rem_euclid(3600),
            underline: (under_top, thick(gm.underline_thickness)),
            strike: (strike_top, thick(gm.strikeout_size)),
        }
    }
}

/// The slant of a synthesised italic (the tangent of the angle).
const SLANT: f32 = 0.2;

/// Sine and cosine of an angle in tenths of a degree: exact at the quarter
/// turns, a Taylor series elsewhere.
pub fn sin_cos(tenths: i32) -> (f32, f32) {
    let t = tenths.rem_euclid(3600);
    match t {
        0 => return (0.0, 1.0),
        900 => return (1.0, 0.0),
        1800 => return (0.0, -1.0),
        2700 => return (-1.0, 0.0),
        _ => {}
    }
    // Reduce to 0..90 degrees by symmetry.
    let (quarter, r) = (t / 900, t % 900);
    let x = r as f32 * core::f32::consts::PI / 1800.0;
    let x2 = x * x;
    let sin = x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0))));
    let cos = 1.0 - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0 * (1.0 - x2 / 56.0 * (1.0 - x2 / 90.0))));
    match quarter {
        0 => (sin, cos),
        1 => (cos, -sin),
        2 => (-sin, -cos),
        _ => (-cos, sin),
    }
}

fn floor_f(x: f32) -> f32 {
    let t = x as i32 as f32;
    if t > x { t - 1.0 } else { t }
}

fn ceil_f(x: f32) -> f32 {
    -floor_f(-x)
}

fn round_f(x: f32) -> i32 {
    if x < 0.0 { -((-x + 0.5) as i32) } else { (x + 0.5) as i32 }
}

/// A glyph rendered through an affine map: `m` takes font units (x right,
/// y up) to device pixels (y down): `x' = m[0] x + m[2] y`,
/// `y' = m[1] x + m[3] y`. Grey coverage; the origin is the pen position.
fn render_xf(font: &Font<'_>, gid: GlyphId, m: [f32; 4], lcd: bool) -> poly_font::Bitmap {
    let mode = if lcd { RenderMode::Lcd } else { RenderMode::Gray };
    let empty = poly_font::Bitmap { mode, ..poly_font::Bitmap::default() };
    let Some(path) = font.outline(gid) else { return empty };
    let dev = path.map(|p| poly_font::Point::new(m[0] * p.x + m[2] * p.y, m[1] * p.x + m[3] * p.y));
    let Some(b) = dev.bounds() else { return empty };
    let pad = if lcd { 1.0 } else { 0.0 };
    let (left, top) = (floor_f(b.x_min) - pad, floor_f(b.y_min));
    let (right, bottom) = (ceil_f(b.x_max) + pad, ceil_f(b.y_max));
    let (w, h) = (right - left, bottom - top);
    if !(w > 0.0 && h > 0.0 && w <= 4096.0 && h <= 4096.0) {
        return empty;
    }
    let (w, h) = (w as usize, h as usize);
    let data = if lcd {
        let mut r = poly_font::Rasterizer::new(w * 3, h);
        r.path(&dev, |p| poly_font::Point::new((p.x - left) * 3.0, p.y - top));
        lcd_filter(&r.coverage_f32(), w, h)
    } else {
        let mut r = poly_font::Rasterizer::new(w, h);
        r.path(&dev, |p| poly_font::Point::new(p.x - left, p.y - top));
        r.coverage()
    };
    poly_font::Bitmap { width: w as u32, height: h as u32, left: left as i32, top: top as i32, mode, data }
}

/// The gain after the filter: thin unhinted stems still reach the text
/// colour, while the 2/3 of a box's edge pixel stays below 1.
const CONTRAST: f32 = 1.4;

/// Coverage of three subpixels per pixel to the three channels: each
/// subpixel is the mean of itself and its two neighbours.
fn lcd_filter(sub: &[f32], w: usize, h: usize) -> Vec<u8> {
    const WEIGHTS: [f32; 3] = [1.0 / 3.0; 3];
    let sw = w * 3;
    let mut out = alloc::vec![0u8; sw * h];
    for y in 0..h {
        let row = &sub[y * sw..(y + 1) * sw];
        for x in 0..sw {
            let mut v = 0.0;
            for (k, wt) in WEIGHTS.iter().enumerate() {
                if let Some(&c) = (x + k).checked_sub(1).and_then(|i| row.get(i)) {
                    v += c * wt;
                }
            }
            out[y * sw + x] = ((v * CONTRAST).min(1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

/// All or nothing: coverage of at least half a pixel is ink.
fn threshold(b: &mut poly_font::Bitmap) {
    let (w, h) = (b.width as usize, b.height as usize);
    let src = b.data.clone();
    for v in &mut b.data {
        *v = if *v >= 128 { 255 } else { 0 };
    }
    // Dropout control: a row the outline touches but that has no pixel set
    // gets the pixel with the most coverage (an apex does not vanish).
    for y in 0..h {
        let row = y * w..(y + 1) * w;
        if src[row.clone()].iter().any(|&v| v != 0)
            && b.data[row.clone()].iter().all(|&v| v == 0)
            && let Some(x) = (0..w).max_by_key(|&x| (src[y * w + x], core::cmp::Reverse(x)))
        {
            b.data[y * w + x] = 255;
        }
    }
}

/// A synthesised bold: every ink pixel also inks the pixel to its right.
fn embolden(b: &poly_font::Bitmap) -> poly_font::Bitmap {
    if b.is_empty() {
        return b.clone();
    }
    let (w, h, ch) = (b.width as usize, b.height as usize, b.mode.channels());
    let mut out = poly_font::Bitmap { width: b.width + 1, data: alloc::vec![0; (w + 1) * h * ch], ..b.clone() };
    for y in 0..h {
        for x in 0..w {
            for c in 0..ch {
                let v = b.data[(y * w + x) * ch + c];
                let o = &mut out.data[(y * (w + 1) + x) * ch + c];
                *o = (*o).max(v);
                let o = &mut out.data[(y * (w + 1) + x + 1) * ch + c];
                *o = (*o).max(v);
            }
        }
    }
    out
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

    /// The outline's description, if this is one.
    pub fn outline(&self) -> Option<&Outline> {
        self.outline.as_ref().map(|(o, _)| o)
    }

    pub fn metrics(&self) -> TextMetric {
        self.outline.as_ref().map_or_else(|| self.face.metrics(), |(o, _)| o.tm)
    }

    fn glyph(font: &Font<'_>, c: u16) -> GlyphId {
        char::from_u32(u32::from(c)).and_then(|ch| font.glyph_index(ch)).unwrap_or(GlyphId(0))
    }

    /// `gid` as a glyph of this font (`.notdef` beyond the font).
    fn clamp_glyph(font: &Font<'_>, gid: u16) -> GlyphId {
        if gid < font.num_glyphs() { GlyphId(gid) } else { GlyphId(0) }
    }

    fn advance_of(o: &Outline, font: &Font<'_>, g: GlyphId) -> i32 {
        o.ex.px(i64::from(font.advance(g)), i64::from(o.upem)) + i32::from(o.synth_bold)
    }

    /// The advance width of `c` in pixels (`GetCharWidth32W`): the font's
    /// advance scaled and rounded to whole pixels; a character the font
    /// lacks is its `.notdef`.
    pub fn width(&self, c: u16) -> i32 {
        match &self.outline {
            None => self.face.width(c),
            Some((o, font)) => Self::advance_of(o, font, Self::glyph(font, c)),
        }
    }

    /// The width of glyph `gid` (`ETO_GLYPH_INDEX`).
    pub fn glyph_width(&self, gid: u16) -> i32 {
        match &self.outline {
            None => self.face.width(gid),
            Some((o, font)) => Self::advance_of(o, font, Self::clamp_glyph(font, gid)),
        }
    }

    /// The advance of `c` in a string being measured: tab, line feed and
    /// carriage return take no room in an outline font
    /// (`GetTextExtentPoint32W.yaml`).
    pub fn advance(&self, c: u16) -> i32 {
        if self.outline.is_some() && matches!(c, 9 | 10 | 13) { 0 } else { self.width(c) }
    }

    /// The extent of `text`: (width, height); an empty string is 0x0.
    pub fn extent(&self, text: &[u16]) -> (i32, i32) {
        if text.is_empty() {
            return (0, 0);
        }
        (text.iter().map(|&c| self.advance(c)).sum(), self.metrics().height)
    }

    /// The extent of a string of glyph indices.
    pub fn glyph_extent(&self, gids: &[u16]) -> (i32, i32) {
        if gids.is_empty() {
            return (0, 0);
        }
        (gids.iter().map(|&g| self.glyph_width(g)).sum(), self.metrics().height)
    }

    /// Draw `text` (or, with `glyph_index`, glyph indices) with its first
    /// pen position at (`x0`, `y0`) on the baseline. `put(x, y, coverage)`
    /// gets every pixel with ink, from 1 to 255. `advances` (`ExtTextOutW`'s
    /// `lpDx`) gives each character's advance instead of the font's. The
    /// text runs along the font's escapement. Returns the pen's position at
    /// the end.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_outline(
        &self,
        cache: &mut GlyphCache,
        text: &[u16],
        glyph_index: bool,
        x0: i32,
        y0: i32,
        advances: Option<&[i32]>,
        mut put: impl FnMut(i32, i32, [u8; 3]),
    ) -> (i32, i32) {
        let Some((o, font)) = &self.outline else { return (x0, y0) };
        let (sn, cs) = sin_cos(o.escapement);
        let upem = o.upem as f32;
        let (sx, sy) = (o.ex.em() / upem, o.ey.em() / upem);
        let k = if o.synth_italic { SLANT } else { 0.0 };
        // Device x = (X sx + k Y sy) cos - Y sy sin; y = -((X sx + k Y sy) sin + Y sy cos).
        let m = [sx * cs, -sx * sn, k * sy * cs - sy * sn, -(k * sy * sn + sy * cs)];
        let upright = o.escapement == 0 && !o.synth_italic && !o.bilevel && !o.synth_bold && !o.lcd && o.ex == o.ey;
        let mut dist = 0i32;
        for (i, &c) in text.iter().enumerate() {
            let gid = if glyph_index { Self::clamp_glyph(font, c) } else { Self::glyph(font, c) };
            let visible = glyph_index || !(c <= 0x20 || c == 0xA0);
            if visible {
                // The pen along the escapement, rounded to a whole pixel.
                let px = x0 + round_f(dist as f32 * cs);
                let py = y0 - round_f(dist as f32 * sn);
                let rendered;
                let bmp = if upright {
                    cache.get(font, GlyphKey::new(o.src.key(), o.ey.em(), gid, 0, RenderMode::Gray))
                } else {
                    let mut b = render_xf(font, gid, m, o.lcd && !o.bilevel);
                    if o.bilevel {
                        threshold(&mut b);
                    }
                    rendered = if o.synth_bold {
                        let mut e = embolden(&b);
                        if o.bilevel {
                            threshold(&mut e);
                        }
                        e
                    } else {
                        b
                    };
                    &rendered
                };
                let ch = bmp.mode.channels();
                for row in 0..bmp.height {
                    for col in 0..bmp.width {
                        let at = (row * bmp.width + col) as usize * ch;
                        let v = if ch == 3 {
                            [bmp.data[at], bmp.data[at + 1], bmp.data[at + 2]]
                        } else {
                            [bmp.data[at]; 3]
                        };
                        if v != [0; 3] {
                            put(px + bmp.left + col as i32, py + bmp.top + row as i32, v);
                        }
                    }
                }
            }
            dist += match advances.and_then(|a| a.get(i)) {
                Some(&dx) => dx,
                None if glyph_index => self.glyph_width(c),
                None => self.advance(c),
            };
        }
        (x0 + round_f(dist as f32 * cs), y0 - round_f(dist as f32 * sn))
    }

    /// The underline's top row and thickness, relative to the baseline
    /// (rows below it), and the strike-out's (negative: above it): an
    /// outline font's own (`post` and `OS/2`), a line a twelfth of the em
    /// thick for a bitmap face.
    pub fn line_metrics(&self) -> ((i32, i32), (i32, i32)) {
        if let Some((o, _)) = &self.outline {
            return (o.underline, (-o.strike.0, o.strike.1));
        }
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
    use crate::fontmap::LOGFONT_SIZE;

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

    pub(crate) fn lf(name: &str, height: i32, weight: i32) -> LogFont {
        let mut b = [0u8; LOGFONT_SIZE];
        b[0..4].copy_from_slice(&height.to_le_bytes());
        b[16..20].copy_from_slice(&weight.to_le_bytes());
        for (i, c) in name.encode_utf16().take(31).enumerate() {
            b[28 + 2 * i..30 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        LogFont::parse(&b).unwrap()
    }

    fn plain() -> FaceStyle {
        FaceStyle { bold: false, italic: false, weight: 400 }
    }

    #[test]
    fn rounding_is_to_the_nearest_pixel_halves_up() {
        let r = Ratio { n: 16, d: 1 };
        assert_eq!(r.px(1901, 2048), 15);
        assert_eq!(r.px(1024, 2048), 8);
        assert_eq!(r.px(0, 2048), 0);
        assert_eq!(Ratio { n: 10, d: 1 }.px(768, 1024), 8); // 7.5
        assert_eq!(Ratio { n: 10, d: 1 }.px(256, 1024), 3); // 2.5
        assert_eq!(round_div(3, 2), 2);
        assert_eq!(round_div(-3, 2), -1);
    }

    /// `CreateFontIndirectW.yaml`'s table (`cfi_height`): metrics of the
    /// test fonts at many heights, as the oracle gave them.
    #[test]
    fn metrics_follow_the_spec_sheets_table() {
        use crate::testfonts as tf;
        let sans = tf::build(&tf::SANS);
        let tall = tf::build(&tf::TALL);
        let mono = tf::build(&tf::MONO);
        let cell = |data: &[u8], h: i32| {
            let font = Font::parse(data).unwrap();
            let o = Outline::new(&font, Src::Private(1), &lf("PolyTest", h, 400), plain(), None);
            let w = |c: char| o.ex.px(i64::from(font.advance(font.glyph_index(c).unwrap())), i64::from(o.upem));
            let t = o.tm;
            [
                t.height,
                t.ascent,
                t.descent,
                t.internal_leading,
                t.external_leading,
                t.ave_char_width,
                t.max_char_width,
                w('A'),
                w('B'),
                w('W'),
                w('i'),
                w('g'),
                w(' '),
            ]
        };
        // height ascent descent intlead extlead avew maxw | A B W i g space
        assert_eq!(cell(&sans, -1), [3, 2, 1, 1, 0, 1, 2, 2, 1, 2, 1, 1, 1]);
        assert_eq!(cell(&sans, -6), [7, 5, 2, 1, 1, 3, 6, 5, 3, 6, 2, 4, 2]);
        assert_eq!(cell(&sans, -10), [11, 8, 3, 1, 1, 5, 10, 8, 5, 10, 3, 6, 4]);
        assert_eq!(cell(&sans, -16), [16, 12, 4, 0, 2, 8, 16, 12, 8, 16, 4, 10, 6]);
        assert_eq!(cell(&sans, -100), [100, 75, 25, 0, 13, 50, 100, 75, 50, 100, 25, 63, 38]);
        assert_eq!(cell(&sans, 0), [16, 12, 4, 0, 2, 8, 16, 12, 8, 16, 4, 10, 6]);
        assert_eq!(cell(&sans, 10), [11, 8, 3, 1, 1, 5, 10, 8, 5, 10, 3, 6, 4]);
        assert_eq!(cell(&tall, -1), [3, 2, 1, 1, 0, 1, 2, 2, 1, 2, 1, 1, 1]);
        assert_eq!(cell(&tall, -16), [19, 14, 5, 3, 0, 10, 16, 12, 8, 16, 4, 10, 6]);
        assert_eq!(cell(&tall, -100), [119, 88, 31, 19, 0, 63, 100, 75, 50, 100, 25, 63, 38]);
        assert_eq!(cell(&tall, 0), [19, 14, 5, 3, 0, 10, 16, 12, 8, 16, 4, 10, 6]);
        // A positive height is the cell: the em is fractional.
        assert_eq!(cell(&tall, 1), [3, 2, 1, 1, 0, 1, 2, 2, 1, 2, 1, 1, 1]);
        assert_eq!(cell(&tall, 2), [2, 1, 1, 0, 0, 1, 2, 1, 1, 2, 0, 1, 1]);
        assert_eq!(cell(&tall, 16), [16, 12, 4, 3, 0, 8, 13, 10, 7, 13, 3, 8, 5]);
        assert_eq!(cell(&tall, 20), [20, 15, 5, 3, 0, 11, 17, 13, 8, 17, 4, 11, 6]);
        assert_eq!(cell(&tall, 48), [48, 35, 13, 8, 0, 25, 40, 30, 20, 40, 10, 25, 15]);
        assert_eq!(cell(&mono, -16), [16, 12, 4, 0, 2, 10, 16, 10, 10, 10, 10, 10, 10]);
        assert_eq!(cell(&mono, -1), [3, 2, 1, 1, 0, 1, 2, 1, 1, 1, 1, 1, 1]);
        assert_eq!(cell(&mono, -100), [100, 75, 25, 0, 13, 63, 100, 63, 63, 63, 63, 63, 63]);
    }

    #[test]
    fn width_style_and_flags() {
        use crate::testfonts as tf;
        let sans = tf::build(&tf::SANS);
        let font = Font::parse(&sans).unwrap();
        let a = |o: &Outline| o.ex.px(i64::from(font.advance(font.glyph_index('A').unwrap())), i64::from(o.upem));
        let mk = |w: i32, weight: i32, italic: bool| {
            let mut l = lf("PolyTest Sans", -16, weight);
            l.width = w;
            l.italic = italic;
            Outline::new(&font, Src::Private(1), &l, plain(), None)
        };
        // lfWidth: the average width is that many pixels.
        for (w, avew, adv_a) in [(0, 8, 12), (1, 1, 2), (4, 4, 6), (6, 6, 9), (10, 10, 15), (24, 24, 36)] {
            let o = mk(w, 400, false);
            assert_eq!((o.tm.ave_char_width, a(&o)), (avew, adv_a), "lfWidth {w}");
        }
        // Bold without a bold face: every advance one pixel wider.
        let b = mk(0, 700, false);
        assert!(b.synth_bold && a(&b) == 12 && Typeface::advance_of(&b, &font, font.glyph_index('A').unwrap()) == 13);
        assert_eq!((b.tm.weight, b.tm.ave_char_width, b.tm.max_char_width), (700, 9, 17));
        assert_eq!(mk(0, 599, false).tm.weight, 400);
        let i = mk(0, 400, true);
        assert!(i.synth_italic);
        assert_eq!(i.tm.italic, 255);
        assert_eq!(i.tm.pitch_and_family, 0x07);
        assert_eq!((i.tm.first_char, i.tm.last_char, i.tm.default_char, i.tm.break_char), (32, 8364, 31, 32));
        // The font's own underline and strike-out: 2 below the baseline, 4
        // above it, one pixel thick.
        assert_eq!((i.underline, i.strike), ((2, 1), (4, 1)));
        let tall = tf::build(&tf::TALL);
        let o =
            Outline::new(&Font::parse(&tall).unwrap(), Src::Private(2), &lf("PolyTest Tall", -16, 400), plain(), None);
        assert_eq!((o.underline, o.strike), ((3, 2), (5, 2)));
        let mono = tf::build(&tf::MONO);
        let o =
            Outline::new(&Font::parse(&mono).unwrap(), Src::Private(3), &lf("PolyTest Mono", -16, 400), plain(), None);
        assert_eq!(o.tm.pitch_and_family, 0x36);
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

    #[test]
    fn dejavu_metrics_are_truetype_like() {
        let Some(mut files) = store() else {
            std::eprintln!("skipped: no DejaVu fonts (POLY_FONT_DIR)");
            return;
        };
        let slot = files.find(Class::Sans, false, false).unwrap();
        let data = files.data(slot).unwrap();
        let font = Font::parse(&data).unwrap();
        let o = Outline::new(&font, Src::Installed(slot), &lf("Segoe UI", -12, 400), plain(), Some(Class::Sans));
        let t = o.tm;
        assert_eq!(t.height, t.ascent + t.descent);
        assert_eq!(t.height - t.internal_leading, 12);
        assert_eq!(t.pitch_and_family & 7, 1 | 2 | 4);
    }
}
