//! The special folders of the `CSIDL` family and the item identifier lists (`PIDL`s) of files
//! and folders: `SHGetFolderPathW/A`, `SHGetSpecialFolderPathW/A`, `SHGetSpecialFolderLocation`,
//! `SHGetPathFromIDListW`, `ILCreateFromPathW`, `ILClone`, `ILGetSize` and `SHParseDisplayName`
//! (spec-win-apps-5: `specs/windows/shell32.dll/SH*.yaml`).
//!
//! Sources: Learn, "CSIDL" (the folder ids and their typical paths), "SHGetFolderPathW",
//! "SHGetSpecialFolderLocation", "SHGetSpecialFolderPathW", "SHGetPathFromIDListW",
//! "ILCreateFromPathW", "SHParseDisplayName", "ITEMIDLIST"; `shlobj_core.h` for the constants; the
//! sheets for what the functions answer (the table of paths, the errors, the virtual folders).
//!
//! **A list is Poly's own byte layout** behind the public shape Learn documents: a sequence of
//! items, each a 2-byte size (including itself) and data, ended by a size of 0; the empty list
//! (2 zero bytes) is the desktop. The data of an item starts with its kind: `0x1F` a virtual
//! folder (a CLSID follows), `0x2F` a drive (its root as UTF-16 text, `C:\`), `0x31` a directory
//! and `0x32` a file (a 4-byte serial, then the name as UTF-16 text with its NUL). The serial of a
//! list made from a path is a number from a counter (the lists of a real shell hold times and
//! attributes: two lists for the same path are not equal); the lists of the special folders have
//! serial 0 (two lists for the same folder are).

use alloc::vec::Vec;

/// `E_FAIL`, `E_INVALIDARG`, `E_HANDLE`, `S_OK`, `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`.
pub const S_OK: u32 = 0;
pub const E_FAIL: u32 = 0x8000_4005;
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_HANDLE: u32 = 0x8007_0006;
pub const E_FILE_NOT_FOUND: u32 = 0x8007_0002;

/// The bits of a `CSIDL` value that are flags (`CSIDL_FLAG_MASK`), and the ones the functions act on.
pub const FLAG_MASK: u32 = 0xFF00;
pub const FLAG_CREATE: u32 = 0x8000;
pub const FLAG_DONT_VERIFY: u32 = 0x4000;

/// What a `CSIDL` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A directory: the value of an environment variable and what follows it.
    Path { variable: &'static str, suffix: &'static str, list_suffix: &'static str },
    /// A folder that is no directory (Control Panel, the recycle bin, this computer, ...): its CLSID.
    Virtual([u8; 16]),
}

/// A `CSIDL`'s entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub kind: Kind,
}

const fn p(id: u32, variable: &'static str, suffix: &'static str) -> Entry {
    Entry { id, kind: Kind::Path { variable, suffix, list_suffix: suffix } }
}

/// A folder whose name has another spelling in a list than in the path text of `SHGetFolderPath`.
const fn q(id: u32, variable: &'static str, suffix: &'static str, list_suffix: &'static str) -> Entry {
    Entry { id, kind: Kind::Path { variable, suffix, list_suffix } }
}

const fn v(id: u32, clsid: [u8; 16]) -> Entry {
    Entry { id, kind: Kind::Virtual(clsid) }
}

const USER: &str = "USERPROFILE";
const PUBLIC: &str = "PUBLIC";
const DATA: &str = "ProgramData";
const ROOT: &str = "SystemRoot";
const FILES: &str = "ProgramFiles";
const FILES86: &str = "ProgramFiles(x86)";

const START: &str = "\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu";
const CSTART: &str = "\\Microsoft\\Windows\\Start Menu";

/// The CLSIDs of the virtual folders (`shlguid.h` / the shell's well-known folder GUIDs, Learn:
/// "CLSID_MyComputer" and the "Shell Namespace" pages), in their in-memory byte order.
const MY_COMPUTER: [u8; 16] = clsid(0x20D0_4FE0, 0x3AEA, 0x1069, [0xA2, 0xD8, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D]);
const NETWORK: [u8; 16] = clsid(0x208D_2C60, 0x3AEA, 0x1069, [0xA2, 0xD7, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D]);
const CONTROLS: [u8; 16] = clsid(0x21EC_2020, 0x3AEA, 0x1069, [0xA2, 0xDD, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D]);
const PRINTERS: [u8; 16] = clsid(0x2227_A280, 0x3AEA, 0x1069, [0xA2, 0xDE, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D]);
const RECYCLE: [u8; 16] = clsid(0x645F_F040, 0x5081, 0x101B, [0x9F, 0x08, 0x00, 0xAA, 0x00, 0x2F, 0x95, 0x4E]);
const CONNECTIONS: [u8; 16] = clsid(0x7007_ACC7, 0x3202, 0x11D1, [0xAA, 0xD2, 0x00, 0x80, 0x5F, 0xC1, 0x27, 0x0E]);
const INTERNET: [u8; 16] = clsid(0x871C_5380, 0x42A0, 0x1069, [0xA2, 0xEA, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D]);

