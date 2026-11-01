use super::frame::{FrameLayout, PADDING};
use super::*;
use alloc::string::String;
use alloc::vec::Vec;

const BG: u32 = Theme::NEUTRAL.background;

/// A window of `w` × `h` pixels of `pixel` at `scale`, undecorated unless
/// asked, at (`x`, `y`).
#[allow(clippy::too_many_arguments)]
fn window(scene: &mut Scene, x: i32, y: i32, w: u32, h: u32, scale: u32, pixel: u32, decorated: bool) -> WindowId {
    let id = scene.add_window(0, decorated);
    scene.resize_content(id, w, h, scale);
    scene.content_mut(id).unwrap().pixels.fill(pixel);
    scene.set_position(id, x, y);
    id
}

/// The picture as one character per pixel, from `key`; `?` for colours not
/// in it.
fn picture(fb: &MemoryFramebuffer, key: &[(char, u32)]) -> Vec<String> {
    (0..fb.height())
        .map(|y| {
            (0..fb.width())
                .map(|x| {
                    let p = fb.pixel(x, y).unwrap();
                    key.iter().find(|(_, c)| *c == p).map_or('?', |(k, _)| *k)
                })
                .collect()
        })
        .collect()
}

#[test]
fn rectangles() {
    let a = Rect::new(0, 0, 10, 10);
    let b = Rect::new(5, 5, 10, 10);
    assert_eq!(a.intersect(&b), Rect::new(5, 5, 5, 5));
    assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
    assert!(a.intersect(&Rect::new(20, 20, 1, 1)).is_empty());
    assert_eq!(Rect::new(1, 1, 3, 3).scale_down_out(2), Rect::new(0, 0, 2, 2));
    assert_eq!(Rect::new(1, 2, 3, 4).scale(2), Rect::new(2, 4, 6, 8));
    assert_eq!(Rect::new(0, 0, -3, 2), Rect::new(0, 0, 0, 2));
    assert!(a.contains(9, 0) && !a.contains(10, 0));
}

#[test]
fn premultiplied_blending() {
    // Opaque replaces, fully transparent keeps.
    assert_eq!(over(0xff11_2233, 0xff44_5566), 0xff11_2233);
    assert_eq!(over(0x0000_0000, 0xff44_5566), 0xff44_5566);
    // Half-transparent black halves (rounded): 0x20 * 127 / 255 = 16.
    assert_eq!(over(0x8000_0000, 0xff20_2020), 0xff10_1010);
    // Premultiplied half red over blue.
    assert_eq!(over(0x8080_0000, 0xff00_00ff), 0xff80_007f);
    assert_eq!(with_alpha(0xff_ff_ff, 0x80), 0x8080_8080);
    assert_eq!(with_alpha(0x12_34_56, 255), 0xff12_3456);
}

/// A scene drawn pixel for pixel: two undecorated windows overlapping, the
/// upper one half transparent.
#[test]
fn composes_a_small_scene() {
    let mut scene = Scene::new(8, 6, 1);
    window(&mut scene, 1, 1, 4, 3, 1, 0xffff_0000, false);
    window(&mut scene, 3, 2, 4, 3, 1, 0x8000_0080, false);
    let mut fb = MemoryFramebuffer::new(8, 6);
    let drawn = scene.render(&mut fb);
    assert_eq!(drawn, [Rect::new(0, 0, 8, 6)]);
    // Blue at half alpha over red: (0xff * 127 + 127) / 255 = 127, plus
    // 0x80 blue; over the background 0x20 * 127 / 255 = 16 each.
    let key = [('.', BG), ('R', 0xff_00_00), ('M', 0x7f_00_80), ('b', 0x10_10_90)];
    assert_eq!(picture(&fb, &key), ["........", ".RRRR...", ".RRMMbb.", ".RRMMbb.", "...bbbb.", "........"]);
    // Nothing changed: nothing drawn.
    assert!(scene.render(&mut fb).is_empty());
}

