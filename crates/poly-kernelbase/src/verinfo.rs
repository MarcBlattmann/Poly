//! Version information (Chrome batch 2, ADR 0155): the logic of our
//! `version.dll` (`GetFileVersionInfoSizeW`, `GetFileVersionInfoW`,
//! `VerQueryValueW`), as host-tested code.
//!
//! Learn's pages for those functions, "VS_VERSIONINFO", "StringFileInfo",
//! "VarFileInfo", "String", "Var", "VS_FIXEDFILEINFO" and "Version
//! Information" give the structures; the PE format specification gives the
//! resource section (`RT_VERSION` is resource type 16, the version is the
//! resource named 1). A file is read at the offsets it needs only: the
//! headers and the section that holds the resources (a 300 MB program is
//! not read whole).

use crate::error::*;
use alloc::vec;
use alloc::vec::Vec;
use poly_pe::{Pe, ResourceDirectory, ResourceKey, dir, resource_type};

/// `ERROR_RESOURCE_DATA_NOT_FOUND` (`winerror.h`).
pub const ERROR_RESOURCE_DATA_NOT_FOUND: u32 = 1812;
/// `ERROR_BAD_EXE_FORMAT` (`winerror.h`): the file is not a PE image.
pub const ERROR_BAD_EXE_FORMAT: u32 = 193;

/// Reads `buf.len()` bytes at the file offset; how many were read.
pub type ReadAt<'a> = &'a mut dyn FnMut(u64, &mut [u8]) -> Result<usize, u32>;

/// The most the headers can take (the file is read once for them).
const HEADER_READ: usize = 0x1000;
/// The most of a resource section that is read.
const MAX_SECTION: usize = 64 << 20;

fn read_exact(read: ReadAt<'_>, at: u64, len: usize) -> Result<Vec<u8>, u32> {
    let mut buf = vec![0u8; len];
    let mut done = 0;
    while done < len {
        let n = read(at + done as u64, &mut buf[done..])?;
        if n == 0 {
            break;
        }
        done += n;
    }
    buf.truncate(done);
    Ok(buf)
}

/// The bytes of the `RT_VERSION` resource (named 1, the first language) of
/// the PE file `read` reads. Errors are the Win32 ones the three pages
/// give: not a PE image `ERROR_BAD_EXE_FORMAT`, no resource section or no
/// version resource `ERROR_RESOURCE_DATA_NOT_FOUND`.
pub fn read_version_resource(read: ReadAt<'_>) -> Result<Vec<u8>, u32> {
    let mut head = read_exact(read, 0, HEADER_READ)?;
    if head.len() < 0x40 || &head[..2] != b"MZ" {
        return Err(ERROR_BAD_EXE_FORMAT);
    }
    let pe = match Pe::parse(&head) {
        Ok(p) => p,
        Err(poly_pe::Error::Truncated) => {
            // Headers longer than a page (many sections): read more.
            head = read_exact(read, 0, 0x10000)?;
            return finish(read, &head);
        }
        Err(_) => return Err(ERROR_BAD_EXE_FORMAT),
    };
    finish_with(read, &pe)
}

fn finish(read: ReadAt<'_>, head: &[u8]) -> Result<Vec<u8>, u32> {
    let pe = Pe::parse(head).map_err(|_| ERROR_BAD_EXE_FORMAT)?;
    finish_with(read, &pe)
}

