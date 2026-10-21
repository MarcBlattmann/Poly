//! `CreateProcessW`'s decisions: which file the command line names, where
//! it is searched for, the child's environment block, and the process
//! parameters `NtCreateUserProcess` is given.
//!
//! Learn (`CreateProcessW`, "lpApplicationName" and "lpCommandLine"):
//!
//! - With `lpApplicationName`, that is the module: a full or partial path
//!   (a partial one is relative to the current drive and directory), never
//!   searched for, its extension never added.
//! - Without it, the module is the first white-space-delimited token of
//!   the command line. A quoted token is taken as it is; an unquoted one
//!   with spaces is tried at each space in turn ("c:\program files\sub
//!   dir\program name" tries `c:\program`, `c:\program files\sub`, ...).
//!   ".exe is appended" when the name has no extension, unless it ends in
//!   a period or contains a path. A name without a path is searched for
//!   in: the directory the application loaded from, the current
//!   directory, the 32-bit system directory, the 16-bit system directory,
//!   the Windows directory, then the `PATH` directories.
//! - `lpEnvironment` NULL: the child gets the parent's environment; else a
//!   block of `name=value\0` strings ending in an empty one, UTF-16 with
//!   `CREATE_UNICODE_ENVIRONMENT`, else ANSI.
//! - `lpCurrentDirectory` NULL: the child gets the parent's.
//!
//! **The process parameters** (ADR 0073, 0079): `NtCreateUserProcess`
//! takes an `RTL_USER_PROCESS_PARAMETERS` built by the caller. Poly reads
//! `ImagePathName` (+ 0x60) and `CommandLine` (+ 0x70) where `winternl.h`
//! puts them, and its own fields for what no public header places: the
//! environment block's address and byte size (+ 0x80, + 0x88, as ADR 0073
//! laid out) and the current directory as a `UNICODE_STRING` at + 0x90
//! ([`PP_CURRENT_DIRECTORY`]).

use alloc::vec::Vec;

/// `CREATE_SUSPENDED`, `CREATE_UNICODE_ENVIRONMENT` (`winbase.h`).
pub const CREATE_SUSPENDED: u32 = 0x4;
pub const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
/// `STILL_ACTIVE` (`minwinbase.h`): `STATUS_PENDING`.
pub const STILL_ACTIVE: u32 = 0x103;

/// Offsets in `RTL_USER_PROCESS_PARAMETERS` (see the module docs).
pub const PP_IMAGE_PATH_NAME: usize = 0x60;
pub const PP_COMMAND_LINE: usize = 0x70;
pub const PP_ENVIRONMENT: usize = 0x80;
pub const PP_ENVIRONMENT_SIZE: usize = 0x88;
pub const PP_CURRENT_DIRECTORY: usize = 0x90;
/// The bytes of the structure (the strings follow it).
pub const PARAMETERS_SIZE: usize = 0x400;

fn is_space(c: u16) -> bool {
    c == u16::from(b' ') || c == u16::from(b'\t')
}

/// The names the command line's first token may be, in the order they are
/// tried.
pub fn program_candidates(cmd: &[u16]) -> Vec<Vec<u16>> {
    let start = cmd.iter().position(|&c| !is_space(c)).unwrap_or(cmd.len());
    let cmd = &cmd[start..];
    if cmd.first() == Some(&u16::from(b'"')) {
        let rest = &cmd[1..];
        let end = rest.iter().position(|&c| c == u16::from(b'"')).unwrap_or(rest.len());
        return alloc::vec![rest[..end].to_vec()];
    }
    let end = cmd.iter().position(|&c| c == u16::from(b'\t')).unwrap_or(cmd.len());
    let end = cmd[..end].iter().rposition(|&c| !is_space(c)).map_or(0, |i| i + 1);
    let cmd = &cmd[..end];
    let mut out: Vec<Vec<u16>> = cmd
        .iter()
        .enumerate()
        .filter(|&(i, &c)| c == u16::from(b' ') && i > 0 && cmd[i - 1] != u16::from(b' '))
        .map(|(i, _)| cmd[..i].to_vec())
        .collect();
    if !cmd.is_empty() {
        out.push(cmd.to_vec());
    }
    out
}

/// Whether a name has a directory part (a separator or a drive).
pub fn has_path(name: &[u16]) -> bool {
    name.iter().any(|&c| c == u16::from(b'\\') || c == u16::from(b'/') || c == u16::from(b':'))
}