#[test]
fn only_damage_is_drawn_and_presented() {
    let mut scene = Scene::new(16, 16, 1);
    let id = window(&mut scene, 4, 4, 8, 8, 1, 0xff00_ff00, false);
    let mut fb = MemoryFramebuffer::new(16, 16);
    scene.render(&mut fb);
    fb.take_presented();
    // Two pixels change; the client damages just them.
    let c = scene.content_mut(id).unwrap();
    c.pixels[0] = 0xff00_00ff;
    c.pixels[1] = 0xff00_00ff;
    scene.damage_content(id, Rect::new(0, 0, 2, 1));
    let drawn = scene.render(&mut fb);
    assert_eq!(drawn, [Rect::new(4, 4, 2, 1)]);
    assert_eq!(fb.take_presented(), drawn);
    assert_eq!((fb.pixel(4, 4), fb.pixel(5, 4), fb.pixel(6, 4)), (Some(0xff), Some(0xff), Some(0xff00)));
    // Moving a window damages where it was and where it is.
    scene.set_position(id, 0, 0);
    let drawn = scene.render(&mut fb);
    assert_eq!(drawn.iter().fold(Rect::default(), |a, r| a.union(r)), Rect::new(0, 0, 12, 12));
    assert_eq!((fb.pixel(0, 0), fb.pixel(11, 11)), (Some(0xff), Some(BG)));
}

#[test]
fn hidpi_scales_logical_coordinates_and_buffers() {
    // Output scale 2: a buffer at scale 1 is doubled, one at scale 2 is
    // copied pixel for pixel; positions are logical.
    let mut scene = Scene::new(16, 8, 2);
    let lo = window(&mut scene, 0, 0, 2, 2, 1, 0, false);
    let hi = window(&mut scene, 4, 0, 4, 4, 2, 0, false);
    let c = scene.content_mut(lo).unwrap();
    c.pixels.copy_from_slice(&[0xffff_0000, 0xff00_ff00, 0xff00_00ff, 0xffff_ffff]);
    let c = scene.content_mut(hi).unwrap();
    for (i, p) in c.pixels.iter_mut().enumerate() {
        *p = 0xff00_0000 | i as u32;
    }
    let mut fb = MemoryFramebuffer::new(16, 8);
    scene.render(&mut fb);
    let row = |y: u32| (0..8).map(|x| fb.pixel(x, y).unwrap()).collect::<Vec<_>>();
    assert_eq!(row(0)[..4], [0xff0000, 0xff0000, 0x00ff00, 0x00ff00]);
    assert_eq!(row(3)[..4], [0x0000ff, 0x0000ff, 0xffffff, 0xffffff]);
    assert_eq!(fb.pixel(4, 0), Some(BG));
    assert_eq!((8..12).map(|x| fb.pixel(x, 1).unwrap()).collect::<Vec<_>>(), [4, 5, 6, 7]);
    // Content damage in buffer pixels lands on whole logical pixels.
    scene.damage_content(hi, Rect::new(1, 1, 1, 1));
    assert_eq!(scene.render(&mut fb), [Rect::new(8, 0, 2, 2)]);
}

