//! The shell drawn with DejaVu Sans, as on Poly: checks that text lands
//! where the layout puts it, and with `POLY_SHELL_PNG_DIR` set writes the
//! pictures as PNGs to look at. Skips without the fetched fonts
//! (`POLY_FONT_DIR`, as `poly-font`'s DejaVu tests).

use poly_font::Font;
use poly_shell::draw::Text;
use poly_shell::{Key, Shell, apps};
use poly_wayland::compose::{Framebuffer, Layer, MemoryFramebuffer, Rect};

fn dejavu() -> Option<&'static [u8]> {
    let dir = std::env::var_os("POLY_FONT_DIR")?;
    let data = std::fs::read(std::path::Path::new(&dir).join("DejaVuSans.ttf")).ok()?;
    Some(Box::leak(data.into_boxed_slice()))
}

fn save(fb: &MemoryFramebuffer, name: &str) {
    let Some(dir) = std::env::var_os("POLY_SHELL_PNG_DIR") else { return };
    let rgb: Vec<u8> = fb.pixels().iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    let png = poly_font::png::encode(fb.width(), fb.height(), 3, &rgb);
    std::fs::write(std::path::Path::new(&dir).join(name), png).unwrap();
}

#[test]
fn the_shell_in_dejavu() {
    let Some(data) = dejavu() else {
        eprintln!("skipped: no DejaVu fonts (POLY_FONT_DIR)");
        return;
    };
    let font = Font::parse(data).unwrap();
    let text = Text::new(Some(font));
    assert!(text.has_outline_font());
    let mut shell = Shell::new(1280, 800, apps::catalog(Vec::new()), text);
    shell.set_time(1_793_491_200 + 21 * 3600 + 41 * 60, 0);
    let all = Rect::new(0, 0, 1280, 800);
    let mut fb = MemoryFramebuffer::new(1280, 800);
    shell.below().draw(&mut fb, all, 1);
    shell.above().draw(&mut fb, all, 1);
    save(&fb, "shell-empty.png");
    // The clock is drawn: light pixels in the ridge's right part.
    assert!((1100..1276).any(|x| (4..20).any(|y| fb.pixel(x, y).is_some_and(|p| p & 0xff > 0xc0))));
    // The placeholder is drawn in the search row, grey.
    assert!((330..420).any(|x| (310..330).any(|y| fb.pixel(x, y).is_some_and(|p| p != 0x20_20_20))));
    for c in "w".chars() {
        shell.key(Key::Char(c));
    }
    shell.below().draw(&mut fb, all, 1);
    shell.above().draw(&mut fb, all, 1);
    save(&fb, "shell-typed.png");
    // HiDPI: the same at scale 2.
    let mut big = MemoryFramebuffer::new(2560, 1600);
    shell.below().draw(&mut big, Rect::new(0, 0, 2560, 1600), 2);
    shell.above().draw(&mut big, Rect::new(0, 0, 2560, 1600), 2);
    save(&big, "shell-scale2.png");
    assert_eq!(big.pixel(1280, 40), Some(0x20_20_20), "the ridge is 48 physical rows at scale 2");

    // A framed window with its title in DejaVu, over the shell.
    let mut scene = poly_wayland::compose::Scene::new(1280, 800, 1);
    scene.set_text(Box::new(poly_shell::draw::TitleFont::new(font, 13.0, 16)));
    let w = scene.add_window(1, true);
    scene.resize_content(w, 360, 240, 1);
    if let Some(c) = scene.content_mut(w) {
        c.pixels.fill(0xff2f_7d6d);
    }
    scene.set_position(w, 460, 304);
    scene.set_title(w, "Poly Demo");
    scene.set_active(w, true);
    shell.hide_launcher();
    let mut fb = MemoryFramebuffer::new(1280, 800);
    scene.render_with(&mut fb, Some(&shell.below()), Some(&shell.above()));
    save(&fb, "shell-window.png");
    // Title text: light pixels in the title bar's left part.
    let lit = (468..560).any(|x| (284..300).any(|y| fb.pixel(x, y).is_some_and(|p| p & 0xff > 0xb0)));
    assert!(lit, "the title is drawn");
}
