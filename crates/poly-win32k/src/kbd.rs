//! The keyboard layout: Poly's own description of the German layout (KLID
//! `00000407`) that the oracle uses, and the functions over it that
//! `user32.dll` exports (`MapVirtualKeyW`, `MapVirtualKeyExW`, `ToUnicode`,
//! `ToUnicodeEx`, `GetKeyboardLayout`) and that `TranslateMessage` needs.
//!
//! Sources: the spec sheets `MapVirtualKeyW.yaml`, `ToUnicode.yaml` and
//! `SendInput.yaml` with the visible tests' recorded results, Microsoft
//! Learn ("Keyboard Input Overview", "Virtual-Key Codes", "Keyboard Input
//! Notifications", the `ToUnicode`/`MapVirtualKeyW` reference pages), the
//! printed legends of the German (T1, DIN 2137) keyboard, and Unicode's
//! precomposed characters for the dead-key combinations. Constants are
//! `winuser.h`'s.
//!
//! The model:
//!
//! - **Scan codes** are set 1 make codes; an extended key carries `0xE0` (or,
//!   for Pause, `0xE1`) in the high byte, the form `MAPVK_VK_TO_VSC_EX` gives
//!   and `MAPVK_VSC_TO_VK(_EX)` accepts.
//! - **Characters** come from the virtual key alone (the scan code only says
//!   whether this is a key-up): each character key has a character for no
//!   modifier, Shift, Ctrl, Ctrl+Shift and AltGr (Ctrl+Alt), and says whether
//!   Caps Lock acts as Shift on it. Alt alone gives the unmodified character.
//! - **Dead keys** (`^` on `VK_OEM_5`, acute and, with Shift, grave on
//!   `VK_OEM_6`) give their spacing character and stay pending in a
//!   [`DeadState`] until the next character key, which combines with them
//!   or, if it does not, comes after the accent.

/// The oracle's keyboard layout handle: the German layout (KLID
/// `00000407`) under the input language English (Switzerland), which has
/// no LCID (`0x2000`, "transient" locale), as `GetKeyboardLayout` returns
/// it (`MapVirtualKeyW.yaml`: low 32 bits `0x04072000`).
pub const HKL: u32 = 0x0407_2000;

/// The layout's KLID (the name `GetKeyboardLayoutNameW` gives).
pub const KLID: &str = "00000407";

// `MapVirtualKey` map types (winuser.h).
pub const MAPVK_VK_TO_VSC: u32 = 0;
pub const MAPVK_VSC_TO_VK: u32 = 1;
pub const MAPVK_VK_TO_CHAR: u32 = 2;
pub const MAPVK_VSC_TO_VK_EX: u32 = 3;
pub const MAPVK_VK_TO_VSC_EX: u32 = 4;

/// `ERROR_INVALID_PARAMETER` (winerror.h).
pub const ERROR_INVALID_PARAMETER: u32 = 87;

// Window messages (winuser.h).
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_DEADCHAR: u32 = 0x0103;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_SYSCHAR: u32 = 0x0106;
pub const WM_SYSDEADCHAR: u32 = 0x0107;

// Virtual keys (winuser.h).
pub const VK_CANCEL: u32 = 0x03;
pub const VK_BACK: u32 = 0x08;
pub const VK_TAB: u32 = 0x09;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12;
pub const VK_PAUSE: u32 = 0x13;
pub const VK_CAPITAL: u32 = 0x14;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;
pub const VK_APPS: u32 = 0x5D;
pub const VK_NUMPAD0: u32 = 0x60;
pub const VK_MULTIPLY: u32 = 0x6A;
pub const VK_ADD: u32 = 0x6B;
pub const VK_SUBTRACT: u32 = 0x6D;
pub const VK_DECIMAL: u32 = 0x6E;
pub const VK_DIVIDE: u32 = 0x6F;
pub const VK_LSHIFT: u32 = 0xA0;
pub const VK_RSHIFT: u32 = 0xA1;
pub const VK_LCONTROL: u32 = 0xA2;
pub const VK_RCONTROL: u32 = 0xA3;
pub const VK_LMENU: u32 = 0xA4;
pub const VK_RMENU: u32 = 0xA5;
pub const VK_OEM_1: u32 = 0xBA;
pub const VK_OEM_PLUS: u32 = 0xBB;
pub const VK_OEM_COMMA: u32 = 0xBC;
pub const VK_OEM_MINUS: u32 = 0xBD;
pub const VK_OEM_PERIOD: u32 = 0xBE;
pub const VK_OEM_2: u32 = 0xBF;
pub const VK_OEM_3: u32 = 0xC0;
pub const VK_OEM_4: u32 = 0xDB;
pub const VK_OEM_5: u32 = 0xDC;
pub const VK_OEM_6: u32 = 0xDD;
pub const VK_OEM_7: u32 = 0xDE;
pub const VK_OEM_102: u32 = 0xE2;
pub const VK_PACKET: u32 = 0xE7;

