//! `_spawn*`, `_exec*`, `system`, `_popen` (Learn: "_spawn, _wspawn
//! functions", "_exec, _wexec functions", "system, _wsystem", "_popen,
//! _wpopen", "_cwait"; `process.h` for the modes): what they decide before
//! a process exists.
//!
//! - **The command line** is the arguments joined by single spaces, as
//!   they are (Learn: "passing _spawn the string "hi there" will result in
//!   the new process getting two arguments, "hi" and "there"").
//! - **The file** (Learn, `_spawn`'s `cmdname`): a name with an extension
//!   is tried as it is; a name ending in a period without it; any other
//!   name with `.com`, `.exe`, `.bat` and `.cmd` in that order. A name
//!   with a drive or a slash is looked for there only; else in the current
//!   directory, and the `p` forms then go through the directories of
//!   `PATH`.
//! - **The command interpreter** (`system`, `_popen`, and a `.bat` / `.cmd`
//!   file) is `COMSPEC`, else `cmd.exe` on `PATH` (Learn: "system uses the
//!   COMSPEC and PATH environment variables to locate the
//!   command-interpreter file CMD.exe"); it runs `<interpreter> /c
//!   <command>`.
//! - **Open descriptors** reach the child (Learn: "Files that are open when
//!   a _spawn call is made remain open in the new process"), except those
//!   opened with `_O_NOINHERIT`, through `STARTUPINFO`'s `cbReserved2` /
//!   `lpReserved2`, which Learn reserves "for use by the C Run-time". Learn
//!   does not give that block's layout: [`encode`] is **Poly's own** (a
//!   `u32` count, a flag byte per descriptor, then an unaligned 64-bit
//!   handle per descriptor), to be confirmed by the spec agent so runtimes
//!   linked into programs read it too.

use crate::fmode::Translation;
use crate::unit::Unit;
use alloc::vec::Vec;

/// `_P_WAIT`, `_P_NOWAIT`, `_P_OVERLAY`, `_P_NOWAITO`, `_P_DETACH`
/// (`process.h`).
pub const P_WAIT: i32 = 0;
pub const P_NOWAIT: i32 = 1;
pub const P_OVERLAY: i32 = 2;
pub const P_NOWAITO: i32 = 3;
pub const P_DETACH: i32 = 4;
/// `_WAIT_CHILD`, `_WAIT_GRANDCHILD` (`process.h`; both wait for the
/// process the handle names on Windows, Learn: `_cwait`).
pub const WAIT_CHILD: i32 = 0;
pub const WAIT_GRANDCHILD: i32 = 1;

/// The longest command line (`CreateProcessW`: 32,767 characters with its
/// NUL); a longer one is `E2BIG`. Learn's `_spawn` page names 1,024 bytes
/// for `E2BIG`, a limit of the past; Poly uses `CreateProcessW`'s (to be
/// confirmed by the spec agent).
pub const MAX_COMMAND_LINE: usize = 32_766;

fn is<C: Unit>(c: C, b: u8) -> bool {
    c.as_ascii() == Some(b)
}

fn lower<C: Unit>(c: C) -> u32 {
    match c.as_ascii() {
        Some(b) => u32::from(b.to_ascii_lowercase()),
        None => c.to_u32(),
    }
}

fn units<C: Unit>(s: &str) -> Vec<C> {
    s.bytes().map(C::ascii).collect()
}

/// The arguments joined by single spaces.
pub fn command_line<C: Unit>(args: &[&[C]]) -> Vec<C> {
    let mut line = Vec::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            line.push(C::ascii(b' '));
        }
        line.extend_from_slice(a);
    }
    line
}

/// Whether `name` has a drive or a slash (no `PATH` search).
pub fn has_path<C: Unit>(name: &[C]) -> bool {
    name.iter().any(|&c| is(c, b'\\') || is(c, b'/') || is(c, b':'))
}

