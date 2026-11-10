//! The logic of our `shell32.dll` (ADR 0155): `CommandLineToArgvW` and the folder table of
//! `SHGetFolderPathW`, the two shell functions Chrome's processes call before anything else
//! (its crash handler reads its arguments with the first and finds the user's local
//! application data with the second).
//!
//! Sources: Learn, "CommandLineToArgvW function (shellapi.h)" (the backslash and quotation
//! mark rules, the empty command line, the single block freed with `LocalFree`); Learn,
//! "Parsing C++ command-line arguments" (the program name is taken as it stands); Learn,
//! "SHGetFolderPathW function (shlobj_core.h)" and "CSIDL" (the folder ids and their typical
//! paths); the constants are `shlobj_core.h`'s and `winerror.h`'s.

use alloc::vec::Vec;

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;

fn is_space(c: u16) -> bool {
    c == u16::from(b' ') || c == u16::from(b'\t')
}

/// Split a command line into its arguments the way `CommandLineToArgvW` documents it.
///
/// * The first argument is the program name: a part in quotation marks is taken as it stands
///   (the marks are dropped, white space inside does not end it, backslashes are plain
///   characters); it ends at the first white space outside quotation marks.
/// * The other arguments are separated by spaces and tabs. A quotation mark starts or ends a
///   quoted part, in which white space belongs to the argument, and is itself dropped.
/// * 2n backslashes followed by a quotation mark give n backslashes and the mark starts or
///   ends a quoted part; 2n+1 backslashes followed by a quotation mark give n backslashes and
///   a literal quotation mark; backslashes not followed by a quotation mark stand for
///   themselves.
///
/// `cmd` has no terminating NUL. An empty command line gives no arguments here; the caller
/// then returns the executable's path, as Learn says. (Learn does not say what two
/// quotation marks in a row inside a quoted part mean; here the second one ends the part
/// like any other.)
pub fn command_line_to_argv(cmd: &[u16]) -> Vec<Vec<u16>> {
    let mut args = Vec::new();
    if cmd.is_empty() {
        return args;
    }
    // The program name.
    let mut i = 0;
    let mut name = Vec::new();
    let mut quoted = false;
    while i < cmd.len() {
        let c = cmd[i];
        if c == QUOTE {
            quoted = !quoted;
        } else if is_space(c) && !quoted {
            break;
        } else {
            name.push(c);
        }
        i += 1;
    }
    args.push(name);
    // The arguments.
    loop {
        while i < cmd.len() && is_space(cmd[i]) {
            i += 1;
        }
        if i >= cmd.len() {
            break;
        }
        let mut arg = Vec::new();
        let mut quoted = false;
        while i < cmd.len() {
            let c = cmd[i];
            if c == BACKSLASH {
                let start = i;
                while i < cmd.len() && cmd[i] == BACKSLASH {
                    i += 1;
                }
                let n = i - start;
                if i < cmd.len() && cmd[i] == QUOTE {
                    arg.extend(core::iter::repeat_n(BACKSLASH, n / 2));
                    if n % 2 == 1 {
                        arg.push(QUOTE);
                    } else {
                        quoted = !quoted;
                    }
                    i += 1;
                } else {
                    arg.extend(core::iter::repeat_n(BACKSLASH, n));
                }
            } else if c == QUOTE {
                quoted = !quoted;
                i += 1;
            } else if is_space(c) && !quoted {
                break;
            } else {
                arg.push(c);
                i += 1;
            }
        }
        args.push(arg);
    }
    args
}

/// The size in bytes of the block `CommandLineToArgvW` returns for these arguments: the
/// pointers (one per argument and a null one after them), then the NUL-terminated strings.
pub fn argv_block_size(args: &[Vec<u16>]) -> usize {
    (args.len() + 1) * 8 + args.iter().map(|a| (a.len() + 1) * 2).sum::<usize>()
}

