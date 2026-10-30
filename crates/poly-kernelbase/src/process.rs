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
    child_parameters(base, &Child { image, command_line, env, current, startup: None })
}

// ----- STARTUPINFO, the standard handles, inheritance (ADR 0089) -----------------------

/// `CREATE_NEW_CONSOLE`, `CREATE_NO_WINDOW`, `DETACHED_PROCESS`
/// (`winbase.h`): accepted, not acted on (Poly's console choice, ADR 0089).
pub const DETACHED_PROCESS: u32 = 0x8;
pub const CREATE_NEW_CONSOLE: u32 = 0x10;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `STARTF_USESTDHANDLES` (`processthreadsapi.h` / `winbase.h`).
pub const STARTF_USESTDHANDLES: u32 = 0x100;
/// `sizeof(STARTUPINFOW)` (and `STARTUPINFOA`) on x64
/// (`processthreadsapi.h`): `cb` at 0, `lpReserved` 8, `lpDesktop` 16,
/// `lpTitle` 24, the eight `DWORD`s `dwX` .. `dwFlags` 32 .. 64,
/// `wShowWindow` 64, `cbReserved2` 66, `lpReserved2` 72, `hStdInput` 80,
/// `hStdOutput` 88, `hStdError` 96.
pub const STARTUPINFO_SIZE: usize = 104;
pub const SI_RESERVED: usize = 8;
pub const SI_DESKTOP: usize = 16;
pub const SI_TITLE: usize = 24;
/// `dwX`: the first of the fields copied as they are.
pub const SI_X: usize = 32;
pub const SI_FLAGS: usize = 60;
pub const SI_CB_RESERVED2: usize = 66;
pub const SI_RESERVED2: usize = 72;
pub const SI_STD_INPUT: usize = 80;
/// The bytes from `dwX` through `cbReserved2`.
pub const SI_FIELDS: usize = 36;

/// `NtCreateUserProcess`'s flags as Poly's NT personality reads them (ADR
/// 0089, `poly_personality_nt::consts`): `ProcessFlags` bit 2 inherits the
/// creator's inheritable handles (Poly's own); `ThreadFlags`
/// `THREAD_CREATE_FLAGS_CREATE_SUSPENDED`.
pub const NT_PROCESS_INHERIT_HANDLES: u32 = 0x4;
pub const NT_THREAD_CREATE_SUSPENDED: u32 = 0x1;

/// `HANDLE_FLAG_INHERIT`, `HANDLE_FLAG_PROTECT_FROM_CLOSE` (`handleapi.h`
/// / `winbase.h`).
pub const HANDLE_FLAG_INHERIT: u32 = 0x1;
pub const HANDLE_FLAG_PROTECT_FROM_CLOSE: u32 = 0x2;
/// `DUPLICATE_CLOSE_SOURCE`, `DUPLICATE_SAME_ACCESS` (`winnt.h`).
pub const DUPLICATE_CLOSE_SOURCE: u32 = 0x1;
pub const DUPLICATE_SAME_ACCESS: u32 = 0x2;

/// **Poly's own** places in `RTL_USER_PROCESS_PARAMETERS` (no public
/// header names them; to be confirmed by the spec agent): the standard
/// handles in `winternl.h`'s `Reserved2[2..5]` (+ 0x20, ADR 0080), and
/// after ADR 0079's fields the creator's `STARTUPINFO` (ADR 0089): its
/// `cb` (+ 0xA0, 0: none was given), its 36 bytes `dwX` .. `cbReserved2`
/// as `STARTUPINFOW` lays them out (+ 0xA4, so `dwFlags` is at + 0xC0),
/// `lpDesktop`, `lpTitle` and `lpReserved` as `UNICODE_STRING`s (at 0xC8,
/// 0xD8, 0xE8; a null `Buffer` for a null pointer), and the
/// `lpReserved2` bytes' address (+ 0xF8; `cbReserved2` of them).
pub const PP_STANDARD_INPUT: usize = 0x20;
pub const PP_STARTUP_CB: usize = 0xA0;
pub const PP_STARTUP_FIELDS: usize = 0xA4;
pub const PP_STARTUP_FLAGS: usize = PP_STARTUP_FIELDS + (SI_FLAGS - SI_X);
pub const PP_STARTUP_CB_RESERVED2: usize = PP_STARTUP_FIELDS + (SI_CB_RESERVED2 - SI_X);
pub const PP_DESKTOP: usize = 0xC8;
pub const PP_TITLE: usize = 0xD8;
pub const PP_SHELL_INFO: usize = 0xE8;
pub const PP_RESERVED2: usize = 0xF8;

