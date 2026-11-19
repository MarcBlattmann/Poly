//! What a Windows program says about itself (ADR 0230): its name, version,
//! maker and description from the version resource, its icon from the icon
//! resources, whether it is signed, whether it is an installer.
//!
//! A file is read at the offsets it needs, never whole: the headers, the
//! resource directory and the few resources asked for (Google's offline
//! installer is 170 MB, most of it one resource).
//!
//! - **The version resource** (`RT_VERSION`, 16; Learn, "VS_VERSIONINFO",
//!   "StringFileInfo", "String", "VS_FIXEDFILEINFO"): a tree of nodes, each
//!   `wLength`, `wValueLength`, `wType` (1: text), a NUL-terminated UTF-16
//!   key, padding to 32 bits, the value, padding, the children.
//! - **Icons** (`RT_GROUP_ICON`, 14, and `RT_ICON`, 3; Learn, "Resource File
//!   Formats", `GRPICONDIR`/`GRPICONDIRENTRY`): a group lists its images
//!   (width, height, colours, planes, bits, size, the `RT_ICON` id); an
//!   image is a PNG file or a DIB (`BITMAPINFOHEADER`, the colour bitmap at
//!   twice its height with the AND mask below it). A DIB becomes a PNG here
//!   (stored deflate blocks: the launcher decodes PNG files).
//! - **The certificate table** (data directory 4, its "RVA" a file offset):
//!   present means the file is Authenticode-signed; who signed it is
//!   `poly_x509::authenticode::publisher_of`, the reader the consent prompt
//!   uses too (ADR 0222), named the same way (`publisher_label`).
//! - **Windows Installer packages** ([MS-CFB]): a compound file whose root
//!   storage has the class `{000C1084-0000-0000-C000-000000000046}`.

use crate::Error;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_pe::{Bits, Pe, ResourceDirectory, ResourceKey, dir, resource_type};

/// Reads into the buffer at a file offset; how many bytes it read (0 at the
/// end of the file or on an error).
pub type ReadAt<'a> = &'a mut dyn FnMut(u64, &mut [u8]) -> usize;

/// How much of the file is read for the headers first, and at most.
const HEADERS: usize = 0x1000;
const MORE_HEADERS: usize = 0x10000;
/// How much of the resource section is read for its directory first, and
/// at most (the tables are at the section's start; a name or a table may be
/// further in).
const TREE_FIRST: usize = 1 << 20;
const TREE_MAX: usize = 32 << 20;
/// The largest single resource read (a version block, an icon image).
const MAX_ITEM: usize = 8 << 20;
/// The largest icon decoded from a DIB.
const MAX_DIB_SIDE: u32 = 512;

/// The PNG file signature.
pub const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// What the installer shows about a Windows program.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// The heading: `ProductName`, else `FileDescription`, else the file's
    /// name without `.exe`.
    pub name: String,
    /// `FileDescription`.
    pub description: String,
    /// `CompanyName`.
    pub company: String,
    /// `ProductVersion` (else `FileVersion`, else the fixed version).
    pub version: String,
    /// The file's size in bytes.
    pub size: u64,
    /// 32 or 64.
    pub bits: u8,
    /// A program with windows (`IMAGE_SUBSYSTEM_WINDOWS_GUI`), not a console
    /// program.
    pub gui: bool,
    /// Whether it installs something (its names say "setup" or "install"):
    /// the window offers **Install** (run it and pick up what it made)
    /// instead of **Run**.
    pub installer: bool,
    /// Authenticode-signed (it has a certificate table).
    pub signed: bool,
    /// Who signed it (the first signer's name; the signature is not checked).
    pub publisher: Option<String>,
    /// Its own icon, as a PNG file.
    pub icon: Option<Vec<u8>>,
}

impl Program {
    /// The window's "Publisher" fact.
    pub fn publisher_text(&self) -> String {
        match (&self.publisher, self.signed) {
            (Some(p), _) => poly_x509::authenticode::publisher_label(p),
            (None, true) => "Unknown publisher (signed)".to_string(),
            (None, false) => "Unknown publisher".to_string(),
        }
    }

    /// The window's "Kind" fact.
    pub fn kind(&self) -> String {
        let what = if self.installer { "Windows installer" } else { "Windows program" };
        alloc::format!("{what} ({}-bit)", self.bits)
    }
}

