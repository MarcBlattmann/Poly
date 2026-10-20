//! SVG progress graphics for the README: badges, the roadmap chart and the
//! API coverage treemap. Output is deterministic so CI can check it is fresh.

use crate::model::{Coverage, Layer, Os, Status};
use crate::roadmap::Roadmap;
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub name: &'static str,
    bg: &'static str,
    border: &'static str,
    text: &'static str,
    muted: &'static str,
    track: &'static str,
    done: &'static str,
    stub: &'static str,
    claimed: &'static str,
    missing: &'static str,
    blocked: &'static str,
    current: &'static str,
}

pub const LIGHT: Theme = Theme {
    name: "light",
    bg: "#ffffff",
    border: "#d0d7de",
    text: "#1f2328",
    muted: "#59636e",
    track: "#eff2f5",
    done: "#2da44e",
    stub: "#d4a72c",
    claimed: "#218bff",
    missing: "#e6eaef",
    blocked: "#8c959f",
    current: "#0969da",
};

pub const DARK: Theme = Theme {
    name: "dark",
    bg: "#0d1117",
    border: "#30363d",
    text: "#e6edf3",
    muted: "#9198a1",
    track: "#161b22",
    done: "#2ea043",
    stub: "#bb8009",
    claimed: "#388bfd",
    missing: "#21262d",
    blocked: "#6e7681",
    current: "#4493f8",
};

const FONT: &str = "-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `1234567` → `1,234,567`
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn percent(done: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { done as f64 * 100.0 / total as f64 }
}

struct Svg {
    out: String,
}

impl Svg {
    fn new(w: f64, h: f64, label: &str) -> Svg {
        let mut out = String::new();
        let _ = write!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img" aria-label="{}">"#,
            esc(label)
        );
        out.push('\n');
        Svg { out }
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, fill: &str, extra: &str) {
        let _ = writeln!(
            self.out,
            r#"<rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{:.1}" fill="{fill}"{extra}/>"#,
            w.max(0.0),
            h.max(0.0)
        );
    }

    #[allow(clippy::too_many_arguments, reason = "mirrors the SVG <text> attributes one to one")]
    fn text(&mut self, x: f64, y: f64, size: f64, fill: &str, weight: u32, anchor: &str, s: &str) {
        let _ = writeln!(
            self.out,
            r#"<text x="{x:.1}" y="{y:.1}" font-family="{FONT}" font-size="{size}" font-weight="{weight}" fill="{fill}" text-anchor="{anchor}">{}</text>"#,
            esc(s)
        );
    }

    fn raw(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn finish(mut self) -> String {
        self.out.push_str("</svg>\n");
        self.out
    }
}

// ---------------------------------------------------------------- badges

/// Rough text width for 11px Verdana-like fonts.
fn text_width(s: &str) -> f64 {
    s.chars()
        .map(|c| {
            if c.is_ascii_uppercase() || c == '%' {
                7.5
            } else if c == ' ' {
                3.5
            } else {
                6.5
            }
        })
        .sum()
}

fn badge_color(pct: f64) -> &'static str {
    match pct {
        p if p >= 90.0 => "#2da44e",
        p if p >= 60.0 => "#79b829",
        p if p >= 30.0 => "#d4a72c",
        p if p >= 10.0 => "#e3742c",
        _ => "#cf4f3a",
    }
}