/// The `STARTUPINFO` a creator passed, as the child's parameters keep it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Startup {
    /// `cb` as given (never 0 once given: 0 is stored as the size).
    pub cb: u32,
    /// `dwX` .. `cbReserved2`, as `STARTUPINFOW` lays them out.
    pub fields: [u8; SI_FIELDS],
    pub desktop: Option<Vec<u16>>,
    pub title: Option<Vec<u16>>,
    pub reserved: Option<Vec<u16>>,
    /// The `lpReserved2` bytes (`cbReserved2` of them).
    pub reserved2: Vec<u8>,
    /// `hStdInput`, `hStdOutput`, `hStdError`.
    pub std: [u64; 3],
}

impl Startup {
    /// The fixed part of a caller's `STARTUPINFOW` (or `STARTUPINFOA`:
    /// the same layout), with the strings and `lpReserved2` bytes it
    /// points at, read by the caller.
    pub fn from_raw(
        si: &[u8; STARTUPINFO_SIZE],
        desktop: Option<Vec<u16>>,
        title: Option<Vec<u16>>,
        reserved: Option<Vec<u16>>,
        reserved2: Vec<u8>,
    ) -> Startup {
        let cb = u32::from_le_bytes(si[0..4].try_into().expect("4 bytes"));
        let mut fields = [0u8; SI_FIELDS];
        fields.copy_from_slice(&si[SI_X..SI_X + SI_FIELDS]);
        let std =
            core::array::from_fn(|i| u64::from_le_bytes(si[SI_STD_INPUT + 8 * i..][..8].try_into().expect("8 bytes")));
        Startup {
            cb: if cb == 0 { STARTUPINFO_SIZE as u32 } else { cb },
            fields,
            desktop,
            title,
            reserved,
            reserved2,
            std,
        }
    }

    /// `dwFlags`.
    pub fn flags(&self) -> u32 {
        u32::from_le_bytes(self.fields[SI_FLAGS - SI_X..][..4].try_into().expect("4 bytes"))
    }

    /// `cbReserved2`.
    pub fn cb_reserved2(&self) -> usize {
        usize::from(u16::from_le_bytes(self.fields[SI_CB_RESERVED2 - SI_X..][..2].try_into().expect("2 bytes")))
    }
}

/// What a child's parameters are made of.
#[derive(Clone, Copy, Debug)]
pub struct Child<'a> {
    pub image: &'a [u16],
    pub command_line: &'a [u16],
    pub env: &'a [u16],
    pub current: &'a [u16],
    /// The `STARTUPINFO` passed on (`None`: none was given).
    pub startup: Option<&'a Startup>,
}

/// A `UNICODE_STRING` with a null buffer for `None`.
fn put_opt_string(buf: &mut Vec<u8>, at: usize, base: u64, s: Option<&[u16]>) -> Result<(), u32> {
    match s {
        Some(s) => put_string(buf, at, base, s),
        None => Ok(()),
    }
}

