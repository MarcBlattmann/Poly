//! MIME types: by file name, by content, and what to show for them.
//!
//! Type names follow the freedesktop.org shared-mime-info database and the
//! IANA registry; the content checks use the magic numbers documented by
//! each format (PNG, JPEG, ELF, ZIP, …).
//!
//! ## Order ([`guess`])
//!
//! shared-mime-info weighs content matches with a priority against
//! filename globs. Poly keeps it simpler and predictable:
//!
//! 1. Folders are `inode/directory`; a broken symbolic link is
//!    `inode/symlink`; devices, FIFOs and sockets `application/octet-stream`.
//! 2. If the **name** has a known extension (or is a known file name such
//!    as `Makefile`), that type wins, so listing a folder never has to open
//!    a file. One exception, when content is at hand: a name that says
//!    *text* (`notes.txt`, `run.sh`) on bytes with a definite binary
//!    signature (an ELF program, a PNG, a ZIP, …) gets the content's type;
//!    a renamed binary does not open in a text editor.
//! 3. Otherwise the **content** decides ([`sniff`]); an empty file is
//!    `text/plain`.
//! 4. With neither, `application/octet-stream`.
//!
//! [`sniff`] only needs the first [`SNIFF_LEN`] bytes.

use crate::fs::Kind;
use crate::path::{extension, file_name};

/// How many bytes [`sniff`] looks at (enough for the ELF program headers
/// of a typical executable and for the `ustar` mark of a tar file).
pub const SNIFF_LEN: usize = 1024;

/// Folders.
pub const DIRECTORY: &str = "inode/directory";
/// Broken symbolic links.
pub const SYMLINK: &str = "inode/symlink";
/// Unknown bytes.
pub const OCTET_STREAM: &str = "application/octet-stream";
/// Plain text.
pub const TEXT: &str = "text/plain";
/// Desktop entries.
pub const DESKTOP: &str = "application/x-desktop";
/// ELF programs.
pub const EXECUTABLE: &str = "application/x-executable";
/// ELF shared libraries.
pub const SHAREDLIB: &str = "application/x-sharedlib";
/// Shell scripts (and `#!` scripts of unknown interpreters).
pub const SHELLSCRIPT: &str = "application/x-shellscript";

const OOXML_DOC: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const OOXML_XLS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const OOXML_PPT: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const ODT: &str = "application/vnd.oasis.opendocument.text";
const ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";
const ODP: &str = "application/vnd.oasis.opendocument.presentation";
const ODG: &str = "application/vnd.oasis.opendocument.graphics";
const EPUB: &str = "application/epub+zip";

