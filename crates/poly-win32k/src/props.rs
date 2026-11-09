//! Window properties (`SetPropW`, `GetPropW`, `RemovePropW`, `EnumPropsW`;
//! Learn: "Window Properties"): a handle-sized value kept for a window
//! under a name or a global atom. Names are compared without regard to
//! case; an atom and a string are different keys here (Windows turns a
//! string into an atom; Chrome and the toolkits use either consistently).
//!
//! The table is one process's (`user32` holds it): properties of windows
//! another process owns are not shared, and a window's properties are
//! dropped by `user32` when the window is destroyed through it, or on a
//! later `SetPropW` that finds the window gone ([`Props::retain`]).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// What a property is called.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    /// A global atom (`MAKEINTATOM`: the low word of `lpString`, 1..=0xFFFF).
    Atom(u16),
    /// A name, folded to lower case (ASCII and the BMP letters Rust folds one to one).
    Name(Vec<u16>),
}

impl Key {
    /// The key for `SetPropW`'s `lpString`: an integer below 0x10000 is an
    /// atom (0 is not a key), anything else a pointer to a name read by `name`.
    pub fn from_raw(raw: usize, name: impl FnOnce() -> Vec<u16>) -> Option<Key> {
        if raw >> 16 == 0 {
            return (raw != 0).then_some(Key::Atom(raw as u16));
        }
        let folded = name()
            .into_iter()
            .map(|c| match char::from_u32(u32::from(c)) {
                Some(ch) => {
                    let mut l = ch.to_lowercase();
                    match (l.next(), l.next()) {
                        (Some(a), None) if (a as u32) < 0x10000 => a as u32 as u16,
                        _ => c,
                    }
                }
                None => c,
            })
            .collect();
        Some(Key::Name(folded))
    }
}

/// One process's window properties.
#[derive(Debug, Default)]
pub struct Props {
    map: BTreeMap<(u64, Key), usize>,
}

impl Props {
    pub const fn new() -> Props {
        Props { map: BTreeMap::new() }
    }

    /// Set (or replace) a window's property.
    pub fn set(&mut self, hwnd: u64, key: Key, value: usize) {
        self.map.insert((hwnd, key), value);
    }

    /// A property's value, or 0 when the window has none by that name.
    pub fn get(&self, hwnd: u64, key: &Key) -> usize {
        self.map.get(&(hwnd, key.clone())).copied().unwrap_or(0)
    }

    /// Remove a property: its value, or 0 when there was none.
    pub fn remove(&mut self, hwnd: u64, key: &Key) -> usize {
        self.map.remove(&(hwnd, key.clone())).unwrap_or(0)
    }

    /// Drop every property of a window.
    pub fn remove_window(&mut self, hwnd: u64) {
        self.map.retain(|(h, _), _| *h != hwnd);
    }

    /// Keep only the properties of windows `alive` says exist.
    pub fn retain(&mut self, mut alive: impl FnMut(u64) -> bool) {
        let mut last: Option<(u64, bool)> = None;
        self.map.retain(|(h, _), _| match last {
            Some((lh, ok)) if lh == *h => ok,
            _ => {
                let ok = alive(*h);
                last = Some((*h, ok));
                ok
            }
        });
    }

    /// A window's properties, in key order.
    pub fn of_window(&self, hwnd: u64) -> Vec<(Key, usize)> {
        self.map.iter().filter(|((h, _), _)| *h == hwnd).map(|((_, k), v)| (k.clone(), *v)).collect()
    }

    /// How many properties there are.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn name(s: &str) -> Key {
        Key::from_raw(0x1_0000, || w(s)).unwrap()
    }

    #[test]
    fn set_get_remove() {
        let mut p = Props::new();
        p.set(0x10010, name("Chrome_Foo"), 0xABCD);
        assert_eq!(p.get(0x10010, &name("chrome_foo")), 0xABCD, "names ignore case");
        assert_eq!(p.get(0x10011, &name("Chrome_Foo")), 0, "per window");
        p.set(0x10010, name("CHROME_FOO"), 7);
        assert_eq!(p.len(), 1, "a second set replaces");
        assert_eq!(p.remove(0x10010, &name("chrome_foo")), 7);
        assert_eq!(p.remove(0x10010, &name("chrome_foo")), 0);
        assert!(p.is_empty());
    }

    #[test]
    fn atoms_and_names_are_apart() {
        assert_eq!(Key::from_raw(0xC123, || panic!("an atom reads no name")), Some(Key::Atom(0xC123)));
        assert_eq!(Key::from_raw(0, || panic!()), None);
        let mut p = Props::new();
        p.set(1, Key::Atom(0xC001), 5);
        assert_eq!(p.get(1, &name("x")), 0);
        assert_eq!(p.get(1, &Key::Atom(0xC001)), 5);
    }

    #[test]
    fn window_cleanup() {
        let mut p = Props::new();
        for h in [1u64, 2, 3] {
            p.set(h, name("a"), 1);
            p.set(h, name("b"), 2);
        }
        p.remove_window(2);
        assert_eq!(p.len(), 4);
        assert_eq!(p.of_window(1).len(), 2);
        p.retain(|h| h == 3);
        assert_eq!(p.len(), 2);
        assert_eq!(p.of_window(3).len(), 2);
    }
}