fn finish_with(read: ReadAt<'_>, pe: &Pe<'_>) -> Result<Vec<u8>, u32> {
    let d = pe.info().directory(dir::RESOURCE).filter(|d| d.virtual_address != 0 && d.size != 0);
    let Some(d) = d else { return Err(ERROR_RESOURCE_DATA_NOT_FOUND) };
    let Some((_, section)) = pe.sections().find_rva(d.virtual_address) else {
        return Err(ERROR_RESOURCE_DATA_NOT_FOUND);
    };
    let len = (section.file_backed_size() as usize).min(MAX_SECTION);
    let sec = read_exact(read, u64::from(section.pointer_to_raw_data), len)?;
    let base = d.virtual_address - section.virtual_address;
    let tree = sec.get(base as usize..).ok_or(ERROR_BAD_EXE_FORMAT)?;
    let not_found = |_| ERROR_RESOURCE_DATA_NOT_FOUND;
    let root = ResourceDirectory::from_tree(tree).map_err(not_found)?;
    let by_name = root
        .find(ResourceKey::Id(resource_type::VERSION))
        .map_err(not_found)?
        .map(|e| e.directory())
        .transpose()
        .map_err(not_found)?
        .flatten()
        .ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
    let by_lang = by_name
        .find(ResourceKey::Id(1))
        .map_err(not_found)?
        .map(|e| e.directory())
        .transpose()
        .map_err(not_found)?
        .flatten()
        .ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
    let leaf = by_lang.entries().next().ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?.map_err(not_found)?;
    let data = leaf.data().map_err(not_found)?.ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
    let start = data.data_rva.checked_sub(section.virtual_address).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)? as usize;
    let bytes = sec.get(start..start + data.size as usize).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
    if bytes.len() < 6 {
        return Err(ERROR_RESOURCE_DATA_NOT_FOUND);
    }
    Ok(bytes.to_vec())
}

// ----- querying a version block ---------------------------------------------------------------------

/// One node of a `VS_VERSIONINFO` tree (Learn: `wLength`, `wValueLength`,
/// `wType`, `szKey`, padding, `Value`, padding, `Children`).
#[derive(Clone, Copy, Debug)]
struct Node {
    /// Where the node's key starts and ends (UTF-16, without the NUL).
    key: (usize, usize),
    /// The value: its offset, its length as the structure counts it (bytes
    /// for binary values, characters for text) and in bytes.
    value: usize,
    value_len: u32,
    value_bytes: usize,
    /// The children, and the end of the node.
    children: usize,
    end: usize,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// The node at `at` in `block`, within `limit`.
fn node(block: &[u8], at: usize, limit: usize) -> Option<Node> {
    let length = u16_at(block, at)? as usize;
    let value_len = u16_at(block, at + 2)?;
    let kind = u16_at(block, at + 4)?;
    let end = (at + length).min(limit).min(block.len());
    if length < 6 {
        return None;
    }
    let key_start = at + 6;
    let mut p = key_start;
    while u16_at(block, p)? != 0 {
        p += 2;
        if p >= end {
            return None;
        }
    }
    let key = (key_start, p);
    let value = align4(p + 2);
    // Text values count characters, binary ones bytes.
    let value_bytes = if kind == 1 { usize::from(value_len) * 2 } else { usize::from(value_len) };
    let children = align4(value.saturating_add(value_bytes));
    Some(Node { key, value, value_len: u32::from(value_len), value_bytes, children, end })
}

fn key_matches(block: &[u8], key: (usize, usize), want: &[u16]) -> bool {
    let n = (key.1 - key.0) / 2;
    n == want.len()
        && (0..n).all(|i| {
            let k = u16_at(block, key.0 + 2 * i).unwrap_or(0);
            fold(k) == fold(want[i])
        })
}

fn fold(c: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c }
}

/// `VerQueryValueW`: where `sub_block` is in `block`, and how long the
/// value is. Learn: `"\\"` is the `VS_FIXEDFILEINFO` (52 bytes);
/// `"\\StringFileInfo\\<language and code page>\\<name>"` a string (its
/// length in characters, with the NUL); `"\\VarFileInfo\\Translation"` an
/// array of language and code page pairs (its length in bytes). The result
/// is the offset of the value in `block` and its length as Learn gives it;
/// a name that is not there is `ERROR_RESOURCE_TYPE_NOT_FOUND`.
pub fn query_value(block: &[u8], sub_block: &[u16]) -> Result<(usize, u32), u32> {
    let not_found = ERROR_RESOURCE_TYPE_NOT_FOUND;
    let root = node(block, 0, block.len()).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
    let mut path = sub_block;
    if path.first() != Some(&u16::from(b'\\')) {
        return Err(not_found);
    }
    path = &path[1..];
    let mut current = root;
    while !path.is_empty() {
        let (comp, rest) = match path.iter().position(|&c| c == u16::from(b'\\')) {
            Some(i) => (&path[..i], &path[i + 1..]),
            None => (path, &path[path.len()..]),
        };
        let mut at = current.children;
        let mut found = None;
        while at < current.end {
            let Some(child) = node(block, at, current.end) else { break };
            if key_matches(block, child.key, comp) {
                found = Some(child);
                break;
            }
            at = align4(child.end);
        }
        current = found.ok_or(not_found)?;
        path = rest;
    }
    if current.value + current.value_bytes > block.len() {
        return Err(ERROR_RESOURCE_DATA_NOT_FOUND);
    }
    Ok((current.value, current.value_len))
}

