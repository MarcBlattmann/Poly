//! The desktop's glue (ADR 0107): what the compositor needs around
//! `poly-shell` — keys for the launcher from evdev codes and the seat's
//! modifiers, the launcher's hotkeys, the apps from `.desktop` files, the
//! font, and the keymap file `wl_keyboard.keymap` hands out.

use poly_input::Modifiers;
use poly_input::codes::key as k;
use poly_input::layout::Layout;
use poly_input::xkb::mods;
use poly_shell::Key;
use poly_shell::apps::{App, parse_desktop_entry};
use std::ffi::CString;
use std::os::fd::RawFd;
use std::path::Path;

/// Where desktop files are (the Desktop Entry Specification's
/// `$XDG_DATA_DIRS/applications`, with the default data directory).
pub const APPLICATIONS: &str = "/usr/share/applications";
/// The shell's font (ADR 0105's font store).
pub const FONT: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

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

/// The launcher's hotkeys: the Windows key tapped alone (Alpine Shell's
/// default) or Ctrl+Space.
#[derive(Debug, Default)]
pub struct Hotkeys {
    /// The Windows key is down and nothing else was pressed since.
    super_alone: bool,
}

impl Hotkeys {
    /// A key event; true if it toggles the launcher. `depressed` is the
    /// seat's modifiers after it.
    pub fn key(&mut self, code: u32, pressed: bool, depressed: u32) -> bool {
        let code = u16::try_from(code).unwrap_or(0);
        let is_super = code == k::LEFTMETA || code == k::RIGHTMETA;
        if is_super {
            if pressed {
                self.super_alone = true;
                return false;
            }
            return std::mem::take(&mut self.super_alone);
        }
        if pressed {
            self.super_alone = false;
            return code == k::SPACE && depressed & mods::CONTROL != 0;
        }
        false
    }
}

/// A window management key (ADR 0117).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmKey {
    /// Alt+Tab (with Shift: backwards) opens the switcher.
    SwitchStart {
        backward: bool,
    },
    /// Tab, Shift+Tab or Left/Right while it is open.
    SwitchStep {
        forward: bool,
    },
    /// Releasing Alt (or Enter) switches.
    SwitchCommit,
    /// Escape closes the switcher without switching.
    SwitchCancel,
    /// Alt+F4 closes the active window.
    Close,
    /// Super+arrow places the active window.
    Place(poly_shell::wm::Arrow),
    /// Super+I opens the settings, Super+N the notification history.
    Settings,
    History,
}

/// What key `code` (`pressed` or released, the seat's modifiers
/// `depressed` before it) means to window management, if anything. All but
/// Alt's release are taken from the clients.
pub fn wm_key(code: u32, pressed: bool, depressed: u32, switcher_open: bool) -> Option<WmKey> {
    use poly_shell::wm::Arrow;
    let code = u16::try_from(code).ok()?;
    let (alt, shift, sup) = (depressed & mods::MOD1 != 0, depressed & mods::SHIFT != 0, depressed & mods::MOD4 != 0);
    if !pressed {
        return (switcher_open && code == k::LEFTALT).then_some(WmKey::SwitchCommit);
    }
    if switcher_open {
        return match code {
            k::TAB => Some(WmKey::SwitchStep { forward: !shift }),
            k::RIGHT => Some(WmKey::SwitchStep { forward: true }),
            k::LEFT => Some(WmKey::SwitchStep { forward: false }),
            k::ESC => Some(WmKey::SwitchCancel),
            k::ENTER => Some(WmKey::SwitchCommit),
            _ => None,
        };
    }
    match code {
        k::TAB if alt => Some(WmKey::SwitchStart { backward: shift }),
        k::F4 if alt => Some(WmKey::Close),
        k::LEFT if sup => Some(WmKey::Place(Arrow::Left)),
        k::RIGHT if sup => Some(WmKey::Place(Arrow::Right)),
        k::UP if sup => Some(WmKey::Place(Arrow::Up)),
        k::DOWN if sup => Some(WmKey::Place(Arrow::Down)),
        k::I if sup => Some(WmKey::Settings),
        k::N if sup => Some(WmKey::History),
        _ => None,
    }
}

/// The apps of every readable desktop file in `dir`.
pub fn desktop_files(dir: &Path) -> Vec<App> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut apps = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".desktop") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(e.path()) {
            match parse_desktop_entry(&name, &text) {
                Ok(app) => apps.push(app),
                Err(why) => eprintln!("poly-compositor: {name}: not offered ({why:?})"),
            }
        }
    }
    apps
}

