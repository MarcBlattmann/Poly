//! The desktop's glue (ADR 0107): what the compositor needs around
//! `poly-shell` — keys for the launcher from evdev codes and the seat's
//! modifiers, the launcher's hotkeys, the apps from `.desktop` files, the
//! font, and the keymap file `wl_keyboard.keymap` hands out.

use crate::io;
use crate::os::{self, RawFd};
use crate::prelude::*;
use poly_input::Modifiers;
use poly_input::codes::key as k;
use poly_input::layout::Layout;
use poly_input::xkb::mods;
use poly_shell::Key;
use poly_shell::apps::App;
use poly_shell::shortcuts::{Hit, Keymap, Matcher, Mods};

/// The shell's fallback font (ADR 0105's font store).
pub const FONT: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

/// Alpine Shell's fonts, where the desktop's images put them (ADR 0127).
pub const SHELL_FONTS: &str = "/usr/share/poly/fonts";

/// The cursor theme's files (ADR 0252): Bibata Modern Ice, as Alpine Shell's pointer, where the
/// desktop's images put them (`cargo xtask cursors fetch`; GPL-3.0, its licence beside them).
pub const CURSORS: &str = "/usr/share/icons/Bibata-Modern-Ice/cursors";

/// The pointer's pictures from the cursor theme's files that are on the disk; `None` without any
/// (the compositor draws its own).
pub fn cursor_theme() -> Option<alloc::sync::Arc<poly_wayland::compose::CursorTheme>> {
    use poly_wayland::compose::{CursorTheme, NamedCursor, XCursor};
    const LOOKS: [NamedCursor; 12] = [
        NamedCursor::Arrow,
        NamedCursor::Progress,
        NamedCursor::Hand,
        NamedCursor::Text,
        NamedCursor::Wait,
        NamedCursor::Crosshair,
        NamedCursor::Move,
        NamedCursor::NotAllowed,
        NamedCursor::ResizeEW,
        NamedCursor::ResizeNS,
        NamedCursor::ResizeNWSE,
        NamedCursor::ResizeNESW,
    ];
    let mut theme = CursorTheme::default();
    for n in LOOKS {
        let path = format!("{CURSORS}/{}", CursorTheme::file_name(n));
        if let Some(c) = poly_rt::fs::read(&path).ok().and_then(|b| XCursor::parse(&b)) {
            theme.insert(n, c);
        }
    }
    if theme.is_empty() {
        println!("poly-compositor: no cursor theme in {CURSORS} (cursors not fetched?): Poly's own pointer");
        return None;
    }
    println!("poly-compositor: cursor theme Bibata-Modern-Ice: {} of {} looks", theme.len(), LOOKS.len());
    Some(alloc::sync::Arc::new(theme))
}

/// Alpine Shell's wallpapers, where the desktop's images put them
/// (`poly_shell::picture` files, ADR 0127).
pub const WALLPAPERS: &str = "/usr/share/poly/wallpapers";

/// The icon theme's files for the shell's app icons (ADR 0146): what
/// `poly_shell::appicon` reads, from the disk. Files over the shell's size
/// limit are not read at all.
#[derive(Debug, Default)]
pub struct DiskIcons;

impl poly_shell::appicon::IconFiles for DiskIcons {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let meta = poly_rt::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > poly_shell::appicon::MAX_FILE as u64 {
            return None;
        }
        poly_rt::fs::read(path).ok()
    }

    fn list(&self, dir: &str) -> Vec<String> {
        poly_rt::fs::read_dir(dir).map(|entries| entries.into_iter().map(|e| e.name).collect()).unwrap_or_default()
    }
}

/// How long an empty desktop waits for the next window of an app that is
/// still running before it shows the launcher: Google Chrome closes its
/// first-run Terms window and opens Welcome, then the sign-in page, then
/// its browser window, each some seconds later on Poly, and the launcher
/// came up over each of them.
pub const REOPEN_WAIT_MS: u64 = 10_000;

