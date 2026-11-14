//! AppImage, type 2 (ADR 0180): what the installer needs to know of one
//! before it installs it, and where an installed one lives.
//!
//! A type 2 AppImage is an ELF program (the *runtime*) with a squashfs
//! image appended: `AI` and the type are at offset 8 of the ELF header, and
//! the squashfs starts where the ELF file ends, which the ELF header says
//! (the end of the section header table, as the AppImage specification
//! describes). The image holds an AppDir: a `.desktop` file and an icon at
//! its top, `AppRun`, and the app's files. This module reads the desktop
//! file and the icon from the image with `poly-squashfs` (the reader the
//! kernel mounts squashfs with), so installing never has to run the app.
//!
//! The rest is naming: an installed AppImage is a directory of its own in
//! the user's data directory (`$XDG_DATA_HOME/poly/appimages/<slug>/`) with
//! the image and its icon, and a desktop file in the user's applications
//! directory (`$XDG_DATA_HOME/applications/poly-appimage-<slug>.desktop`)
//! that the launcher and Files' Applications read like any other. That
//! desktop file names its image (`X-Poly-AppImage`), which is how an
//! installed AppImage is told from other apps, and everything uninstalling
//! removes follows from the slug.

use crate::Error;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_squashfs::{IoFailed, Source, Squashfs};
use poly_vfs::{FileSystem, FileType, Ino};

/// How many bytes from the start of the file [`squashfs_offset`] needs.
pub const HEAD: usize = 64;

/// The desktop file ID prefix of an installed AppImage.
pub const ID_PREFIX: &str = "poly-appimage-";

/// The desktop entry key that names the installed image.
pub const IMAGE_KEY: &str = "X-Poly-AppImage";

/// The environment variable the type 2 runtime reads to extract the image
/// to a temporary directory and run it from there, without mounting it with
/// FUSE (the AppImage project's `APPIMAGE_EXTRACT_AND_RUN`).
pub const EXTRACT_AND_RUN: &str = "APPIMAGE_EXTRACT_AND_RUN";

/// Whether an AppImage has to be started by extracting it
/// ([`EXTRACT_AND_RUN`]) on Poly instead of by mounting its image through
/// FUSE (ADR 0180 says which works and why).
pub const RUNS_BY_EXTRACTING: bool = false;

/// Most bytes of an icon or a desktop file read from an image.
const FILE_LIMIT: u64 = 4 << 20;
/// Most links followed in one path.
const HOPS: usize = 8;

fn le16(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?)))
}

fn le32(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?)))
}

fn le64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Where the squashfs image starts in the AppImage whose first bytes are
/// `head`: the end of the ELF file, that is the end of its section header
/// table (`e_shoff + e_shentsize * e_shnum`). `Err` for a file that is not
/// an AppImage of type 2.
pub fn squashfs_offset(head: &[u8]) -> Result<u64, Error> {
    if !head.starts_with(b"\x7fELF") || head.get(8..10) != Some(b"AI") {
        return Err(Error::NotThisFormat);
    }
    match head.get(10) {
        Some(2) => {}
        Some(1) => return Err(Error::OldAppImage),
        _ => return Err(Error::NotThisFormat),
    }
    if head.get(5) != Some(&1) {
        // Big endian: no such AppImage runs on Poly.
        return Err(Error::NotThisFormat);
    }
    let (shoff, entsize, num) = match head.get(4) {
        Some(2) => (le64(head, 0x28), le16(head, 0x3a), le16(head, 0x3c)),
        Some(1) => (le32(head, 0x20), le16(head, 0x2e), le16(head, 0x30)),
        _ => return Err(Error::NotThisFormat),
    };
    let (shoff, entsize, num) =
        (shoff.ok_or(Error::Truncated)?, entsize.ok_or(Error::Truncated)?, num.ok_or(Error::Truncated)?);
    shoff.checked_add(entsize * num).ok_or(Error::Corrupt)
}

