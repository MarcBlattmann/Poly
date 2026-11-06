//! Input for the program: keys, pasted text, mouse and focus events as the
//! bytes an `xterm-256color` program expects.
//!
//! | Key | Bytes |
//! |---|---|
//! | characters | UTF-8; with Alt, `ESC` first |
//! | Ctrl + letter, `@`/Space, `[ \ ] ^ _`, `?` | the C0 control (`a` = 0x01, Space = NUL, `[` = ESC, `?` = DEL); also Ctrl+`2`..`8` as xterm |
//! | Enter | CR (CR LF in newline mode); keypad Enter `ESC O M` in keypad application mode |
//! | Tab, Shift+Tab | HT, `CSI Z` |
//! | Backspace, Ctrl+Backspace | DEL, BS |
//! | Escape | ESC |
//! | Up, Down, Right, Left, Home, End | `CSI A` .. `D`, `CSI H`, `CSI F`; `SS3 A` ... in cursor application mode |
//! | Insert, Delete, Page Up, Page Down | `CSI 2~`, `3~`, `5~`, `6~` |
//! | F1-F4 | `SS3 P` .. `S` |
//! | F5-F12 | `CSI 15~`, `17~`, `18~`, `19~`, `20~`, `21~`, `23~`, `24~` |
//!
//! With Shift, Alt or Ctrl, the cursor keys, Home, End and F1-F4 send
//! `CSI 1 ; m X` and the `~` keys `CSI n ; m ~`, where `m` is 1 + 1 for
//! Shift + 2 for Alt + 4 for Ctrl (xterm's "PC-style" modifiers). Alt with
//! Enter, Tab, Backspace and Escape sends `ESC` first.

use crate::terminal::{Modes, MouseMode};
use alloc::format;
use alloc::vec::Vec;

/// A key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A character the keyboard layout produced (already shifted).
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 to F12.
    F(u8),
    /// The keypad's Enter.
    KpEnter,
}

/// Modifiers held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyMods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl KeyMods {
    /// xterm's modifier parameter; 1 is none.
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

/// The bytes `key` with `mods` sends in `modes` (empty for none).
pub fn encode(key: Key, mods: KeyMods, modes: &Modes) -> Vec<u8> {
    let m = mods.param();
    let mut out = Vec::new();
    let alt_prefix = |out: &mut Vec<u8>| {
        if mods.alt {
            out.push(0x1b);
        }
    };
    match key {
        Key::Char(c) => {
            alt_prefix(&mut out);
            match ctrl_char(c).filter(|_| mods.ctrl) {
                Some(b) => out.push(b),
                None => {
                    let mut buf = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
            }
        }
        Key::Enter => {
            alt_prefix(&mut out);
            out.extend_from_slice(if modes.newline { b"\r\n" } else { b"\r" });
        }
        Key::KpEnter if modes.app_keypad => out.extend_from_slice(b"\x1bOM"),
        Key::KpEnter => out.extend_from_slice(if modes.newline { b"\r\n" } else { b"\r" }),
        Key::Tab if mods.shift => out.extend_from_slice(b"\x1b[Z"),
        Key::Tab => {
            alt_prefix(&mut out);
            out.push(b'\t');
        }
        Key::Backspace => {
            alt_prefix(&mut out);
            out.push(if mods.ctrl { 0x08 } else { 0x7f });
        }
        Key::Escape => {
            alt_prefix(&mut out);
            out.push(0x1b);
        }
        Key::Up => letter(&mut out, b'A', m, modes.app_cursor),
        Key::Down => letter(&mut out, b'B', m, modes.app_cursor),
        Key::Right => letter(&mut out, b'C', m, modes.app_cursor),
        Key::Left => letter(&mut out, b'D', m, modes.app_cursor),
        Key::Home => letter(&mut out, b'H', m, modes.app_cursor),
        Key::End => letter(&mut out, b'F', m, modes.app_cursor),
        Key::Insert => tilde(&mut out, 2, m),
        Key::Delete => tilde(&mut out, 3, m),
        Key::PageUp => tilde(&mut out, 5, m),
        Key::PageDown => tilde(&mut out, 6, m),
        Key::F(n @ 1..=4) => letter(&mut out, b'P' + (n - 1), m, true),
        Key::F(n @ 5..=12) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde(&mut out, CODES[usize::from(n - 5)], m);
        }
        Key::F(_) => {}
    }
    out
}

