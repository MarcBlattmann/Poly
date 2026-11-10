//! Reading a `.deb` (deb(5), format 2.0): an ar(5) archive of
//! `debian-binary` (the format's version), `control.tar` (the package's
//! control data: `control`, the maintainer scripts, `md5sums`, `conffiles`,
//! `triggers`, ...) and `data.tar` (its files), the tars compressed with
//! gzip, xz or zstd or not at all. The installer only reads the control
//! member, which is near the start of the file: a package of hundreds of
//! megabytes is described from its first kilobytes.

use crate::control::{self, Paragraph};
use crate::{Error, ar, tar};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The most a control member may be, compressed and decompressed: far more
/// than any package's (Chromium's is under 100 KiB compressed).
pub const CONTROL_LIMIT: usize = 64 << 20;

/// How many bytes from the start of a `.deb` reach the control member's
/// header in every package: the archive's magic, the `debian-binary`
/// member and one more header.
pub const HEAD: usize = 512;

/// Where the control member is, from the start of the file (`head`: at
/// least [`HEAD`] bytes, or the whole file if it is shorter).
pub fn control_member(head: &[u8]) -> Result<ar::Member, Error> {
    let members = ar::members(head)?;
    if members.first().is_none_or(|m| m.name != "debian-binary") {
        return Err(Error::NotThisFormat);
    }
    let m = members.into_iter().find(|m| m.name.starts_with("control.tar")).ok_or(Error::NoControl)?;
    if m.size > CONTROL_LIMIT as u64 {
        return Err(Error::TooLarge);
    }
    Ok(m)
}

/// Decompress a member by its name's suffix (deb(5): `.gz`, `.xz`, `.zst`
/// or none).
pub fn decompress(name: &str, data: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let from = |e: poly_compress::Error| match e {
        poly_compress::Error::TooLarge => Error::TooLarge,
        poly_compress::Error::Unsupported => Error::Compression,
        poly_compress::Error::Corrupt => Error::Corrupt,
    };
    match name.rsplit_once('.').map(|(_, ext)| ext) {
        Some("tar") => Ok(data.to_vec()),
        Some("xz") => poly_compress::xz::decompress_stream(data, limit).map_err(from),
        Some("zst") => poly_compress::zstd::decompress(data, limit).map_err(from),
        Some("gz") => gunzip(data, limit),
        _ => Err(Error::Compression),
    }
}

/// A gzip file (RFC 1952): a header with optional fields, a deflate
/// stream, a CRC and the length.
fn gunzip(data: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    if data.len() < 18 {
        return Err(Error::Truncated);
    }
    if data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return Err(Error::Corrupt);
    }
    let flags = data[3];
    let mut at = 10usize;
    if flags & 4 != 0 {
        // FEXTRA: a two-byte length, then that many bytes.
        let len = data.get(at..at + 2).ok_or(Error::Truncated)?;
        at += 2 + usize::from(u16::from_le_bytes([len[0], len[1]]));
    }
    for bit in [8, 16] {
        // FNAME, FCOMMENT: zero-terminated strings.
        if flags & bit != 0 {
            let rest = data.get(at..).ok_or(Error::Truncated)?;
            at += rest.iter().position(|&b| b == 0).ok_or(Error::Truncated)? + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2; // FHCRC
    }
    let stream = data.get(at..).ok_or(Error::Truncated)?;
    match poly_png::inflate::inflate(stream, limit) {
        Ok((out, _)) => Ok(out),
        Err(poly_png::InflateError::TooLarge) => Err(Error::TooLarge),
        Err(_) => Err(Error::Corrupt),
    }
}

/// The files of a control member (`name`: the member's name, for its
/// compression): `control`, `postinst`, `md5sums`, ...
pub fn control_files(name: &str, data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let archive = decompress(name, data, CONTROL_LIMIT)?;
    Ok(tar::files(&archive)?.into_iter().map(|(n, d)| (n, d.to_vec())).collect())
}

/// The control files of the whole package file `deb`.
pub fn read_control_files(deb: &[u8]) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let m = control_member(deb)?;
    control_files(&m.name, ar::data(deb, &m)?)
}

