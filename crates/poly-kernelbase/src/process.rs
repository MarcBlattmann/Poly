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
use poly_rtl::layout::{Layout, X64};

/// `CREATE_SUSPENDED`, `CREATE_UNICODE_ENVIRONMENT` (`winbase.h`).
pub const CREATE_SUSPENDED: u32 = 0x4;
pub const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
/// `STILL_ACTIVE` (`minwinbase.h`): `STATUS_PENDING`.
pub const STILL_ACTIVE: u32 = 0x103;

/// `GetCurrentThreadStackLimits` (Learn: `processthreadsapi.h`): the boundaries of the thread's
/// stack, `(low, high)`: the base of its reservation (`TEB.DeallocationStack`, else the committed
/// limit `NT_TIB.StackLimit`) and `NT_TIB.StackBase`.
pub fn stack_limits(deallocation_stack: u64, stack_limit: u64, stack_base: u64) -> (u64, u64) {
    (if deallocation_stack != 0 { deallocation_stack } else { stack_limit }, stack_base)
}

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

/// Whether every string of an ANSI environment block has the `name=value`
/// form. A UTF-16 block read as bytes does not (its "strings" are single
/// characters): a wide block passed without `CREATE_UNICODE_ENVIRONMENT` is
/// refused this way (the `CreateProcessAsUserW` sheet:
/// `ERROR_INVALID_PARAMETER`).
pub fn ansi_block_is_well_formed(block: &[u8]) -> bool {
    block.split(|&b| b == 0).filter(|s| !s.is_empty()).all(|s| s.contains(&b'='))
}

/// The answer of `GetProcessMitigationPolicy(ProcessMitigationOptionsMask)`:
/// the 64-bit masks 0x3333333333333303 and 0x0333333333333330, little
/// endian, as the spec sheet recorded them (zeros follow for a longer
/// buffer).
pub const fn mitigation_options_mask() -> [u8; 16] {
    let (a, b) = (0x3333_3333_3333_3303u64.to_le_bytes(), 0x0333_3333_3333_3330u64.to_le_bytes());
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 8 {
        out[i] = a[i];
        out[8 + i] = b[i];
        i += 1;
    }
    out
}

/// A block as the child gets it: both final NULs present.
pub fn normalized_block(block: &[u16]) -> Vec<u16> {
    let mut v = block.to_vec();
    while v.len() < 2 || v[v.len() - 2..] != [0, 0] {
        v.push(0);
    }
    v
}

/// A `UNICODE_STRING` (`Length`, `MaximumLength`, then the pointer-sized `Buffer`) at `at` naming a
/// NUL-terminated copy of `s` appended to `buf` (which is placed at `base`).
fn put_string_in(l: &Layout, buf: &mut Vec<u8>, at: usize, base: u64, s: &[u16]) -> Result<(), u32> {
    let len = u16::try_from(s.len() * 2).map_err(|_| crate::error::ERROR_FILENAME_EXCED_RANGE)?;
    let ptr = base + buf.len() as u64;
    buf.extend(s.iter().flat_map(|c| c.to_le_bytes()));
    buf.extend_from_slice(&[0, 0]);
    buf.resize(buf.len().next_multiple_of(8), 0);
    buf[at..at + 2].copy_from_slice(&len.to_le_bytes());
    buf[at + 2..at + 4].copy_from_slice(&(len + 2).to_le_bytes());
    let (bytes, n) = l.ptr_bytes(ptr);
    let field = at + (l.unicode_string - l.ptr) as usize;
    buf[field..field + n].copy_from_slice(&bytes[..n]);
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
    child_parameters(base, &Child { image, command_line, env, current, startup: None, inherited_std: None })
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

/// Where `STARTUPINFOW`'s (and `STARTUPINFOA`'s) fields are for one pointer size
/// (`processthreadsapi.h`): the x64 offsets above, and the 32-bit ones of a WOW64 process
/// (ADR 0186): `cb` 0, `lpReserved` 4, `lpDesktop` 8, `lpTitle` 12, `dwX` .. `dwFlags` 16 .. 48,
/// `wShowWindow` 48, `cbReserved2` 50, `lpReserved2` 52, the three handles 56, 60, 64; 68 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SiLayout {
    pub size: usize,
    pub ptr: usize,
    pub reserved: usize,
    pub desktop: usize,
    pub title: usize,
    pub x: usize,
    pub flags: usize,
    pub cb_reserved2: usize,
    pub reserved2: usize,
    pub std_input: usize,
}

