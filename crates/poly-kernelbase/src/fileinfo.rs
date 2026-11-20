//! The logic of `shell32!SHGetFileInfoW` (sheet `specs/windows/shell32.dll/SHGetFileInfoW.yaml`):
//! the type name, the display name, the attributes and the icon of a file, folder or drive root
//! from its name and whether it is a folder; and the executable type from a file's first bytes.
//! `shell32` looks the path up and fills the caller's `SHFILEINFOW`.
//!
//! Sources (clean room): the sheet; Learn: "SHGetFileInfoW", "SHFILEINFOW", "SFGAO"; the
//! constants are `shellapi.h`'s and `shobjidl_core.h`'s.

use alloc::vec::Vec;

/// `SHGFI_*` (`shellapi.h`).
pub mod shgfi {
    pub const LARGEICON: u32 = 0x0;
    pub const SMALLICON: u32 = 0x1;
    pub const OPENICON: u32 = 0x2;
    pub const SHELLICONSIZE: u32 = 0x4;
    pub const PIDL: u32 = 0x8;
    pub const USEFILEATTRIBUTES: u32 = 0x10;
    pub const ICON: u32 = 0x100;
    pub const DISPLAYNAME: u32 = 0x200;
    pub const TYPENAME: u32 = 0x400;
    pub const ATTRIBUTES: u32 = 0x800;
    pub const ICONLOCATION: u32 = 0x1000;
    pub const EXETYPE: u32 = 0x2000;
    pub const SYSICONINDEX: u32 = 0x4000;
}

/// `FILE_ATTRIBUTE_DIRECTORY` (`winnt.h`).
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// What a name stands for: a drive root (`C:\`), a folder or a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    DriveRoot,
    Folder,
    File,
}

/// The icon of a name in the system image list: files with the same registered icon share one
/// (the sheet's `icon_index`), so this is also the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Icon {
    /// An unknown extension, no extension, a name that starts with a dot.
    Generic = 0,
    Folder = 1,
    Text = 2,
    Program = 3,
    Library = 4,
    Drive = 5,
    Zip = 6,
}

impl Icon {
    /// Every icon, in the order of the system image list.
    pub const ALL: [Icon; 7] =
        [Icon::Generic, Icon::Folder, Icon::Text, Icon::Program, Icon::Library, Icon::Drive, Icon::Zip];

    /// Its index in the system image list.
    pub fn index(self) -> i32 {
        self as i32
    }
}

const BACKSLASH: u16 = b'\\' as u16;
const DOT: u16 = b'.' as u16;

fn lower(c: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c }
}

fn eq_ascii(a: &[u16], b: &str) -> bool {
    a.len() == b.len() && a.iter().zip(b.bytes()).all(|(&x, y)| lower(x) == u16::from(y))
}

/// The path without trailing backslashes, except the one of a drive root (`C:\`).
pub fn trimmed(path: &[u16]) -> &[u16] {
    let mut end = path.len();
    while end > 0 && path[end - 1] == BACKSLASH && !is_drive_root(&path[..end]) {
        end -= 1;
    }
    &path[..end]
}

/// Whether the path is a drive root: `C:\` or `C:`.
pub fn is_drive_root(path: &[u16]) -> bool {
    let letter = |c: u16| (u16::from(b'a')..=u16::from(b'z')).contains(&lower(c));
    match path {
        [d, c] => letter(*d) && *c == u16::from(b':'),
        [d, c, s] => letter(*d) && *c == u16::from(b':') && *s == BACKSLASH,
        _ => false,
    }
}

/// What `path` stands for, given whether it is (or, with `SHGFI_USEFILEATTRIBUTES`, is said to be)
/// a directory.
pub fn kind(path: &[u16], directory: bool) -> Kind {
    if is_drive_root(trimmed(path)) {
        Kind::DriveRoot
    } else if directory {
        Kind::Folder
    } else {
        Kind::File
    }
}

/// The text after the last backslash (of the path without its trailing backslashes).
pub fn file_name(path: &[u16]) -> &[u16] {
    let p = trimmed(path);
    let start = p.iter().rposition(|&c| c == BACKSLASH || c == u16::from(b'/')).map_or(0, |i| i + 1);
    &p[start..]
}

/// The extension of a file name without its dot (`txt`), if it has one: what follows the last dot
/// (a name that starts with a dot has its whole name after it as the extension, `.hidden`).
pub fn extension(name: &[u16]) -> Option<&[u16]> {
    let i = name.iter().rposition(|&c| c == DOT)?;
    if i + 1 == name.len() { None } else { Some(&name[i + 1..]) }
}