/// The keymap as a file for `wl_keyboard.keymap`: the XKB text and its
/// NUL in a file in `dir`, unlinked once open (Poly has no
/// `memfd_create` yet), read-only. Returns the descriptor and the size.
pub fn keymap_file(dir: &Path, layout: &Layout) -> std::io::Result<(RawFd, u32)> {
    let mut text = poly_input::xkb::keymap(layout).into_bytes();
    text.push(0);
    let path = dir.join(format!("poly-keymap-{}", std::process::id()));
    std::fs::write(&path, &text)?;
    let c = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    // SAFETY: a NUL-terminated path.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    let _ = std::fs::remove_file(&path);
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
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
    fn hotkeys() {
        let mut h = Hotkeys::default();
        // The Windows key tapped alone toggles, on release.
        assert!(!h.key(125, true, mods::MOD4));
        assert!(h.key(125, false, 0));
        // With another key in between it does not.
        assert!(!h.key(125, true, mods::MOD4));
        assert!(!h.key(30, true, mods::MOD4));
        assert!(!h.key(30, false, mods::MOD4));
        assert!(!h.key(125, false, 0));
        // Ctrl+Space.
        assert!(h.key(57, true, mods::CONTROL));
        assert!(!h.key(57, true, 0));
        assert!(!h.key(57, false, mods::CONTROL));
    }

    #[test]
    fn window_management_keys() {
        use poly_shell::wm::Arrow;
        // Alt+Tab opens, Tab steps, releasing Alt switches.
        assert_eq!(wm_key(15, true, mods::MOD1, false), Some(WmKey::SwitchStart { backward: false }));
        assert_eq!(wm_key(15, true, mods::MOD1 | mods::SHIFT, false), Some(WmKey::SwitchStart { backward: true }));
        assert_eq!(wm_key(15, true, mods::MOD1, true), Some(WmKey::SwitchStep { forward: true }));
        assert_eq!(wm_key(15, true, mods::MOD1 | mods::SHIFT, true), Some(WmKey::SwitchStep { forward: false }));
        assert_eq!(wm_key(56, false, mods::MOD1, true), Some(WmKey::SwitchCommit));
        assert_eq!(wm_key(56, false, mods::MOD1, false), None, "Alt released without the switcher");
        assert_eq!(wm_key(1, true, mods::MOD1, true), Some(WmKey::SwitchCancel));
        assert_eq!(wm_key(15, true, 0, false), None, "plain Tab is the client's");
        // Alt+F4, Super+arrows, Super+I, Super+N.
        assert_eq!(wm_key(62, true, mods::MOD1, false), Some(WmKey::Close));
        assert_eq!(wm_key(105, true, mods::MOD4, false), Some(WmKey::Place(Arrow::Left)));
        assert_eq!(wm_key(103, true, mods::MOD4, false), Some(WmKey::Place(Arrow::Up)));
        assert_eq!(wm_key(105, true, 0, false), None);
        assert_eq!(wm_key(23, true, mods::MOD4, false), Some(WmKey::Settings));
        assert_eq!(wm_key(49, true, mods::MOD4, false), Some(WmKey::History));
    }

    #[test]
    fn modifier_bits() {
        let m = modifiers(mods::SHIFT | mods::MOD5, mods::LOCK);
        assert!(m.shift() && m.alt_gr() && m.caps_lock() && !m.control());
    }

    #[test]
    fn keymap_files_and_desktop_files() {
        let dir = std::env::temp_dir().join(format!("poly-desktop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (fd, size) = keymap_file(&dir, &US).unwrap();
        let mut buf = vec![0u8; size as usize];
        // SAFETY: reading into a buffer of the length given.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        assert_eq!(n as u32, size);
        assert!(buf.starts_with(b"xkb_keymap {"));
        assert_eq!(buf.last(), Some(&0), "NUL-terminated");
        crate::unix::close(fd);
        std::fs::write(dir.join("a.desktop"), "[Desktop Entry]\nType=Application\nName=A\nExec=/bin/a\n").unwrap();
        std::fs::write(dir.join("b.desktop"), "[Desktop Entry]\nType=Link\nName=B\n").unwrap();
        std::fs::write(dir.join("c.txt"), "x").unwrap();
        let apps = desktop_files(&dir);
        assert_eq!(apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["A"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