/// The image's bytes: reads of the AppImage file from `base` on.
pub struct Region<F> {
    read: F,
    base: u64,
    size: u64,
}

impl<F> core::fmt::Debug for Region<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Region").field("base", &self.base).field("size", &self.size).finish_non_exhaustive()
    }
}

impl<F: Fn(u64, &mut [u8]) -> bool + Send + Sync> Source for Region<F> {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), IoFailed> {
        let at = self.base.checked_add(offset).ok_or(IoFailed)?;
        if (self.read)(at, buf) { Ok(()) } else { Err(IoFailed) }
    }
}

/// What the installer shows of an AppImage and what it installs from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Image {
    /// The desktop file's `Name`.
    pub name: String,
    /// `Comment`, else `GenericName`.
    pub summary: String,
    /// `X-AppImage-Version`, if the desktop file has one.
    pub version: String,
    /// The desktop file as the image has it.
    pub desktop: String,
    /// The icon's PNG file, if the image has one.
    pub icon: Option<Vec<u8>>,
    /// The AppImage file's size in bytes.
    pub size: u64,
}

/// The first group of a desktop file as key/value pairs (the Desktop Entry
/// Specification: `[Desktop Entry]`, `Key=Value`, `#` comments); keys
/// translated into other languages (`Name[de]`) are left out.
pub fn desktop_keys(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
            continue;
        }
        if let (true, Some((k, v))) = (inside, line.split_once('=')) {
            let k = k.trim();
            if !k.contains('[') {
                out.push((k, v.trim()));
            }
        }
    }
    out
}

fn key<'a>(keys: &[(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    keys.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
}

/// The inode at `path` in the image (an absolute or relative path from the
/// root), following links, `None` if there is none or links loop.
fn lookup<S: Source>(fs: &Squashfs<S>, path: &str) -> Option<Ino> {
    let mut parts: Vec<String> = path.split('/').filter(|p| !p.is_empty()).map(ToString::to_string).collect();
    let mut at = fs.root();
    let mut hops = 0;
    let mut i = 0;
    while i < parts.len() {
        let ino = match parts[i].as_str() {
            "." => {
                i += 1;
                continue;
            }
            ".." => fs.lookup(at, b"..").ok()?,
            name => fs.lookup(at, name.as_bytes()).ok()?,
        };
        let attr = fs.getattr(ino).ok()?;
        if attr.kind == FileType::Symlink {
            hops += 1;
            if hops > HOPS {
                return None;
            }
            let target = fs.readlink(ino).ok()?;
            let target = core::str::from_utf8(&target).ok()?;
            // The link replaces its own name; an absolute one starts over.
            let mut rest: Vec<String> = target.split('/').filter(|p| !p.is_empty()).map(ToString::to_string).collect();
            rest.extend(parts.drain(i + 1..));
            parts.truncate(i);
            if target.starts_with('/') {
                parts.clear();
                at = fs.root();
                i = 0;
            }
            parts.extend(rest);
            continue;
        }
        at = ino;
        i += 1;
    }
    Some(at)
}

fn read_file<S: Source>(fs: &Squashfs<S>, ino: Ino) -> Option<Vec<u8>> {
    let attr = fs.getattr(ino).ok()?;
    if attr.kind != FileType::Regular || attr.size > FILE_LIMIT {
        return None;
    }
    let mut out = alloc::vec![0u8; attr.size as usize];
    let mut at = 0;
    while at < out.len() {
        let n = fs.read(ino, at as u64, &mut out[at..]).ok()?;
        if n == 0 {
            return None;
        }
        at += n;
    }
    Some(out)
}

/// A PNG file's signature.
fn is_png(data: &[u8]) -> bool {
    data.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
}

/// The names of the top directory's desktop files.
fn desktop_names<S: Source>(fs: &Squashfs<S>) -> Vec<String> {
    let mut names = Vec::new();
    let _ = fs.readdir(fs.root(), 0, &mut |e| {
        if let Ok(n) = core::str::from_utf8(e.name)
            && n.ends_with(".desktop")
            && e.kind != FileType::Directory
        {
            names.push(n.to_string());
        }
        true
    });
    names.sort();
    names
}

