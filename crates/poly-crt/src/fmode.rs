//! Opening files: `fopen` mode strings (Learn: `fopen`, `_wfopen`), the
//! `_open` flags (`fcntl.h`) and what `CreateFileW` gets for them (Learn:
//! `_open`, `_wopen`), and text-mode translation (Learn: "Text and binary
//! mode file I/O", `_read`, `_write`, `_setmode`).

use alloc::vec::Vec;

/// `fcntl.h`.
pub const O_RDONLY: i32 = 0x0000;
pub const O_WRONLY: i32 = 0x0001;
pub const O_RDWR: i32 = 0x0002;
pub const O_APPEND: i32 = 0x0008;
pub const O_RANDOM: i32 = 0x0010;
pub const O_SEQUENTIAL: i32 = 0x0020;
pub const O_TEMPORARY: i32 = 0x0040;
pub const O_NOINHERIT: i32 = 0x0080;
pub const O_CREAT: i32 = 0x0100;
pub const O_TRUNC: i32 = 0x0200;
pub const O_EXCL: i32 = 0x0400;
pub const O_SHORT_LIVED: i32 = 0x1000;
pub const O_OBTAIN_DIR: i32 = 0x2000;
pub const O_TEXT: i32 = 0x4000;
pub const O_BINARY: i32 = 0x8000;
pub const O_WTEXT: i32 = 0x10000;
pub const O_U16TEXT: i32 = 0x20000;
pub const O_U8TEXT: i32 = 0x40000;

/// `sys/stat.h`: `_S_IREAD`, `_S_IWRITE`.
pub const S_IREAD: i32 = 0x0100;
pub const S_IWRITE: i32 = 0x0080;

/// `share.h`.
pub const SH_DENYRW: i32 = 0x10;
pub const SH_DENYWR: i32 = 0x20;
pub const SH_DENYRD: i32 = 0x30;
pub const SH_DENYNO: i32 = 0x40;

/// How a stream or descriptor translates (`_setmode`'s modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Translation {
    Binary,
    /// ANSI text: `\n` ↔ `\r\n`, Ctrl+Z ends input.
    Text,
    /// UTF-16 text (`_O_WTEXT`, `_O_U16TEXT`).
    Utf16,
    /// UTF-8 text (`_O_U8TEXT`).
    Utf8,
}

impl Translation {
    /// The mode of `_O_*` flags, or the default (`_fmode`) if none.
    pub fn of(flags: i32, default: i32) -> Translation {
        let pick = |f: i32| {
            if f & O_U8TEXT != 0 {
                Some(Translation::Utf8)
            } else if f & (O_U16TEXT | O_WTEXT) != 0 {
                Some(Translation::Utf16)
            } else if f & O_BINARY != 0 {
                Some(Translation::Binary)
            } else if f & O_TEXT != 0 {
                Some(Translation::Text)
            } else {
                None
            }
        };
        pick(flags).or_else(|| pick(default)).unwrap_or(Translation::Text)
    }

    /// The `_O_*` flag `_setmode` returns for this mode.
    pub fn flag(self) -> i32 {
        match self {
            Translation::Binary => O_BINARY,
            Translation::Text => O_TEXT,
            Translation::Utf16 => O_U16TEXT,
            Translation::Utf8 => O_U8TEXT,
        }
    }
}

/// An `fopen` mode string, parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    /// The `_open` flags.
    pub flags: i32,
    /// Stream flags: readable, writable.
    pub read: bool,
    pub write: bool,
    /// `c` (commit on `fflush`).
    pub commit: bool,
}