/// Whether an empty desktop shows the launcher now (the last window gone,
/// or a no-window app exited), with that wait, and since when the desktop has been waiting (`waiting`, kept by
/// the caller). The last window gone while an app the desktop started
/// still runs (`apps_running`) starts the wait; a window, the launcher
/// shown by hand, or the wait running out ends it. An app that exits shows
/// the launcher at once, as before.
pub fn launcher_reopen(
    launcher_visible: bool,
    has_windows: bool,
    had_windows: bool,
    app_exited: bool,
    apps_running: bool,
    waiting: Option<u64>,
    now_ms: u64,
) -> (bool, Option<u64>) {
    if launcher_visible || has_windows {
        return (false, None);
    }
    if app_exited && !apps_running {
        return (true, None);
    }
    match waiting {
        Some(since) if now_ms.saturating_sub(since) >= REOPEN_WAIT_MS => (true, None),
        Some(since) => (false, Some(since)),
        None if had_windows || app_exited => {
            if apps_running {
                (false, Some(now_ms))
            } else {
                (true, None)
            }
        }
        None => (false, None),
    }
}

/// Give the shell the wallpapers' pictures that are on the disk.
pub fn load_wallpapers(shell: &mut poly_shell::Shell) {
    for (id, _, _, _, _) in poly_shell::theme::wallpaper::PRESETS {
        let path = format!("{WALLPAPERS}/{id}.pwal");
        if let Some(p) = poly_rt::fs::read(&path).ok().and_then(|d| poly_shell::picture::parse(&d)) {
            shell.add_wallpaper(id, p);
        }
    }
}

/// A font file read and kept for the program's life.
fn load_font(path: &str) -> Option<poly_font::Font<'static>> {
    let data = poly_rt::fs::read(path).ok()?;
    let data: &'static [u8] = Box::leak(data.into_boxed_slice());
    poly_font::Font::parse(data).ok()
}

/// The shell's fonts: Geist (regular, medium, semibold), Averia Serif
/// Libre and Geist Mono; without them DejaVu Sans for everything; without
/// that the bitmap font.
pub fn shell_fonts() -> poly_shell::draw::Fonts {
    let f = |name: &str| load_font(&format!("{SHELL_FONTS}/{name}"));
    let mut fonts = poly_shell::draw::Fonts {
        sans: f("Geist-Regular.ttf"),
        medium: f("Geist-Medium.ttf"),
        semibold: f("Geist-SemiBold.ttf"),
        display: f("AveriaSerifLibre-Regular.ttf"),
        mono: f("GeistMono-Regular.ttf"),
        display_bold: f("AveriaSerifLibre-Bold.ttf"),
    };
    if fonts.sans.is_none() {
        println!(
            "poly-compositor: no {SHELL_FONTS}/Geist-Regular.ttf (fonts not fetched?): the shell uses DejaVu Sans"
        );
        fonts.sans = load_font(FONT);
        if fonts.sans.is_none() {
            println!("poly-compositor: no {FONT} either: the shell uses the bitmap font");
        }
    }
    fonts
}

/// The system menu's Restart or Shut Down, confirmed: the kernel writes
/// the disks back and restarts or powers off (`os::power`); it says so if
/// it cannot.
pub fn power(action: poly_shell::Power) {
    println!("poly-compositor: {action:?} from the system menu");
    let e = os::power(action == poly_shell::Power::Restart);
    println!("poly-compositor: reboot failed: {e}");
}

/// poly-input's modifiers for the seat's XKB modifier bits.
pub fn modifiers(depressed: u32, locked: u32) -> Modifiers {
    let all = depressed | locked;
    let mut m = Modifiers::NONE;
    for (bit, modifier) in [
        (mods::SHIFT, Modifiers::SHIFT),
        (mods::LOCK, Modifiers::CAPS_LOCK),
        (mods::CONTROL, Modifiers::CONTROL),
        (mods::MOD1, Modifiers::ALT),
        (mods::MOD2, Modifiers::NUM_LOCK),
        (mods::MOD4, Modifiers::META),
        (mods::MOD5, Modifiers::ALT_GR),
    ] {
        if all & bit != 0 {
            m = m.union(modifier);
        }
    }
    m
}