/// The name with `.exe` appended if Learn's rule says so.
pub fn with_default_extension(name: &[u16]) -> Vec<u16> {
    let file = name.rsplit(|&c| c == u16::from(b'\\') || c == u16::from(b'/')).next().unwrap_or(name);
    let mut out = name.to_vec();
    if !has_path(name) && !file.contains(&u16::from(b'.')) {
        out.extend(crate::wide(".exe"));
    }
    out
}

/// The directories a name without a path is searched in, in order.
pub fn search_dirs(app_dir: &[u16], current: &[u16], path_var: Option<&[u16]>) -> Vec<Vec<u16>> {
    let mut out = alloc::vec![
        app_dir.to_vec(),
        current.to_vec(),
        crate::wide("C:\\Windows\\System32"),
        crate::wide("C:\\Windows\\System"),
        crate::wide("C:\\Windows"),
    ];
    if let Some(p) = path_var {
        out.extend(p.split(|&c| c == u16::from(b';')).filter(|d| !d.is_empty()).map(<[u16]>::to_vec));
    }
    out
}

/// `dir` and `name` joined with one separator.
pub fn join(dir: &[u16], name: &[u16]) -> Vec<u16> {
    let mut out = dir.to_vec();
    if !out.is_empty() && !matches!(out.last(), Some(&c) if c == u16::from(b'\\') || c == u16::from(b'/')) {
        out.push(u16::from(b'\\'));
    }
    out.extend_from_slice(name);
    out
}

/// The length in units of a UTF-16 environment block at the start of
/// `units` (through its final empty string), or `None` if it does not end
/// within them. An empty block may be one NUL or two (both accepted; to be
/// confirmed by the spec agent).
pub fn wide_block_len(units: &[u16]) -> Option<usize> {
    if units.first() == Some(&0) {
        return Some(if units.get(1) == Some(&0) { 2 } else { 1 });
    }
    units.windows(2).position(|w| w == [0, 0]).map(|i| i + 2)
}

/// As [`wide_block_len`] for an ANSI block.
pub fn ansi_block_len(bytes: &[u8]) -> Option<usize> {
    if bytes.first() == Some(&0) {
        return Some(if bytes.get(1) == Some(&0) { 2 } else { 1 });
    }
    bytes.windows(2).position(|w| w == [0, 0]).map(|i| i + 2)
}

/// A block as the child gets it: both final NULs present.
pub fn normalized_block(block: &[u16]) -> Vec<u16> {
    let mut v = block.to_vec();
    while v.len() < 2 || v[v.len() - 2..] != [0, 0] {
        v.push(0);
    }
    v
}

fn put_string(buf: &mut Vec<u8>, at: usize, base: u64, s: &[u16]) -> Result<(), u32> {
    let len = u16::try_from(s.len() * 2).map_err(|_| crate::error::ERROR_FILENAME_EXCED_RANGE)?;
    let ptr = base + buf.len() as u64;
    buf.extend(s.iter().flat_map(|c| c.to_le_bytes()));
    buf.extend_from_slice(&[0, 0]);
    buf.resize(buf.len().next_multiple_of(8), 0);
    buf[at..at + 2].copy_from_slice(&len.to_le_bytes());
    buf[at + 2..at + 4].copy_from_slice(&(len + 2).to_le_bytes());
    buf[at + 8..at + 16].copy_from_slice(&ptr.to_le_bytes());
    Ok(())
}

