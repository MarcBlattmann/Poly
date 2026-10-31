//! fontconfig-compatible font discovery, from the fontconfig user
//! documentation (patterns, properties, constants, matching, `fonts.conf`).
//!
//! - [`Pattern`] is a font name like `DejaVu Sans-12:bold` parsed into
//!   properties, and back to text.
//! - [`describe`] turns a parsed [`Font`] into the pattern of properties
//!   fontconfig would give it (family, style, weight, slant, width,
//!   spacing, file, index...).
//! - [`FontSet`] holds the fonts found; [`FontSet::font_match`] is
//!   `fc-match` (the best font for a pattern), [`FontSet::sort`] is
//!   `fc-match --sort`, [`FontSet::list`] is `fc-list` (every font that
//!   has all the properties asked for).
//! - [`Config`] is a `fonts.conf`: font directories and family aliases.
//!   [`POLY_FONTS_CONF`] is Poly's own, installed as
//!   `/etc/fonts/fonts.conf`; the real fontconfig reads it too.
//!
//! # Matching
//!
//! The pattern first goes through the configuration's rules (aliases
//! expand generic names like `sans-serif` into real families) and gets
//! defaults for what it leaves open (regular weight, roman slant, normal
//! width, 12 points at 75 dpi). Then every font is scored on the
//! properties in the documented order of priority (family, spacing,
//! pixel size, style, slant, weight, then width, which the documentation
//! defines but does not rank), and the font with the best score in the
//! first property where they differ wins; ties go to the font added
//! first. Per property:
//!
//! - family: the position in the pattern's family list of the first family
//!   the font has (names compare without case and blanks); fonts with
//!   none of the families come last;
//! - style: 0 if the font has one of the styles asked for, else 1;
//! - spacing, slant, weight, width, pixel size: the distance between the
//!   values (a font without spacing is proportional, 0); pixel size is 0
//!   for scalable fonts, which have every size.
//!
//! Foundry, charset and language, which the documentation ranks too, are
//! not used: fonts here have no foundry, and coverage is not modelled.

mod config;
mod pattern;
mod xml;

pub use config::{Alias, Config, ConfigError, Dir, Include, Rule, family_eq};
pub use pattern::{CONSTANTS, Kind, ParseError, Pattern, Value, constant, kind, parse_value};

use crate::font::{Font, OutlineFormat, font_count, name_id};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

/// Poly's `fonts.conf` (`/etc/fonts/fonts.conf` on the root disk).
pub const POLY_FONTS_CONF: &str = include_str!("../../fonts.conf");

/// fontconfig's weight for an OpenType `usWeightClass`: linear between
/// the classes that name the weight constants (100 thin = 0, 200
/// extra-light = 40, 300 light = 50, 350 semi-light = 55, 380 book = 75,
/// 400 regular = 80, 500 medium = 100, 600 semi-bold = 180, 700 bold =
/// 200, 800 extra-bold = 205, 900 black = 210, 950 extra-black = 215).
pub fn weight_from_opentype(class: u16) -> i32 {
    const ANCHORS: [(f64, f64); 12] = [
        (100.0, 0.0),
        (200.0, 40.0),
        (300.0, 50.0),
        (350.0, 55.0),
        (380.0, 75.0),
        (400.0, 80.0),
        (500.0, 100.0),
        (600.0, 180.0),
        (700.0, 200.0),
        (800.0, 205.0),
        (900.0, 210.0),
        (950.0, 215.0),
    ];
    let c = f64::from(class);
    if c <= ANCHORS[0].0 {
        return 0;
    }
    for w in ANCHORS.windows(2) {
        let ((c0, f0), (c1, f1)) = (w[0], w[1]);
        if c <= c1 {
            let v = f0 + (f1 - f0) * (c - c0) / (c1 - c0);
            return (v + 0.5) as i32;
        }
    }
    215
}

/// fontconfig's width for an OpenType `usWidthClass` (1 to 9: the width
/// constants from ultra-condensed to ultra-expanded).
pub fn width_from_opentype(class: u16) -> i32 {
    const WIDTHS: [i32; 9] = [50, 63, 75, 87, 100, 113, 125, 150, 200];
    WIDTHS.get(usize::from(class).wrapping_sub(1)).copied().unwrap_or(100)
}

