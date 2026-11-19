//! Reading a `.lnk` file (MS-SHLLINK 2; the sheet's `load_*` and `reading_a_link` rules).
//!
//! Only the header is checked: its size must be 0x4C and its class `CLSID_ShellLink` (else
//! `E_FAIL`); an empty file is an empty link. Everything after the header is read as far as it is
//! whole: a list cut short, a missing terminal block, bytes after it and unknown extra blocks are
//! not errors.

use crate::idlist::{self, guid_from};
use crate::props::Store;
use crate::{CLSID_SHELL_LINK, FileInfo, Link, Text, file_show_command, text};

/// Why a file is not a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Shorter than the header, a header size that is not 0x4C, or another class:
    /// `IPersistFile::Load` answers `E_FAIL`.
    NotALink,
}

/// The `LinkFlags` bits (MS-SHLLINK 2.1.1).
pub mod flags {
    pub const HAS_LINK_TARGET_ID_LIST: u32 = 1 << 0;
    pub const HAS_LINK_INFO: u32 = 1 << 1;
    pub const HAS_NAME: u32 = 1 << 2;
    pub const HAS_RELATIVE_PATH: u32 = 1 << 3;
    pub const HAS_WORKING_DIR: u32 = 1 << 4;
    pub const HAS_ARGUMENTS: u32 = 1 << 5;
    pub const HAS_ICON_LOCATION: u32 = 1 << 6;
    pub const IS_UNICODE: u32 = 1 << 7;
    pub const HAS_EXP_STRING: u32 = 1 << 9;
    pub const HAS_EXP_ICON: u32 = 1 << 14;
}

/// The extra data block signatures (MS-SHLLINK 2.5).
pub mod block {
    pub const ENVIRONMENT: u32 = 0xA000_0001;
    pub const TRACKER: u32 = 0xA000_0003;
    pub const SPECIAL_FOLDER: u32 = 0xA000_0005;
    pub const ICON_ENVIRONMENT: u32 = 0xA000_0007;
    pub const PROPERTY_STORE: u32 = 0xA000_0009;
    pub const KNOWN_FOLDER: u32 = 0xA000_000B;
    pub const VISTA_ID_LIST: u32 = 0xA000_000C;
}

/// The size of the header (`HeaderSize`).
pub const HEADER_SIZE: usize = 0x4C;

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?]))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    let b = d.get(at..at + 8)?;
    Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// A NUL-terminated ANSI string at `at`.
fn ansi_z(d: &[u8], at: usize) -> Option<Text> {
    let rest = d.get(at..)?;
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    Some(text::from_ansi(&rest[..end]))
}

/// A NUL-terminated UTF-16 string in a fixed field.
fn unicode_z(d: &[u8]) -> Text {
    d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect()
}

/// The link a file holds ([`Link`], with the parts a launcher needs; see [`Link::target`]).
pub fn parse(data: &[u8]) -> Result<Link, Error> {
    let mut link = Link::new();
    if data.is_empty() {
        return Ok(link);
    }
    if data.len() < HEADER_SIZE
        || u32_at(data, 0) != Some(HEADER_SIZE as u32)
        || guid_from(&data[4..20]) != Some(CLSID_SHELL_LINK)
    {
        return Err(Error::NotALink);
    }
    let f = u32_at(data, 0x14).unwrap_or(0);
    link.find = FileInfo {
        attributes: u32_at(data, 0x18).unwrap_or(0),
        creation: u64_at(data, 0x1C).unwrap_or(0),
        access: u64_at(data, 0x24).unwrap_or(0),
        write: u64_at(data, 0x2C).unwrap_or(0),
        size: u64::from(u32_at(data, 0x34).unwrap_or(0)),
    };
    link.icon_index = u32_at(data, 0x38).unwrap_or(0) as i32;
    link.show_command = file_show_command(u32_at(data, 0x3C).unwrap_or(1) as i32);
    link.hotkey = u16_at(data, 0x40).unwrap_or(0);
    let mut at = HEADER_SIZE;
    let _ = read_body(data, f, &mut at, &mut link);
    Ok(link)
}