fn read_upto(read: ReadAt<'_>, at: u64, len: usize) -> Vec<u8> {
    let mut buf = alloc::vec![0u8; len];
    let mut got = 0;
    while got < len {
        let n = read(at + got as u64, &mut buf[got..]);
        if n == 0 {
            break;
        }
        got += n;
    }
    buf.truncate(got);
    buf
}

/// An image file opened for its resources.
#[allow(missing_debug_implementations)]
pub struct Image<'f> {
    head: Vec<u8>,
    read: ReadAt<'f>,
    /// The resource directory's bytes, from its start, and its RVA.
    tree: Option<(Vec<u8>, u32)>,
}

/// Which entry of a resource directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The first (in the directory's order: names, then ids ascending).
    First,
    /// The `n`th, from 0.
    Nth(usize),
    /// The one with this id.
    Id(u32),
}

impl<'f> Image<'f> {
    /// Open the PE image `read` reads. `Err`: not a PE image.
    pub fn open(read: ReadAt<'f>) -> Result<Image<'f>, Error> {
        let mut head = read_upto(read, 0, HEADERS);
        if head.get(..2) != Some(b"MZ") {
            return Err(Error::NotThisFormat);
        }
        match Pe::parse(&head) {
            Ok(_) => {}
            Err(poly_pe::Error::Truncated) => {
                head = read_upto(read, 0, MORE_HEADERS);
                Pe::parse(&head).map_err(|_| Error::NotThisFormat)?;
            }
            Err(_) => return Err(Error::NotThisFormat),
        }
        let mut image = Image { head, read, tree: None };
        image.tree = image.load_tree();
        Ok(image)
    }

    fn pe(&self) -> Pe<'_> {
        Pe::parse(&self.head).expect("checked when opened")
    }

    /// The file offset of `rva`, if a section holds it in the file.
    fn offset_of(&self, rva: u32) -> Option<(u64, usize)> {
        let pe = self.pe();
        let (_, s) = pe.sections().find_rva(rva)?;
        let into = rva.checked_sub(s.virtual_address)?;
        if into >= s.file_backed_size() {
            return None;
        }
        Some((u64::from(s.pointer_to_raw_data) + u64::from(into), (s.file_backed_size() - into) as usize))
    }

    fn load_tree(&mut self) -> Option<(Vec<u8>, u32)> {
        let d = self.pe().info().directory(dir::RESOURCE).filter(|d| d.virtual_address != 0 && d.size != 0)?;
        let (at, room) = self.offset_of(d.virtual_address)?;
        let mut len = room.min(TREE_FIRST);
        loop {
            let tree = read_upto(self.read, at, len);
            let complete = tree.len() < len || len >= room.min(TREE_MAX);
            // The whole directory has to be walkable in what was read.
            let ok = ResourceDirectory::from_tree(&tree).is_ok_and(|root| root.walk(|_, _| {}).is_ok());
            if ok || complete {
                return ok.then_some((tree, d.virtual_address));
            }
            len = (len * 4).min(room).min(TREE_MAX);
        }
    }

    /// Whether the file has a certificate table (Authenticode).
    pub fn signed(&self) -> bool {
        self.pe().info().directory(dir::SECURITY).is_some_and(|d| d.virtual_address != 0 && d.size >= 8)
    }

    /// 32 or 64.
    pub fn bits(&self) -> u8 {
        if self.pe().info().bits() == Bits::Pe32Plus { 64 } else { 32 }
    }

    /// Whether it is a DLL.
    pub fn is_dll(&self) -> bool {
        self.pe().info().is_dll()
    }

    /// Whether it is a program with windows.
    pub fn gui(&self) -> bool {
        self.pe().optional_header().subsystem == poly_pe::subsystem::WINDOWS_GUI
    }

    /// The bytes of resource `kind` picked by `name`, its first language.
    pub fn resource(&mut self, kind: u32, name: Pick) -> Option<Vec<u8>> {
        let data = {
            let (tree, _) = self.tree.as_ref()?;
            let root = ResourceDirectory::from_tree(tree).ok()?;
            let names = root.find(ResourceKey::Id(kind)).ok()??.directory().ok()??;
            let entry = match name {
                Pick::First => names.entries().next()?.ok()?,
                Pick::Nth(n) => names.entries().nth(n)?.ok()?,
                Pick::Id(id) => names.find(ResourceKey::Id(id)).ok()??,
            };
            let langs = entry.directory().ok()??;
            langs.entries().next()?.ok()?.data().ok()??
        };
        let len = data.size as usize;
        if len > MAX_ITEM {
            return None;
        }
        let (at, room) = self.offset_of(data.data_rva)?;
        if len > room {
            return None;
        }
        let bytes = read_upto(self.read, at, len);
        (bytes.len() == len).then_some(bytes)
    }