/// The pattern describing font `index` of `file`.
pub fn describe(font: &Font<'_>, file: &str, index: u32) -> Pattern {
    let mut p = Pattern::new();
    let push_unique = |p: &mut Pattern, prop: &str, v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.trim().is_empty())
            && !p.strings(prop).any(|have| family_eq(have, &v))
        {
            p.add(prop, Value::Str(v));
        }
    };
    push_unique(&mut p, "family", font.name(name_id::TYPOGRAPHIC_FAMILY));
    push_unique(&mut p, "family", font.name(name_id::FAMILY));
    push_unique(&mut p, "style", font.name(name_id::TYPOGRAPHIC_SUBFAMILY));
    push_unique(&mut p, "style", font.name(name_id::SUBFAMILY));
    push_unique(&mut p, "fullname", font.name(name_id::FULL_NAME));
    push_unique(&mut p, "postscriptname", font.name(name_id::POSTSCRIPT_NAME));
    let s = font.style();
    let style_says_oblique = p.strings("style").any(|st| st.to_ascii_lowercase().contains("oblique"));
    let slant = if s.oblique || (s.italic && style_says_oblique) {
        110
    } else if s.italic {
        100
    } else {
        0
    };
    p.add("slant", Value::Int(slant));
    p.add("weight", Value::Int(weight_from_opentype(s.weight_class)));
    p.add("width", Value::Int(width_from_opentype(s.width_class)));
    if s.monospace {
        p.add("spacing", Value::Int(100));
    }
    p.add("file", Value::Str(file.to_string()));
    p.add("index", Value::Int(index as i32));
    let format = match font.outline_format() {
        OutlineFormat::TrueType => Some("TrueType"),
        OutlineFormat::Cff => Some("CFF"),
        OutlineFormat::None => None,
    };
    if let Some(f) = format {
        p.add("outline", Value::Bool(true));
        p.add("scalable", Value::Bool(true));
        p.add("fontformat", Value::Str(f.to_string()));
    }
    p
}

/// fontconfig's defaults for what a pattern leaves open: regular weight,
/// roman slant, normal width, 12 points, 75 dpi, and the pixel size
/// those give (or the point size a given pixel size gives).
pub fn default_substitute(p: &mut Pattern) {
    for (name, v) in [("weight", 80), ("slant", 0), ("width", 100)] {
        if !p.has(name) {
            p.add(name, Value::Int(v));
        }
    }
    if !p.has("dpi") {
        p.add("dpi", Value::Double(75.0));
    }
    let dpi = p.get_f64("dpi").filter(|d| *d > 0.0).unwrap_or(75.0);
    match (p.get_f64("size"), p.get_f64("pixelsize")) {
        (Some(size), None) => p.add("pixelsize", Value::Double(size * dpi / 72.0)),
        (None, Some(px)) => p.add("size", Value::Double(px * 72.0 / dpi)),
        (None, None) => {
            p.add("size", Value::Double(12.0));
            p.add("pixelsize", Value::Double(12.0 * dpi / 72.0));
        }
        (Some(_), Some(_)) => {}
    }
}

/// The match score of a font for a prepared pattern: lower is better,
/// compared property by property in priority order.
fn score(p: &Pattern, f: &Pattern) -> [f64; 7] {
    let fams: Vec<&str> = p.strings("family").collect();
    let family = if fams.is_empty() {
        0.0
    } else {
        fams.iter().position(|pf| f.strings("family").any(|ff| family_eq(pf, ff))).unwrap_or(fams.len()) as f64
    };
    let dist = |name: &str, font_default: f64| match p.get_f64(name) {
        Some(want) => (want - f.get_f64(name).unwrap_or(font_default)).abs(),
        None => 0.0,
    };
    let pixelsize = if f.get_bool("scalable") == Some(true) { 0.0 } else { dist("pixelsize", 0.0) };
    let styles: Vec<&str> = p.strings("style").collect();
    let style = if styles.is_empty() || f.strings("style").any(|fs| styles.iter().any(|ps| family_eq(ps, fs))) {
        0.0
    } else {
        1.0
    };
    [family, dist("spacing", 0.0), pixelsize, style, dist("slant", 0.0), dist("weight", 80.0), dist("width", 100.0)]
}

fn compare(a: &[f64; 7], b: &[f64; 7]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        match x.partial_cmp(y) {
            Some(Ordering::Equal) | None => {}
            Some(o) => return o,
        }
    }
    Ordering::Equal
}

/// Whether a font value equals a pattern value.
fn value_eq(name: &str, a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => {
            if matches!(name, "family" | "style" | "fullname") {
                family_eq(x, y)
            } else {
                x == y
            }
        }
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-6,
            _ => false,
        },
    }
}

/// A set of fonts to search.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontSet {
    fonts: Vec<Pattern>,
}

impl FontSet {
    pub fn new() -> FontSet {
        FontSet::default()
    }