/// `ToUnicode`'s flags bit 2: the thread's keyboard state (the pending
/// dead key) is not changed (Learn, `ToUnicode.yaml`).
pub const TU_NO_STATE_CHANGE: u32 = 4;

/// `MapVirtualKey(MAPVK_VK_TO_CHAR)`'s mark of a dead key (Learn: "Dead
/// keys (diacritics) are indicated by setting the top bit").
pub const DEAD_KEY_BIT: u32 = 0x8000_0000;

/// Bit 25 of the `lParam` of the `WM_CHAR` that carries a dead key's
/// accent when the next key did not combine with it (`SendInput.yaml`:
/// `^` then `x` gives `0x5E` with `0x022D0001`, then `x` with `0x002D0001`).
pub const LPARAM_UNCOMBINED_DEAD: u64 = 1 << 25;

/// Scan code (set 1, no prefix) to virtual key, as `MAPVK_VSC_TO_VK`
/// observed it for 0x00..0x7F (`MapVirtualKeyW.yaml`, `mvk_scan`).
#[rustfmt::skip]
const SCAN_TO_VK: [u8; 0x80] = [
    // 0x00
    0x00, 0x1B, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36,
    0x37, 0x38, 0x39, 0x30, 0xDB, 0xDD, 0x08, 0x09,
    // 0x10: Q W E R T Z U I, O P Ü + Enter Ctrl A S
    0x51, 0x57, 0x45, 0x52, 0x54, 0x5A, 0x55, 0x49,
    0x4F, 0x50, 0xBA, 0xBB, 0x0D, 0x11, 0x41, 0x53,
    // 0x20: D F G H J K L Ö, Ä ^ LShift # Y X C V
    0x44, 0x46, 0x47, 0x48, 0x4A, 0x4B, 0x4C, 0xC0,
    0xDE, 0xDC, 0x10, 0xBF, 0x59, 0x58, 0x43, 0x56,
    // 0x30: B N M , . - RShift *, Alt Space Caps F1..F5
    0x42, 0x4E, 0x4D, 0xBC, 0xBE, 0xBD, 0x10, 0x6A,
    0x12, 0x20, 0x14, 0x70, 0x71, 0x72, 0x73, 0x74,
    // 0x40: F6..F10 NumLock Scroll Home, Up PgUp - Left Clear Right + End
    0x75, 0x76, 0x77, 0x78, 0x79, 0x90, 0x91, 0x24,
    0x26, 0x21, 0x6D, 0x25, 0x0C, 0x27, 0x6B, 0x23,
    // 0x50: Down PgDn Ins Del SysRq - <> F11, F12 ...
    0x28, 0x22, 0x2D, 0x2E, 0x2C, 0x00, 0xE2, 0x7A,
    0x7B, 0x0C, 0xEE, 0xF1, 0xEA, 0xF9, 0xF5, 0xF3,
    // 0x60: ... F13..F23 at 0x64..0x6E
    0x00, 0x00, 0xFB, 0x2F, 0x7C, 0x7D, 0x7E, 0x7F,
    0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0xED,
    // 0x70: ... F24 at 0x76
    0x00, 0xE9, 0x00, 0xC1, 0x00, 0x00, 0x87, 0x00,
    0x00, 0x00, 0x00, 0xEB, 0x09, 0x00, 0xC2, 0x00,
];