/// What Ctrl + `c` sends, if it is a control character.
fn ctrl_char(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' | '~' => 0x1e,
        '_' | '7' | '/' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// `CSI X` / `SS3 X` (application mode), or `CSI 1 ; m X` with modifiers.
fn letter(out: &mut Vec<u8>, last: u8, m: u8, ss3: bool) {
    if m != 1 {
        out.extend_from_slice(format!("\x1b[1;{m}").as_bytes());
    } else if ss3 {
        out.extend_from_slice(b"\x1bO");
    } else {
        out.extend_from_slice(b"\x1b[");
    }
    out.push(last);
}

/// `CSI n ~`, or `CSI n ; m ~` with modifiers.
fn tilde(out: &mut Vec<u8>, n: u8, m: u8) {
    if m == 1 {
        out.extend_from_slice(format!("\x1b[{n}~").as_bytes());
    } else {
        out.extend_from_slice(format!("\x1b[{n};{m}~").as_bytes());
    }
}

/// Pasted text as the program should get it: line ends become CR (as if
/// typed), ESC and other C0 controls except HT are removed (so the text
/// cannot end the bracket or inject sequences), and with bracketed paste
/// on it comes between `CSI 200~` and `CSI 201~`.
pub fn paste(text: &str, modes: &Modes) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 12);
    if modes.bracketed_paste {
        out.extend_from_slice(b"\x1b[200~");
    }
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(b'\r');
            }
            '\n' => out.push(b'\r'),
            '\t' => out.push(b'\t'),
            '\0'..='\x1f' | '\x7f' | '\u{80}'..='\u{9f}' => {}
            c => {
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    if modes.bracketed_paste {
        out.extend_from_slice(b"\x1b[201~");
    }
    out
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

impl MouseButton {
    fn code(self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
            MouseButton::WheelLeft => 66,
            MouseButton::WheelRight => 67,
        }
    }

    fn is_wheel(self) -> bool {
        self.code() >= 64
    }
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseAction {
    Press(MouseButton),
    Release(MouseButton),
    /// Moved to a new cell, with this button held (or none).
    Motion(Option<MouseButton>),
}

/// A mouse event on cell (`col`, `row`), 0-based display coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub col: u16,
    pub row: u16,
    pub mods: KeyMods,
}

/// The report of `ev` for the program, or nothing if its mouse mode does
/// not ask for this event (or, in the old X10 format, the cell is past
/// column or row 223). Wheel scrolling is reported as presses of buttons
/// 64-67 and has no release.
pub fn mouse(ev: MouseEvent, modes: &Modes) -> Vec<u8> {
    let wanted = match (modes.mouse, ev.action) {
        (MouseMode::None, _) => false,
        (_, MouseAction::Press(_)) => true,
        (_, MouseAction::Release(b)) => !b.is_wheel(),
        (MouseMode::Drag, MouseAction::Motion(held)) => held.is_some(),
        (MouseMode::Any, MouseAction::Motion(_)) => true,
        (MouseMode::Press, MouseAction::Motion(_)) => false,
    };
    if !wanted {
        return Vec::new();
    }
    let mods = 4 * u8::from(ev.mods.shift) + 8 * u8::from(ev.mods.alt) + 16 * u8::from(ev.mods.ctrl);
    let (code, release) = match ev.action {
        MouseAction::Press(b) => (b.code(), false),
        MouseAction::Release(b) => (b.code(), true),
        MouseAction::Motion(held) => (32 + held.map_or(3, |b| b.code()), false),
    };
    let (x, y) = (u32::from(ev.col) + 1, u32::from(ev.row) + 1);
    if modes.sgr_mouse {
        let end = if release { 'm' } else { 'M' };
        format!("\x1b[<{};{x};{y}{end}", code + mods).into_bytes()
    } else {
        // X10 / normal format: release is button 3; values offset by 32.
        let code = if release { 3 } else { code };
        if x > 223 || y > 223 {
            return Vec::new();
        }
        let mut out = Vec::from(*b"\x1b[M");
        out.extend_from_slice(&[32 + code + mods, 32 + x as u8, 32 + y as u8]);
        out
    }
}