    /// Add a font's pattern.
    pub fn add(&mut self, p: Pattern) {
        self.fonts.push(p);
    }

    /// Add every font in a file's data (all of a collection); returns how
    /// many were added (0: not a font this crate reads).
    pub fn add_file(&mut self, file: &str, data: &[u8]) -> usize {
        let mut n = 0;
        for i in 0..font_count(data).min(256) {
            if let Ok(f) = Font::parse_index(data, i) {
                self.fonts.push(describe(&f, file, i));
                n += 1;
            }
        }
        n
    }

    pub fn fonts(&self) -> &[Pattern] {
        &self.fonts
    }

    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// `fc-list`: the fonts having, for every property of the pattern, one
    /// of the values asked for.
    pub fn list(&self, pattern: &Pattern) -> Vec<&Pattern> {
        self.fonts
            .iter()
            .filter(|f| {
                pattern.names().all(|name| {
                    let want = pattern.get(name);
                    f.get(name).iter().any(|have| want.iter().any(|w| value_eq(name, have, w)))
                })
            })
            .collect()
    }

    /// The pattern as matching sees it: the configuration's rules applied
    /// and defaults filled in.
    pub fn prepare(config: &Config, pattern: &Pattern) -> Pattern {
        let mut p = pattern.clone();
        config.substitute(&mut p);
        default_substitute(&mut p);
        p
    }

    /// `fc-match --sort`: every font, best match first.
    pub fn sort(&self, config: &Config, pattern: &Pattern) -> Vec<&Pattern> {
        let p = FontSet::prepare(config, pattern);
        let mut scored: Vec<([f64; 7], usize)> =
            self.fonts.iter().enumerate().map(|(i, f)| (score(&p, f), i)).collect();
        scored.sort_by(|a, b| compare(&a.0, &b.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, i)| &self.fonts[i]).collect()
    }

    /// `fc-match`: the best font for the pattern, as the pattern to render
    /// with: the font's properties, plus the ones the font does not have
    /// from the prepared pattern (size, pixel size, dpi...).
    pub fn font_match(&self, config: &Config, pattern: &Pattern) -> Option<Pattern> {
        let p = FontSet::prepare(config, pattern);
        let best = self
            .fonts
            .iter()
            .map(|f| (score(&p, f), f))
            .reduce(|best, next| if compare(&next.0, &best.0) == Ordering::Less { next } else { best })?
            .1;
        let mut out = best.clone();
        for name in p.names() {
            if !out.has(name) {
                out.set(name, p.get(name).to_vec());
            }
        }
        Some(out)
    }
}

/// The last component of a path (`/` or `\` separated).
fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// One line of `fc-match`'s default output: `file: "family" "style"`.
pub fn match_line(p: &Pattern) -> String {
    alloc::format!(
        "{}: \"{}\" \"{}\"",
        basename(p.get_str("file").unwrap_or("")),
        p.get_str("family").unwrap_or(""),
        p.get_str("style").unwrap_or("")
    )
}