/// The extensions whose type Windows registers, with their type name, whether the display name
/// hides the extension (the oracle hides the extensions of known types, but a `.dll` keeps it), and
/// their icon.
const KNOWN: [(&str, &str, bool, Icon); 4] = [
    ("txt", "Text Document", true, Icon::Text),
    ("dll", "Application extension", false, Icon::Library),
    ("exe", "Application", true, Icon::Program),
    ("zip", "Compressed (zipped) Folder", true, Icon::Zip),
];

fn known(name: &[u16]) -> Option<&'static (&'static str, &'static str, bool, Icon)> {
    let ext = extension(name)?;
    KNOWN.iter().find(|k| eq_ascii(ext, k.0))
}

/// The type name (the sheet's `type_names`).
pub fn type_name(path: &[u16], kind: Kind) -> Vec<u16> {
    match kind {
        Kind::DriveRoot => "Local Disk".encode_utf16().collect(),
        Kind::Folder => "File folder".encode_utf16().collect(),
        Kind::File => {
            let name = file_name(path);
            if let Some(k) = known(name) {
                return k.1.encode_utf16().collect();
            }
            match extension(name) {
                Some(ext) => {
                    let mut out: Vec<u16> = ext
                        .iter()
                        .map(|&c| if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c })
                        .collect();
                    out.extend(" File".encode_utf16());
                    out
                }
                None => "File".encode_utf16().collect(),
            }
        }
    }
}

/// The display name (the sheet's `display_names`): the file name, without the extension of a known
/// type that hides it; a drive root is `Local Disk (C:)`.
pub fn display_name(path: &[u16], kind: Kind) -> Vec<u16> {
    match kind {
        Kind::DriveRoot => {
            let mut out: Vec<u16> = "Local Disk (".encode_utf16().collect();
            out.push(lower(trimmed(path)[0]) - 32);
            out.extend(":)".encode_utf16());
            out
        }
        Kind::Folder => file_name(path).to_vec(),
        Kind::File => {
            let name = file_name(path);
            match known(name) {
                Some(&(_, _, true, _)) => {
                    let dot = name.iter().rposition(|&c| c == DOT).unwrap_or(name.len());
                    if dot == 0 { name.to_vec() } else { name[..dot].to_vec() }
                }
                _ => name.to_vec(),
            }
        }
    }
}

/// `SFGAO_*` (`shobjidl_core.h`).
pub mod sfgao {
    pub const CANCOPY: u32 = 0x1;
    pub const CANMOVE: u32 = 0x2;
    pub const CANLINK: u32 = 0x4;
    pub const STORAGE: u32 = 0x8;
    pub const CANRENAME: u32 = 0x10;
    pub const CANDELETE: u32 = 0x20;
    pub const HASPROPSHEET: u32 = 0x40;
    pub const DROPTARGET: u32 = 0x100;
    pub const STREAM: u32 = 0x0040_0000;
    pub const STORAGEANCESTOR: u32 = 0x0080_0000;
    pub const FILESYSANCESTOR: u32 = 0x1000_0000;
    pub const FOLDER: u32 = 0x2000_0000;
    pub const FILESYSTEM: u32 = 0x4000_0000;
    pub const HASSUBFOLDER: u32 = 0x8000_0000;
}

/// The attributes (the sheet's `attributes`) of a name that was looked up (`looked_up`; with
/// `SHGFI_USEFILEATTRIBUTES` a file or folder has none) and, for a folder, whether it has a
/// subfolder.
pub fn attributes(kind: Kind, looked_up: bool, has_subfolder: bool) -> u32 {
    use sfgao::*;
    let base = CANCOPY | CANMOVE | CANLINK | CANRENAME | CANDELETE | HASPROPSHEET | DROPTARGET | FILESYSTEM;
    match kind {
        Kind::DriveRoot => {
            CANCOPY
                | CANLINK
                | STORAGE
                | CANRENAME
                | HASPROPSHEET
                | DROPTARGET
                | STORAGEANCESTOR
                | FILESYSANCESTOR
                | FOLDER
                | FILESYSTEM
                | HASSUBFOLDER
        }
        _ if !looked_up => 0,
        Kind::Folder => {
            let sub = if has_subfolder { HASSUBFOLDER } else { 0 };
            base | STORAGE | STORAGEANCESTOR | FILESYSANCESTOR | FOLDER | sub
        }
        Kind::File => base | STREAM,
    }
}