/// The parameters' bytes when placed at `base`: the structure, then the
/// strings (NUL-terminated), then the environment block (16-aligned). A
/// string too long for a `UNICODE_STRING` is `ERROR_FILENAME_EXCED_RANGE`.
pub fn parameters(
    base: u64,
    image: &[u16],
    command_line: &[u16],
    env: &[u16],
    current: &[u16],
) -> Result<Vec<u8>, u32> {
    let mut buf = alloc::vec![0u8; PARAMETERS_SIZE];
    put_string(&mut buf, PP_IMAGE_PATH_NAME, base, image)?;
    put_string(&mut buf, PP_COMMAND_LINE, base, command_line)?;
    put_string(&mut buf, PP_CURRENT_DIRECTORY, base, current)?;
    buf.resize(buf.len().next_multiple_of(16), 0);
    let env_at = base + buf.len() as u64;
    buf.extend(env.iter().flat_map(|c| c.to_le_bytes()));
    buf[PP_ENVIRONMENT..PP_ENVIRONMENT + 8].copy_from_slice(&env_at.to_le_bytes());
    buf[PP_ENVIRONMENT_SIZE..PP_ENVIRONMENT_SIZE + 8].copy_from_slice(&((env.len() * 2) as u64).to_le_bytes());
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;

    fn strs(v: &[Vec<u16>]) -> Vec<alloc::string::String> {
        v.iter().map(|s| alloc::string::String::from_utf16(s).unwrap()).collect()
    }

    #[test]
    fn candidates_follow_learns_example() {
        let c = program_candidates(&wide("c:\\program files\\sub dir\\program name"));
        assert_eq!(
            strs(&c),
            [
                "c:\\program",
                "c:\\program files\\sub",
                "c:\\program files\\sub dir\\program",
                "c:\\program files\\sub dir\\program name"
            ]
        );
        assert_eq!(strs(&program_candidates(&wide("\"C:\\A B\\x.exe\" -y"))), ["C:\\A B\\x.exe"]);
        assert_eq!(strs(&program_candidates(&wide("  app.exe arg  "))), ["app.exe", "app.exe arg"]);
        assert_eq!(strs(&program_candidates(&wide("app\targ"))), ["app"]);
        assert!(program_candidates(&wide("   ")).is_empty());
    }

    #[test]
    fn extension_and_search() {
        assert_eq!(with_default_extension(&wide("notepad")), wide("notepad.exe"));
        assert_eq!(with_default_extension(&wide("tool.com")), wide("tool.com"));
        assert_eq!(with_default_extension(&wide("C:\\bin\\tool")), wide("C:\\bin\\tool"));
        assert_eq!(with_default_extension(&wide("tool.")), wide("tool."));
        assert!(has_path(&wide("a\\b")) && has_path(&wide("C:x")) && !has_path(&wide("x.exe")));
        let dirs = search_dirs(&wide("C:\\App"), &wide("C:\\Cur"), Some(&wide("C:\\P1;;C:\\P2\\")));
        assert_eq!(
            strs(&dirs),
            ["C:\\App", "C:\\Cur", "C:\\Windows\\System32", "C:\\Windows\\System", "C:\\Windows", "C:\\P1", "C:\\P2\\"]
        );
        assert_eq!(join(&wide("C:\\P2\\"), &wide("x.exe")), wide("C:\\P2\\x.exe"));
        assert_eq!(join(&wide("C:\\P1"), &wide("x.exe")), wide("C:\\P1\\x.exe"));
    }

    #[test]
    fn environment_blocks() {
        let b = wide("A=1\0B=2\0\0junk");
        assert_eq!(wide_block_len(&b), Some(9));
        assert_eq!(wide_block_len(&[0, 0, 5]), Some(2));
        assert_eq!(wide_block_len(&[0, 5]), Some(1));
        assert_eq!(wide_block_len(&wide("A=1\0")), None);
        assert_eq!(ansi_block_len(b"X=y\0\0z"), Some(5));
        assert_eq!(normalized_block(&[0]), [0, 0]);
        assert_eq!(normalized_block(&wide("A=1\0\0")), wide("A=1\0\0"));
    }

    #[test]
    fn the_parameters_place_their_strings() {
        let base = 0x10_0000;
        let env = wide("A=1\0\0");
        let p = parameters(base, &wide("C:\\x.exe"), &wide("x.exe -a"), &env, &wide("C:\\")).unwrap();
        let u16_at = |o: usize| u16::from_le_bytes([p[o], p[o + 1]]);
        let u64_at = |o: usize| u64::from_le_bytes(p[o..o + 8].try_into().unwrap());
        let string = |o: usize| {
            let (len, ptr) = (u16_at(o) as usize, u64_at(o + 8) - base);
            assert_eq!(u16_at(o + 2) as usize, len + 2);
            let units: Vec<u16> =
                p[ptr as usize..][..len + 2].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            assert_eq!(units.last(), Some(&0));
            alloc::string::String::from_utf16(&units[..len / 2]).unwrap()
        };
        assert_eq!(string(PP_IMAGE_PATH_NAME), "C:\\x.exe");
        assert_eq!(string(PP_COMMAND_LINE), "x.exe -a");
        assert_eq!(string(PP_CURRENT_DIRECTORY), "C:\\");
        let env_at = (u64_at(PP_ENVIRONMENT) - base) as usize;
        assert_eq!(env_at % 16, 0);
        assert_eq!(u64_at(PP_ENVIRONMENT_SIZE), 10);
        assert_eq!(p.len(), env_at + 10);
        let long = alloc::vec![b'a' as u16; 40_000];
        assert!(parameters(base, &long, &[], &env, &[]).is_err());
    }
}