/// Extensions (lowercase, without the dot) and their types.
const BY_EXT: &[(&str, &str)] = &[
    // Text and code
    ("txt", TEXT),
    ("text", TEXT),
    ("md", "text/markdown"),
    ("markdown", "text/markdown"),
    ("rst", "text/x-rst"),
    ("rs", "text/rust"),
    ("c", "text/x-csrc"),
    ("h", "text/x-chdr"),
    ("cpp", "text/x-c++src"),
    ("cc", "text/x-c++src"),
    ("cxx", "text/x-c++src"),
    ("hpp", "text/x-c++hdr"),
    ("hh", "text/x-c++hdr"),
    ("py", "text/x-python"),
    ("go", "text/x-go"),
    ("java", "text/x-java"),
    ("kt", "text/x-kotlin"),
    ("swift", "text/x-swift"),
    ("cs", "text/x-csharp"),
    ("lua", "text/x-lua"),
    ("rb", "application/x-ruby"),
    ("pl", "application/x-perl"),
    ("php", "application/x-php"),
    ("sh", SHELLSCRIPT),
    ("bash", SHELLSCRIPT),
    ("zsh", SHELLSCRIPT),
    ("js", "text/javascript"),
    ("mjs", "text/javascript"),
    ("ts", "text/x-typescript"),
    ("json", "application/json"),
    ("toml", "application/toml"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("xml", "application/xml"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("css", "text/css"),
    ("csv", "text/csv"),
    ("tsv", "text/tab-separated-values"),
    ("log", "text/x-log"),
    ("conf", TEXT),
    ("cfg", TEXT),
    ("ini", TEXT),
    ("sql", "application/sql"),
    ("diff", "text/x-patch"),
    ("patch", "text/x-patch"),
    ("tex", "text/x-tex"),
    ("desktop", DESKTOP),
    // Images
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
    ("ico", "image/vnd.microsoft.icon"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("avif", "image/avif"),
    ("heic", "image/heif"),
    // Audio
    ("mp3", "audio/mpeg"),
    ("ogg", "audio/ogg"),
    ("oga", "audio/ogg"),
    ("opus", "audio/opus"),
    ("flac", "audio/flac"),
    ("wav", "audio/wav"),
    ("m4a", "audio/mp4"),
    ("aac", "audio/aac"),
    // Video
    ("mp4", "video/mp4"),
    ("m4v", "video/mp4"),
    ("mkv", "video/x-matroska"),
    ("webm", "video/webm"),
    ("avi", "video/x-msvideo"),
    ("mov", "video/quicktime"),
    ("ogv", "video/ogg"),
    // Documents
    ("pdf", "application/pdf"),
    ("rtf", "application/rtf"),
    ("ps", "application/postscript"),
    ("epub", EPUB),
    ("doc", "application/msword"),
    ("docx", OOXML_DOC),
    ("xls", "application/vnd.ms-excel"),
    ("xlsx", OOXML_XLS),
    ("ppt", "application/vnd.ms-powerpoint"),
    ("pptx", OOXML_PPT),
    ("odt", ODT),
    ("ods", ODS),
    ("odp", ODP),
    ("odg", ODG),
    // Archives and packages
    ("zip", "application/zip"),
    ("tar", "application/x-tar"),
    ("gz", "application/gzip"),
    ("tgz", "application/x-compressed-tar"),
    ("tar.gz", "application/x-compressed-tar"),
    ("xz", "application/x-xz"),
    ("tar.xz", "application/x-xz-compressed-tar"),
    ("bz2", "application/x-bzip2"),
    ("tar.bz2", "application/x-bzip2-compressed-tar"),
    ("zst", "application/zstd"),
    ("tar.zst", "application/x-zstd-compressed-tar"),
    ("7z", "application/x-7z-compressed"),
    ("rar", "application/vnd.rar"),
    ("deb", "application/vnd.debian.binary-package"),
    ("rpm", "application/x-rpm"),
    ("iso", "application/x-cd-image"),
    ("img", "application/x-raw-disk-image"),
    ("dmg", "application/x-apple-diskimage"),
    ("appimage", "application/vnd.appimage"),
    // Programs
    ("exe", "application/x-msdownload"),
    ("dll", "application/x-msdownload"),
    ("msi", "application/x-msi"),
    ("so", SHAREDLIB),
    ("o", "application/x-object"),
    // Fonts
    ("ttf", "font/ttf"),
    ("otf", "font/otf"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
];

/// Whole file names with a type.
const BY_NAME: &[(&str, &str)] = &[
    ("Makefile", "text/x-makefile"),
    ("makefile", "text/x-makefile"),
    ("GNUmakefile", "text/x-makefile"),
    ("Dockerfile", "text/x-dockerfile"),
    ("README", TEXT),
    ("LICENSE", TEXT),
    ("COPYING", TEXT),
];

/// The type the name of `name` (a file name or path) says, if any.
pub fn by_name(name: &str) -> Option<&'static str> {
    let name = file_name(name);
    if let Some((_, m)) = BY_NAME.iter().find(|(n, _)| *n == name) {
        return Some(m);
    }
    let ext = extension(name)?;
    let find = |e: &str| BY_EXT.iter().find(|(x, _)| *x == e).map(|(_, m)| *m);
    find(&ext).or_else(|| {
        // An unknown compound (`x.tar.foo` cannot happen, but `x.tar.gz`
        // missing from the table would fall back to `gz`).
        ext.rsplit_once('.').and_then(|(_, last)| find(last))
    })
}

/// Reads a little-endian or big-endian `u16`/`u32`/`u64` at `at`.
fn uint(head: &[u8], at: usize, len: usize, big: bool) -> Option<u64> {
    let bytes = head.get(at..at.checked_add(len)?)?;
    let push = |v: u64, b: &u8| (v << 8) | u64::from(*b);
    Some(if big { bytes.iter().fold(0, push) } else { bytes.iter().rev().fold(0, push) })
}

/// An ELF file's type: programs (`ET_EXEC`, and `ET_DYN` with a program
/// interpreter, i.e. position-independent executables), shared libraries,
/// object files and core dumps.
fn elf(head: &[u8]) -> &'static str {
    let wide = head.get(4) == Some(&2);
    let big = head.get(5) == Some(&2);
    match uint(head, 16, 2, big) {
        Some(1) => "application/x-object",
        Some(2) => EXECUTABLE,
        Some(4) => "application/x-core",
        Some(3) => {
            // Look for PT_INTERP (3) among the program headers in the head.
            let (phoff, phentsize, phnum) = if wide {
                (uint(head, 32, 8, big), uint(head, 54, 2, big), uint(head, 56, 2, big))
            } else {
                (uint(head, 28, 4, big), uint(head, 42, 2, big), uint(head, 44, 2, big))
            };
            if let (Some(off), Some(size), Some(num)) = (phoff, phentsize, phnum) {
                for i in 0..num.min(64) {
                    let at = off.saturating_add(i.saturating_mul(size));
                    let Ok(at) = usize::try_from(at) else { break };
                    match uint(head, at, 4, big) {
                        Some(3) => return EXECUTABLE,
                        Some(_) => {}
                        None => break,
                    }
                }
            }
            SHAREDLIB
        }
        _ => EXECUTABLE,
    }
}

/// The type of a `#!` script by its interpreter.
fn script(head: &[u8]) -> &'static str {
    let line = head.split(|&b| b == b'\n').next().unwrap_or(&[]);
    let line = core::str::from_utf8(line).unwrap_or("");
    let mut words = line.trim_start_matches("#!").split_ascii_whitespace();
    let mut prog = words.next().map(file_name).unwrap_or("");
    if prog == "env" {
        prog = words.find(|w| !w.starts_with('-')).map(file_name).unwrap_or("");
    }
    if prog.starts_with("python") {
        "text/x-python"
    } else if prog.starts_with("perl") {
        "application/x-perl"
    } else if prog.starts_with("ruby") {
        "application/x-ruby"
    } else if prog == "node" {
        "text/javascript"
    } else {
        SHELLSCRIPT
    }
}

/// The type stored in a ZIP-based document (OpenDocument, EPUB: an
/// uncompressed first entry named `mimetype` holding the type; Office Open
/// XML: a first entry named `[Content_Types].xml`, then the main part).
fn zip(head: &[u8]) -> &'static str {
    if head.get(30..38) == Some(b"mimetype") {
        let rest = head.get(38..).unwrap_or(&[]);
        for m in [ODT, ODS, ODP, ODG, EPUB] {
            if rest.starts_with(m.as_bytes()) {
                return m;
            }
        }
    }
    let has = |needle: &[u8]| head.windows(needle.len()).any(|w| w == needle);
    if has(b"[Content_Types].xml") || has(b"_rels/.rels") {
        if has(b"word/") {
            return OOXML_DOC;
        }
        if has(b"xl/") {
            return OOXML_XLS;
        }
        if has(b"ppt/") {
            return OOXML_PPT;
        }
    }
    "application/zip"
}