fn last_component<C: Unit>(name: &[C]) -> &[C] {
    let start = name.iter().rposition(|&c| is(c, b'\\') || is(c, b'/') || is(c, b':')).map_or(0, |i| i + 1);
    &name[start..]
}

/// The names `cmdname` stands for, in the order they are tried.
pub fn candidates<C: Unit>(cmdname: &[C]) -> Vec<Vec<C>> {
    let last = last_component(cmdname);
    if last.last().is_some_and(|&c| is(c, b'.')) {
        return alloc::vec![cmdname[..cmdname.len() - 1].to_vec()];
    }
    if last.iter().any(|&c| is(c, b'.')) {
        return alloc::vec![cmdname.to_vec()];
    }
    [".com", ".exe", ".bat", ".cmd"]
        .iter()
        .map(|ext| {
            let mut n = cmdname.to_vec();
            n.extend(units::<C>(ext));
            n
        })
        .collect()
}

/// Whether `name` is a batch file (`.bat` / `.cmd`), which the command
/// interpreter runs.
pub fn is_batch<C: Unit>(name: &[C]) -> bool {
    let n = name.len();
    n >= 4
        && is(name[n - 4], b'.')
        && [".bat", ".cmd"]
            .iter()
            .any(|ext| name[n - 3..].iter().zip(&ext.as_bytes()[1..]).all(|(&c, &e)| lower(c) == u32::from(e)))
}

/// The directories of a `PATH` value: split at `;`, quotes removed, empty
/// entries left out.
pub fn path_dirs<C: Unit>(path: &[C]) -> Vec<Vec<C>> {
    path.split(|&c| is(c, b';'))
        .map(|d| d.iter().copied().filter(|&c| !is(c, b'"')).collect::<Vec<C>>())
        .filter(|d| !d.is_empty())
        .collect()
}

/// `dir\name` (no separator added after one already there).
pub fn join<C: Unit>(dir: &[C], name: &[C]) -> Vec<C> {
    let mut p = dir.to_vec();
    if !p.last().is_some_and(|&c| is(c, b'\\') || is(c, b'/') || is(c, b':')) {
        p.push(C::ascii(b'\\'));
    }
    p.extend_from_slice(name);
    p
}

/// Every path tried for `cmdname`: each candidate in the current directory
/// (as it is, relative), then — with `search` (the `p` forms) and no
/// drive or slash in the name — in each directory of `path`.
pub fn search_order<C: Unit>(cmdname: &[C], search: bool, path: Option<&[C]>) -> Vec<Vec<C>> {
    let names = candidates(cmdname);
    let mut out = names.clone();
    if search && !has_path(cmdname) {
        for dir in path.map(path_dirs).unwrap_or_default() {
            out.extend(names.iter().map(|n| join(&dir, n)));
        }
    }
    out
}

/// The command line that runs `command` through `interpreter`:
/// `<interpreter> /c <command>`, the interpreter's path in quotes if it
/// has a blank.
pub fn interpreter_line<C: Unit>(interpreter: &[C], command: &[C]) -> Vec<C> {
    let mut line = Vec::new();
    let quote = interpreter.iter().any(|&c| is(c, b' ') || is(c, b'\t'));
    if quote {
        line.push(C::ascii(b'"'));
    }
    line.extend_from_slice(interpreter);
    if quote {
        line.push(C::ascii(b'"'));
    }
    line.extend(units::<C>(" /c "));
    line.extend_from_slice(command);
    line
}

/// An environment block from `envp`'s `NAME=value` strings: each with its
/// NUL, then an empty one.
pub fn environment_block<C: Unit>(entries: &[&[C]]) -> Vec<C> {
    let mut b = Vec::new();
    for e in entries {
        b.extend_from_slice(e);
        b.push(C::default());
    }
    if entries.is_empty() {
        b.push(C::default());
    }
    b.push(C::default());
    b
}