/// The parts after the header; `None` where the file ends early (what was read stays).
fn read_body(d: &[u8], f: u32, at: &mut usize, link: &mut Link) -> Option<()> {
    if f & flags::HAS_LINK_TARGET_ID_LIST != 0 {
        let size = usize::from(u16_at(d, *at)?);
        let list = d.get(*at + 2..(*at + 2 + size).min(d.len()))?;
        link.path = idlist::path_of(list);
        *at += 2 + size;
    }
    if f & flags::HAS_LINK_INFO != 0 {
        let size = u32_at(d, *at)? as usize;
        let info = d.get(*at..(*at + size).min(d.len()))?;
        link.link_info_path = link_info_path(info);
        *at += size.max(4);
    }
    let unicode = f & flags::IS_UNICODE != 0;
    for bit in [
        flags::HAS_NAME,
        flags::HAS_RELATIVE_PATH,
        flags::HAS_WORKING_DIR,
        flags::HAS_ARGUMENTS,
        flags::HAS_ICON_LOCATION,
    ] {
        if f & bit == 0 {
            continue;
        }
        let n = usize::from(u16_at(d, *at)?);
        let bytes = if unicode { 2 * n } else { n };
        let raw = d.get(*at + 2..*at + 2 + bytes)?;
        let s: Text = if unicode {
            raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
        } else {
            text::from_ansi(raw)
        };
        *at += 2 + bytes;
        let slot = match bit {
            flags::HAS_NAME => &mut link.description,
            flags::HAS_RELATIVE_PATH => &mut link.relative_path,
            flags::HAS_WORKING_DIR => &mut link.working_dir,
            flags::HAS_ARGUMENTS => &mut link.arguments,
            _ => &mut link.icon_path,
        };
        *slot = Some(s);
    }
    // Extra data blocks up to the terminal block (a size under 4).
    loop {
        let size = u32_at(d, *at)? as usize;
        if size < 8 {
            return Some(());
        }
        let blk = d.get(*at..(*at + size).min(d.len()))?;
        *at += size;
        match u32_at(blk, 4)? {
            block::ENVIRONMENT if f & flags::HAS_EXP_STRING != 0 => link.env_path = environment_text(blk),
            block::ICON_ENVIRONMENT if f & flags::HAS_EXP_ICON != 0 => link.icon_env = environment_text(blk),
            block::PROPERTY_STORE => link.properties = Store::deserialize(blk.get(8..)?),
            _ => {}
        }
    }
}

/// The text of an `EnvironmentVariableDataBlock` or `IconEnvironmentDataBlock` (MS-SHLLINK
/// 2.5.4, 2.5.5): `TargetUnicode` at offset 268 when it is not empty, else `TargetAnsi` at 8.
fn environment_text(blk: &[u8]) -> Option<Text> {
    let wide = unicode_z(blk.get(268..788.min(blk.len())).unwrap_or(&[]));
    let t = if wide.is_empty() { ansi_z(&blk[..268.min(blk.len())], 8)? } else { wide };
    (!t.is_empty()).then_some(t)
}

/// The local base path of a `LinkInfo` (MS-SHLLINK 2.3): `LocalBasePathUnicode` when the header is
/// the long one (0x24 bytes or more), else `LocalBasePath`, followed by the common path suffix.
fn link_info_path(info: &[u8]) -> Option<Text> {
    let header = u32_at(info, 4)? as usize;
    let lflags = u32_at(info, 8)?;
    if lflags & 1 == 0 {
        return None;
    }
    let mut path = if header >= 0x24 && u32_at(info, 0x1C).is_some_and(|o| o != 0) {
        let o = u32_at(info, 0x1C)? as usize;
        unicode_z(info.get(o..)?)
    } else {
        ansi_z(info, u32_at(info, 16)? as usize)?
    };
    let suffix_at = u32_at(info, 24).unwrap_or(0) as usize;
    if suffix_at != 0
        && let Some(suffix) = ansi_z(info, suffix_at)
        && !suffix.is_empty()
    {
        if path.last() != Some(&u16::from(b'\\')) {
            path.push(u16::from(b'\\'));
        }
        path.extend(suffix);
    }
    (!path.is_empty()).then_some(path)
}
