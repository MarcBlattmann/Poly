//! Synthetic clean-room fixtures and tests for `[MS-CFB]`, `[MS-CAB]`, and
//! Windows Installer (`.msi`) parsing, directory resolution, extraction,
//! shortcuts, icons, manifest/uninstall metadata, and malformed/malicious input
//! handling.

use super::cab::{self, COMP_LZX, COMP_MSZIP, COMP_NONE, COMP_QUANTUM, Cabinet};
use super::cfb::{self, Cfb, ENDOFCHAIN, FREESECT, NOSTREAM, OBJ_ROOT, OBJ_STREAM};
use super::msi::{self, MsiDatabase, MsiManifest};
use super::program::{CFB_SIGNATURE, MSI_CLSID, PNG_SIGNATURE};
use super::testfiles::{dib32, fake_png};
use crate::Error;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

fn reader(bytes: &[u8]) -> impl FnMut(u64, &mut [u8]) -> usize + '_ {
    move |at, buf| {
        let at = at as usize;
        if at >= bytes.len() {
            return 0;
        }
        let n = buf.len().min(bytes.len() - at);
        buf[..n].copy_from_slice(&bytes[at..at + n]);
        n
    }
}

// ---------------------------------------------------------------------------
// Synthetic `[MS-CFB]` builder (v3: 512-byte sectors, v4: 4096-byte sectors)
// ---------------------------------------------------------------------------

struct CfbBuilder {
    major: u16,
    clsid: [u8; 16],
    streams: Vec<(Vec<u16>, Vec<u8>)>,
}

impl CfbBuilder {
    fn new(major: u16, clsid: [u8; 16]) -> Self {
        Self { major, clsid, streams: Vec::new() }
    }

    fn add_table(&mut self, name: &str, data: Vec<u8>) {
        self.streams.push((cfb::encode_msi_stream_name(name, true), data));
    }

    fn add_msi_stream(&mut self, name: &str, data: Vec<u8>) {
        self.streams.push((cfb::encode_msi_stream_name(name, false), data));
    }

    fn add_raw_stream(&mut self, name: &str, data: Vec<u8>) {
        self.streams.push((name.encode_utf16().collect(), data));
    }

