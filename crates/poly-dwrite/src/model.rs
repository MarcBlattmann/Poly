//! Families, their fonts (real and simulated) and font matching, as
//! recorded in the spec sheets of `specs/windows/dwrite.dll`
//! (`IDWriteFontCollection.GetFontFamily`, `IDWriteFontFamily.GetFont`,
//! `GetFirstMatchingFont`, `GetMatchingFonts`).
//!
//! # Families
//!
//! Fonts group under their family name (WWS family, else typographic, else
//! name ID 1), compared ignoring case; families are ordered by their en-us
//! name. A family lists its real fonts (by weight, then distance of the
//! stretch from normal, then style), then the faces DirectWrite simulates:
//! an oblique one for every upright face; then, for each class of style
//! and stretch that has no heavy face (weight over 600), a bold one of the
//! face closest to regular; then the bold oblique ones of those.
//!
//! # Matching
//!
//! Every real font of the family offers its best variant (itself, with the
//! bold simulation, with the oblique simulation, or both) and the variants
//! are ordered by a cost: the distance of the stretch (350 per step), of the
//! weight (1 per unit toward the side the request prefers, 1.75 away from
//! it: lighter faces are preferred for requests up to 500, heavier ones
//! above), of the style (an oblique request wants oblique, then italic; an
//! italic one italic, then oblique), plus a charge for each simulation.
//! The constants were fitted to the observed orderings.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::face::{FaceInfo, SIM_BOLD, SIM_OBLIQUE, STYLE_ITALIC, STYLE_NORMAL, STYLE_OBLIQUE};
use crate::names::{Localized, cmp_ci, utf16};

/// One font of a file as the collection knows it.
#[derive(Clone, Debug)]
pub struct FaceEntry {
    /// An opaque identity of the file (the caller's).
    pub file: usize,
    pub index: u32,
    pub info: Arc<FaceInfo>,
    /// Replace the family names the font groups under.
    pub family: Option<Localized>,
    /// Replace the face names.
    pub face: Option<Localized>,
    /// Override `IsSymbolFont`.
    pub symbol: Option<bool>,
    /// Code point ranges the font claims to cover (all, when `None`).
    pub charset: Option<&'static [(u32, u32)]>,
    /// Which variant of the same font file this entry is (an alias of a
    /// system font has its own).
    pub variant: u32,
    /// Report this weight instead of the font's own.
    pub weight: Option<u32>,
}

impl FaceEntry {
    pub fn new(file: usize, index: u32, info: Arc<FaceInfo>) -> FaceEntry {
        FaceEntry { file, index, info, family: None, face: None, symbol: None, charset: None, variant: 0, weight: None }
    }

    /// `GetInformationalStrings`: the font's own, except that an alias
    /// (a family name of ours over another family's file) answers for its
    /// own names: the family and face (ids 11, 12), a full name ("Arial
    /// Bold") and a PostScript name ("Arial-Bold"); it has no typographic
    /// or WWS names.
    pub fn informational(&self, id: u32) -> Option<Localized> {
        let Some(family) = &self.family else { return self.info.informational(id) };
        let fam = family.primary().map(crate::names::to_string).unwrap_or_default();
        let face = self.face.as_ref().and_then(Localized::primary).map(crate::names::to_string).unwrap_or_default();
        let regular = face.is_empty() || face == "Regular";
        match id {
            11 => Some(family.clone()),
            12 => self.face.clone(),
            13 | 14 | 19 => None,
            16 => Some(Localized::en_us(&if regular { fam } else { alloc::format!("{fam} {face}") })),
            17 => {
                let compact = |t: &str| t.chars().filter(|c| *c != ' ').collect::<String>();
                Some(Localized::en_us(&if regular {
                    compact(&fam)
                } else {
                    alloc::format!("{}-{}", compact(&fam), compact(&face))
                }))
            }
            _ => self.info.informational(id),
        }
    }

    fn family_names(&self) -> &Localized {
        self.family.as_ref().unwrap_or(&self.info.family)
    }

    fn face_names(&self) -> &Localized {
        self.face.as_ref().unwrap_or(&self.info.face)
    }
}