    /// How many entries resource type `kind` has.
    pub fn count(&self, kind: u32) -> usize {
        let Some((tree, _)) = self.tree.as_ref() else { return 0 };
        let Ok(root) = ResourceDirectory::from_tree(tree) else { return 0 };
        root.find(ResourceKey::Id(kind))
            .ok()
            .flatten()
            .and_then(|e| e.directory().ok().flatten())
            .map_or(0, |d| d.len())
    }

    /// The icon of icon group `group` (Learn's "icon index" of a shortcut:
    /// from 0 the group's place, below 0 its negated id) as a PNG file: the
    /// largest image, a PNG one before a DIB of the same size.
    pub fn icon(&mut self, group: i32) -> Option<Vec<u8>> {
        let pick = if group >= 0 { Pick::Nth(group as usize) } else { Pick::Id(group.unsigned_abs()) };
        let dir = self
            .resource(resource_type::GROUP_ICON, pick)
            .or_else(|| self.resource(resource_type::GROUP_ICON, Pick::First))?;
        let count = usize::from(u16::from_le_bytes([*dir.get(4)?, *dir.get(5)?]));
        // (width, PNG first, bits) and the image's id.
        let mut entries: Vec<((u32, bool, u16), u32)> = Vec::new();
        for i in 0..count.min(64) {
            let Some(e) = dir.get(6 + i * 14..6 + i * 14 + 14) else { break };
            let width = if e[0] == 0 { 256 } else { u32::from(e[0]) };
            let bits = u16::from_le_bytes([e[6], e[7]]);
            let id = u32::from(u16::from_le_bytes([e[12], e[13]]));
            entries.push(((width, false, bits), id));
        }
        // Largest first; the kind of image is known once it is read.
        entries.sort_by_key(|e| core::cmp::Reverse(e.0));
        let mut best: Option<((u32, bool, u16), Vec<u8>)> = None;
        for ((width, _, bits), id) in entries {
            if best.as_ref().is_some_and(|(k, _)| k.0 > width) {
                break;
            }
            let Some(image) = self.resource(resource_type::ICON, Pick::Id(id)) else { continue };
            let (key, png) = if image.starts_with(PNG_SIGNATURE) {
                ((width, true, bits), image)
            } else {
                match dib_to_png(&image) {
                    Some(png) => ((width, false, bits), png),
                    None => continue,
                }
            };
            if best.as_ref().is_none_or(|(k, _)| key > *k) {
                best = Some((key, png));
            }
        }
        best.map(|(_, png)| png)
    }

    /// The strings and the fixed version of the version resource.
    pub fn version(&mut self) -> Option<VersionInfo> {
        let block = self.resource(resource_type::VERSION, Pick::First)?;
        Some(parse_version(&block))
    }
}

/// What a version resource holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionInfo {
    /// `VS_FIXEDFILEINFO`'s file and product versions (four parts each).
    pub file_version: Option<[u16; 4]>,
    pub product_version: Option<[u16; 4]>,
    /// The strings of the first string table (name, value).
    pub strings: Vec<(String, String)>,
}

