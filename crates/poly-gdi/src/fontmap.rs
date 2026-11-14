//! The GDI font mapper (ADR 0142): which of Poly's fonts a `LOGFONTW` gets.
//!
//! Poly cannot ship Microsoft's fonts, so a face name a program asks for
//! ("Segoe UI", "Courier New", "Consolas") is mapped onto a class (sans
//! serif, serif, fixed pitch) and from there onto the freely licensed font
//! Poly has of that class: the DejaVu family of the font store (ADR 0105).
//! The same table is what `crates/poly-font/fonts.conf` does for Linux
//! programs, so a name means the same thing on both sides.
//!
//! The class picks the outline font ([`crate::outline`] rasterises it with
//! `poly-font`); where that font's files are not there, or for the raster
//! font names, one of the four bitmap [`Face`]s draws instead, see
//! [`bitmap_face`]. Names and constants are from the public documentation:
//! `LOGFONTW` and the pitch and family constants of `wingdi.h`, and the font
//! names Microsoft documents for its fonts.

use crate::font::Face;

/// Which kind of typeface a name stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Sans serif (Arial, Segoe UI, Tahoma...).
    Sans,
    /// Serif (Times New Roman, Georgia...).
    Serif,
    /// Fixed pitch (Courier New, Consolas...).
    Mono,
}

impl Class {
    /// The family of Poly's font store that stands in for the class.
    pub const fn family(self) -> &'static str {
        match self {
            Class::Sans => "DejaVu Sans",
            Class::Serif => "DejaVu Serif",
            Class::Mono => "DejaVu Sans Mono",
        }
    }
}

// `lfPitchAndFamily` (`wingdi.h`).
const FIXED_PITCH: u8 = 1;
const FF_ROMAN: u8 = 0x10;
const FF_MODERN: u8 = 0x30;

/// `LOGFONTW`'s size in bytes (`wingdi.h`: five `LONG`s, eight `BYTE`s, 32
/// `WCHAR`s).
pub const LOGFONT_SIZE: usize = 92;

/// The fields of a `LOGFONTW` the mapper looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogFont {
    pub height: i32,
    /// `lfWidth`: the average character width in logical units; 0 keeps the
    /// font's natural proportions.
    pub width: i32,
    pub escapement: i32,
    pub orientation: i32,
    pub weight: i32,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    pub char_set: u8,
    /// `lfQuality`: 3 is `NONANTIALIASED_QUALITY`.
    pub quality: u8,
    pub pitch_and_family: u8,
    face: [u16; 32],
}

impl LogFont {
    /// Read a 92-byte `LOGFONTW`; `None` if `b` is too short.
    pub fn parse(b: &[u8]) -> Option<LogFont> {
        let b = b.get(..LOGFONT_SIZE)?;
        let int = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let mut face = [0u16; 32];
        for (i, f) in face.iter_mut().enumerate() {
            *f = u16::from_le_bytes([b[28 + 2 * i], b[29 + 2 * i]]);
        }
        // The name is NUL-terminated; a full 32 characters have no NUL.
        if let Some(n) = face.iter().position(|&c| c == 0) {
            for f in &mut face[n..] {
                *f = 0;
            }
        }
        Some(LogFont {
            height: int(0),
            width: int(4),
            escapement: int(8),
            orientation: int(12),
            weight: int(16),
            italic: b[20] != 0,
            underline: b[21] != 0,
            strike_out: b[22] != 0,
            char_set: b[23],
            quality: b[26],
            pitch_and_family: b[27],
            face,
        })
    }

    /// The face name (without the NUL).
    pub fn face_name(&self) -> &[u16] {
        let n = self.face.iter().position(|&c| c == 0).unwrap_or(self.face.len());
        &self.face[..n]
    }
}

/// What the mapper decided for a `LOGFONTW`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub class: Class,
    /// The Poly font family for the outline path ([`Class::family`]).
    pub family: &'static str,
    /// Bold (`lfWeight` of 600 or more).
    pub bold: bool,
    pub italic: bool,
    /// The name is one of Windows' raster fonts, which only exist as
    /// bitmaps: that [`Face`] is the answer whatever the size.
    pub raster: Option<Face>,
}

/// Windows' raster font names (`System`, `Terminal`, `Fixedsys`, `Courier`,
/// `MS Sans Serif`, `MS Serif`, `Small Fonts`): the stock bitmap face that
/// stands for each.
const RASTER: &[(&str, Face, Class)] = &[
    ("system", Face::System, Class::Sans),
    ("terminal", Face::OemFixed, Class::Mono),
    ("fixedsys", Face::AnsiFixed, Class::Mono),
    ("courier", Face::AnsiFixed, Class::Mono),
    ("ms sans serif", Face::Gui, Class::Sans),
    ("ms serif", Face::Gui, Class::Serif),
    ("small fonts", Face::Gui, Class::Sans),
];

