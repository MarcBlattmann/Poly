//! The shell drawn with real fonts, as on Poly: Alpine Shell's (Geist,
//! Averia Serif Libre, Geist Mono from `POLY_SHELL_FONT_DIR`, ADR 0127)
//! or DejaVu Sans (`POLY_FONT_DIR`). Checks that text lands where the
//! layout puts it, and with `POLY_SHELL_PNG_DIR` set writes every part —
//! the ridge, each panel, the launcher, the notices, the switcher, a
//! framed window, every settings page — as PNGs, which the comparison with
//! Alpine Shell's screenshots uses. Skips without the fetched fonts.

use poly_dbus::notifications::Notify;
use poly_font::Font;
use poly_shell::draw::{Fonts, Text};
use poly_shell::settings_ui::{Page, SystemInfo};
use poly_shell::switcher::{Entry, Thumb};
use poly_shell::{Key, Panel, Shell, apps};
use poly_wayland::compose::{Framebuffer, Layer, MemoryFramebuffer, Rect, WindowId};
use std::path::Path;

fn leak(path: &Path) -> Option<Font<'static>> {
    let data = std::fs::read(path).ok()?;
    Font::parse(Box::leak(data.into_boxed_slice())).ok()
}

/// Alpine's fonts if fetched, else DejaVu Sans, else nothing.
fn fonts() -> Option<(Fonts, &'static str)> {
    if let Some(dir) = std::env::var_os("POLY_SHELL_FONT_DIR") {
        let d = Path::new(&dir);
        let f = |n: &str| leak(&d.join(n));
        let fonts = Fonts {
            sans: f("Geist-Regular.ttf"),
            medium: f("Geist-Medium.ttf"),
            semibold: f("Geist-SemiBold.ttf"),
            display: f("AveriaSerifLibre-Regular.ttf"),
            mono: f("GeistMono-Regular.ttf"),
            display_bold: f("AveriaSerifLibre-Bold.ttf"),
        };
        if fonts.sans.is_some() {
            return Some((fonts, "Geist"));
        }
    }
    let dir = std::env::var_os("POLY_FONT_DIR")?;
    let sans = leak(&Path::new(&dir).join("DejaVuSans.ttf"))?;
    Some((Fonts { sans: Some(sans), ..Fonts::default() }, "DejaVu"))
}

fn save(fb: &MemoryFramebuffer, name: &str) {
    let Some(dir) = std::env::var_os("POLY_SHELL_PNG_DIR") else { return };
    let rgb: Vec<u8> = fb.pixels().iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    let png = poly_font::png::encode(fb.width(), fb.height(), 3, &rgb);
    std::fs::write(Path::new(&dir).join(name), png).unwrap();
}

fn draw(shell: &Shell, fb: &mut MemoryFramebuffer, s: i32) {
    let all = Rect::new(0, 0, fb.width() as i32, fb.height() as i32);
    shell.below().draw(fb, all, s);
    shell.above().draw(fb, all, s);
}

fn notify(summary: &str, body: &str) -> Notify {
    Notify {
        app_name: "notify-send".into(),
        summary: summary.into(),
        body: body.into(),
        expire_timeout: -1,
        ..Notify::default()
    }
}

fn info() -> SystemInfo {
    SystemInfo {
        os: "Poly 0.0.1".into(),
        kernel: "Linux 6.1.0-poly #1".into(),
        machine: "x86_64".into(),
        cpu: "QEMU Virtual CPU version 2.5+".into(),
        memory_kib: 512 * 1024,
        memory_available_kib: 300 * 1024,
        display: (1280, 800),
        modes: vec![(1280, 800)],
        sound: Some("virtio-sound".into()),
        network: "connected (eth0)".into(),
        session_bus: "unix:path=/run/user/0/bus".into(),
    }
}