/// The icon the image offers for the desktop entry's `Icon`: the file
/// `.DirIcon` names (the AppImage specification: a PNG of the icon), else
/// `<Icon>.png` at the top, else the biggest `<Icon>.png` of the icon
/// theme inside it.
fn find_icon<S: Source>(fs: &Squashfs<S>, icon: &str) -> Option<Vec<u8>> {
    let png = |path: &str| lookup(fs, path).and_then(|i| read_file(fs, i)).filter(|d| is_png(d));
    if let Some(d) = png(".DirIcon") {
        return Some(d);
    }
    let icon = icon.trim_end_matches(".png");
    if !icon.is_empty() && !icon.contains('/') {
        if let Some(d) = png(&alloc::format!("{icon}.png")) {
            return Some(d);
        }
        let hicolor = lookup(fs, "usr/share/icons/hicolor")?;
        let mut sizes: Vec<String> = Vec::new();
        let _ = fs.readdir(hicolor, 0, &mut |e| {
            if let Ok(n) = core::str::from_utf8(e.name) {
                sizes.push(n.to_string());
            }
            true
        });
        // `256x256` before `48x48`, `scalable` (no PNG) last.
        let edge = |s: &String| s.split('x').next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
        sizes.sort_by_key(|s| core::cmp::Reverse(edge(s)));
        for size in sizes {
            if let Some(d) = png(&alloc::format!("usr/share/icons/hicolor/{size}/apps/{icon}.png")) {
                return Some(d);
            }
        }
    }
    None
}

/// Read an AppImage: `read` fills a buffer from the file at an offset
/// (`false` on failure), `size` is the file's size, `head` its first bytes
/// (at least [`HEAD`]).
pub fn read<F>(head: &[u8], size: u64, read: F) -> Result<Image, Error>
where
    F: Fn(u64, &mut [u8]) -> bool + Send + Sync,
{
    let base = squashfs_offset(head)?;
    if base >= size {
        return Err(Error::Truncated);
    }
    let fs = Squashfs::mount(Region { read, base, size: size - base }).map_err(|_| Error::Corrupt)?;
    let names = desktop_names(&fs);
    let name = names.first().ok_or(Error::NoDesktopEntry)?;
    let ino = fs.lookup(fs.root(), name.as_bytes()).map_err(|_| Error::Corrupt)?;
    let text = read_file(&fs, ino).ok_or(Error::Corrupt)?;
    let desktop = String::from_utf8_lossy(&text).into_owned();
    let keys = desktop_keys(&desktop);
    let app_name = key(&keys, "Name").filter(|n| !n.is_empty()).ok_or(Error::NoDesktopEntry)?.to_string();
    let summary = key(&keys, "Comment").or_else(|| key(&keys, "GenericName")).unwrap_or("").to_string();
    let version = key(&keys, "X-AppImage-Version").unwrap_or("").to_string();
    let icon = find_icon(&fs, key(&keys, "Icon").unwrap_or(""));
    Ok(Image { name: app_name, summary, version, desktop, icon, size })
}

/// A name as one word of a file name: ASCII letters and digits, lowercase,
/// other runs of characters one `-`; at most 40 characters, never empty.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() { "app".to_string() } else { out.to_string() }
}

/// Whether `slug` is one this module makes (so a path built from it stays
/// inside the AppImage directories).
pub fn is_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 40
        && !slug.starts_with('-')
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The desktop file ID of the AppImage with `slug`.
pub fn id_of(slug: &str) -> String {
    alloc::format!("{ID_PREFIX}{slug}.desktop")
}

/// The slug of an installed AppImage's desktop file ID, if it is one.
pub fn slug_of_id(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX)?.strip_suffix(".desktop").filter(|s| is_slug(s))
}