/// The focus report (`CSI I` / `CSI O`) if the program asked for them.
pub fn focus(focused: bool, modes: &Modes) -> Vec<u8> {
    match (modes.focus_events, focused) {
        (false, _) => Vec::new(),
        (true, true) => Vec::from(*b"\x1b[I"),
        (true, false) => Vec::from(*b"\x1b[O"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyMods = KeyMods { shift: false, ctrl: false, alt: false };
    const SHIFT: KeyMods = KeyMods { shift: true, ctrl: false, alt: false };
    const CTRL: KeyMods = KeyMods { shift: false, ctrl: true, alt: false };
    const ALT: KeyMods = KeyMods { shift: false, ctrl: false, alt: true };

    fn enc(key: Key, mods: KeyMods) -> Vec<u8> {
        encode(key, mods, &Modes::default())
    }

    fn app() -> Modes {
        Modes { app_cursor: true, app_keypad: true, ..Modes::default() }
    }

    #[test]
    fn characters_and_controls() {
        assert_eq!(enc(Key::Char('a'), NONE), b"a");
        assert_eq!(enc(Key::Char('é'), NONE), "é".as_bytes());
        assert_eq!(enc(Key::Char('c'), CTRL), b"\x03");
        assert_eq!(enc(Key::Char('C'), CTRL), b"\x03");
        assert_eq!(enc(Key::Char(' '), CTRL), b"\0");
        assert_eq!(enc(Key::Char('@'), CTRL), b"\0");
        assert_eq!(enc(Key::Char('['), CTRL), b"\x1b");
        assert_eq!(enc(Key::Char('\\'), CTRL), b"\x1c");
        assert_eq!(enc(Key::Char(']'), CTRL), b"\x1d");
        assert_eq!(enc(Key::Char('^'), CTRL), b"\x1e");
        assert_eq!(enc(Key::Char('_'), CTRL), b"\x1f");
        assert_eq!(enc(Key::Char('?'), CTRL), b"\x7f");
        assert_eq!(enc(Key::Char('é'), CTRL), "é".as_bytes());
        assert_eq!(enc(Key::Char('x'), ALT), b"\x1bx");
        assert_eq!(enc(Key::Char('x'), KeyMods { alt: true, ctrl: true, shift: false }), b"\x1b\x18");
    }

    #[test]
    fn editing_keys() {
        assert_eq!(enc(Key::Enter, NONE), b"\r");
        assert_eq!(enc(Key::Enter, ALT), b"\x1b\r");
        assert_eq!(encode(Key::Enter, NONE, &Modes { newline: true, ..Modes::default() }), b"\r\n");
        assert_eq!(enc(Key::Tab, NONE), b"\t");
        assert_eq!(enc(Key::Tab, SHIFT), b"\x1b[Z");
        assert_eq!(enc(Key::Backspace, NONE), b"\x7f");
        assert_eq!(enc(Key::Backspace, CTRL), b"\x08");
        assert_eq!(enc(Key::Backspace, ALT), b"\x1b\x7f");
        assert_eq!(enc(Key::Escape, NONE), b"\x1b");
        assert_eq!(enc(Key::KpEnter, NONE), b"\r");
        assert_eq!(encode(Key::KpEnter, NONE, &app()), b"\x1bOM");
    }

    #[test]
    fn cursor_keys() {
        let normal = [(Key::Up, "\x1b[A"), (Key::Down, "\x1b[B"), (Key::Right, "\x1b[C"), (Key::Left, "\x1b[D")];
        for (k, s) in normal {
            assert_eq!(enc(k, NONE), s.as_bytes());
        }
        assert_eq!(enc(Key::Home, NONE), b"\x1b[H");
        assert_eq!(enc(Key::End, NONE), b"\x1b[F");
        let a = app();
        assert_eq!(encode(Key::Up, NONE, &a), b"\x1bOA");
        assert_eq!(encode(Key::Left, NONE, &a), b"\x1bOD");
        assert_eq!(encode(Key::Home, NONE, &a), b"\x1bOH");
        assert_eq!(encode(Key::End, NONE, &a), b"\x1bOF");
        // Modifiers win over application mode.
        assert_eq!(encode(Key::Up, CTRL, &a), b"\x1b[1;5A");
        assert_eq!(enc(Key::Right, SHIFT), b"\x1b[1;2C");
        assert_eq!(enc(Key::Left, ALT), b"\x1b[1;3D");
        assert_eq!(enc(Key::Down, KeyMods { shift: true, ctrl: true, alt: true }), b"\x1b[1;8B");
        assert_eq!(enc(Key::End, CTRL), b"\x1b[1;5F");
    }

    #[test]
    fn tilde_and_function_keys() {
        assert_eq!(enc(Key::Insert, NONE), b"\x1b[2~");
        assert_eq!(enc(Key::Delete, NONE), b"\x1b[3~");
        assert_eq!(enc(Key::PageUp, NONE), b"\x1b[5~");
        assert_eq!(enc(Key::PageDown, NONE), b"\x1b[6~");
        assert_eq!(enc(Key::Delete, CTRL), b"\x1b[3;5~");
        assert_eq!(enc(Key::PageUp, SHIFT), b"\x1b[5;2~");
        assert_eq!(enc(Key::F(1), NONE), b"\x1bOP");
        assert_eq!(enc(Key::F(4), NONE), b"\x1bOS");
        assert_eq!(enc(Key::F(2), SHIFT), b"\x1b[1;2Q");
        let tails = ["15", "17", "18", "19", "20", "21", "23", "24"];
        for (i, t) in tails.iter().enumerate() {
            assert_eq!(enc(Key::F(5 + i as u8), NONE), format!("\x1b[{t}~").as_bytes());
        }
        assert_eq!(enc(Key::F(12), CTRL), b"\x1b[24;5~");
        assert_eq!(enc(Key::F(0), NONE), b"");
        assert_eq!(enc(Key::F(13), NONE), b"");
    }

    #[test]
    fn pasting() {
        let plain = Modes::default();
        assert_eq!(paste("a\nb\r\nc\rd", &plain), b"a\rb\rc\rd");
        assert_eq!(paste("x\x1b[201~y\tz", &plain), b"x[201~y\tz");
        let br = Modes { bracketed_paste: true, ..Modes::default() };
        assert_eq!(paste("ls\n", &br), b"\x1b[200~ls\r\x1b[201~");
        assert_eq!(paste("é", &br), "\x1b[200~é\x1b[201~".as_bytes());
    }

    #[test]
    fn mouse_reports() {
        let ev = |action, col, row| MouseEvent { action, col, row, mods: NONE };
        let press = ev(MouseAction::Press(MouseButton::Left), 4, 2);
        assert_eq!(mouse(press, &Modes::default()), b"");
        let normal = Modes { mouse: MouseMode::Press, ..Modes::default() };
        assert_eq!(mouse(press, &normal), b"\x1b[M\x20\x25\x23");
        assert_eq!(mouse(ev(MouseAction::Release(MouseButton::Left), 4, 2), &normal), b"\x1b[M\x23\x25\x23");
        assert_eq!(mouse(ev(MouseAction::Motion(Some(MouseButton::Left)), 4, 2), &normal), b"");
        assert_eq!(mouse(ev(MouseAction::Press(MouseButton::Left), 300, 2), &normal), b"");
        let sgr = Modes { mouse: MouseMode::Drag, sgr_mouse: true, ..Modes::default() };
        assert_eq!(mouse(press, &sgr), b"\x1b[<0;5;3M");
        assert_eq!(mouse(ev(MouseAction::Release(MouseButton::Right), 0, 0), &sgr), b"\x1b[<2;1;1m");
        assert_eq!(mouse(ev(MouseAction::Motion(Some(MouseButton::Left)), 9, 9), &sgr), b"\x1b[<32;10;10M");
        assert_eq!(mouse(ev(MouseAction::Motion(None), 9, 9), &sgr), b"");
        assert_eq!(mouse(ev(MouseAction::Press(MouseButton::WheelDown), 0, 0), &sgr), b"\x1b[<65;1;1M");
        assert_eq!(mouse(ev(MouseAction::Release(MouseButton::WheelDown), 0, 0), &sgr), b"");
        let any = Modes { mouse: MouseMode::Any, sgr_mouse: true, ..Modes::default() };
        assert_eq!(mouse(ev(MouseAction::Motion(None), 1, 1), &any), b"\x1b[<35;2;2M");
        let ctrl_press = MouseEvent { mods: CTRL, ..press };
        assert_eq!(mouse(ctrl_press, &sgr), b"\x1b[<16;5;3M");
    }

    #[test]
    fn focus_reports() {
        assert_eq!(focus(true, &Modes::default()), b"");
        let m = Modes { focus_events: true, ..Modes::default() };
        assert_eq!(focus(true, &m), b"\x1b[I");
        assert_eq!(focus(false, &m), b"\x1b[O");
    }
}
