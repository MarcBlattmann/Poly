//! Clean-room Microsoft Cabinet (`[MS-CAB]`) extractor.
//!
//! Written strictly from Microsoft's public `[MS-CAB]` specification. Supports:
//! - `tcompTYPE_NONE` (0): uncompressed blocks;
//! - `tcompTYPE_MSZIP` (1): RFC 1951 DEFLATE blocks prefixed with `CK`
//!   (`0x43, 0x4B`), maintaining the 32 KiB sliding history window across
//!   blocks within a folder via [`poly_png::inflate::inflate_with_history`].
//!
//! Other compression types (Quantum, LZX) return [`Error::Compression`].

use crate::Error;
use alloc::string::String;
use alloc::vec::Vec;

/// `[MS-CAB]` signature: `"MSCF"`.
pub const CAB_SIGNATURE: &[u8; 4] = b"MSCF";

/// `CFHEADER.flags` bits (`[MS-CAB]` 2.1.1).
const FLAG_PREV_CABINET: u16 = 0x0001;
const FLAG_NEXT_CABINET: u16 = 0x0002;
const FLAG_RESERVE_PRESENT: u16 = 0x0004;

/// `CFFOLDER.typeCompress` low-nibble method values (`[MS-CAB]` 2.1.2).
pub const COMP_NONE: u16 = 0x0000;
pub const COMP_MSZIP: u16 = 0x0001;
pub const COMP_QUANTUM: u16 = 0x0002;
pub const COMP_LZX: u16 = 0x0003;

/// `CFFILE.attribs` bit indicating UTF-8 `szName` (`[MS-CAB]` 2.1.3).
const ATTR_NAME_IS_UTF: u16 = 0x0080;

/// Special `CFFILE.iFolder` continuation indices (`[MS-CAB]` 2.1.3).
const IFOLDER_CONTINUED_FROM_PREV: u16 = 0xFFFD;

/// Safety limits for untrusted cabinet archives.
const MAX_FOLDERS: usize = 4096;
const MAX_FILES: usize = 65_536;
const MAX_NAME_LEN: usize = 1024;
const MAX_UNCOMP_BLOCK: usize = 32_768;
const MAX_FOLDER_UNCOMP: usize = 256 << 20;

/// A `CFFOLDER` entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Folder {
    pub data_offset: u32,
    pub block_count: u16,
    pub compress_type: u16,
}

/// A `CFFILE` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CabFile {
    pub name: String,
    pub size: u32,
    pub folder_offset: u32,
    pub folder_index: u16,
    pub attribs: u16,
}

/// Parsed `[MS-CAB]` header, folders, and file directory.
#[derive(Clone, Debug)]
pub struct Cabinet<'a> {
    data: &'a [u8],
    pub folders: Vec<Folder>,
    pub files: Vec<CabFile>,
    cb_cfdata_reserve: usize,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Compute the `[MS-CAB]` `CSUMCompute` checksum over `bytes` with initial
/// `seed` (`[MS-CAB]` 2.1.4): XOR of 32-bit little-endian words, with any
/// trailing 1..3 bytes packed big-endian into the final word.
pub fn csum_compute(bytes: &[u8], mut csum: u32) -> u32 {
    let mut chunks = bytes.chunks_exact(4);
    for c in &mut chunks {
        csum ^= u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut tail = 0u32;
        for &b in rem {
            tail = (tail << 8) | u32::from(b);
        }
        csum ^= tail;
    }
    csum
}

/// Decode a Windows-1252 byte string into a Rust `String`.
pub(crate) fn decode_cp1252(bytes: &[u8]) -> String {
    const TABLE: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{02C6}',
        '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}', '\u{0090}', '\u{2018}',
        '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}',
        '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            0x00..=0x7F => out.push(char::from(b)),
            0x80..=0x9F => out.push(TABLE[usize::from(b - 0x80)]),
            0xA0..=0xFF => out.push(char::from(b)),
        }
    }
    out
}

fn read_cstr<'a>(data: &'a [u8], pos: &mut usize) -> Result<&'a [u8], Error> {
    let start = *pos;
    let limit = data.len().min(start.saturating_add(MAX_NAME_LEN));
    let slice = data.get(start..limit).ok_or(Error::Truncated)?;
    let nul = slice.iter().position(|&b| b == 0).ok_or(Error::Corrupt)?;
    *pos = start + nul + 1;
    Ok(&slice[..nul])
}

