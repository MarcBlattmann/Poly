//! Drawing one damaged rectangle: the desktop, then every window bottom to
//! top — its frame, then its content over it — clipped to the rectangle.

use super::busy;
use super::frame::{CORNER_RADIUS, Glyph, PhysFrame, glyph_covers};
use super::paint::{self, Radii, ShadowCache};
use super::scene::{Backdrop, Content, CursorLook, Placed, Scene, Window, WindowId};
use super::{CURSOR_SIZE, Framebuffer, Layer, Rect, SCALE_UNIT, Scale, over, with_alpha};
use alloc::collections::BTreeMap;

const OPAQUE: u32 = 0xff00_0000;

/// Draw the physical rectangle `clip` (inside the framebuffer) of `scene`:
/// the background (or the `below` layer), the windows, the `above` layer,
/// the cursor. Just before a window whose backdrop is kept is drawn, what
/// the framebuffer holds under it is its backdrop.
pub(super) fn draw(
    scene: &Scene,
    fb: &mut dyn Framebuffer,
    clip: Rect,
    below: Option<&dyn Layer>,
    above: Option<&dyn Layer>,
    backdrops: &mut BTreeMap<WindowId, Backdrop>,
    shadows: &mut ShadowCache,
) {
    let s = scene.output().2;
    match below {
        Some(layer) => layer.draw(fb, clip, s),
        None => fill(fb, clip, clip, scene.theme().background),
    }
    for w in scene.windows().iter().filter(|w| w.is_visible()) {
        if scene.physical_extent(scene.drawn_extent(w)).intersect(&clip).is_empty() {
            continue;
        }
        if let Some(b) = backdrops.get_mut(&w.id()) {
            b.capture(fb, clip);
        }
        let placed = w.placed(s);
        if let Some(f) = &placed.frame {
            draw_shadow(scene, shadows, fb, clip, w, f, s);
            draw_frame(scene, fb, clip, w, f, s);
        }
        draw_content(fb, clip, w, &placed, s);
    }
    if let Some(layer) = above {
        layer.draw(fb, clip, s);
    }
    if let Some((x, y)) = scene.cursor() {
        match scene.cursor_look() {
            CursorLook::Arrow => draw_cursor(fb, clip, x, y, s),
            CursorLook::Image { content, hx, hy } => {
                let (w, h) = content.logical_size();
                let dest = Rect::new(s.px(x - hx), s.px(y - hy), s.px(w), s.px(h));
                draw_image(fb, clip, content, dest, dest, s);
            }
            CursorLook::Busy(phase) => {
                draw_cursor(fb, clip, x, y, s);
                // Dark or light as the frame's title bar is.
                let dark = scene.theme().title_active & 0xff < 0x80;
                busy::draw(fb, clip, x, y, s, *phase, dark);
            }
            CursorLook::Hidden => {}
        }
    }
}

/// The arrow: `X` outline (black), `.` fill (white), hot spot top left.
const ARROW: [&[u8; CURSOR_SIZE.0 as usize]; CURSOR_SIZE.1 as usize] = [
    b"X           ",
    b"XX          ",
    b"X.X         ",
    b"X..X        ",
    b"X...X       ",
    b"X....X      ",
    b"X.....X     ",
    b"X......X    ",
    b"X.......X   ",
    b"X........X  ",
    b"X.........X ",
    b"X......XXXXX",
    b"X...X..X    ",
    b"X..XX..X    ",
    b"X.X  X..X   ",
    b"XX   X..X   ",
    b"X     X..X  ",
    b"      X..X  ",
    b"       XX   ",
];

/// The software cursor with its hot spot at logical (`x`, `y`): the arrow's
/// pixels are as large as the scale makes a logical pixel.
pub(super) fn draw_cursor(fb: &mut dyn Framebuffer, clip: Rect, x: i32, y: i32, s: Scale) {
    let (ox, oy) = (s.px(x), s.px(y));
    let area = Rect::new(ox, oy, s.px(CURSOR_SIZE.0) + 1, s.px(CURSOR_SIZE.1) + 1).intersect(&clip);
    for py in area.y..area.bottom() {
        let row = fb.row_mut(py as u32);
        let line = ARROW[(s.to_logical(py - oy) as usize).min(ARROW.len() - 1)];
        for px in area.x..area.right() {
            match line[(s.to_logical(px - ox) as usize).min(line.len() - 1)] {
                b'X' => row[px as usize] = OPAQUE,
                b'.' => row[px as usize] = OPAQUE | 0x00ff_ffff,
                _ => {}
            }
        }
    }
}