/// Where an installed AppImage's files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub slug: String,
    /// The desktop file ID.
    pub id: String,
    /// The directory of the app: its image and icon.
    pub dir: String,
    pub image: String,
    pub icon: String,
    /// The desktop file.
    pub desktop: String,
}

/// The files of the AppImage with `slug` below the user's data directory
/// `data_home` (`$XDG_DATA_HOME`, `~/.local/share` without it).
pub fn layout(data_home: &str, slug: &str) -> Layout {
    let data = data_home.trim_end_matches('/');
    let dir = alloc::format!("{data}/poly/appimages/{slug}");
    let id = id_of(slug);
    Layout {
        slug: slug.to_string(),
        image: alloc::format!("{dir}/{slug}.AppImage"),
        icon: alloc::format!("{dir}/icon.png"),
        desktop: alloc::format!("{data}/applications/{id}"),
        id,
        dir,
    }
}

/// The user's applications directory below `data_home`.
pub fn applications_dir(data_home: &str) -> String {
    alloc::format!("{}/applications", data_home.trim_end_matches('/'))
}

/// An `Exec` argument: quoted if it holds anything the Desktop Entry
/// Specification's `Exec` would split or interpret.
fn exec_arg(arg: &str) -> String {
    if arg.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '"'
                | '\''
                | '\\'
                | '>'
                | '<'
                | '~'
                | '|'
                | '&'
                | ';'
                | '$'
                | '*'
                | '?'
                | '#'
                | '('
                | ')'
                | '`'
                | '%'
        )
    }) {
        let mut out = String::from("\"");
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    } else {
        arg.to_string()
    }
}

/// The desktop file the installer writes for `image` at `layout`: the
/// image's own entry reduced to what the launcher and Files read, started
/// through the installed copy (`extract_and_run`: the way the app has to be
/// started on this system, see [`run_environment`]), with the icon that was
/// taken out of the image.
pub fn desktop_file(image: &Image, layout: &Layout, extract_and_run: bool) -> String {
    let keys = desktop_keys(&image.desktop);
    let mut out = String::from("[Desktop Entry]\nType=Application\n");
    let clean = |v: &str| v.replace(['\n', '\r'], " ");
    out.push_str(&alloc::format!("Name={}\n", clean(&image.name)));
    if !image.summary.is_empty() {
        out.push_str(&alloc::format!("Comment={}\n", clean(&image.summary)));
    }
    let exec = exec_arg(&layout.image);
    if extract_and_run {
        out.push_str(&alloc::format!("Exec=env {EXTRACT_AND_RUN}=1 {exec}\n"));
    } else {
        out.push_str(&alloc::format!("Exec={exec}\n"));
    }
    if image.icon.is_some() {
        out.push_str(&alloc::format!("Icon={}\n", layout.icon));
    }
    for k in ["Categories", "Keywords"] {
        if let Some(v) = key(&keys, k).filter(|v| !v.is_empty()) {
            out.push_str(&alloc::format!("{k}={}\n", clean(v)));
        }
    }
    if key(&keys, "Terminal") == Some("true") {
        out.push_str("Terminal=true\n");
    }
    out.push_str(&alloc::format!("{IMAGE_KEY}={}\n", layout.image));
    out
}

/// The image path a desktop file of an installed AppImage names.
pub fn image_of_desktop(text: &str) -> Option<String> {
    key(&desktop_keys(text), IMAGE_KEY).filter(|p| p.starts_with('/')).map(ToString::to_string)
}

/// Environment variables to start the AppImage at `path` with: none when
/// the runtime can mount the image itself, [`EXTRACT_AND_RUN`] when it has
/// to extract it.
pub fn run_environment(extract_and_run: bool) -> Vec<(&'static str, &'static str)> {
    if extract_and_run { alloc::vec![(EXTRACT_AND_RUN, "1")] } else { Vec::new() }
}

#[cfg(test)]
mod tests;
