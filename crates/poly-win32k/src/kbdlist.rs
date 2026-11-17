//! The keyboard layouts a process has loaded and the layout of each of its
//! threads (`LoadKeyboardLayoutW`, `UnloadKeyboardLayout`,
//! `ActivateKeyboardLayout`, `GetKeyboardLayoutList`; ADR 0212).
//!
//! Sources: the spec sheets `LoadKeyboardLayoutW.yaml`,
//! `UnloadKeyboardLayout.yaml`, `ActivateKeyboardLayout.yaml` and their
//! visible tests' recorded results; Microsoft Learn ("Keyboard identifiers and
//! input method editors", the three reference pages); `winuser.h` for the
//! flags.
//!
//! The model: an `HKL` is `(device identifier << 16) | language identifier`
//! of the layout's KLID (the layouts with a layout id in the KLID's high word
//! get `0xF000 | layout id`). The list starts with the three layouts the
//! oracle's user has installed (the Swiss German and German ones under
//! English (UK), and the thread's layout in the middle: the desktop's). A
//! loaded layout is appended and can be unloaded again; a thread has the
//! process's layout until it activates another.

use alloc::vec::Vec;

/// `ActivateKeyboardLayout`'s `HKL_PREV` and `HKL_NEXT` (winuser.h).
pub const HKL_PREV: u32 = 0;
pub const HKL_NEXT: u32 = 1;

// `LoadKeyboardLayout` / `ActivateKeyboardLayout` flags (winuser.h).
pub const KLF_ACTIVATE: u32 = 0x1;
pub const KLF_SUBSTITUTE_OK: u32 = 0x2;
pub const KLF_REORDER: u32 = 0x8;
pub const KLF_NOTELLSHELL: u32 = 0x80;
pub const KLF_SETFORPROCESS: u32 = 0x100;

/// US English: what a KLID this table does not know loads as
/// (`LoadKeyboardLayoutW.yaml` `odd_names`).
pub const US: u32 = 0x0409_0409;

/// The user's installed layouts (`LoadKeyboardLayoutW.yaml` `substitutes`):
/// Swiss German and German under English (UK).
pub const INSTALLED_BEFORE: u32 = 0x0807_0809;
pub const INSTALLED_AFTER: u32 = 0x0407_0809;

/// The layouts whose KLID is `0000` and the language identifier: their HKL
/// repeats it (`LoadKeyboardLayoutW.yaml` `hkl_value`).
const SIMPLE: &[u16] = &[
    0x0409, 0x0407, 0x040C, 0x0807, 0x0809, 0x0410, 0x040A, 0x0C0C, 0x100C, 0x0813, 0x0401, 0x0419, 0x0411, 0x0412,
    0x0804,
];

/// KLIDs with a layout id in the high word, with their HKLs (the registry's
/// "Layout Id" of each: the HKL's high word is `0xF000` and it).
const LAYOUT_IDS: &[(u32, u32)] = &[(0x0001_0409, 0xF002_0409), (0x0002_0409, 0xF001_0409), (0x0001_040C, 0xF0CE_040C)];

/// The user's installed layouts' KLIDs and what `KLF_SUBSTITUTE_OK` makes of them.
const SUBSTITUTES: &[(u32, u32)] =
    &[(0xD001_0809, INSTALLED_BEFORE), (0x0000_0809, INSTALLED_AFTER), (0x0000_2000, 0x0407_2000)];

/// The KLID a string of 8 hexadecimal digits names, `None` for anything else.
pub fn parse_klid(s: &[u16]) -> Option<u32> {
    if s.len() != 8 {
        return None;
    }
    let mut v = 0u32;
    for &c in s {
        let d = char::from_u32(u32::from(c))?.to_digit(16)?;
        v = (v << 4) | d;
    }
    Some(v)
}

/// The `HKL` `LoadKeyboardLayoutW(klid, flags)` gives: the layout's own for a
/// known KLID, the user's installed layout for a substitute with
/// `KLF_SUBSTITUTE_OK`, and the US layout's for everything else.
pub fn klid_to_hkl(klid: Option<u32>, flags: u32) -> u32 {
    let Some(k) = klid else { return US };
    if flags & KLF_SUBSTITUTE_OK != 0
        && let Some(&(_, h)) = SUBSTITUTES.iter().find(|e| e.0 == k)
    {
        return h;
    }
    if let Some(&(_, h)) = LAYOUT_IDS.iter().find(|e| e.0 == k) {
        return h;
    }
    if k >> 16 == 0
        && let Some(&l) = SIMPLE.iter().find(|&&l| u32::from(l) == k)
    {
        return (u32::from(l) << 16) | u32::from(l);
    }
    US
}

/// The KLID `GetKeyboardLayoutNameW` gives for an `HKL`: `0000` and its
/// device identifier (`ActivateKeyboardLayout.yaml` `name`: 0x08070809 is
/// `00000807`), or the KLID of a layout with a layout id.
pub fn hkl_to_klid(hkl: u32) -> [u8; 8] {
    let value = LAYOUT_IDS.iter().find(|e| e.1 == hkl).map_or(hkl >> 16 & 0xFFFF, |e| e.0);
    let mut out = [b'0'; 8];
    for (i, o) in out.iter_mut().enumerate() {
        let d = ((value >> (28 - 4 * i)) & 0xF) as u8;
        *o = if d < 10 { b'0' + d } else { b'A' + d - 10 };
    }
    out
}

