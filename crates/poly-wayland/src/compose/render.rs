//! Drawing one damaged rectangle: the desktop, then every window bottom to
//! top — its frame, then its content over it — clipped to the rectangle.

use super::frame::{FrameLayout, cross_covers};
use super::scene::{Scene, Window};
use super::{CURSOR_SIZE, Framebuffer, Layer, Rect, over, with_alpha};

const OPAQUE: u32 = 0xff00_0000;

/// Draw the physical rectangle `clip` (inside the framebuffer) of `scene`:
/// the background (or the `below` layer), the windows, the `above` layer,
/// the cursor.
pub(super) fn draw(
    scene: &Scene,
    fb: &mut dyn Framebuffer,
    clip: Rect,
    below: Option<&dyn Layer>,
    above: Option<&dyn Layer>,
) {
    let s = scene.output().2 as i32;
    match below {
        Some(layer) => layer.draw(fb, clip, s),
        None => fill(fb, clip, clip, scene.theme().background),
    }
    for w in scene.windows().iter().filter(|w| w.is_visible()) {
        if w.extent().scale(s).intersect(&clip).is_empty() {
            continue;
        }
        if let Some(f) = w.frame() {
            draw_frame(scene, fb, clip, w, &f, s);
        }
        draw_content(fb, clip, w, s);
    }
    if let Some(layer) = above {
        layer.draw(fb, clip, s);
    }
    if let Some((x, y)) = scene.cursor() {
        draw_cursor(fb, clip, x, y, s);
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

/// The software cursor with its hot spot at logical (`x`, `y`).
fn draw_cursor(fb: &mut dyn Framebuffer, clip: Rect, x: i32, y: i32, s: i32) {
    let area = Rect::new(x, y, CURSOR_SIZE.0, CURSOR_SIZE.1).scale(s).intersect(&clip);
    for py in area.y..area.bottom() {
        let row = fb.row_mut(py as u32);
        let line = ARROW[((py - y * s) / s) as usize];
        for px in area.x..area.right() {
            match line[((px - x * s) / s) as usize] {
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

fn draw_frame(scene: &Scene, fb: &mut dyn Framebuffer, clip: Rect, w: &Window, f: &FrameLayout, s: i32) {
    let theme = scene.theme();
    let (title_bg, text, border) = if w.is_active() {
        (theme.title_active, theme.text_active, theme.border_active)
    } else {
        (theme.title_inactive, theme.text_inactive, theme.border_inactive)
    };
    for strip in f.border() {
        fill(fb, clip, strip.scale(s), border);
    }
    fill(fb, clip, f.title.scale(s), title_bg);
    draw_title(scene, fb, clip, f, w.title(), text, s);
    // The close button: a cross in the title text's colour.
    let close = f.close.scale(s);
    let area = close.intersect(&clip);
    for y in area.y..area.bottom() {
        let row = fb.row_mut(y as u32);
        for x in area.x..area.right() {
            if cross_covers(x - close.x, y - close.y, close.width, s) {
                row[x as usize] = text | OPAQUE;
            }
        }
    }
}

/// The title, from the left of the text area, vertically centred, cut off
/// where the area ends.
fn draw_title(scene: &Scene, fb: &mut dyn Framebuffer, clip: Rect, f: &FrameLayout, title: &str, rgb: u32, s: i32) {
    let font = scene.text();
    let h = font.line_height() as i32;
    let area = f.text.scale(s).intersect(&clip);
    if area.is_empty() {
        return;
    }
    let top = f.text.y + (f.text.height - h) / 2;
    let mut pen = f.text.x;
    for c in title.chars() {
        let adv = font.advance(c) as i32;
        if pen >= f.text.right() {
            break;
        }
        let cell = Rect::new(pen, top, adv, h).scale(s).intersect(&area);
        for y in cell.y..cell.bottom() {
            let gy = (y / s - top) as u32;
            let row = fb.row_mut(y as u32);
            for x in cell.x..cell.right() {
                let cov = font.coverage(c, (x / s - pen) as u32, gy);
                if cov != 0 {
                    let px = &mut row[x as usize];
                    *px = over(with_alpha(rgb, cov), *px) | OPAQUE;
                }
            }
        }
        pen += adv;
    }
}

/// The content, scaled from its buffer scale to the output's (nearest
/// neighbour), blended over what is below. In a frame, only its visible
/// bounds are drawn (xdg-shell lets compositors clip to the window
/// geometry): what a client draws outside them would be under the frame.
fn draw_content(fb: &mut dyn Framebuffer, clip: Rect, w: &Window, s: i32) {
    let c = w.content();
    let dest = w.surface_rect().scale(s);
    let shown = if w.is_decorated() { w.bounds().scale(s) } else { dest };
    let area = shown.intersect(&clip);
    if area.is_empty() {
        return;
    }
    let bs = c.scale.max(1) as i32;
    let width = c.width as usize;
    for y in area.y..area.bottom() {
        let sy = ((y - dest.y) * bs / s) as usize;
        let src = &c.pixels[sy * width..(sy + 1) * width];
        let row = fb.row_mut(y as u32);
        if bs == s {
            let x0 = (area.x - dest.x) as usize;
            let src = &src[x0..x0 + area.width as usize];
            for (dst, &p) in row[area.x as usize..area.right() as usize].iter_mut().zip(src) {
                *dst = over(p, *dst) | OPAQUE;
            }
        } else {
            for x in area.x..area.right() {
                let p = src[((x - dest.x) * bs / s) as usize];
                let dst = &mut row[x as usize];
                *dst = over(p, *dst) | OPAQUE;
            }
        }
    }
}
