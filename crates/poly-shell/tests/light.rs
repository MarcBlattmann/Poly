//! The shell in the light theme (ADR 0127: Alpine's light tokens), drawn
//! with real fonts like `preview.rs`, in its own test program because the
//! theme is the process's. With `POLY_SHELL_PNG_DIR` set the pictures are
//! written as `light-*.png` for the comparison with the live Alpine Shell.

use poly_font::Font;
use poly_shell::draw::{Fonts, Text};
use poly_shell::settings::{Change, Settings};
use poly_shell::settings_ui::{Page, SystemInfo};
use poly_shell::{Key, Panel, Shell, apps};
use poly_wayland::compose::{Framebuffer, Layer, MemoryFramebuffer, Rect};
use std::path::Path;

fn leak(path: &Path) -> Option<Font<'static>> {
    let data = std::fs::read(path).ok()?;
    Font::parse(Box::leak(data.into_boxed_slice())).ok()
}

fn save(fb: &MemoryFramebuffer, name: &str) {
    let Some(dir) = std::env::var_os("POLY_SHELL_PNG_DIR") else { return };
    let rgb: Vec<u8> = fb.pixels().iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    std::fs::write(Path::new(&dir).join(name), poly_font::png::encode(fb.width(), fb.height(), 3, &rgb)).unwrap();
}

fn draw(shell: &Shell, fb: &mut MemoryFramebuffer) {
    let all = Rect::new(0, 0, fb.width() as i32, fb.height() as i32);
    shell.below().draw(fb, all, 1);
    shell.above().draw(fb, all, 1);
}

#[test]
fn the_shell_in_the_light_theme() {
    let fonts = std::env::var_os("POLY_SHELL_FONT_DIR").map(|d| {
        let d = Path::new(&d).to_path_buf();
        let f = |n: &str| leak(&d.join(n));
        Fonts {
            sans: f("Geist-Regular.ttf"),
            medium: f("Geist-Medium.ttf"),
            semibold: f("Geist-SemiBold.ttf"),
            display: f("AveriaSerifLibre-Regular.ttf"),
            mono: f("GeistMono-Regular.ttf"),
            display_bold: f("AveriaSerifLibre-Bold.ttf"),
        }
    });
    let mut s = Shell::new(1920, 1080, apps::catalog(Vec::new()), Text::with_fonts(fonts.unwrap_or_default()));
    s.set_animations(false);
    s.set_settings(Settings { theme: "light".into(), ..Settings::default() });
    s.set_time(1_793_491_200 + 20 * 3600 + 52 * 60, 0);
    s.set_network(poly_shell::Network::Online("eth0".into()));
    s.set_volume(Some((60, false)));
    s.set_now_ms(1000);
    assert!(poly_shell::theme::is_light());
    let mut fb = MemoryFramebuffer::new(1920, 1080);
    draw(&s, &mut fb);
    save(&fb, "light-launcher-empty.png");
    // The ridge in the light ground.
    assert_eq!(fb.pixel(300, 10), Some(poly_shell::theme::LIGHT.bar));
    s.key(Key::Char('s'));
    draw(&s, &mut fb);
    save(&fb, "light-launcher-typed.png");
    s.hide_launcher();
    for (p, name) in [(Panel::Menu, "menu"), (Panel::Control, "control"), (Panel::Calendar, "calendar")] {
        s.open_panel(p);
        draw(&s, &mut fb);
        save(&fb, &format!("light-panel-{name}.png"));
    }
    s.close_panel();
    s.notify(poly_dbus::notifications::Notify {
        app_name: "notify-send".into(),
        summary: "Hello from a test client".into(),
        body: "Sent through org.freedesktop.Notifications on the session bus".into(),
        expire_timeout: -1,
        ..Default::default()
    });
    draw(&s, &mut fb);
    save(&fb, "light-notices.png");
    let info = SystemInfo {
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
    };
    for page in Page::ALL {
        s.settings_window.open_at(page);
        let mut w = MemoryFramebuffer::new(1200, 800);
        s.draw_settings(&mut w, 1200, 800, 1, &info, 1_793_491_200);
        let name = page.label().to_lowercase().replace(" & ", "-").replace(' ', "-");
        save(&w, &format!("light-settings-{name}-1200.png"));
        assert_eq!(w.pixel(10, 700), Some(poly_shell::theme::LIGHT.s0), "{page:?}: the light ground");
    }
    // Back to dark.
    assert!(s.apply(&Change::Theme("dark".into())));
    assert!(!poly_shell::theme::is_light());
}