pub const SI_X64: SiLayout = SiLayout {
    size: STARTUPINFO_SIZE,
    ptr: 8,
    reserved: SI_RESERVED,
    desktop: SI_DESKTOP,
    title: SI_TITLE,
    x: SI_X,
    flags: SI_FLAGS,
    cb_reserved2: SI_CB_RESERVED2,
    reserved2: SI_RESERVED2,
    std_input: SI_STD_INPUT,
};

pub const SI_X86: SiLayout = SiLayout {
    size: 68,
    ptr: 4,
    reserved: 4,
    desktop: 8,
    title: 12,
    x: 16,
    flags: 44,
    cb_reserved2: 50,
    reserved2: 52,
    std_input: 56,
};

/// The `STARTUPINFO` of the process this code is compiled for.
#[cfg(target_pointer_width = "32")]
pub const SI_NATIVE: SiLayout = SI_X86;
/// The `STARTUPINFO` of the process this code is compiled for.
#[cfg(not(target_pointer_width = "32"))]
pub const SI_NATIVE: SiLayout = SI_X64;

impl SiLayout {
    /// The pointer-sized field at `at` of a `STARTUPINFO` laid out this way.
    pub fn pointer(&self, si: &[u8], at: usize) -> u64 {
        let mut b = [0u8; 8];
        b[..self.ptr].copy_from_slice(&si[at..at + self.ptr]);
        u64::from_le_bytes(b)
    }

    fn put_ptr(&self, si: &mut [u8], at: usize, v: u64) {
        si[at..at + self.ptr].copy_from_slice(&v.to_le_bytes()[..self.ptr]);
    }
}

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
        Startup::from_raw_in(&SI_X64, si, desktop, title, reserved, reserved2)
    }

    /// [`Startup::from_raw`] for a `STARTUPINFO` laid out as `sl` says (`si`: its `sl.size` bytes).
    pub fn from_raw_in(
        sl: &SiLayout,
        si: &[u8],
        desktop: Option<Vec<u16>>,
        title: Option<Vec<u16>>,
        reserved: Option<Vec<u16>>,
        reserved2: Vec<u8>,
    ) -> Startup {
        let cb = u32::from_le_bytes(si[0..4].try_into().expect("4 bytes"));
        let mut fields = [0u8; SI_FIELDS];
        fields.copy_from_slice(&si[sl.x..sl.x + SI_FIELDS]);
        // A 32-bit handle is sign extended (`INVALID_HANDLE_VALUE` is -1 in both widths).
        let std = core::array::from_fn(|i| {
            let v = sl.pointer(si, sl.std_input + sl.ptr * i);
            if sl.ptr == 4 { v as u32 as i32 as i64 as u64 } else { v }
        });
        Startup { cb: if cb == 0 { sl.size as u32 } else { cb }, fields, desktop, title, reserved, reserved2, std }
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
    /// The creator's own standard handles, which a child inherits when the
    /// `STARTUPINFO` does not choose them (see [`inherited_std_handles`]).
    pub inherited_std: Option<[u64; 3]>,
}

