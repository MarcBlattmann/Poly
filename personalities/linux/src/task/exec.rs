//! execve(2): reading the arguments and environment from user memory, and
//! interpreter scripts (`#!`).
//!
//! From execve(2):
//!
//! - `argv` and `envp` are arrays of string pointers ended by a null
//!   pointer; "On Linux, argv and envp can be specified as NULL", which
//!   means an empty list. A string longer than `MAX_ARG_STRLEN` (32 pages)
//!   or more than `MAX_ARG_STRINGS` strings is `E2BIG`, as is a total
//!   beyond the limit (a quarter of the stack limit; the kernel checks the
//!   final size when it builds the stack, Poly also stops reading here so
//!   a huge list never reaches the kernel). An unreadable pointer is
//!   `EFAULT`.
//! - A file starting with `#!` is a script: "interpreter [optional-arg]"
//!   on the first line, the interpreter run with `interpreter`,
//!   `optional-arg` (if any), the script's path, then `argv[1]` onwards.
//!   The interpreter may itself be a script, a few levels deep ("ELOOP
//!   ... the maximum recursion limit was reached during recursive script
//!   interpretation"); Poly allows [`MAX_SCRIPT_DEPTH`]. The line is
//!   limited ([`SHEBANG_MAX`]); text beyond is ignored, like the rest of
//!   the line after the optional argument's trailing blanks.

use crate::abi::PAGE_SIZE;
use crate::errno::Errno;
use crate::kernel::{LinuxKernel, UserPtr};
use alloc::vec::Vec;

/// The longest argument or environment string, with its NUL
/// (`MAX_ARG_STRLEN`, execve(2): 32 pages).
pub const MAX_ARG_STRLEN: usize = 32 * PAGE_SIZE as usize;
/// The most strings in `argv` or `envp` (`MAX_ARG_STRINGS`).
pub const MAX_ARG_STRINGS: usize = 0x7fff_ffff;
/// The most bytes of strings (with NULs) and pointers Poly reads: a quarter
/// of the 8 MiB default stack limit.
pub const MAX_ARG_TOTAL: usize = 2 << 20;
/// The longest path (`PATH_MAX`, `linux/limits.h`), with its NUL.
pub const PATH_MAX: usize = 4096;
/// How many `#!` scripts may lead to the binary (to be confirmed by the
/// diff runner).
pub const MAX_SCRIPT_DEPTH: usize = 4;
/// How many bytes of a script's first line count, `#!` included (execve(2):
/// "the maximum length of the #! line" is 255 characters since Linux 5.1;
/// Poly reads 256 bytes and takes the line within them; to be confirmed).
pub const SHEBANG_MAX: usize = 256;

/// `ELOOP` (`asm-generic/errno.h`).
pub const ELOOP: Errno = match Errno::new(40) {
    Some(e) => e,
    None => panic!("40 is a valid errno"),
};

/// What execve(2) and execveat(2) ask of the kernel: run `path`
/// (relative to `dirfd`'s directory or the working directory for
/// `AT_FDCWD`; empty: the file `dirfd` is open on, execveat's
/// `AT_EMPTY_PATH`) with `argv` and `envp`, already read from user memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecRequest<'a> {
    pub dirfd: i32,
    pub path: &'a [u8],
    pub argv: &'a [Vec<u8>],
    pub envp: &'a [Vec<u8>],
    /// Follow a final symbolic link (execveat without
    /// `AT_SYMLINK_NOFOLLOW`; a link is `ELOOP` otherwise).
    pub follow: bool,
    /// The name the new image is known by: its `AT_EXECFN`, and the path
    /// a script's interpreter gets ([`exec_name`]).
    pub name: &'a [u8],
    /// Whether an interpreter could open a script by [`name`](Self::name):
    /// not when the descriptor it goes through has `FD_CLOEXEC`
    /// (execveat's spec sheet), and a script is then `ENOENT`.
    pub script_reachable: bool,
}

impl<'a> ExecRequest<'a> {
    /// execve(2): `path` from the working directory, known by that name.
    pub fn new(path: &'a [u8], argv: &'a [Vec<u8>], envp: &'a [Vec<u8>]) -> ExecRequest<'a> {
        ExecRequest {
            dirfd: crate::fs::at::AT_FDCWD,
            path,
            argv,
            envp,
            follow: true,
            name: path,
            script_reachable: true,
        }
    }
}

/// The name execveat(2) records for the program `path` from `dirfd`
/// (execveat's spec sheet): the path itself when it is absolute or
/// `dirfd` is `AT_FDCWD`, `/dev/fd/<dirfd>` for an empty path
/// (`AT_EMPTY_PATH`), else `/dev/fd/<dirfd>/<path>`. Only a string:
/// nothing needs to exist at `/dev/fd`.
pub fn exec_name(dirfd: i32, path: &[u8]) -> Vec<u8> {
    if path.first() == Some(&b'/') || dirfd == crate::fs::at::AT_FDCWD {
        return path.to_vec();
    }
    let mut name = alloc::format!("/dev/fd/{dirfd}").into_bytes();
    if !path.is_empty() {
        name.push(b'/');
        name.extend_from_slice(path);
    }
    name
}

/// Read the path argument: `ENAMETOOLONG` past `PATH_MAX`, `ENOENT` if
/// empty (path_resolution(7)), `EFAULT` if unreadable.
pub fn read_path(k: &dyn LinuxKernel, ptr: UserPtr) -> Result<Vec<u8>, Errno> {
    let mut buf = alloc::vec![0u8; PATH_MAX];
    let n = k.strncpy_from_user(ptr, &mut buf)?;
    if n == 0 {
        return Err(Errno::ENOENT);
    }
    buf.truncate(n);
    Ok(buf)
}

/// Read a NULL-terminated array of string pointers at `array` (null: an
/// empty list). `budget` is the bytes left of [`MAX_ARG_TOTAL`] and is
/// charged for every string and pointer.
pub fn read_strings(k: &dyn LinuxKernel, array: UserPtr, budget: &mut usize) -> Result<Vec<Vec<u8>>, Errno> {
    let mut out = Vec::new();
    if array.is_null() {
        return Ok(out);
    }
    let mut buf = alloc::vec![0u8; MAX_ARG_STRLEN];
    loop {
        let at = array.offset(out.len() as u64 * 8).ok_or(Errno::EFAULT)?;
        let mut word = [0u8; 8];
        k.copy_from_user(at, &mut word)?;
        let ptr = UserPtr(u64::from_le_bytes(word));
        if ptr.is_null() {
            return Ok(out);
        }
        if out.len() >= MAX_ARG_STRINGS {
            return Err(Errno::E2BIG);
        }
        let n = match k.strncpy_from_user(ptr, &mut buf) {
            Err(Errno::ENAMETOOLONG) => return Err(Errno::E2BIG),
            r => r?,
        };
        *budget = budget.checked_sub(n + 1 + 8).ok_or(Errno::E2BIG)?;
        out.push(buf[..n].to_vec());
    }
}

/// The first line of a `#!` script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shebang {
    pub interpreter: Vec<u8>,
    pub arg: Option<Vec<u8>>,
}