/// What a key press (evdev `code`) means to the launcher, with the seat's
/// modifiers, on `layout`.
pub fn launcher_key(code: u32, depressed: u32, locked: u32, layout: &Layout) -> Option<Key> {
    let code = u16::try_from(code).ok()?;
    let ctrl = depressed & mods::CONTROL != 0;
    match code {
        k::ESC => return Some(Key::Escape),
        k::ENTER | k::KPENTER => return Some(Key::Enter),
        k::BACKSPACE if ctrl => return Some(Key::BackspaceWord),
        k::BACKSPACE => return Some(Key::Backspace),
        k::UP => return Some(Key::Up),
        k::DOWN => return Some(Key::Down),
        k::P if ctrl => return Some(Key::Up),
        k::N if ctrl => return Some(Key::Down),
        k::W if ctrl => return Some(Key::BackspaceWord),
        k::U if ctrl => return Some(Key::Clear),
        _ => {}
    }
    if ctrl || depressed & (mods::MOD1 | mods::MOD4) != 0 {
        return None;
    }
    let key = poly_input::keycode::key(code)?;
    let sym = poly_input::keysym::keysym(layout, key, modifiers(depressed, locked));
    sym.to_char().filter(|c| !c.is_control()).map(Key::Char)
}

/// The shell's modifiers for the seat's XKB modifier bits.
pub fn shortcut_mods(depressed: u32) -> Mods {
    let mut m = Mods::NONE;
    for (bit, modifier) in
        [(mods::MOD4, Mods::WIN), (mods::CONTROL, Mods::CTRL), (mods::MOD1, Mods::ALT), (mods::SHIFT, Mods::SHIFT)]
    {
        if depressed & bit != 0 {
            m = m.union(modifier);
        }
    }
    m
}

/// The keyboard shortcuts (ADR 0129): evdev keys matched against the
/// shell's keymap (`poly_shell::shortcuts`), which the settings change.
#[derive(Debug, Default)]
pub struct Shortcuts {
    matcher: Matcher,
}

impl Shortcuts {
    /// Key `code` went down or up, the seat's modifiers `depressed` before
    /// it: what it does by `map`, if anything.
    pub fn key(
        &mut self,
        map: &Keymap,
        code: u32,
        pressed: bool,
        depressed: u32,
        layout: &Layout,
        switcher_open: bool,
    ) -> Option<Hit> {
        let key = poly_input::keycode::key(u16::try_from(code).ok()?)?;
        self.matcher.key(map, key, pressed, shortcut_mods(depressed), layout, switcher_open)
    }
}

/// The disk for the shell's reading of the installed apps
/// (`poly_shell::apps::installed`).
#[derive(Clone, Copy, Debug, Default)]
pub struct DiskApps;

impl poly_shell::apps::AppFiles for DiskApps {
    fn list(&self, dir: &str) -> Vec<String> {
        poly_rt::fs::read_dir(dir).map(|entries| entries.into_iter().map(|e| e.name).collect()).unwrap_or_default()
    }

    fn read(&self, path: &str) -> Option<String> {
        poly_rt::fs::read_to_string(path).ok()
    }
}

/// The desktop user's own applications directory: the AppImages the
/// installer put there (ADR 0180).
fn user_applications() -> String {
    let data = poly_shell::apps::data_home(poly_rt::env::var("XDG_DATA_HOME"), poly_rt::env::var("HOME"));
    poly_shell::apps::user_applications(&data)
}

/// Whether `app` starts as the desktop's user (ADR 0218,
/// [`poly_shell::apps::runs_as_user`]), from the disk as it is now.
pub fn runs_as_user(app: &App) -> bool {
    poly_shell::apps::runs_as_user(&DiskApps, &user_applications(), app)
}

