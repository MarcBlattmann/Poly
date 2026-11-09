//! The instrumented program: renaming a PE's imports.

use super::*;

// ----- the instrumented program -----------------------------------------------------------

fn rd16(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "truncated PE headers".to_string())
}
fn rd32(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "truncated PE headers".to_string())
}
fn wr32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn align_up(v: u32, a: u32) -> u32 {
    v.div_ceil(a.max(1)) * a.max(1)
}

/// The PE checksum (`CheckSumMappedFile`'s algorithm as the PE format
/// describes the field: the 16-bit one's-complement-style sum of the file
/// with the checksum field taken as zero, plus the file's length).
pub fn pe_checksum(b: &[u8], checksum_at: usize) -> u32 {
    let mut sum: u64 = 0;
    for (i, chunk) in b.chunks(2).enumerate() {
        let at = i * 2;
        if at >= checksum_at && at < checksum_at + 4 {
            continue;
        }
        let w = u16::from_le_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]) as u64;
        sum += w;
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    sum = (sum & 0xFFFF) + (sum >> 16);
    (sum as u32).wrapping_add(b.len() as u32)
}

/// A copy of a PE image whose import (and delay-load import) directory
/// names `rename(dll)` instead of each DLL it returns a name for. The new
/// names go into an appended section `.ptname`; the descriptors' name
/// fields point there, bound imports are dropped (the directory and the
/// descriptors' time stamps), as is an Authenticode signature (no longer
/// valid); the checksum is recomputed. Nothing else changes.
pub fn rename_imports(bytes: &[u8], rename: impl Fn(&str) -> Option<String>) -> Result<Vec<u8>> {
    let pe = poly_pe::Pe::parse(bytes).map_err(|e| format!("not a PE image: {e:?}"))?;
    if pe.info().bits() != poly_pe::Bits::Pe32Plus {
        return Err("only PE32+ (x64) images are supported".into());
    }
    let mut b = bytes.to_vec();
    let e = rd32(&b, 0x3C)? as usize;
    if b.get(e..e + 4) != Some(b"PE\0\0") {
        return Err("no PE signature".into());
    }
    let fh = e + 4;
    let nsect = rd16(&b, fh + 2)? as usize;
    let opt = fh + 20;
    let opt_size = rd16(&b, fh + 16)? as usize;
    let sect_align = rd32(&b, opt + 32)?;
    let file_align = rd32(&b, opt + 36)?;
    let size_of_headers = rd32(&b, opt + 60)? as usize;
    let checksum_at = opt + 64;
    let ndirs = rd32(&b, opt + 108)? as usize;
    let dir_at = |i: usize| opt + 112 + 8 * i;
    let table = opt + opt_size;
    let sections: Vec<poly_pe::SectionHeader> = pe.sections().iter().collect();
    // Room for one more section header in the headers.
    let new_hdr = table + 40 * nsect;
    let first_raw = sections.iter().map(|s| s.pointer_to_raw_data).filter(|&p| p != 0).min().unwrap_or(0) as usize;
    if new_hdr + 40 > size_of_headers
        || (first_raw != 0 && new_hdr + 40 > first_raw)
        || b.get(new_hdr..new_hdr + 40).is_none_or(|h| h.iter().any(|&x| x != 0))
    {
        return Err("no room for another section header".into());
    }
    let to_off = |rva: u32| -> Result<usize> {
        sections
            .iter()
            .find(|s| rva >= s.virtual_address && rva < s.virtual_address + s.virtual_size.max(s.size_of_raw_data))
            .map(|s| (rva - s.virtual_address + s.pointer_to_raw_data) as usize)
            .ok_or_else(|| format!("RVA {rva:#x} is in no section"))
    };
    // The new names, and where each descriptor's name field is.
    let mut names = Vec::new();
    let mut fixups: Vec<(usize, u32)> = Vec::new();
    let mut stamps: Vec<usize> = Vec::new();
    let mut add = |dll: &[u8], field: usize, names: &mut Vec<u8>| -> bool {
        match rename(&String::from_utf8_lossy(dll)) {
            Some(new) => {
                fixups.push((field, names.len() as u32));
                names.extend_from_slice(new.as_bytes());
                names.push(0);
                if names.len() % 2 == 1 {
                    names.push(0);
                }
                true
            }
            None => false,
        }
    };
    let image_base = pe.info().optional.image_base;
    if ndirs > 1 && rd32(&b, dir_at(1) + 4)? != 0 {
        let mut d = to_off(rd32(&b, dir_at(1))?)?;
        loop {
            let desc = b.get(d..d + 20).ok_or("import directory runs off the file")?;
            if desc.iter().all(|&x| x == 0) {
                break;
            }
            let name = to_off(rd32(&b, d + 12)?)?;
            let len = b[name..].iter().position(|&c| c == 0).ok_or("unterminated DLL name")?;
            if add(&b[name..name + len], d + 12, &mut names) && rd32(&b, d)? != 0 {
                stamps.push(d + 4);
            }
            d += 20;
        }
    }
    if ndirs > 13 && rd32(&b, dir_at(13) + 4)? != 0 {
        let mut d = to_off(rd32(&b, dir_at(13))?)?;
        loop {
            let desc = b.get(d..d + 32).ok_or("delay-load directory runs off the file")?;
            if desc.iter().all(|&x| x == 0) {
                break;
            }
            let attrs = rd32(&b, d)?;
            let raw = rd32(&b, d + 4)?;
            // Attribute bit 0: the fields are RVAs; old-style ones are VAs.
            let rva = if attrs & 1 != 0 { raw } else { (raw as u64).wrapping_sub(image_base) as u32 };
            if attrs & 1 == 0 {
                return Err("old-style (VA) delay-load descriptors are not supported".into());
            }
            let name = to_off(rva)?;
            let len = b[name..].iter().position(|&c| c == 0).ok_or("unterminated DLL name")?;
            add(&b[name..name + len], d + 4, &mut names);
            d += 32;
        }
    }
    if fixups.is_empty() {
        return Err("the image imports none of the DLLs to rename".into());
    }
    // Drop an Authenticode signature (it sits at the end of the file).
    if ndirs > 4 {
        let (off, size) = (rd32(&b, dir_at(4))? as usize, rd32(&b, dir_at(4) + 4)? as usize);
        if size != 0 {
            if off + size >= b.len() && off <= b.len() {
                b.truncate(off);
            }
            wr32(&mut b, dir_at(4), 0);
            wr32(&mut b, dir_at(4) + 4, 0);
        }
    }
    if ndirs > 11 {
        wr32(&mut b, dir_at(11), 0);
        wr32(&mut b, dir_at(11) + 4, 0);
    }
    for at in stamps {
        // A bound descriptor (time stamp set, lookup table present): unbind.
        wr32(&mut b, at, 0);
        wr32(&mut b, at + 4, 0);
    }
    // The new section: after the last one in memory, at the end of the file.
    let va = sections
        .iter()
        .map(|s| align_up(s.virtual_address + s.virtual_size.max(s.size_of_raw_data), sect_align))
        .max()
        .unwrap_or(sect_align);
    let raw = align_up(b.len() as u32, file_align);
    let raw_size = align_up(names.len() as u32, file_align);
    b.resize(raw as usize, 0);
    b.extend_from_slice(&names);
    b.resize((raw + raw_size) as usize, 0);
    let mut h = [0u8; 40];
    h[..8].copy_from_slice(b".ptname\0");
    h[8..12].copy_from_slice(&(names.len() as u32).to_le_bytes());
    h[12..16].copy_from_slice(&va.to_le_bytes());
    h[16..20].copy_from_slice(&raw_size.to_le_bytes());
    h[20..24].copy_from_slice(&raw.to_le_bytes());
    // IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ (winnt.h)
    h[36..40].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    b[new_hdr..new_hdr + 40].copy_from_slice(&h);
    b[fh + 2..fh + 4].copy_from_slice(&((nsect + 1) as u16).to_le_bytes());
    wr32(&mut b, opt + 56, align_up(va + names.len() as u32, sect_align));
    let init = rd32(&b, opt + 8)?;
    wr32(&mut b, opt + 8, init.wrapping_add(raw_size));
    for (field, at) in fixups {
        wr32(&mut b, field, va + at);
    }
    wr32(&mut b, checksum_at, 0);
    let sum = pe_checksum(&b, checksum_at);
    wr32(&mut b, checksum_at, sum);
    Ok(b)
}