/// A shell on a `w` × `h` screen at 20:52 on 1 November 2026, with its
/// status known.
fn shell(w: i32, h: i32, fonts: Fonts) -> Shell {
    let mut s = Shell::new(w, h, apps::catalog(Vec::new()), Text::with_fonts(fonts));
    s.set_animations(false);
    s.set_time(1_793_491_200 + 20 * 3600 + 52 * 60, 0);
    s.set_network(poly_shell::Network::Online("eth0".into()));
    s.set_volume(Some((60, false)));
    s.set_sound_card(Some("virtio-sound".into()));
    s.set_now_ms(1000);
    // Alpine Shell's wallpapers, as `cargo xtask` converts them
    // (`POLY_SHELL_WALLPAPER_DIR`: `target/wallpapers/`, files `<id>-*.pwal`).
    if let Some(dir) = std::env::var_os("POLY_SHELL_WALLPAPER_DIR") {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            for (id, ..) in poly_shell::theme::wallpaper::PRESETS {
                if name.starts_with(&format!("{id}-"))
                    && let Some(p) = std::fs::read(entry.path()).ok().and_then(|d| poly_shell::picture::parse(&d))
                {
                    s.add_wallpaper(id, p);
                }
            }
        }
    }
    s
}

#[test]
fn every_part_of_the_shell() {
    let Some((f, which)) = fonts() else {
        eprintln!("skipped: no fonts (POLY_SHELL_FONT_DIR or POLY_FONT_DIR)");
        return;
    };
    let (w, h) = (1920, 1080);
    let mut s = shell(w, h, f);
    let mut fb = MemoryFramebuffer::new(w as u32, h as u32);
    draw(&s, &mut fb, 1);
    save(&fb, "launcher-empty.png");
    // The clock is drawn: light pixels in the ridge's right part.
    let lit = (w - 160..w - 4).any(|x| (4..20).any(|y| fb.pixel(x as u32, y).is_some_and(|p| p & 0xff > 0xc0)));
    assert!(lit, "{which}: the clock");
    // The placeholder in the launcher hanging from the ridge.
    let grey = (640..900).any(|x| (36..60).any(|y| fb.pixel(x, y).is_some_and(|p| p != 0x20_20_20)));
    assert!(grey, "{which}: the placeholder");
    for c in "s".chars() {
        s.key(Key::Char(c));
    }
    draw(&s, &mut fb, 1);
    save(&fb, "launcher-typed.png");
    s.hide_launcher();
    draw(&s, &mut fb, 1);
    save(&fb, "ridge.png");
    // Each panel, opened from its place on the ridge.
    for (panel, name) in
        [(Panel::Menu, "panel-menu"), (Panel::Control, "panel-control"), (Panel::Calendar, "panel-calendar")]
    {
        s.open_panel(panel);
        draw(&s, &mut fb, 1);
        save(&fb, &format!("{name}.png"));
    }
    s.close_panel();
    // The notices, then the history.
    s.notify(notify("Hello from a test client", "Sent through org.freedesktop.Notifications on the session bus"));
    let mut update = notify("Update ready", "Poly 0.0.2 can be installed now.");
    update.actions = vec![("default".into(), "Open".into()), ("later".into(), "Later".into())];
    s.notify(update);
    draw(&s, &mut fb, 1);
    save(&fb, "notices.png");
    assert_eq!(fb.pixel(w as u32 - 60, 24 + 30), Some(0x20_20_20), "{which}: the notices hang at the top right");
    s.open_panel(Panel::History);
    draw(&s, &mut fb, 1);
    save(&fb, "panel-history.png");
    s.close_panel();
    // The switcher with two windows.
    let entry = |id: u32, title: &str, colour: u32| Entry {
        window: WindowId(id),
        title: title.into(),
        app_id: String::new(),
        minimized: false,
        thumb: Thumb { width: 200, height: 124, pixels: vec![colour; 200 * 124] },
    };
    s.open_switcher(vec![entry(1, "Settings", 0x21_21_20), entry(2, "Poly Demo", 0x2f_7d_6d)], false);
    draw(&s, &mut fb, 1);
    save(&fb, "switcher.png");
    s.switcher_cancel();
    // HiDPI: the same at scale 2.
    let mut small = shell(960, 540, fonts().unwrap().0);
    small.open_panel(Panel::Control);
    let mut big = MemoryFramebuffer::new(1920, 1080);
    draw(&small, &mut big, 2);
    save(&big, "panel-control-scale2.png");
    assert_eq!(big.pixel(960, 40), Some(0x20_20_20), "the ridge is 48 physical rows at scale 2");
}

