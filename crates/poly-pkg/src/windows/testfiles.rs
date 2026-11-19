//! Test files written from the public formats: a PE image with a resource
//! section (the PE format specification: headers, one `.rsrc` section, the
//! resource directory tables, data entries), a version resource (Learn,
//! "VS_VERSIONINFO") and icon groups.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// One resource: type id, name id, language, bytes.
pub type Res<'a> = (u32, u32, u32, &'a [u8]);

const SECTION_RVA: u32 = 0x1000;
const FILE_ALIGN: usize = 0x200;

fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// The resource section's bytes for `leaves`, laid out at `rva`.
pub fn resource_tree(rva: u32, leaves: &[Res<'_>]) -> Vec<u8> {
    // type -> name -> lang -> data
    let mut tree: BTreeMap<u32, BTreeMap<u32, BTreeMap<u32, &[u8]>>> = BTreeMap::new();
    for &(t, n, l, d) in leaves {
        tree.entry(t).or_default().entry(n).or_default().insert(l, d);
    }
    let dir_size = |entries: usize| 16 + 8 * entries;
    // Offsets: the root, then each type's directory, then each name's,
    // then the data entries, then the data.
    let mut offset = dir_size(tree.len());
    let mut type_at = BTreeMap::new();
    for (t, names) in &tree {
        type_at.insert(*t, offset);
        offset += dir_size(names.len());
    }
    let mut name_at = BTreeMap::new();
    for (t, names) in &tree {
        for (n, langs) in names {
            name_at.insert((*t, *n), offset);
            offset += dir_size(langs.len());
        }
    }
    let mut entry_at = BTreeMap::new();
    for (t, names) in &tree {
        for (n, langs) in names {
            for l in langs.keys() {
                entry_at.insert((*t, *n, *l), offset);
                offset += 16;
            }
        }
    }
    let mut data_at = BTreeMap::new();
    for (t, names) in &tree {
        for (n, langs) in names {
            for (l, d) in langs {
                data_at.insert((*t, *n, *l), offset);
                offset = (offset + d.len() + 3) & !3;
            }
        }
    }
    let mut out = Vec::with_capacity(offset);
    let dir = |out: &mut Vec<u8>, ids: &[(u32, u32)]| {
        put32(out, 0);
        put32(out, 0);
        put16(out, 0);
        put16(out, 0);
        put16(out, 0);
        put16(out, ids.len() as u16);
        for &(id, target) in ids {
            put32(out, id);
            put32(out, target);
        }
    };
    let high = 0x8000_0000u32;
    let ids: Vec<(u32, u32)> = tree.keys().map(|t| (*t, type_at[t] as u32 | high)).collect();
    dir(&mut out, &ids);
    for (t, names) in &tree {
        let ids: Vec<(u32, u32)> = names.keys().map(|n| (*n, name_at[&(*t, *n)] as u32 | high)).collect();
        dir(&mut out, &ids);
    }
    for (t, names) in &tree {
        for (n, langs) in names {
            let ids: Vec<(u32, u32)> = langs.keys().map(|l| (*l, entry_at[&(*t, *n, *l)] as u32)).collect();
            dir(&mut out, &ids);
        }
    }
    for (t, names) in &tree {
        for (n, langs) in names {
            for (l, d) in langs {
                put32(&mut out, rva + data_at[&(*t, *n, *l)] as u32);
                put32(&mut out, d.len() as u32);
                put32(&mut out, 0);
                put32(&mut out, 0);
            }
        }
    }
    for names in tree.values() {
        for langs in names.values() {
            for d in langs.values() {
                out.extend_from_slice(d);
                while !out.len().is_multiple_of(4) {
                    out.push(0);
                }
            }
        }
    }
    out
}

/// A PE32+ image (or PE32 with `bits` 32) with `leaves` as its resources;
/// a DLL with `dll`; GUI or console; with a certificate table of `cert`
/// bytes after the section if not empty.
pub fn pe(bits: u8, dll: bool, gui: bool, leaves: &[Res<'_>], cert: &[u8]) -> Vec<u8> {
    let rsrc = resource_tree(SECTION_RVA, leaves);
    let raw_size = rsrc.len().div_ceil(FILE_ALIGN) * FILE_ALIGN;
    let mut b = alloc::vec![0u8; 0x40];
    b[0..2].copy_from_slice(b"MZ");
    b[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    b.extend_from_slice(b"PE\0\0");
    let opt_size: u16 = if bits == 64 { 240 } else { 224 };
    put16(&mut b, if bits == 64 { 0x8664 } else { 0x14c });
    put16(&mut b, 1);
    put32(&mut b, 0);
    put32(&mut b, 0);
    put32(&mut b, 0);
    put16(&mut b, opt_size);
    put16(&mut b, 0x0022 | if dll { 0x2000 } else { 0 } | if bits == 32 { 0x0100 } else { 0 });
    let opt_start = b.len();
    put16(&mut b, if bits == 64 { 0x20B } else { 0x10B });
    b.extend_from_slice(&[14, 0]);
    put32(&mut b, 0);
    put32(&mut b, raw_size as u32);
    put32(&mut b, 0);
    put32(&mut b, 0);
    put32(&mut b, 0);
    if bits == 64 {
        b.extend_from_slice(&0x1_4000_0000u64.to_le_bytes());
    } else {
        put32(&mut b, 0);
        put32(&mut b, 0x40_0000);
    }
    put32(&mut b, 0x1000);
    put32(&mut b, FILE_ALIGN as u32);
    for v in [6u16, 0, 0, 0, 6, 0] {
        put16(&mut b, v);
    }
    put32(&mut b, 0);
    let image_size = SECTION_RVA as usize + rsrc.len().div_ceil(0x1000).max(1) * 0x1000;
    put32(&mut b, image_size as u32);
    put32(&mut b, FILE_ALIGN as u32);
    put32(&mut b, 0);
    put16(&mut b, if gui { 2 } else { 3 });
    put16(&mut b, 0x8160);
    for _ in 0..4 {
        if bits == 64 {
            b.extend_from_slice(&0x10_0000u64.to_le_bytes());
        } else {
            put32(&mut b, 0x10_0000);
        }
    }
    put32(&mut b, 0);
    put32(&mut b, 16);
    let cert_at = FILE_ALIGN + raw_size;
    for i in 0..16 {
        match i {
            2 => {
                put32(&mut b, SECTION_RVA);
                put32(&mut b, rsrc.len() as u32);
            }
            4 if !cert.is_empty() => {
                put32(&mut b, cert_at as u32);
                put32(&mut b, cert.len() as u32);
            }
            _ => {
                put32(&mut b, 0);
                put32(&mut b, 0);
            }
        }
    }
    assert_eq!(b.len() - opt_start, usize::from(opt_size));
    b.extend_from_slice(b".rsrc\0\0\0");
    put32(&mut b, rsrc.len() as u32);
    put32(&mut b, SECTION_RVA);
    put32(&mut b, raw_size as u32);
    put32(&mut b, FILE_ALIGN as u32);
    put32(&mut b, 0);
    put32(&mut b, 0);
    put16(&mut b, 0);
    put16(&mut b, 0);
    put32(&mut b, 0x4000_0040);
    b.resize(FILE_ALIGN, 0);
    b.extend_from_slice(&rsrc);
    b.resize(FILE_ALIGN + raw_size, 0);
    b.extend_from_slice(cert);
    b
}

fn version_node(key: &str, text: bool, value_len: u16, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
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

/// A `VS_VERSIONINFO` with `version` as file and product version and
/// `strings` in table `table` (e.g. `040904b0`).
pub fn version_info(version: [u16; 4], table: &str, strings: &[(&str, &str)]) -> Vec<u8> {
    let ms = (u32::from(version[0]) << 16) | u32::from(version[1]);
    let ls = (u32::from(version[2]) << 16) | u32::from(version[3]);
    let mut fixed = Vec::new();
    for v in [0xFEEF_04BDu32, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0] {
        put32(&mut fixed, v);
    }
    let entries: Vec<Vec<u8>> = strings
        .iter()
        .map(|(k, v)| {
            let value: Vec<u8> = v.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
            version_node(k, true, (value.len() / 2) as u16, &value, &[])
        })
        .collect();
    let table = version_node(table, true, 0, &[], &entries);
    let sfi = version_node("StringFileInfo", true, 0, &[], &[table]);
    let translation = version_node("Translation", false, 4, &[0x09, 0x04, 0xB0, 0x04], &[]);
    let vfi = version_node("VarFileInfo", true, 0, &[], &[translation]);
    version_node("VS_VERSION_INFO", false, 52, &fixed, &[sfi, vfi])
}

/// A `GRPICONDIR` naming images (width, bits, size in bytes, `RT_ICON` id).
pub fn icon_group(images: &[(u8, u16, u32, u16)]) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 0);
    put16(&mut b, 1);
    put16(&mut b, images.len() as u16);
    for &(w, bits, size, id) in images {
        b.extend_from_slice(&[w, w, 0, 0]);
        put16(&mut b, 1);
        put16(&mut b, bits);
        put32(&mut b, size);
        put16(&mut b, id);
    }
    b
}

/// A 32-bit DIB icon image `side` pixels square filled with `bgra`.
pub fn dib32(side: u32, bgra: [u8; 4]) -> Vec<u8> {
    let mut b = Vec::new();
    put32(&mut b, 40);
    put32(&mut b, side);
    put32(&mut b, side * 2);
    put16(&mut b, 1);
    put16(&mut b, 32);
    for _ in 0..6 {
        put32(&mut b, 0);
    }
    for _ in 0..side * side {
        b.extend_from_slice(&bgra);
    }
    // The AND mask: rows of 32-bit words.
    b.extend(core::iter::repeat_n(0u8, (side.div_ceil(32) * 4 * side) as usize));
    b
}

/// A 4-bit DIB icon `side` square with a 16-colour palette: every pixel
/// colour `index`, the left half transparent by the mask.
pub fn dib4(side: u32, palette: &[[u8; 4]; 16], index: u8) -> Vec<u8> {
    let mut b = Vec::new();
    put32(&mut b, 40);
    put32(&mut b, side);
    put32(&mut b, side * 2);
    put16(&mut b, 1);
    put16(&mut b, 4);
    for _ in 0..6 {
        put32(&mut b, 0);
    }
    for c in palette {
        b.extend_from_slice(c);
    }
    let stride = (side * 4).div_ceil(32) * 4;
    for _ in 0..side {
        let mut row = alloc::vec![(index << 4) | index; (side as usize).div_ceil(2)];
        row.resize(stride as usize, 0);
        b.extend_from_slice(&row);
    }
    let mstride = side.div_ceil(32) * 4;
    for _ in 0..side {
        let mut row = alloc::vec![0u8; mstride as usize];
        for x in 0..side / 2 {
            row[(x / 8) as usize] |= 0x80 >> (x % 8);
        }
        b.extend_from_slice(&row);
    }
    b
}

/// A tiny PNG file's bytes (its signature and an IHDR are enough for the
/// reader, which does not decode it).
pub fn fake_png(tag: u8) -> Vec<u8> {
    let mut b = crate::windows::program::PNG_SIGNATURE.to_vec();
    b.extend_from_slice(&[0, 0, 0, 13]);
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&[0, 0, 1, 0, 0, 0, 1, 0, 8, 6, 0, 0, 0, tag, tag, tag, tag]);
    b
}