/// The virtual key of a scan code, `0xE0`/`0xE1` prefix in the high byte;
/// `ex` distinguishes left and right Shift, Ctrl and Alt
/// (`MAPVK_VSC_TO_VK_EX`). 0 for none.
fn scan_to_vk(scan: u32, ex: bool) -> u32 {
    let low = scan & 0xFF;
    match scan >> 8 {
        0 => {
            if low >= 0x80 {
                return 0;
            }
            match (low, ex) {
                (0x1D, true) => VK_LCONTROL,
                (0x2A, true) => VK_LSHIFT,
                (0x36, true) => VK_RSHIFT,
                (0x38, true) => VK_LMENU,
                _ => u32::from(SCAN_TO_VK[low as usize]),
            }
        }
        0xE0 => match low {
            0x1C => VK_RETURN,
            0x1D => {
                if ex {
                    VK_RCONTROL
                } else {
                    VK_CONTROL
                }
            }
            0x35 => VK_DIVIDE,
            0x37 => 0x2C, // VK_SNAPSHOT
            0x38 => {
                if ex {
                    VK_RMENU
                } else {
                    VK_MENU
                }
            }
            0x46 => VK_CANCEL, // Ctrl+Break
            // The navigation block, as without the prefix.
            0x47..=0x53 => u32::from(SCAN_TO_VK[low as usize]),
            0x5B => VK_LWIN,
            0x5C => VK_RWIN,
            0x5D => VK_APPS,
            _ => 0,
        },
        0xE1 if low == 0x1D => VK_PAUSE,
        _ => 0,
    }
}

/// The scan code of a virtual key; `ex` adds the `0xE0`/`0xE1` prefix of
/// the extended keys (`MAPVK_VK_TO_VSC_EX`). 0 for none.
fn vk_to_scan(vk: u32, ex: bool) -> u32 {
    let (scan, prefix) = match vk {
        0 | 0xFF => (0, 0),
        VK_LSHIFT => (0x2A, 0),
        VK_RSHIFT => (0x36, 0),
        VK_LCONTROL => (0x1D, 0),
        VK_RCONTROL => (0x1D, 0xE0),
        VK_LMENU => (0x38, 0),
        VK_RMENU => (0x38, 0xE0),
        VK_CANCEL => (0x46, 0xE0),
        VK_PAUSE => (if ex { 0x1D } else { 0 }, 0xE1),
        VK_LWIN => (0x5B, 0xE0),
        VK_RWIN => (0x5C, 0xE0),
        VK_APPS => (0x5D, 0xE0),
        VK_DIVIDE => (0x35, 0xE0),
        VK_DECIMAL => (0x53, 0),
        // The numpad digits share the navigation keys' scan codes.
        0x60..=0x69 => ([0x52, 0x4F, 0x50, 0x51, 0x4B, 0x4C, 0x4D, 0x47, 0x48, 0x49][(vk - VK_NUMPAD0) as usize], 0),
        _ => (SCAN_TO_VK.iter().skip(1).position(|&v| u32::from(v) == vk).map_or(0, |i| i as u32 + 1), 0),
    };
    if ex && scan != 0 { scan | (prefix << 8) } else { scan }
}

/// "No character" in the tables below (U+FFFF is a noncharacter).
const NO: u32 = 0xFFFF;
/// A dead key's spacing character in the tables below.
const D: u32 = DEAD_KEY_BIT;

/// One character key: its characters for no modifier, Shift, Ctrl,
/// Ctrl+Shift and AltGr, whether Caps Lock acts as Shift on it, and whether
/// Alt alone gives nothing (the numpad digits) instead of the unmodified
/// character.
#[derive(Clone, Copy, Debug)]
struct Key {
    vk: u8,
    chars: [u32; 5],
    caps: bool,
    alt_none: bool,
}

const fn k(vk: u32, chars: [u32; 5], caps: bool) -> Key {
    Key { vk: vk as u8, chars, caps, alt_none: false }
}

