//! Tests.

use super::*;
use poly_wayland::compose::MemoryFramebuffer;

fn info() -> SystemInfo {
    SystemInfo {
        os: "Poly 0.0.1".into(),
        kernel: "Poly 0.0.1".into(),
        machine: "x86_64".into(),
        cpu: "QEMU Virtual CPU".into(),
        memory_kib: 512 * 1024,
        memory_available_kib: 256 * 1024,
        display: (1280, 800),
        modes: vec![(1280, 800)],
        sound: Some("virtio-sound".into()),
        network: "eth0 up".into(),
        session_bus: "unix:path=/run/user/0/bus".into(),
    }
}

static NO_PICTURES: crate::picture::Pictures = crate::picture::Pictures::new();

fn cx<'a>(st: &'a Settings, i: &'a SystemInfo, text: &'a Text) -> Context<'a> {
    Context { settings: st, info: i, text, now: "12:00 Nov 1", utc: 1_793_491_200, pictures: &NO_PICTURES }
}

const W: i32 = t::WIDTH;
const H: i32 = t::HEIGHT;

/// The centre of the first target `want`.
fn at(win: &SettingsWindow, c: &Context<'_>, want: Target) -> (i32, i32) {
    let (_, targets) = win.layout(W, H, c);
    let r = targets.iter().find(|(_, t)| *t == want).unwrap_or_else(|| panic!("{want:?}")).0;
    (r.x + r.width / 2, r.y + r.height / 2)
}

#[test]
fn tabs_pages_and_changes() {
    let text = Text::new(None);
    let (st, i) = (Settings::default(), info());
    let c = cx(&st, &i, &text);
    let mut win = SettingsWindow::default();
    assert_eq!(win.page, Page::Home);
    // The Devices tab opens its first page; its pages are in the sidebar.
    let (x, y) = at(&win, &c, Target::Tab(3));
    assert_eq!(win.button(x, y, true, W, H, &c), (true, None));
    assert_eq!(win.page, Page::Display);
    assert_eq!(win.nav().iter().map(|n| n.1).collect::<Vec<_>>(), [Page::Display, Page::Sound, Page::Keyboard]);
    let (x, y) = at(&win, &c, Target::Nav(Page::Sound));
    win.button(x, y, true, W, H, &c);
    assert_eq!(win.page, Page::Sound);
    win.button(x, y, false, W, H, &c);
    // The volume slider: a press at its middle is 50 %, a drag moves it.
    let (sx, sy) = at(&win, &c, Target::Volume);
    let (_, change) = win.button(sx, sy, true, W, H, &c);
    assert_eq!(change, Some(Change::Volume(50)));
    let (_, change) = win.motion(sx + 1000, sy, W, H, &c);
    assert_eq!(change, Some(Change::Volume(100)));
    win.button(sx, sy, false, W, H, &c);
    let (mx, my) = at(&win, &c, Target::Mute);
    assert_eq!(win.button(mx, my, true, W, H, &c).1, Some(Change::Muted(true)));
    // The keyboard layouts.
    win.page = Page::Keyboard;
    let (kx, ky) = at(&win, &c, Target::Keyboard(1));
    assert_eq!(win.button(kx, ky, true, W, H, &c).1, Some(Change::Keyboard("de-ch".into())));
    // Wallpapers, time zones, the scale.
    win.page = Page::Wallpaper;
    let (wx, wy) = at(&win, &c, Target::Wallpaper(4));
    assert_eq!(win.button(wx, wy, true, W, H, &c).1, Some(Change::Wallpaper("forest".into())));
    win.page = Page::DateTime;
    let (zx, zy) = at(&win, &c, Target::Zone(1));
    assert_eq!(win.button(zx, zy, true, W, H, &c).1, Some(Change::TimeZone("Europe/Zurich".into())));
    win.page = Page::Display;
    let (dx, dy) = at(&win, &c, Target::Scale(2));
    assert_eq!(win.button(dx, dy, true, W, H, &c).1, Some(Change::Scale(2)));
    // Home's cards lead to pages.
    win.page = Page::Home;
    let (hx, hy) = at(&win, &c, Target::Nav(Page::Sound));
    win.button(hx, hy, true, W, H, &c);
    assert_eq!(win.page, Page::Sound);
}