/// Parse the start of a file: `None` if it is not a script, `ENOEXEC` for
/// a `#!` line without an interpreter.
pub fn parse_shebang(header: &[u8]) -> Result<Option<Shebang>, Errno> {
    let Some(rest) = header.strip_prefix(b"#!") else { return Ok(None) };
    let line = &rest[..rest.len().min(SHEBANG_MAX - 2)];
    let line = line.split(|&b| b == b'\n').next().unwrap_or_default();
    let blank = |b: &u8| *b == b' ' || *b == b'\t';
    let start = line.iter().position(|b| !blank(b)).unwrap_or(line.len());
    let line = &line[start..];
    let end = line.iter().position(blank).unwrap_or(line.len());
    let (interpreter, rest) = line.split_at(end);
    if interpreter.is_empty() {
        return Err(Errno::ENOEXEC);
    }
    let arg_start = rest.iter().position(|b| !blank(b)).unwrap_or(rest.len());
    let arg_end = rest.iter().rposition(|b| !blank(b)).map_or(arg_start, |i| i + 1);
    let arg = (arg_start < arg_end).then(|| rest[arg_start..arg_end].to_vec());
    Ok(Some(Shebang { interpreter: interpreter.to_vec(), arg }))
}

/// The argument list the interpreter of `script` (run as `path` with
/// `argv`) gets: interpreter, optional argument, `path`, `argv[1..]`.
pub fn script_argv(script: &Shebang, path: &[u8], argv: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(argv.len() + 2);
    out.push(script.interpreter.clone());
    out.extend(script.arg.clone());
    out.push(path.to_vec());
    out.extend(argv.iter().skip(1).cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<Vec<u8>> {
        s.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    #[test]
    fn shebang_lines() {
        let s = parse_shebang(b"#!/bin/sh\necho hi\n").unwrap().unwrap();
        assert_eq!((s.interpreter.as_slice(), s.arg), (b"/bin/sh".as_slice(), None));
        let s = parse_shebang(b"#! /usr/bin/env  python3 -u  \t\nx").unwrap().unwrap();
        assert_eq!(s.interpreter, b"/usr/bin/env");
        assert_eq!(s.arg.as_deref(), Some(b"python3 -u".as_slice()), "one argument, inner blanks kept");
        assert_eq!(parse_shebang(b"\x7fELF"), Ok(None));
        assert_eq!(parse_shebang(b"#!"), Err(Errno::ENOEXEC));
        assert_eq!(parse_shebang(b"#!   \n/bin/sh"), Err(Errno::ENOEXEC));
        let s = parse_shebang(b"#!/bin/interp").unwrap().unwrap();
        assert_eq!(s.interpreter, b"/bin/interp", "no newline at all");
        let mut long = b"#!/bin/x ".to_vec();
        long.extend(core::iter::repeat_n(b'a', 400));
        let s = parse_shebang(&long).unwrap().unwrap();
        assert_eq!(s.arg.unwrap().len(), SHEBANG_MAX - 9, "the line is cut");
    }

    #[test]
    fn execveat_names() {
        assert_eq!(exec_name(3, b"script"), b"/dev/fd/3/script");
        assert_eq!(exec_name(3, b""), b"/dev/fd/3");
        assert_eq!(exec_name(12, b"a/b"), b"/dev/fd/12/a/b");
        assert_eq!(exec_name(3, b"/bin/sh"), b"/bin/sh", "absolute: dirfd ignored");
        assert_eq!(exec_name(crate::fs::at::AT_FDCWD, b"prog"), b"prog");
    }

    #[test]
    fn script_arguments() {
        let s = Shebang { interpreter: b"/bin/sh".to_vec(), arg: Some(b"-e".to_vec()) };
        assert_eq!(script_argv(&s, b"/t/run", &v(&["run", "a", "b"])), v(&["/bin/sh", "-e", "/t/run", "a", "b"]));
        let s = Shebang { interpreter: b"/bin/sh".to_vec(), arg: None };
        assert_eq!(script_argv(&s, b"/t/run", &[]), v(&["/bin/sh", "/t/run"]));
        assert_eq!(script_argv(&s, b"x", &[b"x".to_vec()]), v(&["/bin/sh", "x"]));
    }
}