    fn build(self) -> Vec<u8> {
        let shift: u16 = if self.major == 4 { 12 } else { 9 };
        let sec_size: usize = 1 << shift;
        let mini_cutoff: usize = 4096;
        let mini_size: usize = 64;

        let mut sectors: Vec<Vec<u8>> = Vec::new();
        let mut fat: Vec<u32> = Vec::new();
        let mut mini_data: Vec<u8> = Vec::new();
        let mut mini_fat: Vec<u32> = Vec::new();

        let alloc_chain = |sectors: &mut Vec<Vec<u8>>, fat: &mut Vec<u32>, data: &[u8]| -> u32 {
            if data.is_empty() {
                return ENDOFCHAIN;
            }
            let first = sectors.len() as u32;
            let count = data.len().div_ceil(sec_size);
            for i in 0..count {
                let mut buf = alloc::vec![0u8; sec_size];
                let start = i * sec_size;
                let take = (data.len() - start).min(sec_size);
                buf[..take].copy_from_slice(&data[start..start + take]);
                let sid = sectors.len() as u32;
                sectors.push(buf);
                fat.push(if i + 1 == count { ENDOFCHAIN } else { sid + 1 });
            }
            first
        };

        // Store streams either in Mini Stream (< 4096) or regular FAT (>= 4096).
        let mut stream_meta: Vec<(Vec<u16>, u32, u64)> = Vec::new();
        for (name, data) in self.streams {
            let size = data.len() as u64;
            let start = if data.is_empty() {
                ENDOFCHAIN
            } else if data.len() < mini_cutoff {
                let first_mini = mini_fat.len() as u32;
                let count = data.len().div_ceil(mini_size);
                for i in 0..count {
                    let start_b = i * mini_size;
                    let take = (data.len() - start_b).min(mini_size);
                    mini_data.extend_from_slice(&data[start_b..start_b + take]);
                    mini_data.resize(mini_data.len() + (mini_size - take), 0);
                    let mid = mini_fat.len() as u32;
                    mini_fat.push(if i + 1 == count { ENDOFCHAIN } else { mid + 1 });
                }
                first_mini
            } else {
                alloc_chain(&mut sectors, &mut fat, &data)
            };
            stream_meta.push((name, start, size));
        }

        // Write Mini Stream sectors and Mini FAT sectors.
        let mini_stream_start = alloc_chain(&mut sectors, &mut fat, &mini_data);
        let mini_stream_size = mini_data.len() as u64;
        let (first_mini_fat, num_mini_fat) = if mini_fat.is_empty() {
            (ENDOFCHAIN, 0u32)
        } else {
            let mut raw = Vec::with_capacity(mini_fat.len() * 4);
            for v in &mini_fat {
                raw.extend_from_slice(&v.to_le_bytes());
            }
            let count = raw.len().div_ceil(sec_size) as u32;
            (alloc_chain(&mut sectors, &mut fat, &raw), count)
        };

        // Build directory entries (entry 0 = Root Storage, entries 1..=N chained as right siblings).
        let mut dir_bytes = Vec::new();
        let write_entry = |out: &mut Vec<u8>,
                           name: &[u16],
                           obj_type: u8,
                           left: u32,
                           right: u32,
                           child: u32,
                           clsid: [u8; 16],
                           start: u32,
                           size: u64| {
            let mut e = [0u8; 128];
            let count = name.len().min(31);
            for (i, &u) in name[..count].iter().enumerate() {
                e[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
            }
            let name_len = ((count + 1) * 2) as u16;
            e[0x40..0x42].copy_from_slice(&name_len.to_le_bytes());
            e[0x42] = obj_type;
            e[0x43] = 1; // black
            e[0x44..0x48].copy_from_slice(&left.to_le_bytes());
            e[0x48..0x4C].copy_from_slice(&right.to_le_bytes());
            e[0x4C..0x50].copy_from_slice(&child.to_le_bytes());
            e[0x50..0x60].copy_from_slice(&clsid);
            e[0x74..0x78].copy_from_slice(&start.to_le_bytes());
            e[0x78..0x80].copy_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&e);
        };
        let root_name: Vec<u16> = "Root Entry".encode_utf16().collect();
        let root_child = if stream_meta.is_empty() { NOSTREAM } else { 1 };
        write_entry(
            &mut dir_bytes,
            &root_name,
            OBJ_ROOT,
            NOSTREAM,
            NOSTREAM,
            root_child,
            self.clsid,
            mini_stream_start,
            mini_stream_size,
        );
        for (i, (name, start, size)) in stream_meta.iter().enumerate() {
            let right = if i + 1 < stream_meta.len() { (i + 2) as u32 } else { NOSTREAM };
            write_entry(&mut dir_bytes, name, OBJ_STREAM, NOSTREAM, right, NOSTREAM, [0; 16], *start, *size);
        }
        let num_dir_sectors = dir_bytes.len().div_ceil(sec_size) as u32;
        let first_dir_sector = alloc_chain(&mut sectors, &mut fat, &dir_bytes);

        // Allocate FAT sectors (supports up to 109 header DIFAT entries — plenty for tests).
        let entries_per_fat = sec_size / 4;
        let mut fat_sids = Vec::new();
        loop {
            let need = fat.len().div_ceil(entries_per_fat).max(1);
            if fat_sids.len() == need {
                break;
            }
            let sid = sectors.len() as u32;
            sectors.push(alloc::vec![0u8; sec_size]);
            fat.push(0xFFFF_FFFD); // FATSECT
            fat_sids.push(sid);
        }
        while !fat.len().is_multiple_of(entries_per_fat) {
            fat.push(FREESECT);
        }
        for (idx, &sid) in fat_sids.iter().enumerate() {
            let slice = &fat[idx * entries_per_fat..(idx + 1) * entries_per_fat];
            let sec = &mut sectors[sid as usize];
            for (j, &v) in slice.iter().enumerate() {
                sec[j * 4..j * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }

        // Build header (512 bytes, padded to sector size for v4).
        let mut out = alloc::vec![0u8; sec_size];
        out[..8].copy_from_slice(&CFB_SIGNATURE);
        out[0x18..0x1A].copy_from_slice(&0x003Eu16.to_le_bytes());
        out[0x1A..0x1C].copy_from_slice(&self.major.to_le_bytes());
        out[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
        out[0x1E..0x20].copy_from_slice(&shift.to_le_bytes());
        out[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
        if self.major == 4 {
            out[0x28..0x2C].copy_from_slice(&num_dir_sectors.to_le_bytes());
        }
        out[0x2C..0x30].copy_from_slice(&(fat_sids.len() as u32).to_le_bytes());
        out[0x30..0x34].copy_from_slice(&first_dir_sector.to_le_bytes());
        out[0x38..0x3C].copy_from_slice(&(mini_cutoff as u32).to_le_bytes());
        out[0x3C..0x40].copy_from_slice(&first_mini_fat.to_le_bytes());
        out[0x40..0x44].copy_from_slice(&num_mini_fat.to_le_bytes());
        out[0x44..0x48].copy_from_slice(&ENDOFCHAIN.to_le_bytes());
        out[0x48..0x4C].copy_from_slice(&0u32.to_le_bytes());
        for i in 0..109 {
            let v = fat_sids.get(i).copied().unwrap_or(FREESECT);
            out[0x4C + i * 4..0x4C + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        for s in sectors {
            out.extend_from_slice(&s);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Synthetic `[MS-CAB]` builder (NONE, MSZIP with stored/fixed DEFLATE, LZX)
// ---------------------------------------------------------------------------

/// Build an MSZIP block payload (`"CK"` + stored DEFLATE block(s)).
fn mszip_stored_block(raw: &[u8]) -> Vec<u8> {
    let mut out = alloc::vec![b'C', b'K'];
    out.push(1); // BFINAL=1, BTYPE=00 (stored)
    let len = raw.len() as u16;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(raw);
    out
}

/// Build a `[MS-CAB]` archive with one folder containing `files` and the given
/// pre-encoded `(compressed_payload, uncomp_len)` blocks.
fn build_cab(compress_type: u16, files: &[(&str, usize)], blocks: &[(Vec<u8>, u16)], with_csum: bool) -> Vec<u8> {
    // Layout: CFHEADER (36) + 1 CFFOLDER (8) + CFFILEs + CFDATAs.
    let coff_files = 36u32 + 8;
    let mut file_section = Vec::new();
    let mut off = 0u32;
    for &(name, size) in files {
        file_section.extend_from_slice(&(size as u32).to_le_bytes());
        file_section.extend_from_slice(&off.to_le_bytes());
        file_section.extend_from_slice(&0u16.to_le_bytes()); // iFolder = 0
        file_section.extend_from_slice(&0u16.to_le_bytes()); // date
        file_section.extend_from_slice(&0u16.to_le_bytes()); // time
        file_section.extend_from_slice(&0x80u16.to_le_bytes()); // UTF-8 name
        file_section.extend_from_slice(name.as_bytes());
        file_section.push(0);
        off += size as u32;
    }
    let coff_data = coff_files + file_section.len() as u32;
    let mut data_section = Vec::new();
    for (payload, uncomp) in blocks {
        let mut hdr = [0u8; 4];
        hdr[0..2].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        hdr[2..4].copy_from_slice(&uncomp.to_le_bytes());
        let csum = if with_csum { cab::csum_compute(&hdr, cab::csum_compute(payload, 0)) } else { 0 };
        data_section.extend_from_slice(&csum.to_le_bytes());
        data_section.extend_from_slice(&hdr);
        data_section.extend_from_slice(payload);
    }
    let total = coff_data + data_section.len() as u32;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"MSCF");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&coff_files.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(3); // minor
    out.push(1); // major
    out.extend_from_slice(&1u16.to_le_bytes()); // cFolders
    out.extend_from_slice(&(files.len() as u16).to_le_bytes()); // cFiles
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&1u16.to_le_bytes()); // setID
    out.extend_from_slice(&0u16.to_le_bytes()); // iCabinet
    // CFFOLDER
    out.extend_from_slice(&coff_data.to_le_bytes());
    out.extend_from_slice(&(blocks.len() as u16).to_le_bytes());
    out.extend_from_slice(&compress_type.to_le_bytes());
    out.extend_from_slice(&file_section);
    out.extend_from_slice(&data_section);
    out
}

fn build_stored_cab(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut blob = Vec::new();
    let mut meta = Vec::new();
    for &(name, data) in files {
        meta.push((name, data.len()));
        blob.extend_from_slice(data);
    }
    let blocks: Vec<(Vec<u8>, u16)> = blob.chunks(32_768).map(|c| (c.to_vec(), c.len() as u16)).collect();
    build_cab(COMP_NONE, &meta, &blocks, true)
}

fn build_mszip_cab(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut blob = Vec::new();
    let mut meta = Vec::new();
    for &(name, data) in files {
        meta.push((name, data.len()));
        blob.extend_from_slice(data);
    }
    let blocks: Vec<(Vec<u8>, u16)> = blob.chunks(32_768).map(|c| (mszip_stored_block(c), c.len() as u16)).collect();
    build_cab(COMP_MSZIP, &meta, &blocks, true)
}

// ---------------------------------------------------------------------------
// Synthetic MSI database builder
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Val {
    Null,
    Str(&'static str),
    Int(i32),
    Bin,
}

struct TableSpec {
    name: String,
    cols: Vec<(String, u16)>,
    rows: Vec<Vec<Val>>,
}

struct MsiBuilder {
    codepage: u16,
    long_strings: bool,
    pool_map: BTreeMap<String, u32>,
    pool_list: Vec<Vec<u8>>,
    tables: Vec<TableSpec>,
    extra_streams: Vec<(String, Vec<u8>, bool)>, // (name, data, encode_msi)
}

impl MsiBuilder {
    fn new(long_strings: bool) -> Self {
        Self {
            codepage: 65001,
            long_strings,
            pool_map: BTreeMap::new(),
            pool_list: Vec::new(),
            tables: Vec::new(),
            extra_streams: Vec::new(),
        }
    }

    fn intern(&mut self, s: &str) -> u32 {
        if s.is_empty() {
            return 0;
        }
        if let Some(&id) = self.pool_map.get(s) {
            return id;
        }
        let id = (self.pool_list.len() + 1) as u32;
        self.pool_list.push(s.as_bytes().to_vec());
        self.pool_map.insert(s.to_string(), id);
        id
    }

    fn add_table(&mut self, name: &str, cols: &[(&str, u16)], rows: Vec<Vec<Val>>) {
        self.tables.push(TableSpec {
            name: name.to_string(),
            cols: cols.iter().map(|&(n, t)| (n.to_string(), t)).collect(),
            rows,
        });
    }

    fn add_msi_stream(&mut self, name: &str, data: Vec<u8>) {
        self.extra_streams.push((name.to_string(), data, true));
    }

    fn add_raw_stream(&mut self, name: &str, data: Vec<u8>) {
        self.extra_streams.push((name.to_string(), data, false));
    }

    fn encode_table(&mut self, cols: &[(String, u16)], rows: &[Vec<Val>]) -> Vec<u8> {
        let mut out = Vec::new();
        for (c_idx, (_, col_type)) in cols.iter().enumerate() {
            let is_str = (col_type & 0x0800) != 0 && ((col_type & 0x0400) != 0 || (col_type & 0xFF) != 0);
            let is_bin = (col_type & 0x0800) != 0 && (col_type & 0x0400) == 0 && (col_type & 0xFF) == 0;
            let width = col_type & 0xFF;
            for row in rows {
                match &row[c_idx] {
                    Val::Null => {
                        let n = if is_str {
                            if self.long_strings { 3 } else { 2 }
                        } else if is_bin {
                            2
                        } else {
                            usize::from(width)
                        };
                        out.extend(core::iter::repeat_n(0u8, n));
                    }
                    Val::Str(s) => {
                        let id = self.intern(s);
                        if self.long_strings {
                            out.push((id & 0xFF) as u8);
                            out.push(((id >> 8) & 0xFF) as u8);
                            out.push(((id >> 16) & 0xFF) as u8);
                        } else {
                            out.extend_from_slice(&(id as u16).to_le_bytes());
                        }
                    }
                    Val::Int(v) => {
                        if width == 2 {
                            let enc = ((*v as i16) as u16) ^ 0x8000;
                            out.extend_from_slice(&enc.to_le_bytes());
                        } else {
                            let enc = (*v as u32) ^ 0x8000_0000;
                            out.extend_from_slice(&enc.to_le_bytes());
                        }
                    }
                    Val::Bin => {
                        out.extend_from_slice(&1u16.to_le_bytes());
                    }
                }
            }
        }
        out
    }

    fn build(mut self, major_cfb: u16) -> Vec<u8> {
        // Pre-encode user tables and collect `_Tables` and `_Columns` rows.
        let mut cfb = CfbBuilder::new(major_cfb, MSI_CLSID);
        let mut table_names: Vec<String> = Vec::new();
        let mut col_rows: Vec<(String, i32, String, i32)> = Vec::new();
        let tables = core::mem::take(&mut self.tables);
        for spec in &tables {
            table_names.push(spec.name.clone());
            for (idx, (cname, ctype)) in spec.cols.iter().enumerate() {
                col_rows.push((spec.name.clone(), (idx + 1) as i32, cname.clone(), i32::from(*ctype as i16)));
            }
            let encoded = self.encode_table(&spec.cols, &spec.rows);
            cfb.add_table(&spec.name, encoded);
        }

        // Encode `_Tables`
        let mut tables_raw = Vec::new();
        for t in &table_names {
            let id = self.intern(t);
            if self.long_strings {
                tables_raw.extend_from_slice(&[id as u8, (id >> 8) as u8, (id >> 16) as u8]);
            } else {
                tables_raw.extend_from_slice(&(id as u16).to_le_bytes());
            }
        }
        cfb.add_table("_Tables", tables_raw);

        // Encode `_Columns` (column-major: Table, Number, Name, Type)
        let mut cols_raw = Vec::new();
        let mut t_ids = Vec::new();
        let mut n_ids = Vec::new();
        for (t, _, n, _) in &col_rows {
            t_ids.push(self.intern(t));
            n_ids.push(self.intern(n));
        }
        for id in t_ids {
            if self.long_strings {
                cols_raw.extend_from_slice(&[id as u8, (id >> 8) as u8, (id >> 16) as u8]);
            } else {
                cols_raw.extend_from_slice(&(id as u16).to_le_bytes());
            }
        }
        for (_, num, _, _) in &col_rows {
            let enc = ((*num as i16) as u16) ^ 0x8000;
            cols_raw.extend_from_slice(&enc.to_le_bytes());
        }
        for id in n_ids {
            if self.long_strings {
                cols_raw.extend_from_slice(&[id as u8, (id >> 8) as u8, (id >> 16) as u8]);
            } else {
                cols_raw.extend_from_slice(&(id as u16).to_le_bytes());
            }
        }
        for (_, _, _, ctype) in &col_rows {
            let enc = ((*ctype as i16) as u16) ^ 0x8000;
            cols_raw.extend_from_slice(&enc.to_le_bytes());
        }
        cfb.add_table("_Columns", cols_raw);

        // Encode `_StringPool` and `_StringData`
        let mut sp = Vec::new();
        let mut sd = Vec::new();
        sp.extend_from_slice(&self.codepage.to_le_bytes());
        let flags: u16 = if self.long_strings { 0x8000 } else { 0 };
        sp.extend_from_slice(&flags.to_le_bytes());
        for bytes in &self.pool_list {
            sp.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            sp.extend_from_slice(&1u16.to_le_bytes()); // refcount = 1
            sd.extend_from_slice(bytes);
        }
        cfb.add_table("_StringPool", sp);
        cfb.add_table("_StringData", sd);

        for (name, data, encode) in self.extra_streams {
            if encode {
                cfb.add_msi_stream(&name, data);
            } else {
                cfb.add_raw_stream(&name, data);
            }
        }
        cfb.build()
    }
}

/// Build a `[MS-OLEPS]` `\x05SummaryInformation` stream with VT_LPSTR properties.
fn summary_stream(props: &[(u32, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0xFFFEu16.to_le_bytes()); // byteOrder
    out.extend_from_slice(&0u16.to_le_bytes()); // version
    out.extend_from_slice(&0u32.to_le_bytes()); // osVer
    out.extend_from_slice(&[0u8; 16]); // clsid
    out.extend_from_slice(&1u32.to_le_bytes()); // cSections
    out.extend_from_slice(&[0u8; 16]); // FMTID
    out.extend_from_slice(&48u32.to_le_bytes()); // section offset = 48

    let mut sec = Vec::new();
    sec.extend_from_slice(&0u32.to_le_bytes()); // placeholder size
    sec.extend_from_slice(&(props.len() as u32).to_le_bytes());
    let mut val_offset = 8 + props.len() * 8;
    let mut values = Vec::new();
    for &(pid, text) in props {
        sec.extend_from_slice(&pid.to_le_bytes());
        sec.extend_from_slice(&(val_offset as u32).to_le_bytes());
        let mut entry = Vec::new();
        entry.extend_from_slice(&30u32.to_le_bytes()); // VT_LPSTR
        let bytes = text.as_bytes();
        entry.extend_from_slice(&((bytes.len() + 1) as u32).to_le_bytes());
        entry.extend_from_slice(bytes);
        entry.push(0);
        while !entry.len().is_multiple_of(4) {
            entry.push(0);
        }
        val_offset += entry.len();
        values.extend_from_slice(&entry);
    }
    sec.extend_from_slice(&values);
    let sec_len = sec.len() as u32;
    sec[0..4].copy_from_slice(&sec_len.to_le_bytes());
    out.extend_from_slice(&sec);
    out
}

/// Build a synthetic `.ico` file containing one DIB or PNG image.
fn build_ico(images: &[(u8, u16, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + images.len() * 16;
    for &(w, bits, data) in images {
        out.extend_from_slice(&[w, w, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for &(_, _, data) in images {
        out.extend_from_slice(data);
    }
    out
}

/// Column type constants for synthetic tables.
const S_KEY: u16 = 0x2D48; // PK string
const S_REQ: u16 = 0x0D48; // required string
const S_OPT: u16 = 0x1D48; // nullable string
const L_REQ: u16 = 0x0F00; // localizable unlimited string
const I2_KEY: u16 = 0x2502;
const I2_REQ: u16 = 0x0502;
const I2_OPT: u16 = 0x1502;
const I4_REQ: u16 = 0x0504;
const BIN_COL: u16 = 0x0900;

fn sample_msi(
    cabinet: Option<&'static str>,
    cab_bytes: Option<(&str, Vec<u8>)>,
    long_strings: bool,
    cfb_major: u16,
) -> Vec<u8> {
    let mut b = MsiBuilder::new(long_strings);
    b.add_table(
        "Property",
        &[("Property", S_KEY), ("Value", L_REQ)],
        alloc::vec![
            alloc::vec![Val::Str("ProductName"), Val::Str("Widget Studio")],
            alloc::vec![Val::Str("ProductVersion"), Val::Str("2.4.1")],
            alloc::vec![Val::Str("Manufacturer"), Val::Str("Widget Corp")],
            alloc::vec![Val::Str("ProductCode"), Val::Str("{11111111-2222-3333-4444-555555555555}")],
            alloc::vec![Val::Str("ARPCOMMENTS"), Val::Str("Design and build widgets")],
            alloc::vec![Val::Str("ARPPRODUCTICON"), Val::Str("AppIcon.ico")],
            alloc::vec![Val::Str("ALLUSERS"), Val::Str("1")],
        ],
    );
    b.add_table(
        "Directory",
        &[("Directory", S_KEY), ("Directory_Parent", S_OPT), ("DefaultDir", L_REQ)],
        alloc::vec![
            alloc::vec![Val::Str("TARGETDIR"), Val::Null, Val::Str("SourceDir")],
            alloc::vec![Val::Str("ProgramFiles64Folder"), Val::Str("TARGETDIR"), Val::Str(".")],
            alloc::vec![
                Val::Str("INSTALLDIR"),
                Val::Str("ProgramFiles64Folder"),
                Val::Str("WIDGET~1|Widget Studio:widgets")
            ],
            alloc::vec![Val::Str("SUBDIR"), Val::Str("INSTALLDIR"), Val::Str("data")],
            alloc::vec![Val::Str("ProgramMenuFolder"), Val::Str("TARGETDIR"), Val::Str(".")],
            alloc::vec![Val::Str("DesktopFolder"), Val::Str("TARGETDIR"), Val::Str(".")],
        ],
    );
    b.add_table(
        "Component",
        &[
            ("Component", S_KEY),
            ("ComponentId", S_OPT),
            ("Directory_", S_REQ),
            ("Attributes", I2_REQ),
            ("Condition", S_OPT),
            ("KeyPath", S_OPT),
        ],
        alloc::vec![
            alloc::vec![
                Val::Str("MainComp"),
                Val::Str("{AAAA-BBBB}"),
                Val::Str("INSTALLDIR"),
                Val::Int(0),
                Val::Null,
                Val::Str("MainExe"),
            ],
            alloc::vec![
                Val::Str("DataComp"),
                Val::Str("{CCCC-DDDD}"),
                Val::Str("SUBDIR"),
                Val::Int(0),
                Val::Null,
                Val::Str("ReadmeFile"),
            ],
        ],
    );
    b.add_table(
        "File",
        &[
            ("File", S_KEY),
            ("Component_", S_REQ),
            ("FileName", L_REQ),
            ("FileSize", I4_REQ),
            ("Version", S_OPT),
            ("Attributes", I2_OPT),
            ("Sequence", I2_REQ),
        ],
        alloc::vec![
            alloc::vec![
                Val::Str("MainExe"),
                Val::Str("MainComp"),
                Val::Str("WIDGET~1.EXE|widget.exe"),
                Val::Int(12),
                Val::Str("2.4.1.0"),
                Val::Int(0x4000),
                Val::Int(1),
            ],
            alloc::vec![
                Val::Str("ReadmeFile"),
                Val::Str("DataComp"),
                Val::Str("readme.txt"),
                Val::Int(18),
                Val::Null,
                Val::Int(0x4000),
                Val::Int(2),
            ],
        ],
    );
    b.add_table(
        "Media",
        &[("DiskId", I2_KEY), ("LastSequence", I2_REQ), ("Cabinet", S_OPT), ("Source", S_OPT)],
        alloc::vec![alloc::vec![
            Val::Int(1),
            Val::Int(2),
            match cabinet {
                Some(c) => Val::Str(c),
                None => Val::Null,
            },
            Val::Null,
        ]],
    );
    b.add_table(
        "Shortcut",
        &[
            ("Shortcut", S_KEY),
            ("Directory_", S_REQ),
            ("Name", L_REQ),
            ("Component_", S_REQ),
            ("Target", S_REQ),
            ("Arguments", S_OPT),
            ("Description", S_OPT),
            ("Icon_", S_OPT),
            ("IconIndex", I2_OPT),
            ("WkDir", S_OPT),
        ],
        alloc::vec![
            alloc::vec![
                Val::Str("StartMenuSc"),
                Val::Str("ProgramMenuFolder"),
                Val::Str("WIDGET~1|Widget Studio"),
                Val::Str("MainComp"),
                Val::Str("[#MainExe]"),
                Val::Str("--mode \"[ProductName]\""),
                Val::Str("Launch Widget Studio"),
                Val::Str("AppIcon.ico"),
                Val::Int(0),
                Val::Str("INSTALLDIR"),
            ],
            // Duplicate on Desktop pointing to same argv -> deduplicated.
            alloc::vec![
                Val::Str("DesktopSc"),
                Val::Str("DesktopFolder"),
                Val::Str("Widget Studio"),
                Val::Str("MainComp"),
                Val::Str("MainFeature"), // advertised shortcut -> resolves via MainComp.KeyPath!
                Val::Str("--mode \"[ProductName]\""),
                Val::Str("Desktop link"),
                Val::Str("AppIcon.ico"),
                Val::Int(0),
                Val::Str("INSTALLDIR"),
            ],
            // Uninstaller shortcut -> filtered out.
            alloc::vec![
                Val::Str("UninstSc"),
                Val::Str("ProgramMenuFolder"),
                Val::Str("Uninstall Widget Studio"),
                Val::Str("MainComp"),
                Val::Str("[#MainExe]"),
                Val::Str("/uninstall"),
                Val::Str("Remove"),
                Val::Null,
                Val::Int(0),
                Val::Null,
            ],
            // Non-exe shortcut -> filtered out.
            alloc::vec![
                Val::Str("ReadmeSc"),
                Val::Str("ProgramMenuFolder"),
                Val::Str("Readme"),
                Val::Str("DataComp"),
                Val::Str("[#ReadmeFile]"),
                Val::Null,
                Val::Null,
                Val::Null,
                Val::Int(0),
                Val::Null,
            ],
        ],
    );
    b.add_table(
        "Registry",
        &[
            ("Registry", S_KEY),
            ("Root", I2_REQ),
            ("Key", L_REQ),
            ("Name", S_OPT),
            ("Value", S_OPT),
            ("Component_", S_REQ),
        ],
        alloc::vec![alloc::vec![
            Val::Str("Reg1"),
            Val::Int(2),
            Val::Str("Software\\Widget Corp\\Widget Studio"),
            Val::Str("InstallDir"),
            Val::Str("[INSTALLDIR]"),
            Val::Str("MainComp"),
        ]],
    );
    b.add_table(
        "Icon",
        &[("Name", S_KEY), ("Data", BIN_COL)],
        alloc::vec![alloc::vec![Val::Str("AppIcon.ico"), Val::Bin]],
    );

    let ico = build_ico(&[(16, 32, &dib32(16, [0xFF, 0x00, 0x00, 0xFF]))]);
    b.add_msi_stream("Icon.AppIcon.ico", ico);
    b.add_raw_stream(
        "\x05SummaryInformation",
        summary_stream(&[(2, "Installation Database"), (3, "Widget Studio"), (4, "Widget Corp"), (7, "x64;1033")]),
    );
    if let Some((stream_name, bytes)) = cab_bytes {
        b.add_msi_stream(stream_name, bytes);
    }
    b.build(cfb_major)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn cfb_v3_and_v4_read_mini_and_regular_streams_and_msi_names() {
    for name in ["_StringPool", "_StringData", "_Tables", "_Columns", "Property", "Icon.App.ico", "data1.cab"] {
        let enc_t = cfb::encode_msi_stream_name(name, true);
        assert_eq!(cfb::decode_msi_stream_name(&enc_t), (true, name.to_string()));
        let enc_s = cfb::encode_msi_stream_name(name, false);
        assert_eq!(cfb::decode_msi_stream_name(&enc_s), (false, name.to_string()));
    }

    let big = alloc::vec![0xABu8; 6000]; // >= 4096 -> regular FAT stream
    let small = b"hello mini stream".to_vec(); // < 4096 -> mini stream
    for major in [3u16, 4u16] {
        let mut b = CfbBuilder::new(major, MSI_CLSID);
        b.add_table("Property", small.clone());
        b.add_msi_stream("data1.cab", big.clone());
        let bytes = b.build();
        let cfb = Cfb::open(&mut reader(&bytes)).unwrap();
        assert_eq!(cfb.root_clsid(), MSI_CLSID);
        let p = cfb.find_table("Property").unwrap();
        assert_eq!(cfb.read_stream(p, &mut reader(&bytes), 1 << 20).unwrap(), small);
        let c = cfb.find_stream("DATA1.CAB").unwrap();
        assert_eq!(cfb.read_stream(c, &mut reader(&bytes), 1 << 20).unwrap(), big);
    }

    // Truncated / corrupt / non-CFB inputs never panic.
    assert_eq!(Cfb::open(&mut reader(b"not a cfb")).unwrap_err(), Error::Truncated);
    let mut bad = alloc::vec![0u8; 512];
    assert_eq!(Cfb::open(&mut reader(&bad)).unwrap_err(), Error::NotThisFormat);
    bad[..8].copy_from_slice(&CFB_SIGNATURE);
    assert_eq!(Cfb::open(&mut reader(&bad)).unwrap_err(), Error::Corrupt);
}

#[test]
fn cab_extracts_uncompressed_and_mszip_including_cross_block_history() {
    // 1. Uncompressed CAB
    let cab_none = build_stored_cab(&[("MainExe", b"MZ\x90\x00widget!!"), ("ReadmeFile", b"Hello from stored!")]);
    let parsed = Cabinet::parse(&cab_none).unwrap();
    assert_eq!(
        parsed.extract_all().unwrap(),
        alloc::vec![
            ("MainExe".to_string(), b"MZ\x90\x00widget!!".to_vec()),
            ("ReadmeFile".to_string(), b"Hello from stored!".to_vec()),
        ]
    );

    // 2. MSZIP single & multi-block CAB with cross-block LZ77 history reference!
    // Block 0: 32768 bytes ending in "Poly!!", stored inside an MSZIP block.
    let mut block0_plain = alloc::vec![b'A'; 32_762];
    block0_plain.extend_from_slice(b"Poly!!");
    let block0_comp = mszip_stored_block(&block0_plain);
    // Block 1: MSZIP fixed-Huffman DEFLATE copying 6 bytes at distance 6 from block 0's tail ("Poly!!").
    let mut bits = 0u64;
    let mut n = 0u32;
    let mut push = |val: u32, count: u32| {
        bits |= u64::from(val) << n;
        n += count;
    };
    let rev = |v: u32, w: u32| -> u32 {
        (0..w).fold(0, |acc, i| if (v >> i) & 1 != 0 { acc | (1 << (w - 1 - i)) } else { acc })
    };
    push(1, 1); // BFINAL
    push(1, 2); // BTYPE=01 (fixed)
    push(rev(4, 7), 7); // sym 260 -> len 6
    push(rev(4, 5), 5); // dist sym 4 -> base 5, 1 extra bit
    push(1, 1); // extra = 1 -> dist 6
    push(rev(0, 7), 7); // sym 256 -> EOB
    let mut block1_comp = alloc::vec![b'C', b'K'];
    for i in 0..n.div_ceil(8) {
        block1_comp.push(((bits >> (i * 8)) & 0xFF) as u8);
    }
    let cab_mszip = build_cab(
        COMP_MSZIP,
        &[("FirstFile", 32_768), ("SecondFile", 6)],
        &[(block0_comp, 32_768), (block1_comp, 6)],
        true,
    );
    let parsed = Cabinet::parse(&cab_mszip).unwrap();
    let files = parsed.extract_all().unwrap();
    assert_eq!(files[0].0, "FirstFile");
    assert_eq!(files[0].1, block0_plain);
    assert_eq!(files[1], ("SecondFile".to_string(), b"Poly!!".to_vec()));

    // 3. Unsupported compression types (LZX, Quantum) return Error::Compression cleanly.
    for comp in [COMP_LZX, COMP_QUANTUM, 0x000F] {
        let unsupported = build_cab(comp, &[("a", 4)], &[(b"1234".to_vec(), 4)], false);
        let parsed = Cabinet::parse(&unsupported).unwrap();
        assert_eq!(parsed.extract_all().unwrap_err(), Error::Compression);
    }

    // 4. Bad checksum & truncated inputs return clean errors.
    let mut bad_csum = cab_none.clone();
    let last = bad_csum.len() - 1;
    bad_csum[last] ^= 0xFF;
    assert_eq!(Cabinet::parse(&bad_csum).unwrap().extract_all().unwrap_err(), Error::Corrupt);
    assert_eq!(Cabinet::parse(&cab_none[..20]).unwrap_err(), Error::Truncated);
    assert_eq!(Cabinet::parse(b"NOT_A_CABINET_HEADER_WITH_36_BYTES!!").unwrap_err(), Error::NotThisFormat);
}

#[test]
fn msi_inspect_plan_extract_embedded_mszip_and_24bit_pool() {
    let cab_data = build_mszip_cab(&[("MainExe", b"MZ\x90\x00widget!!"), ("ReadmeFile", b"Hello from widget!")]);
    for (long_strings, cfb_major) in [(false, 3u16), (true, 4u16)] {
        let msi_bytes = sample_msi(Some("#data1.cab"), Some(("data1.cab", cab_data.clone())), long_strings, cfb_major);
        let info =
            msi::inspect("/home/poly/Downloads/widget.msi", msi_bytes.len() as u64, &mut reader(&msi_bytes)).unwrap();
        assert_eq!(info.name, "Widget Studio");
        assert_eq!(info.version, "2.4.1");
        assert_eq!(info.manufacturer, "Widget Corp");
        assert_eq!(info.summary, "Design and build widgets");
        assert_eq!(info.product_code, "{11111111-2222-3333-4444-555555555555}");
        assert_eq!(info.installed_size, 30);
        assert_eq!(info.file_count, 2);
        assert_eq!(info.bits, Some(64));
        assert_eq!(info.kind(), "Windows Installer package (64-bit)");
        assert!(info.icon.as_deref().is_some_and(|p| p.starts_with(PNG_SIGNATURE)));

        let db = MsiDatabase::open(&mut reader(&msi_bytes)).unwrap();
        assert_eq!(db.registry.len(), 1);
        assert_eq!(db.registry[0].key, "Software\\Widget Corp\\Widget Studio");

        let plan = db.plan_install().unwrap();
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.files[0].vfs_path, "/nt/c/Program Files/Widget Studio/widget.exe");
        assert_eq!(plan.files[0].dos_path, "C:\\Program Files\\Widget Studio\\widget.exe");
        assert_eq!(plan.files[1].vfs_path, "/nt/c/Program Files/Widget Studio/data/readme.txt");

        // Deduplicated shortcuts: StartMenu & Desktop share the same argv; Uninstall & Readme filtered out.
        assert_eq!(plan.shortcuts.len(), 1);
        assert_eq!(plan.shortcuts[0].id, "poly-windows-widget-studio.desktop");
        assert_eq!(plan.shortcuts[0].argv, ["/nt/c/Program Files/Widget Studio/widget.exe", "--mode", "Widget Studio"]);
        assert_eq!(plan.shortcuts[0].working_dir.as_deref(), Some("/nt/c/Program Files/Widget Studio"));

        let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let extracted = db
            .extract_files(
                &plan,
                &mut reader(&msi_bytes),
                |_| None,
                |path, data| {
                    written.insert(path.to_string(), data.to_vec());
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(extracted.len(), 2);
        assert_eq!(written["/nt/c/Program Files/Widget Studio/widget.exe"], b"MZ\x90\x00widget!!");
        assert_eq!(written["/nt/c/Program Files/Widget Studio/data/readme.txt"], b"Hello from widget!");

        let desktop = msi::desktop_file(&plan.shortcuts[0], Some("/h/widget.png"), "widget-studio", &info);
        assert!(desktop.contains("X-Poly-Msi-Id=widget-studio\n"), "{desktop}");
        assert!(desktop.contains("X-Poly-Windows-Target=C:\\Program Files\\Widget Studio\\widget.exe\n"));
        assert!(desktop.contains("X-Poly-Windows-Publisher=Widget Corp\n"));
        assert!(desktop.contains("X-Poly-Windows-Version=2.4.1\n"));
    }
}

#[test]
fn msi_extracts_external_cab_and_loose_files() {
    // 1. External CAB sitting next to the `.msi`
    let ext_cab = build_stored_cab(&[("MainExe", b"MZext"), ("ReadmeFile", b"external readme")]);
    let msi_ext = sample_msi(Some("media1.cab"), None, false, 3);
    let db = MsiDatabase::open(&mut reader(&msi_ext)).unwrap();
    let plan = db.plan_install().unwrap();
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    db.extract_files(
        &plan,
        &mut reader(&msi_ext),
        |name| (name == "media1.cab").then(|| ext_cab.clone()),
        |path, data| {
            written.insert(path.to_string(), data.to_vec());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(written["/nt/c/Program Files/Widget Studio/widget.exe"], b"MZext");
    assert_eq!(written["/nt/c/Program Files/Widget Studio/data/readme.txt"], b"external readme");

    // 2. Uncompressed loose files (Media.Cabinet is empty)
    let msi_loose = sample_msi(None, None, false, 3);
    let db = MsiDatabase::open(&mut reader(&msi_loose)).unwrap();
    let plan = db.plan_install().unwrap();
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    db.extract_files(
        &plan,
        &mut reader(&msi_loose),
        |rel| match rel {
            "widgets/widget.exe" => Some(b"MZloose".to_vec()),
            "widgets/data/readme.txt" => Some(b"loose text".to_vec()),
            _ => None,
        },
        |path, data| {
            written.insert(path.to_string(), data.to_vec());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(written["/nt/c/Program Files/Widget Studio/widget.exe"], b"MZloose");
    assert_eq!(written["/nt/c/Program Files/Widget Studio/data/readme.txt"], b"loose text");
}

#[test]
fn msi_falls_back_to_main_exe_when_shortcut_table_is_empty() {
    let mut b = MsiBuilder::new(false);
    b.add_table(
        "Property",
        &[("Property", S_KEY), ("Value", L_REQ)],
        alloc::vec![alloc::vec![Val::Str("ProductName"), Val::Str("MiniTool"),]],
    );
    b.add_table(
        "Directory",
        &[("Directory", S_KEY), ("Directory_Parent", S_OPT), ("DefaultDir", L_REQ)],
        alloc::vec![
            alloc::vec![Val::Str("TARGETDIR"), Val::Null, Val::Str("SourceDir")],
            alloc::vec![Val::Str("ProgramFilesFolder"), Val::Str("TARGETDIR"), Val::Str(".")],
            alloc::vec![Val::Str("APPDIR"), Val::Str("ProgramFilesFolder"), Val::Str("MiniTool")],
        ],
    );
    b.add_table(
        "Component",
        &[("Component", S_KEY), ("Directory_", S_REQ), ("Attributes", I2_REQ)],
        alloc::vec![alloc::vec![Val::Str("C1"), Val::Str("APPDIR"), Val::Int(0)]],
    );
    b.add_table(
        "File",
        &[("File", S_KEY), ("Component_", S_REQ), ("FileName", L_REQ), ("FileSize", I4_REQ), ("Sequence", I2_REQ)],
        alloc::vec![
            alloc::vec![Val::Str("F1"), Val::Str("C1"), Val::Str("uninstall.exe"), Val::Int(10), Val::Int(1)],
            alloc::vec![Val::Str("F2"), Val::Str("C1"), Val::Str("minitool.exe"), Val::Int(20), Val::Int(2)],
        ],
    );
    let bytes = b.build(3);
    let db = MsiDatabase::open(&mut reader(&bytes)).unwrap();
    let plan = db.plan_install().unwrap();
    assert_eq!(plan.shortcuts.len(), 1);
    assert_eq!(plan.shortcuts[0].name, "MiniTool");
    assert_eq!(plan.shortcuts[0].target_vfs, "/nt/c/Program Files (x86)/MiniTool/minitool.exe");
}

#[test]
fn directory_resolution_maps_system_folders_and_rejects_traversal() {
    assert_eq!(msi::system_folder_segments("TARGETDIR", true).unwrap(), Vec::<String>::new());
    assert_eq!(msi::system_folder_segments("ProgramFilesFolder", false).unwrap(), ["Program Files (x86)"]);
    assert_eq!(msi::system_folder_segments("ProgramFiles64Folder", false).unwrap(), ["Program Files"]);
    assert_eq!(
        msi::system_folder_segments("CommonFilesFolder", false).unwrap(),
        ["Program Files (x86)", "Common Files"]
    );
    assert_eq!(msi::system_folder_segments("SystemFolder", false).unwrap(), ["Windows", "SysWOW64"]);
    assert_eq!(msi::system_folder_segments("System64Folder", false).unwrap(), ["Windows", "System32"]);
    assert_eq!(msi::system_folder_segments("WindowsFolder", false).unwrap(), ["Windows"]);
    assert_eq!(msi::system_folder_segments("DesktopFolder", true).unwrap(), ["Users", "Public", "Desktop"]);
    assert_eq!(msi::system_folder_segments("DesktopFolder", false).unwrap(), ["Users", "poly", "Desktop"]);
    assert_eq!(
        msi::system_folder_segments("ProgramMenuFolder", true).unwrap(),
        ["ProgramData", "Microsoft", "Windows", "Start Menu", "Programs"]
    );
    assert_eq!(
        msi::system_folder_segments("ProgramMenuFolder", false).unwrap(),
        ["Users", "poly", "AppData", "Roaming", "Microsoft", "Windows", "Start Menu", "Programs"]
    );

    // Malicious `DefaultDir` cases: `..`, absolute path, NUL, drive letter.
    for evil in ["../etc", "..\\Windows", "/etc/passwd", "C:\\evil", "bad\0name"] {
        let mut b = MsiBuilder::new(false);
        b.add_table(
            "Directory",
            &[("Directory", S_KEY), ("Directory_Parent", S_OPT), ("DefaultDir", L_REQ)],
            alloc::vec![
                alloc::vec![Val::Str("TARGETDIR"), Val::Null, Val::Str("SourceDir")],
                alloc::vec![Val::Str("BAD"), Val::Str("TARGETDIR"), Val::Str(evil)],
            ],
        );
        let bytes = b.build(3);
        let db = MsiDatabase::open(&mut reader(&bytes)).unwrap();
        assert_eq!(db.resolve_directories().unwrap_err(), Error::Corrupt, "should reject {evil:?}");
    }

    // Malicious `File.FileName` traversal.
    let mut b = MsiBuilder::new(false);
    b.add_table(
        "Directory",
        &[("Directory", S_KEY), ("Directory_Parent", S_OPT), ("DefaultDir", L_REQ)],
        alloc::vec![alloc::vec![Val::Str("TARGETDIR"), Val::Null, Val::Str("SourceDir")]],
    );
    b.add_table(
        "Component",
        &[("Component", S_KEY), ("Directory_", S_REQ), ("Attributes", I2_REQ)],
        alloc::vec![alloc::vec![Val::Str("C1"), Val::Str("TARGETDIR"), Val::Int(0)]],
    );
    b.add_table(
        "File",
        &[("File", S_KEY), ("Component_", S_REQ), ("FileName", L_REQ), ("FileSize", I4_REQ), ("Sequence", I2_REQ)],
        alloc::vec![alloc::vec![Val::Str("F1"), Val::Str("C1"), Val::Str("..\\escape.exe"), Val::Int(4), Val::Int(1)]],
    );
    let bytes = b.build(3);
    let db = MsiDatabase::open(&mut reader(&bytes)).unwrap();
    assert_eq!(db.plan_install().unwrap_err(), Error::Corrupt);

    // Malicious external cabinet traversal (`../secret.cab`).
    let msi_bad_cab = sample_msi(Some("../secret.cab"), None, false, 3);
    let db = MsiDatabase::open(&mut reader(&msi_bad_cab)).unwrap();
    assert_eq!(db.plan_install().unwrap_err(), Error::Corrupt);

    // Directory parent cycle.
    let mut b = MsiBuilder::new(false);
    b.add_table(
        "Directory",
        &[("Directory", S_KEY), ("Directory_Parent", S_OPT), ("DefaultDir", L_REQ)],
        alloc::vec![
            alloc::vec![Val::Str("A"), Val::Str("B"), Val::Str("a")],
            alloc::vec![Val::Str("B"), Val::Str("A"), Val::Str("b")],
        ],
    );
    let bytes = b.build(3);
    let db = MsiDatabase::open(&mut reader(&bytes)).unwrap();
    assert_eq!(db.resolve_directories().unwrap_err(), Error::Corrupt);
}

#[test]
fn manifest_roundtrip_and_path_safety_guards() {
    assert!(msi::is_safe_install_path("/nt/c/Program Files/Widget/app.exe"));
    assert!(!msi::is_safe_install_path("/nt/c"));
    assert!(!msi::is_safe_install_path("/nt/c/Program Files/../Windows/x"));
    assert!(!msi::is_safe_install_path("/etc/passwd"));
    assert!(!msi::is_safe_install_path("/nt/c/bad\0file"));

    assert!(msi::is_removable_dir("/nt/c/Program Files/Widget"));
    assert!(!msi::is_removable_dir("/nt/c/Program Files"));
    assert!(!msi::is_removable_dir("/nt/c/Program Files (x86)"));
    assert!(!msi::is_removable_dir("/nt/c/Windows"));
    assert!(!msi::is_removable_dir("/nt/c/Windows/System32"));
    assert!(!msi::is_removable_dir("/nt/c/Users/poly/Desktop"));

    let m = MsiManifest {
        name: "Widget Studio".into(),
        version: "2.4.1".into(),
        publisher: "Widget Corp".into(),
        product_code: "{1111}".into(),
        size: 1024,
        desktops: alloc::vec!["poly-windows-widget-studio.desktop".into()],
        icons: alloc::vec!["/home/poly/.local/share/poly/windows/poly-windows-widget-studio.png".into()],
        files: alloc::vec!["/nt/c/Program Files/Widget Studio/widget.exe".into()],
        dirs: alloc::vec!["/nt/c/Program Files/Widget Studio".into()],
    };
    let encoded = m.encode();
    let parsed = MsiManifest::parse(&encoded).unwrap();
    assert_eq!(parsed, m);

    // Unsafe paths in a tampered manifest are ignored on parse.
    let tampered =
        alloc::format!("{encoded}File=/etc/shadow\nFile=/nt/c/a/../b\nDir=/nt/c/Windows\nDir=/nt/c/Program Files\n");
    let parsed = MsiManifest::parse(&tampered).unwrap();
    assert_eq!(parsed.files, m.files);
    assert_eq!(parsed.dirs, m.dirs);

    // Icon decoding: PNG passthrough, ICO with PNG, invalid bytes.
    let png = fake_png(7);
    assert_eq!(msi::icon_bytes_to_png(&png, 0), Some(png.clone()));
    let ico_png = build_ico(&[(32, 32, &png)]);
    assert_eq!(msi::icon_bytes_to_png(&ico_png, 0), Some(png));
    assert_eq!(msi::icon_bytes_to_png(b"garbage", 0), None);
}