#[test]
fn the_search_finds_pages_by_words() {
    assert_eq!(search("vol").first().map(|r| r.0), Some(Page::Sound));
    assert_eq!(search("zone").first().map(|r| r.0), Some(Page::DateTime));
    assert!(search("  ").is_empty());
    let text = Text::new(None);
    let (st, i) = (Settings::default(), info());
    let c = cx(&st, &i, &text);
    let mut win = SettingsWindow::default();
    let (fx, fy) = at(&win, &c, Target::Search);
    win.button(fx, fy, true, W, H, &c);
    for ch in "layout".chars() {
        assert!(win.key(Key::Char(ch)));
    }
    let (rx, ry) = at(&win, &c, Target::Result(0));
    win.button(rx, ry, true, W, H, &c);
    assert_eq!(win.page, Page::Keyboard);
    assert_eq!(win.search_text(), "", "going to a page ends the search");
    // Enter opens the first result; Escape clears.
    for ch in "scale".chars() {
        win.key(Key::Char(ch));
    }
    assert!(win.key(Key::Enter));
    assert_eq!(win.page, Page::Display);
    win.key(Key::Char('x'));
    assert!(win.key(Key::Escape));
    assert_eq!(win.search_text(), "");
}

#[test]
fn the_arrows_go_back_and_forward() {
    let text = Text::new(None);
    let (st, i) = (Settings::default(), info());
    let c = cx(&st, &i, &text);
    let mut win = SettingsWindow::default();
    let (_, targets) = win.layout(W, H, &c);
    assert!(!targets.iter().any(|(_, t)| *t == Target::Back), "nowhere to go back to");
    let (x, y) = at(&win, &c, Target::Tab(4));
    win.button(x, y, true, W, H, &c);
    let (bx, by) = at(&win, &c, Target::Back);
    win.button(bx, by, true, W, H, &c);
    assert_eq!(win.page, Page::Home);
    let (fx, fy) = at(&win, &c, Target::Forward);
    win.button(fx, fy, true, W, H, &c);
    assert_eq!(win.page, Page::DateTime);
}

#[test]
fn every_page_fits_and_draws() {
    let text = Text::new(None);
    let (st, i) = (Settings::default(), info());
    let c = cx(&st, &i, &text);
    let mut win = SettingsWindow::default();
    for page in Page::ALL {
        win.page = page;
        let (items, targets) = win.layout(W, H, &c);
        for (r, _) in &targets {
            assert!(r.right() <= W && r.bottom() <= H, "{page:?}: {r:?} is outside the window");
        }
        assert!(!items.is_empty());
        let mut fb = MemoryFramebuffer::new(W as u32, H as u32);
        win.draw(&mut fb, W, H, 1, &c);
        assert_eq!(fb.pixel(10, H as u32 - 10), Some(pal().s0), "the sidebar's ground");
        assert_eq!(fb.pixel(W as u32 - 5, H as u32 - 5), Some(pal().s1), "the page");
        assert_eq!(fb.pixel(t::SIDEBAR as u32 + 40, t::HEADER as u32), Some(pal().line), "the page's hairline");
    }
    // The active page is highlighted in the sidebar, its tab in the header.
    win.page = Page::Display;
    let mut fb = MemoryFramebuffer::new(W as u32, H as u32);
    win.draw(&mut fb, W, H, 1, &c);
    let row = win.nav().into_iter().find(|n| n.1 == Page::Display).unwrap().0;
    assert_eq!(fb.pixel((row.right() - 6) as u32, (row.y + 4) as u32), Some(pal().s3));
    let tab = win.tabs(&text).into_iter().find(|t| t.1 == 3).unwrap().0;
    assert_eq!(fb.pixel((tab.x + 4) as u32, (tab.y + 14) as u32), Some(pal().s3));
    // At scale 2 too.
    let mut fb = MemoryFramebuffer::new(2 * W as u32, 2 * H as u32);
    win.draw(&mut fb, W, H, 2, &c);
    assert_eq!(fb.pixel(2 * W as u32 - 5, 2 * H as u32 - 5), Some(pal().s1));
}