const fn clsid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

/// The table: Learn's CSIDL page, the paths as the sheets observed them (`SHGetSpecialFolderLocation.yaml`
/// `folders`).
pub const TABLE: &[Entry] = &[
    p(0x00, USER, "\\Desktop"),
    v(0x01, INTERNET),
    p(0x02, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs"),
    v(0x03, CONTROLS),
    v(0x04, PRINTERS),
    p(0x05, USER, "\\Documents"),
    p(0x06, USER, "\\Favorites"),
    p(0x07, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup"),
    p(0x08, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\Recent"),
    p(0x09, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\SendTo"),
    v(0x0A, RECYCLE),
    p(0x0B, USER, START),
    p(0x0D, USER, "\\Music"),
    p(0x0E, USER, "\\Videos"),
    p(0x10, USER, "\\Desktop"),
    v(0x11, MY_COMPUTER),
    v(0x12, NETWORK),
    p(0x13, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\Network Shortcuts"),
    p(0x14, ROOT, "\\Fonts"),
    p(0x15, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\Templates"),
    p(0x16, DATA, CSTART),
    p(0x17, DATA, "\\Microsoft\\Windows\\Start Menu\\Programs"),
    p(0x18, DATA, "\\Microsoft\\Windows\\Start Menu\\Programs\\Startup"),
    p(0x19, PUBLIC, "\\Desktop"),
    p(0x1A, USER, "\\AppData\\Roaming"),
    p(0x1B, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\Printer Shortcuts"),
    p(0x1C, USER, "\\AppData\\Local"),
    p(0x1D, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup"),
    p(0x1E, DATA, "\\Microsoft\\Windows\\Start Menu\\Programs\\Startup"),
    p(0x1F, USER, "\\Favorites"),
    p(0x20, USER, "\\AppData\\Local\\Microsoft\\Windows\\INetCache"),
    p(0x21, USER, "\\AppData\\Local\\Microsoft\\Windows\\INetCookies"),
    p(0x22, USER, "\\AppData\\Local\\Microsoft\\Windows\\History"),
    p(0x23, DATA, ""),
    p(0x24, ROOT, ""),
    q(0x25, ROOT, "\\system32", "\\System32"),
    p(0x26, FILES, ""),
    p(0x27, USER, "\\Pictures"),
    p(0x28, USER, ""),
    p(0x29, ROOT, "\\SysWOW64"),
    p(0x2A, FILES86, ""),
    p(0x2B, FILES, "\\Common Files"),
    p(0x2C, FILES86, "\\Common Files"),
    p(0x2D, DATA, "\\Microsoft\\Windows\\Templates"),
    p(0x2E, PUBLIC, "\\Documents"),
    p(0x2F, DATA, "\\Microsoft\\Windows\\Start Menu\\Programs\\Administrative Tools"),
    p(0x30, USER, "\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Administrative Tools"),
    v(0x31, CONNECTIONS),
    p(0x35, PUBLIC, "\\Music"),
    p(0x36, PUBLIC, "\\Pictures"),
    p(0x37, PUBLIC, "\\Videos"),
    q(0x38, ROOT, "\\resources", "\\Resources"),
    p(0x3B, USER, "\\AppData\\Local\\Microsoft\\Windows\\Burn\\Burn"),
];

/// `CSIDL_COMMON_OEM_LINKS`: a path folder `SHGetFolderPath` knows, which `SHGetSpecialFolderLocation`
/// does not name.
pub const COMMON_OEM_LINKS: u32 = 0x3A;
const OEM_LINKS: Entry = p(COMMON_OEM_LINKS, DATA, "\\OEM Links");

/// The entry of a `CSIDL` value (flags allowed): `None` for a value that is no folder
/// (`E_INVALIDARG`).
pub fn entry(csidl: i32) -> Option<Entry> {
    let id = csidl as u32 & !FLAG_MASK;
    if id == COMMON_OEM_LINKS {
        return Some(OEM_LINKS);
    }
    TABLE.iter().find(|e| e.id == id).copied()
}

/// The text of a path folder: the variable's value without a trailing backslash, then the
/// suffix (`list`: in the spelling a list has). `None` when the variable is not set.
pub fn path_text(entry: &Entry, variable: impl Fn(&str) -> Option<Vec<u16>>, list: bool) -> Option<Vec<u16>> {
    let Kind::Path { variable: name, suffix, list_suffix } = entry.kind else { return None };
    let mut out = variable(name).filter(|v| !v.is_empty())?;
    while out.len() > 3 && out.last() == Some(&u16::from(b'\\')) {
        out.pop();
    }
    out.extend((if list { list_suffix } else { suffix }).encode_utf16());
    Some(out)
}

/// The folder is below the user's profile (`SHGetFolderPath` asks the token for it).
pub fn is_user_folder(entry: &Entry) -> bool {
    matches!(entry.kind, Kind::Path { variable: "USERPROFILE", .. })
}

// ----- item identifier lists ------------------------------------------------------------------

/// The item kinds.
pub const ITEM_VIRTUAL: u8 = 0x1F;
pub const ITEM_DRIVE: u8 = 0x2F;
pub const ITEM_DIRECTORY: u8 = 0x31;
pub const ITEM_FILE: u8 = 0x32;

fn push_item(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&((data.len() + 2) as u16).to_le_bytes());
    out.extend_from_slice(data);
}

/// The list of a virtual folder.
pub fn virtual_list(clsid: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut data = alloc::vec![ITEM_VIRTUAL, 0x50];
    data.extend_from_slice(clsid);
    push_item(&mut out, &data);
    out.extend_from_slice(&[0, 0]);
    out
}

/// The empty list: the desktop.
pub fn empty_list() -> Vec<u8> {
    alloc::vec![0, 0]
}

/// The list of an absolute DOS path (`C:\a\b`; no trailing backslash except for a root):
/// a drive item and one item per name, the last of them a file when `file` is set. `serial`
/// goes into every name item. `None` for a path that is not `X:\...`.
pub fn path_list(path: &[u16], file: bool, serial: u32) -> Option<Vec<u8>> {
    let b = u16::from(b'\\');
    if path.len() < 3 || path[1] != u16::from(b':') || path[2] != b || !is_letter(path[0]) {
        return None;
    }
    let mut out = Vec::new();
    let mut root = alloc::vec![ITEM_DRIVE, 0];
    for &c in &path[..3] {
        root.extend_from_slice(&c.to_le_bytes());
    }
    root.extend_from_slice(&[0, 0]);
    push_item(&mut out, &root);
    let names: Vec<&[u16]> = path[3..].split(|&c| c == b).filter(|n| !n.is_empty()).collect();
    for (i, name) in names.iter().enumerate() {
        let kind = if file && i + 1 == names.len() { ITEM_FILE } else { ITEM_DIRECTORY };
        let mut data = alloc::vec![kind, 0];
        data.extend_from_slice(&serial.to_le_bytes());
        for &c in *name {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.extend_from_slice(&[0, 0]);
        push_item(&mut out, &data);
    }
    out.extend_from_slice(&[0, 0]);
    Some(out)
}

fn is_letter(c: u16) -> bool {
    (u16::from(b'A')..=u16::from(b'Z')).contains(&c) || (u16::from(b'a')..=u16::from(b'z')).contains(&c)
}

/// What a list leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The empty list: the desktop.
    Desktop,
    /// A folder that is no directory.
    Virtual,
    /// A path of the file system.
    Path(Vec<u16>),
    /// Not a list this module made.
    Unknown,
}

/// The items of a list (`None` when the sizes do not add up to a terminated list).
fn items(list: &[u8]) -> Option<Vec<&[u8]>> {
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        let cb = usize::from(u16::from_le_bytes([*list.get(at)?, *list.get(at + 1)?]));
        if cb == 0 {
            return Some(out);
        }
        if cb < 3 {
            return None;
        }
        out.push(list.get(at + 2..at + cb)?);
        at += cb;
    }
}

/// Where a list leads (`SHGetPathFromIDListW`).
pub fn target(list: &[u8]) -> Target {
    let Some(items) = items(list) else { return Target::Unknown };
    let Some(first) = items.first() else { return Target::Desktop };
    match first[0] {
        ITEM_VIRTUAL => return Target::Virtual,
        ITEM_DRIVE => {}
        _ => return Target::Unknown,
    }
    let mut path: Vec<u16> = Vec::new();
    let text = |b: &[u8]| -> Vec<u16> {
        b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect()
    };
    path.extend(text(&first[2..]));
    for (i, it) in items.iter().enumerate().skip(1) {
        if !matches!(it[0], ITEM_DIRECTORY | ITEM_FILE) || it.len() < 6 {
            return Target::Unknown;
        }
        if i > 1 {
            path.push(u16::from(b'\\'));
        }
        path.extend(text(&it[6..]));
    }
    Target::Path(path)
}

/// The size of a list with its terminator (`ILGetSize`); 0 for no list at all.
pub fn size(list: &[u8]) -> usize {
    let mut at = 0;
    while let (Some(&a), Some(&b)) = (list.get(at), list.get(at + 1)) {
        let cb = usize::from(u16::from_le_bytes([a, b]));
        at += 2.max(cb);
        if cb == 0 {
            return at;
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn env(name: &str) -> Option<Vec<u16>> {
        Some(w(match name {
            "USERPROFILE" => "C:\\Users\\poly",
            "SystemRoot" => "C:\\Windows",
            "ProgramData" => "C:\\ProgramData\\",
            "PUBLIC" => "C:\\Users\\Public",
            "ProgramFiles" => "C:\\Program Files",
            _ => return None,
        }))
    }

    #[test]
    fn values_that_are_no_folder_have_no_entry() {
        for bad in [0x0C, 0x0F, 0x32, 0x33, 0x3F, 0x40, 0x7F, 0xFF, 0x7FFF, 0xFFFF, -1, 0x10000, 0x100000] {
            assert!(entry(bad).is_none(), "{bad:#x}");
        }
        // The flags do not matter; a table value does.
        assert_eq!(entry(0x1A | 0x8000), entry(0x1A));
        assert!(entry(0x3A).is_some());
    }

    #[test]
    fn the_table_gives_the_sheets_paths() {
        let s = |csidl: i32, list: bool| {
            String::from_utf16(&path_text(&entry(csidl).unwrap(), env, list).unwrap()).unwrap()
        };
        assert_eq!(s(0x1A, false), "C:\\Users\\poly\\AppData\\Roaming");
        assert_eq!(s(0x23, false), "C:\\ProgramData");
        assert_eq!(s(0x25, false), "C:\\Windows\\system32");
        assert_eq!(s(0x25, true), "C:\\Windows\\System32");
        assert_eq!(s(0x38, false), "C:\\Windows\\resources");
        assert_eq!(s(0x38, true), "C:\\Windows\\Resources");
        assert_eq!(s(0x3A, false), "C:\\ProgramData\\OEM Links");
        assert_eq!(path_text(&entry(0x2A).unwrap(), env, false), None);
        assert!(path_text(&entry(0x11).unwrap(), env, false).is_none());
    }

    #[test]
    fn a_path_round_trips_through_its_list() {
        let list = path_list(&w("C:\\Users\\poly\\notes.txt"), true, 7).unwrap();
        assert_eq!(target(&list), Target::Path(w("C:\\Users\\poly\\notes.txt")));
        assert_eq!(size(&list), list.len());
        let root = path_list(&w("C:\\"), false, 0).unwrap();
        assert_eq!(target(&root), Target::Path(w("C:\\")));
        // The same path, two serials: two lists that differ.
        assert_ne!(path_list(&w("C:\\a"), true, 1), path_list(&w("C:\\a"), true, 2));
    }

    #[test]
    fn paths_that_are_not_absolute_dos_paths_make_no_list() {
        assert!(path_list(&w("C:/a/b"), false, 0).is_none());
        assert!(path_list(&w("a\\b"), false, 0).is_none());
        assert!(path_list(&w("\\\\server\\share"), false, 0).is_none());
    }

    #[test]
    fn the_desktop_is_the_empty_list_and_virtual_folders_have_no_path() {
        assert_eq!(empty_list().len(), 2);
        assert_eq!(target(&empty_list()), Target::Desktop);
        assert_eq!(size(&empty_list()), 2);
        let drives = virtual_list(&MY_COMPUTER);
        assert!(drives.len() > 2);
        assert_eq!(target(&drives), Target::Virtual);
        assert_eq!(size(&[]), 0);
    }
}
