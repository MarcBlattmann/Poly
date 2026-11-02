//! The terminal of `cargo xtask start`'s menu and progress table (ADR 0133):
//! raw input, keys, the window's size, and ANSI escapes for the screen.
//!
//! No crate: what is needed is small, and the terminal crates pull in
//! Windows crates linked through `raw-dylib`, which the `windows-gnu`
//! toolchain cannot link without MinGW's `dlltool`. On Unix the terminal is
//! put into raw mode with termios (`cfmakeraw`) and its size read with
//! `TIOCGWINSZ`; on Windows the console is switched to virtual terminal
//! input and output (`SetConsoleMode`, Windows 10 and later; the console
//! API's documentation is the reference), so both send the same escape
//! sequences for keys and understand the same ones for the screen.

use std::io::{self, Read};

/// The alternate screen on and off, the cursor hidden and shown, the
/// screen cleared, the cursor home.
pub const ALT_ON: &str = "\x1b[?1049h";
pub const ALT_OFF: &str = "\x1b[?1049l";
pub const HIDE: &str = "\x1b[?25l";
pub const SHOW: &str = "\x1b[?25h";
pub const CLEAR: &str = "\x1b[H\x1b[2J";

/// Move the cursor to `row`, `col` (from 0).
pub fn goto(row: usize, col: usize) -> String {
    format!("\x1b[{};{}H", row + 1, col + 1)
}

/// Select Graphic Rendition: `codes` (`0` reset, `1` bold, `7` reverse,
/// `3x` colours, `90` dark grey).
pub fn sgr(codes: &str) -> String {
    format!("\x1b[{codes}m")
}

/// A key, as the menu sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Space,
    Esc,
    Tab,
    Backspace,
    Char(char),
}

/// The keys in `bytes`, one read's worth of input: escape sequences
/// (`ESC [ A`, `ESC O A`, with or without modifiers) for the arrows, a lone
/// `ESC` for Esc, Ctrl-C as Esc, and UTF-8 text.
pub fn parse_keys(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            let Some(&kind) = bytes.get(i + 1) else {
                keys.push(Key::Esc);
                break;
            };
            if kind != b'[' && kind != b'O' {
                // Alt+key or a stray ESC: Esc.
                keys.push(Key::Esc);
                i += 1;
                continue;
            }
            // The sequence ends with a byte in 0x40..=0x7e after the
            // parameters.
            let mut j = i + 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            match bytes.get(j) {
                Some(b'A') => keys.push(Key::Up),
                Some(b'B') => keys.push(Key::Down),
                Some(b'C') => keys.push(Key::Right),
                Some(b'D') => keys.push(Key::Left),
                Some(b'Z') => keys.push(Key::Up),
                _ => {}
            }
            i = j + 1;
            continue;
        }
        let key = match b {
            b'\r' | b'\n' => Some(Key::Enter),
            b'\t' => Some(Key::Tab),
            b' ' => Some(Key::Space),
            0x7f | 0x08 => Some(Key::Backspace),
            0x03 => Some(Key::Esc),
            _ => None,
        };
        if let Some(k) = key {
            // Windows sends Enter as \r\n in some modes: one Enter.
            if !(b == b'\n' && i > 0 && bytes[i - 1] == b'\r') {
                keys.push(k);
            }
            i += 1;
            continue;
        }
        let len = match b {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 1,
        };
        let end = (i + len).min(bytes.len());
        if let Some(c) = std::str::from_utf8(&bytes[i..end]).ok().and_then(|s| s.chars().next())
            && !c.is_control()
        {
            keys.push(Key::Char(c));
        }
        i = end;
    }
    keys
}

/// Read the next keys (waits for at least one byte).
pub fn read_keys() -> io::Result<Vec<Key>> {
    let mut buf = [0u8; 64];
    loop {
        let n = io::stdin().read(&mut buf)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "standard input closed"));
        }
        let keys = parse_keys(&buf[..n]);
        if !keys.is_empty() {
            return Ok(keys);
        }
    }
}