#[test]
fn draws_the_neutral_frame_around_toplevels() {
    let theme = Theme::NEUTRAL;
    let mut scene = Scene::new(200, 100, 1);
    let id = window(&mut scene, 20, 40, 120, 40, 1, 0xff10_2030, true);
    scene.set_title(id, "Hi");
    scene.set_active(id, true);
    let mut fb = MemoryFramebuffer::new(200, 100);
    scene.render(&mut fb);
    let f = FrameLayout::around(Rect::new(20, 40, 120, 40));
    assert_eq!(f.outer, Rect::new(19, 15, 122, 66));
    assert_eq!(scene.frame_rect(id), Some(f.outer));
    // Border on every side, the title bar above the content.
    for (x, y) in [(19, 15), (140, 15), (19, 80), (140, 80), (19, 50), (140, 50), (60, 15), (60, 80)] {
        assert_eq!(fb.pixel(x, y), Some(theme.border_active), "border at {x},{y}");
    }
    assert_eq!(fb.pixel(18, 50), Some(BG));
    assert_eq!(fb.pixel(100, 17), Some(theme.title_active));
    assert_eq!(fb.pixel(60, 40), Some(0x10_20_30));
    // The title in the 8x16 font, from PADDING, centred in the bar.
    let font = &poly_gfx::SPLEEN_8X16;
    let (tx, ty) = (20 + PADDING, 16 + (TITLE_HEIGHT - 16) / 2);
    for gy in 0..16 {
        for gx in 0..8 {
            let want = if font.glyph('H').is_set(gx, gy) { theme.text_active } else { theme.title_active };
            assert_eq!(fb.pixel(tx as u32 + gx, ty as u32 + gy), Some(want), "glyph pixel {gx},{gy}");
        }
    }
    // The close button: the centre of its cross is drawn, its corners not.
    let c = f.close;
    assert_eq!(c, Rect::new(140 - 4 - CLOSE_SIZE, 20, CLOSE_SIZE, CLOSE_SIZE));
    assert_eq!(fb.pixel((c.x + 7) as u32, (c.y + 7) as u32), Some(theme.text_active));
    assert_eq!(fb.pixel(c.x as u32, c.y as u32), Some(theme.title_active));
    // Inactive: other colours.
    scene.set_active(id, false);
    scene.render(&mut fb);
    assert_eq!(fb.pixel(100, 17), Some(theme.title_inactive));
    assert_eq!(fb.pixel(19, 50), Some(theme.border_inactive));
}

#[test]
fn the_frame_follows_the_window_geometry_and_the_scale() {
    let mut scene = Scene::new(400, 200, 2);
    let id = window(&mut scene, 40, 40, 100, 60, 1, 0xff00_0000, true);
    // A geometry 10 pixels inside the content on each side.
    scene.set_geometry(id, Some(Rect::new(10, 10, 80, 40)));
    let f = FrameLayout::around(Rect::new(50, 50, 80, 40));
    assert_eq!(scene.frame_rect(id), Some(f.outer));
    let mut fb = MemoryFramebuffer::new(400, 200);
    scene.render(&mut fb);
    // At scale 2 the border is two physical pixels wide.
    let theme = Theme::NEUTRAL;
    let (ox, oy) = (f.outer.x * 2, f.outer.y * 2);
    assert_eq!(fb.pixel(ox as u32, (oy + 30) as u32), Some(theme.border_inactive));
    assert_eq!(fb.pixel((ox + 1) as u32, (oy + 30) as u32), Some(theme.border_inactive));
    assert_eq!(fb.pixel((ox - 1) as u32, (oy + 30) as u32), Some(BG));
}

#[test]
fn hit_tests_find_the_topmost_part() {
    let mut scene = Scene::new(300, 200, 1);
    let a = window(&mut scene, 20, 40, 100, 50, 1, 0xff00_0000, true);
    let b = window(&mut scene, 60, 60, 100, 50, 1, 0xff00_0000, true);
    let fb_close = FrameLayout::around(Rect::new(60, 60, 100, 50)).close;
    assert_eq!(scene.hit_test(fb_close.x + 2, fb_close.y + 2), Hit::Close(b));
    assert_eq!(scene.hit_test(70, 45), Hit::TitleBar(b));
    assert_eq!(scene.hit_test(30, 50), Hit::Surface { window: a, x: 10, y: 10 });
    assert_eq!(scene.hit_test(70, 70), Hit::Surface { window: b, x: 10, y: 10 });
    assert_eq!(scene.hit_test(19, 50), Hit::Border(a));
    assert_eq!(scene.hit_test(5, 5), Hit::Desktop);
    scene.raise(a);
    assert_eq!(scene.hit_test(70, 70), Hit::Surface { window: a, x: 50, y: 30 });
}