/// Fill the part of `r` inside `clip` with the opaque colour `rgb`.
fn fill(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rgb: u32) {
    let r = r.intersect(&clip);
    if r.is_empty() {
        return;
    }
    for y in r.y..r.bottom() {
        fb.row_mut(y as u32)[r.x as usize..r.right() as usize].fill(rgb | OPAQUE);
    }
}

fn draw_frame(scene: &Scene, fb: &mut dyn Framebuffer, clip: Rect, w: &Window, f: &PhysFrame, s: Scale) {
    let theme = scene.theme();
    let (title_bg, text, border) = if w.is_active() {
        (theme.title_active, theme.text_active, theme.border_active)
    } else {
        (theme.title_inactive, theme.text_inactive, theme.border_inactive)
    };
    // The top: border and title bar with rounded corners; then the border's
    // sides and bottom along the content.
    let r = (CORNER_RADIUS * s) as f32;
    paint::fill_rounded(fb, clip, f.top, Radii::top(r), border, 255);
    paint::fill_rounded(fb, clip, f.title, Radii::top(r - f.border as f32), title_bg, 255);
    for strip in f.strips {
        let from = strip.y.max(f.top.bottom());
        fill(fb, clip, Rect::new(strip.x, from, strip.width, strip.bottom() - from), border);
    }
    draw_title(scene, fb, clip, f, w.title(), text, s);
    // The buttons: glyphs in the title text's colour, lines as thick as the
    // border's logical pixel.
    let line = s.line(1);
    let middle = if w.is_maximized() { Glyph::Restore } else { Glyph::Maximize };
    for (b, glyph) in [(f.close, Glyph::Close), (f.maximize, middle), (f.minimize, Glyph::Minimize)] {
        let area = b.intersect(&clip);
        for y in area.y..area.bottom() {
            let row = fb.row_mut(y as u32);
            for x in area.x..area.right() {
                if glyph_covers(glyph, x - b.x, y - b.y, b.width, line) {
                    row[x as usize] = text | OPAQUE;
                }
            }
        }
    }
}

/// The frame's soft shadow (the theme's, for an active or inactive
/// window), not under the frame itself and not over the rows a shell
/// keeps for itself (its strip is under the windows).
fn draw_shadow(
    scene: &Scene,
    shadows: &mut ShadowCache,
    fb: &mut dyn Framebuffer,
    clip: Rect,
    w: &Window,
    f: &PhysFrame,
    s: Scale,
) {
    let Some(shadow) = scene.theme().shadow(w.is_active()) else { return };
    let top = s.px(scene.reserved_top());
    let clip = clip.intersect(&Rect::new(clip.x, top, clip.width, (clip.bottom() - top).max(0)));
    let outer = f.outer;
    let r = s.px(CORNER_RADIUS);
    let occluder = Rect::new(outer.x, outer.y + r, outer.width, outer.height - r);
    shadow.scaled(s).draw_cached(shadows, fb, clip, outer, Radii::top(r as f32), occluder);
}

/// The title, from the left of the text area, vertically centred, cut off
/// where the area ends, drawn at the scale's own size.
fn draw_title(scene: &Scene, fb: &mut dyn Framebuffer, clip: Rect, f: &PhysFrame, title: &str, rgb: u32, s: Scale) {
    let font = scene.text();
    let h = font.line_height_at(s) as i32;
    let area = f.text.intersect(&clip);
    if area.is_empty() {
        return;
    }
    let top = f.text.y + (f.text.height - h) / 2;
    let mut pen = f.text.x;
    for c in title.chars() {
        let adv = font.advance_at(c, s) as i32;
        if pen >= f.text.right() {
            break;
        }
        let cell = Rect::new(pen, top, adv, h).intersect(&area);
        for y in cell.y..cell.bottom() {
            let gy = (y - top) as u32;
            let row = fb.row_mut(y as u32);
            for x in cell.x..cell.right() {
                let cov = font.coverage_at(c, (x - pen) as u32, gy, s);
                if cov != 0 {
                    let px = &mut row[x as usize];
                    *px = over(with_alpha(rgb, cov), *px) | OPAQUE;
                }
            }
        }
        pen += adv;
    }
}

/// The content, scaled from its buffer to the output's pixels, blended over
/// what is below. In a frame, only its visible bounds are drawn (xdg-shell
/// lets compositors clip to the window geometry): what a client draws
/// outside them would be under the frame.
fn draw_content(fb: &mut dyn Framebuffer, clip: Rect, w: &Window, placed: &Placed, s: Scale) {
    let shown = if w.is_decorated() { placed.bounds } else { placed.surface };
    draw_image(fb, clip, w.content(), placed.surface, shown, s);
}