/// Every installed app: Poly's own and the desktop files' (the one list
/// the launcher offers and Files' Applications folder shows, ADR 0175).
pub fn installed_apps() -> Vec<App> {
    let user = user_applications();
    poly_shell::apps::installed_for_user(&DiskApps, &user, &mut |name, why| {
        eprintln!("poly-compositor: {name}: not offered ({why:?})");
    })
}

/// The keymap as a file for `wl_keyboard.keymap`: the XKB text and its
/// NUL in a sealed memory object, read-only (the runtime directory `_dir`
/// held it as an unlinked file before memory objects). Returns the handle
/// and the size.
pub fn keymap_file(_dir: &str, layout: &Layout) -> io::Result<(RawFd, u32)> {
    let mut text = poly_input::xkb::keymap(layout).into_bytes();
    text.push(0);
    let fd = os::read_only_file("poly-keymap", &text).ok_or_else(|| io::Error::other("no memory object"))?;
    Ok((fd, text.len() as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_input::layout::{DE_CH, US};

    #[test]
    fn launcher_keys() {
        assert_eq!(launcher_key(30, 0, 0, &US), Some(Key::Char('a')));
        assert_eq!(launcher_key(30, mods::SHIFT, 0, &US), Some(Key::Char('A')));
        assert_eq!(launcher_key(30, 0, mods::LOCK, &US), Some(Key::Char('A')));
        assert_eq!(launcher_key(57, 0, 0, &US), Some(Key::Char(' ')));
        assert_eq!(launcher_key(21, 0, 0, &DE_CH), Some(Key::Char('z')), "Swiss German swaps Y and Z");
        assert_eq!(launcher_key(1, 0, 0, &US), Some(Key::Escape));
        assert_eq!(launcher_key(28, 0, 0, &US), Some(Key::Enter));
        assert_eq!(launcher_key(14, 0, 0, &US), Some(Key::Backspace));
        assert_eq!(launcher_key(14, mods::CONTROL, 0, &US), Some(Key::BackspaceWord));
        assert_eq!(launcher_key(108, 0, 0, &US), Some(Key::Down));
        assert_eq!(launcher_key(25, mods::CONTROL, 0, &US), Some(Key::Up), "Ctrl+P");
        assert_eq!(launcher_key(30, mods::CONTROL, 0, &US), None, "Ctrl+A types nothing");
        assert_eq!(launcher_key(42, 0, 0, &US), None, "Shift alone");
    }

    #[test]
    fn shortcuts_from_evdev_keys() {
        use poly_shell::shortcuts::{Action, Chord};
        use poly_shell::wm::Arrow;
        let mut map = Keymap::default();
        let mut s = Shortcuts::default();
        let mut key = |map: &Keymap, code, pressed, depressed, open| s.key(map, code, pressed, depressed, &US, open);
        // The Windows key tapped alone, and Ctrl+Space.
        assert_eq!(key(&map, 125, true, 0, false), None);
        assert_eq!(key(&map, 125, false, mods::MOD4, false), Some(Hit::Launcher));
        assert_eq!(key(&map, 57, true, mods::CONTROL, false), Some(Hit::Launcher));
        assert_eq!(key(&map, 57, true, 0, false), None);
        // Alt+Tab opens, Tab steps, releasing Alt switches.
        assert_eq!(key(&map, 15, true, mods::MOD1, false), Some(Hit::SwitchStart { backward: false }));
        assert_eq!(key(&map, 15, true, mods::MOD1 | mods::SHIFT, false), Some(Hit::SwitchStart { backward: true }));
        assert_eq!(key(&map, 15, true, mods::MOD1, true), Some(Hit::SwitchStep { forward: true }));
        assert_eq!(key(&map, 56, false, mods::MOD1, true), Some(Hit::SwitchCommit));
        assert_eq!(key(&map, 56, false, mods::MOD1, false), None, "Alt released without the switcher");
        assert_eq!(key(&map, 1, true, mods::MOD1, true), Some(Hit::SwitchCancel));
        assert_eq!(key(&map, 15, true, 0, false), None, "plain Tab is the client's");
        // Alt+F4, Super+arrows, Super+I, Super+N.
        assert_eq!(key(&map, 62, true, mods::MOD1, false), Some(Hit::Close));
        assert_eq!(key(&map, 105, true, mods::MOD4, false), Some(Hit::Place(Arrow::Left)));
        assert_eq!(key(&map, 103, true, mods::MOD4, false), Some(Hit::Place(Arrow::Up)));
        assert_eq!(key(&map, 105, true, 0, false), None);
        assert_eq!(key(&map, 23, true, mods::MOD4, false), Some(Hit::Settings));
        assert_eq!(key(&map, 49, true, mods::MOD4, false), Some(Hit::Notifications));
        // Rebound in the settings: the new combination, not the old.
        map.set(Action::Settings, Chord::parse("Ctrl+Alt+O").unwrap());
        assert_eq!(key(&map, 23, true, mods::MOD4, false), None);
        assert_eq!(key(&map, 24, true, mods::CONTROL | mods::MOD1, false), Some(Hit::Settings));
    }

    #[test]
    fn modifier_bits() {
        let m = modifiers(mods::SHIFT | mods::MOD5, mods::LOCK);
        assert!(m.shift() && m.alt_gr() && m.caps_lock() && !m.control());
    }

    /// Without a kernel (the host) there are no desktop files and no
    /// memory objects; the desktop boot tests check both on Poly.
    #[test]
    fn keymap_files_and_desktop_files_need_the_kernel() {
        let own: Vec<String> = poly_shell::apps::builtins().into_iter().map(|a| a.id).collect();
        assert!(installed_apps().iter().all(|a| own.contains(&a.id)), "no disk: only Poly's own apps");
        assert!(keymap_file("/run/user/0", &US).is_err());
    }

    #[test]
    fn an_empty_desktop_returns_to_the_launcher_after_an_app_exits() {
        assert_eq!(
            launcher_reopen(false, false, false, true, false, None, 0),
            (true, None),
            "no-window apps such as notify-send"
        );
        assert_eq!(launcher_reopen(false, false, true, false, false, None, 0), (true, None), "the last window closed");
        assert_eq!(
            launcher_reopen(true, false, false, true, false, None, 0),
            (false, None),
            "do not toggle a visible launcher off"
        );
        assert_eq!(
            launcher_reopen(false, true, true, true, false, None, 0),
            (false, None),
            "another window is still open"
        );
        assert_eq!(
            launcher_reopen(false, false, false, false, true, None, 0),
            (false, None),
            "a live app has not opened a window yet"
        );
    }

    #[test]
    fn the_launcher_waits_for_a_running_apps_next_window() {
        let w = REOPEN_WAIT_MS;
        // Chrome closes its Terms window and is still running: wait.
        assert_eq!(launcher_reopen(false, false, true, false, true, None, 1000), (false, Some(1000)));
        // Its next window comes: the wait ends, no launcher.
        assert_eq!(launcher_reopen(false, true, false, false, true, Some(1000), 3000), (false, None));
        // No window within the wait: the launcher.
        assert_eq!(launcher_reopen(false, false, false, false, true, Some(1000), 1000 + w), (true, None));
        assert_eq!(launcher_reopen(false, false, false, false, true, Some(1000), 999 + w), (false, Some(1000)));
        // The last app exits: at once.
        assert_eq!(launcher_reopen(false, false, false, true, false, Some(1000), 2000), (true, None));
        assert_eq!(launcher_reopen(false, false, true, true, false, None, 2000), (true, None));
        // Nothing of the desktop's runs (a window of a program started
        // elsewhere closed): at once, as before.
        assert_eq!(launcher_reopen(false, false, true, false, false, None, 5), (true, None));
        // Shown by hand meanwhile: the wait is over.
        assert_eq!(launcher_reopen(true, false, false, false, true, Some(1000), 2000), (false, None));
    }
}