/// Parse `r`, `w`, `a`, then any of `+`, `t`/`b`, `x` (with `w`), `c`/`n`,
/// `N`, `S`/`R`, `T`, `D`, then optionally `, ccs=UTF-8|UTF-16LE|UNICODE`
/// (Learn: `fopen`). `None` is an invalid mode (`EINVAL`).
pub fn parse_mode(mode: &[u8]) -> Option<Mode> {
    let (main, ccs) = match mode.iter().position(|&c| c == b',') {
        Some(i) => (&mode[..i], Some(&mode[i + 1..])),
        None => (mode, None),
    };
    let mut it = main.iter().copied().skip_while(|&c| c == b' ');
    let first = it.next()?;
    let mut flags = match first {
        b'r' => O_RDONLY,
        b'w' => O_WRONLY | O_CREAT | O_TRUNC,
        b'a' => O_WRONLY | O_CREAT | O_APPEND,
        _ => return None,
    };
    let (mut plus, mut tb, mut commit_seen, mut seq_seen) = (false, false, false, false);
    let (mut commit, mut tmp, mut short, mut excl, mut noinherit) = (false, false, false, false, false);
    for c in it {
        match c {
            b'+' if !plus => {
                plus = true;
                flags = (flags & !O_WRONLY) | O_RDWR;
            }
            b't' if !tb => {
                tb = true;
                flags |= O_TEXT;
            }
            b'b' if !tb => {
                tb = true;
                flags |= O_BINARY;
            }
            b'c' | b'n' if !commit_seen => {
                commit_seen = true;
                commit = c == b'c';
            }
            b'S' | b'R' if !seq_seen => {
                seq_seen = true;
                flags |= if c == b'S' { O_SEQUENTIAL } else { O_RANDOM };
            }
            b'T' if !short => short = true,
            b'D' if !tmp => tmp = true,
            b'N' if !noinherit => noinherit = true,
            b'x' if first == b'w' && !excl => excl = true,
            b' ' => {}
            _ => return None,
        }
    }
    if short {
        flags |= O_SHORT_LIVED;
    }
    if tmp {
        flags |= O_TEMPORARY;
    }
    if noinherit {
        flags |= O_NOINHERIT;
    }
    if excl {
        flags |= O_EXCL;
    }
    if let Some(ccs) = ccs {
        let ccs: Vec<u8> = ccs.iter().copied().filter(|&c| c != b' ').collect();
        let enc = ccs.strip_prefix(b"ccs=")?;
        flags = (flags & !(O_TEXT | O_BINARY))
            | match enc.to_ascii_uppercase().as_slice() {
                b"UTF-8" => O_U8TEXT,
                b"UTF-16LE" | b"UNICODE" => O_U16TEXT,
                _ => return None,
            };
    }
    let read = first == b'r' || plus;
    let write = first != b'r' || plus;
    Some(Mode { flags, read, write, commit })
}

/// `winnt.h` / `fileapi.h` constants for [`create_file_args`].
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const FILE_SHARE_READ: u32 = 1;
pub const FILE_SHARE_WRITE: u32 = 2;
pub const CREATE_NEW: u32 = 1;
pub const CREATE_ALWAYS: u32 = 2;
pub const OPEN_EXISTING: u32 = 3;
pub const OPEN_ALWAYS: u32 = 4;
pub const TRUNCATE_EXISTING: u32 = 5;
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
pub const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;
pub const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
pub const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
pub const FILE_FLAG_RANDOM_ACCESS: u32 = 0x1000_0000;
pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// What `CreateFileW` gets for an `_open`/`_sopen`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateArgs {
    pub access: u32,
    pub share: u32,
    pub disposition: u32,
    pub attributes: u32,
}