/// Content `c` shown over the physical rectangle `dest`, only the part in
/// `shown` and `clip`, blended over what is below: pixel for pixel when its
/// buffer is the output's pixels (no transform or viewport, a buffer scale
/// the output's), by nearest neighbour when the scale is a whole number
/// but another, and smoothed (bilinear) for a client that draws at another
/// fraction (a buffer scale of 2 on a 150 % output is shrunk, ADR 0179).
fn draw_image(fb: &mut dyn Framebuffer, clip: Rect, c: &Content, dest: Rect, shown: Rect, s: Scale) {
    let area = shown.intersect(&clip).intersect(&dest);
    if area.is_empty() {
        return;
    }
    if !c.view.is_identity() {
        for y in area.y..area.bottom() {
            let row = fb.row_mut(y as u32);
            for x in area.x..area.right() {
                let p = c.sample(x - dest.x, y - dest.y, dest.width, dest.height);
                let dst = &mut row[x as usize];
                *dst = over(p, *dst) | OPAQUE;
            }
        }
        return;
    }
    let bs = c.scale.max(1) as i32;
    let width = c.width as usize;
    if s.as_120ths() == bs as u32 * SCALE_UNIT {
        for y in area.y..area.bottom() {
            let sy = (y - dest.y) as usize;
            let x0 = (area.x - dest.x) as usize;
            let src = &c.pixels[sy * width + x0..sy * width + x0 + area.width as usize];
            let row = fb.row_mut(y as u32);
            for (dst, &p) in row[area.x as usize..area.right() as usize].iter_mut().zip(src) {
                *dst = over(p, *dst) | OPAQUE;
            }
        }
    } else if s.is_integer() {
        let si = s.ceil() as i32;
        for y in area.y..area.bottom() {
            let sy = ((y - dest.y) * bs / si) as usize;
            let src = &c.pixels[sy * width..(sy + 1) * width];
            let row = fb.row_mut(y as u32);
            for x in area.x..area.right() {
                let p = src[((x - dest.x) * bs / si) as usize];
                let dst = &mut row[x as usize];
                *dst = over(p, *dst) | OPAQUE;
            }
        }
    } else {
        draw_smooth(fb, c, dest, area, bs, s);
    }
}

/// [`draw_image`] for a buffer that is not a whole number of buffer pixels
/// per output pixel: each output pixel takes the four buffer pixels around
/// its centre, weighted by distance (premultiplied channels, so edges stay
/// clean).
fn draw_smooth(fb: &mut dyn Framebuffer, c: &Content, dest: Rect, area: Rect, bs: i32, s: Scale) {
    let (w, h) = (i64::from(c.width), i64::from(c.height));
    if w == 0 || h == 0 {
        return;
    }
    // Buffer pixels per output pixel, 16.16.
    let step = ((i64::from(bs) * i64::from(SCALE_UNIT)) << 16) / i64::from(s.as_120ths());
    // The buffer coordinate of an output pixel's centre, less half a buffer
    // pixel, 16.16: the position among the buffer pixels' centres.
    let at = |o: i32| (i64::from(o) * step + step / 2 - 0x8000).max(0);
    let pick = |f: i64, len: i64| {
        let i = (f >> 16).min(len - 1);
        (i, (i + 1).min(len - 1), ((f & 0xffff) >> 8) as u32)
    };
    for y in area.y..area.bottom() {
        let (y0, y1, wy) = pick(at(y - dest.y), h);
        let line = |i: i64| &c.pixels[(i * w) as usize..((i + 1) * w) as usize];
        let (r0, r1) = (line(y0), line(y1));
        let row = fb.row_mut(y as u32);
        for x in area.x..area.right() {
            let (x0, x1, wx) = pick(at(x - dest.x), w);
            let (x0, x1) = (x0 as usize, x1 as usize);
            let p = blend4(r0[x0], r0[x1], r1[x0], r1[x1], wx, wy);
            let dst = &mut row[x as usize];
            *dst = over(p, *dst) | OPAQUE;
        }
    }
}

/// The bilinear mix of four premultiplied pixels (top left, top right,
/// bottom left, bottom right); `wx` and `wy` are 0 to 255, the weights of
/// the right and bottom ones.
fn blend4(p00: u32, p10: u32, p01: u32, p11: u32, wx: u32, wy: u32) -> u32 {
    let lerp = |a: u32, b: u32, w: u32| (a * (256 - w) + b * w) >> 8;
    let channel = |shift: u32| {
        let ch = |p: u32| (p >> shift) & 0xff;
        let top = lerp(ch(p00), ch(p10), wx);
        let bottom = lerp(ch(p01), ch(p11), wx);
        lerp(top, bottom, wy) << shift
    };
    channel(24) | channel(16) | channel(8) | channel(0)
}