/// TrueType and other outline names programs ask for, and the class of
/// each. A name matches whole, ignoring ASCII case: "Segoe UI PolyNoSuch" is
/// not "Segoe UI" just because it starts with it (`CreateFontIndirectW.yaml`:
/// a prefix or an extension of a name does not match). The families Windows
/// has under several names are listed one by one.
const NAMES: &[(&str, Class)] = &[
    // Sans serif.
    ("arial", Class::Sans),
    ("arial black", Class::Sans),
    ("arial narrow", Class::Sans),
    ("arial rounded mt bold", Class::Sans),
    ("arial unicode ms", Class::Sans),
    ("segoe ui", Class::Sans),
    ("segoe ui black", Class::Sans),
    ("segoe ui emoji", Class::Sans),
    ("segoe ui historic", Class::Sans),
    ("segoe ui light", Class::Sans),
    ("segoe ui semibold", Class::Sans),
    ("segoe ui semilight", Class::Sans),
    ("segoe ui symbol", Class::Sans),
    ("segoe ui variable display", Class::Sans),
    ("segoe ui variable small", Class::Sans),
    ("segoe ui variable text", Class::Sans),
    ("tahoma", Class::Sans),
    ("verdana", Class::Sans),
    ("calibri", Class::Sans),
    ("calibri light", Class::Sans),
    ("candara", Class::Sans),
    ("corbel", Class::Sans),
    ("trebuchet ms", Class::Sans),
    ("microsoft sans serif", Class::Sans),
    ("ms shell dlg", Class::Sans),
    ("ms shell dlg 2", Class::Sans),
    ("ms ui gothic", Class::Sans),
    ("lucida sans", Class::Sans),
    ("lucida sans unicode", Class::Sans),
    ("century gothic", Class::Sans),
    ("franklin gothic medium", Class::Sans),
    ("helvetica", Class::Sans),
    ("open sans", Class::Sans),
    ("roboto", Class::Sans),
    ("dejavu sans", Class::Sans),
    ("liberation sans", Class::Sans),
    ("liberation sans narrow", Class::Sans),
    ("bitstream vera sans", Class::Sans),
    ("sans-serif", Class::Sans),
    ("meiryo", Class::Sans),
    ("meiryo ui", Class::Sans),
    ("yu gothic", Class::Sans),
    ("yu gothic ui", Class::Sans),
    ("yu gothic light", Class::Sans),
    ("yu gothic medium", Class::Sans),
    ("yu gothic ui light", Class::Sans),
    ("yu gothic ui semibold", Class::Sans),
    ("yu gothic ui semilight", Class::Sans),
    // Serif.
    ("times new roman", Class::Serif),
    ("times", Class::Serif),
    ("georgia", Class::Serif),
    ("cambria", Class::Serif),
    ("constantia", Class::Serif),
    ("palatino linotype", Class::Serif),
    ("book antiqua", Class::Serif),
    ("bookman old style", Class::Serif),
    ("garamond", Class::Serif),
    ("century schoolbook", Class::Serif),
    ("dejavu serif", Class::Serif),
    ("liberation serif", Class::Serif),
    ("bitstream vera serif", Class::Serif),
    ("serif", Class::Serif),
    ("ms mincho", Class::Serif),
    // Fixed pitch.
    ("courier new", Class::Mono),
    ("consolas", Class::Mono),
    ("lucida console", Class::Mono),
    ("cascadia code", Class::Mono),
    ("cascadia mono", Class::Mono),
    ("andale mono", Class::Mono),
    ("monaco", Class::Mono),
    ("menlo", Class::Mono),
    ("source code pro", Class::Mono),
    ("dejavu sans mono", Class::Mono),
    ("liberation mono", Class::Mono),
    ("bitstream vera sans mono", Class::Mono),
    ("monospace", Class::Mono),
    ("mono", Class::Mono),
    ("ms gothic", Class::Mono),
];

/// `name` lower-cased (ASCII), without a leading `@` (a vertical-writing
/// variant of the same face), as bytes; non-ASCII names do not match. A
/// blank before or after the name, or a doubled one, makes another name
/// (`CreateFontIndirectW.yaml`, rule `names`).
fn normalise(name: &[u16], out: &mut [u8; 32]) -> usize {
    let name = name.strip_prefix(&[u16::from(b'@')]).unwrap_or(name);
    let mut n = 0;
    for &c in name.iter().take(32) {
        let Ok(b) = u8::try_from(c) else { return 0 };
        out[n] = b.to_ascii_lowercase();
        n += 1;
    }
    n
}