/// [`parameters`] with everything a child gets: with a `STARTUPINFO`, its
/// fields (see [`PP_STARTUP_CB`]) and, with `STARTF_USESTDHANDLES`, its
/// three handles as the standard handles (+ 0x20).
pub fn child_parameters(base: u64, c: &Child<'_>) -> Result<Vec<u8>, u32> {
    let mut buf = alloc::vec![0u8; PARAMETERS_SIZE];
    put_string(&mut buf, PP_IMAGE_PATH_NAME, base, c.image)?;
    put_string(&mut buf, PP_COMMAND_LINE, base, c.command_line)?;
    put_string(&mut buf, PP_CURRENT_DIRECTORY, base, c.current)?;
    if let Some(s) = c.startup {
        buf[PP_STARTUP_CB..PP_STARTUP_CB + 4].copy_from_slice(&s.cb.to_le_bytes());
        buf[PP_STARTUP_FIELDS..PP_STARTUP_FIELDS + SI_FIELDS].copy_from_slice(&s.fields);
        put_opt_string(&mut buf, PP_DESKTOP, base, s.desktop.as_deref())?;
        put_opt_string(&mut buf, PP_TITLE, base, s.title.as_deref())?;
        put_opt_string(&mut buf, PP_SHELL_INFO, base, s.reserved.as_deref())?;
        let n = s.cb_reserved2().min(s.reserved2.len());
        if n > 0 {
            let at = base + buf.len() as u64;
            buf.extend_from_slice(&s.reserved2[..n]);
            buf.resize(buf.len().next_multiple_of(8), 0);
            buf[PP_RESERVED2..PP_RESERVED2 + 8].copy_from_slice(&at.to_le_bytes());
        }
        // The count says how many bytes there are.
        buf[PP_STARTUP_CB_RESERVED2..PP_STARTUP_CB_RESERVED2 + 2].copy_from_slice(&(n as u16).to_le_bytes());
        if s.flags() & STARTF_USESTDHANDLES != 0 {
            for (i, h) in s.std.iter().enumerate() {
                buf[PP_STANDARD_INPUT + 8 * i..][..8].copy_from_slice(&h.to_le_bytes());
            }
        }
    }
    buf.resize(buf.len().next_multiple_of(16), 0);
    let env_at = base + buf.len() as u64;
    buf.extend(c.env.iter().flat_map(|c| c.to_le_bytes()));
    buf[PP_ENVIRONMENT..PP_ENVIRONMENT + 8].copy_from_slice(&env_at.to_le_bytes());
    buf[PP_ENVIRONMENT_SIZE..PP_ENVIRONMENT_SIZE + 8].copy_from_slice(&((c.env.len() * 2) as u64).to_le_bytes());
    Ok(buf)
}

/// `GetStartupInfoW`'s structure from the process's own parameters `pp`
/// (their first [`PARAMETERS_SIZE`] bytes; the pointers in it are the
/// process's own): Learn: "the STARTUPINFO structure that was specified
/// when the calling process was created". A process no creator gave one
/// (one the kernel started) gets `cb` and zeros. The string pointers are
/// the parameters' NUL-terminated copies; the standard handles are filled
/// in when `dwFlags` has `STARTF_USESTDHANDLES` (they are the process's
/// standard handles then), else zero.
pub fn startup_info(pp: &[u8]) -> [u8; STARTUPINFO_SIZE] {
    let mut si = [0u8; STARTUPINFO_SIZE];
    si[0..4].copy_from_slice(&(STARTUPINFO_SIZE as u32).to_le_bytes());
    if pp.len() < PP_RESERVED2 + 8 {
        return si;
    }
    let u64_at = |o: usize| u64::from_le_bytes(pp[o..o + 8].try_into().expect("8 bytes"));
    if u32::from_le_bytes(pp[PP_STARTUP_CB..PP_STARTUP_CB + 4].try_into().expect("4 bytes")) == 0 {
        return si;
    }
    si[SI_X..SI_X + SI_FIELDS].copy_from_slice(&pp[PP_STARTUP_FIELDS..PP_STARTUP_FIELDS + SI_FIELDS]);
    for (field, string) in [(SI_RESERVED, PP_SHELL_INFO), (SI_DESKTOP, PP_DESKTOP), (SI_TITLE, PP_TITLE)] {
        si[field..field + 8].copy_from_slice(&u64_at(string + 8).to_le_bytes());
    }
    si[SI_RESERVED2..SI_RESERVED2 + 8].copy_from_slice(&u64_at(PP_RESERVED2).to_le_bytes());
    let flags = u32::from_le_bytes(si[SI_FLAGS..SI_FLAGS + 4].try_into().expect("4 bytes"));
    if flags & STARTF_USESTDHANDLES != 0 {
        si[SI_STD_INPUT..SI_STD_INPUT + 24].copy_from_slice(&pp[PP_STANDARD_INPUT..PP_STANDARD_INPUT + 24]);
    }
    si
}