/// The German layout's keys other than the letters (`letter_key`).
/// Columns: none, Shift, Ctrl, Ctrl+Shift, AltGr (`ToUnicode.yaml`).
#[rustfmt::skip]
const KEYS: &[Key] = &[
    // The digit row: Caps Lock acts as Shift; AltGr per the legends.
    k(0x31, ['1' as u32, '!' as u32, NO, NO, NO], true),
    k(0x32, ['2' as u32, '"' as u32, NO, 0x0000, 0xB2], true),
    k(0x33, ['3' as u32, 0xA7, NO, NO, 0xB3], true),
    k(0x34, ['4' as u32, '$' as u32, NO, NO, NO], true),
    k(0x35, ['5' as u32, '%' as u32, NO, NO, NO], true),
    k(0x36, ['6' as u32, '&' as u32, NO, NO, NO], true),
    k(0x37, ['7' as u32, '/' as u32, NO, NO, '{' as u32], true),
    k(0x38, ['8' as u32, '(' as u32, NO, NO, '[' as u32], true),
    k(0x39, ['9' as u32, ')' as u32, NO, NO, ']' as u32], true),
    k(0x30, ['0' as u32, '=' as u32, NO, NO, '}' as u32], true),
    k(VK_OEM_4, [0xDF, '?' as u32, NO, NO, '\\' as u32], true),
    k(VK_OEM_6, [D | 0xB4, D | 0x60, NO, NO, NO], false),
    // The letter rows' other keys.
    k(VK_OEM_1, [0xFC, 0xDC, 0x1B, NO, NO], true),
    k(VK_OEM_PLUS, ['+' as u32, '*' as u32, 0x1D, NO, '~' as u32], true),
    k(VK_OEM_3, [0xF6, 0xD6, NO, NO, NO], true),
    k(VK_OEM_7, [0xE4, 0xC4, NO, NO, NO], true),
    k(VK_OEM_2, ['#' as u32, '\'' as u32, 0x1C, NO, NO], true),
    k(VK_OEM_5, [D | 0x5E, 0xB0, NO, NO, NO], false),
    k(VK_OEM_102, ['<' as u32, '>' as u32, NO, NO, '|' as u32], false),
    k(VK_OEM_COMMA, [',' as u32, ';' as u32, NO, NO, NO], true),
    k(VK_OEM_PERIOD, ['.' as u32, ':' as u32, NO, NO, NO], true),
    k(VK_OEM_MINUS, ['-' as u32, '_' as u32, NO, 0x1F, NO], false),
    // Control keys.
    k(VK_SPACE, [0x20, 0x20, 0x20, NO, NO], false),
    k(VK_RETURN, [0x0D, 0x0D, 0x0A, NO, NO], false),
    k(VK_BACK, [0x08, 0x08, 0x7F, NO, NO], false),
    k(VK_TAB, [0x09, 0x09, NO, NO, NO], false),
    k(VK_ESCAPE, [0x1B, 0x1B, 0x1B, NO, NO], false),
    k(VK_CANCEL, [0x03, 0x03, 0x03, NO, NO], false),
    // The numpad's operators and decimal key (',' on the German layout).
    k(VK_MULTIPLY, ['*' as u32, '*' as u32, NO, NO, NO], false),
    k(VK_ADD, ['+' as u32, '+' as u32, NO, NO, NO], false),
    k(VK_SUBTRACT, ['-' as u32, '-' as u32, NO, NO, NO], false),
    k(VK_DECIMAL, [',' as u32, ',' as u32, NO, NO, NO], false),
    k(VK_DIVIDE, ['/' as u32, '/' as u32, NO, NO, NO], false),
];

/// AltGr on the letter keys (the legends): `@` on Q, `€` on E, `µ` on M.
fn letter_altgr(vk: u32) -> u32 {
    match vk {
        0x51 => '@' as u32,
        0x45 => 0x20AC,
        0x4D => 0xB5,
        _ => NO,
    }
}