/// Fill the block (of [`argv_block_size`] bytes, at address `base` in the caller's memory):
/// "a contiguous block of memory for pointers to the argument strings, and for the argument
/// strings themselves" (Learn), so one `LocalFree` releases everything.
pub fn write_argv_block(args: &[Vec<u16>], base: u64, block: &mut [u8]) {
    let mut at = (args.len() + 1) * 8;
    for (n, a) in args.iter().enumerate() {
        block[n * 8..n * 8 + 8].copy_from_slice(&(base + at as u64).to_le_bytes());
        for c in a.iter().chain(core::iter::once(&0)) {
            block[at..at + 2].copy_from_slice(&c.to_le_bytes());
            at += 2;
        }
    }
    block[args.len() * 8..args.len() * 8 + 8].copy_from_slice(&0u64.to_le_bytes());
}

/// `CSIDL_FLAG_MASK` (`shlobj_core.h`): the bits of `nFolder` that are flags.
pub const CSIDL_FLAG_MASK: u32 = 0xFF00;
/// `CSIDL_FLAG_CREATE` (`shlobj_core.h`): create the folder if it does not exist.
pub const CSIDL_FLAG_CREATE: u32 = 0x8000;
/// `CSIDL_APPDATA` (`shlobj_core.h`).
pub const CSIDL_APPDATA: u32 = 0x001A;
/// `CSIDL_LOCAL_APPDATA` (`shlobj_core.h`).
pub const CSIDL_LOCAL_APPDATA: u32 = 0x001C;
/// `CSIDL_COMMON_APPDATA` (`shlobj_core.h`).
pub const CSIDL_COMMON_APPDATA: u32 = 0x0023;
/// `CSIDL_WINDOWS` (`shlobj_core.h`).
pub const CSIDL_WINDOWS: u32 = 0x0024;
/// `CSIDL_SYSTEM` (`shlobj_core.h`).
pub const CSIDL_SYSTEM: u32 = 0x0025;
/// `CSIDL_PROGRAM_FILES` (`shlobj_core.h`).
pub const CSIDL_PROGRAM_FILES: u32 = 0x0026;
/// `CSIDL_PROFILE` (`shlobj_core.h`).
pub const CSIDL_PROFILE: u32 = 0x0028;
/// The highest folder id `shlobj_core.h` defines (`CSIDL_COMPUTERSNEARME`).
pub const CSIDL_LAST: u32 = 0x003D;

/// `S_OK` (`winerror.h`).
pub const S_OK: u32 = 0;
/// `E_FAIL` (`winerror.h`).
pub const E_FAIL: u32 = 0x8000_4005;
/// `E_INVALIDARG` (`winerror.h`).
pub const E_INVALIDARG: u32 = 0x8007_0057;

/// Where a folder is: the environment variable that holds its path (the session manager
/// sets them from the user's profile, ADR 0073) and what follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Folder {
    pub variable: &'static str,
    pub suffix: &'static str,
}