/// The icon of a name.
pub fn icon(path: &[u16], kind: Kind) -> Icon {
    match kind {
        Kind::DriveRoot => Icon::Drive,
        Kind::Folder => Icon::Folder,
        Kind::File => known(file_name(path)).map_or(Icon::Generic, |k| k.3),
    }
}

/// `SHGFI_EXETYPE` from a file's first bytes (`head`, at least the headers): a console program
/// is `'PE'` (0x4550) with 0 in the high word (the sheet's `exe_type`); a windowed one has its
/// subsystem version in the high word, an MS-DOS program `'MZ'` (Learn, "SHGetFileInfoW"); anything
/// else 0.
pub fn exe_type(head: &[u8]) -> usize {
    let u16_at = |o: usize| head.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| head.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u16_at(0) != Some(0x5A4D) {
        return 0;
    }
    let Some(pe) = u32_at(0x3C).map(|v| v as usize) else { return 0x5A4D };
    if u32_at(pe) != Some(0x0000_4550) {
        return 0x5A4D;
    }
    // The optional header follows the 20-byte file header; its subsystem version is at 48 and 50,
    // the subsystem at 68 (the same offsets in PE32 and PE32+).
    let opt = pe + 24;
    let (Some(subsystem), Some(major), Some(minor)) = (u16_at(opt + 68), u16_at(opt + 48), u16_at(opt + 50)) else {
        return 0;
    };
    // IMAGE_SUBSYSTEM_WINDOWS_CUI (winnt.h).
    if subsystem == 3 {
        return 0x4550;
    }
    0x4550 | (usize::from(major.min(0xFF)) << 24) | (usize::from(minor.min(0xFF)) << 16)
}