/// What a package says about itself (its `control` file, deb-control(5)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// `Package`: the package's name.
    pub name: String,
    pub version: String,
    /// `Architecture`: `amd64`, `all`, ...
    pub architecture: String,
    /// The first line of `Description`.
    pub summary: String,
    /// The rest of `Description`, paragraphs separated by empty lines.
    pub description: String,
    pub maintainer: String,
    pub homepage: String,
    pub section: String,
    /// `Installed-Size`: what it takes on disk, in KiB.
    pub installed_kib: Option<u64>,
    /// The packages it needs (`Pre-Depends` and `Depends`: the first
    /// alternative of each).
    pub depends: Vec<String>,
}

impl Info {
    /// Read a `control` file.
    pub fn parse(control: &str) -> Result<Info, Error> {
        let p = control::first(control).ok_or(Error::NoControl)?;
        Info::from_paragraph(&p)
    }

    /// From a control paragraph (a `control` file's, or one of dpkg's
    /// status database).
    pub fn from_paragraph(p: &Paragraph) -> Result<Info, Error> {
        let get = |k: &str| p.get(k).unwrap_or("").to_string();
        let name = get("Package");
        if name.is_empty() {
            return Err(Error::NoControl);
        }
        let (summary, description) = control::description(p.get("Description").unwrap_or(""));
        let mut depends = Vec::new();
        for field in ["Pre-Depends", "Depends"] {
            for d in control::relation_names(&p.folded(field).unwrap_or_default()) {
                if !depends.contains(&d) {
                    depends.push(d);
                }
            }
        }
        Ok(Info {
            name,
            version: get("Version"),
            architecture: get("Architecture"),
            summary,
            description,
            maintainer: get("Maintainer"),
            homepage: get("Homepage"),
            section: get("Section"),
            installed_kib: p.get("Installed-Size").and_then(|s| s.trim().parse().ok()),
            depends,
        })
    }

    /// The maintainer without the e-mail address (`Name <address>`).
    pub fn publisher(&self) -> &str {
        self.maintainer.split('<').next().unwrap_or("").trim()
    }
}

/// Read what the whole package file `deb` says about itself.
pub fn read_info(deb: &[u8]) -> Result<Info, Error> {
    let files = read_control_files(deb)?;
    let (_, control) = files.iter().find(|(n, _)| n == "control").ok_or(Error::NoControl)?;
    Info::parse(core::str::from_utf8(control).map_err(|_| Error::Corrupt)?)
}

/// A size in KiB as people read it: `812 KB`, `1.3 MB`, `2.1 GB`.
pub fn human_kib(kib: u64) -> String {
    if kib < 1024 {
        return alloc::format!("{kib} KB");
    }
    let (value, unit) = if kib < 1024 * 1024 { (kib * 10 / 1024, "MB") } else { (kib * 10 / (1024 * 1024), "GB") };
    alloc::format!("{}.{} {unit}", value / 10, value % 10)
}