/// Whether a child inherits its creator's standard handles: Learn ("Console
/// Handles": "the parent's standard handles are inherited by any child
/// process"; `GetStdHandle`: a console program's handles are filled "if the
/// parent didn't already fill the standard handle table by inheritance").
/// Poly passes them when the `STARTUPINFO` does not choose its own
/// (`STARTF_USESTDHANDLES`), handles are inherited (`bInheritHandles`), and
/// the child shares the creator's console (none of `CREATE_NEW_CONSOLE`,
/// `DETACHED_PROCESS`, `CREATE_NO_WINDOW`). The other cases are not specified
/// yet: the child keeps getting its console's handles there (ADR 0194).
pub fn inherits_std_handles(startup_flags: u32, inherit_handles: bool, creation_flags: u32) -> bool {
    startup_flags & STARTF_USESTDHANDLES == 0
        && inherit_handles
        && creation_flags & (CREATE_NEW_CONSOLE | DETACHED_PROCESS | CREATE_NO_WINDOW) == 0
}

/// [`parameters`] with everything a child gets: with a `STARTUPINFO`, its
/// fields (see [`PP_STARTUP_CB`]) and, with `STARTF_USESTDHANDLES`, its
/// three handles as the standard handles (+ 0x20).
pub fn child_parameters(base: u64, c: &Child<'_>) -> Result<Vec<u8>, u32> {
    child_parameters_in(&X64, base, c)
}

/// [`child_parameters`] laid out for a creator of layout `l` (a 32-bit `kernelbase` passes the
/// 32-bit structure, ADR 0186; the kernel reads it in the caller's layout).
pub fn child_parameters_in(l: &Layout, base: u64, c: &Child<'_>) -> Result<Vec<u8>, u32> {
    let at = |o: u64| o as usize;
    let ptr = l.ptr as usize;
    let mut buf = alloc::vec![0u8; PARAMETERS_SIZE];
    put_string_in(l, &mut buf, at(l.pp_image_path_name), base, c.image)?;
    put_string_in(l, &mut buf, at(l.pp_command_line), base, c.command_line)?;
    put_string_in(l, &mut buf, at(l.pp_current_directory), base, c.current)?;
    if let Some(s) = c.startup {
        let (cb_at, fields_at) = (at(l.pp_startup), at(l.pp_startup) + 4);
        buf[cb_at..cb_at + 4].copy_from_slice(&s.cb.to_le_bytes());
        buf[fields_at..fields_at + SI_FIELDS].copy_from_slice(&s.fields);
        let strings = [
            (l.pp_desktop, s.desktop.as_deref()),
            (l.pp_title, s.title.as_deref()),
            (l.pp_shell_info, s.reserved.as_deref()),
        ];
        for (field, text) in strings {
            if let Some(text) = text {
                put_string_in(l, &mut buf, at(field), base, text)?;
            }
        }
        let n = s.cb_reserved2().min(s.reserved2.len());
        if n > 0 {
            let r2 = base + buf.len() as u64;
            buf.extend_from_slice(&s.reserved2[..n]);
            buf.resize(buf.len().next_multiple_of(8), 0);
            buf[at(l.pp_reserved2)..][..ptr].copy_from_slice(&r2.to_le_bytes()[..ptr]);
        }
        // The count says how many bytes there are.
        let cb2 = fields_at + (SI_CB_RESERVED2 - SI_X);
        buf[cb2..cb2 + 2].copy_from_slice(&(n as u16).to_le_bytes());
        if s.flags() & STARTF_USESTDHANDLES != 0 {
            for (i, h) in s.std.iter().enumerate() {
                buf[at(l.pp_standard_input) + ptr * i..][..ptr].copy_from_slice(&h.to_le_bytes()[..ptr]);
            }
        }
    }
    // The creator's own handles, in the same place; the kernel tells them
    // apart by the flag (`STARTF_USESTDHANDLES` is not set then).
    let chosen = c.startup.is_some_and(|s| s.flags() & STARTF_USESTDHANDLES != 0);
    if let (Some(std), false) = (c.inherited_std, chosen) {
        for (i, h) in std.iter().enumerate() {
            buf[at(l.pp_standard_input) + ptr * i..][..ptr].copy_from_slice(&h.to_le_bytes()[..ptr]);
        }
    }
    buf.resize(buf.len().next_multiple_of(16), 0);
    let env_at = base + buf.len() as u64;
    buf.extend(c.env.iter().flat_map(|c| c.to_le_bytes()));
    buf[at(l.pp_environment)..][..ptr].copy_from_slice(&env_at.to_le_bytes()[..ptr]);
    buf[at(l.pp_environment_size)..][..ptr].copy_from_slice(&((c.env.len() * 2) as u64).to_le_bytes()[..ptr]);
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
    startup_info_in(&X64, &SI_X64, pp).try_into().expect("the x64 size")
}

