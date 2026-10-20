//! `cargo xtask font [--check]`: turn the BDF fonts in `third_party/` into
//! the glyph tables `poly-gfx` compiles in.
//!
//! BDF (Glyph Bitmap Distribution Format 2.1, Adobe) is a text format: a
//! header with the font's bounding box and properties, then one block per
//! glyph with its code point (`ENCODING`), its bounding box (`BBX`) and one
//! line of hex per bitmap row, the leftmost pixel in the most significant
//! bit. The generated table keeps only what a fixed-cell terminal font needs:
//! every glyph placed in the full cell, rows padded to whole bytes, and the
//! code points as ranges of consecutive glyphs.
//!
//! The tables are checked in (`crates/poly-gfx/src/font/<name>.rs`), so a
//! build needs neither the BDF nor this tool; `cargo xtask ci` runs this with
//! `--check` to make sure they still match their sources.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// One font to convert: the BDF, the Rust module to write, and the name of
/// the static in it.
struct Job {
    bdf: &'static str,
    module: &'static str,
    name: &'static str,
    title: &'static str,
}

const JOBS: [Job; 2] = [
    Job {
        bdf: "third_party/spleen/spleen-8x16.bdf",
        module: "crates/poly-gfx/src/font/spleen_8x16.rs",
        name: "SPLEEN_8X16",
        title: "Spleen 8x16",
    },
    Job {
        bdf: "third_party/spleen/spleen-12x24.bdf",
        module: "crates/poly-gfx/src/font/spleen_12x24.rs",
        name: "SPLEEN_12X24",
        title: "Spleen 12x24",
    },
];

/// Code points left out: C0 and C1 controls and DEL (a terminal never draws
/// them), and the private use area (vendor symbols with no agreed meaning).
fn wanted(cp: u32) -> bool {
    !(cp < 0x20 || (0x7f..=0x9f).contains(&cp) || (0xe000..=0xf8ff).contains(&cp))
}

/// `font [--check]`
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [a] if a == "--check" => true,
        _ => return Err("usage: cargo xtask font [--check]".into()),
    };
    let mut stale = Vec::new();
    for job in &JOBS {
        let bdf_path = root.join(job.bdf);
        let text = std::fs::read_to_string(&bdf_path).map_err(|e| format!("{}: {e}", bdf_path.display()))?;
        let font = parse(&text).map_err(|e| format!("{}: {e}", bdf_path.display()))?;
        let source = generate(&font, job);
        let out: PathBuf = root.join(job.module);
        let current = std::fs::read_to_string(&out).unwrap_or_default();
        if current == source {
            continue;
        }
        if check {
            stale.push(job.module);
        } else {
            std::fs::write(&out, source).map_err(|e| format!("{}: {e}", out.display()))?;
            eprintln!("==> wrote {} ({} glyphs)", job.module, font.glyphs.len());
        }
    }
    if stale.is_empty() { Ok(()) } else { Err(format!("out of date, run `cargo xtask font`: {}", stale.join(", "))) }
}

/// A parsed BDF font, every glyph already placed in the full cell.
#[derive(Debug)]
pub struct Font {
    pub width: u32,
    pub height: u32,
    /// Rows from the top of the cell to the baseline.
    pub ascent: u32,
    pub version: String,
    pub copyright: String,
    /// Sorted by code point, without duplicates.
    pub glyphs: Vec<Glyph>,
}

#[derive(Debug)]
pub struct Glyph {
    pub codepoint: u32,
    /// `height` rows of `width` pixels, leftmost in the most significant
    /// bit of the row's first byte.
    pub rows: Vec<Vec<u8>>,
}

impl Font {
    fn row_bytes(&self) -> usize {
        self.width.div_ceil(8) as usize
    }

    fn glyph(&self, cp: u32) -> Option<&Glyph> {
        self.glyphs.binary_search_by_key(&cp, |g| g.codepoint).ok().map(|i| &self.glyphs[i])
    }
}

fn numbers<const N: usize>(rest: &str, what: &str) -> Result<[i32; N], String> {
    let v: Vec<i32> =
        rest.split_whitespace().map(str::parse).collect::<Result<_, _>>().map_err(|_| what.to_string())?;
    v.try_into().map_err(|_| format!("{what}: expected {N} numbers"))
}

fn quoted(rest: &str) -> String {
    rest.trim().trim_matches('"').to_string()
}