/// `_open(name, flags, pmode)` / `_sopen(…, shflag, pmode)` → `CreateFileW`'s
/// arguments; `None` is `EINVAL` (an unknown access mode, a share flag
/// outside `share.h`, `_O_TRUNC` with `_O_RDONLY`: Poly's reading of Learn's
/// "can't be specified with _O_RDONLY").
pub fn create_file_args(flags: i32, share: i32, pmode: i32) -> Option<CreateArgs> {
    let access = match flags & 3 {
        O_RDONLY => GENERIC_READ,
        O_WRONLY => GENERIC_WRITE,
        O_RDWR => GENERIC_READ | GENERIC_WRITE,
        _ => return None,
    };
    if flags & 3 == O_RDONLY && flags & O_TRUNC != 0 {
        return None;
    }
    let share = match share {
        SH_DENYRW => 0,
        SH_DENYWR => FILE_SHARE_READ,
        SH_DENYRD => FILE_SHARE_WRITE,
        SH_DENYNO => FILE_SHARE_READ | FILE_SHARE_WRITE,
        _ => return None,
    };
    let disposition = match (flags & O_CREAT != 0, flags & O_EXCL != 0, flags & O_TRUNC != 0) {
        (true, true, _) => CREATE_NEW,
        (true, false, true) => CREATE_ALWAYS,
        (true, false, false) => OPEN_ALWAYS,
        (false, _, true) => TRUNCATE_EXISTING,
        (false, _, false) => OPEN_EXISTING,
    };
    let mut attributes = FILE_ATTRIBUTE_NORMAL;
    if flags & O_CREAT != 0 && pmode & S_IWRITE == 0 {
        attributes = FILE_ATTRIBUTE_READONLY;
    }
    if flags & O_SHORT_LIVED != 0 {
        attributes |= FILE_ATTRIBUTE_TEMPORARY;
    }
    if flags & O_TEMPORARY != 0 {
        attributes |= FILE_FLAG_DELETE_ON_CLOSE;
    }
    if flags & O_SEQUENTIAL != 0 {
        attributes |= FILE_FLAG_SEQUENTIAL_SCAN;
    } else if flags & O_RANDOM != 0 {
        attributes |= FILE_FLAG_RANDOM_ACCESS;
    }
    if flags & O_OBTAIN_DIR != 0 {
        attributes |= FILE_FLAG_BACKUP_SEMANTICS;
    }
    Some(CreateArgs { access, share, disposition, attributes })
}

/// Text-mode output: each `\n` becomes `\r\n`.
pub fn to_crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 8);
    for &b in data {
        if b == b'\n' {
            out.push(b'\r');
        }
        out.push(b);
    }
    out
}

/// What [`from_crlf`] made of a block read in text mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextIn {
    /// The translated bytes.
    pub data: Vec<u8>,
    /// A Ctrl+Z (0x1A) was found: the end of the file for text input; the
    /// bytes after it were not taken.
    pub eof: bool,
    /// The block ended in a `\r` that was held back: the caller looks at
    /// the next byte (a `\n` makes it one `\n`, anything else keeps the
    /// `\r` and is read next).
    pub pending_cr: bool,
}