/// A font of a family: a real font or a simulated one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontDesc {
    /// Index into the entries the families were built from.
    pub entry: usize,
    pub weight: u32,
    pub stretch: u32,
    pub style: u32,
    pub simulations: u32,
    pub face_names: Localized,
}

#[derive(Clone, Debug)]
pub struct Family {
    pub names: Localized,
    pub fonts: Vec<FontDesc>,
}

fn key(l: &Localized) -> Vec<u16> {
    l.primary().map(<[u16]>::to_vec).unwrap_or_default().iter().map(|&c| crate::names::lower(c)).collect()
}

/// Group `entries` into families (see the module documentation).
pub fn build_families(entries: &[FaceEntry]) -> Vec<Family> {
    // Group by family key, keeping first-seen names.
    let mut groups: Vec<(Vec<u16>, Localized, Vec<usize>)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let names = e.family_names().clone();
        let k = key(&names);
        match groups.iter_mut().find(|g| g.0 == k) {
            Some(g) => {
                for (loc, text) in &names.entries {
                    g.1.insert(loc, text);
                }
                g.2.push(i)
            }
            None => groups.push((k, names, alloc::vec![i])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups.into_iter().map(|(_, names, members)| Family { names, fonts: family_fonts(entries, &members) }).collect()
}

fn real_font(entries: &[FaceEntry], i: usize) -> FontDesc {
    let e = &entries[i];
    FontDesc {
        entry: i,
        weight: e.weight.unwrap_or(e.info.weight),
        stretch: e.info.stretch,
        style: e.info.style,
        simulations: 0,
        face_names: e.face_names().clone(),
    }
}

fn family_fonts(entries: &[FaceEntry], members: &[usize]) -> Vec<FontDesc> {
    let mut real: Vec<FontDesc> = members.iter().map(|&i| real_font(entries, i)).collect();
    real.sort_by_key(|f| (f.weight, f.stretch.abs_diff(5), f.stretch, f.style));
    let mut out = real.clone();
    let mut sims: Vec<FontDesc> = Vec::new();
    // Oblique simulations of every upright face.
    for f in real.iter().filter(|f| f.style == STYLE_NORMAL) {
        sims.push(simulate(entries, f, SIM_OBLIQUE));
    }
    // Bold simulations of every face of a class (stretch, style) that has
    // no heavy face, and the oblique ones of the upright ones.
    for f in &real {
        let heavy = real.iter().any(|g| g.stretch == f.stretch && g.style == f.style && g.weight > 600);
        if heavy {
            continue;
        }
        sims.push(simulate(entries, f, SIM_BOLD));
        if f.style == STYLE_NORMAL {
            sims.push(simulate(entries, f, SIM_BOLD | SIM_OBLIQUE));
        }
    }
    sims.sort_by_key(|f| (f.weight, f.stretch.abs_diff(5), f.stretch, f.style));
    out.extend(sims);
    out
}

/// The font `base` with simulations added.
pub fn simulate(entries: &[FaceEntry], base: &FontDesc, sims: u32) -> FontDesc {
    let mut f = base.clone();
    f.simulations |= sims;
    if sims & SIM_BOLD != 0 {
        f.weight = 700;
    }
    if sims & SIM_OBLIQUE != 0 {
        f.style = STYLE_OBLIQUE;
    }
    f.face_names = Localized::en_us(&sim_name(entries, base, sims));
    f
}

/// Words of a face name that say how heavy it is.
fn is_weight_word(w: &str) -> bool {
    const WORDS: [&str; 20] = [
        "thin",
        "extralight",
        "ultralight",
        "light",
        "semilight",
        "demilight",
        "regular",
        "normal",
        "book",
        "roman",
        "medium",
        "semibold",
        "demibold",
        "extra",
        "ultra",
        "semi",
        "demi",
        "-",
        "bold",
        "plain",
    ];
    WORDS.iter().any(|x| w.eq_ignore_ascii_case(x))
}

fn join(parts: &[&str]) -> String {
    let mut s = String::new();
    for p in parts.iter().filter(|p| !p.is_empty()) {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(p);
    }
    s
}

/// The en-us name DirectWrite gives a simulated face of `base`: the
/// bold simulation drops the name's weight words and ends it in "Bold"
/// ("Regular" and "Light" give "Bold", "Italic" "Italic Bold", "Condensed"
/// "Condensed Bold"); the oblique simulation appends "Oblique" ("Regular"
/// gives "Oblique").
fn sim_name(entries: &[FaceEntry], base: &FontDesc, sims: u32) -> String {
    let _ = entries;
    let base_name = base.face_names.primary().map(crate::names::to_string).unwrap_or_default();
    let mut name = if sims & SIM_BOLD != 0 {
        let kept: Vec<&str> = base_name.split(' ').filter(|w| !w.is_empty() && !is_weight_word(w)).collect();
        let mut parts = kept;
        parts.push("Bold");
        join(&parts)
    } else if base_name == "Regular" || base_name.is_empty() {
        String::new()
    } else {
        base_name
    };
    if sims & SIM_OBLIQUE != 0 {
        name = join(&[&name, "Oblique"]);
    }
    name
}

// ----- matching

const STRETCH_STEP: i64 = 1400;
const BOLD_SIM: i64 = 900;
const OBLIQUE_SIM: i64 = 200;
const STYLE_NEAR: i64 = 800;
const STYLE_FAR: i64 = 1100;

fn weight_cost(request: i64, w: i64) -> i64 {
    let d = w - request;
    let heavier_preferred = request > 500;
    if d == 0 {
        0
    } else if (d > 0) == heavier_preferred {
        4 * d.abs()
    } else {
        7 * d.abs()
    }
}

fn style_cost(request: u32, candidate: u32) -> i64 {
    if request == candidate {
        0
    } else if request == STYLE_NORMAL || candidate == STYLE_NORMAL {
        STYLE_FAR
    } else {
        STYLE_NEAR
    }
}

/// A request, with out-of-range values clamped.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub weight: i64,
    pub stretch: i64,
    pub style: u32,
}

impl Request {
    pub fn new(weight: u32, stretch: u32, style: u32) -> Request {
        Request {
            weight: i64::from(weight.clamp(1, 999)),
            stretch: i64::from(stretch.clamp(1, 9)),
            style: if style > 2 { STYLE_ITALIC } else { style },
        }
    }
}

fn candidates(entries: &[FaceEntry], family: &Family, req: &Request) -> Vec<(i64, i64, u32, usize, FontDesc)> {
    let mut out = Vec::new();
    for (n, base) in family.fonts.iter().enumerate().filter(|(_, f)| f.simulations == 0) {
        let mut best: Option<(i64, FontDesc)> = None;
        for sims in 0..4u32 {
            if sims & SIM_BOLD != 0 && base.weight > 550 {
                continue;
            }
            if sims & SIM_OBLIQUE != 0 && base.style != STYLE_NORMAL {
                continue;
            }
            let mut cost = (i64::from(base.stretch) - req.stretch).abs() * STRETCH_STEP;
            let weight = if sims & SIM_BOLD != 0 { 700 } else { i64::from(base.weight) };
            cost += weight_cost(req.weight, weight);
            let style = if sims & SIM_OBLIQUE != 0 { STYLE_OBLIQUE } else { base.style };
            cost += style_cost(req.style, style);
            if sims & SIM_BOLD != 0 {
                cost += BOLD_SIM;
            }
            if sims & SIM_OBLIQUE != 0 {
                cost += OBLIQUE_SIM;
            }
            if best.as_ref().is_none_or(|b| cost < b.0) {
                best = Some((cost, if sims == 0 { base.clone() } else { simulate(entries, base, sims) }));
            }
        }
        let (cost, font) = best.expect("a variant");
        out.push((cost, (i64::from(base.weight) - req.weight).abs(), base.stretch, n, font));
    }
    out.sort_by_key(|c| (c.0, c.1, c.2, c.3));
    out
}

/// The font that best matches the request.
pub fn first_matching(
    entries: &[FaceEntry],
    family: &Family,
    weight: u32,
    stretch: u32,
    style: u32,
) -> Option<FontDesc> {
    let req = Request::new(weight, stretch, style);
    candidates(entries, family, &req).into_iter().next().map(|c| c.4)
}

/// The fonts of the family ordered by how well they match.
pub fn matching_fonts(entries: &[FaceEntry], family: &Family, weight: u32, stretch: u32, style: u32) -> Vec<FontDesc> {
    let req = Request::new(weight, stretch, style);
    candidates(entries, family, &req).into_iter().map(|c| c.4).collect()
}

/// Sort key helper for the collection: families by en-us name.
pub fn name_order(a: &Family, b: &Family) -> core::cmp::Ordering {
    cmp_ci(a.names.primary().unwrap_or(&[]), b.names.primary().unwrap_or(&[]))
}

/// The index of the family called `name` (ignoring case), over every
/// language it has a name in.
pub fn find_family(families: &[Family], name: &[u16]) -> Option<usize> {
    families.iter().position(|f| f.names.entries.iter().any(|e| crate::names::eq_ci(&e.1, name)))
}

/// Convenience for tests: a family's en-us name as a Rust string.
pub fn family_name(f: &Family) -> String {
    f.names.primary().map(crate::names::to_string).unwrap_or_default()
}

#[allow(dead_code)]
fn _unused() -> Vec<u16> {
    utf16("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::face::Metrics;

    fn entry(file: usize, family: &str, face: &str, weight: u32, stretch: u32, style: u32) -> FaceEntry {
        let info = FaceInfo {
            names: Vec::new(),
            weight,
            stretch,
            style,
            is_symbol: false,
            num_glyphs: 8,
            metrics: Metrics::default(),
            family: Localized::en_us(family),
            face: Localized::en_us(face),
            typo_ascender: 0,
            typo_descender: 0,
            extra: Default::default(),
            caret: [0, 1, 0],
        };
        FaceEntry::new(file, 0, Arc::new(info))
    }

    fn alpha() -> Vec<FaceEntry> {
        alloc::vec![
            entry(0, "PolyTest Alpha", "Regular", 400, 5, 0),
            entry(1, "PolyTest Alpha", "Bold", 700, 5, 0),
            entry(2, "PolyTest Alpha", "Italic", 400, 5, 2),
            entry(3, "PolyTest Alpha", "Bold Italic", 700, 5, 2),
        ]
    }

    fn beta() -> Vec<FaceEntry> {
        alloc::vec![
            entry(4, "PolyTest Beta", "Light", 300, 5, 0),
            entry(5, "PolyTest Beta", "Regular", 400, 5, 0),
            entry(6, "PolyTest Beta", "Black", 900, 5, 0),
            entry(7, "PolyTest Beta", "Condensed", 400, 3, 0),
        ]
    }

    fn describe(f: &FontDesc) -> String {
        alloc::format!(
            "w{} s{} st{} sim{} {}",
            f.weight,
            f.stretch,
            f.style,
            f.simulations,
            crate::names::to_string(f.face_names.primary().unwrap())
        )
    }

    #[test]
    fn alpha_family_order() {
        let e = alpha();
        let fams = build_families(&e);
        assert_eq!(fams.len(), 1);
        let l: Vec<String> = fams[0].fonts.iter().map(describe).collect();
        assert_eq!(
            l,
            [
                "w400 s5 st0 sim0 Regular",
                "w400 s5 st2 sim0 Italic",
                "w700 s5 st0 sim0 Bold",
                "w700 s5 st2 sim0 Bold Italic",
                "w400 s5 st1 sim2 Oblique",
                "w700 s5 st1 sim2 Bold Oblique",
            ]
        );
    }

    #[test]
    fn beta_family_order() {
        let e = beta();
        let fams = build_families(&e);
        let l: Vec<String> = fams[0].fonts.iter().map(describe).collect();
        assert_eq!(
            l,
            [
                "w300 s5 st0 sim0 Light",
                "w400 s5 st0 sim0 Regular",
                "w400 s3 st0 sim0 Condensed",
                "w900 s5 st0 sim0 Black",
                "w300 s5 st1 sim2 Light Oblique",
                "w400 s5 st1 sim2 Oblique",
                "w400 s3 st1 sim2 Condensed Oblique",
                "w700 s3 st0 sim1 Condensed Bold",
                "w700 s3 st1 sim3 Condensed Bold Oblique",
                "w900 s5 st1 sim2 Black Oblique",
            ]
        );
    }

    #[test]
    fn alpha_matching_lists() {
        let e = alpha();
        let f = &build_families(&e)[0];
        let m = |w, s, st| matching_fonts(&e, f, w, s, st).iter().map(describe).collect::<Vec<_>>();
        assert_eq!(
            m(400, 5, 0),
            [
                "w400 s5 st0 sim0 Regular",
                "w400 s5 st2 sim0 Italic",
                "w700 s5 st0 sim0 Bold",
                "w700 s5 st2 sim0 Bold Italic"
            ]
        );
        assert_eq!(
            m(700, 5, 0),
            [
                "w700 s5 st0 sim0 Bold",
                "w700 s5 st0 sim1 Bold",
                "w700 s5 st2 sim0 Bold Italic",
                "w700 s5 st2 sim1 Italic Bold"
            ]
        );
        assert_eq!(
            m(400, 5, 1),
            [
                "w400 s5 st1 sim2 Oblique",
                "w400 s5 st2 sim0 Italic",
                "w700 s5 st1 sim2 Bold Oblique",
                "w700 s5 st2 sim0 Bold Italic"
            ]
        );
        assert_eq!(
            m(400, 5, 2),
            [
                "w400 s5 st2 sim0 Italic",
                "w400 s5 st1 sim2 Oblique",
                "w700 s5 st2 sim0 Bold Italic",
                "w700 s5 st1 sim2 Bold Oblique"
            ]
        );
        assert_eq!(
            m(700, 5, 2),
            [
                "w700 s5 st2 sim0 Bold Italic",
                "w700 s5 st2 sim1 Italic Bold",
                "w700 s5 st1 sim2 Bold Oblique",
                "w700 s5 st1 sim3 Bold Oblique"
            ]
        );
    }

    #[test]
    fn beta_matching_lists() {
        let e = beta();
        let f = &build_families(&e)[0];
        let m = |w, s, st| matching_fonts(&e, f, w, s, st).iter().map(describe).collect::<Vec<_>>();
        assert_eq!(
            m(400, 5, 0),
            [
                "w400 s5 st0 sim0 Regular",
                "w300 s5 st0 sim0 Light",
                "w400 s3 st0 sim0 Condensed",
                "w900 s5 st0 sim0 Black"
            ]
        );
        assert_eq!(
            m(700, 5, 0),
            [
                "w900 s5 st0 sim0 Black",
                "w700 s5 st0 sim1 Bold",
                "w700 s5 st0 sim1 Bold",
                "w700 s3 st0 sim1 Condensed Bold"
            ]
        );
        assert_eq!(
            m(900, 3, 0),
            [
                "w700 s3 st0 sim1 Condensed Bold",
                "w900 s5 st0 sim0 Black",
                "w700 s5 st0 sim1 Bold",
                "w700 s5 st0 sim1 Bold"
            ]
        );
        assert_eq!(
            m(300, 5, 0),
            [
                "w300 s5 st0 sim0 Light",
                "w400 s5 st0 sim0 Regular",
                "w400 s3 st0 sim0 Condensed",
                "w900 s5 st0 sim0 Black"
            ]
        );
        assert_eq!(
            m(0, 0, 0),
            [
                "w400 s3 st0 sim0 Condensed",
                "w300 s5 st0 sim0 Light",
                "w400 s5 st0 sim0 Regular",
                "w900 s5 st0 sim0 Black"
            ]
        );
        assert_eq!(
            m(700, 5, 2),
            [
                "w900 s5 st1 sim2 Black Oblique",
                "w700 s5 st1 sim3 Bold Oblique",
                "w700 s5 st1 sim3 Bold Oblique",
                "w700 s3 st1 sim3 Condensed Bold Oblique"
            ]
        );
        assert_eq!(
            m(500, 7, 2),
            [
                "w400 s5 st1 sim2 Oblique",
                "w300 s5 st1 sim2 Light Oblique",
                "w900 s5 st1 sim2 Black Oblique",
                "w400 s3 st1 sim2 Condensed Oblique"
            ]
        );
    }

    #[test]
    fn lone_fonts_get_simulated_neighbours() {
        let names = |e: Vec<FaceEntry>| -> Vec<String> { build_families(&e)[0].fonts.iter().map(describe).collect() };
        // A lone regular font: Oblique, Bold and Bold Oblique are simulated.
        assert_eq!(
            names(alloc::vec![entry(0, "F", "Regular", 400, 5, 0)]),
            [
                "w400 s5 st0 sim0 Regular",
                "w400 s5 st1 sim2 Oblique",
                "w700 s5 st0 sim1 Bold",
                "w700 s5 st1 sim3 Bold Oblique"
            ]
        );
        // A lone italic font named Regular: only a bold simulation.
        assert_eq!(
            names(alloc::vec![entry(0, "F", "Regular", 400, 5, 2)]),
            ["w400 s5 st2 sim0 Regular", "w700 s5 st2 sim1 Bold"]
        );
        // A lone oblique font.
        assert_eq!(
            names(alloc::vec![entry(0, "F", "Oblique", 400, 5, 1)]),
            ["w400 s5 st1 sim0 Oblique", "w700 s5 st1 sim1 Oblique Bold"]
        );
        // Regular, Bold and Italic: an oblique pair and a bold italic.
        let e = alloc::vec![
            entry(0, "F", "Regular", 400, 5, 0),
            entry(1, "F", "Bold", 700, 5, 0),
            entry(2, "F", "Italic", 400, 5, 2),
        ];
        assert_eq!(
            names(e),
            [
                "w400 s5 st0 sim0 Regular",
                "w400 s5 st2 sim0 Italic",
                "w700 s5 st0 sim0 Bold",
                "w400 s5 st1 sim2 Oblique",
                "w700 s5 st1 sim2 Bold Oblique",
                "w700 s5 st2 sim1 Italic Bold"
            ]
        );
    }

    #[test]
    fn aliases_answer_informational_strings_for_themselves() {
        let mut e = entry(0, "DejaVu Sans", "Book", 700, 5, 2);
        e.family = Some(Localized::en_us("Arial"));
        e.face = Some(Localized::en_us("Bold Italic"));
        let text = |id| crate::names::to_string(e.informational(id).unwrap().primary().unwrap());
        assert_eq!(
            (text(11), text(12), text(16), text(17)),
            ("Arial".into(), "Bold Italic".into(), "Arial Bold Italic".into(), "Arial-BoldItalic".into())
        );
        assert!(e.informational(13).is_none() && e.informational(19).is_none());
    }

    #[test]
    fn first_matching_weights() {
        let e = alpha();
        let f = &build_families(&e)[0];
        for w in [0, 1, 100, 399, 400, 450, 501] {
            assert_eq!(first_matching(&e, f, w, 5, 0).unwrap().weight, 400, "w{w}");
        }
        for w in [550, 599, 700, 999, 1000, 5000] {
            assert_eq!(first_matching(&e, f, w, 5, 0).unwrap().weight, 700, "w{w}");
        }
        let b = beta();
        let f = &build_families(&b)[0];
        for (w, want) in [(100, 300), (350, 300), (399, 400), (550, 400), (600, 900), (950, 900)] {
            assert_eq!(first_matching(&b, f, w, 5, 0).unwrap().weight, want, "w{w}");
        }
        let c = first_matching(&b, f, 600, 3, 0).unwrap();
        assert_eq!((c.weight, c.stretch, c.simulations), (700, 3, 1));
        let c = first_matching(&b, f, 300, 3, 0).unwrap();
        assert_eq!((c.weight, c.stretch, c.simulations), (400, 3, 0));
    }
}