/// What an activation did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activation {
    /// The thread's layout before.
    pub previous: u32,
    /// Its layout now.
    pub current: u32,
}

impl Activation {
    /// Whether the thread's layout changed (and `WM_INPUTLANGCHANGE` is sent).
    pub fn changed(&self) -> bool {
        self.previous != self.current
    }
}

/// The layouts of a process.
#[derive(Clone, Debug)]
pub struct LayoutSet {
    /// `GetKeyboardLayoutList`'s list, in order.
    list: Vec<u32>,
    /// The layouts this process loaded (and can unload): not the installed ones.
    loaded: Vec<u32>,
    /// The desktop's layout, the one the list's middle entry shows.
    desktop: u32,
    /// The layout of a thread that has not activated another.
    default: u32,
    /// The layouts threads activated: (thread id, `HKL`).
    threads: Vec<(u32, u32)>,
}

impl LayoutSet {
    /// A process on a desktop whose layout is `desktop`.
    pub fn new(desktop: u32) -> LayoutSet {
        LayoutSet {
            list: alloc::vec![INSTALLED_BEFORE, desktop, INSTALLED_AFTER],
            loaded: Vec::new(),
            desktop,
            default: desktop,
            threads: Vec::new(),
        }
    }

    /// The desktop's layout is `hkl` now (its settings changed): the list's
    /// entry for it and, unless a program set the process's layout, the
    /// layout of the threads that did not choose one.
    pub fn set_desktop(&mut self, hkl: u32) {
        if hkl == self.desktop {
            return;
        }
        let old = self.desktop;
        self.desktop = hkl;
        if self.default == old {
            self.default = hkl;
        }
        if let Some(i) = self.list.iter().position(|&h| h == old) {
            if self.list.contains(&hkl) {
                self.list.remove(i);
            } else {
                self.list[i] = hkl;
            }
        } else if !self.list.contains(&hkl) {
            self.list.insert(1.min(self.list.len()), hkl);
        }
    }

    /// `GetKeyboardLayoutList`'s layouts.
    pub fn list(&self) -> &[u32] {
        &self.list
    }

    /// Whether `hkl` is one of the layouts of the list.
    pub fn contains(&self, hkl: u32) -> bool {
        self.list.contains(&hkl)
    }

    /// The layout of thread `tid`.
    pub fn thread_hkl(&self, tid: u32) -> u32 {
        self.threads.iter().find(|t| t.0 == tid).map_or(self.default, |t| t.1)
    }

    /// The layout of threads that chose none.
    pub fn default_hkl(&self) -> u32 {
        self.default
    }

    /// Add `hkl` to the list if it is not there; `true` if it was added (and
    /// so can be unloaded).
    pub fn load(&mut self, hkl: u32) -> bool {
        if self.list.contains(&hkl) {
            return false;
        }
        self.list.push(hkl);
        self.loaded.push(hkl);
        true
    }

    /// `UnloadKeyboardLayout(hkl)` from thread `tid`: `None` when `hkl` is not
    /// a layout this process loaded; otherwise the layout the calling thread
    /// has now if it had `hkl` (the first of the list, which `WM_INPUTLANGCHANGE`
    /// announces), or `Some(None)`.
    pub fn unload(&mut self, tid: u32, hkl: u32) -> Option<Option<u32>> {
        let i = self.loaded.iter().position(|&h| h == hkl)?;
        self.loaded.remove(i);
        self.list.retain(|&h| h != hkl);
        let first = self.list.first().copied().unwrap_or(self.desktop);
        let had = self.thread_hkl(tid) == hkl;
        self.threads.retain(|t| t.1 != hkl);
        if self.default == hkl {
            self.default = first;
        }
        if had {
            self.threads.push((tid, first));
            Some(Some(first))
        } else {
            Some(None)
        }
    }

    /// `ActivateKeyboardLayout(hkl, flags)` from thread `tid`: `None` when
    /// `hkl` is not in the list (`HKL_NEXT` and `HKL_PREV` walk the list from
    /// the thread's layout, with wrap-around).
    pub fn activate(&mut self, tid: u32, hkl: u32, flags: u32) -> Option<Activation> {
        let previous = self.thread_hkl(tid);
        let target = match hkl {
            HKL_NEXT | HKL_PREV if self.list.is_empty() => return None,
            HKL_NEXT => match self.list.iter().position(|&h| h == previous) {
                Some(i) => self.list[(i + 1) % self.list.len()],
                None => self.list[0],
            },
            HKL_PREV => match self.list.iter().position(|&h| h == previous) {
                Some(i) => self.list[(i + self.list.len() - 1) % self.list.len()],
                None => self.list[self.list.len() - 1],
            },
            h if self.list.contains(&h) => h,
            _ => return None,
        };
        if flags & KLF_REORDER != 0
            && let Some(i) = self.list.iter().position(|&h| h == target)
        {
            let h = self.list.remove(i);
            self.list.insert(0, h);
        }
        if flags & KLF_SETFORPROCESS != 0 {
            self.default = target;
            self.threads.clear();
        }
        match self.threads.iter_mut().find(|t| t.0 == tid) {
            Some(t) => t.1 = target,
            None => self.threads.push((tid, target)),
        }
        Some(Activation { previous, current: target })
    }
}

#[cfg(test)]
#[path = "kbdlist_tests.rs"]
mod tests;