#[cfg(unix)]
mod sys {
    use std::io;

    /// The terminal's settings before raw mode.
    pub struct Saved(libc::termios);

    pub fn is_console() -> bool {
        // SAFETY: isatty only looks at the descriptor.
        unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 }
    }

    pub fn raw() -> io::Result<Saved> {
        // SAFETY: a zeroed termios is a valid value to be filled by
        // tcgetattr, which only writes into it.
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: `t` is a valid termios to write to.
        if unsafe { libc::tcgetattr(0, &mut t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let saved = Saved(t);
        // SAFETY: cfmakeraw only changes the flags of the termios given.
        unsafe { libc::cfmakeraw(&mut t) };
        // Output processing stays on, so "\n" still starts a line.
        t.c_oflag |= libc::OPOST;
        // SAFETY: `t` is a valid termios from tcgetattr.
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(saved)
    }

    pub fn restore(saved: &Saved) {
        // SAFETY: the termios tcgetattr gave.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &saved.0) };
    }

    pub fn vt_output() {}

    pub fn size() -> Option<(usize, usize)> {
        // SAFETY: a zeroed winsize is valid, and TIOCGWINSZ only writes it.
        let mut w: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: the request takes a pointer to a winsize.
        let ok = unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut w) } == 0;
        (ok && w.ws_col > 0 && w.ws_row > 0).then_some((w.ws_col as usize, w.ws_row as usize))
    }
}

#[cfg(windows)]
mod sys {
    //! The console API (Windows SDK `consoleapi.h`, `wincon.h`,
    //! `processenv.h`; the constants are theirs).
    use std::io;

    type Handle = isize;
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    const ENABLE_LINE_INPUT: u32 = 0x0002;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    #[repr(C)]
    #[derive(Default)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    #[derive(Default)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    /// `CONSOLE_SCREEN_BUFFER_INFO`.
    #[repr(C)]
    #[derive(Default)]
    struct ScreenBufferInfo {
        size: Coord,
        cursor: Coord,
        attributes: u16,
        window: SmallRect,
        max_window: Coord,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> Handle;
        fn GetConsoleMode(console: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(console: Handle, mode: u32) -> i32;
        fn GetConsoleScreenBufferInfo(console: Handle, info: *mut ScreenBufferInfo) -> i32;
    }

    fn mode(which: u32) -> Option<(Handle, u32)> {
        // SAFETY: GetStdHandle takes any value and returns a handle or
        // INVALID_HANDLE_VALUE/NULL, which GetConsoleMode then refuses;
        // `m` is a valid u32 to write to.
        unsafe {
            let h = GetStdHandle(which);
            let mut m = 0;
            (GetConsoleMode(h, &mut m) != 0).then_some((h, m))
        }
    }

    /// The console modes before raw mode (input, output).
    pub struct Saved(u32, u32);

    /// Whether standard input and output are the console (mintty's
    /// pipes are not).
    pub fn is_console() -> bool {
        mode(STD_INPUT_HANDLE).is_some() && mode(STD_OUTPUT_HANDLE).is_some()
    }

    pub fn raw() -> io::Result<Saved> {
        let err = || io::Error::other("standard input or output is not a console");
        let (input, im) = mode(STD_INPUT_HANDLE).ok_or_else(err)?;
        let (output, om) = mode(STD_OUTPUT_HANDLE).ok_or_else(err)?;
        let raw_in =
            (im & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT)) | ENABLE_VIRTUAL_TERMINAL_INPUT;
        let vt_out = om | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
        // SAFETY: handles GetConsoleMode accepted; any mode bits are
        // checked by the console, which fails the call on bad ones.
        unsafe {
            if SetConsoleMode(input, raw_in) == 0 || SetConsoleMode(output, vt_out) == 0 {
                SetConsoleMode(input, im);
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Saved(im, om))
    }

