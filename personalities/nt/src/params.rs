//! A new NT process's parameters: `PEB.ProcessParameters`, an
//! `RTL_USER_PROCESS_PARAMETERS` with the image path, the command line and
//! the environment block (ADR 0073; ADR 0071 writes the same for processes
//! started through our `ntdll`).
//!
//! What public headers give (`winternl.h`, x64):
//!
//! | Field | Offset | Source |
//! |---|---|---|
//! | `PEB.ProcessParameters` | PEB + 0x20 | `winternl.h` (after `Reserved1[2]`, `BeingDebugged`, `Reserved2[1]`, `Reserved3[2]`, `Ldr`) |
//! | `RTL_USER_PROCESS_PARAMETERS.ImagePathName` | + 0x60 | `winternl.h` (after `Reserved1[16]`, `Reserved2[10]`) |
//! | `RTL_USER_PROCESS_PARAMETERS.CommandLine` | + 0x70 | `winternl.h` |
//! | `UNICODE_STRING` | 16 bytes: `Length`, `MaximumLength` (bytes, `u16`), `Buffer` at + 8 | `ntdef.h` |
//!
//! **Poly's own**, which no public header settles (to be confirmed by the
//! spec agent against real Windows): the environment block's address is
//! the pointer right after the documented `CommandLine`, at + 0x80
//! ([`PP_ENVIRONMENT`]); its size in bytes, terminators included, follows
//! at + 0x88 ([`PP_ENVIRONMENT_SIZE`]). The structure is
//! [`PARAMETERS_SIZE`] bytes, zero but for these fields, followed in the
//! same allocation by the strings (each with a terminating NUL that
//! `MaximumLength` counts) and the environment block. Our `ntdll` /
//! `kernelbase` (`GetEnvironmentStringsW`, `GetEnvironmentVariableW`,
//! `GetCommandLineW`) read these; a program that reads the reserved fields
//! itself is not served yet.
//!
//! The command line is made from the arguments by the documented parsing
//! rules run backwards ([`command_line`]; Learn: "Parsing C++ command-line
//! arguments", `CommandLineToArgvW`).

use crate::backend::Backend;
use crate::consts::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
use crate::system::NtProcess;
use alloc::string::String;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::STATUS_INVALID_PARAMETER;

/// `PEB.ProcessParameters`.
pub const PEB_PROCESS_PARAMETERS: u64 = 0x20;
/// `RTL_USER_PROCESS_PARAMETERS.ImagePathName`.
pub const PP_IMAGE_PATH_NAME: u64 = 0x60;
/// `RTL_USER_PROCESS_PARAMETERS.CommandLine`.
pub const PP_COMMAND_LINE: u64 = 0x70;
/// Poly's own: the environment block's address.
pub const PP_ENVIRONMENT: u64 = 0x80;
/// Poly's own: the environment block's size in bytes.
pub const PP_ENVIRONMENT_SIZE: u64 = 0x88;
/// The bytes kept for the structure (zero past the fields above).
pub const PARAMETERS_SIZE: u64 = 0x400;

/// Poly's own (ADR 0080): the console connection handle (0: none) in the
/// first pointer of `winternl.h`'s `Reserved2[10]` (+ 0x10), and the three
/// standard handles (input, output, error) in its third to fifth
/// (+ 0x20, + 0x28, + 0x30). `winternl.h` names none of them; to be
/// confirmed by the spec agent against real Windows.
pub const PP_CONSOLE_HANDLE: u64 = 0x10;
pub const PP_STANDARD_INPUT: u64 = 0x20;
pub const PP_STANDARD_OUTPUT: u64 = 0x28;
pub const PP_STANDARD_ERROR: u64 = 0x30;

/// Put a process's console connection and standard handles into its
/// parameters at `base` (what [`write`] returned).
pub fn write_handles(b: &mut dyn Backend, base: u64, console: u64, std: [u64; 3]) -> Result<(), NtStatus> {
    b.write(base + PP_CONSOLE_HANDLE, &console.to_le_bytes())?;
    for (i, h) in std.iter().enumerate() {
        b.write(base + PP_STANDARD_INPUT + 8 * i as u64, &h.to_le_bytes())?;
    }
    Ok(())
}

/// A process's parameters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parameters {
    /// The executable's full DOS path (`C:\...\app.exe`).
    pub image_path: Vec<u16>,
    pub command_line: Vec<u16>,
    /// A whole environment block, both terminators included
    /// ([`crate::layout::env::Block::to_utf16`]).
    pub environment: Vec<u16>,
}

fn put_u64(buf: &mut [u8], at: u64, v: u64) {
    buf[at as usize..][..8].copy_from_slice(&v.to_le_bytes());
}