/// Parse a BDF font with a fixed cell: every glyph must fit inside the font
/// bounding box (`FONTBOUNDINGBOX`).
pub fn parse(text: &str) -> Result<Font, String> {
    let mut lines = text.lines().enumerate().map(|(n, l)| (n + 1, l.trim_end()));
    let (mut bbox, mut ascent, mut version, mut copyright) = (None, None, String::new(), String::new());
    let mut glyphs = Vec::new();
    while let Some((n, line)) = lines.next() {
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "FONTBOUNDINGBOX" => bbox = Some(numbers::<4>(rest, "FONTBOUNDINGBOX")?),
            "FONT_ASCENT" => ascent = Some(rest.trim().parse::<u32>().map_err(|_| format!("line {n}: FONT_ASCENT"))?),
            "FONT_VERSION" => version = quoted(rest),
            "COPYRIGHT" => copyright = quoted(rest),
            "STARTCHAR" => {
                let [w, h, xoff, yoff] = bbox.ok_or(format!("line {n}: glyph before FONTBOUNDINGBOX"))?;
                let ascent = ascent.unwrap_or((h + yoff) as u32);
                let glyph =
                    parse_glyph(&mut lines, [w, h, xoff, yoff], ascent).map_err(|e| format!("line {n}: {e}"))?;
                if let Some(g) = glyph {
                    glyphs.push(g);
                }
            }
            _ => {}
        }
    }
    let [w, h, _, yoff] = bbox.ok_or("no FONTBOUNDINGBOX")?;
    if w <= 0 || h <= 0 || w > 32 || h > 64 {
        return Err(format!("unsupported cell size {w}x{h}"));
    }
    glyphs.sort_by_key(|g: &Glyph| g.codepoint);
    if glyphs.windows(2).any(|p| p[0].codepoint == p[1].codepoint) {
        return Err("a code point has two glyphs".into());
    }
    let ascent = ascent.unwrap_or((h + yoff) as u32);
    Ok(Font { width: w as u32, height: h as u32, ascent, version, copyright, glyphs })
}

/// The rest of one `STARTCHAR` block. `None` for glyphs without a Unicode
/// code point (`ENCODING -1`) or that [`wanted`] leaves out.
fn parse_glyph<'a>(
    lines: &mut impl Iterator<Item = (usize, &'a str)>,
    [cell_w, cell_h, cell_x, _]: [i32; 4],
    ascent: u32,
) -> Result<Option<Glyph>, String> {
    let (mut encoding, mut bbx, mut bitmap) = (None, None, Vec::new());
    loop {
        let (_, line) = lines.next().ok_or("STARTCHAR without ENDCHAR")?;
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "ENCODING" => encoding = Some(numbers::<1>(rest.split_whitespace().next().unwrap_or(""), "ENCODING")?[0]),
            "BBX" => bbx = Some(numbers::<4>(rest, "BBX")?),
            "BITMAP" => loop {
                let (_, row) = lines.next().ok_or("BITMAP without ENDCHAR")?;
                if row == "ENDCHAR" {
                    break;
                }
                bitmap.push(row.to_string());
            },
            _ => {}
        }
        if key == "ENDCHAR" || key == "BITMAP" {
            break;
        }
    }
    let Some(cp) = encoding.and_then(|e| u32::try_from(e).ok()) else { return Ok(None) };
    if !wanted(cp) {
        return Ok(None);
    }
    let [w, h, xoff, yoff] = bbx.ok_or("glyph without BBX")?;
    if bitmap.len() != h as usize {
        return Err(format!("U+{cp:04X}: {} bitmap rows, BBX says {h}", bitmap.len()));
    }
    // Where the glyph's box sits in the cell: x from the left edge, y (of
    // its top row) from the top, both relative to the font bounding box.
    let left = xoff - cell_x;
    let top = ascent as i32 - (h + yoff);
    if left < 0 || top < 0 || left + w > cell_w || top + h > cell_h {
        return Err(format!("U+{cp:04X}: glyph box {w}x{h}{xoff:+}{yoff:+} does not fit the cell"));
    }
    let row_bytes = (cell_w as usize).div_ceil(8);
    let mut rows = vec![vec![0u8; row_bytes]; cell_h as usize];
    for (y, hex) in bitmap.iter().enumerate() {
        let bits = u64::from_str_radix(hex, 16).map_err(|_| format!("U+{cp:04X}: bad bitmap row {hex:?}"))?;
        // BDF pads each row to whole bytes on the right.
        let stored = hex.len() as i32 * 4;
        if stored < w || stored > 64 {
            return Err(format!("U+{cp:04X}: bitmap row {hex:?} is not {w} pixels wide"));
        }
        for x in 0..w {
            if bits >> (stored - 1 - x) & 1 != 0 {
                let (cx, cy) = ((left + x) as usize, top as usize + y);
                rows[cy][cx / 8] |= 0x80 >> (cx % 8);
            }
        }
    }
    Ok(Some(Glyph { codepoint: cp, rows }))
}