#[test]
fn controls_are_drawn_in_alpines_colours() {
    let text = Text::new(None);
    let st = Settings { muted: true, ..Settings::default() };
    let i = info();
    let c = cx(&st, &i, &text);
    let win = SettingsWindow { page: Page::Sound, ..SettingsWindow::default() };
    let (items, _) = win.layout(W, H, &c);
    let toggle = items.iter().find_map(|i| if let Item::Toggle { rect, .. } = i { Some(*rect) } else { None }).unwrap();
    let mut fb = MemoryFramebuffer::new(W as u32, H as u32);
    win.draw(&mut fb, W, H, 1, &c);
    // On: the track in the ink, the knob in the ground colour on the right.
    assert_eq!(fb.pixel((toggle.x + 6) as u32, (toggle.y + 11) as u32), Some(pal().ink_primary));
    assert_eq!(fb.pixel((toggle.x + 27) as u32, (toggle.y + 11) as u32), Some(pal().s0));
}

/// The places the `desktop-shell` boot test clicks (tools/xtask/src/desktop.rs).
#[test]
fn the_boot_tests_clicks_land() {
    let text = Text::new(None);
    let (st, i) = (Settings::default(), info());
    let c = cx(&st, &i, &text);
    let mut win = SettingsWindow::default();
    assert_eq!(win.target_at(400, 339, W, H, &c), Some(Target::Nav(Page::Sound)));
    win.button(400, 339, true, W, H, &c);
    assert_eq!(win.button(691, 178, true, W, H, &c).1, Some(Change::Volume(30)));
    win.button(691, 178, false, W, H, &c);
    assert_eq!(win.target_at(100, 138, W, H, &c), Some(Target::Nav(Page::Keyboard)));
    win.button(100, 138, true, W, H, &c);
    assert_eq!(win.target_at(500, 221, W, H, &c), Some(Target::Keyboard(1)));
    // Keyboard shortcuts (ADR 0129): Up three times from Keyboard, the
    // Search hotkey's Set (its right edge does not move with the font).
    for _ in 0..3 {
        win.key(Key::Up);
    }
    assert_eq!(win.page, Page::Shortcuts);
    assert_eq!(win.target_at(858, 178, W, H, &c), Some(Target::Record(Action::Launcher)));
    // Once one is changed, Restore all defaults on the title's line.
    let mut changed = st.clone();
    changed.apply(&Change::Shortcut(Action::Launcher, crate::shortcuts::Chord::parse("Ctrl+Alt+K").unwrap()));
    let c = cx(&changed, &i, &text);
    assert_eq!(win.target_at(872, 77, W, H, &c), Some(Target::ResetShortcuts));
}

#[test]
fn appearance_tiling_and_shortcuts() {
    let text = Text::new(None);
    let (st, i) = (Settings::default(), info());
    let c = cx(&st, &i, &text);
    let mut win = SettingsWindow { page: Page::Appearance, ..SettingsWindow::default() };
    let (x, y) = at(&win, &c, Target::Theme(true));
    assert_eq!(win.button(x, y, true, W, H, &c).1, Some(Change::Theme("light".into())));
    win.page = Page::Tiling;
    let (x, y) = at(&win, &c, Target::Tiling);
    assert_eq!(win.button(x, y, true, W, H, &c).1, Some(Change::Tiling(false)));
    win.button(x, y, false, W, H, &c);
    let (x, y) = at(&win, &c, Target::SnapKeys);
    assert_eq!(win.button(x, y, true, W, H, &c).1, Some(Change::SnapKeys(false)));
    let (x, y) = at(&win, &c, Target::SnapDrag);
    assert_eq!(win.button(x, y, true, W, H, &c).1, Some(Change::SnapDrag(false)));
    // The Desktop tab lists Alpine's Desktop pages.
    assert_eq!(win.nav().iter().map(|n| n.1).collect::<Vec<_>>(), [Page::Search, Page::Tiling, Page::Shortcuts]);
    win.page = Page::Shortcuts;
    let (items, _) = win.layout(W, H, &c);
    assert!(items.iter().any(|i| matches!(i, Item::Caps(c) if c.iter().any(|k| k.1 == "Tab"))));
    // With tiling off, the page is the switch alone.
    let off = Settings { tiling: false, ..Settings::default() };
    let c = cx(&off, &i, &text);
    win.page = Page::Tiling;
    let (_, targets) = win.layout(W, H, &c);
    assert!(targets.iter().any(|t| t.1 == Target::Tiling));
    assert!(!targets.iter().any(|t| t.1 == Target::SnapKeys));
    assert_eq!(off.snaps(), (false, false));
}