impl VersionInfo {
    /// The string `name` (case-insensitive), if it is there and not empty.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.strings.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// One node of a version tree.
struct Node {
    key: String,
    /// The value's bytes.
    value: (usize, usize),
    text: bool,
    children: usize,
    end: usize,
}

fn utf16(b: &[u8], from: usize, to: usize) -> String {
    let units: Vec<u16> = (from..to).step_by(2).filter_map(|i| u16_at(b, i)).collect();
    String::from_utf16_lossy(&units)
}

fn node(b: &[u8], at: usize, limit: usize) -> Option<Node> {
    let length = usize::from(u16_at(b, at)?);
    let value_len = usize::from(u16_at(b, at + 2)?);
    let text = u16_at(b, at + 4)? == 1;
    if length < 6 {
        return None;
    }
    let end = (at + length).min(limit).min(b.len());
    let key_start = at + 6;
    let mut p = key_start;
    while u16_at(b, p)? != 0 {
        p += 2;
        if p >= end {
            return None;
        }
    }
    let key = utf16(b, key_start, p);
    let value = align4(p + 2);
    let bytes = if text { value_len * 2 } else { value_len };
    let value_end = (value + bytes).min(end);
    Some(Node { key, value: (value.min(end), value_end), text, children: align4(value + bytes), end })
}

fn children(b: &[u8], parent: &Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut at = parent.children;
    while at + 6 <= parent.end && out.len() < 4096 {
        let Some(n) = node(b, at, parent.end) else { break };
        at = align4(n.end.max(at + 6));
        out.push(n);
    }
    out
}

fn fixed_version(b: &[u8], at: usize) -> Option<[u16; 4]> {
    let ms = u32_at(b, at)?;
    let ls = u32_at(b, at + 4)?;
    Some([(ms >> 16) as u16, ms as u16, (ls >> 16) as u16, ls as u16])
}

/// The strings and fixed versions of a `VS_VERSIONINFO` block.
pub fn parse_version(block: &[u8]) -> VersionInfo {
    let mut info = VersionInfo::default();
    let Some(root) = node(block, 0, block.len()) else { return info };
    let (v, ve) = root.value;
    // VS_FIXEDFILEINFO: dwSignature 0xFEEF04BD, dwStrucVersion, then the
    // file version (MS, LS) and the product version (MS, LS).
    if ve - v >= 52 && u32_at(block, v) == Some(0xFEEF_04BD) {
        info.file_version = fixed_version(block, v + 8);
        info.product_version = fixed_version(block, v + 16);
    }
    let Some(sfi) = children(block, &root).into_iter().find(|n| n.key == "StringFileInfo") else { return info };
    let tables = children(block, &sfi);
    // U.S. English in Unicode or Windows-1252 if there is such a table, else the first.
    let table =
        tables.iter().find(|t| t.key.eq_ignore_ascii_case("040904b0") || t.key.eq_ignore_ascii_case("040904e4"));
    let Some(table) = table.or(tables.first()) else { return info };
    for s in children(block, table) {
        let (from, to) = s.value;
        let mut value = if s.text || to > from { utf16(block, from, to) } else { String::new() };
        if let Some(nul) = value.find('\0') {
            value.truncate(nul);
        }
        info.strings.push((s.key, value));
    }
    info
}

/// A version as `a.b.c.d`.
pub fn dotted(v: [u16; 4]) -> String {
    alloc::format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3])
}

/// Whether one of a program's names says it installs something.
pub fn names_an_installer(names: &[&str]) -> bool {
    names.iter().any(|n| {
        let n = n.to_ascii_lowercase();
        n.contains("setup") || n.contains("install")
    })
}

/// What the installer shows about the program file `file_name` of `size`
/// bytes that `read` reads. `Err`: not a Windows program (a DLL, or no PE
/// image at all).
pub fn read_program(file_name: &str, size: u64, read: ReadAt<'_>) -> Result<Program, Error> {
    let mut image = Image::open(read)?;
    if image.is_dll() {
        return Err(Error::Library);
    }
    let version = image.version().unwrap_or_default();
    let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let stem = match stem.len().checked_sub(4) {
        Some(cut) if stem.is_char_boundary(cut) && stem[cut..].eq_ignore_ascii_case(".exe") => &stem[..cut],
        _ => stem,
    };
    let name = version.get("ProductName").or(version.get("FileDescription")).unwrap_or(stem).to_string();
    let text_version = version.get("ProductVersion").or(version.get("FileVersion")).map(|v| v.replace(", ", "."));
    let fixed = version.product_version.or(version.file_version).filter(|v| v.iter().any(|&p| p != 0));
    let installer = names_an_installer(&[
        stem,
        version.get("FileDescription").unwrap_or(""),
        version.get("ProductName").unwrap_or(""),
        version.get("OriginalFilename").unwrap_or(""),
        version.get("InternalName").unwrap_or(""),
    ]);
    Ok(Program {
        name,
        description: version.get("FileDescription").unwrap_or("").to_string(),
        company: version.get("CompanyName").unwrap_or("").to_string(),
        version: text_version.or(fixed.map(dotted)).unwrap_or_default(),
        size,
        bits: image.bits(),
        gui: image.gui(),
        installer,
        signed: image.signed(),
        publisher: poly_x509::authenticode::publisher_of(&mut *image.read),
        icon: image.icon(0),
    })
}