    pub fn restore(saved: &Saved) {
        // SAFETY: as in `raw`, with the modes the console had.
        unsafe {
            if let Some((h, _)) = mode(STD_INPUT_HANDLE) {
                SetConsoleMode(h, saved.0);
            }
            if let Some((h, _)) = mode(STD_OUTPUT_HANDLE) {
                SetConsoleMode(h, saved.1);
            }
        }
    }

    /// Escapes on the console's output (standard output and error).
    pub fn vt_output() {
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            if let Some((h, m)) = mode(which) {
                // SAFETY: a handle GetConsoleMode accepted.
                unsafe { SetConsoleMode(h, m | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING) };
            }
        }
    }

    pub fn size() -> Option<(usize, usize)> {
        let (h, _) = mode(STD_OUTPUT_HANDLE).or_else(|| mode(STD_ERROR_HANDLE))?;
        let mut info = ScreenBufferInfo::default();
        // SAFETY: `info` is a valid CONSOLE_SCREEN_BUFFER_INFO to write.
        if unsafe { GetConsoleScreenBufferInfo(h, &mut info) } == 0 {
            return None;
        }
        let w = i32::from(info.window.right) - i32::from(info.window.left) + 1;
        let h = i32::from(info.window.bottom) - i32::from(info.window.top) + 1;
        (w > 0 && h > 0).then_some((w as usize, h as usize))
    }
}

#[cfg(not(any(unix, windows)))]
mod sys {
    use std::io;
    pub struct Saved;
    pub fn is_console() -> bool {
        false
    }
    pub fn raw() -> io::Result<Saved> {
        Err(io::Error::other("no terminal support on this host"))
    }
    pub fn restore(_: &Saved) {}
    pub fn vt_output() {}
    pub fn size() -> Option<(usize, usize)> {
        None
    }
}

/// Whether standard input and output are a terminal the menu can use.
pub fn is_console() -> bool {
    sys::is_console()
}

/// Raw input while it lives (keys one by one, no echo); the terminal's
/// settings come back when it is dropped.
pub struct Raw(sys::Saved);

impl Raw {
    pub fn enable() -> io::Result<Raw> {
        sys::raw().map(Raw)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        sys::restore(&self.0);
    }
}

impl std::fmt::Debug for Raw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Raw")
    }
}

/// Let the escapes through on standard output and error (Windows' console
/// needs it switched on; terminals elsewhere always do).
pub fn vt_output() {
    sys::vt_output();
}

/// The window's size (columns, rows), or a guess.
pub fn size() -> (usize, usize) {
    sys::size().unwrap_or((100, 30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_from_escape_sequences() {
        assert_eq!(parse_keys(b"\x1b[A\x1b[B\x1b[C\x1b[D"), [Key::Up, Key::Down, Key::Right, Key::Left]);
        assert_eq!(parse_keys(b"\x1bOA\x1b[1;5B"), [Key::Up, Key::Down]);
        assert_eq!(parse_keys(b"\x1b"), [Key::Esc]);
        assert_eq!(parse_keys(b"\x1b[3~"), []);
        assert_eq!(parse_keys(b"\x1b[Z"), [Key::Up]);
        assert_eq!(parse_keys(b"\x1bx"), [Key::Esc, Key::Char('x')]);
    }

    #[test]
    fn keys_from_text() {
        assert_eq!(parse_keys(b"\r"), [Key::Enter]);
        assert_eq!(parse_keys(b"\r\n"), [Key::Enter]);
        assert_eq!(parse_keys(b"\n"), [Key::Enter]);
        assert_eq!(
            parse_keys(b"a 1\t\x7f\x08\x03"),
            [Key::Char('a'), Key::Space, Key::Char('1'), Key::Tab, Key::Backspace, Key::Backspace, Key::Esc]
        );
        assert_eq!(parse_keys("ü×".as_bytes()), [Key::Char('ü'), Key::Char('×')]);
        assert_eq!(parse_keys(&[0xff, 0x01]), []);
    }

    #[test]
    fn escapes() {
        assert_eq!(goto(0, 0), "\x1b[1;1H");
        assert_eq!(sgr("1;7"), "\x1b[1;7m");
    }
}