/// The type the first bytes of a file show, or `None` for no bytes. Bytes
/// with no known signature are `text/plain` when they are UTF-8 without
/// NUL bytes (a character cut off at the end of the head is fine), else
/// `application/octet-stream`.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.is_empty() {
        return None;
    }
    let at = |off: usize, magic: &[u8]| head.get(off..off + magic.len()) == Some(magic);
    let m = if at(0, b"\x7fELF") {
        elf(head)
    } else if at(0, b"#!") {
        script(head)
    } else if at(0, b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if at(0, b"\xff\xd8\xff") {
        "image/jpeg"
    } else if at(0, b"GIF87a") || at(0, b"GIF89a") {
        "image/gif"
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        "image/webp"
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        "audio/wav"
    } else if at(0, b"RIFF") && at(8, b"AVI ") {
        "video/x-msvideo"
    } else if at(0, b"BM") && head.len() >= 14 && at(6, b"\0\0\0\0") {
        "image/bmp"
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        "image/tiff"
    } else if at(0, b"\0\0\x01\0") && head.len() >= 6 {
        "image/vnd.microsoft.icon"
    } else if at(0, b"%PDF-") {
        "application/pdf"
    } else if at(0, b"%!PS") {
        "application/postscript"
    } else if at(0, b"{\\rtf") {
        "application/rtf"
    } else if at(0, b"PK\x03\x04") || at(0, b"PK\x05\x06") {
        zip(head)
    } else if at(0, b"\x1f\x8b") {
        "application/gzip"
    } else if at(0, b"\xfd7zXZ\0") {
        "application/x-xz"
    } else if at(0, b"BZh") {
        "application/x-bzip2"
    } else if at(0, b"\x28\xb5\x2f\xfd") {
        "application/zstd"
    } else if at(0, b"7z\xbc\xaf\x27\x1c") {
        "application/x-7z-compressed"
    } else if at(0, b"Rar!\x1a\x07") {
        "application/vnd.rar"
    } else if at(0, b"!<arch>\ndebian-binary") {
        "application/vnd.debian.binary-package"
    } else if at(0, b"!<arch>\n") {
        "application/x-archive"
    } else if at(0, b"\xed\xab\xee\xdb") {
        "application/x-rpm"
    } else if at(257, b"ustar") {
        "application/x-tar"
    } else if at(0, b"OggS") {
        if at(28, b"\x80theora") {
            "video/ogg"
        } else if at(28, b"OpusHead") {
            "audio/opus"
        } else {
            "audio/ogg"
        }
    } else if at(0, b"fLaC") {
        "audio/flac"
    } else if at(0, b"ID3") || head.len() >= 2 && head[0] == 0xff && matches!(head[1], 0xfb | 0xf3 | 0xf2) {
        "audio/mpeg"
    } else if at(0, b"\x1a\x45\xdf\xa3") {
        if head.windows(4).any(|w| w == b"webm") { "video/webm" } else { "video/x-matroska" }
    } else if at(4, b"ftyp") {
        match head.get(8..12) {
            Some(b"qt  ") => "video/quicktime",
            Some(b"M4A ") => "audio/mp4",
            Some(b"avif") => "image/avif",
            Some(b"heic" | b"heix" | b"mif1") => "image/heif",
            _ => "video/mp4",
        }
    } else if at(0, b"MZ") {
        "application/x-msdownload"
    } else if at(0, b"\0\x01\0\0") || at(0, b"true") {
        "font/ttf"
    } else if at(0, b"OTTO") {
        "font/otf"
    } else if at(0, b"wOFF") {
        "font/woff"
    } else if at(0, b"wOF2") {
        "font/woff2"
    } else {
        return Some(text_like(head));
    };
    Some(m)
}