// ----- building a version resource (ADR 0155, step 9) -----------------------------------------

/// The version Poly's system DLLs carry in their version resources: the Windows version Poly
/// reports to programs (`poly_registry::defaults`: Windows 10, build 19045, update 2006). Learn,
/// "Getting the System Version": "To obtain the full version number for the operating system,
/// call the GetFileVersionInfo function on one of the system DLLs, such as Kernel32.dll, then
/// call VerQueryValue to obtain the \\StringFileInfo\\<lang><codepage>\\ProductVersion
/// subblock". Chrome's browser does that with `kernelbase.dll` and `kernel32.dll` when it
/// starts and ends itself when neither has a version.
pub const SYSTEM_VERSION: [u16; 4] = [10, 0, 19045, 2006];

/// `VS_FFI_SIGNATURE`, `VS_FFI_STRUCVERSION`, `VS_FFI_FILEFLAGSMASK`, `VOS_NT_WINDOWS32`,
/// `VFT_DLL` (`verrsrc.h`).
const VS_FFI_SIGNATURE: u32 = 0xFEEF_04BD;
const VS_FFI_STRUCVERSION: u32 = 0x0001_0000;
const VS_FFI_FILEFLAGSMASK: u32 = 0x3F;
const VOS_NT_WINDOWS32: u32 = 0x0004_0004;
const VFT_DLL: u32 = 2;

/// One node of a version resource (Learn, "VS_VERSIONINFO", "StringFileInfo", "String"):
/// `wLength`, `wValueLength`, `wType` (1 for text), the key, padding to 32 bits, the value,
/// and the children, each aligned to 32 bits.
fn build_node(key: &str, text: bool, value_len: u16, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = alloc::vec![0u8; 6];
    for c in key.encode_utf16().chain([0]) {
        b.extend_from_slice(&c.to_le_bytes());
    }
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    b.extend_from_slice(value);
    for c in children {
        while !b.len().is_multiple_of(4) {
            b.push(0);
        }
        b.extend_from_slice(c);
    }
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b[2..4].copy_from_slice(&value_len.to_le_bytes());
    b[4..6].copy_from_slice(&u16::from(text).to_le_bytes());
    b
}

/// The `VS_VERSIONINFO` of a DLL: the fixed file information with `version` as the file and
/// the product version, and `strings` (name, value) for U.S. English in Unicode (`040904B0`),
/// with the translation that names that block.
pub fn build_version_info(version: [u16; 4], strings: &[(&str, &str)]) -> Vec<u8> {
    let ms = (u32::from(version[0]) << 16) | u32::from(version[1]);
    let ls = (u32::from(version[2]) << 16) | u32::from(version[3]);
    let mut fixed = Vec::with_capacity(52);
    for v in [
        VS_FFI_SIGNATURE,
        VS_FFI_STRUCVERSION,
        ms,
        ls,
        ms,
        ls,
        VS_FFI_FILEFLAGSMASK,
        0,
        VOS_NT_WINDOWS32,
        VFT_DLL,
        0,
        0,
        0,
    ] {
        fixed.extend_from_slice(&v.to_le_bytes());
    }
    let entries: Vec<Vec<u8>> = strings
        .iter()
        .map(|(key, value)| {
            let v: Vec<u8> = value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
            build_node(key, true, (v.len() / 2) as u16, &v, &[])
        })
        .collect();
    let table = build_node("040904B0", true, 0, &[], &entries);
    let string_info = build_node("StringFileInfo", true, 0, &[], &[table]);
    // The translation: language 0x0409, code page 1200 (0x04B0).
    let translation = build_node("Translation", false, 4, &[0x09, 0x04, 0xB0, 0x04], &[]);
    let var_info = build_node("VarFileInfo", true, 0, &[], &[translation]);
    build_node("VS_VERSION_INFO", false, 52, &fixed, &[string_info, var_info])
}