#[test]
fn raising_and_removing_change_what_is_on_top() {
    let mut scene = Scene::new(10, 10, 1);
    let a = window(&mut scene, 0, 0, 5, 5, 1, 0xffff_0000, false);
    let _b = window(&mut scene, 2, 2, 5, 5, 1, 0xff00_ff00, false);
    let mut fb = MemoryFramebuffer::new(10, 10);
    scene.render(&mut fb);
    assert_eq!(fb.pixel(3, 3), Some(0x00ff00));
    scene.raise(a);
    scene.render(&mut fb);
    assert_eq!(fb.pixel(3, 3), Some(0xff0000));
    scene.remove_window(a);
    scene.render(&mut fb);
    assert_eq!((fb.pixel(3, 3), fb.pixel(0, 0)), (Some(0x00ff00), Some(BG)));
}

#[test]
fn windows_without_content_are_not_drawn() {
    let mut scene = Scene::new(64, 64, 1);
    let id = scene.add_window(7, true);
    scene.set_position(id, 10, 30);
    let mut fb = MemoryFramebuffer::new(64, 64);
    scene.render(&mut fb);
    assert!(fb.pixels().iter().all(|p| p & 0xff_ffff == BG));
    assert_eq!(scene.hit_test(12, 32), Hit::Desktop);
    assert_eq!(scene.window(id).unwrap().owner(), 7);
}

#[test]
fn new_windows_cascade() {
    let mut scene = Scene::new(400, 300, 1);
    assert_eq!(scene.place(0, 0, 100, 100, true), (32 + BORDER, 32 + TITLE_HEIGHT + BORDER));
    assert_eq!(scene.place(0, 0, 100, 100, true), (64 + BORDER, 64 + TITLE_HEIGHT + BORDER));
    // One that would not fit starts over.
    assert_eq!(scene.place(5, 5, 350, 200, false), (32 - 5, 32 - 5));
}

#[test]
fn a_ppm_holds_the_picture() {
    let mut fb = MemoryFramebuffer::new(2, 1);
    fb.row_mut(0).copy_from_slice(&[0xff11_2233, 0x44_5566]);
    assert_eq!(fb.to_ppm(), [b"P6\n2 1\n255\n".as_slice(), &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]].concat());
}

/// A 3 × 2 picture `abc` / `def` (one colour per letter) drawn through
/// every buffer transform: what the client turned, the compositor turns
/// back (wayland.xml: transforms are counter-clockwise, flips first).
#[test]
fn buffer_transforms_are_undone() {
    let colours = [0xffff_0000, 0xff00_ff00, 0xff00_00ff, 0xffff_ff00, 0xff00_ffff, 0xffff_00ff];
    let key = [
        ('.', BG),
        ('a', 0xff0000),
        ('b', 0x00ff00),
        ('c', 0x0000ff),
        ('d', 0xffff00),
        ('e', 0x00ffff),
        ('f', 0xff00ff),
    ];
    let draw = |transform: u32| {
        let mut scene = Scene::new(4, 4, 1);
        let id = scene.add_window(0, false);
        // A buffer holding abc/def with `transform` applied.
        let (w, h) = if transform & 1 != 0 { (2, 3) } else { (3, 2) };
        scene.resize_content(id, w, h, 1);
        scene.set_view(id, View { transform, ..View::default() });
        let c = scene.content_mut(id).unwrap();
        for v in 0..2 {
            for u in 0..3 {
                let (bx, by) = buffer_point(transform, u, v, 3, 2);
                c.pixels[(by * w as i32 + bx) as usize] = colours[(v * 3 + u) as usize];
            }
        }
        let mut fb = MemoryFramebuffer::new(4, 4);
        scene.render(&mut fb);
        picture(&fb, &key)
    };
    for t in 0..8 {
        assert_eq!(draw(t), ["abc.", "def.", "....", "...."], "transform {t}");
    }
    // The 90° buffer as the client draws it: the picture turned
    // counter-clockwise (its right column on top, its left at the bottom).
    assert_eq!(buffer_point(1, 2, 0, 3, 2), (0, 0));
    assert_eq!(buffer_point(1, 0, 0, 3, 2), (0, 2));
    // Flipped: mirrored around the vertical axis.
    assert_eq!(buffer_point(4, 0, 0, 3, 2), (2, 0));
}

