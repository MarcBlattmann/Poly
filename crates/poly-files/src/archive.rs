//! Unified archive extraction over [`Fs`] for `.zip`, `.tar`, `.tar.gz`/`.tgz`,
//! and `.tar.xz`/`.txz`.
//!
//! Format detection uses the archive's extension when present and falls back to
//! sniffing magic bytes for extensionless files.

use alloc::string::String;
use core::fmt;

use crate::fs::{Fs, FsError, Kind};
use crate::path::{extension, normalize};
use crate::tar::{self, TarError};
use crate::zip::{self, ZipError};

pub use crate::zip::{ExtractReport, Limits};

/// MIME type for `.zip`.
pub const MIME_ZIP: &str = zip::MIME;
/// MIME type for `.tar`.
pub const MIME_TAR: &str = "application/x-tar";
/// MIME type for `.tar.gz` and `.tgz`.
pub const MIME_TAR_GZ: &str = "application/x-compressed-tar";
/// MIME type for `.tar.xz` and `.txz`.
pub const MIME_TAR_XZ: &str = "application/x-xz-compressed-tar";

/// All MIME types supported by built-in extraction.
pub const SUPPORTED_MIMES: [&str; 4] = [MIME_ZIP, MIME_TAR, MIME_TAR_GZ, MIME_TAR_XZ];

/// Supported archive formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarXz,
}

/// Why extracting an archive failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveError {
    Fs(FsError),
    Zip(ZipError),
    Tar(TarError),
    UnsupportedFormat,
}

impl ArchiveError {
    /// A short human-readable message for the UI status line.
    pub fn message(&self) -> String {
        match self {
            ArchiveError::Fs(e) => e.message().into(),
            ArchiveError::Zip(e) => e.message(),
            ArchiveError::Tar(e) => e.message(),
            ArchiveError::UnsupportedFormat => "Unsupported archive format".into(),
        }
    }
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl From<FsError> for ArchiveError {
    fn from(e: FsError) -> Self {
        ArchiveError::Fs(e)
    }
}

impl From<ZipError> for ArchiveError {
    fn from(e: ZipError) -> Self {
        ArchiveError::Zip(e)
    }
}

impl From<TarError> for ArchiveError {
    fn from(e: TarError) -> Self {
        ArchiveError::Tar(e)
    }
}

/// Whether `mime` is an archive format supported by [`extract`].
pub fn is_supported_mime(mime: &str) -> bool {
    format_for_mime(mime).is_some()
}

/// The archive [`Format`] corresponding to `mime`, if supported.
pub fn format_for_mime(mime: &str) -> Option<Format> {
    match mime {
        MIME_ZIP => Some(Format::Zip),
        MIME_TAR => Some(Format::Tar),
        MIME_TAR_GZ => Some(Format::TarGz),
        MIME_TAR_XZ => Some(Format::TarXz),
        _ => None,
    }
}

/// The archive [`Format`] implied by `path`'s extension, if supported.
pub fn format_for_path(path: &str) -> Option<Format> {
    match extension(path)?.as_str() {
        "zip" => Some(Format::Zip),
        "tar" => Some(Format::Tar),
        "tar.gz" | "tgz" => Some(Format::TarGz),
        "tar.xz" | "txz" => Some(Format::TarXz),
        _ => None,
    }
}

/// Guess the archive [`Format`] from leading magic bytes.
pub fn sniff_format(head: &[u8]) -> Option<Format> {
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        Some(Format::Zip)
    } else if head.starts_with(&[0x1f, 0x8b]) {
        Some(Format::TarGz)
    } else if head.starts_with(b"\xfd7zXZ\0") {
        Some(Format::TarXz)
    } else if head.get(257..262) == Some(b"ustar") {
        Some(Format::Tar)
    } else {
        None
    }
}

/// Extract the archive at `archive_path` (`.zip`, `.tar`, `.tar.gz`/`.tgz`,
/// or `.tar.xz`/`.txz`) into a new folder inside `dest_parent` with
/// [`Limits::default`].
pub fn extract(fs: &dyn Fs, archive_path: &str, dest_parent: &str) -> Result<ExtractReport, ArchiveError> {
    extract_with_limits(fs, archive_path, dest_parent, &Limits::default())
}