/// Markup recognised by its first tag, else plain text or bytes.
fn text_like(head: &[u8]) -> &'static str {
    let ok = match core::str::from_utf8(head) {
        Ok(_) => true,
        // Only a character cut off by the end of the head.
        Err(e) => e.error_len().is_none(),
    };
    if !ok || head.contains(&0) {
        return OCTET_STREAM;
    }
    let body = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    let start = body.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(body.len());
    let body = &body[start..];
    let starts_ci = |p: &[u8]| body.get(..p.len()).is_some_and(|b| b.eq_ignore_ascii_case(p));
    let contains = |p: &[u8]| body.windows(p.len()).any(|w| w == p);
    if starts_ci(b"<svg") || starts_ci(b"<?xml") && contains(b"<svg") {
        "image/svg+xml"
    } else if starts_ci(b"<!doctype html") || starts_ci(b"<html") {
        "text/html"
    } else if starts_ci(b"<?xml") {
        "application/xml"
    } else {
        TEXT
    }
}

/// Whether `mime` is a text type (`text/*` or a script).
pub fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            SHELLSCRIPT
                | DESKTOP
                | "application/json"
                | "application/toml"
                | "application/yaml"
                | "application/xml"
                | "application/x-perl"
                | "application/x-ruby"
                | "application/x-php"
                | "application/sql"
        )
}