/// The picture of an icon of `size` x `size` pixels (16 or 32), top row first, as `0xAARRGGBB`
/// (straight alpha; outside the shape transparent): a page for a file (with lines for text, a
/// square for a library), a window for a program, a folder, a zipped folder, a drive.
pub fn icon_pixels(icon: Icon, size: u32) -> Vec<u32> {
    let n = size as usize;
    let mut px = alloc::vec![0u32; n * n];
    // Coordinates in sixteenths of the icon.
    let u = |v: u32| (v * size / 16) as usize;
    let rect = |px: &mut Vec<u32>, x0: u32, y0: u32, x1: u32, y1: u32, c: u32| {
        for y in u(y0)..u(y1).min(n) {
            for x in u(x0)..u(x1).min(n) {
                px[y * n + x] = c;
            }
        }
    };
    let frame = |px: &mut Vec<u32>, x0: u32, y0: u32, x1: u32, y1: u32, c: u32| {
        let (x0, y0, x1, y1) = (u(x0), u(y0), u(x1).min(n) - 1, u(y1).min(n) - 1);
        for x in x0..=x1 {
            px[y0 * n + x] = c;
            px[y1 * n + x] = c;
        }
        for y in y0..=y1 {
            px[y * n + x0] = c;
            px[y * n + x1] = c;
        }
    };
    const WHITE: u32 = 0xFFFF_FFFF;
    const EDGE: u32 = 0xFF6B_6B6B;
    const FOLDER: u32 = 0xFFF2_C14E;
    const FOLDER_TAB: u32 = 0xFFD9_A23A;
    match icon {
        Icon::Generic | Icon::Text | Icon::Library => {
            rect(&mut px, 3, 1, 13, 15, WHITE);
            frame(&mut px, 3, 1, 13, 15, EDGE);
            if icon == Icon::Text {
                for y in [5, 7, 9, 11] {
                    rect(&mut px, 5, y, 11, y + 1, 0xFF9A_9A9A);
                }
            }
            if icon == Icon::Library {
                rect(&mut px, 6, 7, 10, 11, 0xFF7B_5EA7);
            }
        }
        Icon::Program => {
            rect(&mut px, 1, 2, 15, 14, WHITE);
            rect(&mut px, 1, 2, 15, 5, 0xFF2B_6CB0);
            frame(&mut px, 1, 2, 15, 14, EDGE);
        }
        Icon::Folder | Icon::Zip => {
            rect(&mut px, 1, 2, 7, 4, FOLDER_TAB);
            rect(&mut px, 1, 4, 15, 14, FOLDER);
            frame(&mut px, 1, 4, 15, 14, FOLDER_TAB);
            if icon == Icon::Zip {
                for y in (4..14).step_by(2) {
                    rect(&mut px, 7, y, 9, y + 1, 0xFF4A_4A4A);
                }
            }
        }
        Icon::Drive => {
            rect(&mut px, 1, 5, 15, 11, 0xFFB8_B8B8);
            frame(&mut px, 1, 5, 15, 11, EDGE);
            rect(&mut px, 12, 8, 14, 9, 0xFF3C_B34A);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn s(v: &[u16]) -> String {
        String::from_utf16_lossy(v)
    }

    #[test]
    fn type_names_by_extension() {
        let t = |p: &str, dir: bool| s(&type_name(&w(p), kind(&w(p), dir)));
        assert_eq!(t(r"C:\x\file.txt", false), "Text Document");
        assert_eq!(t(r"C:\x\FILE.TXT", false), "Text Document");
        assert_eq!(t("file.dll", false), "Application extension");
        assert_eq!(t("file.exe", false), "Application");
        assert_eq!(t("file.zip", false), "Compressed (zipped) Folder");
        assert_eq!(t("file.polytestxyz", false), "POLYTESTXYZ File");
        assert_eq!(t(".hidden", false), "HIDDEN File");
        assert_eq!(t("file", false), "File");
        assert_eq!(t("folder.txt", true), "File folder");
        assert_eq!(t(r"C:\", true), "Local Disk");
    }

    #[test]
    fn display_names_hide_the_extensions_of_known_types_but_dll() {
        let d = |p: &str, dir: bool| s(&display_name(&w(p), kind(&w(p), dir)));
        assert_eq!(d(r"C:\x\file.txt", false), "file");
        assert_eq!(d("file.exe", false), "file");
        assert_eq!(d("file.zip", false), "file");
        assert_eq!(d("file.dll", false), "file.dll");
        assert_eq!(d("file.polytestxyz", false), "file.polytestxyz");
        assert_eq!(d(".hidden", false), ".hidden");
        assert_eq!(d(r"C:\x\folder.txt", true), "folder.txt");
        assert_eq!(d(r"C:\x\sub.polytestdir\", true), "sub.polytestdir");
        assert_eq!(d(r"c:\", true), "Local Disk (C:)");
    }

    #[test]
    fn attributes_as_recorded() {
        assert_eq!(attributes(Kind::File, true, false), 0x4040_0177);
        assert_eq!(attributes(Kind::Folder, true, false), 0x7080_017F);
        assert_eq!(attributes(Kind::Folder, true, true), 0xF080_017F);
        assert_eq!(attributes(Kind::File, false, false), 0);
        assert_eq!(attributes(Kind::Folder, false, false), 0);
        assert_eq!(attributes(Kind::DriveRoot, false, false), 0xF080_015D);
    }

    #[test]
    fn icons_are_shared_by_type() {
        let i = |p: &str, dir: bool| icon(&w(p), kind(&w(p), dir));
        assert_eq!(i("a.txt", false), i("B.TXT", false));
        assert_eq!(i("a.polytestxyz", false), i("a", false));
        assert_eq!(i("a", false), i(".hidden", false));
        assert_eq!(i("folder.txt", true), i("other", true));
        let distinct =
            [i("a.txt", false), i("a", false), i("f", true), i("a.dll", false), i("a.exe", false), i(r"C:\", true)];
        for (n, a) in distinct.iter().enumerate() {
            assert!(distinct[n + 1..].iter().all(|b| a != b), "{a:?}");
        }
        for (n, k) in Icon::ALL.iter().enumerate() {
            assert_eq!(k.index(), n as i32);
        }
    }

    #[test]
    fn exe_types() {
        let mut pe = alloc::vec![0u8; 512];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3C] = 0x80;
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        let opt = 0x80 + 24;
        pe[opt + 68] = 3;
        assert_eq!(exe_type(&pe), 0x4550);
        pe[opt + 68] = 2;
        pe[opt + 48] = 6;
        assert_eq!(exe_type(&pe), 0x0600_4550);
        assert_eq!(exe_type(b"MZ"), 0x5A4D);
        assert_eq!(exe_type(b"hello"), 0);
        assert_eq!(exe_type(b""), 0);
    }

    #[test]
    fn icon_pictures_have_the_size_and_differ() {
        for size in [16, 32] {
            let all: Vec<Vec<u32>> = Icon::ALL.iter().map(|&i| icon_pixels(i, size)).collect();
            for (n, p) in all.iter().enumerate() {
                assert_eq!(p.len(), (size * size) as usize);
                assert_eq!(p[0], 0, "the corner is transparent");
                assert!(p.iter().any(|&c| c >> 24 == 0xFF));
                assert!(all[n + 1..].iter().all(|q| q != p), "{n} at {size}");
            }
        }
    }
}