/// A viewport crops a source rectangle and scales it to its destination.
#[test]
fn viewports_crop_and_scale() {
    let mut scene = Scene::new(8, 4, 1);
    let id = scene.add_window(0, false);
    scene.resize_content(id, 4, 2, 1);
    let c = scene.content_mut(id).unwrap();
    c.pixels.copy_from_slice(&[
        0xffff_0000,
        0xff00_ff00,
        0xff00_00ff,
        0xffff_ffff,
        0xff00_0000,
        0xff00_ff00,
        0xff00_00ff,
        0xffff_ffff,
    ]);
    // The middle two columns of the top row, shown 4 × 2.
    scene.set_view(id, View { source: Some((256, 0, 512, 256)), destination: Some((4, 2)), ..View::default() });
    assert_eq!(scene.window(id).unwrap().surface_rect(), Rect::new(0, 0, 4, 2));
    let mut fb = MemoryFramebuffer::new(8, 4);
    scene.render(&mut fb);
    let key = [('.', BG), ('g', 0x00ff00), ('b', 0x0000ff)];
    assert_eq!(picture(&fb, &key), ["ggbb....", "ggbb....", "........", "........"]);
    // Only a destination: the whole buffer scaled down.
    scene.set_view(id, View { destination: Some((2, 1)), ..View::default() });
    scene.render(&mut fb);
    let key = [('.', BG), ('g', 0x00ff00), ('w', 0xffffff)];
    assert_eq!(picture(&fb, &key), ["gw......", "........", "........", "........"]);
}

/// A window part of another (a subsurface, a popup) moves and is raised
/// with it, and can be stacked below it.
#[test]
fn families_move_and_stack_together() {
    let mut scene = Scene::new(20, 10, 1);
    let a = window(&mut scene, 0, 0, 6, 6, 1, 0xffff_0000, false);
    let b = window(&mut scene, 4, 2, 6, 6, 1, 0xff00_ff00, false);
    let child = window(&mut scene, 2, 2, 2, 2, 1, 0xff00_00ff, false);
    scene.set_parent(child, Some(a));
    scene.place_above(child, a);
    assert_eq!(scene.family(a), [a, child]);
    assert_eq!(scene.root(child), a);
    // Raising the parent brings the child along.
    scene.raise(a);
    let order: Vec<WindowId> = scene.windows().iter().map(|w| w.id()).collect();
    assert_eq!(order, [b, a, child]);
    scene.set_position(a, 10, 0);
    assert_eq!(scene.window(child).unwrap().position(), (12, 2));
    // Below its parent: hidden behind it.
    scene.restack_below(child, a);
    let order: Vec<WindowId> = scene.windows().iter().map(|w| w.id()).collect();
    assert_eq!(order, [b, child, a]);
    let mut fb = MemoryFramebuffer::new(20, 10);
    scene.render(&mut fb);
    assert_eq!(fb.pixel(12, 2), Some(0xff0000));
    scene.restack_above(child, a);
    scene.render(&mut fb);
    assert_eq!(fb.pixel(12, 2), Some(0x0000ff));
    // No cycles.
    scene.set_parent(a, Some(child));
    assert_eq!(scene.window(a).unwrap().parent(), None);
    scene.remove_window(a);
    assert_eq!(scene.window(child).unwrap().parent(), None);
}

