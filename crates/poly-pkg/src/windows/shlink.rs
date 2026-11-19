//! Shell links (`.lnk` files), read with Microsoft's public specification
//! [MS-SHLLINK] "Shell Link (.LNK) Binary File Format" (ADR 0230).
//!
//! A shell link is:
//!
//! 1. `ShellLinkHeader` (2.1), 76 bytes: `HeaderSize` 0x4C, `LinkCLSID`
//!    `{00021401-0000-0000-C000-000000000046}`, `LinkFlags`,
//!    `FileAttributes`, three `FILETIME`s, `FileSize`, `IconIndex`,
//!    `ShowCommand`, `HotKey` and three reserved fields;
//! 2. `LinkTargetIDList` (2.2) if `HasLinkTargetIDList`: `IDListSize`, then
//!    item IDs. Their contents belong to the shell's namespaces and are not
//!    part of the specification, so the target is not read from them;
//! 3. `LinkInfo` (2.3) if `HasLinkInfo`: the target as `LocalBasePath` plus
//!    `CommonPathSuffix` (in the system code page, and in UTF-16 when the
//!    header is 0x24 bytes or more);
//! 4. `StringData` (2.4), each present by its flag, in this order:
//!    `NAME_STRING`, `RELATIVE_PATH`, `WORKING_DIR`,
//!    `COMMAND_LINE_ARGUMENTS`, `ICON_LOCATION`: a count of characters,
//!    then the characters (UTF-16 if `IsUnicode`), no terminator;
//! 5. `ExtraData` (2.5): blocks of `BlockSize` and `BlockSignature`, ended
//!    by a block smaller than 4 bytes. `EnvironmentVariableDataBlock`
//!    (0xA0000001) holds the target with environment variables,
//!    `IconEnvironmentDataBlock` (0xA0000007) the icon's.
//!
//! The target is, in order: the environment block's (`HasExpString`), the
//! `LinkInfo` path, the relative path against the link's own folder.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// `HeaderSize`.
pub const HEADER_SIZE: u32 = 0x4C;
/// `LinkCLSID`, `{00021401-0000-0000-C000-000000000046}`, as its bytes.
pub const LINK_CLSID: [u8; 16] = [0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

/// `LinkFlags` (2.1.1).
pub mod flags {
    pub const HAS_LINK_TARGET_ID_LIST: u32 = 1 << 0;
    pub const HAS_LINK_INFO: u32 = 1 << 1;
    pub const HAS_NAME: u32 = 1 << 2;
    pub const HAS_RELATIVE_PATH: u32 = 1 << 3;
    pub const HAS_WORKING_DIR: u32 = 1 << 4;
    pub const HAS_ARGUMENTS: u32 = 1 << 5;
    pub const HAS_ICON_LOCATION: u32 = 1 << 6;
    pub const IS_UNICODE: u32 = 1 << 7;
    pub const FORCE_NO_LINK_INFO: u32 = 1 << 8;
    pub const HAS_EXP_STRING: u32 = 1 << 9;
    pub const HAS_EXP_ICON: u32 = 1 << 14;
}

/// `LinkInfoFlags` (2.3): `VolumeIDAndLocalBasePath`.
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 1;
/// `EnvironmentVariableDataBlock`, `IconEnvironmentDataBlock` (2.5.4, 2.5.5).
const ENVIRONMENT_BLOCK: u32 = 0xA000_0001;
const ICON_ENVIRONMENT_BLOCK: u32 = 0xA000_0007;
/// Their size: `BlockSize`, `BlockSignature`, `TargetAnsi` (260 bytes),
/// `TargetUnicode` (520 bytes).
const ENVIRONMENT_BLOCK_SIZE: usize = 0x314;

/// Why a file is not a shell link Poly reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// No `ShellLinkHeader` (size or class).
    NotALink,
    /// It ends inside a structure.
    Truncated,
}

/// What a shell link says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub flags: u32,
    /// The target as a Windows path, as the link has it (environment
    /// variables not expanded), if it names one.
    pub target: Option<String>,
    /// `NAME_STRING`: the link's description.
    pub description: String,
    /// `RELATIVE_PATH`.
    pub relative_path: String,
    /// `WORKING_DIR`.
    pub working_dir: String,
    /// `COMMAND_LINE_ARGUMENTS`, as one Windows command-line string.
    pub arguments: String,
    /// `ICON_LOCATION` (or the icon environment block's), and `IconIndex`.
    pub icon_location: String,
    pub icon_index: i32,
    /// `ShowCommand`: 1 normal, 3 maximized, 7 minimized.
    pub show: u32,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A NUL-terminated string of the system code page from `at`. Poly takes
/// the code page to be Windows-1252's Latin-1 range (ADR 0230).
fn ansi_z(b: &[u8], at: usize) -> Option<String> {
    let rest = b.get(at..)?;
    let end = rest.iter().position(|&c| c == 0)?;
    Some(rest[..end].iter().map(|&c| char::from(c)).collect())
}