/// Text-mode input: `\r\n` → `\n` (a lone `\r` stays), Ctrl+Z ends the
/// input (Learn: `_read`: "in text mode ... CTRL+Z ... end-of-file").
pub fn from_crlf(data: &[u8]) -> TextIn {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            0x1A => return TextIn { data: out, eof: true, pending_cr: false },
            b'\r' if i + 1 == data.len() => return TextIn { data: out, eof: false, pending_cr: true },
            b'\r' if data[i + 1] == b'\n' => {
                out.push(b'\n');
                i += 2;
                continue;
            }
            b => out.push(b),
        }
        i += 1;
    }
    TextIn { data: out, eof: false, pending_cr: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_strings() {
        let m = parse_mode(b"r").unwrap();
        assert_eq!((m.flags, m.read, m.write), (O_RDONLY, true, false));
        let m = parse_mode(b"w+b").unwrap();
        assert_eq!((m.flags, m.read, m.write), (O_RDWR | O_CREAT | O_TRUNC | O_BINARY, true, true));
        let m = parse_mode(b"at").unwrap();
        assert_eq!(m.flags, O_WRONLY | O_CREAT | O_APPEND | O_TEXT);
        let m = parse_mode(b"wx").unwrap();
        assert_eq!(m.flags, O_WRONLY | O_CREAT | O_TRUNC | O_EXCL);
        let m = parse_mode(b"rbcNSTD").unwrap();
        assert!(m.commit);
        assert_eq!(m.flags, O_BINARY | O_NOINHERIT | O_SEQUENTIAL | O_SHORT_LIVED | O_TEMPORARY);
        let m = parse_mode(b"w, ccs=UTF-8").unwrap();
        assert_eq!(m.flags, O_WRONLY | O_CREAT | O_TRUNC | O_U8TEXT);
        let m = parse_mode(b"r,ccs=UNICODE").unwrap();
        assert_eq!(m.flags, O_U16TEXT);
        for bad in [&b""[..], b"q", b"rx", b"rbt", b"r++", b"r,ccs=KOI8", b"r,css=UTF-8", b"rz"] {
            assert_eq!(parse_mode(bad), None, "{:?}", core::str::from_utf8(bad));
        }
    }

    #[test]
    fn open_flags_to_create_file() {
        let a = create_file_args(O_RDONLY, SH_DENYNO, 0).unwrap();
        assert_eq!(
            a,
            CreateArgs {
                access: GENERIC_READ,
                share: 3,
                disposition: OPEN_EXISTING,
                attributes: FILE_ATTRIBUTE_NORMAL
            }
        );
        let a = create_file_args(O_WRONLY | O_CREAT | O_TRUNC, SH_DENYNO, S_IREAD | S_IWRITE).unwrap();
        assert_eq!((a.access, a.disposition, a.attributes), (GENERIC_WRITE, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL));
        let a = create_file_args(O_RDWR | O_CREAT | O_EXCL, SH_DENYWR, S_IREAD).unwrap();
        assert_eq!(
            (a.access, a.share, a.disposition, a.attributes),
            (GENERIC_READ | GENERIC_WRITE, 1, CREATE_NEW, FILE_ATTRIBUTE_READONLY)
        );
        assert_eq!(create_file_args(O_WRONLY | O_CREAT, SH_DENYRW, S_IWRITE).unwrap().disposition, OPEN_ALWAYS);
        assert_eq!(create_file_args(O_WRONLY | O_TRUNC, SH_DENYNO, 0).unwrap().disposition, TRUNCATE_EXISTING);
        let a = create_file_args(O_RDWR | O_TEMPORARY | O_SHORT_LIVED | O_RANDOM, SH_DENYNO, 0).unwrap();
        assert_eq!(
            a.attributes,
            FILE_ATTRIBUTE_NORMAL | FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE | FILE_FLAG_RANDOM_ACCESS
        );
        assert_eq!(create_file_args(3, SH_DENYNO, 0), None);
        assert_eq!(create_file_args(O_RDONLY | O_TRUNC, SH_DENYNO, 0), None);
        assert_eq!(create_file_args(O_RDONLY, 0, 0), None);
    }

    #[test]
    fn translations() {
        assert_eq!(Translation::of(O_BINARY, O_TEXT), Translation::Binary);
        assert_eq!(Translation::of(0, O_BINARY), Translation::Binary);
        assert_eq!(Translation::of(0, 0), Translation::Text);
        assert_eq!(Translation::of(O_WTEXT, 0), Translation::Utf16);
        assert_eq!(Translation::of(O_U8TEXT | O_TEXT, 0), Translation::Utf8);
        assert_eq!(Translation::Utf16.flag(), O_U16TEXT);
        assert_eq!(to_crlf(b"a\nb\n"), b"a\r\nb\r\n");
        assert_eq!(from_crlf(b"a\r\nb\rc\n"), TextIn { data: b"a\nb\rc\n".to_vec(), eof: false, pending_cr: false });
        assert_eq!(from_crlf(b"ab\r"), TextIn { data: b"ab".to_vec(), eof: false, pending_cr: true });
        assert_eq!(from_crlf(b"ab\x1Acd"), TextIn { data: b"ab".to_vec(), eof: true, pending_cr: false });
    }
}