pub fn badge(label: &str, value: &str, pct: f64) -> String {
    let lw = text_width(label) + 12.0;
    let vw = text_width(value) + 12.0;
    let w = lw + vw;
    let mut s = Svg::new(w, 20.0, &format!("{label}: {value}"));
    s.raw(&format!(r#"<clipPath id="r"><rect width="{w}" height="20" rx="4"/></clipPath><g clip-path="url(#r)">"#));
    s.rect(0.0, 0.0, lw, 20.0, "#444d56", "");
    s.rect(lw, 0.0, vw, 20.0, badge_color(pct), "");
    s.raw("</g>");
    s.text(lw / 2.0, 14.0, 11.0, "#ffffff", 600, "middle", label);
    s.text(lw + vw / 2.0, 14.0, 11.0, "#ffffff", 600, "middle", value);
    s.finish()
}

// ---------------------------------------------------------------- roadmap

pub fn roadmap_chart(r: &Roadmap, t: &Theme) -> String {
    const W: f64 = 900.0;
    const ROW: f64 = 30.0;
    const TOP: f64 = 46.0;
    const LABEL_W: f64 = 340.0;
    const COUNT_W: f64 = 70.0;
    let bar_x = LABEL_W;
    let bar_w = W - LABEL_W - COUNT_W - 16.0;
    let h = TOP + ROW * r.phases.len() as f64 + 30.0;
    let (done, total) = (r.done() as u64, r.total() as u64);
    let mut s = Svg::new(W, h, &format!("Roadmap: {done} of {total} milestones done"));
    s.rect(0.5, 0.5, W - 1.0, h - 1.0, t.bg, &format!(r#" rx="8" stroke="{}""#, t.border));
    s.text(16.0, 28.0, 16.0, t.text, 600, "start", "Roadmap");
    s.text(
        W - 16.0,
        28.0,
        13.0,
        t.muted,
        400,
        "end",
        &format!("{done} / {total} milestones · {:.0}%", percent(done, total)),
    );

    let current = r.current().map(|p| p.number);
    for (i, phase) in r.phases.iter().enumerate() {
        let y = TOP + ROW * i as f64;
        let is_current = Some(phase.number) == current;
        let color = if is_current {
            t.current
        } else if phase.is_complete() {
            t.text
        } else {
            t.muted
        };
        let weight = if is_current { 600 } else { 400 };
        s.text(16.0, y + 19.0, 13.0, color, weight, "start", &format!("{:>2}  {}", phase.number, phase.title));
        if is_current {
            s.text(LABEL_W - 12.0, y + 19.0, 11.0, t.current, 600, "end", "NOW");
        }

        // One block per milestone; hover shows its text.
        let n = phase.items.len().max(1) as f64;
        let gap = if n > 40.0 { 1.0 } else { 2.0 };
        let bw = (bar_w - gap * (n - 1.0)) / n;
        for (j, item) in phase.items.iter().enumerate() {
            let x = bar_x + j as f64 * (bw + gap);
            let fill = if item.done {
                t.done
            } else if is_current && phase.items.iter().position(|it| !it.done) == Some(j) {
                t.current
            } else {
                t.track
            };
            let stroke = if item.done { String::new() } else { format!(r#" stroke="{}""#, t.border) };
            let _ = write!(s.out, r#"<g><title>{}{}</title>"#, if item.done { "✔ " } else { "" }, esc(&item.text));
            s.out.push('\n');
            s.rect(x, y + 7.0, bw, 16.0, fill, &format!(r#" rx="3"{stroke}"#));
            s.raw("</g>");
        }
        s.text(W - 16.0, y + 19.0, 12.0, t.muted, 400, "end", &format!("{}/{}", phase.done(), phase.items.len()));
    }

    let ly = h - 14.0;
    legend(&mut s, 16.0, ly, &[(t.done, "done"), (t.current, "next up"), (t.track, "to do")], t);
    s.finish()
}

fn legend(s: &mut Svg, mut x: f64, y: f64, items: &[(&str, &str)], t: &Theme) {
    for (color, label) in items {
        s.rect(x, y - 9.0, 10.0, 10.0, color, &format!(r#" rx="2" stroke="{}""#, t.border));
        s.text(x + 15.0, y, 11.0, t.muted, 400, "start", label);
        x += 30.0 + text_width(label);
    }
}

// ---------------------------------------------------------------- treemap

/// `name` shortened to fit `width` pixels, or `None` if not even a few
/// characters fit.
fn fit_label(name: &str, width: f64) -> Option<String> {
    if text_width(name) * 0.95 <= width {
        return Some(name.to_string());
    }
    let mut out = String::new();
    for c in name.chars() {
        if text_width(&format!("{out}{c}…")) * 0.95 > width {
            break;
        }
        out.push(c);
    }
    (out.chars().count() >= 3).then(|| format!("{}…", out.trim_end()))
}

#[derive(Clone, Debug, Default)]
struct Group {
    name: String,
    counts: BTreeMap<Status, u64>,
}

impl Group {
    fn total(&self) -> u64 {
        self.counts.values().sum()
    }

    fn get(&self, s: Status) -> u64 {
        self.counts.get(&s).copied().unwrap_or(0)
    }
}

/// Display grouping. Families of versioned redistributable DLLs are shown as
/// one block each, so the map shows what matters instead of 20 `msvcp*` tiles.
fn display_group(os: Os, layer: Layer, component: &str) -> String {
    match (os, layer) {
        (Os::Windows, Layer::Syscall) => match component {
            "ntoskrnl" => "NT services".into(),
            "win32k" => "win32k services".into(),
            c => c.into(),
        },
        (Os::Windows, Layer::Api) => {
            let c = component;
            let msvc = ["msvcp", "msvcr", "vcruntime", "msvcirt", "concrt", "vcomp"];
            let dx = ["d3dx", "d3dcompiler_", "xinput", "xaudio", "x3daudio", "xapofx"];
            if msvc.iter().any(|p| c.starts_with(p)) {
                "MSVC runtimes".into()
            } else if dx.iter().any(|p| c.starts_with(p)) {
                "DirectX redist".into()
            } else {
                c.trim_end_matches(".dll").into()
            }
        }
        (Os::Linux, Layer::Syscall) => "syscalls".into(),
        (Os::Macos, Layer::Syscall) => match component {
            "bsd" => "BSD syscalls".into(),
            "mach" => "Mach traps".into(),
            c => c.into(),
        },
        (Os::Macos, Layer::Api) => {
            let c = component;
            if c.starts_with("libswift") && c != "libswiftCore.dylib" {
                "Swift overlays".into()
            } else {
                c.trim_end_matches(".framework").trim_end_matches(".dylib").into()
            }
        }
        _ => component.into(),
    }
}

/// Squarified treemap (Bruls, Huizing, van Wijk 2000). `values` must be
/// sorted in descending order. Returns one rectangle per value.
fn squarify(values: &[f64], x: f64, y: f64, w: f64, h: f64) -> Vec<(f64, f64, f64, f64)> {
    let total: f64 = values.iter().sum();
    let mut out = Vec::with_capacity(values.len());
    if total <= 0.0 {
        return out;
    }
    let scale = w * h / total;
    let areas: Vec<f64> = values.iter().map(|v| v * scale).collect();
    let (mut x, mut y, mut w, mut h) = (x, y, w, h);
    let mut i = 0;
    while i < areas.len() {
        let side = w.min(h);
        let mut j = i + 1;
        let worst = |row: &[f64]| {
            let s: f64 = row.iter().sum();
            let (mx, mn) = row.iter().fold((f64::MIN, f64::MAX), |(a, b), &v| (a.max(v), b.min(v)));
            (side * side * mx / (s * s)).max(s * s / (side * side * mn))
        };
        while j < areas.len() && worst(&areas[i..=j]) <= worst(&areas[i..j]) {
            j += 1;
        }
        let row = &areas[i..j];
        let sum: f64 = row.iter().sum();
        let thick = sum / side;
        let mut off = 0.0;
        for a in row {
            let len = a / thick;
            if w >= h {
                out.push((x, y + off, thick, len));
            } else {
                out.push((x + off, y, len, thick));
            }
            off += len;
        }
        if w >= h {
            x += thick;
            w -= thick;
        } else {
            y += thick;
            h -= thick;
        }
        i = j;
    }
    out
}

const STATUS_ORDER: [Status; 5] =
    [Status::Green, Status::Stub, Status::Claimed, Status::BlockedByDesign, Status::Missing];

fn status_color(st: Status, t: &Theme) -> &'static str {
    match st {
        Status::Green => t.done,
        Status::Stub => t.stub,
        Status::Claimed => t.claimed,
        Status::BlockedByDesign => t.blocked,
        Status::Missing => t.missing,
    }
}

/// Draw one group: one cell per entry when cells are big enough to see,
/// otherwise a proportional fill per status.
fn draw_group(s: &mut Svg, g: &Group, (x, y, w, h): (f64, f64, f64, f64), t: &Theme) {
    let total = g.total();
    let pct = percent(g.get(Status::Green), total);
    let _ = writeln!(
        s.out,
        "<g><title>{}: {} / {} done ({pct:.1}%)</title>",
        esc(&g.name),
        thousands(g.get(Status::Green)),
        thousands(total)
    );
    let (ix, iy, iw, ih) = (x + 1.0, y + 1.0, (w - 2.0).max(0.0), (h - 2.0).max(0.0));
    s.rect(ix, iy, iw, ih, t.missing, "");
    let statuses: Vec<Status> =
        STATUS_ORDER.iter().copied().flat_map(|st| std::iter::repeat_n(st, g.get(st) as usize)).collect();
    let cell_area = iw * ih / total.max(1) as f64;
    if cell_area >= 20.0 && total <= 2000 {
        let rows = ((total as f64 * ih / iw).sqrt().round() as u64).clamp(1, total);
        let mut k = 0usize;
        for r in 0..rows {
            let in_row = total / rows + u64::from(r < total % rows);
            let cw = iw / in_row as f64;
            let rh = ih / rows as f64;
            for c in 0..in_row {
                let st = statuses[k];
                k += 1;
                if st != Status::Missing {
                    s.rect(ix + c as f64 * cw, iy + r as f64 * rh, cw, rh, status_color(st, t), "");
                }
                s.rect(
                    ix + c as f64 * cw,
                    iy + r as f64 * rh,
                    cw,
                    rh,
                    "none",
                    &format!(r#" stroke="{}" stroke-width="0.5""#, t.bg),
                );
            }
        }
    } else {
        let mut off = 0.0;
        for st in STATUS_ORDER {
            let n = g.get(st);
            if n == 0 || st == Status::Missing {
                continue;
            }
            let fw = iw * n as f64 / total as f64;
            s.rect(ix + off, iy, fw, ih, status_color(st, t), "");
            off += fw;
        }
    }
    s.rect(x, y, w, h, "none", &format!(r#" stroke="{}" stroke-width="1.5""#, t.bg));

    // Label: full name if it fits, otherwise shortened with an ellipsis.
    if let Some(label) = fit_label(&g.name, w - 10.0)
        && h > 20.0
    {
        s.text(x + 6.0, y + 15.0, 11.0, t.text, 500, "start", &label);
        if h > 34.0 && w > 46.0 {
            s.text(x + 6.0, y + 29.0, 10.0, t.muted, 400, "start", &format!("{pct:.0}%"));
        }
    }
    s.raw("</g>");
}

/// Layout follows the data: the OS with the most entries gets a full-width
/// row, the second two thirds of the row below, the smallest one third.
pub fn coverage_map(c: &Coverage, t: &Theme) -> String {
    const W: f64 = 900.0;
    const PAD: f64 = 15.0;
    const GAP: f64 = 15.0;
    let inner = W - 2.0 * PAD - GAP;
    let mut order = Os::ALL;
    order.sort_by_key(|os| std::cmp::Reverse(c.entries.iter().filter(|e| e.os == *os).count()));
    let (wide, second_w, third_w) = (W - 2.0 * PAD, inner * 2.0 / 3.0, inner / 3.0);
    let layout = [
        (order[0], PAD, 46.0, wide, 330.0, 300),
        (order[1], PAD, 430.0, second_w, 220.0, 200),
        (order[2], PAD + second_w + GAP, 430.0, third_w, 220.0, 60),
    ];
    let h = 430.0 + PANEL_HEAD + 220.0 + 36.0;

    let mut s = Svg::new(W, h, "API coverage map");
    s.rect(0.5, 0.5, W - 1.0, h - 1.0, t.bg, &format!(r#" rx="8" stroke="{}""#, t.border));
    s.text(16.0, 28.0, 16.0, t.text, 600, "start", "API coverage");
    s.text(
        W - 16.0,
        28.0,
        12.0,
        t.muted,
        400,
        "end",
        "one block per library or call table, sized by its number of entries",
    );
    for (os, x, y, w, map_h, max_groups) in layout {
        panel(&mut s, c, os, (x, y, w, map_h), max_groups, t);
    }
    legend(
        &mut s,
        16.0,
        h - 12.0,
        &[(t.done, "green"), (t.stub, "stub"), (t.claimed, "claimed"), (t.blocked, "blocked"), (t.missing, "missing")],
        t,
    );
    s.finish()
}

const PANEL_HEAD: f64 = 38.0;

fn panel(s: &mut Svg, c: &Coverage, os: Os, (x, y, w, map_h): (f64, f64, f64, f64), max_groups: usize, t: &Theme) {
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for e in c.entries.iter().filter(|e| e.os == os) {
        let name = display_group(os, e.layer, &e.component);
        let g = groups.entry(name.clone()).or_insert_with(|| Group { name, ..Default::default() });
        *g.counts.entry(e.status).or_default() += 1;
    }
    let mut groups: Vec<Group> = groups.into_values().collect();
    groups.sort_by(|a, b| b.total().cmp(&a.total()).then(a.name.cmp(&b.name)));
    if groups.len() > max_groups {
        let rest = groups.split_off(max_groups - 1);
        let mut other = Group { name: format!("{} smaller libraries", rest.len()), ..Default::default() };
        for g in &rest {
            for (st, n) in &g.counts {
                *other.counts.entry(*st).or_default() += n;
            }
        }
        groups.push(other);
        groups.sort_by(|a, b| b.total().cmp(&a.total()).then(a.name.cmp(&b.name)));
    }

    let total: u64 = groups.iter().map(Group::total).sum();
    let done: u64 = groups.iter().map(|g| g.get(Status::Green)).sum();
    let mut sub = format!("{} / {} entries", thousands(done), thousands(total));
    if os == Os::Macos && !c.entries.iter().any(|e| e.os == Os::Macos && e.layer == Layer::Api) {
        sub.push_str(" · frameworks not mapped yet");
    }
    s.text(x, y + 14.0, 14.0, t.text, 600, "start", os.title());
    s.text(x + w, y + 14.0, 14.0, t.text, 600, "end", &format!("{:.1}%", percent(done, total)));
    s.text(x, y + 29.0, 11.0, t.muted, 400, "start", &sub);

    let top = y + PANEL_HEAD;
    if groups.is_empty() {
        s.rect(x, top, w, map_h, t.track, &format!(r#" rx="4" stroke="{}""#, t.border));
        return;
    }
    let values: Vec<f64> = groups.iter().map(|g| g.total() as f64).collect();
    for (g, r) in groups.iter().zip(squarify(&values, x, top, w, map_h)) {
        draw_group(s, g, r, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_covers_area() {
        let rects = squarify(&[6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0], 0.0, 0.0, 6.0, 4.0);
        assert_eq!(rects.len(), 7);
        let area: f64 = rects.iter().map(|r| r.2 * r.3).sum();
        assert!((area - 24.0).abs() < 1e-9);
        for (x, y, w, h) in rects {
            assert!(x >= -1e-9 && y >= -1e-9 && x + w <= 6.0 + 1e-9 && y + h <= 4.0 + 1e-9);
        }
    }

    #[test]
    fn number_formatting() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(130147), "130,147");
        assert_eq!(percent(1, 0), 0.0);
    }

    #[test]
    fn grouping() {
        assert_eq!(display_group(Os::Windows, Layer::Api, "msvcp140.dll"), "MSVC runtimes");
        assert_eq!(display_group(Os::Windows, Layer::Api, "d3dx9_43.dll"), "DirectX redist");
        assert_eq!(display_group(Os::Windows, Layer::Api, "kernel32.dll"), "kernel32");
        assert_eq!(display_group(Os::Windows, Layer::Syscall, "ntoskrnl"), "NT services");
        assert_eq!(display_group(Os::Macos, Layer::Api, "AppKit.framework"), "AppKit");
        assert_eq!(display_group(Os::Macos, Layer::Api, "libswiftFoundation.dylib"), "Swift overlays");
        assert_eq!(display_group(Os::Macos, Layer::Api, "libswiftCore.dylib"), "libswiftCore");
    }

    #[test]
    fn labels_shorten() {
        assert_eq!(fit_label("ntdll", 200.0).as_deref(), Some("ntdll"));
        let short = fit_label("DirectX redist", 50.0).unwrap();
        assert!(short.ends_with('…') && short.len() < "DirectX redist".len());
        assert_eq!(fit_label("kernel32", 5.0), None);
    }

    #[test]
    fn badge_is_svg() {
        let b = badge("linux", "0.0%", 0.0);
        assert!(b.starts_with("<svg") && b.trim_end().ends_with("</svg>"));
        assert!(b.contains("linux"));
    }
}