/// Extract the archive at `archive_path` into a new folder inside `dest_parent`,
/// enforcing `limits`.
pub fn extract_with_limits(
    fs: &dyn Fs,
    archive_path: &str,
    dest_parent: &str,
    limits: &Limits,
) -> Result<ExtractReport, ArchiveError> {
    let archive_path = normalize(archive_path);
    let dest_parent = normalize(dest_parent);
    let format = match format_for_path(&archive_path) {
        Some(f) => f,
        None => {
            let meta = fs.stat(&archive_path)?;
            if meta.kind == Kind::Dir {
                return Err(ArchiveError::Fs(FsError::IsADirectory));
            }
            let head = fs.read_head(&archive_path, 512)?;
            sniff_format(&head).ok_or(ArchiveError::UnsupportedFormat)?
        }
    };
    match format {
        Format::Zip => Ok(zip::extract_with_limits(fs, &archive_path, &dest_parent, limits)?),
        Format::Tar => Ok(tar::extract_tar_with_limits(fs, &archive_path, &dest_parent, limits)?),
        Format::TarGz => Ok(tar::extract_tar_gz_with_limits(fs, &archive_path, &dest_parent, limits)?),
        Format::TarXz => Ok(tar::extract_tar_xz_with_limits(fs, &archive_path, &dest_parent, limits)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tar::test_helpers::{build_gzip, build_tar, build_xz};
    use crate::testfs::MemFs;

    #[test]
    fn recognizes_supported_mimes_and_extensions() {
        for m in SUPPORTED_MIMES {
            assert!(is_supported_mime(m), "mime={m}");
        }
        assert!(!is_supported_mime("application/gzip"));
        assert!(!is_supported_mime("application/x-xz"));
        assert!(!is_supported_mime("text/plain"));

        assert_eq!(format_for_path("/a/foo.ZIP"), Some(Format::Zip));
        assert_eq!(format_for_path("/a/foo.tar"), Some(Format::Tar));
        assert_eq!(format_for_path("/a/foo.tar.gz"), Some(Format::TarGz));
        assert_eq!(format_for_path("/a/foo.tgz"), Some(Format::TarGz));
        assert_eq!(format_for_path("/a/foo.tar.xz"), Some(Format::TarXz));
        assert_eq!(format_for_path("/a/foo.txz"), Some(Format::TarXz));
        assert_eq!(format_for_path("/a/foo.gz"), None);
        assert_eq!(format_for_path("/a/foo.xz"), None);
    }

    #[test]
    fn unified_extract_handles_all_formats_and_sniffed_extensionless_tar() {
        let tar = build_tar(&[("hello.txt", b'0', 0o644, b"world")]);
        let gz = build_gzip(&tar);
        let xz = build_xz(&tar, 0x04);

        let fs = MemFs::new();
        fs.create_dir("/a").unwrap();
        fs.write("/a/one.tar", &tar).unwrap();
        fs.write("/a/two.tar.gz", &gz).unwrap();
        fs.write("/a/three.tgz", &gz).unwrap();
        fs.write("/a/four.tar.xz", &xz).unwrap();
        fs.write("/a/five.txz", &xz).unwrap();
        fs.write("/a/notar_ext", &tar).unwrap();
        fs.write("/a/plain.txt", b"just text").unwrap();

        for (file, folder) in [
            ("/a/one.tar", "one"),
            ("/a/two.tar.gz", "two"),
            ("/a/three.tgz", "three"),
            ("/a/four.tar.xz", "four"),
            ("/a/five.txz", "five"),
            // File `/a/notar_ext` already exists in `/a`, so collision avoidance picks `notar_ext 2`.
            ("/a/notar_ext", "notar_ext 2"),
        ] {
            let r = extract(&fs, file, "/a").unwrap();
            assert_eq!(r.name, folder);
            assert_eq!(fs.read_to_string(&alloc::format!("/a/{folder}/hello.txt")).unwrap(), "world");
        }

        assert_eq!(extract(&fs, "/a/plain.txt", "/a"), Err(ArchiveError::UnsupportedFormat));
    }
}
