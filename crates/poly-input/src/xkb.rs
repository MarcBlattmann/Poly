//! A layout as an XKB keymap in the text format (`xkb_v1`), which a Wayland
//! compositor hands to its clients with `wl_keyboard.keymap` (ADR 0107).
//!
//! Written from the public descriptions of the format (the XKB protocol
//! specification's description of key types, modifiers and the compat
//! map; xkbcommon's documentation of the keymap text format, "The XKB
//! keymap text format, V1"; wayland.xml's `wl_keyboard.keymap_format`), not
//! from any keymap database or library: the four sections are generated
//! from Poly's own layout tables.
//!
//! - **Key codes**: `<Knnn>`, `nnn` the evdev key code, at XKB code
//!   `evdev + 8` (wayland.xml: clients add 8 to `wl_keyboard.key`'s code).
//! - **Types**: `ONE_LEVEL` (the same on every level: space, modifiers),
//!   `TWO_LEVEL` (Shift), `ALPHABETIC` (Shift or Caps Lock), `KEYPAD` (Num
//!   Lock or Shift selects the digits) and `POLY_SIX_LEVEL` for keys with
//!   AltGr symbols or a Caps Lock level of their own: the six levels of
//!   [`KeyDef`] in its order (base, Shift, Caps Lock, Caps Lock + Shift,
//!   AltGr, AltGr + Shift). Types use real modifiers only.
//! - **Modifiers**: Shift, Lock (Caps Lock), Control, Mod1 (left Alt), Mod2
//!   (Num Lock), Mod4 (the Windows keys), Mod5 (the right Alt key, always
//!   `ISO_Level3_Shift` here, so that one modifier map serves every
//!   layout). [`modifier_bit`] gives the bit of each key, which is what
//!   `wl_keyboard.modifiers` carries.
//! - **Symbols**: keysyms as hexadecimal numbers (`0x...`), so no name
//!   table is needed.

use crate::key::{Key, Modifiers};
use crate::keycode;
use crate::keysym::{Keysym, keysym};
use crate::layout::{KeyDef, Layout, Sym};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// The real modifier bits of `wl_keyboard.modifiers` (XKB's order: Shift,
/// Lock, Control, Mod1 ... Mod5).
pub mod mods {
    pub const SHIFT: u32 = 1 << 0;
    pub const LOCK: u32 = 1 << 1;
    pub const CONTROL: u32 = 1 << 2;
    pub const MOD1: u32 = 1 << 3;
    pub const MOD2: u32 = 1 << 4;
    pub const MOD4: u32 = 1 << 6;
    pub const MOD5: u32 = 1 << 7;
}

/// What a key does to the modifiers in this keymap: sets `bit` while held,
/// or (`locks`) toggles it on each press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModKey {
    pub bit: u32,
    pub locks: bool,
}

/// The modifier the evdev key `code` sets in the keymaps made here.
pub fn modifier_bit(code: u16) -> Option<ModKey> {
    let set = |bit| Some(ModKey { bit, locks: false });
    match keycode::key(code)? {
        Key::ShiftLeft | Key::ShiftRight => set(mods::SHIFT),
        Key::ControlLeft | Key::ControlRight => set(mods::CONTROL),
        Key::AltLeft => set(mods::MOD1),
        Key::AltRight => set(mods::MOD5),
        Key::MetaLeft | Key::MetaRight => set(mods::MOD4),
        Key::CapsLock => Some(ModKey { bit: mods::LOCK, locks: true }),
        Key::NumLock => Some(ModKey { bit: mods::MOD2, locks: true }),
        _ => None,
    }
}

/// The key's symbol on a level, as a keysym.
fn sym(s: Sym) -> Keysym {
    match s {
        Sym::Char(c) => Keysym::of_char(c),
        Sym::Dead(d) => Keysym::of_dead(d),
        Sym::None => Keysym::NO_SYMBOL,
    }
}

