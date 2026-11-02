//! The stock fonts (ADR 0128, Decision 5; `TextOutW.yaml`): their
//! `TEXTMETRICW` values and advance widths as the oracle reports them, and
//! the glyphs Poly draws them with (the system font's bitmap glyphs,
//! `crate::sysfont`). TrueType faces through `poly-font` come with the GDI
//! text slice.

use crate::sysfont::GLYPHS;

/// Which stock font a font object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// `SYSTEM_FONT`: bold, 16 pixels.
    System,
    /// `DEFAULT_GUI_FONT`.
    Gui,
    /// `ANSI_FIXED_FONT` (also `SYSTEM_FIXED_FONT`).
    AnsiFixed,
    /// `OEM_FIXED_FONT`.
    OemFixed,
}

/// `TEXTMETRICW` (`wingdi.h`), in field order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextMetric {
    pub height: i32,
    pub ascent: i32,
    pub descent: i32,
    pub internal_leading: i32,
    pub external_leading: i32,
    pub ave_char_width: i32,
    pub max_char_width: i32,
    pub weight: i32,
    pub overhang: i32,
    pub aspect_x: i32,
    pub aspect_y: i32,
    pub first_char: u16,
    pub last_char: u16,
    pub default_char: u16,
    pub break_char: u16,
    pub italic: u8,
    pub underlined: u8,
    pub struck_out: u8,
    pub pitch_and_family: u8,
    pub char_set: u8,
}

impl TextMetric {
    /// The 60-byte `TEXTMETRICW`.
    pub fn to_bytes(&self) -> [u8; 60] {
        let mut b = [0u8; 60];
        let ints = [
            self.height,
            self.ascent,
            self.descent,
            self.internal_leading,
            self.external_leading,
            self.ave_char_width,
            self.max_char_width,
            self.weight,
            self.overhang,
            self.aspect_x,
            self.aspect_y,
        ];
        for (i, v) in ints.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, v) in [self.first_char, self.last_char, self.default_char, self.break_char].iter().enumerate() {
            b[44 + i * 2..46 + i * 2].copy_from_slice(&v.to_le_bytes());
        }
        b[52] = self.italic;
        b[53] = self.underlined;
        b[54] = self.struck_out;
        b[55] = self.pitch_and_family;
        b[56] = self.char_set;
        b
    }
}

impl Face {
    pub fn metrics(self) -> TextMetric {
        match self {
            Face::System => TextMetric {
                height: 16,
                ascent: 13,
                descent: 3,
                internal_leading: 3,
                external_leading: 0,
                ave_char_width: 7,
                max_char_width: 14,
                weight: 700,
                overhang: 0,
                aspect_x: 96,
                aspect_y: 96,
                first_char: 32,
                last_char: 8482,
                default_char: 8364,
                break_char: 32,
                pitch_and_family: 0x21,
                char_set: 0,
                ..TextMetric::default()
            },
            Face::Gui => TextMetric {
                height: 13,
                ascent: 11,
                descent: 2,
                ave_char_width: 5,
                max_char_width: 23,
                weight: 400,
                aspect_x: 96,
                aspect_y: 96,
                first_char: 32,
                last_char: 65533,
                default_char: 31,
                break_char: 32,
                pitch_and_family: 0x27,
                ..TextMetric::default()
            },
            Face::AnsiFixed => TextMetric {
                height: 13,
                ascent: 11,
                descent: 2,
                ave_char_width: 8,
                max_char_width: 8,
                weight: 400,
                aspect_x: 96,
                aspect_y: 96,
                first_char: 32,
                last_char: 255,
                default_char: 128,
                break_char: 32,
                pitch_and_family: 0x30,
                ..TextMetric::default()
            },
            Face::OemFixed => TextMetric {
                height: 12,
                ascent: 10,
                descent: 2,
                ave_char_width: 8,
                max_char_width: 8,
                weight: 400,
                aspect_x: 72,
                aspect_y: 96,
                first_char: 1,
                last_char: 254,
                default_char: 32,
                break_char: 32,
                pitch_and_family: 0x30,
                char_set: 255,
                ..TextMetric::default()
            },
        }
    }

    /// The advance width of `c`.
    pub fn width(self, c: u16) -> i32 {
        match self {
            Face::System => system_width(c),
            Face::AnsiFixed | Face::OemFixed => 8,
            Face::Gui => match c {
                0x20 => 3,
                0x2C => 3,
                0x41 => 7,
                0x48 => 7,
                0x57 => 11,
                0x64 => 6,
                0x65 => 6,
                0x69 => 2,
                0x6C => 2,
                0x6F => 6,
                0x72 => 4,
                0x77 => 8,
                _ if c < 0x20 => 1,
                _ => (system_width(c) * 3 + 2) / 4,
            },
        }
    }

    /// The glyph rows of `c` (top to bottom) and its width; characters the
    /// font does not have are a box.
    pub fn glyph(self, c: u16) -> (i32, [u16; 16]) {
        let w = self.width(c);
        if (0x20..=0x7E).contains(&c) {
            return (w, GLYPHS[usize::from(c - 0x20)].1);
        }
        if c < 0x20 {
            return (w, [0; 16]);
        }
        // A box for everything else.
        let mut rows = [0u16; 16];
        let wbits = (w - 2).clamp(1, 14) as u32;
        let full: u16 = !(0xFFFFu16 >> wbits) >> 1;
        let edges: u16 = (1 << 14) | (1 << (15 - wbits));
        for (r, row) in rows.iter_mut().enumerate().take(13).skip(3) {
            *row = if r == 3 || r == 12 { full } else { edges };
        }
        (w, rows)
    }
}

fn system_width(c: u16) -> i32 {
    if c < 0x20 {
        // Control characters measure one pixel (`DrawTextW.yaml`: a tab
        // without DT_EXPANDTABS, a newline on one line).
        1
    } else if c <= 0x7E {
        i32::from(GLYPHS[usize::from(c - 0x20)].0)
    } else {
        8
    }
}

/// The extent of `text` in `face`: (width, height); an empty string is 0x0.
pub fn extent(face: Face, text: &[u16]) -> (i32, i32) {
    if text.is_empty() {
        return (0, 0);
    }
    (text.iter().map(|&c| face.width(c)).sum(), face.metrics().height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> i32 {
        let v: alloc::vec::Vec<u16> = s.encode_utf16().collect();
        extent(Face::System, &v).0
    }

    #[test]
    fn system_font_extents_match_the_sheets() {
        assert_eq!(w("A"), 8);
        assert_eq!(w("i"), 4);
        assert_eq!(w("W"), 14);
        assert_eq!(w("Hello, world"), 77);
        assert_eq!(w(" "), 4);
        assert_eq!(extent(Face::System, &[]), (0, 0));
        assert_eq!(extent(Face::System, &[0x41]).1, 16);
        let gw = |s: &str| {
            let v: alloc::vec::Vec<u16> = s.encode_utf16().collect();
            extent(Face::Gui, &v).0
        };
        assert_eq!((gw("A"), gw("i"), gw("W"), gw("Hello, world"), gw(" ")), (7, 2, 11, 55, 3));
    }
}