/// A `UNICODE_STRING` at `at` for `len` UTF-16 units at `buffer`, with room
/// for a NUL.
fn put_string(buf: &mut [u8], at: u64, buffer: u64, len: usize) -> Result<(), NtStatus> {
    let length = u16::try_from(len * 2).map_err(|_| STATUS_INVALID_PARAMETER)?;
    let max = length.checked_add(2).ok_or(STATUS_INVALID_PARAMETER)?;
    let at = at as usize;
    buf[at..at + 2].copy_from_slice(&length.to_le_bytes());
    buf[at + 2..at + 4].copy_from_slice(&max.to_le_bytes());
    buf[at + 4..at + 8].fill(0);
    buf[at + 8..at + 16].copy_from_slice(&buffer.to_le_bytes());
    Ok(())
}

fn push_wide(buf: &mut Vec<u8>, s: &[u16]) {
    buf.extend(s.iter().flat_map(|c| c.to_le_bytes()));
}

/// The bytes of the parameters when placed at `base`: the structure, then
/// the image path and command line (NUL-terminated, 8-aligned), then the
/// environment block (16-aligned). A string longer than a `UNICODE_STRING`
/// can describe (32,766 units) is `STATUS_INVALID_PARAMETER`.
pub fn image(base: u64, p: &Parameters) -> Result<Vec<u8>, NtStatus> {
    let mut buf = alloc::vec![0u8; PARAMETERS_SIZE as usize];
    let align = |buf: &mut Vec<u8>, to: usize| buf.resize(buf.len().next_multiple_of(to), 0);
    let image_at = base + buf.len() as u64;
    push_wide(&mut buf, &p.image_path);
    buf.extend_from_slice(&[0, 0]);
    align(&mut buf, 8);
    let command_at = base + buf.len() as u64;
    push_wide(&mut buf, &p.command_line);
    buf.extend_from_slice(&[0, 0]);
    align(&mut buf, 16);
    let env_at = base + buf.len() as u64;
    push_wide(&mut buf, &p.environment);
    put_string(&mut buf, PP_IMAGE_PATH_NAME, image_at, p.image_path.len())?;
    put_string(&mut buf, PP_COMMAND_LINE, command_at, p.command_line.len())?;
    put_u64(&mut buf, PP_ENVIRONMENT, env_at);
    put_u64(&mut buf, PP_ENVIRONMENT_SIZE, (p.environment.len() * 2) as u64);
    Ok(buf)
}

/// Give the new process `proc` (its PEB at `peb`) its parameters: one
/// private read-write allocation holding [`image`], and
/// `PEB.ProcessParameters` pointing at it. Returns the structure's address.
pub fn write(proc: &mut NtProcess, b: &mut dyn Backend, peb: u64, p: &Parameters) -> Result<u64, NtStatus> {
    // The size does not depend on the base: lay it out once to measure.
    let size = image(0, p)?.len() as u64;
    let (base, _) = proc.vm.allocate(b, 0, size, 0, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)?;
    let bytes = image(base, p)?;
    b.write(base, &bytes)?;
    b.write(peb + PEB_PROCESS_PARAMETERS, &base.to_le_bytes())?;
    Ok(base)
}

