//! Drawing the window.

use super::*;

impl SettingsWindow {
    /// Draw the window, `w` × `h` logical at scale `s`, into `fb` (which is
    /// at least that big, physical).
    pub fn draw(&self, fb: &mut dyn Framebuffer, w: i32, h: i32, s: i32, cx: &Context<'_>) {
        let all = Rect::new(0, 0, w * s, h * s);
        fill(fb, all, all, pal().s0, 255);
        // The page's surface: rounded at the top left, a hairline along its
        // top and left edges.
        let page = Rect::new(t::SIDEBAR, t::HEADER, w - t::SIDEBAR, h - t::HEADER).scale(s);
        let r = (t::CARD_RADIUS * s) as f32;
        let grown = Rect::new(page.x, page.y, page.width + 2 * r as i32, page.height + 2 * r as i32);
        paint::fill_rounded(fb, page, grown, Radii::all(r), pal().line, 255);
        let inner = Rect::new(grown.x + s, grown.y + s, grown.width - s, grown.height - s);
        paint::fill_rounded(fb, page, inner, Radii::all(r - s as f32), pal().s1, 255);
        let (items, _) = self.layout(w, h, cx);
        let text = cx.text;
        for item in &items {
            match item {
                Item::Text { x, y, max_x, text: s_, style, rgb } => {
                    text.draw_style(fb, all, (x * s) as f32, y * s, max_x * s, s_, style.scaled(s), *rgb, 255);
                }
                Item::Card(c) => rounded_rect(fb, all, c.scale(s), t::CARD_RADIUS * s, pal().ink, pal().ghost),
                Item::Line(l) => fill(fb, all, Rect::new(l.x * s, l.y * s, l.width * s, s), pal().ink, pal().hairline),
                Item::Fill { rect, rgb, alpha, radius } => {
                    rounded_rect(fb, all, rect.scale(s), radius * s, *rgb, *alpha)
                }
                Item::Field { rect, focused } => {
                    let f = rect.scale(s);
                    rounded_rect(fb, all, f, t::FIELD_RADIUS * s, pal().s2, 255);
                    let line = if *focused { pal().faint } else { pal().line };
                    rounded_border(fb, all, f, t::FIELD_RADIUS * s, s, line, 255);
                }
                Item::Bar { rect, value } => {
                    let r = rect.scale(s);
                    rounded_rect(fb, all, r, r.height / 2, pal().ink, pal().track);
                    let filled = Rect::new(r.x, r.y, r.width * (*value).min(100) as i32 / 100, r.height);
                    if filled.width > 0 {
                        rounded_rect(fb, all.intersect(&filled), r, r.height / 2, pal().ink_primary, 255);
                    }
                }
                Item::Segmented { rect, options } => {
                    let rs = rect.scale(s);
                    rounded_outline(fb, all, rs, rs.height / 2, pal().line, 255);
                    for (r, label, active) in options {
                        let st = DENSE.scaled(s);
                        let o = r.scale(s);
                        if *active {
                            rounded_rect(fb, all, o, o.height / 2, pal().ink_primary, 255);
                        }
                        let ink = if *active { pal().on_solid } else { pal().quiet };
                        let lw = text.measure(label, st);
                        let base = text.centre(o.y, o.height, st);
                        text.draw_style(
                            fb,
                            all,
                            o.x as f32 + (o.width as f32 - lw) / 2.0,
                            base,
                            o.right(),
                            label,
                            st,
                            ink,
                            255,
                        );
                    }
                }
                Item::Slider { rect, value, enabled } => {
                    let r = rect.scale(s);
                    let a = if *enabled { 255 } else { 110 };
                    rounded_rect(fb, all, r, r.height / 2, pal().ink, pal().track);
                    let filled = Rect::new(r.x, r.y, r.width * *value as i32 / 100, r.height);
                    if filled.width > 0 {
                        rounded_rect(fb, all.intersect(&filled), r, r.height / 2, pal().ink_primary, a);
                    }
                }
                Item::Toggle { rect, on } => {
                    let r = rect.scale(s);
                    let (track, track_a, knob, kx) = if *on {
                        (pal().ink_primary, 255, pal().on_solid, r.x + 19 * s)
                    } else {
                        (pal().ink, pal().track, pal().knob_off, r.x + 3 * s)
                    };
                    rounded_rect(fb, all, r, r.height / 2, track, track_a);
                    let k = Rect::new(kx, r.y + 3 * s, 16 * s, 16 * s);
                    // Alpine's knob shadow, `0 1px 2px rgba(0,0,0,.25)`.
                    Shadow::new(0.0, 1.0, 2.0, 0.0, 0, 64).scaled(s).draw(
                        fb,
                        all,
                        k,
                        Radii::all(8.0 * s as f32),
                        Rect::default(),
                    );
                    rounded_rect(fb, all, k, 8 * s, knob, 255);
                }
                Item::Tile { rect, id, selected } => {
                    let r = rect.scale(s);
                    let mut tile = TileFb { fb, origin: (r.x, r.y), size: (r.width, r.height) };
                    let full = Rect::new(0, 0, r.width, r.height);
                    crate::shell::draw_wallpaper(&mut tile, full, id, r.width, r.height, cx.pictures);
                    // Cut the corners round: the page's colour over them.
                    corner_mask(fb, all, r, 8 * s);
                    if *selected {
                        rounded_border(fb, all, r.grow(3 * s), 11 * s, 2 * s, pal().ink_primary, 255);
                    } else {
                        rounded_border(fb, all, r, 8 * s, s, pal().line, 255);
                    }
                }
                Item::Icon { icon, x, y, size, rgb, alpha } => {
                    icon.draw(fb, all, (x * s) as f32, (y * s) as f32, (size * s) as f32, *rgb, *alpha);
                }
                Item::Button { rect, label, hover } => {
                    let r = rect.scale(s);
                    if *hover {
                        rounded_rect(fb, all, r, t::FIELD_RADIUS * s, pal().ink, pal().ghost);
                    }
                    let st = Style::new(Face::Medium, t::DENSE_SIZE).scaled(s);
                    let lw = text.measure(label, st);
                    let b = text.centre(r.y, r.height, st);
                    text.draw_style(
                        fb,
                        all,
                        r.x as f32 + (r.width as f32 - lw) / 2.0,
                        b,
                        r.right(),
                        label,
                        st,
                        pal().ink_primary,
                        255,
                    );
                }
                Item::IconButton { rect, icon, size, hover, enabled } => {
                    let r = rect.scale(s);
                    if *hover {
                        rounded_rect(fb, all, r, t::FIELD_RADIUS * s, pal().ink, pal().ghost);
                    }
                    let rgb = if *enabled { pal().ink_primary } else { pal().faint };
                    let sz = size * s;
                    icon.draw(
                        fb,
                        all,
                        (r.x + (r.width - sz) / 2) as f32,
                        (r.y + (r.height - sz) / 2) as f32,
                        sz as f32,
                        rgb,
                        255,
                    );
                }
                Item::Pill { rect, text: label } => {
                    let r = rect.scale(s);
                    rounded_border(fb, all, r, 5 * s, s, pal().quiet, 255);
                    let st = Style::new(Face::Sans, 11.0).scaled(s);
                    let lw = text.measure(label, st);
                    let b = text.centre(r.y, r.height, st);
                    text.draw_style(
                        fb,
                        all,
                        r.x as f32 + (r.width as f32 - lw) / 2.0,
                        b,
                        r.right(),
                        label,
                        st,
                        pal().ink_primary,
                        255,
                    );
                }
                Item::Caps(caps) => {
                    let mono = Style::new(Face::Mono, 11.0).scaled(s);
                    for (r, k) in caps {
                        let c = r.scale(s);
                        rounded_rect(fb, all, c, 5 * s, pal().ink, pal().ghost);
                        rounded_border(fb, all, c, 5 * s, s, pal().ink, pal().hairline);
                        fill(fb, all, Rect::new(c.x + 3 * s, c.bottom() - s, c.width - 6 * s, s), pal().line, 255);
                        let w = text.measure(k, mono);
                        let b = text.centre(c.y, c.height, mono);
                        text.draw_style(
                            fb,
                            all,
                            c.x as f32 + (c.width as f32 - w) / 2.0,
                            b,
                            c.right(),
                            k,
                            mono,
                            pal().quiet,
                            255,
                        );
                    }
                }
            }
        }
    }
}

