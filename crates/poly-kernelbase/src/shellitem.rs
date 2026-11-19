//! The logic of `shell32.dll`'s shell items, file operations and `ShellExecuteExW` (ADR 0223):
//! what can be decided from the text of a path without touching the file system.
//!
//! - [`parse`]: which parsing names `SHCreateItemFromParsingName` and `SHParseDisplayName` refuse,
//!   which are relative, and the path an item keeps.
//! - [`file_name`], [`parent`], [`display_name`], [`url`]: what `IShellItem` reports.
//! - [`attributes`]: the `SFGAO_*` bits of a file and of a folder.
//! - [`collision_name`]: the name `FOF_RENAMEONCOLLISION` gives a moved or copied file.
//! - [`new_name_is_plain`]: which new names `RenameItem` acts on.
//! - [`expand_command`]: a `shell\<verb>\command` value with the file and the parameters in it.
//! - [`quote_for_command_line`]: the command line `ShellExecuteExW` gives a program.
//!
//! Sources (clean room): the spec sheets `specs/windows/shell32.dll/SHCreateItemFromParsingName.yaml`,
//! `IFileOperation.MoveItem.yaml`, `SHParseDisplayName.yaml` and `ShellExecuteExW.yaml`; Learn:
//! "IShellItem", "SIGDN", "SFGAO", "IFileOperation", "ShellExecuteExW", "SHELLEXECUTEINFOW",
//! "Launching Applications (ShellExecute, ShellExecuteEx, SHELLEXECUTEINFO)", "File Types";
//! the constants are `shobjidl_core.h`'s, `shtypes.h`'s, `shellapi.h`'s and `winerror.h`'s.

use alloc::vec::Vec;