/// One argument quoted so that `CommandLineToArgvW` (Learn: "Parsing C++
/// command-line arguments") gives it back: as it is when it has no space,
/// tab or `"` and is not empty; otherwise in quotes, each `"` escaped as
/// `\"` with the backslashes before it doubled, and the backslashes before
/// the closing quote doubled.
pub fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"', '\n', '\u{b}']) {
        return arg.into();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(core::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(core::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(core::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// A command line for `program` (the DOS path, always quoted when it has a
/// space: the first token is read by the rules for the program name) and
/// `args`.
pub fn command_line(program: &str, args: &[&str]) -> String {
    let mut out = if program.contains([' ', '\t']) { alloc::format!("\"{program}\"") } else { program.into() };
    for a in args {
        out.push(' ');
        out.push_str(&quote(a));
    }
    out
}

/// The `Parameters` for `image_path` run with `args` in `environment`.
pub fn for_program(image_path: &str, args: &[&str], environment: Vec<u16>) -> Parameters {
    Parameters {
        image_path: image_path.encode_utf16().collect(),
        command_line: command_line(image_path, args).encode_utf16().collect(),
        environment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rules of Learn's "Parsing C++ command-line arguments", as a
    /// reference parser for the arguments after the program name.
    fn parse_args(s: &str) -> Vec<String> {
        let chars: Vec<char> = s.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let mut arg = String::new();
            let mut quoted = false;
            while i < chars.len() && (quoted || (chars[i] != ' ' && chars[i] != '\t')) {
                if chars[i] == '\\' {
                    let mut n = 0;
                    while i < chars.len() && chars[i] == '\\' {
                        n += 1;
                        i += 1;
                    }
                    if i < chars.len() && chars[i] == '"' {
                        arg.extend(core::iter::repeat_n('\\', n / 2));
                        if n % 2 == 1 {
                            arg.push('"');
                            i += 1;
                        }
                    } else {
                        arg.extend(core::iter::repeat_n('\\', n));
                    }
                } else if chars[i] == '"' {
                    if quoted && i + 1 < chars.len() && chars[i + 1] == '"' {
                        arg.push('"');
                        i += 2;
                        continue;
                    }
                    quoted = !quoted;
                    i += 1;
                } else {
                    arg.push(chars[i]);
                    i += 1;
                }
            }
            out.push(arg);
        }
        out
    }

    #[test]
    fn quoting_round_trips_through_the_documented_rules() {
        let cases: &[&str] = &[
            "plain",
            "",
            "two words",
            "tab\there",
            "a\"b",
            "back\\slash",
            "ends\\",
            "ends with space\\",
            "\\\"",
            "C:\\Program Files\\x\\",
            "q\"\"q",
        ];
        for &c in cases {
            let line = quote(c);
            assert_eq!(parse_args(&line), alloc::vec![String::from(c)], "{c:?} quoted as {line:?}");
        }
        let line = command_line("C:\\Program Files\\App\\app.exe", &["-x", "a b", ""]);
        assert_eq!(line, "\"C:\\Program Files\\App\\app.exe\" -x \"a b\" \"\"");
        assert_eq!(command_line("C:\\a.exe", &[]), "C:\\a.exe");
    }

    fn u16_at(b: &[u8], at: u64) -> u16 {
        u16::from_le_bytes([b[at as usize], b[at as usize + 1]])
    }

    fn u64_at(b: &[u8], at: u64) -> u64 {
        u64::from_le_bytes(b[at as usize..][..8].try_into().unwrap())
    }

    fn wide_at(b: &[u8], base: u64, addr: u64, units: usize) -> Vec<u16> {
        let off = (addr - base) as usize;
        b[off..off + units * 2].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    }

    #[test]
    fn the_image_places_the_strings_and_the_environment() {
        let base = 0x7_0000_0000;
        let env: Vec<u16> = "A=1\0B=2\0\0".encode_utf16().collect();
        let p = for_program("C:\\Users\\poly\\x.exe", &["arg"], env.clone());
        let img = image(base, &p).unwrap();
        assert_eq!(u16_at(&img, PP_IMAGE_PATH_NAME), (p.image_path.len() * 2) as u16);
        assert_eq!(u16_at(&img, PP_IMAGE_PATH_NAME + 2), (p.image_path.len() * 2 + 2) as u16);
        let path_at = u64_at(&img, PP_IMAGE_PATH_NAME + 8);
        assert_eq!(path_at, base + PARAMETERS_SIZE);
        assert_eq!(wide_at(&img, base, path_at, p.image_path.len() + 1).last(), Some(&0));
        assert_eq!(wide_at(&img, base, path_at, p.image_path.len()), p.image_path);
        let cmd_at = u64_at(&img, PP_COMMAND_LINE + 8);
        assert_eq!(cmd_at % 8, 0);
        assert_eq!(wide_at(&img, base, cmd_at, p.command_line.len()), p.command_line);
        let env_at = u64_at(&img, PP_ENVIRONMENT);
        assert_eq!(env_at % 16, 0);
        assert_eq!(u64_at(&img, PP_ENVIRONMENT_SIZE), (env.len() * 2) as u64);
        assert_eq!(wide_at(&img, base, env_at, env.len()), env);
        assert_eq!(img.len() as u64, env_at - base + (env.len() * 2) as u64);
        // Everything else of the structure is zero.
        for (i, &byte) in img[..PARAMETERS_SIZE as usize].iter().enumerate() {
            let i = i as u64;
            let field = [PP_IMAGE_PATH_NAME, PP_COMMAND_LINE, PP_ENVIRONMENT, PP_ENVIRONMENT_SIZE]
                .iter()
                .any(|&f| (f..f + 16).contains(&i) && (f != PP_ENVIRONMENT || i < f + 8));
            if !field {
                assert_eq!(byte, 0, "byte {i:#x}");
            }
        }
        // Too long for a UNICODE_STRING.
        let long = Parameters { image_path: alloc::vec![b'a' as u16; 40_000], ..Parameters::default() };
        assert_eq!(image(base, &long), Err(STATUS_INVALID_PARAMETER));
    }
}