/// The key description of `vk`, if it makes characters.
fn key(vk: u32) -> Option<Key> {
    match vk {
        // Letters: lower and upper case, Ctrl gives the control character.
        0x41..=0x5A => Some(Key {
            vk: vk as u8,
            chars: [vk + 0x20, vk, vk - 0x40, vk - 0x40, letter_altgr(vk)],
            caps: true,
            alt_none: false,
        }),
        // Numpad digits: only with no modifier (or Caps Lock).
        0x60..=0x69 => Some(Key {
            vk: vk as u8,
            chars: [vk - VK_NUMPAD0 + '0' as u32, NO, NO, NO, NO],
            caps: false,
            alt_none: true,
        }),
        _ => KEYS.iter().find(|k| u32::from(k.vk) == vk).copied(),
    }
}

/// The modifiers `ToUnicode` reads from a key-state array: the high (down)
/// bit of `VK_SHIFT`, `VK_CONTROL` and `VK_MENU`, the low (toggle) bit of
/// `VK_CAPITAL`.
fn modifiers(key_state: &[u8; 256]) -> (bool, bool, bool, bool) {
    let down = |vk: u32| key_state[vk as usize] & 0x80 != 0;
    (down(VK_SHIFT), down(VK_CONTROL), down(VK_MENU), key_state[VK_CAPITAL as usize] & 1 != 0)
}

/// The table entry (character, `D|` for a dead key, or `NO`) of `vk` with
/// these modifiers.
fn key_entry(vk: u32, key_state: &[u8; 256]) -> u32 {
    let Some(key) = key(vk) else { return NO };
    let (shift, ctrl, alt, caps) = modifiers(key_state);
    match (ctrl, alt) {
        // AltGr is Ctrl+Alt; Shift+AltGr gives nothing on this layout.
        (true, true) => {
            if shift {
                NO
            } else {
                key.chars[4]
            }
        }
        (true, false) => key.chars[if shift { 3 } else { 2 }],
        (false, true) if key.alt_none => NO,
        // Alt alone gives the unmodified character.
        _ => key.chars[usize::from(shift ^ (caps && key.caps))],
    }
}

/// A dead key's combinations with the next character (Unicode's
/// precomposed characters); Space gives the accent alone.
fn combine(dead: u16, c: u16) -> Option<u16> {
    // (base letters, their combinations) per accent, same order.
    const BASES: &[u16] = &[0x61, 0x65, 0x69, 0x6F, 0x75, 0x41, 0x45, 0x49, 0x4F, 0x55, 0x79, 0x59];
    const CIRCUMFLEX: &[u16] = &[0xE2, 0xEA, 0xEE, 0xF4, 0xFB, 0xC2, 0xCA, 0xCE, 0xD4, 0xDB];
    const ACUTE: &[u16] = &[0xE1, 0xE9, 0xED, 0xF3, 0xFA, 0xC1, 0xC9, 0xCD, 0xD3, 0xDA, 0xFD, 0xDD];
    const GRAVE: &[u16] = &[0xE0, 0xE8, 0xEC, 0xF2, 0xF9, 0xC0, 0xC8, 0xCC, 0xD2, 0xD9];
    let table = match dead {
        0x5E => CIRCUMFLEX,
        0xB4 => ACUTE,
        0x60 => GRAVE,
        _ => return None,
    };
    if c == 0x20 {
        return Some(dead);
    }
    let i = BASES.iter().position(|&b| b == c)?;
    table.get(i).copied()
}

/// The dead key a thread has pending (its keyboard state between
/// `ToUnicode` calls and `TranslateMessage`s). Windows keeps it per thread
/// (per input queue); the caller stores one per thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeadState {
    pending: Option<u16>,
}

impl DeadState {
    /// The pending dead key's spacing character, if any.
    pub fn pending(&self) -> Option<u16> {
        self.pending
    }

    /// Forget the pending dead key.
    pub fn clear(&mut self) {
        self.pending = None;
    }
}

/// What one key-down makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// No character (key-up, modifier, navigation key, a state without a
    /// character); a pending dead key stays pending.
    Nothing,
    /// A dead key with nothing pending: its spacing character.
    Dead(u16),
    /// One or two characters (two: a pending accent that did not combine,
    /// then the key's own character); nothing is pending afterwards.
    Chars([u16; 2], usize),
}