/// The version resource of one of Poly's system DLLs: [`SYSTEM_VERSION`], the strings Learn's
/// "Version Information" lists as required for a file (`CompanyName`, `FileDescription`,
/// `FileVersion`, `InternalName`, `OriginalFilename`, `ProductName`, `ProductVersion`).
pub fn system_dll_version_info(file: &str, description: &str) -> Vec<u8> {
    let v = SYSTEM_VERSION;
    let text = alloc::format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]);
    let internal = file.strip_suffix(".dll").unwrap_or(file);
    build_version_info(
        v,
        &[
            ("CompanyName", "Poly"),
            ("FileDescription", description),
            ("FileVersion", &text),
            ("InternalName", internal),
            ("OriginalFilename", file),
            ("ProductName", "Poly"),
            ("ProductVersion", &text),
        ],
    )
}

/// A resource file (`.res`, Learn: "Resource File Formats", `RESOURCEHEADER`) that holds
/// `version` as the resource `RT_VERSION` (16) named 1 for U.S. English: what the linker takes
/// to put a version resource into an image. A `.res` file starts with an empty resource; each
/// resource is its header (the data size, the header size, the type and the name as 0xFFFF and
/// an id, the data version, the memory flags, the language, the version, the characteristics)
/// and its data, padded to 32 bits.
pub fn res_file(version: &[u8]) -> Vec<u8> {
    fn header(data_size: u32, kind: u16, name: u16, flags: u16, language: u16) -> Vec<u8> {
        let mut h = Vec::with_capacity(32);
        h.extend_from_slice(&data_size.to_le_bytes());
        h.extend_from_slice(&32u32.to_le_bytes());
        for id in [kind, name] {
            h.extend_from_slice(&0xFFFFu16.to_le_bytes());
            h.extend_from_slice(&id.to_le_bytes());
        }
        h.extend_from_slice(&0u32.to_le_bytes());
        h.extend_from_slice(&flags.to_le_bytes());
        h.extend_from_slice(&language.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h
    }
    let mut out = header(0, 0, 0, 0, 0);
    // MOVEABLE | PURE (0x0030), the flags of a version resource; language 0x0409.
    out.extend(header(version.len() as u32, 16, 1, 0x0030, 0x0409));
    out.extend_from_slice(version);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;
    extern crate std;

    /// A node: `wLength`, `wValueLength`, `wType`, key, padding, value,
    /// padding, children.
    fn make_node(key: &str, kind: u16, value_len: u16, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
        let mut b = vec![0u8; 6];
        for c in key.encode_utf16().chain([0]) {
            b.extend_from_slice(&c.to_le_bytes());
        }
        while !b.len().is_multiple_of(4) {
            b.push(0);
        }
        b.extend_from_slice(value);
        for c in children {
            while !b.len().is_multiple_of(4) {
                b.push(0);
            }
            b.extend_from_slice(c);
        }
        let len = b.len() as u16;
        b[0..2].copy_from_slice(&len.to_le_bytes());
        b[2..4].copy_from_slice(&value_len.to_le_bytes());
        b[4..6].copy_from_slice(&kind.to_le_bytes());
        b
    }

    fn string(key: &str, value: &str) -> Vec<u8> {
        let v: Vec<u8> = value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        make_node(key, 1, (v.len() / 2) as u16, &v, &[])
    }

    fn fixed_file_info(major: u16, minor: u16) -> Vec<u8> {
        let mut f = vec![0u8; 52];
        f[0..4].copy_from_slice(&0xFEEF_04BDu32.to_le_bytes());
        f[4..8].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        f[8..12].copy_from_slice(&((u32::from(major) << 16) | u32::from(minor)).to_le_bytes());
        f
    }

    /// A version resource for 157.0 with a few strings and one translation.
    pub fn sample() -> Vec<u8> {
        let strings = make_node(
            "040904b0",
            0,
            0,
            &[],
            &[string("CompanyName", "Poly"), string("FileVersion", "157.0.8103.38"), string("ProductName", "Chrome")],
        );
        let sfi = make_node("StringFileInfo", 0, 0, &[], &[strings]);
        let translation = make_node("Translation", 0, 4, &[0x09, 0x04, 0xB0, 0x04], &[]);
        let vfi = make_node("VarFileInfo", 0, 0, &[], &[translation]);
        make_node("VS_VERSION_INFO", 0, 52, &fixed_file_info(157, 0), &[sfi, vfi])
    }

    fn utf16(block: &[u8], at: usize, n: usize) -> std::string::String {
        let units: Vec<u16> = (0..n).map(|i| u16::from_le_bytes([block[at + 2 * i], block[at + 2 * i + 1]])).collect();
        std::string::String::from_utf16_lossy(&units)
    }

    #[test]
    fn queries() {
        let block = sample();
        // The fixed file information.
        let (at, len) = query_value(&block, &wide("\\")).unwrap();
        assert_eq!(len, 52);
        assert_eq!(u32::from_le_bytes(block[at..at + 4].try_into().unwrap()), 0xFEEF_04BD);
        // A string: the length counts the NUL.
        let (at, len) = query_value(&block, &wide("\\StringFileInfo\\040904b0\\FileVersion")).unwrap();
        assert_eq!(len, 14);
        assert_eq!(utf16(&block, at, 13), "157.0.8103.38");
        assert_eq!(block[at + 26], 0);
        // Case does not matter; the last string is found too.
        let (at, len) = query_value(&block, &wide("\\stringfileinfo\\040904B0\\PRODUCTNAME")).unwrap();
        assert_eq!((len, utf16(&block, at, 6)), (7, "Chrome".into()));
        // The translation: bytes, not characters.
        let (at, len) = query_value(&block, &wide("\\VarFileInfo\\Translation")).unwrap();
        assert_eq!(len, 4);
        assert_eq!(&block[at..at + 4], &[0x09, 0x04, 0xB0, 0x04]);
        // Things that are not there.
        for p in [
            "\\StringFileInfo\\040904b0\\Nope",
            "\\StringFileInfo\\0407Aaaa\\FileVersion",
            "\\Nope",
            "",
            "StringFileInfo",
        ] {
            assert_eq!(query_value(&block, &wide(p)), Err(ERROR_RESOURCE_TYPE_NOT_FOUND), "{p:?}");
        }
        assert_eq!(query_value(&[1, 2, 3], &wide("\\")), Err(ERROR_RESOURCE_DATA_NOT_FOUND));
    }

    /// The version resource Poly's system DLLs carry answers what Learn's "Getting the System
    /// Version" asks of it, and its `.res` file is two resource headers and the data.
    #[test]
    fn a_built_system_version_is_read_back() {
        let block = system_dll_version_info("kernel32.dll", "Poly kernel32");
        let (at, len) = query_value(&block, &wide("\\")).unwrap();
        assert_eq!(len, 52);
        let q = |o: usize| u32::from_le_bytes(block[at + o..at + o + 4].try_into().unwrap());
        assert_eq!(q(0), 0xFEEF_04BD);
        // dwFileVersionMS/LS and dwProductVersionMS/LS: 10.0.19045.2006.
        assert_eq!((q(8), q(12), q(16), q(20)), (0x000A_0000, 0x4A65_07D6, 0x000A_0000, 0x4A65_07D6));
        assert_eq!((q(32), q(36)), (0x0004_0004, 2));
        let (at, len) = query_value(&block, &wide("\\VarFileInfo\\Translation")).unwrap();
        assert_eq!((len, &block[at..at + 4]), (4, &[0x09, 0x04, 0xB0, 0x04][..]));
        for (name, want) in [
            ("ProductVersion", "10.0.19045.2006"),
            ("FileVersion", "10.0.19045.2006"),
            ("OriginalFilename", "kernel32.dll"),
            ("InternalName", "kernel32"),
            ("CompanyName", "Poly"),
        ] {
            let path = std::format!("\\StringFileInfo\\040904B0\\{name}");
            let (at, len) = query_value(&block, &wide(&path)).unwrap();
            assert_eq!(len as usize, want.len() + 1, "{name}");
            assert_eq!(utf16(&block, at, want.len()), want, "{name}");
        }
        // The .res file: an empty resource, then RT_VERSION / 1 / 0x409 with the block.
        let res = res_file(&block);
        assert_eq!(&res[..8], &[0, 0, 0, 0, 32, 0, 0, 0]);
        assert_eq!(u32::from_le_bytes(res[32..36].try_into().unwrap()) as usize, block.len());
        assert_eq!(&res[40..48], &[0xFF, 0xFF, 16, 0, 0xFF, 0xFF, 1, 0]);
        assert_eq!(u16::from_le_bytes([res[54], res[55]]), 0x0409);
        assert_eq!(&res[64..64 + block.len()], &block[..]);
        assert_eq!(res.len() % 4, 0);
        // And the resource is found in a file that carries it.
        let pe = pe_with_version(&block);
        let mut read = |at: u64, buf: &mut [u8]| -> Result<usize, u32> {
            let at = at as usize;
            let n = buf.len().min(pe.len().saturating_sub(at));
            buf[..n].copy_from_slice(&pe[at..at + n]);
            Ok(n)
        };
        assert_eq!(read_version_resource(&mut read), Ok(block));
    }

    /// A PE32+ file with only a `.rsrc` section holding `version` as the
    /// resource `RT_VERSION` / 1 / 0x409, at file offset 0x400 and RVA 0x3000.
    fn pe_with_version(version: &[u8]) -> Vec<u8> {
        let rsrc_rva = 0x3000u32;
        // Tree: root (type 16) at 0, name table at 0x18, language table at
        // 0x30, the data entry at 0x48, the data at 0x58.
        let mut tree = vec![0u8; 0x58];
        let dir = |tree: &mut Vec<u8>, at: usize, id: u32, target: u32| {
            tree[at + 14..at + 16].copy_from_slice(&1u16.to_le_bytes());
            tree[at + 16..at + 20].copy_from_slice(&id.to_le_bytes());
            tree[at + 20..at + 24].copy_from_slice(&target.to_le_bytes());
        };
        dir(&mut tree, 0, 16, 0x8000_0018);
        dir(&mut tree, 0x18, 1, 0x8000_0030);
        dir(&mut tree, 0x30, 0x409, 0x48);
        tree[0x48..0x4C].copy_from_slice(&(rsrc_rva + 0x58).to_le_bytes());
        tree[0x4C..0x50].copy_from_slice(&(version.len() as u32).to_le_bytes());
        tree.extend_from_slice(version);
        let raw = tree.len().next_multiple_of(0x200);
        let mut f = vec![0u8; 0x400 + raw];
        f[0..2].copy_from_slice(b"MZ");
        f[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        f[0x80..0x84].copy_from_slice(b"PE\0\0");
        f[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        f[0x86..0x88].copy_from_slice(&1u16.to_le_bytes()); // one section
        f[0x94..0x96].copy_from_slice(&240u16.to_le_bytes()); // SizeOfOptionalHeader
        f[0x96..0x98].copy_from_slice(&0x2022u16.to_le_bytes());
        let opt = 0x98;
        f[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        f[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes()); // SectionAlignment
        f[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
        f[opt + 56..opt + 60].copy_from_slice(&0x4000u32.to_le_bytes()); // SizeOfImage
        f[opt + 60..opt + 64].copy_from_slice(&0x400u32.to_le_bytes()); // SizeOfHeaders
        f[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        let d = opt + 112 + 2 * 8; // data directory 2
        f[d..d + 4].copy_from_slice(&rsrc_rva.to_le_bytes());
        f[d + 4..d + 8].copy_from_slice(&(tree.len() as u32).to_le_bytes());
        let s = opt + 240;
        f[s..s + 5].copy_from_slice(b".rsrc");
        f[s + 8..s + 12].copy_from_slice(&(tree.len() as u32).to_le_bytes());
        f[s + 12..s + 16].copy_from_slice(&rsrc_rva.to_le_bytes());
        f[s + 16..s + 20].copy_from_slice(&(raw as u32).to_le_bytes());
        f[s + 20..s + 24].copy_from_slice(&0x400u32.to_le_bytes());
        f[s + 36..s + 40].copy_from_slice(&0x4000_0040u32.to_le_bytes());
        f[0x400..0x400 + tree.len()].copy_from_slice(&tree);
        f
    }

    #[test]
    fn reads_the_resource_of_a_file() {
        let version = sample();
        let file = pe_with_version(&version);
        let mut reads: Vec<(u64, usize)> = Vec::new();
        let mut read = |at: u64, buf: &mut [u8]| -> Result<usize, u32> {
            reads.push((at, buf.len()));
            let at = at as usize;
            let n = buf.len().min(file.len().saturating_sub(at));
            buf[..n].copy_from_slice(&file[at..at + n]);
            Ok(n)
        };
        assert_eq!(read_version_resource(&mut read).unwrap(), version);
        // The headers and the resource section only (a read that returns
        // less than asked is asked again, once, for the rest).
        assert_eq!(reads[0], (0, HEADER_READ));
        assert!(reads.iter().all(|&(at, len)| (at == 0 || at >= 0x400) && len <= 0x1000), "{reads:?}");
        assert!(reads.iter().any(|&(at, _)| at == 0x400));
        // Not a PE file; a PE file without resources; a read error.
        let mut junk = |_: u64, buf: &mut [u8]| -> Result<usize, u32> {
            buf.fill(b'x');
            Ok(buf.len())
        };
        assert_eq!(read_version_resource(&mut junk), Err(ERROR_BAD_EXE_FORMAT));
        let mut none = file.clone();
        none[0x98 + 112 + 16..0x98 + 112 + 24].fill(0);
        let mut read2 = |at: u64, buf: &mut [u8]| -> Result<usize, u32> {
            let at = at as usize;
            let n = buf.len().min(none.len().saturating_sub(at));
            buf[..n].copy_from_slice(&none[at..at + n]);
            Ok(n)
        };
        assert_eq!(read_version_resource(&mut read2), Err(ERROR_RESOURCE_DATA_NOT_FOUND));
        let mut broken = |_: u64, _: &mut [u8]| -> Result<usize, u32> { Err(ERROR_ACCESS_DENIED) };
        assert_eq!(read_version_resource(&mut broken), Err(ERROR_ACCESS_DENIED));
    }

    /// The version information of the files of the Chrome package, when it is
    /// cached (`cargo xtask chrome-win fetch`): the root block (`"\\"`),
    /// the translation table and the string table all resolve.
    #[test]
    fn chromes_own_files_answer_the_queries_chrome_makes() {
        // `POLY_CHROME_DIR`: the unpacked package (the cache of `cargo xtask chrome-win fetch`).
        let Ok(dir) = std::env::var("POLY_CHROME_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        {
            for name in ["chrome.exe", "chrome_elf.dll"] {
                let Ok(file) = std::fs::read(dir.join(name)) else { continue };
                let mut read = |at: u64, buf: &mut [u8]| -> Result<usize, u32> {
                    let at = at as usize;
                    let n = buf.len().min(file.len().saturating_sub(at));
                    buf[..n].copy_from_slice(&file[at..at + n]);
                    Ok(n)
                };
                let block = read_version_resource(&mut read).unwrap_or_else(|e| panic!("{name}: error {e}"));
                let w = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
                let (at, n) = query_value(&block, &w("\\")).unwrap_or_else(|e| panic!("{name}: root query error {e}"));
                assert_eq!(n, 52, "{name}");
                assert_eq!(&block[at..at + 4], &0xFEEF04BDu32.to_le_bytes(), "{name}: VS_FIXEDFILEINFO signature");
                assert!(query_value(&block, &w("\\VarFileInfo\\Translation")).is_ok(), "{name}");
            }
        }
    }
}