/// What `_popen`'s mode asks: reading the command's output (`r`) or writing
/// its input (`w`), and text (`t`, the default) or binary (`b`). `None`
/// for anything else (Learn: "r", "w", "b", "t").
pub fn popen_mode<C: Unit>(mode: &[C]) -> Option<(bool, Translation)> {
    let mut read = None;
    let mut tr = Translation::Text;
    for &c in mode {
        match c.as_ascii() {
            Some(b'r') if read.is_none() => read = Some(true),
            Some(b'w') if read.is_none() => read = Some(false),
            Some(b't') => tr = Translation::Text,
            Some(b'b') => tr = Translation::Binary,
            _ => return None,
        }
    }
    read.map(|r| (r, tr))
}

/// A descriptor passed to a child: Poly's flag byte (open; translation;
/// append) and its handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Passed {
    pub mode: Translation,
    pub append: bool,
    pub handle: u64,
}

const OPEN: u8 = 0x01;
const APPEND: u8 = 0x08;

fn mode_bits(m: Translation) -> u8 {
    (match m {
        Translation::Binary => 0,
        Translation::Text => 1,
        Translation::Utf16 => 2,
        Translation::Utf8 => 3,
    }) << 1
}

fn mode_of(bits: u8) -> Translation {
    match (bits >> 1) & 3 {
        0 => Translation::Binary,
        1 => Translation::Text,
        2 => Translation::Utf16,
        _ => Translation::Utf8,
    }
}

/// The most descriptors the block holds (`cbReserved2` is a `WORD`).
pub const MAX_PASSED: usize = (u16::MAX as usize - 4) / 9;

/// The `lpReserved2` block for descriptors `0..fds.len()` (`None`: not
/// passed); at most [`MAX_PASSED`], trailing unpassed ones left out.
pub fn encode(fds: &[Option<Passed>]) -> Vec<u8> {
    let n = fds.iter().rposition(Option::is_some).map_or(0, |i| i + 1).min(MAX_PASSED);
    let mut b = Vec::with_capacity(4 + 9 * n);
    b.extend_from_slice(&(n as u32).to_le_bytes());
    for f in &fds[..n] {
        b.push(f.map_or(0, |p| OPEN | mode_bits(p.mode) | if p.append { APPEND } else { 0 }));
    }
    for f in &fds[..n] {
        b.extend_from_slice(&f.map_or(0, |p| p.handle).to_le_bytes());
    }
    b
}