/// The folder of a `nFolder` value (flags allowed), as Learn's CSIDL page gives its typical
/// path: `Err(E_INVALIDARG)` for a value that is no CSIDL, `Err(E_FAIL)` for a folder Poly
/// does not have yet.
pub fn folder(n_folder: u32) -> Result<Folder, u32> {
    let f = |variable, suffix| Ok(Folder { variable, suffix });
    match n_folder & !CSIDL_FLAG_MASK {
        CSIDL_APPDATA => f("APPDATA", ""),
        CSIDL_LOCAL_APPDATA => f("LOCALAPPDATA", ""),
        CSIDL_COMMON_APPDATA => f("ProgramData", ""),
        CSIDL_WINDOWS => f("SystemRoot", ""),
        CSIDL_SYSTEM => f("SystemRoot", "\\System32"),
        CSIDL_PROGRAM_FILES => f("ProgramFiles", ""),
        CSIDL_PROFILE => f("USERPROFILE", ""),
        n if n <= CSIDL_LAST => Err(E_FAIL),
        _ => Err(E_INVALIDARG),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    fn split(s: &str) -> Vec<String> {
        let w: Vec<u16> = s.encode_utf16().collect();
        command_line_to_argv(&w).iter().map(|a| String::from_utf16(a).unwrap()).collect()
    }

    #[test]
    fn arguments_are_separated_by_spaces_and_tabs() {
        assert_eq!(split("a.exe b c"), ["a.exe", "b", "c"]);
        assert_eq!(split("a.exe \t b\t\tc  "), ["a.exe", "b", "c"]);
        assert_eq!(split("a.exe"), ["a.exe"]);
        assert!(split("").is_empty());
    }

    #[test]
    fn the_program_name_is_taken_as_it_stands() {
        assert_eq!(split("\"C:\\Program Files\\Chrome\\chrome.exe\" --x"), ["C:\\Program Files\\Chrome\\chrome.exe", "--x"]);
        // Backslashes before the closing mark are plain characters in the name.
        assert_eq!(split("\"C:\\dir\\\" b"), ["C:\\dir\\", "b"]);
        assert_eq!(split("C:\\a\\b.exe c"), ["C:\\a\\b.exe", "c"]);
    }

    #[test]
    fn quotation_marks_keep_white_space_in_an_argument() {
        assert_eq!(split("x \"a b\" c"), ["x", "a b", "c"]);
        assert_eq!(split("x \"--user-data-dir=C:\\my profile\" /prefetch:4"), ["x", "--user-data-dir=C:\\my profile", "/prefetch:4"]);
        assert_eq!(split("x d\"e f\"g h"), ["x", "de fg", "h"]);
        assert_eq!(split("x \"\" y"), ["x", "", "y"]);
    }

    /// Learn's three rules, with n = 1 and n = 2.
    #[test]
    fn backslashes_before_a_quotation_mark() {
        // 2n backslashes + mark: n backslashes, the mark starts or ends a quoted part.
        assert_eq!(split("x \"a b\\\\\" c"), ["x", "a b\\", "c"]);
        assert_eq!(split("x \\\\\\\\\"a b\""), ["x", "\\\\a b"]);
        // 2n+1 backslashes + mark: n backslashes and a literal mark.
        assert_eq!(split("x \\\"a\\\" b"), ["x", "\"a\"", "b"]);
        assert_eq!(split("x \"a\\\\\\\"b\" c"), ["x", "a\\\"b", "c"]);
        // Backslashes not followed by a mark stand for themselves.
        assert_eq!(split("x a\\\\\\b C:\\dir\\"), ["x", "a\\\\\\b", "C:\\dir\\"]);
    }

    #[test]
    fn the_block_is_the_pointers_then_the_strings() {
        let args = command_line_to_argv(&"p.exe ab c".encode_utf16().collect::<Vec<u16>>());
        let size = argv_block_size(&args);
        assert_eq!(size, 4 * 8 + (6 + 3 + 2) * 2);
        let mut block = std::vec![0xAAu8; size];
        write_argv_block(&args, 0x1000, &mut block);
        let ptr = |n: usize| u64::from_le_bytes(block[n * 8..n * 8 + 8].try_into().unwrap());
        assert_eq!((ptr(0), ptr(1), ptr(2), ptr(3)), (0x1020, 0x102C, 0x1032, 0));
        let text = |at: u64| {
            let mut o = (at - 0x1000) as usize;
            let mut s = Vec::new();
            loop {
                let c = u16::from_le_bytes([block[o], block[o + 1]]);
                if c == 0 {
                    return String::from_utf16(&s).unwrap();
                }
                s.push(c);
                o += 2;
            }
        };
        assert_eq!((text(ptr(0)), text(ptr(1)), text(ptr(2))), ("p.exe".into(), "ab".into(), "c".into()));
        // No arguments: just the null pointer.
        assert_eq!(argv_block_size(&[]), 8);
    }

    #[test]
    fn folders_come_from_the_profile_variables() {
        assert_eq!(folder(0x1C), Ok(Folder { variable: "LOCALAPPDATA", suffix: "" }));
        assert_eq!(folder(0x1A), Ok(Folder { variable: "APPDATA", suffix: "" }));
        assert_eq!(folder(0x28), Ok(Folder { variable: "USERPROFILE", suffix: "" }));
        assert_eq!(folder(0x25), Ok(Folder { variable: "SystemRoot", suffix: "\\System32" }));
        // The flags do not change the folder.
        assert_eq!(folder(0x1C | CSIDL_FLAG_CREATE), folder(0x1C));
        // A CSIDL Poly has no folder for yet, and a value that is none.
        assert_eq!(folder(0x05), Err(E_FAIL));
        assert_eq!(folder(0x7F), Err(E_INVALIDARG));
        assert_eq!((S_OK, E_FAIL, E_INVALIDARG), (0, 0x8000_4005, 0x8007_0057));
    }
}