#[test]
fn a_framed_window_and_the_settings() {
    let Some((f, _)) = fonts() else {
        eprintln!("skipped: no fonts (POLY_SHELL_FONT_DIR or POLY_FONT_DIR)");
        return;
    };
    let title = f.sans;
    let mut s = shell(1920, 1080, f);
    s.hide_launcher();
    // Every settings page, at the window's size and at Alpine's.
    let info = info();
    for (w, h, tag) in
        [(poly_shell::theme::settings::WIDTH, poly_shell::theme::settings::HEIGHT, ""), (1200, 800, "-1200")]
    {
        for page in Page::ALL {
            s.settings_window.page = page;
            let mut fb = MemoryFramebuffer::new(w as u32, h as u32);
            s.draw_settings(&mut fb, w, h, 1, &info, 1_793_491_200);
            let name = page.label().to_lowercase().replace(" & ", "-");
            save(&fb, &format!("settings-{name}{tag}.png"));
        }
    }
    // Keyboard shortcuts being edited (ADR 0128): one changed (its reset
    // shows), one refused (its message in red), one recording.
    {
        use poly_input::Key as K;
        use poly_shell::settings::Change;
        use poly_shell::shortcuts::{Action, Chord};
        s.apply(&Change::Shortcut(Action::Settings, Chord::parse("Ctrl+Alt+O").unwrap()));
        s.settings_window.open_at(Page::Shortcuts);
        s.settings_window.recorder.start(Action::Notifications, 0);
        s.shortcut_key(K::ControlLeft, true, &poly_input::layout::US);
        s.shortcut_key(K::Space, true, &poly_input::layout::US);
        s.shortcut_key(K::Space, false, &poly_input::layout::US);
        s.shortcut_key(K::ControlLeft, false, &poly_input::layout::US);
        s.settings_window.recorder.start(Action::CloseWindow, 0);
        for (w, h, tag) in
            [(poly_shell::theme::settings::WIDTH, poly_shell::theme::settings::HEIGHT, ""), (1200, 800, "-1200")]
        {
            let mut fb = MemoryFramebuffer::new(w as u32, h as u32);
            s.draw_settings(&mut fb, w, h, 1, &info, 1_793_491_200);
            save(&fb, &format!("settings-shortcuts-editing{tag}.png"));
        }
        s.apply(&Change::ResetShortcuts);
        s.settings_window.recorder.stop();
    }
    // A framed window — the settings — over the shell.
    let mut scene = poly_wayland::compose::Scene::new(1920, 1080, 1);
    if let Some(font) = title {
        scene.set_text(Box::new(poly_shell::draw::TitleFont::new(font, 12.0, 16)));
    }
    let (ww, wh) = (1280, 820);
    let win = scene.add_window(1, true);
    scene.resize_content(win, ww as u32, wh as u32, 1);
    s.settings_window.page = Page::Sound;
    let mut content = MemoryFramebuffer::new(ww as u32, wh as u32);
    s.draw_settings(&mut content, ww, wh, 1, &info, 1_793_491_200);
    if let Some(c) = scene.content_mut(win) {
        c.pixels.copy_from_slice(content.pixels());
    }
    scene.set_position(win, 320, 130);
    scene.set_title(win, "Settings");
    scene.set_active(win, true);
    let mut fb = MemoryFramebuffer::new(1920, 1080);
    scene.render_with(&mut fb, Some(&s.below()), Some(&s.above()));
    save(&fb, "window-settings.png");
    // Title text: light pixels in the title bar's left part.
    let lit = (330..420).any(|x| (108..128).any(|y| fb.pixel(x, y).is_some_and(|p| p & 0xff > 0xb0)));
    assert!(lit, "the title is drawn");
}