/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`.
pub const E_FILE_NOT_FOUND: u32 = 0x8007_0002;
/// `E_INVALIDARG`.
pub const E_INVALIDARG: u32 = 0x8007_0057;

const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const COLON: u16 = b':' as u16;
const DOT: u16 = b'.' as u16;

/// What a parsing name is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// The empty string: the desktop, an item with no file-system path.
    Desktop,
    /// A full path (`X:\...` or `\\server\share\...`), as the item keeps it: the trailing
    /// backslash of a folder is dropped, the one of a drive's root (`C:\`) stays.
    Path(Vec<u16>),
}

fn is_letter(c: u16) -> bool {
    (u16::from(b'a')..=u16::from(b'z')).contains(&c) || (u16::from(b'A')..=u16::from(b'Z')).contains(&c)
}

/// Whether `path` is a drive's root, `X:\`.
pub fn is_drive_root(path: &[u16]) -> bool {
    path.len() == 3 && is_letter(path[0]) && path[1] == COLON && path[2] == BACKSLASH
}

/// A parsing name checked as the sheet's rules say (`refused_names`, `not_there`, `empty_name`):
/// forward slashes, the `\\?\` and `\\.\` prefixes, a stream part (a colon after the drive's),
/// wildcards and the characters no file name has are `E_INVALIDARG`; a relative name is
/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)` (relative names are not resolved against the
/// current directory). Whether the object exists is the caller's question.
pub fn parse(name: &[u16]) -> Result<Parsed, u32> {
    if name.is_empty() {
        return Ok(Parsed::Desktop);
    }
    let drive = name.len() >= 2 && is_letter(name[0]) && name[1] == COLON;
    for (i, &c) in name.iter().enumerate() {
        let bad = c == SLASH
            || c < 0x20
            || b"*?<>|\"".iter().any(|&b| c == u16::from(b))
            || (c == COLON && !(drive && i == 1));
        if bad {
            return Err(E_INVALIDARG);
        }
    }
    let unc = name.len() > 2 && name[0] == BACKSLASH && name[1] == BACKSLASH;
    let full = (drive && name.len() >= 3 && name[2] == BACKSLASH) || unc;
    if !full {
        return Err(E_FILE_NOT_FOUND);
    }
    let mut path = name.to_vec();
    while path.len() > 3 && path.last() == Some(&BACKSLASH) {
        path.pop();
    }
    Ok(Parsed::Path(path))
}

/// The last component of a full path (the drive's root is its own name, `C:\`).
pub fn file_name(path: &[u16]) -> &[u16] {
    if is_drive_root(path) {
        return path;
    }
    match path.iter().rposition(|&c| c == BACKSLASH) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// The folder a full path is in, without a trailing backslash (a drive's root keeps its
/// own); `None` for a drive's root, whose parent is no file-system folder.
pub fn parent(path: &[u16]) -> Option<Vec<u16>> {
    if is_drive_root(path) {
        return None;
    }
    let i = path.iter().rposition(|&c| c == BACKSLASH)?;
    let mut p = path[..i].to_vec();
    if p.len() == 2 && p[1] == COLON {
        p.push(BACKSLASH);
    }
    if p.is_empty() { None } else { Some(p) }
}

/// `SIGDN` values (`shobjidl_core.h`).
pub mod sigdn {
    pub const NORMALDISPLAY: u32 = 0;
    pub const PARENTRELATIVEPARSING: u32 = 0x8001_8001;
    pub const DESKTOPABSOLUTEPARSING: u32 = 0x8002_8000;
    pub const PARENTRELATIVEEDITING: u32 = 0x8003_1001;
    pub const DESKTOPABSOLUTEEDITING: u32 = 0x8004_C000;
    pub const FILESYSPATH: u32 = 0x8005_8000;
    pub const URL: u32 = 0x8006_8000;
    pub const PARENTRELATIVEFORADDRESSBAR: u32 = 0x8007_C001;
    pub const PARENTRELATIVE: u32 = 0x8008_0001;
    pub const PARENTRELATIVEFORUI: u32 = 0x8009_4001;
}

/// The name an item of `path` gives for `sigdn` (`None` = the desktop): the full path for
/// the absolute forms, the file name for the relative ones, a `file:///` url for `SIGDN_URL`.
/// The desktop has no file-system path or url (`E_INVALIDARG`); its other names are `Desktop`.
/// An unknown `sigdn` is `E_INVALIDARG`.
pub fn display_name(path: Option<&[u16]>, sigdn: u32) -> Result<Vec<u16>, u32> {
    use sigdn::*;
    let known = [
        NORMALDISPLAY,
        PARENTRELATIVEPARSING,
        DESKTOPABSOLUTEPARSING,
        PARENTRELATIVEEDITING,
        DESKTOPABSOLUTEEDITING,
        FILESYSPATH,
        URL,
        PARENTRELATIVEFORADDRESSBAR,
        PARENTRELATIVE,
        PARENTRELATIVEFORUI,
    ];
    if !known.contains(&sigdn) {
        return Err(E_INVALIDARG);
    }
    let Some(path) = path else {
        return match sigdn {
            FILESYSPATH | URL => Err(E_INVALIDARG),
            _ => Ok("Desktop".encode_utf16().collect()),
        };
    };
    Ok(match sigdn {
        DESKTOPABSOLUTEPARSING | DESKTOPABSOLUTEEDITING | FILESYSPATH => path.to_vec(),
        URL => url(path),
        _ => file_name(path).to_vec(),
    })
}

/// The `file:///` url of a full path: backslashes become slashes, and every character that is
/// not unreserved (RFC 3986) or one of `/:` is percent-encoded from its UTF-8 bytes.
pub fn url(path: &[u16]) -> Vec<u16> {
    let mut out: Vec<u16> = "file:///".encode_utf16().collect();
    let text = alloc::string::String::from_utf16_lossy(path);
    for b in text.bytes() {
        let plain = b.is_ascii_alphanumeric() || b"-._~/:".contains(&b);
        if b == b'\\' {
            out.push(SLASH);
        } else if plain {
            out.push(u16::from(b));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.extend([u16::from(b'%'), u16::from(HEX[usize::from(b >> 4)]), u16::from(HEX[usize::from(b & 15)])]);
        }
    }
    out
}

/// `SFGAO_*` (`shobjidl_core.h`).
pub mod sfgao {
    pub const CANCOPY: u32 = 0x1;
    pub const CANMOVE: u32 = 0x2;
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

/// The attributes of a file (`folder` false) or a folder: the sheet's `attributes` rule (a plain
/// file is a file-system stream that can be copied, moved, renamed and deleted; a folder is a
/// file-system folder, not a stream).
pub fn attributes(folder: bool) -> u32 {
    use sfgao::*;
    let common = CANCOPY | CANMOVE | CANRENAME | CANDELETE | HASPROPSHEET | FILESYSTEM;
    if folder { common | FOLDER | FILESYSANCESTOR | STORAGEANCESTOR | DROPTARGET } else { common | STREAM }
}

/// `IShellItem::GetAttributes`' answer: the asked bits that are set, and `S_OK` (0) when all of
/// them are, else `S_FALSE` (1).
pub fn masked_attributes(all: u32, mask: u32) -> (u32, u32) {
    let got = all & mask;
    (got, u32::from(got != mask))
}

fn lower(c: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c }
}

/// Paths compared as the file system does, without regard to case (ASCII).
pub fn same_path(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| lower(x) == lower(y))
}

/// The order of two paths without regard to case: -1, 0 or 1.
pub fn compare_paths(a: &[u16], b: &[u16]) -> i32 {
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (lower(x), lower(y));
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    a.len().cmp(&b.len()) as i32
}

/// `name` joined to `folder` with one backslash between them.
pub fn join(folder: &[u16], name: &[u16]) -> Vec<u16> {
    let mut p = folder.to_vec();
    if p.last() != Some(&BACKSLASH) {
        p.push(BACKSLASH);
    }
    p.extend_from_slice(name);
    p
}

/// The `n`th name `FOF_RENAMEONCOLLISION` tries for `name` (`n` from 1): `<stem> - Copy.<ext>`,
/// then `<stem> - Copy (2).<ext>`, `(3)`, ... (the sheet's `onto_existing`: `fc-b - Copy.bin`).
/// The extension is what follows the last dot, if that dot is not the first character.
pub fn collision_name(name: &[u16], n: u32) -> Vec<u16> {
    let split = name.iter().rposition(|&c| c == DOT).filter(|&i| i > 0).unwrap_or(name.len());
    let (stem, ext) = name.split_at(split);
    let mut out = stem.to_vec();
    out.extend(" - Copy".encode_utf16());
    if n > 1 {
        out.extend(alloc::format!(" ({n})").encode_utf16());
    }
    out.extend_from_slice(ext);
    out
}

/// Whether `RenameItem` acts on a new name: an empty name, `.`, `..` and names with a wildcard
/// leave the file as it is (the sheet's `rename_item`), as do names with a separator or a colon
/// (Windows asks the user about those; Poly treats them as aborted).
pub fn new_name_is_plain(name: &[u16]) -> bool {
    let dots = name.iter().all(|&c| c == DOT);
    !name.is_empty() && !dots && !name.iter().any(|&c| b"*?\\/:<>|\"".iter().any(|&b| c == u16::from(b)))
}

/// The extension of a file name, with its dot (`.txt`), if it has one.
pub fn extension(name: &[u16]) -> Option<&[u16]> {
    let name = file_name(name);
    let i = name.iter().rposition(|&c| c == DOT)?;
    if i + 1 == name.len() { None } else { Some(&name[i..]) }
}

/// Whether a file of this name is run as a program rather than opened by the program of its
/// type (`.exe`, `.com`).
pub fn is_program(name: &[u16]) -> bool {
    extension(name).is_some_and(|e| same_path(e, &[DOT, 0x65, 0x78, 0x65]) || same_path(e, &[DOT, 0x63, 0x6F, 0x6D]))
}

/// A `shell\<verb>\command` value with the document and the parameters in it (Learn, "File
/// Types": `%1` and `%L` are the document's full path, `%*` the parameters, `%%` a percent
/// sign; `%2`..`%9` and `%I`/`%D` here are empty). A value with no `%1`/`%L` gets the document
/// appended in quotation marks.
pub fn expand_command(template: &[u16], document: &[u16], params: &[u16]) -> Vec<u16> {
    let mut out = Vec::new();
    let mut had_document = false;
    let mut i = 0;
    while i < template.len() {
        let c = template[i];
        if c == u16::from(b'%') && i + 1 < template.len() {
            let n = template[i + 1];
            match u8::try_from(n).unwrap_or(0) {
                b'1' | b'L' | b'l' => {
                    out.extend_from_slice(document);
                    had_document = true;
                }
                b'*' => out.extend_from_slice(params),
                b'%' => out.push(c),
                b'2'..=b'9' | b'I' | b'i' | b'D' | b'd' | b'S' | b's' | b'H' | b'h' | b'V' | b'v' | b'W' | b'w' => {}
                _ => {
                    out.push(c);
                    out.push(n);
                }
            }
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    if !had_document {
        out.extend(" \"".encode_utf16());
        out.extend_from_slice(document);
        out.push(u16::from(b'"'));
    }
    out
}

/// The command line a program started by `ShellExecuteExW` gets: its path in quotation marks,
/// then a space and the parameters when there are any (the sheet's `executable`).
pub fn quote_for_command_line(program: &[u16], params: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(program.len() + params.len() + 3);
    out.push(u16::from(b'"'));
    out.extend_from_slice(program);
    out.push(u16::from(b'"'));
    if !params.is_empty() {
        out.push(u16::from(b' '));
        out.extend_from_slice(params);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn s(v: &[u16]) -> String {
        String::from_utf16(v).unwrap()
    }

    #[test]
    fn names_the_sheet_refuses() {
        for bad in [
            "C:/dir/file.bin",
            "\\\\?\\C:\\dir\\file.bin",
            "\\\\.\\C:\\dir",
            "C:\\dir\\file.bin:Zone.Identifier",
            "C:\\dir\\*.bin",
            "C:\\dir\\a?.bin",
            "C:\\dir\\a<b.bin",
            "C:\\dir\\a|b",
        ] {
            assert_eq!(parse(&w(bad)), Err(E_INVALIDARG), "{bad}");
        }
    }

    #[test]
    fn relative_names_are_not_found_and_the_empty_name_is_the_desktop() {
        assert_eq!(parse(&w("file.bin")), Err(E_FILE_NOT_FOUND));
        assert_eq!(parse(&w("dir\\file.bin")), Err(E_FILE_NOT_FOUND));
        assert_eq!(parse(&w("C:file")), Err(E_FILE_NOT_FOUND));
        assert_eq!(parse(&w("\\Windows")), Err(E_FILE_NOT_FOUND));
        assert_eq!(parse(&[]), Ok(Parsed::Desktop));
    }

    #[test]
    fn a_folder_loses_its_trailing_backslash_and_a_root_keeps_it() {
        assert_eq!(parse(&w("C:\\Users\\a\\Downloads\\")), Ok(Parsed::Path(w("C:\\Users\\a\\Downloads"))));
        assert_eq!(parse(&w("C:\\")), Ok(Parsed::Path(w("C:\\"))));
        assert_eq!(parse(&w("c:\\x\\y.bin")), Ok(Parsed::Path(w("c:\\x\\y.bin"))));
        assert_eq!(parse(&w("\\\\server\\share\\f")), Ok(Parsed::Path(w("\\\\server\\share\\f"))));
    }

    #[test]
    fn names_parents_and_urls() {
        let p = w("C:\\Users\\a\\Downloads\\poly dl.txt");
        assert_eq!(s(file_name(&p)), "poly dl.txt");
        assert_eq!(s(&parent(&p).unwrap()), "C:\\Users\\a\\Downloads");
        assert_eq!(s(&parent(&w("C:\\x")).unwrap()), "C:\\");
        assert_eq!(parent(&w("C:\\")), None);
        assert_eq!(s(file_name(&w("C:\\"))), "C:\\");
        assert_eq!(s(&url(&p)), "file:///C:/Users/a/Downloads/poly%20dl.txt");
    }

    #[test]
    fn display_names_by_sigdn() {
        let p = w("C:\\d\\item-file.polyx");
        assert_eq!(s(&display_name(Some(&p), sigdn::FILESYSPATH).unwrap()), "C:\\d\\item-file.polyx");
        assert_eq!(s(&display_name(Some(&p), sigdn::DESKTOPABSOLUTEPARSING).unwrap()), "C:\\d\\item-file.polyx");
        assert_eq!(s(&display_name(Some(&p), sigdn::NORMALDISPLAY).unwrap()), "item-file.polyx");
        assert_eq!(s(&display_name(Some(&p), sigdn::PARENTRELATIVEPARSING).unwrap()), "item-file.polyx");
        assert_eq!(s(&display_name(Some(&p), sigdn::PARENTRELATIVEEDITING).unwrap()), "item-file.polyx");
        assert_eq!(s(&display_name(Some(&p), sigdn::URL).unwrap()), "file:///C:/d/item-file.polyx");
        assert_eq!(display_name(None, sigdn::FILESYSPATH), Err(E_INVALIDARG));
        assert_eq!(display_name(Some(&p), 12345), Err(E_INVALIDARG));
    }

    #[test]
    fn attributes_of_a_file_and_a_folder_as_the_sheet_saw_them() {
        use sfgao::*;
        let file_mask = FILESYSTEM | FOLDER | STREAM | CANRENAME | CANMOVE;
        assert_eq!(masked_attributes(attributes(false), file_mask), (0x4040_0012, 1));
        let folder_mask = FILESYSTEM | FOLDER | STREAM;
        assert_eq!(masked_attributes(attributes(true), folder_mask), (0x6000_0000, 1));
        assert_eq!(masked_attributes(attributes(false), FILESYSTEM), (FILESYSTEM, 0));
        assert_eq!(masked_attributes(attributes(false), FILESYSTEM | STREAM), (0x4040_0000, 0));
    }

    #[test]
    fn comparisons_ignore_case() {
        assert!(same_path(&w("C:\\A\\b.BIN"), &w("c:\\a\\B.bin")));
        assert_eq!(compare_paths(&w("C:\\a"), &w("c:\\A")), 0);
        assert_eq!(compare_paths(&w("C:\\a"), &w("C:\\b")), -1);
        assert_eq!(compare_paths(&w("C:\\ab"), &w("C:\\a")), 1);
    }

    #[test]
    fn collision_names() {
        assert_eq!(s(&collision_name(&w("fc-b.bin"), 1)), "fc-b - Copy.bin");
        assert_eq!(s(&collision_name(&w("fc-b.bin"), 2)), "fc-b - Copy (2).bin");
        assert_eq!(s(&collision_name(&w("README"), 1)), "README - Copy");
        assert_eq!(s(&collision_name(&w(".hidden"), 1)), ".hidden - Copy");
        assert_eq!(s(&collision_name(&w("a.tar.gz"), 1)), "a.tar - Copy.gz");
    }

    #[test]
    fn rename_names() {
        assert!(new_name_is_plain(&w("fc-n.bin")));
        assert!(new_name_is_plain(&w("Unconfirmed 123456.crdownload")));
        for bad in ["", ".", "..", "a*.bin", "a?.bin", "a\\b", "a:b"] {
            assert!(!new_name_is_plain(&w(bad)), "{bad}");
        }
    }

    #[test]
    fn extensions_and_programs() {
        assert_eq!(extension(&w("C:\\d\\a.TXT")).map(s), Some(String::from(".TXT")));
        assert_eq!(extension(&w("C:\\d.x\\a")), None);
        assert!(is_program(&w("C:\\d\\Setup.EXE")));
        assert!(!is_program(&w("C:\\d\\doc.pdf")));
    }

    #[test]
    fn command_templates() {
        let doc = w("C:\\d\\a b.polyse");
        assert_eq!(
            s(&expand_command(&w("\"C:\\p.exe\" --open \"%1\""), &doc, &[])),
            "\"C:\\p.exe\" --open \"C:\\d\\a b.polyse\""
        );
        assert_eq!(s(&expand_command(&w("p.exe %L %*"), &doc, &w("x y"))), "p.exe C:\\d\\a b.polyse x y");
        assert_eq!(s(&expand_command(&w("p.exe 100%%"), &doc, &[])), "p.exe 100% \"C:\\d\\a b.polyse\"");
        assert_eq!(s(&expand_command(&w("p.exe %2 \"%1\""), &doc, &[])), "p.exe  \"C:\\d\\a b.polyse\"");
    }

    #[test]
    fn program_command_lines() {
        assert_eq!(
            s(&quote_for_command_line(&w("C:\\t\\se.exe"), &w("--child alpha beta"))),
            "\"C:\\t\\se.exe\" --child alpha beta"
        );
        assert_eq!(s(&quote_for_command_line(&w("C:\\t\\se.exe"), &[])), "\"C:\\t\\se.exe\"");
    }
}