/// [`startup_info`] for a process of layout `l` whose `STARTUPINFO` is laid out as `sl` says (a
/// 32-bit process's is 68 bytes, ADR 0186).
pub fn startup_info_in(l: &Layout, sl: &SiLayout, pp: &[u8]) -> Vec<u8> {
    let mut si = alloc::vec![0u8; sl.size];
    si[0..4].copy_from_slice(&(sl.size as u32).to_le_bytes());
    let ptr = l.ptr as usize;
    if pp.len() < l.pp_reserved2 as usize + ptr {
        return si;
    }
    let ptr_at = |o: u64| {
        let mut b = [0u8; 8];
        b[..ptr].copy_from_slice(&pp[o as usize..o as usize + ptr]);
        u64::from_le_bytes(b)
    };
    let cb_at = l.pp_startup as usize;
    if u32::from_le_bytes(pp[cb_at..cb_at + 4].try_into().expect("4 bytes")) == 0 {
        return si;
    }
    si[sl.x..sl.x + SI_FIELDS].copy_from_slice(&pp[cb_at + 4..cb_at + 4 + SI_FIELDS]);
    let buffer = l.unicode_string - l.ptr;
    for (field, string) in [(sl.reserved, l.pp_shell_info), (sl.desktop, l.pp_desktop), (sl.title, l.pp_title)] {
        sl.put_ptr(&mut si, field, ptr_at(string + buffer));
    }
    sl.put_ptr(&mut si, sl.reserved2, ptr_at(l.pp_reserved2));
    let flags = u32::from_le_bytes(si[sl.flags..sl.flags + 4].try_into().expect("4 bytes"));
    if flags & STARTF_USESTDHANDLES != 0 {
        for i in 0..3u64 {
            sl.put_ptr(&mut si, sl.std_input + sl.ptr * i as usize, ptr_at(l.pp_standard_input + l.ptr * i));
        }
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

    #[test]
    fn mitigation_mask_bytes() {
        assert_eq!(
            mitigation_options_mask(),
            [0x03, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x30, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x03]
        );
    }

    #[test]
    fn ansi_environment_blocks() {
        assert!(ansi_block_is_well_formed(b"A=1\0PATH=C:\\x\0\0"));
        assert!(ansi_block_is_well_formed(b"=C:=C:\\\0A=\0\0"));
        assert!(ansi_block_is_well_formed(b"\0\0"));
        assert!(ansi_block_is_well_formed(b"\0"));
        assert!(!ansi_block_is_well_formed(b"A=1\0B\0\0"));
        // A UTF-16 block seen as bytes.
        let wide: Vec<u8> = "POLYVAR=9\0\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let len = ansi_block_len(&wide).unwrap();
        assert!(!ansi_block_is_well_formed(&wide[..len]));
    }

    #[test]
    fn stack_limits_are_the_reservation_and_the_top() {
        assert_eq!(stack_limits(0x10_0000, 0x13_0000, 0x14_0000), (0x10_0000, 0x14_0000));
        // Without a recorded reservation the committed limit stands in.
        assert_eq!(stack_limits(0, 0x13_0000, 0x14_0000), (0x13_0000, 0x14_0000));
    }
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
        let c = Child {
            image: &wide("C:\\x.exe"),
            command_line: &wide("x"),
            env: &env,
            current: &[],
            startup: Some(&s),
            inherited_std: Some([0x40, 0x44, 0x48]),
        };
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
        let c = Child { startup: Some(&s), inherited_std: None, ..c };
        let p = child_parameters(base, &c).unwrap();
        assert!(p[0x20..0x38].iter().all(|&b| b == 0));
        // The creator's own handles go where the chosen ones would, but
        // GetStartupInfoW does not report them.
        let inherited = child_parameters(base, &Child { inherited_std: Some([0x40, 0x44, 0x48]), ..c }).unwrap();
        let u64_at = |o: usize| u64::from_le_bytes(inherited[o..o + 8].try_into().unwrap());
        assert_eq!([u64_at(0x20), u64_at(0x28), u64_at(0x30)], [0x40, 0x44, 0x48]);
        assert!(startup_info(&inherited)[80..104].iter().all(|&b| b == 0));
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

    #[test]
    fn a_32_bit_creator_s_parameters_and_startup_info_have_the_32_bit_layout() {
        use poly_rtl::layout::X86;
        let mut raw = [0u8; 68];
        raw[0..4].copy_from_slice(&68u32.to_le_bytes());
        raw[SI_X86.flags..SI_X86.flags + 4].copy_from_slice(&(STARTF_USESTDHANDLES | 1).to_le_bytes());
        for (i, h) in [0x10u32, 0x14, 0xFFFF_FFFF].iter().enumerate() {
            raw[56 + 4 * i..60 + 4 * i].copy_from_slice(&h.to_le_bytes());
        }
        let s = Startup::from_raw_in(&SI_X86, &raw, None, Some("t".encode_utf16().collect()), None, Vec::new());
        assert_eq!(s.std, [0x10, 0x14, u64::MAX]);
        assert_eq!(s.flags(), STARTF_USESTDHANDLES | 1);
        let image: Vec<u16> = "C:\\a.exe".encode_utf16().collect();
        let env: Vec<u16> = "A=1\0\0".encode_utf16().collect();
        let c = Child {
            image: &image,
            command_line: &image,
            env: &env,
            current: &[],
            startup: Some(&s),
            inherited_std: None,
        };
        let base = 0x0040_0000u64;
        let pp = child_parameters_in(&X86, base, &c).unwrap();
        let u16_at = |o: u64| u16::from_le_bytes(pp[o as usize..o as usize + 2].try_into().unwrap());
        let u32_at = |o: u64| u32::from_le_bytes(pp[o as usize..o as usize + 4].try_into().unwrap());
        assert_eq!(u16_at(X86.pp_image_path_name), (image.len() * 2) as u16);
        let image_at = u64::from(u32_at(X86.pp_image_path_name + 4)) - base;
        assert_eq!(&pp[image_at as usize..image_at as usize + 2], &[b'C', 0]);
        assert_eq!(u32_at(X86.pp_standard_input + 8), 0xFFFF_FFFF);
        assert_eq!(u32_at(X86.pp_environment_size), (env.len() * 2) as u32);
        let si = startup_info_in(&X86, &SI_X86, &pp);
        assert_eq!(si.len(), 68);
        assert_eq!(u32::from_le_bytes(si[44..48].try_into().unwrap()), STARTF_USESTDHANDLES | 1);
        assert_eq!(u32::from_le_bytes(si[56..60].try_into().unwrap()), 0x10);
        assert_eq!(u32::from_le_bytes(si[64..68].try_into().unwrap()), 0xFFFF_FFFF);
        // lpTitle points at the parameters' NUL-terminated copy of the title.
        let title = u64::from(u32::from_le_bytes(si[12..16].try_into().unwrap())) - base;
        assert_eq!(&pp[title as usize..title as usize + 4], &[b't', 0, 0, 0]);
        assert_eq!(&si[4..8], &[0; 4]);
    }
}