/// One line of `fc-list`'s output: with `elements`, the font's values of
/// those properties as a pattern (`fc-list : family style`); without,
/// `file: families:style=styles`.
pub fn list_line(p: &Pattern, elements: &[&str]) -> String {
    if elements.is_empty() {
        alloc::format!("{}: {}", p.get_str("file").unwrap_or(""), p.filter(&["family", "style"]))
    } else {
        alloc::format!("{}", p.filter(elements))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfont::{self, Spec};
    use alloc::vec;

    #[test]
    fn opentype_classes_to_fontconfig_values() {
        assert_eq!(weight_from_opentype(1), 0);
        assert_eq!(weight_from_opentype(100), 0);
        assert_eq!(weight_from_opentype(400), 80);
        assert_eq!(weight_from_opentype(450), 90);
        assert_eq!(weight_from_opentype(700), 200);
        assert_eq!(weight_from_opentype(900), 210);
        assert_eq!(weight_from_opentype(1000), 215);
        assert_eq!(width_from_opentype(5), 100);
        assert_eq!(width_from_opentype(3), 75);
        assert_eq!(width_from_opentype(0), 100);
        assert_eq!(width_from_opentype(10), 100);
    }

    /// A family of synthetic fonts: Poly Sans (regular, bold, oblique,
    /// condensed), Poly Serif (regular, italic), Poly Mono (regular, bold).
    fn set() -> FontSet {
        let specs = [
            ("sans.ttf", Spec { family: "Poly Sans", style: "Book", ..Spec::default() }),
            (
                "sans-bold.ttf",
                Spec { family: "Poly Sans", style: "Bold", weight: 700, fs_selection: 0x20, ..Spec::default() },
            ),
            ("sans-oblique.ttf", Spec { family: "Poly Sans", style: "Oblique", fs_selection: 0x01, ..Spec::default() }),
            (
                "sans-condensed.ttf",
                Spec {
                    family: "Poly Sans Condensed",
                    style: "Book",
                    typographic: Some(("Poly Sans", "Condensed")),
                    width: 3,
                    ..Spec::default()
                },
            ),
            ("serif.ttf", Spec { family: "Poly Serif", style: "Book", ..Spec::default() }),
            ("serif-italic.ttf", Spec { family: "Poly Serif", style: "Italic", fs_selection: 0x01, ..Spec::default() }),
            ("mono.ttf", Spec { family: "Poly Mono", style: "Book", mono: true, ..Spec::default() }),
            (
                "mono-bold.ttf",
                Spec {
                    family: "Poly Mono",
                    style: "Bold",
                    weight: 700,
                    fs_selection: 0x20,
                    mono: true,
                    ..Spec::default()
                },
            ),
        ];
        let mut s = FontSet::new();
        for (file, spec) in specs {
            assert_eq!(s.add_file(file, &testfont::build_styled(&spec)), 1);
        }
        s
    }

    fn config() -> Config {
        Config::parse(
            r#"<fontconfig>
  <match target="pattern">
    <test qual="all" name="family" compare="not_eq"><string>sans-serif</string></test>
    <test qual="all" name="family" compare="not_eq"><string>serif</string></test>
    <test qual="all" name="family" compare="not_eq"><string>monospace</string></test>
    <edit name="family" mode="append_last"><string>sans-serif</string></edit>
  </match>
  <alias><family>sans-serif</family><prefer><family>Poly Sans</family></prefer></alias>
  <alias><family>serif</family><prefer><family>Poly Serif</family></prefer></alias>
  <alias><family>monospace</family><prefer><family>Poly Mono</family></prefer></alias>
</fontconfig>"#,
        )
        .unwrap()
    }

    fn best(s: &FontSet, c: &Config, pattern: &str) -> String {
        let m = s.font_match(c, &Pattern::parse(pattern).unwrap()).unwrap();
        String::from(m.get_str("file").unwrap())
    }

    #[test]
    fn describes_fonts() {
        let s = set();
        let sans = &s.fonts()[0];
        assert_eq!(sans.get_str("family"), Some("Poly Sans"));
        assert_eq!(sans.get_str("style"), Some("Book"));
        assert_eq!(sans.get("weight"), &[Value::Int(80)]);
        assert_eq!(sans.get("slant"), &[Value::Int(0)]);
        assert_eq!(sans.get("width"), &[Value::Int(100)]);
        assert!(!sans.has("spacing"));
        assert_eq!(sans.get_bool("scalable"), Some(true));
        assert_eq!(sans.get_str("fontformat"), Some("TrueType"));
        assert_eq!(sans.get_str("fullname"), Some("Poly Sans Book"));
        assert_eq!(s.fonts()[2].get("slant"), &[Value::Int(110)], "italic bit + 'Oblique' style");
        assert_eq!(s.fonts()[5].get("slant"), &[Value::Int(100)]);
        assert_eq!(s.fonts()[1].get("weight"), &[Value::Int(200)]);
        assert_eq!(s.fonts()[6].get("spacing"), &[Value::Int(100)]);
        let cond = &s.fonts()[3];
        assert_eq!(cond.strings("family").collect::<Vec<_>>(), ["Poly Sans", "Poly Sans Condensed"]);
        assert_eq!(cond.strings("style").collect::<Vec<_>>(), ["Condensed", "Book"]);
        assert_eq!(cond.get("width"), &[Value::Int(75)]);
        assert_eq!(FontSet::new().add_file("x", b"not a font"), 0);
    }

    #[test]
    fn matches_family_then_style_then_size() {
        let s = set();
        let c = config();
        assert_eq!(best(&s, &c, "Poly Sans"), "sans.ttf");
        assert_eq!(best(&s, &c, "poly sans:bold"), "sans-bold.ttf");
        assert_eq!(best(&s, &c, "Poly Sans:style=Bold"), "sans-bold.ttf");
        // Italic asked of a family with only an oblique: the oblique (110
        // is nearer 100 than the roman's 0).
        assert_eq!(best(&s, &c, "Poly Sans:italic"), "sans-oblique.ttf");
        assert_eq!(best(&s, &c, "Poly Serif:italic"), "serif-italic.ttf");
        // Bold asked of a family without one: the family wins over weight.
        assert_eq!(best(&s, &c, "Poly Serif:bold"), "serif.ttf");
        assert_eq!(best(&s, &c, "Poly Sans:condensed"), "sans-condensed.ttf");
        assert_eq!(best(&s, &c, "Poly Sans Condensed"), "sans-condensed.ttf");
        // Generic names through the aliases.
        assert_eq!(best(&s, &c, "sans-serif"), "sans.ttf");
        assert_eq!(best(&s, &c, "serif"), "serif.ttf");
        assert_eq!(best(&s, &c, "monospace:bold"), "mono-bold.ttf");
        // Unknown families fall back to sans-serif; so does no family.
        assert_eq!(best(&s, &c, "Comic Sans MS"), "sans.ttf");
        assert_eq!(best(&s, &c, ":bold"), "sans-bold.ttf");
        // Family lists: the first family present wins.
        assert_eq!(best(&s, &c, "Nope,Poly Serif,Poly Sans"), "serif.ttf");
        // Family ranks above spacing: `:spacing=mono` alone keeps the
        // fallback family (a monospaced font is asked for by family).
        assert_eq!(best(&s, &c, ":spacing=mono"), "sans.ttf");
        assert_eq!(best(&s, &c, "Poly Sans,Poly Mono:spacing=mono"), "sans.ttf");
        // Sizes do not matter to scalable fonts, and are passed through.
        let m = s.font_match(&c, &Pattern::parse("Poly Mono-10").unwrap()).unwrap();
        assert_eq!(m.get_str("file"), Some("mono.ttf"));
        assert_eq!(m.get_f64("size"), Some(10.0));
        assert!((m.get_f64("pixelsize").unwrap() - 10.0 * 75.0 / 72.0).abs() < 1e-9);
        assert_eq!(match_line(&m), r#"mono.ttf: "Poly Mono" "Book""#);
        assert!(FontSet::new().font_match(&c, &Pattern::new()).is_none());
    }

    #[test]
    fn sorts_and_lists() {
        let s = set();
        let c = config();
        let sorted: Vec<&str> =
            s.sort(&c, &Pattern::parse("Poly Mono:bold").unwrap()).iter().map(|p| p.get_str("file").unwrap()).collect();
        assert_eq!(sorted[..2], ["mono-bold.ttf", "mono.ttf"]);
        assert_eq!(sorted.len(), 8);
        let files = |pat: &str| -> Vec<String> {
            s.list(&Pattern::parse(pat).unwrap()).iter().map(|p| p.get_str("file").unwrap().to_string()).collect()
        };
        assert_eq!(files(""), s.fonts().iter().map(|p| p.get_str("file").unwrap().to_string()).collect::<Vec<_>>());
        assert_eq!(files("Poly Sans:bold"), ["sans-bold.ttf"]);
        assert_eq!(files(":spacing=mono"), ["mono.ttf", "mono-bold.ttf"]);
        assert_eq!(files("Poly Serif:style=italic,bold"), ["serif-italic.ttf"]);
        assert_eq!(files("polysans:width=75"), ["sans-condensed.ttf"]);
        assert!(files("Nope").is_empty());
        let mono = s.list(&Pattern::parse("Poly Mono:bold").unwrap())[0];
        assert_eq!(list_line(mono, &["family", "style"]), "Poly Mono:style=Bold");
        assert_eq!(list_line(mono, &[]), "mono-bold.ttf: Poly Mono:style=Bold");
        assert_eq!(list_line(mono, &["weight"]), ":weight=200");
    }

    #[test]
    fn polys_fonts_conf_parses() {
        let c = Config::parse(POLY_FONTS_CONF).unwrap();
        assert_eq!(c.ignored, 0);
        assert!(c.dirs.iter().any(|d| d.path == "/usr/share/fonts"));
        let mut p = Pattern::parse("Arial").unwrap();
        c.substitute(&mut p);
        let fams: Vec<&str> = p.strings("family").collect();
        assert_eq!(fams[0], "Arial");
        assert!(fams.contains(&"DejaVu Sans"), "{fams:?}");
        let mut p = Pattern::parse("monospace").unwrap();
        c.substitute(&mut p);
        assert_eq!(p.get_str("family"), Some("DejaVu Sans Mono"));
        // A Windows monospaced name ends up monospaced, not sans-serif.
        let mut p = Pattern::parse("Courier New").unwrap();
        c.substitute(&mut p);
        let fams: Vec<&str> = p.strings("family").collect();
        assert_eq!(fams, vec!["Courier New", "DejaVu Sans Mono", "monospace"]);
    }
}