/// `SetHandleInformation`: the new `(inherit, protect_from_close)` from the
/// current ones, `dwMask` and `dwFlags` (Learn: the mask says which flags
/// change, the flags give their new values). Unknown bits in the mask are
/// `ERROR_INVALID_PARAMETER` (to be confirmed by the spec agent).
pub fn handle_flags(current: (bool, bool), mask: u32, flags: u32) -> Result<(bool, bool), u32> {
    if mask & !(HANDLE_FLAG_INHERIT | HANDLE_FLAG_PROTECT_FROM_CLOSE) != 0 {
        return Err(crate::error::ERROR_INVALID_PARAMETER);
    }
    let pick = |bit: u32, old: bool| if mask & bit != 0 { flags & bit != 0 } else { old };
    Ok((pick(HANDLE_FLAG_INHERIT, current.0), pick(HANDLE_FLAG_PROTECT_FROM_CLOSE, current.1)))
}

/// `GetHandleInformation`'s `HANDLE_FLAG_*` bits.
pub fn handle_flag_bits(inherit: bool, protect_from_close: bool) -> u32 {
    (if inherit { HANDLE_FLAG_INHERIT } else { 0 })
        | (if protect_from_close { HANDLE_FLAG_PROTECT_FROM_CLOSE } else { 0 })
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
        // No STARTUPINFO: its fields and the standard handles stay zero.
        assert!(p[PP_STANDARD_INPUT..PP_STANDARD_INPUT + 24].iter().all(|&b| b == 0));
        assert!(p[PP_STARTUP_CB..PP_RESERVED2 + 8].iter().all(|&b| b == 0));
        assert_eq!(startup_info(&p)[..4], 104u32.to_le_bytes());
        assert!(startup_info(&p)[4..].iter().all(|&b| b == 0));
    }

    /// A `STARTUPINFOW` as a caller fills it.
    fn raw_startup(flags: u32, cb_reserved2: u16, std: [u64; 3]) -> [u8; STARTUPINFO_SIZE] {
        let mut si = [0u8; STARTUPINFO_SIZE];
        si[0..4].copy_from_slice(&104u32.to_le_bytes());
        si[SI_X..SI_X + 4].copy_from_slice(&10u32.to_le_bytes()); // dwX
        si[SI_X + 28..SI_X + 32].copy_from_slice(&flags.to_le_bytes());
        si[64..66].copy_from_slice(&5u16.to_le_bytes()); // wShowWindow
        si[SI_CB_RESERVED2..SI_CB_RESERVED2 + 2].copy_from_slice(&cb_reserved2.to_le_bytes());
        for (i, h) in std.iter().enumerate() {
            si[SI_STD_INPUT + 8 * i..][..8].copy_from_slice(&h.to_le_bytes());
        }
        si
    }

    #[test]
    fn a_startupinfo_goes_into_the_parameters_and_comes_back() {
        let base = 0x20_0000;
        let raw = raw_startup(STARTF_USESTDHANDLES | 0x1, 3, [0x10, 0x14, 0x18]);
        let s = Startup::from_raw(&raw, None, Some(wide("Title")), Some(wide("")), alloc::vec![7, 8, 9, 10]);
        assert_eq!((s.flags(), s.cb_reserved2(), s.std), (0x101, 3, [0x10, 0x14, 0x18]));
        let env = wide("A=1\0\0");
        let c =
            Child { image: &wide("C:\\x.exe"), command_line: &wide("x"), env: &env, current: &[], startup: Some(&s) };
        let p = child_parameters(base, &c).unwrap();
        let u64_at = |o: usize| u64::from_le_bytes(p[o..o + 8].try_into().unwrap());
        // The standard handles where ADR 0080 keeps them.
        assert_eq!([u64_at(0x20), u64_at(0x28), u64_at(0x30)], [0x10, 0x14, 0x18]);
        assert_eq!(u32::from_le_bytes(p[PP_STARTUP_FLAGS..][..4].try_into().unwrap()), 0x101);
        // A null lpDesktop has a null buffer; an empty lpReserved has one.
        assert_eq!(u64_at(PP_DESKTOP + 8), 0);
        assert_ne!(u64_at(PP_SHELL_INFO + 8), 0);
        // cbReserved2 bytes of lpReserved2 (not more than it says).
        let r2 = (u64_at(PP_RESERVED2) - base) as usize;
        assert_eq!(&p[r2..r2 + 3], &[7, 8, 9]);
        // The environment still follows, 16-aligned.
        assert_eq!(u64_at(PP_ENVIRONMENT) % 16, 0);

        // GetStartupInfoW in the child: the same fields, its own pointers.
        let si = startup_info(&p);
        assert_eq!(si[..4], 104u32.to_le_bytes());
        assert_eq!(si[SI_X..SI_X + SI_FIELDS], raw[SI_X..SI_X + SI_FIELDS]);
        let ptr = |o: usize| u64::from_le_bytes(si[o..o + 8].try_into().unwrap());
        assert_eq!(ptr(SI_DESKTOP), 0);
        let title = (ptr(SI_TITLE) - base) as usize;
        let units: Vec<u16> = p[title..title + 12].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(units, wide("Title\0"));
        assert_eq!(ptr(SI_RESERVED2), u64_at(PP_RESERVED2));
        assert_eq!([ptr(80), ptr(88), ptr(96)], [0x10, 0x14, 0x18]);

        // Without STARTF_USESTDHANDLES the handles are neither passed nor
        // reported; a zero cb is taken as the size.
        let mut raw = raw_startup(0x1, 0, [0x10, 0x14, 0x18]);
        raw[0..4].fill(0);
        let s = Startup::from_raw(&raw, None, None, None, Vec::new());
        assert_eq!(s.cb, 104);
        let c = Child { startup: Some(&s), ..c };
        let p = child_parameters(base, &c).unwrap();
        assert!(p[0x20..0x38].iter().all(|&b| b == 0));
        assert_eq!(u64::from_le_bytes(p[PP_RESERVED2..][..8].try_into().unwrap()), 0);
        let si = startup_info(&p);
        assert!(si[80..104].iter().all(|&b| b == 0));
        assert_eq!(u32::from_le_bytes(si[SI_FLAGS..][..4].try_into().unwrap()), 1);
    }

    #[test]
    fn handle_information_flags() {
        let both = HANDLE_FLAG_INHERIT | HANDLE_FLAG_PROTECT_FROM_CLOSE;
        assert_eq!(handle_flags((false, false), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT), Ok((true, false)));
        assert_eq!(handle_flags((true, true), HANDLE_FLAG_INHERIT, 0), Ok((false, true)));
        // Bits outside the mask do not count.
        assert_eq!(handle_flags((false, false), HANDLE_FLAG_PROTECT_FROM_CLOSE, both), Ok((false, true)));
        assert_eq!(handle_flags((true, false), 0, 0), Ok((true, false)));
        assert_eq!(handle_flags((false, false), 4, 4), Err(crate::error::ERROR_INVALID_PARAMETER));
        assert_eq!(handle_flag_bits(true, true), both);
        assert_eq!(handle_flag_bits(false, true), HANDLE_FLAG_PROTECT_FROM_CLOSE);
        assert_eq!(handle_flag_bits(false, false), 0);
    }
}