/// A NUL-terminated UTF-16 string from `at`, within `max` units.
fn wide_z(b: &[u8], at: usize, max: usize) -> Option<String> {
    let mut units = Vec::new();
    for i in 0..max {
        let u = u16_at(b, at + 2 * i)?;
        if u == 0 {
            return Some(String::from_utf16_lossy(&units));
        }
        units.push(u);
    }
    Some(String::from_utf16_lossy(&units))
}

/// The `LinkInfo` structure's local path, if it has one.
fn link_info_path(info: &[u8]) -> Option<String> {
    let header = u32_at(info, 4)? as usize;
    let link_flags = u32_at(info, 8)?;
    if link_flags & VOLUME_ID_AND_LOCAL_BASE_PATH == 0 {
        return None;
    }
    let base = u32_at(info, 16)? as usize;
    let suffix = u32_at(info, 24)? as usize;
    if header >= 0x24 {
        let base_w = u32_at(info, 28)? as usize;
        let suffix_w = u32_at(info, 32)? as usize;
        if base_w != 0 {
            let mut path = wide_z(info, base_w, 32768)?;
            if suffix_w != 0 {
                path.push_str(&wide_z(info, suffix_w, 32768)?);
            }
            return Some(path);
        }
    }
    let mut path = ansi_z(info, base)?;
    if suffix != 0 {
        path.push_str(&ansi_z(info, suffix).unwrap_or_default());
    }
    Some(path)
}

/// The target of an environment block (`TargetUnicode`, else `TargetAnsi`).
fn environment_target(block: &[u8]) -> Option<String> {
    if block.len() < ENVIRONMENT_BLOCK_SIZE {
        return None;
    }
    let wide = wide_z(block, 8 + 260, 260).filter(|s| !s.is_empty());
    wide.or_else(|| ansi_z(&block[..8 + 260], 8)).filter(|s| !s.is_empty())
}

/// Read the shell link in `b`.
pub fn parse(b: &[u8]) -> Result<Link, LinkError> {
    if u32_at(b, 0) != Some(HEADER_SIZE) || b.get(4..20) != Some(&LINK_CLSID[..]) {
        return Err(LinkError::NotALink);
    }
    let t = LinkError::Truncated;
    let mut link = Link {
        flags: u32_at(b, 20).ok_or(t)?,
        icon_index: u32_at(b, 56).ok_or(t)? as i32,
        show: u32_at(b, 60).ok_or(t)?,
        ..Link::default()
    };
    let f = link.flags;
    let mut at = HEADER_SIZE as usize;
    if f & flags::HAS_LINK_TARGET_ID_LIST != 0 {
        at += 2 + usize::from(u16_at(b, at).ok_or(t)?);
    }
    if f & flags::HAS_LINK_INFO != 0 {
        let size = u32_at(b, at).ok_or(t)? as usize;
        let info = b.get(at..at + size).ok_or(t)?;
        if f & flags::FORCE_NO_LINK_INFO == 0 {
            link.target = link_info_path(info).filter(|p| !p.is_empty());
        }
        at += size;
    }
    let unicode = f & flags::IS_UNICODE != 0;
    let mut string = |present: u32| -> Result<String, LinkError> {
        if f & present == 0 {
            return Ok(String::new());
        }
        let count = usize::from(u16_at(b, at).ok_or(t)?);
        at += 2;
        let s = if unicode {
            let bytes = b.get(at..at + 2 * count).ok_or(t)?;
            at += 2 * count;
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        } else {
            let bytes = b.get(at..at + count).ok_or(t)?;
            at += count;
            bytes.iter().map(|&c| char::from(c)).collect()
        };
        Ok(s)
    };
    link.description = string(flags::HAS_NAME)?;
    link.relative_path = string(flags::HAS_RELATIVE_PATH)?;
    link.working_dir = string(flags::HAS_WORKING_DIR)?;
    link.arguments = string(flags::HAS_ARGUMENTS)?;
    link.icon_location = string(flags::HAS_ICON_LOCATION)?;
    // ExtraData: blocks until the terminal block (or the end of the file).
    let mut guard = 0;
    while let Some(size) = u32_at(b, at) {
        let size = size as usize;
        if size < 4 || guard > 64 {
            break;
        }
        guard += 1;
        let Some(block) = b.get(at..at + size) else { break };
        match u32_at(block, 4) {
            Some(ENVIRONMENT_BLOCK) if f & flags::HAS_EXP_STRING != 0 => {
                if let Some(target) = environment_target(block) {
                    link.target = Some(target);
                }
            }
            Some(ICON_ENVIRONMENT_BLOCK) if f & flags::HAS_EXP_ICON != 0 => {
                if let Some(icon) = environment_target(block) {
                    link.icon_location = icon;
                }
            }
            _ => {}
        }
        at += size;
    }
    Ok(link)
}