/// The type of an entry named `name` of kind `kind` (links already
/// followed, so [`Kind::Symlink`] means a broken link), with the file's
/// first bytes if they were read. See the module docs for the order.
pub fn guess(name: &str, kind: Kind, head: Option<&[u8]>) -> &'static str {
    match kind {
        Kind::Dir => return DIRECTORY,
        Kind::Symlink => return SYMLINK,
        Kind::Other => return OCTET_STREAM,
        Kind::File => {}
    }
    let sniffed = head.map(|h| sniff(h).unwrap_or(TEXT));
    match (by_name(name), sniffed) {
        (Some(named), Some(content)) if is_text(named) && !is_text(content) && content != OCTET_STREAM => content,
        (Some(named), _) => named,
        (None, Some(content)) => content,
        (None, None) => OCTET_STREAM,
    }
}

/// Words for `mime` in the UI's "Type" column.
pub fn description(mime: &str) -> &'static str {
    let exact = match mime {
        DIRECTORY => "Folder",
        SYMLINK => "Broken link",
        TEXT => "Text",
        "text/markdown" => "Markdown document",
        "text/html" => "Web page",
        "text/css" => "Style sheet",
        "text/csv" | "text/tab-separated-values" => "Table",
        "text/x-log" => "Log",
        "text/x-patch" => "Patch",
        DESKTOP => "App shortcut",
        EXECUTABLE => "Program",
        SHAREDLIB => "Library",
        "application/x-object" => "Object file",
        "application/x-core" => "Core dump",
        SHELLSCRIPT => "Shell script",
        "application/x-msdownload" => "Windows program",
        "application/x-msi" => "Windows installer",
        "application/vnd.appimage" => "AppImage program",
        "application/json" => "JSON document",
        "application/toml" => "TOML document",
        "application/yaml" => "YAML document",
        "application/xml" => "XML document",
        "application/pdf" => "PDF document",
        "application/postscript" => "PostScript document",
        "application/rtf" => "Rich text document",
        EPUB => "E-book",
        "application/msword" | OOXML_DOC | ODT => "Document",
        "application/vnd.ms-excel" | OOXML_XLS | ODS => "Spreadsheet",
        "application/vnd.ms-powerpoint" | OOXML_PPT | ODP => "Presentation",
        ODG => "Drawing",
        "application/vnd.debian.binary-package" | "application/x-rpm" => "Package",
        "application/x-cd-image" | "application/x-raw-disk-image" | "application/x-apple-diskimage" => "Disk image",
        "image/png" => "PNG image",
        "image/jpeg" => "JPEG image",
        "image/gif" => "GIF image",
        "image/webp" => "WebP image",
        "image/bmp" => "BMP image",
        "image/svg+xml" => "SVG image",
        "image/tiff" => "TIFF image",
        "image/vnd.microsoft.icon" => "Icon",
        "audio/mpeg" => "MP3 audio",
        "audio/flac" => "FLAC audio",
        "audio/ogg" | "audio/opus" => "Ogg audio",
        "audio/wav" => "WAV audio",
        "video/mp4" => "MP4 video",
        "video/x-matroska" => "Matroska video",
        "video/webm" => "WebM video",
        OCTET_STREAM => "Binary file",
        _ => "",
    };
    if !exact.is_empty() {
        return exact;
    }
    match icon(mime) {
        FileIcon::Archive => "Archive",
        FileIcon::Code => "Source code",
        FileIcon::Script => "Script",
        FileIcon::Text => "Text",
        FileIcon::Image => "Image",
        FileIcon::Audio => "Audio",
        FileIcon::Video => "Video",
        FileIcon::Font => "Font",
        _ => "File",
    }
}