/// The descriptors of a block [`encode`] made; empty for anything that is
/// not one (too short, or a count the block cannot hold).
pub fn decode(block: &[u8]) -> Vec<Option<Passed>> {
    let Some(head) = block.get(..4) else { return Vec::new() };
    let n = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if n > MAX_PASSED || block.len() < 4 + 9 * n {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let flags = block[4 + i];
            let at = 4 + n + 8 * i;
            let handle = u64::from_le_bytes(block[at..at + 8].try_into().expect("8 bytes"));
            (flags & OPEN != 0 && handle != 0 && handle != u64::MAX).then_some(Passed {
                mode: mode_of(flags),
                append: flags & APPEND != 0,
                handle,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn s(v: &[Vec<u8>]) -> Vec<String> {
        v.iter().map(|x| String::from_utf8(x.clone()).unwrap()).collect()
    }

    #[test]
    fn arguments_are_joined_as_they_are() {
        assert_eq!(command_line::<u8>(&[b"prog", b"hi there", b"x"]), b"prog hi there x");
        assert_eq!(command_line::<u8>(&[b"p", b"", b"\"q\""]), b"p  \"q\"");
        let w: Vec<u16> = "a".encode_utf16().collect();
        assert_eq!(command_line::<u16>(&[&w, &w]), "a a".encode_utf16().collect::<Vec<u16>>());
    }

    #[test]
    fn extensions_as_learn_lists_them() {
        assert_eq!(s(&candidates(b"tool")), ["tool.com", "tool.exe", "tool.bat", "tool.cmd"]);
        assert_eq!(s(&candidates(b"tool.exe")), ["tool.exe"]);
        assert_eq!(s(&candidates(b"tool.")), ["tool"]);
        assert_eq!(s(&candidates(br"C:\a.b\tool")).len(), 4);
        assert_eq!(s(&candidates(br"dir\x.y")), [r"dir\x.y"]);
    }

    #[test]
    fn search_through_path() {
        let order = search_order(b"t.exe", true, Some(b"C:\\bin;;\"D:\\my dir\";E:\\"));
        assert_eq!(s(&order), ["t.exe", r"C:\bin\t.exe", r"D:\my dir\t.exe", r"E:\t.exe"]);
        assert_eq!(s(&search_order(b"t.exe", false, Some(b"C:\\bin"))), ["t.exe"]);
        assert_eq!(s(&search_order(br"sub\t.exe", true, Some(b"C:\\bin"))), [r"sub\t.exe"]);
        assert_eq!(search_order(b"t", true, None).len(), 4);
        assert!(has_path(b"C:t") && has_path(b"a/b") && !has_path(b"t.exe"));
    }

    #[test]
    fn batch_files_and_the_interpreter() {
        assert!(is_batch(b"x.BAT") && is_batch(b"y.cmd") && !is_batch(b"z.exe") && !is_batch(b"bat"));
        assert_eq!(
            interpreter_line::<u8>(br"C:\Windows\System32\cmd.exe", b"dir *.c"),
            br"C:\Windows\System32\cmd.exe /c dir *.c"
        );
        assert_eq!(interpreter_line::<u8>(br"C:\Program Files\sh.exe", b"x"), br#""C:\Program Files\sh.exe" /c x"#);
    }

    #[test]
    fn environment_blocks() {
        assert_eq!(environment_block::<u8>(&[b"A=1", b"B=2"]), b"A=1\0B=2\0\0");
        assert_eq!(environment_block::<u8>(&[]), b"\0\0");
    }

    #[test]
    fn popen_modes() {
        assert_eq!(popen_mode(b"r"), Some((true, Translation::Text)));
        assert_eq!(popen_mode(b"wb"), Some((false, Translation::Binary)));
        assert_eq!(popen_mode(b"rt"), Some((true, Translation::Text)));
        assert_eq!(popen_mode(b"rw"), None);
        assert_eq!(popen_mode(b"b"), None);
        assert_eq!(popen_mode(b"rx"), None);
    }

    #[test]
    fn descriptor_blocks_round_trip() {
        let fds = [
            Some(Passed { mode: Translation::Text, append: false, handle: 0x10 }),
            None,
            Some(Passed { mode: Translation::Utf8, append: true, handle: 0x1_0000_0024 }),
            None,
        ];
        let b = encode(&fds);
        assert_eq!(b.len(), 4 + 3 * 9, "trailing unpassed descriptors left out");
        assert_eq!(&b[..4], &3u32.to_le_bytes());
        assert_eq!(&b[4..7], &[0x03, 0x00, 0x01 | (3 << 1) | 0x08]);
        assert_eq!(&b[7..15], &0x10u64.to_le_bytes());
        assert_eq!(decode(&b), fds[..3]);
        assert!(decode(&b[..20]).is_empty(), "short");
        assert!(decode(&[]).is_empty());
        assert!(decode(&u32::MAX.to_le_bytes()).is_empty());
        assert_eq!(encode(&[None, None]), 0u32.to_le_bytes());
        let binary = [Some(Passed { mode: Translation::Binary, append: false, handle: 8 })];
        assert_eq!(decode(&encode(&binary)), binary);
        let many = alloc::vec![Some(Passed { mode: Translation::Binary, append: false, handle: 4 }); MAX_PASSED + 5];
        assert!(encode(&many).len() <= u16::MAX as usize);
    }
}