fn matches(pattern: &str, name: &[u8]) -> bool {
    name == pattern.as_bytes()
}

/// Map a `LOGFONTW`: by the face name when it is one the table knows, else
/// by the pitch and family the program gave (fixed pitch or `FF_MODERN`:
/// fixed; `FF_ROMAN`: serif; everything else, including no hint at all:
/// sans serif, the system's default look).
pub fn map(lf: &LogFont) -> Mapping {
    let mut buf = [0u8; 32];
    let n = normalise(lf.face_name(), &mut buf);
    let name = &buf[..n];
    let mut raster = None;
    let mut class = None;
    if n > 0 {
        if let Some(&(_, face, c)) = RASTER.iter().find(|(r, _, _)| r.as_bytes() == name) {
            raster = Some(face);
            class = Some(c);
        } else if let Some(&(_, c)) = NAMES.iter().find(|(p, _)| matches(p, name)) {
            class = Some(c);
        }
    }
    let class = class.unwrap_or_else(|| {
        let pf = lf.pitch_and_family;
        if pf & 3 == FIXED_PITCH || pf & 0xF0 == FF_MODERN {
            Class::Mono
        } else if pf & 0xF0 == FF_ROMAN {
            Class::Serif
        } else {
            Class::Sans
        }
    });
    Mapping { class, family: class.family(), bold: lf.weight >= 600, italic: lf.italic, raster }
}

/// Whether the mapper knows the face name of `lf` (a raster name or one of
/// the outline names of its table), rather than going by the pitch and
/// family alone.
pub fn is_known(lf: &LogFont) -> bool {
    let mut buf = [0u8; 32];
    let n = normalise(lf.face_name(), &mut buf);
    let name = &buf[..n];
    n > 0 && (RASTER.iter().any(|(r, _, _)| r.as_bytes() == name) || NAMES.iter().any(|(p, _)| matches(p, name)))
}

/// The tallest `lfHeight` (in pixels, either sign) the 13-pixel
/// `DEFAULT_GUI_FONT` face stands in for; taller text gets the 16-pixel
/// system face, the only other proportional face Poly has as a bitmap.
const GUI_MAX_HEIGHT: u32 = 13;