#[test]
fn shortcuts_are_recorded_refused_and_reset() {
    use crate::shortcuts::{Chord, Mods};
    use poly_input::Key as K;
    use poly_input::layout::US;
    let text = Text::new(None);
    let (mut st, i) = (Settings::default(), info());
    let mut win = SettingsWindow { page: Page::Shortcuts, ..SettingsWindow::default() };
    let c = cx(&st, &i, &text);
    // Set starts recording: "Press a shortcut", the button says Cancel.
    let (x, y) = at(&win, &c, Target::Record(Action::Settings));
    assert_eq!(win.button(x, y, true, W, H, &c), (true, None));
    assert!(win.recording());
    let (items, _) = win.layout(W, H, &c);
    assert!(items.iter().any(|i| matches!(i, Item::Pill { text, .. } if text == "Press a shortcut")));
    assert!(items.iter().any(|i| matches!(i, Item::Button { label, .. } if label == "Cancel")));
    // Win+N is taken: refused, the row says so in red.
    win.recorder.key(K::MetaLeft, true);
    let (_, done) = win.recorder.key(K::N, true);
    assert_eq!(win.recorded(done.unwrap(), &st.shortcuts, &US), None);
    assert!(!win.recording());
    let (items, _) = win.layout(W, H, &c);
    let red = |items: &[Item]| {
        items.iter().any(
            |i| matches!(i, Item::Text { text, rgb, .. } if text == "Already used by Notifications." && *rgb == pal().danger),
        )
    };
    assert!(red(&items));
    // Again, Ctrl+Alt+O: taken.
    win.button(x, y, true, W, H, &c);
    assert!(!red(&win.layout(W, H, &c).0), "a new recording clears the refusal");
    win.recorder.key(K::ControlLeft, true);
    win.recorder.key(K::AltLeft, true);
    let (_, done) = win.recorder.key(K::O, true);
    let change = win.recorded(done.unwrap(), &st.shortcuts, &US).unwrap();
    let chord = Chord::key(Mods::CTRL.union(Mods::ALT), "O");
    assert_eq!(change, Change::Shortcut(Action::Settings, chord.clone()));
    assert!(st.apply(&change));
    assert_eq!(st.shortcuts.chord(Action::Settings), chord);
    // Changed: its caps, its reset, and Restore all defaults.
    let c = cx(&st, &i, &text);
    let (items, targets) = win.layout(W, H, &c);
    assert!(
        items.iter().any(|i| matches!(i, Item::Caps(k) if k.iter().map(|k| k.1.as_str()).eq(["Ctrl", "Alt", "O"])))
    );
    assert!(targets.iter().any(|t| t.1 == Target::ResetShortcuts));
    let (x, y) = at(&win, &c, Target::ResetShortcut(Action::Settings));
    assert_eq!(win.button(x, y, true, W, H, &c).1, Some(Change::ResetShortcut(Action::Settings)));
    let (x, y) = at(&win, &c, Target::ResetShortcuts);
    assert_eq!(win.button(x, y, true, W, H, &c).1, Some(Change::ResetShortcuts));
    // Escape cancels; Cancel and a click elsewhere end it too.
    let (x, y) = at(&win, &c, Target::Record(Action::Launcher));
    win.button(x, y, true, W, H, &c);
    let (_, done) = win.recorder.key(K::Escape, true);
    assert_eq!(done, Some(Recorded::Cancelled));
    assert_eq!(win.recorded(done.unwrap(), &st.shortcuts, &US), None);
    win.button(x, y, true, W, H, &c);
    win.button(x, y, true, W, H, &c);
    assert!(!win.recording(), "Cancel");
    win.button(x, y, true, W, H, &c);
    win.button(W - 20, H - 20, true, W, H, &c);
    assert!(!win.recording(), "a click elsewhere");
    // Nothing changed: Restore all defaults does nothing.
    let st = Settings::default();
    let c = cx(&st, &i, &text);
    assert!(!win.layout(W, H, &c).1.iter().any(|t| t.1 == Target::ResetShortcuts));
    // The Search page's hotkey records the same binding.
    win.page = Page::Search;
    assert!(win.layout(W, H, &c).1.iter().any(|t| t.1 == Target::Record(Action::Launcher)));
}

#[test]
fn offsets() {
    assert_eq!(utc_offset(60), "UTC+01:00");
    assert_eq!(utc_offset(-300), "UTC-05:00");
    assert_eq!(utc_offset(330), "UTC+05:30");
}