/// The glyph for characters the font lacks: the outline of a box as tall as
/// a capital letter ("H") and one pixel in from either side of the cell.
pub fn fallback(font: &Font) -> Vec<Vec<u8>> {
    let lit = |rows: &[Vec<u8>], y: usize| rows[y].iter().any(|&b| b != 0);
    let (top, bottom) = match font.glyph('H' as u32) {
        Some(h) => {
            let first = (0..font.height as usize).find(|&y| lit(&h.rows, y)).unwrap_or(0);
            let last = (0..font.height as usize).rev().find(|&y| lit(&h.rows, y)).unwrap_or(font.height as usize - 1);
            (first, last)
        }
        None => (1, font.height as usize - 2),
    };
    let (left, right) = (1usize, font.width as usize - 2);
    let mut rows = vec![vec![0u8; font.row_bytes()]; font.height as usize];
    for (y, row) in rows.iter_mut().enumerate().take(bottom + 1).skip(top) {
        let edge = y == top || y == bottom;
        for x in left..=right {
            if edge || x == left || x == right {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    rows
}

/// Runs of consecutive code points: (first, last, index of the first glyph).
pub fn ranges(font: &Font) -> Vec<(u32, u32, usize)> {
    let mut out: Vec<(u32, u32, usize)> = Vec::new();
    for (i, g) in font.glyphs.iter().enumerate() {
        match out.last_mut() {
            Some(r) if r.1 + 1 == g.codepoint => r.1 = g.codepoint,
            _ => out.push((g.codepoint, g.codepoint, i)),
        }
    }
    out
}

fn hex(rows: &[Vec<u8>]) -> String {
    rows.iter().flatten().map(|b| format!("{b:02x}")).collect()
}

/// A comment naming the glyph: its code point and, when it prints on its
/// own, the character.
fn describe(cp: u32) -> String {
    let combining = (0x300..=0x36f).contains(&cp);
    match char::from_u32(cp) {
        Some(c) if !combining && !c.is_whitespace() && cp != 0xad => format!("U+{cp:04X} {c}"),
        _ => format!("U+{cp:04X}"),
    }
}

/// The Rust module for `font`.
fn generate(font: &Font, job: &Job) -> String {
    let ranges = ranges(font);
    let row_bytes = font.row_bytes();
    let glyph_bytes = row_bytes * font.height as usize;
    let mut s = String::new();
    let w = &mut s;
    let _ = writeln!(w, "//! {} {}: {}.", job.title, font.version, font.copyright);
    let _ = writeln!(w, "//! BSD 2-Clause licence, see `third_party/spleen/LICENSE` and `THIRD_PARTY.md`.");
    let _ = writeln!(w, "//!");
    let _ = writeln!(w, "//! Generated from `{}` by `cargo xtask font`; do not edit.", job.bdf);
    let _ = writeln!(w, "//! {} glyphs in {} ranges, {} bytes each.", font.glyphs.len(), ranges.len(), glyph_bytes);
    let _ = writeln!(w);
    let _ = writeln!(w, "use super::{{Font, GlyphRange, hex}};");
    let _ = writeln!(w);
    let _ = writeln!(w, "pub static {}: Font = Font {{", job.name);
    let _ = writeln!(w, "    name: \"{}\",", job.title);
    let _ = writeln!(w, "    width: {},", font.width);
    let _ = writeln!(w, "    height: {},", font.height);
    let _ = writeln!(w, "    ascent: {},", font.ascent);
    let _ = writeln!(w, "    ranges: &RANGES,");
    let _ = writeln!(w, "    bitmaps: &BITMAPS,");
    let _ = writeln!(w, "    fallback: &FALLBACK,");
    let _ = writeln!(w, "}};");
    let _ = writeln!(w);
    let _ = writeln!(w, "#[rustfmt::skip]");
    let _ = writeln!(w, "static RANGES: [GlyphRange; {}] = [", ranges.len());
    for (first, last, index) in &ranges {
        let _ = writeln!(w, "    GlyphRange {{ first: {first:#06x}, last: {last:#06x}, index: {index} }},");
    }
    let _ = writeln!(w, "];");
    let _ = writeln!(w);
    let _ = writeln!(w, "#[rustfmt::skip]");
    let _ = writeln!(w, "static FALLBACK: [u8; {glyph_bytes}] = hex(\"{}\");", hex(&fallback(font)));
    let _ = writeln!(w);
    let _ = writeln!(w, "#[rustfmt::skip]");
    let _ = writeln!(w, "static BITMAPS: [u8; {}] = hex(concat!(", glyph_bytes * font.glyphs.len());
    for g in &font.glyphs {
        let _ = writeln!(w, "    \"{}\", // {}", hex(&g.rows), describe(g.codepoint));
    }
    let _ = writeln!(w, "));");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x6 font with its baseline 4 rows down: an "A" filling its own
    /// smaller box, a "." in a 1x1 box on the baseline, a control character
    /// and an unencoded glyph that are both dropped.
    const TINY: &str = "\
STARTFONT 2.1
FONTBOUNDINGBOX 4 6 0 -2
STARTPROPERTIES 3
FONT_ASCENT 4
FONT_VERSION \"1.0\"
COPYRIGHT \"nobody\"
ENDPROPERTIES
CHARS 4
STARTCHAR A
ENCODING 65
BBX 3 4 0 0
BITMAP
40
A0
E0
A0
ENDCHAR
STARTCHAR period
ENCODING 46
BBX 1 1 1 0
BITMAP
80
ENDCHAR
STARTCHAR control
ENCODING 7
BBX 4 6 0 -2
BITMAP
F0
F0
F0
F0
F0
F0
ENDCHAR
STARTCHAR unencoded
ENCODING -1
BBX 4 6 0 -2
BITMAP
F0
F0
F0
F0
F0
F0
ENDCHAR
ENDFONT
";

    #[test]
    fn parses_and_places_glyphs_in_the_cell() {
        let f = parse(TINY).unwrap();
        assert_eq!((f.width, f.height, f.ascent), (4, 6, 4));
        assert_eq!((f.version.as_str(), f.copyright.as_str()), ("1.0", "nobody"));
        let cps: Vec<u32> = f.glyphs.iter().map(|g| g.codepoint).collect();
        assert_eq!(cps, [0x2e, 0x41]);
        // "A": rows 0-3 (ascent 4 = its height), then the two descender rows.
        let a: Vec<u8> = f.glyph(0x41).unwrap().rows.iter().map(|r| r[0]).collect();
        assert_eq!(a, [0x40, 0xa0, 0xe0, 0xa0, 0, 0]);
        // ".": one pixel at x 1, on the row just above the baseline.
        let dot: Vec<u8> = f.glyph(0x2e).unwrap().rows.iter().map(|r| r[0]).collect();
        assert_eq!(dot, [0, 0, 0, 0x40, 0, 0]);
    }

    #[test]
    fn fallback_is_a_box_as_tall_as_h() {
        let mut f = parse(TINY).unwrap();
        // Replace "A" by an "H" spanning rows 1-3.
        f.glyphs[1] =
            Glyph { codepoint: 'H' as u32, rows: vec![vec![0], vec![0xa0], vec![0xe0], vec![0xa0], vec![0], vec![0]] };
        let rows: Vec<u8> = fallback(&f).iter().map(|r| r[0]).collect();
        assert_eq!(rows, [0, 0x60, 0x60, 0x60, 0, 0]);
    }

    #[test]
    fn ranges_group_consecutive_code_points() {
        let mut f = parse(TINY).unwrap();
        let blank = || vec![vec![0u8]; 6];
        f.glyphs = [0x20, 0x21, 0x22, 0x41, 0x43, 0x44]
            .into_iter()
            .map(|codepoint| Glyph { codepoint, rows: blank() })
            .collect();
        assert_eq!(ranges(&f), [(0x20, 0x22, 0), (0x41, 0x41, 3), (0x43, 0x44, 4)]);
    }

    #[test]
    fn rejects_glyphs_outside_the_cell() {
        let bad = TINY.replace("BBX 1 1 1 0", "BBX 1 1 4 0");
        assert!(parse(&bad).unwrap_err().contains("U+002E"));
        let bad = TINY.replace("BBX 3 4 0 0\nBITMAP\n40\n", "BBX 3 4 0 0\nBITMAP\n");
        assert!(parse(&bad).unwrap_err().contains("bitmap rows"));
    }

    /// The checked-in tables match the BDFs they were generated from.
    #[test]
    fn checked_in_tables_are_up_to_date() {
        run(&crate::root(), &["--check".to_string()]).unwrap();
    }
}