/// The bitmap face that draws a mapped font for now: a raster font's own
/// face, a fixed-pitch class on the fixed face, else the GUI face for small
/// text (a height of 0 asks for the default size) and the system face for
/// larger.
pub fn bitmap_face(lf: &LogFont) -> Face {
    let m = map(lf);
    if let Some(f) = m.raster {
        return f;
    }
    match m.class {
        Class::Mono => Face::AnsiFixed,
        _ if lf.height.unsigned_abs() <= GUI_MAX_HEIGHT => Face::Gui,
        _ => Face::System,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn lf(name: &str, height: i32, weight: i32, pf: u8) -> LogFont {
        let mut b = [0u8; LOGFONT_SIZE];
        b[0..4].copy_from_slice(&height.to_le_bytes());
        b[16..20].copy_from_slice(&weight.to_le_bytes());
        b[27] = pf;
        for (i, c) in name.encode_utf16().take(31).enumerate() {
            b[28 + 2 * i..30 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        LogFont::parse(&b).unwrap()
    }

    #[test]
    fn parses_the_wingdi_layout() {
        let mut b = [0u8; LOGFONT_SIZE];
        b[0..4].copy_from_slice(&(-12i32).to_le_bytes());
        b[16..20].copy_from_slice(&700i32.to_le_bytes());
        b[20] = 1;
        b[23] = 2;
        b[27] = 0x31;
        for (i, c) in "Segoe UI".encode_utf16().enumerate() {
            b[28 + 2 * i..30 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        let l = LogFont::parse(&b).unwrap();
        assert_eq!((l.height, l.weight, l.italic, l.char_set, l.pitch_and_family), (-12, 700, true, 2, 0x31));
        assert_eq!((l.width, l.escapement, l.orientation, l.underline, l.strike_out), (0, 0, 0, false, false));
        assert_eq!(l.face_name(), "Segoe UI".encode_utf16().collect::<Vec<_>>().as_slice());
        assert!(LogFont::parse(&b[..91]).is_none());
    }

    #[test]
    fn a_name_without_a_nul_takes_all_32_characters() {
        let mut b = [0u8; LOGFONT_SIZE];
        for i in 0..32 {
            b[28 + 2 * i] = b'a';
        }
        assert_eq!(LogFont::parse(&b).unwrap().face_name().len(), 32);
    }

    #[test]
    fn names_map_to_poly_families() {
        let fam = |n: &str| map(&lf(n, -12, 400, 0)).family;
        assert_eq!(fam("Segoe UI"), "DejaVu Sans");
        assert_eq!(fam("segoe ui semibold"), "DejaVu Sans");
        assert_eq!(fam("ARIAL"), "DejaVu Sans");
        assert_eq!(fam("Arial Black"), "DejaVu Sans");
        assert_eq!(fam("Tahoma"), "DejaVu Sans");
        assert_eq!(fam("MS Shell Dlg"), "DejaVu Sans");
        assert_eq!(fam("MS Shell Dlg 2"), "DejaVu Sans");
        assert_eq!(fam("Times New Roman"), "DejaVu Serif");
        assert_eq!(fam("Georgia"), "DejaVu Serif");
        assert_eq!(fam("Courier New"), "DejaVu Sans Mono");
        assert_eq!(fam("Consolas"), "DejaVu Sans Mono");
        assert_eq!(fam("Lucida Console"), "DejaVu Sans Mono");
        assert_eq!(fam("@Arial"), "DejaVu Sans");
    }

    #[test]
    fn a_name_is_matched_whole() {
        let known = |n: &str| is_known(&lf(n, -12, 400, 0));
        assert!(known("Segoe UI") && known("segoe ui") && known("@Segoe UI") && known("Segoe UI Semibold"));
        // A prefix, an extension, a blank before, after or doubled: another name.
        for n in ["Segoe UI PolyNoSuch", "Segoe", "Aria", "Arial ", " Arial", "Segoe  UI", "Courier New Baltic", ""] {
            assert!(!known(n), "{n:?}");
        }
        // Such a name follows the hints like any unknown one.
        let m = |n: &str, pf: u8| map(&lf(n, -12, 400, pf)).class;
        assert_eq!(m("Courier New PolyNoSuch", 0), Class::Sans);
        assert_eq!(m("Courier New PolyNoSuch", FF_MODERN), Class::Mono);
        assert_eq!(m("Arial ", FF_ROMAN), Class::Serif);
    }

    #[test]
    fn unknown_names_follow_the_pitch_and_family() {
        let m = |n: &str, pf: u8| map(&lf(n, -12, 400, pf)).class;
        assert_eq!(m("Zzyzx", 0), Class::Sans);
        assert_eq!(m("Zzyzx", FF_ROMAN), Class::Serif);
        assert_eq!(m("Zzyzx", 0x20 | 2), Class::Sans);
        assert_eq!(m("Zzyzx", FF_MODERN), Class::Mono);
        assert_eq!(m("Zzyzx", FIXED_PITCH), Class::Mono);
        assert_eq!(m("", 0x12), Class::Serif);
        // A known name wins over the hint.
        assert_eq!(m("Arial", FIXED_PITCH), Class::Sans);
        assert_eq!(m("Courier New", 0x20), Class::Mono);
        // A name with non-ASCII characters is not in the table.
        assert_eq!(m("Ärial", FF_ROMAN), Class::Serif);
    }

    #[test]
    fn weight_and_italic_choose_the_style() {
        let m = map(&lf("Arial", -12, 700, 0));
        assert!(m.bold && !m.italic);
        let m = map(&lf("Arial", -12, 599, 0));
        assert!(!m.bold);
        let mut b = [0u8; LOGFONT_SIZE];
        b[20] = 1;
        assert!(map(&LogFont::parse(&b).unwrap()).italic);
    }

    #[test]
    fn bitmap_faces_by_class_and_size() {
        let f = |n: &str, h: i32, pf: u8| bitmap_face(&lf(n, h, 400, pf));
        // Raster fonts are their own bitmaps at any size.
        assert_eq!(f("System", -40, 0), Face::System);
        assert_eq!(f("Terminal", -40, 0), Face::OemFixed);
        assert_eq!(f("Fixedsys", 8, 0), Face::AnsiFixed);
        assert_eq!(f("MS Sans Serif", 40, 0), Face::Gui);
        // Fixed pitch: the fixed face.
        assert_eq!(f("Courier New", -16, 0), Face::AnsiFixed);
        assert_eq!(f("Consolas", -13, 0), Face::AnsiFixed);
        // Proportional: GUI up to 13 pixels (Segoe UI 9 pt is 12), 0 is the
        // default size, system above.
        assert_eq!(f("Segoe UI", -12, 0), Face::Gui);
        assert_eq!(f("Segoe UI", 13, 0), Face::Gui);
        assert_eq!(f("Segoe UI", 0, 0), Face::Gui);
        assert_eq!(f("Segoe UI", -14, 0), Face::System);
        assert_eq!(f("Times New Roman", -32, 0), Face::System);
    }
}