fn outcome(vk: u32, scan: u32, key_state: &[u8; 256], pending: Option<u16>) -> Outcome {
    if scan & 0x8000 != 0 || vk == 0 || vk > 0xFE {
        return Outcome::Nothing;
    }
    let e = key_entry(vk, key_state);
    if e == NO {
        return Outcome::Nothing;
    }
    let c = (e & 0xFFFF) as u16;
    let dead = e & D != 0;
    match pending {
        None if dead => Outcome::Dead(c),
        None => Outcome::Chars([c, 0], 1),
        Some(p) => match if dead { None } else { combine(p, c) } {
            Some(x) => Outcome::Chars([x, 0], 1),
            None => Outcome::Chars([p, c], 2),
        },
    }
}

/// `MapVirtualKeyW(code, map_type)` / `MapVirtualKeyExW` with the German
/// layout: `Err(ERROR_INVALID_PARAMETER)` for an unknown map type and for a
/// virtual key above 0xFF (the caller sets the last error and returns 0);
/// `Ok(0)` when there is no translation (`MapVirtualKeyW.yaml`).
pub fn try_map_virtual_key(code: u32, map_type: u32) -> Result<u32, u32> {
    match map_type {
        MAPVK_VK_TO_VSC | MAPVK_VK_TO_CHAR | MAPVK_VK_TO_VSC_EX if code > 0xFF => Err(ERROR_INVALID_PARAMETER),
        MAPVK_VK_TO_VSC => Ok(vk_to_scan(code, false)),
        MAPVK_VK_TO_VSC_EX => Ok(vk_to_scan(code, true)),
        MAPVK_VSC_TO_VK => Ok(scan_to_vk(code, false)),
        MAPVK_VSC_TO_VK_EX => Ok(scan_to_vk(code, true)),
        MAPVK_VK_TO_CHAR => Ok(match code {
            0 => 0,
            // Observed on the oracle (`MapVirtualKeyW.yaml`: "0xFF gives 0x5E").
            0xFF => 0x5E,
            0x41..=0x5A => code,
            _ => match key(code).map(|k| k.chars[0]) {
                Some(e) if e != NO => e,
                _ => 0,
            },
        }),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

/// `MapVirtualKeyW(code, map_type)`: the translated code, 0 for none or on
/// error (see [`try_map_virtual_key`] for the error).
pub fn map_virtual_key(code: u32, map_type: u32) -> u32 {
    try_map_virtual_key(code, map_type).unwrap_or(0)
}

/// `ToUnicode(vk, scan, key_state, buf, buf.len(), flags)` /
/// `ToUnicodeEx` with the German layout and the calling thread's `dead`
/// state (`ToUnicode.yaml`):
///
/// - returns the number of UTF-16 units written (at most `buf.len()`), 0
///   for no character, -1 for a dead key (its spacing character in
///   `buf[0]`, and it becomes pending unless `flags` has
///   [`TU_NO_STATE_CHANGE`]);
/// - a pending dead key combines with the next character key, or (no
///   combination) the accent comes first and the key's character second;
///   with room for one unit only the accent is written and the key's
///   character is lost; key-ups (`scan & 0x8000`) and keys without a
///   character return 0 and leave it pending;
/// - the units of `buf` after the result are zeroed;
/// - an empty `buf` returns 0 and changes nothing (the `user32` wrapper
///   writes the NUL Windows puts at unit 0 even then, if the pointer is
///   not NULL).
pub fn to_unicode(vk: u32, scan: u32, key_state: &[u8; 256], buf: &mut [u16], flags: u32, dead: &mut DeadState) -> i32 {
    if buf.is_empty() {
        return 0;
    }
    buf.fill(0);
    let keep = flags & TU_NO_STATE_CHANGE != 0;
    match outcome(vk, scan, key_state, dead.pending) {
        Outcome::Nothing => 0,
        Outcome::Dead(c) => {
            buf[0] = c;
            if !keep {
                dead.pending = Some(c);
            }
            -1
        }
        Outcome::Chars(units, n) => {
            let n = n.min(buf.len());
            buf[..n].copy_from_slice(&units[..n]);
            if !keep {
                dead.pending = None;
            }
            n as i32
        }
    }
}

/// A character message `TranslateMessage` posts: to the key message's
/// window, `msg` with `wparam` (the UTF-16 unit) and `lparam`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharMsg {
    pub msg: u32,
    pub wparam: u64,
    pub lparam: u64,
}

/// The (zero, one or two) character messages of one key message, in
/// posting order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharMsgs {
    msgs: [Option<CharMsg>; 2],
}