/// Whether `name` is a shell link's file name (`.lnk`, any case).
pub fn is_link_name(name: &str) -> bool {
    name.len() > 4 && name.is_char_boundary(name.len() - 4) && name[name.len() - 4..].eq_ignore_ascii_case(".lnk")
}

/// A shell link as Windows' shell writes one for a program: the header, a
/// `LinkInfo` with the local path (in the code page and, for characters
/// outside it, in UTF-16), the strings in UTF-16 and the terminal block.
/// Poly's tests and fixtures are made with it.
pub fn build(
    target: &str,
    arguments: &str,
    working_dir: &str,
    description: &str,
    icon: Option<(&str, i32)>,
) -> Vec<u8> {
    let mut f = flags::HAS_LINK_INFO | flags::IS_UNICODE;
    for (s, flag) in [
        (description, flags::HAS_NAME),
        (working_dir, flags::HAS_WORKING_DIR),
        (arguments, flags::HAS_ARGUMENTS),
        (icon.map_or("", |i| i.0), flags::HAS_ICON_LOCATION),
    ] {
        if !s.is_empty() {
            f |= flag;
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(&HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&LINK_CLSID);
    out.extend_from_slice(&f.to_le_bytes());
    // FILE_ATTRIBUTE_NORMAL... the times, FileSize: zero.
    out.extend_from_slice(&0x20u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 28]);
    out.extend_from_slice(&icon.map_or(0, |i| i.1).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    debug_assert_eq!(out.len(), HEADER_SIZE as usize);
    // LinkInfo with a 0x24-byte header: VolumeID (a fixed disk, no label),
    // LocalBasePath (code page, '?' for what it lacks), an empty
    // CommonPathSuffix, and the two in UTF-16.
    let ansi: Vec<u8> = target.chars().map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' }).collect();
    let wide: Vec<u8> = target.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let volume: Vec<u8> = [0x11u32, 3, 0x1234_5678, 0x10].iter().flat_map(|v| v.to_le_bytes()).chain([0]).collect();
    let header = 0x24usize;
    let volume_at = header;
    let base_at = volume_at + volume.len();
    let suffix_at = base_at + ansi.len() + 1;
    let base_w = suffix_at + 1;
    let suffix_w = base_w + wide.len();
    let size = suffix_w + 2;
    let mut info = Vec::with_capacity(size);
    for v in [size, header, 1, volume_at, base_at, 0, suffix_at, base_w, suffix_w] {
        info.extend_from_slice(&(v as u32).to_le_bytes());
    }
    info.extend_from_slice(&volume);
    info.extend_from_slice(&ansi);
    info.push(0);
    info.push(0);
    info.extend_from_slice(&wide);
    info.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&info);
    for s in [description, working_dir, arguments, icon.map_or("", |i| i.0)] {
        if s.is_empty() {
            continue;
        }
        let units: Vec<u16> = s.encode_utf16().collect();
        out.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

/// A shell link whose target is only in an environment block
/// (`HasExpString`, `ForceNoLinkInfo`): what an installer writes for a
/// path like `%ProgramFiles%\App\app.exe`.
pub fn build_with_environment(target: &str, arguments: &str) -> Vec<u8> {
    let f = flags::IS_UNICODE
        | flags::HAS_EXP_STRING
        | flags::FORCE_NO_LINK_INFO
        | if arguments.is_empty() { 0 } else { flags::HAS_ARGUMENTS };
    let mut out = Vec::new();
    out.extend_from_slice(&HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&LINK_CLSID);
    out.extend_from_slice(&f.to_le_bytes());
    out.extend_from_slice(&[0u8; 32]);
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    if !arguments.is_empty() {
        let units: Vec<u16> = arguments.encode_utf16().collect();
        out.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    let mut block = alloc::vec![0u8; ENVIRONMENT_BLOCK_SIZE];
    block[0..4].copy_from_slice(&(ENVIRONMENT_BLOCK_SIZE as u32).to_le_bytes());
    block[4..8].copy_from_slice(&ENVIRONMENT_BLOCK.to_le_bytes());
    for (i, c) in target.bytes().take(259).enumerate() {
        block[8 + i] = c;
    }
    for (i, u) in target.encode_utf16().take(259).enumerate() {
        block[8 + 260 + 2 * i..8 + 260 + 2 * i + 2].copy_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&block);
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

/// The program a link starts and its arguments, if it starts one: the
/// target, else the relative path (against `link_dir`, the link's own
/// folder as a Windows path).
pub fn program(link: &Link, link_dir: &str) -> Option<String> {
    if let Some(t) = &link.target {
        return Some(t.clone());
    }
    if link.relative_path.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = link_dir.split('\\').filter(|p| !p.is_empty()).collect();
    for p in link.relative_path.split('\\') {
        match p {
            "" | "." => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("\\")).filter(|p| !p.is_empty()).map(|p| p.to_string())
}