/// The type and keysyms of one key.
fn key_symbols(layout: &Layout, key: Key) -> Option<(&'static str, Vec<Keysym>)> {
    let def: &KeyDef = layout.get(key);
    if *def == KeyDef::NONE {
        if key.is_keypad() && key != Key::NumLock {
            let off = keysym(layout, key, Modifiers::NONE);
            let on = keysym(layout, key, Modifiers::NUM_LOCK);
            if off != on {
                return Some(("KEYPAD", alloc::vec![off, on]));
            }
        }
        let k = if key == Key::AltRight { Keysym::ISO_LEVEL3_SHIFT } else { keysym(layout, key, Modifiers::NONE) };
        return (k != Keysym::NO_SYMBOL).then(|| ("ONE_LEVEL", alloc::vec![k]));
    }
    let levels = [def.base, def.shift, def.caps, def.caps_shift, def.alt_gr, def.alt_gr_shift].map(sym);
    let [base, shift, caps, caps_shift, alt_gr, alt_gr_shift] = levels;
    if levels.iter().all(|&k| k == base) {
        return Some(("ONE_LEVEL", alloc::vec![base]));
    }
    if alt_gr != Keysym::NO_SYMBOL || alt_gr_shift != Keysym::NO_SYMBOL {
        return Some(("POLY_SIX_LEVEL", levels.to_vec()));
    }
    if levels[..4].iter().all(|&k| k == base) {
        return Some(("ONE_LEVEL", alloc::vec![base]));
    }
    if (caps, caps_shift) == (base, shift) {
        return Some(("TWO_LEVEL", alloc::vec![base, shift]));
    }
    if (caps, caps_shift) == (shift, base) {
        return Some(("ALPHABETIC", alloc::vec![base, shift]));
    }
    Some(("POLY_SIX_LEVEL", levels[..4].to_vec()))
}

const TYPES: &str = r#"xkb_types "poly" {
    type "ONE_LEVEL" {
        modifiers= none;
        level_name[Level1]= "Any";
    };
    type "TWO_LEVEL" {
        modifiers= Shift;
        map[Shift]= Level2;
        level_name[Level1]= "Base";
        level_name[Level2]= "Shift";
    };
    type "ALPHABETIC" {
        modifiers= Shift+Lock;
        map[Shift]= Level2;
        map[Lock]= Level2;
        level_name[Level1]= "Base";
        level_name[Level2]= "Caps";
    };
    type "KEYPAD" {
        modifiers= Shift+Mod2;
        map[Shift]= Level2;
        map[Mod2]= Level2;
        level_name[Level1]= "Base";
        level_name[Level2]= "Number";
    };
    type "POLY_SIX_LEVEL" {
        modifiers= Shift+Lock+Mod5;
        map[Shift]= Level2;
        map[Lock]= Level3;
        map[Shift+Lock]= Level4;
        map[Mod5]= Level5;
        map[Shift+Mod5]= Level6;
        map[Lock+Mod5]= Level5;
        map[Shift+Lock+Mod5]= Level6;
        level_name[Level1]= "Base";
        level_name[Level2]= "Shift";
        level_name[Level3]= "Caps";
        level_name[Level4]= "Caps Shift";
        level_name[Level5]= "AltGr";
        level_name[Level6]= "AltGr Shift";
    };
};
"#;

const COMPAT: &str = r#"xkb_compatibility "poly" {
    interpret Shift_L { action= SetMods(modifiers=Shift); };
    interpret Shift_R { action= SetMods(modifiers=Shift); };
    interpret Control_L { action= SetMods(modifiers=Control); };
    interpret Control_R { action= SetMods(modifiers=Control); };
    interpret Alt_L { action= SetMods(modifiers=Mod1); };
    interpret Super_L { action= SetMods(modifiers=Mod4); };
    interpret Super_R { action= SetMods(modifiers=Mod4); };
    interpret ISO_Level3_Shift { action= SetMods(modifiers=Mod5); };
    interpret Caps_Lock { action= LockMods(modifiers=Lock); };
    interpret Num_Lock { action= LockMods(modifiers=Mod2); };
    indicator "Caps Lock" { modifiers= Lock; };
    indicator "Num Lock" { modifiers= Mod2; };
};
"#;