/// A client's cursor image replaces the arrow, with its hot spot.
#[test]
fn cursor_images_and_hidden_cursors() {
    let mut scene = Scene::new(10, 10, 1);
    scene.set_cursor(Some((5, 5)));
    let content = Content { width: 2, height: 2, scale: 1, pixels: alloc::vec![0xff12_3456; 4], ..Content::default() };
    scene.set_cursor_look(CursorLook::Image { content, hx: 1, hy: 1 });
    let mut fb = MemoryFramebuffer::new(10, 10);
    scene.render(&mut fb);
    assert_eq!((fb.pixel(4, 4), fb.pixel(5, 5), fb.pixel(6, 6)), (Some(0x123456), Some(0x123456), Some(BG)));
    scene.set_cursor_look(CursorLook::Hidden);
    scene.render(&mut fb);
    assert_eq!(fb.pixel(4, 4), Some(BG));
    assert_eq!(scene.cursor_rect(5, 5), Rect::default());
}

/// The picture behind a window: what is under it, without it, kept up to
/// date with damage as things below it change, and left alone by changes
/// elsewhere (ADR 0116; Liquid Glass samples it later).
#[test]
fn backdrops_hold_what_is_behind_a_window() {
    let mut scene = Scene::new(20, 10, 1);
    let below = window(&mut scene, 0, 0, 8, 8, 1, 0xffff_0000, false);
    let far = window(&mut scene, 14, 0, 4, 4, 1, 0xff00_00ff, false);
    let glass = window(&mut scene, 4, 2, 6, 6, 1, 0x8000_0000, false);
    scene.set_backdrop(glass, true);
    let mut fb = MemoryFramebuffer::new(20, 10);
    scene.render(&mut fb);
    let b = scene.backdrop(glass).unwrap();
    assert_eq!(b.rect(), Rect::new(4, 2, 6, 6));
    // Red where the window below is, the background beside it; not the
    // half-transparent black of the window itself.
    assert_eq!((b.pixel(5, 3), b.pixel(9, 3)), (Some(0xff0000), Some(BG)));
    assert_ne!(fb.pixel(5, 3), Some(0xff0000), "the screen shows the glass over it");
    assert_eq!(scene.take_backdrop_damage(glass), [Rect::new(4, 2, 6, 6)]);
    // A change far away: no backdrop damage.
    scene.content_mut(far).unwrap().pixels.fill(0xff00_ff00);
    scene.damage_content(far, Rect::new(0, 0, 4, 4));
    scene.render(&mut fb);
    assert!(scene.take_backdrop_damage(glass).is_empty());
    // A change below: that part is drawn again.
    scene.content_mut(below).unwrap().pixels.fill(0xff00_ff00);
    scene.damage_content(below, Rect::new(5, 3, 2, 2));
    scene.render(&mut fb);
    assert_eq!(scene.take_backdrop_damage(glass), [Rect::new(5, 3, 2, 2)]);
    assert_eq!(scene.backdrop(glass).unwrap().pixel(5, 3), Some(0x00ff00));
    // Moving the window takes the picture along.
    scene.set_position(glass, 10, 2);
    scene.render(&mut fb);
    let b = scene.backdrop(glass).unwrap();
    assert_eq!(b.rect(), Rect::new(10, 2, 6, 6));
    assert_eq!((b.pixel(15, 3), b.pixel(10, 9)), (Some(0x00ff00), None));
    // HiDPI: physical pixels.
    let mut scene = Scene::new(20, 20, 2);
    window(&mut scene, 0, 0, 10, 10, 1, 0xffff_0000, false);
    let glass = window(&mut scene, 2, 2, 4, 4, 1, 0x0000_0000, false);
    scene.set_backdrop(glass, true);
    let mut fb = MemoryFramebuffer::new(20, 20);
    scene.render(&mut fb);
    assert_eq!(scene.backdrop(glass).unwrap().rect(), Rect::new(4, 4, 8, 8));
    assert_eq!(scene.backdrop(glass).unwrap().pixel(5, 5), Some(0xff0000));
    scene.set_backdrop(glass, false);
    assert!(scene.backdrop(glass).is_none());
}