/// Paint the corners of `r` outside a radius-`rad` rounding in the page's
/// colour (wallpaper tiles are drawn square, then rounded this way).
fn corner_mask(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rad: i32) {
    let b = paint::Box2::from_rect(r);
    for (cx, cy) in [(r.x, r.y), (r.right() - rad, r.y), (r.x, r.bottom() - rad), (r.right() - rad, r.bottom() - rad)] {
        let area = Rect::new(cx, cy, rad, rad).intersect(&clip);
        for y in area.y..area.bottom() {
            let row = fb.row_mut(y as u32);
            for x in area.x..area.right() {
                let cov = paint::rounded_coverage(&b, Radii::all(rad as f32), x, y);
                let a = ((1.0 - cov) * 255.0 + 0.5) as u32;
                if a > 0 {
                    row[x as usize] = paint::mix(row[x as usize], pal().s1, a);
                }
            }
        }
    }
}

/// A window onto part of a framebuffer, for drawing a wallpaper tile with
/// the wallpaper's own code.
struct TileFb<'a> {
    fb: &'a mut dyn Framebuffer,
    origin: (i32, i32),
    size: (i32, i32),
}

impl Framebuffer for TileFb<'_> {
    fn width(&self) -> u32 {
        self.size.0 as u32
    }

    fn height(&self) -> u32 {
        self.size.1 as u32
    }

    fn row_mut(&mut self, y: u32) -> &mut [u32] {
        let (ox, oy) = self.origin;
        let w = self.size.0 as usize;
        let row = self.fb.row_mut((oy + y as i32) as u32);
        &mut row[ox as usize..ox as usize + w]
    }
}
