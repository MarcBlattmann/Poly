//! `SHGetFileInfoW`'s rules (spec-win-apps-5: `specs/windows/shell32.dll/SHGetFileInfoW.yaml`):
//! the type name, the display name, the attributes and the icon index of a file, a folder or a
//! drive root, from its name and kind; plus the layout of `SHFILEINFOW`.
//!
//! Sources: Learn, "SHGetFileInfoW function (shellapi.h)", "SHFILEINFOW", "SFGAO" (the
//! attribute bits); the sheet for what the oracle answered.

use alloc::vec::Vec;

/// `SHGFI_*` (`shellapi.h`).
pub const SHGFI_ICON: u32 = 0x100;
pub const SHGFI_DISPLAYNAME: u32 = 0x200;
pub const SHGFI_TYPENAME: u32 = 0x400;
pub const SHGFI_ATTRIBUTES: u32 = 0x800;
pub const SHGFI_ICONLOCATION: u32 = 0x1000;
pub const SHGFI_EXETYPE: u32 = 0x2000;
pub const SHGFI_SYSICONINDEX: u32 = 0x4000;
pub const SHGFI_LINKOVERLAY: u32 = 0x8000;
pub const SHGFI_SELECTED: u32 = 0x10000;
pub const SHGFI_ATTR_SPECIFIED: u32 = 0x20000;
pub const SHGFI_LARGEICON: u32 = 0;
pub const SHGFI_SMALLICON: u32 = 0x1;
pub const SHGFI_OPENICON: u32 = 0x2;
pub const SHGFI_SHELLICONSIZE: u32 = 0x4;
pub const SHGFI_PIDL: u32 = 0x8;
pub const SHGFI_USEFILEATTRIBUTES: u32 = 0x10;

/// `FILE_ATTRIBUTE_DIRECTORY` (`winnt.h`).
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// The `SFGAO` attributes of a file system file, folder, folder with subfolders and drive root
/// as the sheet observed them (`attributes`).
pub const ATTR_FILE: u32 = 0x4040_0177;
pub const ATTR_FOLDER: u32 = 0x7080_017F;
pub const ATTR_HAS_SUBFOLDER: u32 = 0x8000_0000;
pub const ATTR_DRIVE: u32 = 0xF080_015D;

/// What a name is, for the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    /// `C:\`.
    DriveRoot,
}

/// `SHFILEINFOW` field offsets and sizes (`shellapi.h`), for a pointer of `ptr` bytes.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub icon_index: usize,
    pub attributes: usize,
    pub display_name: usize,
    pub type_name: usize,
    pub size: usize,
}

pub const DISPLAY_NAME_UNITS: usize = 260;
pub const TYPE_NAME_UNITS: usize = 80;

pub fn layout(ptr: usize) -> Layout {
    let icon_index = ptr;
    let attributes = icon_index + 4;
    let display_name = attributes + 4;
    let type_name = display_name + 2 * DISPLAY_NAME_UNITS;
    Layout { icon_index, attributes, display_name, type_name, size: type_name + 2 * TYPE_NAME_UNITS }
}

fn lower(c: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c }
}

fn upper(c: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c }
}

/// The text after the last backslash or slash (the whole text when there is none), without a
/// trailing separator.
pub fn last_part(path: &[u16]) -> &[u16] {
    let mut end = path.len();
    while end > 0 && matches!(path[end - 1], 0x5C | 0x2F) {
        end -= 1;
    }
    let start = path[..end].iter().rposition(|&c| c == 0x5C || c == 0x2F).map_or(0, |i| i + 1);
    &path[start..end]
}

/// Whether `path` is a drive root: `C:` or `C:\`.
pub fn is_drive_root(path: &[u16]) -> bool {
    let b = u16::from(b'\\');
    matches!(path, [l, c, ..] if (u16::from(b'A')..=u16::from(b'Z')).contains(&upper(*l)) && *c == u16::from(b':'))
        && (path.len() == 2 || (path.len() == 3 && path[2] == b))
}

/// The extension of a name (without the dot, lower case); `None` for none and for a name that
/// only starts with a dot.
pub fn extension(name: &[u16]) -> Option<Vec<u16>> {
    let dot = name.iter().rposition(|&c| c == u16::from(b'.'))?;
    (dot > 0 && dot + 1 < name.len()).then(|| name[dot + 1..].iter().map(|&c| lower(c)).collect())
}