/// The XKB keymap of `layout`, as text (ending in a newline; the Wayland
/// keymap file adds the terminating NUL wayland.xml asks for).
pub fn keymap(layout: &Layout) -> String {
    let mut codes = String::new();
    let mut symbols = String::new();
    let mut modmap: [(u32, &str, Vec<String>); 7] = [
        (mods::SHIFT, "Shift", Vec::new()),
        (mods::LOCK, "Lock", Vec::new()),
        (mods::CONTROL, "Control", Vec::new()),
        (mods::MOD1, "Mod1", Vec::new()),
        (mods::MOD2, "Mod2", Vec::new()),
        (mods::MOD4, "Mod4", Vec::new()),
        (mods::MOD5, "Mod5", Vec::new()),
    ];
    for key in Key::ALL {
        let code = keycode::code(key);
        let name = format!("<K{code:03}>");
        let _ = writeln!(codes, "    {name} = {};", u32::from(code) + 8);
        if let Some((ty, syms)) = key_symbols(layout, key) {
            let list: Vec<String> = syms.iter().map(|k| format!("0x{:x}", k.0)).collect();
            let _ = writeln!(symbols, "    key {name} {{ type= \"{ty}\", symbols[Group1]= [ {} ] }};", list.join(", "));
        }
        if let Some(m) = modifier_bit(code)
            && let Some(entry) = modmap.iter_mut().find(|(bit, _, _)| *bit == m.bit)
        {
            entry.2.push(name);
        }
    }
    for (_, name, keys) in &modmap {
        if !keys.is_empty() {
            let _ = writeln!(symbols, "    modifier_map {name} {{ {} }};", keys.join(", "));
        }
    }
    format!(
        "xkb_keymap {{\nxkb_keycodes \"poly\" {{\n    minimum = 8;\n    maximum = 255;\n{codes}    indicator 1 = \"Caps Lock\";\n    indicator 2 = \"Num Lock\";\n}};\n{TYPES}{COMPAT}xkb_symbols \"poly\" {{\n    name[Group1]= \"{}\";\n{symbols}}};\n}};\n",
        layout.description
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{DE, DE_CH, US};

    fn line<'a>(map: &'a str, key: &str) -> &'a str {
        map.lines().find(|l| l.trim_start().starts_with(&format!("key {key} "))).unwrap_or("")
    }

    #[test]
    fn us_keymap() {
        let m = keymap(&US);
        assert!(m.starts_with("xkb_keymap {\nxkb_keycodes \"poly\" {"));
        assert!(m.ends_with("};\n"));
        // KEY_A is 30: XKB code 38, a letter.
        assert!(m.contains("    <K030> = 38;"));
        assert_eq!(line(&m, "<K030>"), "    key <K030> { type= \"ALPHABETIC\", symbols[Group1]= [ 0x61, 0x41 ] };");
        // KEY_1: 1 and !, Caps Lock does nothing.
        assert_eq!(line(&m, "<K002>"), "    key <K002> { type= \"TWO_LEVEL\", symbols[Group1]= [ 0x31, 0x21 ] };");
        // Enter, Space, Escape.
        assert!(line(&m, "<K028>").contains("[ 0xff0d ]"));
        assert!(line(&m, "<K057>").contains("\"ONE_LEVEL\", symbols[Group1]= [ 0x20 ]"));
        assert!(line(&m, "<K001>").contains("[ 0xff1b ]"));
        // Keypad 7 (KEY_KP7 = 71): Home, or 7 with Num Lock.
        assert_eq!(line(&m, "<K071>"), "    key <K071> { type= \"KEYPAD\", symbols[Group1]= [ 0xff95, 0xffb7 ] };");
        // The right Alt key is ISO_Level3_Shift on every layout.
        assert!(line(&m, "<K100>").contains("[ 0xfe03 ]"));
        assert!(m.contains("    modifier_map Shift { <K042>, <K054> };"));
        assert!(m.contains("    modifier_map Lock { <K058> };"));
        assert!(m.contains("    modifier_map Mod5 { <K100> };"));
        assert!(m.contains("    modifier_map Mod4 { <K125>, <K126> };"));
        // Braces balance.
        assert_eq!(m.matches('{').count(), m.matches('}').count());
    }

    #[test]
    fn altgr_and_dead_keys() {
        // German: Q has @ on AltGr, so six levels; the key left of 1 is a
        // dead circumflex.
        let m = keymap(&DE);
        assert_eq!(
            line(&m, "<K016>"),
            "    key <K016> { type= \"POLY_SIX_LEVEL\", symbols[Group1]= [ 0x71, 0x51, 0x51, 0x71, 0x40, 0x0 ] };"
        );
        assert!(line(&m, "<K041>").contains("0xfe52"), "dead circumflex: {}", line(&m, "<K041>"));
        // Swiss German E: € (U+20AC) on AltGr.
        let m = keymap(&DE_CH);
        assert!(line(&m, "<K018>").contains("0x10020ac"));
    }

    #[test]
    fn modifier_bits() {
        use crate::codes::key;
        assert_eq!(modifier_bit(key::LEFTSHIFT), Some(ModKey { bit: mods::SHIFT, locks: false }));
        assert_eq!(modifier_bit(key::CAPSLOCK), Some(ModKey { bit: mods::LOCK, locks: true }));
        assert_eq!(modifier_bit(key::RIGHTALT).map(|m| m.bit), Some(mods::MOD5));
        assert_eq!(modifier_bit(key::A), None);
    }
}