/// A minimal `.deb` with `control` and other control files, uncompressed
/// (deb(5) allows it): for tests and fixtures.
pub fn build(control_files: &[(&str, &[u8])], data_files: &[(&str, &[u8])]) -> Vec<u8> {
    let control = tar::build(control_files);
    let data = tar::build(data_files);
    ar::build(&[("debian-binary", b"2.0\n"), ("control.tar", &control), ("data.tar", &data)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROL: &[u8] = b"Package: hello-gui\nVersion: 1.2-3\nArchitecture: amd64\n\
Maintainer: Jane Doe <jane@example.org>\nInstalled-Size: 2048\n\
Pre-Depends: libc6 (>= 2.34)\nDepends: libgtk-3-0t64 | libgtk-3-0, libc6, hello-data (= 1.2-3)\n\
Section: utils\nHomepage: https://example.org/hello\n\
Description: says hello in a window\n A small program.\n";

    #[test]
    fn a_package_describes_itself() {
        let deb = build(&[("control", CONTROL), ("postinst", b"#!/bin/sh\n")], &[("usr/bin/hello", b"\x7fELF")]);
        let info = read_info(&deb).unwrap();
        assert_eq!(info.name, "hello-gui");
        assert_eq!(info.version, "1.2-3");
        assert_eq!(info.architecture, "amd64");
        assert_eq!(info.summary, "says hello in a window");
        assert_eq!(info.description, "A small program.");
        assert_eq!(info.publisher(), "Jane Doe");
        assert_eq!(info.installed_kib, Some(2048));
        assert_eq!(info.depends, ["libc6", "libgtk-3-0t64", "hello-data"]);
        let files = read_control_files(&deb).unwrap();
        assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["control", "postinst"]);
    }

    #[test]
    fn the_control_member_is_found_from_the_files_start() {
        let deb = build(&[("control", CONTROL)], &[("usr/share/big", &[0u8; 100_000])]);
        let m = control_member(&deb[..HEAD]).unwrap();
        assert_eq!((m.name.as_str(), m.offset), ("control.tar", 132));
        let files = control_files(&m.name, &deb[m.offset as usize..(m.offset + m.size) as usize]).unwrap();
        assert_eq!(Info::parse(core::str::from_utf8(&files[0].1).unwrap()).unwrap().name, "hello-gui");
    }

    #[test]
    fn what_is_wrong_with_a_file_is_said() {
        assert_eq!(read_info(b"not a package at all"), Err(Error::NotThisFormat));
        assert_eq!(read_info(&ar::build(&[("lib.o", b"12")])), Err(Error::NotThisFormat));
        assert_eq!(read_info(&ar::build(&[("debian-binary", b"2.0\n")])), Err(Error::NoControl));
        let deb = build(&[("control", CONTROL)], &[]);
        assert_eq!(read_info(&deb[..400]), Err(Error::Truncated));
        let no_name = build(&[("control", b"Version: 1\n")], &[]);
        assert_eq!(read_info(&no_name), Err(Error::NoControl));
        let unknown = ar::build(&[("debian-binary", b"2.0\n"), ("control.tar.bz2", b"BZh9")]);
        assert_eq!(read_info(&unknown), Err(Error::Compression));
        let bad = ar::build(&[("debian-binary", b"2.0\n"), ("control.tar.xz", &[0u8; 64])]);
        assert_eq!(read_info(&bad), Err(Error::Corrupt));
    }

    /// A `control.tar` of `control` ("Package: tiny\nVersion: 1\n") and an
    /// `md5sums` of 2 KiB, compressed by gzip(1) and by xz(1) with
    /// `--block-size=600` (18 blocks).
    const GZ: &[u8] = include_bytes!("../testdata/control.tar.gz");
    const XZ: &[u8] = include_bytes!("../testdata/control.tar.xz");

    /// A zstd frame (RFC 8878) holding `data` in raw blocks: the magic, a
    /// frame header with the content size in two bytes, the blocks.
    fn zstd_raw(data: &[u8]) -> Vec<u8> {
        assert!((256..65536 + 256).contains(&data.len()));
        let mut out = alloc::vec![0x28, 0xb5, 0x2f, 0xfd, 0x60];
        out.extend_from_slice(&((data.len() - 256) as u16).to_le_bytes());
        let mut chunks = data.chunks(4096).peekable();
        while let Some(c) = chunks.next() {
            let header = ((c.len() as u32) << 3) | u32::from(chunks.peek().is_none());
            out.extend_from_slice(&header.to_le_bytes()[..3]);
            out.extend_from_slice(c);
        }
        out
    }

    #[test]
    fn every_compression_dpkg_writes_is_read() {
        let zst = zstd_raw(&tar::build(&[("control", b"Package: tiny\nVersion: 1\n")]));
        for (name, data) in [("control.tar.gz", GZ), ("control.tar.xz", XZ), ("control.tar.zst", zst.as_slice())] {
            let deb = ar::build(&[("debian-binary", b"2.0\n"), (name, data), ("data.tar", b"")]);
            let info = read_info(&deb).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!((info.name.as_str(), info.version.as_str()), ("tiny", "1"), "{name}");
        }
    }

    #[test]
    fn sizes_read_as_people_write_them() {
        assert_eq!(human_kib(812), "812 KB");
        assert_eq!(human_kib(1297), "1.2 MB");
        assert_eq!(human_kib(1024 * 1024 * 2 + 150_000), "2.1 GB");
    }
}