impl<'a> Cabinet<'a> {
    /// Parse a cabinet header, folder list, and file list.
    pub fn parse(data: &'a [u8]) -> Result<Cabinet<'a>, Error> {
        if data.len() < 36 {
            return Err(Error::Truncated);
        }
        if &data[..4] != CAB_SIGNATURE {
            return Err(Error::NotThisFormat);
        }
        let cb_cabinet = u32_at(data, 8).ok_or(Error::Truncated)? as usize;
        let coff_files = u32_at(data, 16).ok_or(Error::Truncated)? as usize;
        let c_folders = usize::from(u16_at(data, 26).ok_or(Error::Truncated)?);
        let c_files = usize::from(u16_at(data, 28).ok_or(Error::Truncated)?);
        let flags = u16_at(data, 30).ok_or(Error::Truncated)?;

        if cb_cabinet > data.len() {
            return Err(Error::Truncated);
        }
        if c_folders > MAX_FOLDERS || c_files > MAX_FILES {
            return Err(Error::TooLarge);
        }

        let mut pos = 36usize;
        let (mut cb_cffolder, mut cb_cfdata) = (0usize, 0usize);
        if flags & FLAG_RESERVE_PRESENT != 0 {
            let cb_cfheader = usize::from(u16_at(data, pos).ok_or(Error::Truncated)?);
            cb_cffolder = usize::from(*data.get(pos + 2).ok_or(Error::Truncated)?);
            cb_cfdata = usize::from(*data.get(pos + 3).ok_or(Error::Truncated)?);
            pos = pos.checked_add(4 + cb_cfheader).ok_or(Error::Corrupt)?;
            if pos > data.len() {
                return Err(Error::Truncated);
            }
        }
        if flags & FLAG_PREV_CABINET != 0 {
            let _ = read_cstr(data, &mut pos)?;
            let _ = read_cstr(data, &mut pos)?;
        }
        if flags & FLAG_NEXT_CABINET != 0 {
            let _ = read_cstr(data, &mut pos)?;
            let _ = read_cstr(data, &mut pos)?;
        }

        let folder_stride = 8usize.checked_add(cb_cffolder).ok_or(Error::Corrupt)?;
        let mut folders = Vec::with_capacity(c_folders);
        for _ in 0..c_folders {
            let end = pos.checked_add(folder_stride).ok_or(Error::Corrupt)?;
            if end > data.len() {
                return Err(Error::Truncated);
            }
            let data_offset = u32_at(data, pos).ok_or(Error::Truncated)?;
            let block_count = u16_at(data, pos + 4).ok_or(Error::Truncated)?;
            let compress_type = u16_at(data, pos + 6).ok_or(Error::Truncated)?;
            folders.push(Folder { data_offset, block_count, compress_type });
            pos = end;
        }

        let mut file_pos = coff_files;
        let mut files = Vec::with_capacity(c_files);
        for _ in 0..c_files {
            if file_pos.checked_add(16).is_none_or(|e| e > data.len()) {
                return Err(Error::Truncated);
            }
            let size = u32_at(data, file_pos).ok_or(Error::Truncated)?;
            let folder_offset = u32_at(data, file_pos + 4).ok_or(Error::Truncated)?;
            let folder_index = u16_at(data, file_pos + 8).ok_or(Error::Truncated)?;
            let attribs = u16_at(data, file_pos + 14).ok_or(Error::Truncated)?;
            file_pos += 16;
            let raw_name = read_cstr(data, &mut file_pos)?;
            let name = if attribs & ATTR_NAME_IS_UTF != 0 {
                String::from_utf8_lossy(raw_name).into_owned()
            } else {
                decode_cp1252(raw_name)
            };
            files.push(CabFile { name, size, folder_offset, folder_index, attribs });
        }

        Ok(Cabinet { data, folders, files, cb_cfdata_reserve: cb_cfdata })
    }

    /// Decompress all `CFDATA` blocks of `folder_idx` into a contiguous buffer.
    pub fn decompress_folder(&self, folder_idx: usize) -> Result<Vec<u8>, Error> {
        let folder = *self.folders.get(folder_idx).ok_or(Error::Corrupt)?;
        let method = folder.compress_type & 0x000F;
        if method != COMP_NONE && method != COMP_MSZIP {
            return Err(Error::Compression);
        }
        let mut pos = folder.data_offset as usize;
        let mut out = Vec::new();
        for _ in 0..folder.block_count {
            let hdr_end =
                pos.checked_add(8).and_then(|p| p.checked_add(self.cb_cfdata_reserve)).ok_or(Error::Corrupt)?;
            if hdr_end > self.data.len() {
                return Err(Error::Truncated);
            }
            let csum = u32_at(self.data, pos).ok_or(Error::Truncated)?;
            let cb_data = usize::from(u16_at(self.data, pos + 4).ok_or(Error::Truncated)?);
            let cb_uncomp = usize::from(u16_at(self.data, pos + 6).ok_or(Error::Truncated)?);
            let data_end = hdr_end.checked_add(cb_data).ok_or(Error::Corrupt)?;
            let payload = self.data.get(hdr_end..data_end).ok_or(Error::Truncated)?;

            if csum != 0 {
                // [MS-CAB] 2.1.4: checksum covers cbData, cbUncomp, abReserve (if any) and abData.
                let body = &self.data[pos + 8..data_end];
                let expected = csum_compute(&self.data[pos + 4..pos + 8], csum_compute(body, 0));
                if expected != csum {
                    return Err(Error::Corrupt);
                }
            }
            if cb_uncomp > MAX_UNCOMP_BLOCK {
                return Err(Error::Corrupt);
            }
            if out.len().saturating_add(cb_uncomp) > MAX_FOLDER_UNCOMP {
                return Err(Error::TooLarge);
            }

            match method {
                COMP_NONE => {
                    if payload.len() != cb_uncomp {
                        return Err(Error::Corrupt);
                    }
                    out.extend_from_slice(payload);
                }
                COMP_MSZIP => {
                    if payload.len() < 2 || &payload[..2] != b"CK" {
                        return Err(Error::Corrupt);
                    }
                    let hist_start = out.len().saturating_sub(32_768);
                    let (decoded, _) =
                        poly_png::inflate::inflate_with_history(&payload[2..], &out[hist_start..], cb_uncomp)
                            .map_err(|_| Error::Corrupt)?;
                    if decoded.len() != cb_uncomp {
                        return Err(Error::Corrupt);
                    }
                    out.extend_from_slice(&decoded);
                }
                _ => return Err(Error::Compression),
            }
            pos = data_end;
        }
        Ok(out)
    }

    /// Extract files matching `want` one folder at a time, invoking `on_file`
    /// with each file's cabinet name and uncompressed bytes.
    pub fn extract_matching<W, F>(&self, mut want: W, mut on_file: F) -> Result<(), Error>
    where
        W: FnMut(&str) -> bool,
        F: FnMut(&str, Vec<u8>) -> Result<(), Error>,
    {
        for folder_idx in 0..self.folders.len() {
            let matching: Vec<&CabFile> =
                self.files.iter().filter(|f| usize::from(f.folder_index) == folder_idx && want(&f.name)).collect();
            if matching.is_empty() {
                continue;
            }
            let buf = self.decompress_folder(folder_idx)?;
            for file in matching {
                let start = file.folder_offset as usize;
                let end = start.checked_add(file.size as usize).ok_or(Error::Corrupt)?;
                let slice = buf.get(start..end).ok_or(Error::Truncated)?;
                on_file(&file.name, slice.to_vec())?;
            }
        }
        // Reject files that claim continuation across split cabinets if wanted.
        for file in &self.files {
            if file.folder_index >= IFOLDER_CONTINUED_FROM_PREV && want(&file.name) {
                return Err(Error::Corrupt);
            }
        }
        Ok(())
    }

    /// Extract every file in the cabinet.
    pub fn extract_all(&self) -> Result<Vec<(String, Vec<u8>)>, Error> {
        let mut out = Vec::with_capacity(self.files.len());
        self.extract_matching(
            |_| true,
            |name, bytes| {
                out.push((String::from(name), bytes));
                Ok(())
            },
        )?;
        Ok(out)
    }
}