/// As [`extension`], but a name that only starts with a dot has its text as the extension (the
/// type name of `.hidden` is `HIDDEN File`).
fn type_extension(name: &[u16]) -> Option<Vec<u16>> {
    let dot = name.iter().rposition(|&c| c == u16::from(b'.'))?;
    (dot + 1 < name.len()).then(|| name[dot + 1..].iter().map(|&c| lower(c)).collect())
}

fn eq(a: &[u16], s: &str) -> bool {
    a.len() == s.len() && a.iter().zip(s.bytes()).all(|(&x, y)| x == u16::from(y))
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// The icon kinds and their index in the system image list.
pub mod icon {
    pub const FILE: i32 = 0;
    pub const FOLDER: i32 = 1;
    pub const APPLICATION: i32 = 2;
    pub const LIBRARY: i32 = 3;
    pub const DRIVE: i32 = 4;
    pub const TEXT: i32 = 5;
    pub const ARCHIVE: i32 = 6;
    /// How many images the list starts with.
    pub const COUNT: i32 = 7;
}

/// The icon index of a name of kind `kind`: files with the same registered icon share one; the
/// generic icon belongs to every extension nobody registered (also no extension and a name that
/// starts with a dot); all folders share one, the open folder has none of its own.
pub fn icon_index(path: &[u16], kind: Kind) -> i32 {
    match kind {
        Kind::DriveRoot => icon::DRIVE,
        Kind::Directory => icon::FOLDER,
        Kind::File => match extension(last_part(path)) {
            Some(e) if eq(&e, "txt") || eq(&e, "log") || eq(&e, "ini") => icon::TEXT,
            Some(e) if eq(&e, "exe") => icon::APPLICATION,
            Some(e) if eq(&e, "dll") || eq(&e, "sys") => icon::LIBRARY,
            Some(e) if eq(&e, "zip") || eq(&e, "7z") || eq(&e, "rar") => icon::ARCHIVE,
            _ => icon::FILE,
        },
    }
}

/// The type name: by extension, without regard to case, for names that do not exist as well; a
/// directory is a "File folder", a drive root a "Local Disk".
pub fn type_name(path: &[u16], kind: Kind) -> Vec<u16> {
    match kind {
        Kind::DriveRoot => utf16("Local Disk"),
        Kind::Directory => utf16("File folder"),
        Kind::File => match type_extension(last_part(path)) {
            None => utf16("File"),
            Some(e) if eq(&e, "txt") => utf16("Text Document"),
            Some(e) if eq(&e, "dll") => utf16("Application extension"),
            Some(e) if eq(&e, "exe") => utf16("Application"),
            Some(e) if eq(&e, "zip") => utf16("Compressed (zipped) Folder"),
            Some(e) => {
                let mut t: Vec<u16> = e.iter().map(|&c| upper(c)).collect();
                t.extend(utf16(" File"));
                t
            }
        },
    }
}

/// The display name: the text after the last backslash; the extensions the shell hides for known
/// types (`.txt`, `.exe`, `.zip`) are left off, `.dll` and unknown ones are not; a drive root is
/// `Local Disk (C:)` (`label` is the volume's label, empty for none).
pub fn display_name(path: &[u16], kind: Kind, label: &[u16]) -> Vec<u16> {
    if kind == Kind::DriveRoot {
        let mut t = if label.is_empty() { utf16("Local Disk") } else { label.to_vec() };
        t.extend(utf16(" ("));
        t.push(upper(path[0]));
        t.extend(utf16(":)"));
        return t;
    }
    let name = last_part(path);
    if kind == Kind::File
        && let Some(e) = extension(name)
        && (eq(&e, "txt") || eq(&e, "exe") || eq(&e, "zip"))
    {
        return name[..name.len() - e.len() - 1].to_vec();
    }
    name.to_vec()
}

/// The attributes of something that exists (`has_subfolder`: a directory with a subdirectory);
/// with `SHGFI_USEFILEATTRIBUTES` only a drive root has any.
pub fn attributes(kind: Kind, has_subfolder: bool, used_file_attributes: bool) -> u32 {
    match kind {
        Kind::DriveRoot => ATTR_DRIVE,
        _ if used_file_attributes => 0,
        Kind::File => ATTR_FILE,
        Kind::Directory => ATTR_FOLDER | if has_subfolder { ATTR_HAS_SUBFOLDER } else { 0 },
    }
}

/// `SHGFI_EXETYPE`'s result for the first bytes of a file: `PE` for a portable executable, `MZ`
/// for a DOS one, 0 otherwise (the high word, the version, is 0 for PE).
pub fn exe_type(head: &[u8]) -> u32 {
    if head.len() < 0x40 || &head[..2] != b"MZ" {
        return 0;
    }
    let e = u32::from_le_bytes([head[0x3C], head[0x3D], head[0x3E], head[0x3F]]) as usize;
    match head.get(e..e + 4) {
        Some(b"PE\0\0") => 0x4550,
        _ => 0x5A4D,
    }
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
        String::from_utf16(v).unwrap()
    }

    #[test]
    fn type_names_come_from_the_extension() {
        let t = |p: &str, k| s(&type_name(&w(p), k));
        assert_eq!(t("C:\\x\\FILE.TXT", Kind::File), "Text Document");
        assert_eq!(t("a.dll", Kind::File), "Application extension");
        assert_eq!(t("a.exe", Kind::File), "Application");
        assert_eq!(t("a.zip", Kind::File), "Compressed (zipped) Folder");
        assert_eq!(t("x.polytestxyz", Kind::File), "POLYTESTXYZ File");
        assert_eq!(t(".hidden", Kind::File), "HIDDEN File");
        assert_eq!(t("noext", Kind::File), "File");
        assert_eq!(t("folder.txt", Kind::Directory), "File folder");
        assert_eq!(t("C:\\", Kind::DriveRoot), "Local Disk");
    }

    #[test]
    fn display_names_hide_the_extensions_of_known_types() {
        let d = |p: &str, k| s(&display_name(&w(p), k, &[]));
        assert_eq!(d("C:\\x\\notes.txt", Kind::File), "notes");
        assert_eq!(d("C:\\x\\a.exe", Kind::File), "a");
        assert_eq!(d("C:\\x\\a.zip", Kind::File), "a");
        assert_eq!(d("C:\\x\\file.dll", Kind::File), "file.dll");
        assert_eq!(d("C:\\x\\file.polytestxyz", Kind::File), "file.polytestxyz");
        assert_eq!(d("C:\\x\\.hidden", Kind::File), ".hidden");
        assert_eq!(d("C:\\x\\folder.txt", Kind::Directory), "folder.txt");
        assert_eq!(d("C:\\x\\sub\\", Kind::Directory), "sub");
        assert_eq!(d("c:\\", Kind::DriveRoot), "Local Disk (C:)");
        assert_eq!(s(&display_name(&w("D:"), Kind::DriveRoot, &w("Data"))), "Data (D:)");
    }

    #[test]
    fn icons_are_shared_as_the_sheet_says() {
        let i = |p: &str, k| icon_index(&w(p), k);
        assert_eq!(i("a.txt", Kind::File), i("B.TXT", Kind::File));
        assert_eq!(i("a.polytestxyz", Kind::File), i("noext", Kind::File));
        assert_eq!(i(".hidden", Kind::File), i("noext", Kind::File));
        assert_eq!(i("folder.txt", Kind::Directory), i("sub", Kind::Directory));
        let all = [i("a.txt", Kind::File), i("a.exe", Kind::File), i("a.dll", Kind::File), i("x", Kind::File), i("d", Kind::Directory), i("C:\\", Kind::DriveRoot)];
        for (n, a) in all.iter().enumerate() {
            assert!(all[n + 1..].iter().all(|b| a != b));
        }
    }

    #[test]
    fn attributes_follow_the_sheet() {
        assert_eq!(attributes(Kind::File, false, false), 0x4040_0177);
        assert_eq!(attributes(Kind::Directory, false, false), 0x7080_017F);
        assert_eq!(attributes(Kind::Directory, true, false), 0xF080_017F);
        assert_eq!(attributes(Kind::File, false, true), 0);
        assert_eq!(attributes(Kind::Directory, false, true), 0);
        assert_eq!(attributes(Kind::DriveRoot, false, true), 0xF080_015D);
    }

    #[test]
    fn the_executable_type_is_pe_or_mz() {
        let mut pe = std::vec![0u8; 0x100];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3C] = 0x80;
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        assert_eq!(exe_type(&pe), 0x4550);
        pe[0x80] = 0;
        assert_eq!(exe_type(&pe), 0x5A4D);
        assert_eq!(exe_type(b"hello"), 0);
    }

    #[test]
    fn the_structure_has_the_sizes_learn_gives() {
        assert_eq!(layout(8).size, 8 + 4 + 4 + 520 + 160);
        assert_eq!(layout(4).size, 4 + 4 + 4 + 520 + 160);
        assert!(is_drive_root(&w("C:\\")) && is_drive_root(&w("c:")) && !is_drive_root(&w("C:\\a")));
    }
}