/// What a file looks like in the list; the UI maps this to a Lucide glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileIcon {
    Folder,
    File,
    Text,
    Code,
    Image,
    Audio,
    Video,
    Archive,
    Pdf,
    Document,
    Spreadsheet,
    Presentation,
    Executable,
    Script,
    DesktopEntry,
    Font,
    Disk,
    /// A broken symbolic link.
    Symlink,
}

/// The icon for `mime`.
pub fn icon(mime: &str) -> FileIcon {
    match mime {
        DIRECTORY => return FileIcon::Folder,
        SYMLINK => return FileIcon::Symlink,
        DESKTOP => return FileIcon::DesktopEntry,
        "application/pdf" | "application/postscript" => return FileIcon::Pdf,
        EXECUTABLE | SHAREDLIB | "application/x-msdownload" | "application/x-msi" | "application/vnd.appimage" => {
            return FileIcon::Executable;
        }
        SHELLSCRIPT | "application/x-perl" | "application/x-ruby" | "application/x-php" => return FileIcon::Script,
        "application/vnd.ms-excel" | OOXML_XLS | ODS | "text/csv" | "text/tab-separated-values" => {
            return FileIcon::Spreadsheet;
        }
        "application/vnd.ms-powerpoint" | OOXML_PPT | ODP => return FileIcon::Presentation,
        "application/msword" | OOXML_DOC | ODT | "application/rtf" | EPUB | ODG => return FileIcon::Document,
        "application/x-cd-image" | "application/x-raw-disk-image" | "application/x-apple-diskimage" => {
            return FileIcon::Disk;
        }
        "application/json" | "application/toml" | "application/yaml" | "application/xml" | "application/sql"
        | "text/html" | "text/css" | "text/javascript" => return FileIcon::Code,
        "text/x-python" => return FileIcon::Script,
        TEXT | "text/markdown" | "text/x-log" | "text/x-rst" => return FileIcon::Text,
        _ => {}
    }
    let (top, sub) = mime.split_once('/').unwrap_or((mime, ""));
    match top {
        "image" => FileIcon::Image,
        "audio" => FileIcon::Audio,
        "video" => FileIcon::Video,
        "font" => FileIcon::Font,
        "text" if sub.starts_with("x-") || sub == "rust" => FileIcon::Code,
        "text" => FileIcon::Text,
        "application"
            if sub.contains("zip")
                || sub.contains("tar")
                || sub.contains("compressed")
                || matches!(
                    sub,
                    "gzip"
                        | "x-xz"
                        | "x-bzip2"
                        | "zstd"
                        | "vnd.rar"
                        | "x-archive"
                        | "vnd.debian.binary-package"
                        | "x-rpm"
                ) =>
        {
            FileIcon::Archive
        }
        _ => FileIcon::File,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn by_names() {
        assert_eq!(by_name("a.TXT"), Some("text/plain"));
        assert_eq!(by_name("/x/lib.rs"), Some("text/rust"));
        assert_eq!(by_name("backup.tar.gz"), Some("application/x-compressed-tar"));
        assert_eq!(by_name("x.gz"), Some("application/gzip"));
        assert_eq!(by_name("Makefile"), Some("text/x-makefile"));
        assert_eq!(by_name("app.desktop"), Some(DESKTOP));
        assert_eq!(by_name("photo.JPEG"), Some("image/jpeg"));
        assert_eq!(by_name("noext"), None);
        assert_eq!(by_name(".bashrc"), None);
        assert_eq!(by_name("x.unknownext"), None);
    }

    fn elf64(e_type: u16, interp: bool) -> Vec<u8> {
        let mut h = alloc::vec![0u8; 256];
        h[..4].copy_from_slice(b"\x7fELF");
        h[4] = 2; // 64-bit
        h[5] = 1; // little-endian
        h[16..18].copy_from_slice(&e_type.to_le_bytes());
        h[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        h[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        h[56..58].copy_from_slice(&2u16.to_le_bytes()); // e_phnum
        h[64..68].copy_from_slice(&6u32.to_le_bytes()); // PT_PHDR
        h[120..124].copy_from_slice(&(if interp { 3u32 } else { 1 }).to_le_bytes());
        h
    }

    #[test]
    fn sniffing() {
        assert_eq!(sniff(&elf64(2, false)), Some(EXECUTABLE));
        assert_eq!(sniff(&elf64(3, true)), Some(EXECUTABLE), "a PIE");
        assert_eq!(sniff(&elf64(3, false)), Some(SHAREDLIB));
        assert_eq!(sniff(&elf64(1, false)), Some("application/x-object"));
        assert_eq!(sniff(b"\x7fELF"), Some(EXECUTABLE), "truncated");
        assert_eq!(sniff(b"#!/bin/sh\necho"), Some(SHELLSCRIPT));
        assert_eq!(sniff(b"#!/usr/bin/env -S python3 -u\n"), Some("text/x-python"));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a"), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVEfmt "), Some("audio/wav"));
        assert_eq!(sniff(b"BM\x36\0\0\0\0\0\0\0\x36\0\0\0"), Some("image/bmp"));
        assert_eq!(sniff(b"BMW is a car"), Some(TEXT));
        assert_eq!(sniff(b"%PDF-1.7"), Some("application/pdf"));
        assert_eq!(sniff(b"PK\x03\x04rest"), Some("application/zip"));
        let mut odt = alloc::vec![0u8; 30];
        odt[..4].copy_from_slice(b"PK\x03\x04");
        odt.extend_from_slice(b"mimetypeapplication/vnd.oasis.opendocument.text");
        assert_eq!(sniff(&odt), Some(ODT));
        let mut docx = b"PK\x03\x04".to_vec();
        docx.extend_from_slice(b"..[Content_Types].xml...word/document.xml");
        assert_eq!(sniff(&docx), Some(OOXML_DOC));
        assert_eq!(sniff(b"\x1f\x8b\x08"), Some("application/gzip"));
        assert_eq!(sniff(b"\xfd7zXZ\0"), Some("application/x-xz"));
        assert_eq!(sniff(b"BZh91AY"), Some("application/x-bzip2"));
        assert_eq!(sniff(b"\x28\xb5\x2f\xfd"), Some("application/zstd"));
        assert_eq!(sniff(b"7z\xbc\xaf\x27\x1c"), Some("application/x-7z-compressed"));
        assert_eq!(sniff(b"OggS"), Some("audio/ogg"));
        assert_eq!(sniff(b"fLaC"), Some("audio/flac"));
        assert_eq!(sniff(b"ID3\x04"), Some("audio/mpeg"));
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3....webm"), Some("video/webm"));
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3....matroska"), Some("video/x-matroska"));
        assert_eq!(sniff(b"\0\0\0\x20ftypisom"), Some("video/mp4"));
        assert_eq!(sniff(b"\0\0\0\x20ftypqt  "), Some("video/quicktime"));
        assert_eq!(sniff(b"MZ\x90\0"), Some("application/x-msdownload"));
        assert_eq!(sniff(b"\0\x01\0\0\0\x10"), Some("font/ttf"));
        assert_eq!(sniff(b"OTTO"), Some("font/otf"));
        let mut tar = alloc::vec![b'a'; 300];
        tar[100] = 0;
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(sniff(&tar), Some("application/x-tar"));
        assert_eq!(sniff(b"<?xml version=\"1.0\"?>\n<svg xmlns="), Some("image/svg+xml"));
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><a/>"), Some("application/xml"));
        assert_eq!(sniff(b"\xef\xbb\xbf  <!DOCTYPE HTML><html>"), Some("text/html"));
        assert_eq!(sniff("hello wörld".as_bytes()), Some(TEXT));
        assert_eq!(sniff(&"ü".as_bytes()[..1]), Some(TEXT), "a character cut off by the head");
        assert_eq!(sniff(b"a\0b"), Some(OCTET_STREAM));
        assert_eq!(sniff(b"\xc3\x28"), Some(OCTET_STREAM));
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn guessing() {
        assert_eq!(guess("x", Kind::Dir, None), DIRECTORY);
        assert_eq!(guess("x", Kind::Symlink, None), SYMLINK);
        assert_eq!(guess("a.png", Kind::File, None), "image/png");
        // The name wins over plain-text content…
        assert_eq!(guess("a.rs", Kind::File, Some(b"fn main() {}")), "text/rust");
        // …but not when a text name hides a program or an image.
        assert_eq!(guess("notes.txt", Kind::File, Some(&elf64(2, false))), EXECUTABLE);
        assert_eq!(guess("run.sh", Kind::File, Some(b"\x89PNG\r\n\x1a\n")), "image/png");
        assert_eq!(guess("a.txt", Kind::File, Some(b"a\0b")), TEXT);
        // A binary name stays what it says.
        assert_eq!(guess("a.zip", Kind::File, Some(b"hello")), "application/zip");
        assert_eq!(guess("prog", Kind::File, Some(&elf64(3, true))), EXECUTABLE);
        assert_eq!(guess("empty", Kind::File, Some(b"")), TEXT);
        assert_eq!(guess("blob", Kind::File, None), OCTET_STREAM);
        assert_eq!(guess("x.desktop", Kind::File, Some(b"[Desktop Entry]")), DESKTOP);
    }

    #[test]
    fn descriptions_and_icons() {
        assert_eq!(description(DIRECTORY), "Folder");
        assert_eq!(description("image/png"), "PNG image");
        assert_eq!(description(EXECUTABLE), "Program");
        assert_eq!(description("application/x-xz-compressed-tar"), "Archive");
        assert_eq!(description("text/x-go"), "Source code");
        assert_eq!(description("image/x-weird"), "Image");
        assert_eq!(description("application/x-whatever"), "File");
        assert_eq!(icon(DIRECTORY), FileIcon::Folder);
        assert_eq!(icon("text/rust"), FileIcon::Code);
        assert_eq!(icon(TEXT), FileIcon::Text);
        assert_eq!(icon("image/webp"), FileIcon::Image);
        assert_eq!(icon("audio/flac"), FileIcon::Audio);
        assert_eq!(icon("video/webm"), FileIcon::Video);
        assert_eq!(icon("application/x-compressed-tar"), FileIcon::Archive);
        assert_eq!(icon("application/zip"), FileIcon::Archive);
        assert_eq!(icon("application/pdf"), FileIcon::Pdf);
        assert_eq!(icon(OOXML_XLS), FileIcon::Spreadsheet);
        assert_eq!(icon(ODP), FileIcon::Presentation);
        assert_eq!(icon(EXECUTABLE), FileIcon::Executable);
        assert_eq!(icon(SHELLSCRIPT), FileIcon::Script);
        assert_eq!(icon(DESKTOP), FileIcon::DesktopEntry);
        assert_eq!(icon("font/woff2"), FileIcon::Font);
        assert_eq!(icon("application/x-cd-image"), FileIcon::Disk);
        assert_eq!(icon(OCTET_STREAM), FileIcon::File);
        // Every table type has words.
        for (_, m) in BY_EXT.iter().chain(BY_NAME) {
            assert!(!description(m).is_empty(), "{m}");
        }
    }
}