impl CharMsgs {
    /// The messages, in the order to post them.
    pub fn iter(&self) -> impl Iterator<Item = CharMsg> + '_ {
        self.msgs.iter().flatten().copied()
    }

    pub fn len(&self) -> usize {
        self.iter().count()
    }

    pub fn is_empty(&self) -> bool {
        self.msgs[0].is_none()
    }
}

/// `TranslateMessage`'s keyboard part: the character messages a
/// `WM_KEYDOWN` / `WM_SYSKEYDOWN` makes with the German layout, given the
/// message (`message`, `wparam` = virtual key, `lparam` = repeat count,
/// scan code in bits 16-23, extended bit 24, context bit 29, previous
/// state bit 30), the thread's key state (what `GetKeyboardState` gives:
/// the high bit of `VK_SHIFT`/`VK_CONTROL`/`VK_MENU`, the low bit of
/// `VK_CAPITAL` are read) and the thread's dead-key state, which it updates
/// (the same state `ToUnicode` uses: Windows keeps one per thread).
///
/// - A character: one `WM_CHAR` (`WM_SYSCHAR` for `WM_SYSKEYDOWN`) with the
///   key message's `lParam`.
/// - A dead key: one `WM_DEADCHAR` (`WM_SYSDEADCHAR`) with the spacing
///   accent; it becomes pending.
/// - A key after a dead key: the combined character in one `WM_CHAR`, or,
///   when they do not combine, two `WM_CHAR`s: the accent with
///   [`LPARAM_UNCOMBINED_DEAD`] (bit 25) added to the `lParam`, then the
///   key's own character (`SendInput.yaml` `dead_keys`).
/// - Nothing for keys without a character (`VK_LEFT`, F1, `VK_SHIFT`: a
///   pending dead key stays pending), for other messages, and for
///   `VK_PACKET`: its UTF-16 unit is not in the message (`wParam` is exactly
///   `VK_PACKET`), so the input path posts its `WM_CHAR` itself.
///
/// `TranslateMessage` posts the returned messages in order, to the key
/// message's window, and returns nonzero for any `WM_KEYDOWN`/`WM_KEYUP`/
/// `WM_SYSKEYDOWN`/`WM_SYSKEYUP` whether or not something was posted.
pub fn translate_key(message: u32, wparam: u64, lparam: u64, key_state: &[u8; 256], dead: &mut DeadState) -> CharMsgs {
    let (char_msg, dead_msg) = match message {
        WM_KEYDOWN => (WM_CHAR, WM_DEADCHAR),
        WM_SYSKEYDOWN => (WM_SYSCHAR, WM_SYSDEADCHAR),
        _ => return CharMsgs::default(),
    };
    if wparam > 0xFF || wparam as u32 == VK_PACKET {
        return CharMsgs::default();
    }
    // A key-down's transition bit (31) is 0, so this is never a key-up.
    let scan = ((lparam >> 16) & 0xFF) as u32;
    let msg = |msg, c: u16, lparam| Some(CharMsg { msg, wparam: u64::from(c), lparam });
    match outcome(wparam as u32, scan, key_state, dead.pending) {
        Outcome::Nothing => CharMsgs::default(),
        Outcome::Dead(c) => {
            dead.pending = Some(c);
            CharMsgs { msgs: [msg(dead_msg, c, lparam), None] }
        }
        Outcome::Chars(u, n) => {
            dead.pending = None;
            if n == 1 {
                CharMsgs { msgs: [msg(char_msg, u[0], lparam), None] }
            } else {
                CharMsgs { msgs: [msg(char_msg, u[0], lparam | LPARAM_UNCOMBINED_DEAD), msg(char_msg, u[1], lparam)] }
            }
        }
    }
}

#[cfg(test)]
#[path = "kbd_tests.rs"]
mod tests;