/// The compound-file signature ([MS-CFB] 2.2, `Header Signature`).
pub const CFB_SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
/// The class of a Windows Installer package's root storage,
/// `{000C1084-0000-0000-C000-000000000046}`, as its 16 bytes on disk.
pub const MSI_CLSID: [u8; 16] = [0x84, 0x10, 0x0C, 0x00, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

/// Whether the compound file `read` reads is a Windows Installer package:
/// the root directory entry ([MS-CFB] 2.6: the first entry of the first
/// directory sector, its CLSID at offset 0x50) has [`MSI_CLSID`]. The
/// header gives the sector size (`Sector Shift` at 0x1E) and the first
/// directory sector (at 0x30); sector `n` starts at `(n + 1) << shift`.
pub fn is_msi(read: ReadAt<'_>) -> bool {
    let head = read_upto(read, 0, 0x200);
    if head.len() < 0x200 || head[..8] != CFB_SIGNATURE {
        return false;
    }
    let (Some(shift), Some(first)) = (u16_at(&head, 0x1E), u32_at(&head, 0x30)) else { return false };
    if !(9..=12).contains(&shift) || first >= 0xFFFF_FFFA {
        return false;
    }
    let at = (u64::from(first) + 1) << shift;
    let root = read_upto(read, at, 0x80);
    // The root entry: type 5 (root storage) at 0x42.
    root.len() == 0x80 && root[0x42] == 5 && root[0x50..0x60] == MSI_CLSID
}

/// An icon image in DIB form (`BITMAPINFOHEADER`, the colours, the colour
/// bitmap bottom-up, the AND mask) as a PNG file: 32-bit with its alpha,
/// 24-bit and 1/4/8-bit with the palette, transparency from the mask.
pub fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>> {
    let header = u32_at(dib, 0)? as usize;
    if header < 40 {
        return None;
    }
    let width = u32_at(dib, 4)? as i32;
    // The colour bitmap and the mask together: twice the height.
    let height = (u32_at(dib, 8)? as i32) / 2;
    let bits = u16_at(dib, 14)?;
    let compression = u32_at(dib, 16)?;
    if width <= 0 || height <= 0 || width as u32 > MAX_DIB_SIDE || height as u32 > MAX_DIB_SIDE || compression != 0 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let used = u32_at(dib, 32)? as usize;
    let colours = match bits {
        1 | 4 | 8 => {
            if used == 0 {
                1 << bits
            } else {
                used.min(256)
            }
        }
        24 | 32 => 0,
        _ => return None,
    };
    let palette = header;
    let pixels = palette + colours * 4;
    let stride = (w * usize::from(bits)).div_ceil(32) * 4;
    let mask = pixels + stride * h;
    let mask_stride = w.div_ceil(32) * 4;
    let mut rgba = alloc::vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        // Bottom-up rows.
        let row = pixels + (h - 1 - y) * stride;
        for x in 0..w {
            let (r, g, b, a) = match bits {
                32 => {
                    let p = dib.get(row + x * 4..row + x * 4 + 4)?;
                    any_alpha |= p[3] != 0;
                    (p[2], p[1], p[0], p[3])
                }
                24 => {
                    let p = dib.get(row + x * 3..row + x * 3 + 3)?;
                    (p[2], p[1], p[0], 255)
                }
                _ => {
                    let bit = x * usize::from(bits);
                    let byte = *dib.get(row + bit / 8)?;
                    let shift = 8 - usize::from(bits) - bit % 8;
                    let index = usize::from(byte >> shift) & ((1 << bits) - 1);
                    let c = dib.get(palette + index * 4..palette + index * 4 + 4)?;
                    (c[2], c[1], c[0], 255)
                }
            };
            let o = (y * w + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    // Without an alpha channel the AND mask says what is transparent.
    if !(bits == 32 && any_alpha) {
        for y in 0..h {
            let row = mask + (h - 1 - y) * mask_stride;
            for x in 0..w {
                let transparent = dib.get(row + x / 8).is_some_and(|b| b & (0x80 >> (x % 8)) != 0);
                rgba[(y * w + x) * 4 + 3] = if transparent { 0 } else { 255 };
            }
        }
    }
    Some(encode_png(width as u32, height as u32, &rgba))
}

/// An RGBA image as a PNG file (8-bit RGBA, no filter, stored deflate
/// blocks: RFC 1950/1951, the PNG specification).
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = alloc::vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(0xFFFF).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    if blocks.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    z.extend_from_slice(&poly_png::inflate::adler32(&raw).to_be_bytes());
    let mut out = PNG_SIGNATURE.to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&poly_png::crc32(&[kind, data]).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}
