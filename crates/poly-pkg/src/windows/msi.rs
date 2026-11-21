//! Clean-room Windows Installer (`.msi`) reader, planner, extractor, and
//! uninstall-manifest support.
//!
//! Written strictly from Microsoft's public Windows Installer documentation
//! (database tables `_StringPool`, `_StringData`, `_Tables`, `_Columns`,
//! `Property`, `Directory`, `Component`, `File`, `Media`, `Shortcut`,
//! `Registry`, `Icon`; `[MS-CFB]`; `[MS-CAB]`; `[MS-OLEPS]` Summary
//! Information property set; Microsoft Learn "System Folder Properties",
//! "Directory Table", "Formatted").

use crate::Error;
use crate::appimage;
use crate::windows::apps as winapps;
use crate::windows::cab::{self, Cabinet};
use crate::windows::cfb::Cfb;
use crate::windows::program::{self, MSI_CLSID, PNG_SIGNATURE, ReadAt};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Desktop entry key identifying an app installed from an `.msi` package.
pub const MSI_ID_KEY: &str = "X-Poly-Msi-Id";

/// Limits on table streams, embedded CAB streams, and recursion.
const MAX_TABLE_BYTES: usize = 32 << 20;
const MAX_STRING_DATA_BYTES: usize = 32 << 20;
const MAX_ICON_BYTES: usize = 8 << 20;
const MAX_CAB_BYTES: usize = 256 << 20;
const MAX_DIR_DEPTH: usize = 64;
const MAX_FORMAT_DEPTH: usize = 8;

/// Column type flags in `_Columns.Type` (Microsoft Learn, "Column Definition Format").
const COL_TYPE_STRING: u16 = 0x0800;
const COL_TYPE_NON_BINARY: u16 = 0x0400;

/// Summary of an `.msi` package shown in Poly's installer before installation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MsiInfo {
    /// `ProductName`, else the file's stem without `.msi`.
    pub name: String,
    /// `ProductVersion`.
    pub version: String,
    /// `Manufacturer`.
    pub manufacturer: String,
    /// `ARPCOMMENTS` or Summary Information `Comments`/`Subject`, else `Manufacturer`.
    pub summary: String,
    /// `ProductCode` GUID string, if present.
    pub product_code: String,
    /// `.msi` file size in bytes.
    pub size: u64,
    /// Sum of `File.FileSize` across the package's `File` table.
    pub installed_size: u64,
    /// Number of entries in the `File` table.
    pub file_count: usize,
    /// Target architecture (`Some(64)` or `Some(32)`) from Summary Information template, if present.
    pub bits: Option<u8>,
    /// Product icon as PNG bytes, if present in the `Icon` table.
    pub icon: Option<Vec<u8>>,
}

impl MsiInfo {
    /// The window's "Kind" fact.
    pub fn kind(&self) -> String {
        match self.bits {
            Some(bits) => alloc::format!("Windows Installer package ({bits}-bit)"),
            None => "Windows Installer package".to_string(),
        }
    }
}

/// Optional metadata from the `[MS-OLEPS]` `\x05SummaryInformation` stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SummaryInfo {
    pub title: String,
    pub subject: String,
    pub author: String,
    pub comments: String,
    pub template: String,
}

/// A column definition from `_Columns`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnDef {
    pub number: u16,
    pub name: String,
    pub col_type: u16,
}

impl ColumnDef {
    /// Whether this column stores a string pool ID.
    pub fn is_string(&self) -> bool {
        (self.col_type & COL_TYPE_STRING) != 0
            && ((self.col_type & COL_TYPE_NON_BINARY) != 0 || (self.col_type & 0xFF) != 0)
    }

    /// Whether this column stores a binary stream reference (`v0`/`V0`).
    pub fn is_binary(&self) -> bool {
        (self.col_type & COL_TYPE_STRING) != 0
            && (self.col_type & COL_TYPE_NON_BINARY) == 0
            && (self.col_type & 0xFF) == 0
    }

    /// Byte width of one cell in the column-major table stream.
    pub fn byte_width(&self, long_strings: bool) -> Result<usize, Error> {
        if self.is_string() {
            Ok(if long_strings { 3 } else { 2 })
        } else if self.is_binary() {
            Ok(2)
        } else {
            match self.col_type & 0x00FF {
                2 => Ok(2),
                4 => Ok(4),
                _ => Err(Error::Corrupt),
            }
        }
    }
}

/// One decoded cell in an MSI table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Null,
    Int(i32),
    Str(String),
    Binary,
}

impl Cell {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Cell::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Cell::Int(v) => Some(*v),
            _ => None,
        }
    }
}

/// A row in the `Directory` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryRow {
    pub directory: String,
    pub parent: Option<String>,
    pub default_dir: String,
}

/// A row in the `Component` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentRow {
    pub component: String,
    pub component_id: String,
    pub directory: String,
    pub attributes: i32,
    pub condition: String,
    pub key_path: Option<String>,
}

/// A row in the `File` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    pub file: String,
    pub component: String,
    pub file_name: String,
    pub file_size: u64,
    pub version: String,
    pub attributes: i32,
    pub sequence: i32,
}

/// A row in the `Media` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRow {
    pub disk_id: i32,
    pub last_sequence: i32,
    pub cabinet: Option<String>,
    pub source: String,
}

/// A row in the `Shortcut` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutRow {
    pub shortcut: String,
    pub directory: String,
    pub name: String,
    pub component: String,
    pub target: String,
    pub arguments: String,
    pub description: String,
    pub icon: Option<String>,
    pub icon_index: i32,
    pub wk_dir: Option<String>,
}

/// A row in the `Registry` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryRow {
    pub registry: String,
    pub root: i32,
    pub key: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub component: String,
}

/// A decoded MSI string pool (`_StringPool` + `_StringData`).
#[derive(Clone, Debug)]
struct StringPool {
    long_strings: bool,
    strings: Vec<String>,
}

impl StringPool {
    fn parse(pool: &[u8], data: &[u8]) -> Result<StringPool, Error> {
        if pool.len() < 4 || !pool.len().is_multiple_of(4) {
            return Err(Error::Corrupt);
        }
        let codepage = u16::from_le_bytes([pool[0], pool[1]]);
        let flags = u16::from_le_bytes([pool[2], pool[3]]);
        let long_strings = (flags & 0x8000) != 0;

        let mut strings = Vec::new();
        strings.push(String::new()); // ID 0 is always empty/NULL.
        let mut data_pos = 0usize;
        let mut idx = 4usize;
        while idx < pool.len() {
            let len_u16 = u16::from_le_bytes([pool[idx], pool[idx + 1]]);
            let refcount = u16::from_le_bytes([pool[idx + 2], pool[idx + 3]]);
            idx += 4;
            if len_u16 == 0 && refcount == 0 {
                strings.push(String::new());
                continue;
            }
            let byte_len = if len_u16 == 0 {
                // Extended 32-bit length occupies the next 4-byte pool slot.
                let next = pool.get(idx..idx + 4).ok_or(Error::Corrupt)?;
                idx += 4;
                let long_len = u32::from_le_bytes([next[0], next[1], next[2], next[3]]) as usize;
                // Reserve the extra string ID slot consumed by the length word.
                let slice = data
                    .get(data_pos..data_pos.checked_add(long_len).ok_or(Error::Corrupt)?)
                    .ok_or(Error::Truncated)?;
                data_pos += long_len;
                strings.push(decode_msi_text(slice, codepage));
                strings.push(String::new());
                continue;
            } else {
                usize::from(len_u16)
            };
            let end = data_pos.checked_add(byte_len).ok_or(Error::Corrupt)?;
            let slice = data.get(data_pos..end).ok_or(Error::Truncated)?;
            data_pos = end;
            strings.push(decode_msi_text(slice, codepage));
        }
        Ok(StringPool { long_strings, strings })
    }

    fn get(&self, id: u32) -> Result<&str, Error> {
        if id == 0 {
            return Ok("");
        }
        self.strings.get(id as usize).map(String::as_str).ok_or(Error::Corrupt)
    }
}

fn decode_msi_text(bytes: &[u8], codepage: u16) -> String {
    if codepage == 65001 {
        String::from_utf8_lossy(bytes).into_owned()
    } else if let Ok(s) = core::str::from_utf8(bytes) {
        s.to_string()
    } else {
        cab::decode_cp1252(bytes)
    }
}

/// Read one column-major MSI table stream given its column definitions.
fn read_table_rows(raw: &[u8], cols: &[ColumnDef], pool: &StringPool) -> Result<Vec<Vec<Cell>>, Error> {
    if cols.is_empty() || raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut widths = Vec::with_capacity(cols.len());
    let mut row_width = 0usize;
    for col in cols {
        let w = col.byte_width(pool.long_strings)?;
        widths.push(w);
        row_width = row_width.checked_add(w).ok_or(Error::Corrupt)?;
    }
    if row_width == 0 || !raw.len().is_multiple_of(row_width) {
        return Err(Error::Corrupt);
    }
    let num_rows = raw.len() / row_width;
    let mut rows: Vec<Vec<Cell>> = (0..num_rows).map(|_| Vec::with_capacity(cols.len())).collect();
    let mut col_offset = 0usize;
    for (col, &w) in cols.iter().zip(&widths) {
        let col_bytes = num_rows.checked_mul(w).ok_or(Error::Corrupt)?;
        let slice = raw.get(col_offset..col_offset + col_bytes).ok_or(Error::Truncated)?;
        col_offset += col_bytes;
        for (r, row) in rows.iter_mut().enumerate() {
            let at = r * w;
            let cell = if col.is_string() {
                let id = if w == 3 {
                    u32::from(slice[at]) | (u32::from(slice[at + 1]) << 8) | (u32::from(slice[at + 2]) << 16)
                } else {
                    u32::from(u16::from_le_bytes([slice[at], slice[at + 1]]))
                };
                if id == 0 { Cell::Null } else { Cell::Str(pool.get(id)?.to_string()) }
            } else if col.is_binary() {
                let v = u16::from_le_bytes([slice[at], slice[at + 1]]);
                if v == 0 { Cell::Null } else { Cell::Binary }
            } else if w == 2 {
                let v = u16::from_le_bytes([slice[at], slice[at + 1]]);
                if v == 0 { Cell::Null } else { Cell::Int(i32::from((v ^ 0x8000) as i16)) }
            } else {
                let v = u32::from_le_bytes([slice[at], slice[at + 1], slice[at + 2], slice[at + 3]]);
                if v == 0 { Cell::Null } else { Cell::Int((v ^ 0x8000_0000) as i32) }
            };
            row.push(cell);
        }
    }
    Ok(rows)
}

fn col_index(cols: &[ColumnDef], name: &str) -> Option<usize> {
    cols.iter().position(|c| c.name == name)
}

fn cell_str(row: &[Cell], idx: Option<usize>) -> Option<String> {
    let i = idx?;
    row.get(i)?.as_str().filter(|s| !s.is_empty()).map(ToString::to_string)
}

fn cell_int(row: &[Cell], idx: Option<usize>) -> Option<i32> {
    let i = idx?;
    row.get(i)?.as_int()
}

/// Parse the `[MS-OLEPS]` `\x05SummaryInformation` property set stream, if valid.
pub fn parse_summary_info(bytes: &[u8]) -> Option<SummaryInfo> {
    if bytes.len() < 48 {
        return None;
    }
    let byte_order = u16::from_le_bytes([bytes[0], bytes[1]]);
    if byte_order != 0xFFFE {
        return None;
    }
    let section_count = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    if section_count == 0 {
        return None;
    }
    // First section offset is at byte 44 (28-byte header + 16-byte FMTID).
    let sec_off = u32::from_le_bytes([bytes[44], bytes[45], bytes[46], bytes[47]]) as usize;
    let sec = bytes.get(sec_off..)?;
    if sec.len() < 8 {
        return None;
    }
    let prop_count = u32::from_le_bytes([sec[4], sec[5], sec[6], sec[7]]).min(256) as usize;
    let mut info = SummaryInfo::default();
    let mut codepage = 1252u16;

    // First pass: find PID_CODEPAGE (1) if present.
    for i in 0..prop_count {
        let off = 8 + i * 8;
        let entry = sec.get(off..off + 8)?;
        let pid = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let rel = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]) as usize;
        if pid == 1
            && let Some(val) = sec.get(rel..rel + 6)
        {
            let vt = u32::from_le_bytes([val[0], val[1], val[2], val[3]]);
            if vt == 2 {
                codepage = u16::from_le_bytes([val[4], val[5]]);
            }
        }
    }

    for i in 0..prop_count {
        let off = 8 + i * 8;
        let entry = sec.get(off..off + 8)?;
        let pid = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let rel = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]) as usize;
        let Some(val) = sec.get(rel..) else { continue };
        if val.len() < 8 {
            continue;
        }
        let vt = u32::from_le_bytes([val[0], val[1], val[2], val[3]]);
        let text = match vt {
            // VT_LPSTR (30)
            30 => {
                let len = u32::from_le_bytes([val[4], val[5], val[6], val[7]]) as usize;
                let Some(raw) = val.get(8..8usize.checked_add(len)?) else { continue };
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                decode_msi_text(&raw[..end], codepage)
            }
            // VT_LPWSTR (31)
            31 => {
                let count = u32::from_le_bytes([val[4], val[5], val[6], val[7]]) as usize;
                let byte_len = count.checked_mul(2)?;
                let Some(raw) = val.get(8..8usize.checked_add(byte_len)?) else { continue };
                let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
                String::from_utf16_lossy(&units[..end])
            }
            _ => continue,
        };
        match pid {
            2 => info.title = text,
            3 => info.subject = text,
            4 => info.author = text,
            6 => info.comments = text,
            7 => info.template = text,
            _ => {}
        }
    }
    Some(info)
}

/// Architecture bits indicated by a Summary Information `Template` string
/// (e.g., `"x64;1033"`, `"AMD64;1033"`, `"Intel64;1033"`, `"Intel;1033"`).
fn template_bits(template: &str) -> Option<u8> {
    let arch = template.split(';').next().unwrap_or("").trim();
    if arch.eq_ignore_ascii_case("x64") || arch.eq_ignore_ascii_case("amd64") || arch.eq_ignore_ascii_case("intel64") {
        Some(64)
    } else if arch.eq_ignore_ascii_case("intel") || arch.eq_ignore_ascii_case("x86") {
        Some(32)
    } else {
        None
    }
}

/// A parsed MSI database (metadata and standard tables).
#[derive(Clone, Debug)]
pub struct MsiDatabase {
    pub cfb: Cfb,
    pub summary: Option<SummaryInfo>,
    pub tables: Vec<String>,
    pub columns: BTreeMap<String, Vec<ColumnDef>>,
    pub properties: BTreeMap<String, String>,
    pub directories: Vec<DirectoryRow>,
    pub components: Vec<ComponentRow>,
    pub files: Vec<FileRow>,
    pub media: Vec<MediaRow>,
    pub shortcuts: Vec<ShortcutRow>,
    pub registry: Vec<RegistryRow>,
    pub icons: Vec<String>,
}

impl MsiDatabase {
    /// Open and parse an `.msi` database from `read`.
    pub fn open(read: ReadAt<'_>) -> Result<MsiDatabase, Error> {
        let cfb = Cfb::open(read)?;
        if cfb.root_clsid() != MSI_CLSID {
            return Err(Error::NotThisFormat);
        }

        let sp_entry = cfb.find_table("_StringPool").ok_or(Error::Corrupt)?.clone();
        let sd_entry = cfb.find_table("_StringData").ok_or(Error::Corrupt)?.clone();
        let sp_bytes = cfb.read_stream(&sp_entry, read, MAX_TABLE_BYTES)?;
        let sd_bytes = cfb.read_stream(&sd_entry, read, MAX_STRING_DATA_BYTES)?;
        let pool = StringPool::parse(&sp_bytes, &sd_bytes)?;

        // Parse `_Tables` (fixed schema: single string column `Name`).
        let tables_schema = alloc::vec![ColumnDef { number: 1, name: "Name".into(), col_type: 0x2D40 }];
        let mut tables = Vec::new();
        if let Some(entry) = cfb.find_table("_Tables").cloned() {
            let bytes = cfb.read_stream(&entry, read, MAX_TABLE_BYTES)?;
            for row in read_table_rows(&bytes, &tables_schema, &pool)? {
                if let Some(name) = cell_str(&row, Some(0)) {
                    tables.push(name);
                }
            }
        }

        // Parse `_Columns` (fixed schema: Table(s64), Number(i2), Name(s64), Type(i2)).
        let cols_schema = alloc::vec![
            ColumnDef { number: 1, name: "Table".into(), col_type: 0x2D40 },
            ColumnDef { number: 2, name: "Number".into(), col_type: 0x2502 },
            ColumnDef { number: 3, name: "Name".into(), col_type: 0x0D40 },
            ColumnDef { number: 4, name: "Type".into(), col_type: 0x0502 },
        ];
        let mut columns: BTreeMap<String, Vec<ColumnDef>> = BTreeMap::new();
        let cols_entry = cfb.find_table("_Columns").ok_or(Error::Corrupt)?.clone();
        let cols_bytes = cfb.read_stream(&cols_entry, read, MAX_TABLE_BYTES)?;
        for row in read_table_rows(&cols_bytes, &cols_schema, &pool)? {
            let Some(table) = cell_str(&row, Some(0)) else { continue };
            let Some(number) = cell_int(&row, Some(1)).and_then(|v| u16::try_from(v).ok()) else { continue };
            let Some(name) = cell_str(&row, Some(2)) else { continue };
            let Some(col_type) = cell_int(&row, Some(3)).map(|v| v as u16) else { continue };
            columns.entry(table).or_default().push(ColumnDef { number, name, col_type });
        }
        for defs in columns.values_mut() {
            defs.sort_by_key(|c| c.number);
        }

        let summary = cfb
            .find_stream("\x05SummaryInformation")
            .cloned()
            .and_then(|e| cfb.read_stream(&e, read, 64 * 1024).ok())
            .and_then(|b| parse_summary_info(&b));

        // Helper to read a named table if both schema and stream exist.
        let mut load = |table_name: &str| -> Result<(Vec<ColumnDef>, Vec<Vec<Cell>>), Error> {
            let Some(defs) = columns.get(table_name).cloned() else {
                return Ok((Vec::new(), Vec::new()));
            };
            let Some(entry) = cfb.find_table(table_name).cloned() else {
                return Ok((defs, Vec::new()));
            };
            let raw = cfb.read_stream(&entry, read, MAX_TABLE_BYTES)?;
            let rows = read_table_rows(&raw, &defs, &pool)?;
            Ok((defs, rows))
        };

        // Property
        let mut properties = BTreeMap::new();
        {
            let (defs, rows) = load("Property")?;
            let (k_i, v_i) = (col_index(&defs, "Property"), col_index(&defs, "Value"));
            for row in rows {
                if let Some(k) = cell_str(&row, k_i) {
                    let v = cell_str(&row, v_i).unwrap_or_default();
                    properties.insert(k, v);
                }
            }
        }

        // Directory
        let mut directories = Vec::new();
        {
            let (defs, rows) = load("Directory")?;
            let (d_i, p_i, dd_i) =
                (col_index(&defs, "Directory"), col_index(&defs, "Directory_Parent"), col_index(&defs, "DefaultDir"));
            for row in rows {
                let Some(directory) = cell_str(&row, d_i) else { continue };
                let parent = cell_str(&row, p_i);
                let default_dir = cell_str(&row, dd_i).unwrap_or_default();
                directories.push(DirectoryRow { directory, parent, default_dir });
            }
        }

        // Component
        let mut components = Vec::new();
        {
            let (defs, rows) = load("Component")?;
            let (c_i, cid_i, dir_i, attr_i, cond_i, kp_i) = (
                col_index(&defs, "Component"),
                col_index(&defs, "ComponentId"),
                col_index(&defs, "Directory_"),
                col_index(&defs, "Attributes"),
                col_index(&defs, "Condition"),
                col_index(&defs, "KeyPath"),
            );
            for row in rows {
                let Some(component) = cell_str(&row, c_i) else { continue };
                let Some(directory) = cell_str(&row, dir_i) else { continue };
                components.push(ComponentRow {
                    component,
                    component_id: cell_str(&row, cid_i).unwrap_or_default(),
                    directory,
                    attributes: cell_int(&row, attr_i).unwrap_or(0),
                    condition: cell_str(&row, cond_i).unwrap_or_default(),
                    key_path: cell_str(&row, kp_i),
                });
            }
        }

        // File
        let mut files = Vec::new();
        {
            let (defs, rows) = load("File")?;
            let (f_i, c_i, fn_i, sz_i, v_i, a_i, sq_i) = (
                col_index(&defs, "File"),
                col_index(&defs, "Component_"),
                col_index(&defs, "FileName"),
                col_index(&defs, "FileSize"),
                col_index(&defs, "Version"),
                col_index(&defs, "Attributes"),
                col_index(&defs, "Sequence"),
            );
            for row in rows {
                let Some(file) = cell_str(&row, f_i) else { continue };
                let Some(component) = cell_str(&row, c_i) else { continue };
                let Some(file_name) = cell_str(&row, fn_i) else { continue };
                let file_size = cell_int(&row, sz_i).and_then(|v| u64::try_from(v).ok()).unwrap_or(0);
                let sequence = cell_int(&row, sq_i).unwrap_or(0);
                files.push(FileRow {
                    file,
                    component,
                    file_name,
                    file_size,
                    version: cell_str(&row, v_i).unwrap_or_default(),
                    attributes: cell_int(&row, a_i).unwrap_or(0),
                    sequence,
                });
            }
        }

        // Media
        let mut media = Vec::new();
        {
            let (defs, rows) = load("Media")?;
            let (id_i, ls_i, cab_i, src_i) = (
                col_index(&defs, "DiskId"),
                col_index(&defs, "LastSequence"),
                col_index(&defs, "Cabinet"),
                col_index(&defs, "Source"),
            );
            for row in rows {
                let Some(disk_id) = cell_int(&row, id_i) else { continue };
                let Some(last_sequence) = cell_int(&row, ls_i) else { continue };
                media.push(MediaRow {
                    disk_id,
                    last_sequence,
                    cabinet: cell_str(&row, cab_i),
                    source: cell_str(&row, src_i).unwrap_or_default(),
                });
            }
            media.sort_by_key(|m| (m.last_sequence, m.disk_id));
        }

        // Shortcut
        let mut shortcuts = Vec::new();
        {
            let (defs, rows) = load("Shortcut")?;
            let (s_i, d_i, n_i, c_i, t_i, a_i, desc_i, ic_i, ii_i, wk_i) = (
                col_index(&defs, "Shortcut"),
                col_index(&defs, "Directory_"),
                col_index(&defs, "Name"),
                col_index(&defs, "Component_"),
                col_index(&defs, "Target"),
                col_index(&defs, "Arguments"),
                col_index(&defs, "Description"),
                col_index(&defs, "Icon_"),
                col_index(&defs, "IconIndex"),
                col_index(&defs, "WkDir"),
            );
            for row in rows {
                let Some(shortcut) = cell_str(&row, s_i) else { continue };
                let Some(directory) = cell_str(&row, d_i) else { continue };
                let Some(name) = cell_str(&row, n_i) else { continue };
                let Some(component) = cell_str(&row, c_i) else { continue };
                let Some(target) = cell_str(&row, t_i) else { continue };
                shortcuts.push(ShortcutRow {
                    shortcut,
                    directory,
                    name,
                    component,
                    target,
                    arguments: cell_str(&row, a_i).unwrap_or_default(),
                    description: cell_str(&row, desc_i).unwrap_or_default(),
                    icon: cell_str(&row, ic_i),
                    icon_index: cell_int(&row, ii_i).unwrap_or(0),
                    wk_dir: cell_str(&row, wk_i),
                });
            }
        }

        // Registry
        let mut registry = Vec::new();
        {
            let (defs, rows) = load("Registry")?;
            let (r_i, root_i, key_i, name_i, val_i, c_i) = (
                col_index(&defs, "Registry"),
                col_index(&defs, "Root"),
                col_index(&defs, "Key"),
                col_index(&defs, "Name"),
                col_index(&defs, "Value"),
                col_index(&defs, "Component_"),
            );
            for row in rows {
                let Some(id) = cell_str(&row, r_i) else { continue };
                let root = cell_int(&row, root_i).unwrap_or(-1);
                let Some(key) = cell_str(&row, key_i) else { continue };
                let Some(component) = cell_str(&row, c_i) else { continue };
                registry.push(RegistryRow {
                    registry: id,
                    root,
                    key,
                    name: cell_str(&row, name_i),
                    value: cell_str(&row, val_i),
                    component,
                });
            }
        }

        // Icon
        let mut icons = Vec::new();
        {
            let (defs, rows) = load("Icon")?;
            let n_i = col_index(&defs, "Name");
            for row in rows {
                if let Some(name) = cell_str(&row, n_i) {
                    icons.push(name);
                }
            }
        }

        Ok(MsiDatabase {
            cfb,
            summary,
            tables,
            columns,
            properties,
            directories,
            components,
            files,
            media,
            shortcuts,
            registry,
            icons,
        })
    }

    /// Look up a `Property` value.
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties.get(name).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// Whether this package targets per-machine installation (`ALLUSERS=1` or `2`).
    pub fn is_all_users(&self) -> bool {
        matches!(self.property("ALLUSERS"), Some("1" | "2"))
    }

    /// Target architecture bits from Summary Information, if known.
    pub fn bits(&self) -> Option<u8> {
        self.summary.as_ref().and_then(|s| template_bits(&s.template))
    }

    /// Read and decode an `Icon` table entry (stored in CFB stream `Icon.<name>`) as PNG bytes.
    pub fn read_icon_png(&self, icon_name: &str, icon_index: i32, read: ReadAt<'_>) -> Option<Vec<u8>> {
        let stream_name = alloc::format!("Icon.{icon_name}");
        let entry = self.cfb.find_stream(&stream_name)?;
        let bytes = self.cfb.read_stream(entry, read, MAX_ICON_BYTES).ok()?;
        icon_bytes_to_png(&bytes, icon_index)
    }

    /// Read the package's product icon (`ARPPRODUCTICON`, else first shortcut's
    /// icon, else first `Icon` table entry) as PNG bytes.
    pub fn product_icon_png(&self, read: ReadAt<'_>) -> Option<Vec<u8>> {
        if let Some(name) = self.property("ARPPRODUCTICON")
            && let Some(png) = self.read_icon_png(name, 0, read)
        {
            return Some(png);
        }
        for sc in &self.shortcuts {
            if let Some(name) = &sc.icon
                && let Some(png) = self.read_icon_png(name, sc.icon_index, read)
            {
                return Some(png);
            }
        }
        for name in &self.icons {
            if let Some(png) = self.read_icon_png(name, 0, read) {
                return Some(png);
            }
        }
        None
    }

    /// Build [`MsiInfo`] for display in Poly's installer window.
    pub fn info(&self, file_name: &str, size: u64, read: ReadAt<'_>) -> MsiInfo {
        let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        let stem = match stem.len().checked_sub(4) {
            Some(cut) if stem.is_char_boundary(cut) && stem[cut..].eq_ignore_ascii_case(".msi") => &stem[..cut],
            _ => stem,
        };
        let name = self
            .property("ProductName")
            .or_else(|| self.summary.as_ref().map(|s| s.subject.trim()).filter(|s| !s.is_empty()))
            .unwrap_or(stem)
            .to_string();
        let version = self.property("ProductVersion").unwrap_or("").to_string();
        let manufacturer = self
            .property("Manufacturer")
            .or_else(|| self.summary.as_ref().map(|s| s.author.trim()).filter(|s| !s.is_empty()))
            .unwrap_or("")
            .to_string();
        let summary = self
            .property("ARPCOMMENTS")
            .or_else(|| self.summary.as_ref().map(|s| s.comments.trim()).filter(|s| !s.is_empty()))
            .or_else(|| self.summary.as_ref().map(|s| s.subject.trim()).filter(|s| !s.is_empty()))
            .filter(|s| *s != name)
            .unwrap_or(&manufacturer)
            .to_string();
        let product_code = self.property("ProductCode").unwrap_or("").to_string();
        let installed_size = self.files.iter().fold(0u64, |acc, f| acc.saturating_add(f.file_size));
        MsiInfo {
            name,
            version,
            manufacturer,
            summary,
            product_code,
            size,
            installed_size,
            file_count: self.files.len(),
            bits: self.bits(),
            icon: self.product_icon_png(read),
        }
    }

    /// Resolve every `Directory` entry to a safe path under Poly's `C:` root.
    pub fn resolve_directories(&self) -> Result<BTreeMap<String, ResolvedDir>, Error> {
        let all_users = self.is_all_users();
        let by_id: BTreeMap<&str, &DirectoryRow> = self.directories.iter().map(|d| (d.directory.as_str(), d)).collect();
        let mut cache: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for id in by_id.keys() {
            let mut visiting = BTreeSet::new();
            resolve_dir_segments(id, &by_id, all_users, &mut visiting, &mut cache)?;
        }
        let mut out = BTreeMap::new();
        for (id, segs) in cache {
            out.insert(id, ResolvedDir::from_segments(&segs));
        }
        Ok(out)
    }

    /// Build the full installation plan: resolved files, media sources, and
    /// launcher shortcuts.
    pub fn plan_install(&self) -> Result<InstallPlan, Error> {
        let dirs = self.resolve_directories()?;
        let comp_map: BTreeMap<&str, &ComponentRow> =
            self.components.iter().map(|c| (c.component.as_str(), c)).collect();

        let mut planned_files = Vec::with_capacity(self.files.len());
        let mut file_paths: BTreeMap<String, (String, String)> = BTreeMap::new(); // File key -> (vfs, dos)

        for f in &self.files {
            let comp = comp_map.get(f.component.as_str()).ok_or(Error::Corrupt)?;
            let dir = dirs.get(&comp.directory).ok_or(Error::Corrupt)?;
            let long_name = parse_short_long(&f.file_name);
            validate_segment(long_name)?;

            let mut segs = dir.segments.clone();
            segs.push(long_name.to_string());
            let vfs_path = join_vfs(&segs);
            let dos_path = join_dos(&segs);

            let media_row = self.media_for_sequence(f.sequence);
            let source = match media_row.and_then(|m| m.cabinet.as_deref()) {
                Some(cab) if !cab.is_empty() => {
                    if let Some(stream) = cab.strip_prefix('#') {
                        if stream.is_empty() {
                            return Err(Error::Corrupt);
                        }
                        FileSource::EmbeddedCab(stream.to_string())
                    } else {
                        validate_segment(cab)?;
                        FileSource::ExternalCab(cab.to_string())
                    }
                }
                _ => {
                    // Uncompressed loose file next to the `.msi`.
                    let mut source_segs = self.resolve_source_dir(&comp.directory)?;
                    source_segs.push(long_name.to_string());
                    FileSource::Loose { rel_path: source_segs.join("/"), file_name: long_name.to_string() }
                }
            };

            file_paths.insert(f.file.clone(), (vfs_path.clone(), dos_path.clone()));
            planned_files.push(PlannedFile {
                file_key: f.file.clone(),
                component: f.component.clone(),
                vfs_path,
                dos_path,
                file_size: f.file_size,
                source,
            });
        }

        // Resolve shortcuts into launcher candidates.
        let mut shortcuts = Vec::new();
        for sc in &self.shortcuts {
            let sc_name = parse_short_long(&sc.name).trim();
            if sc_name.is_empty() || sc_name.to_ascii_lowercase().starts_with("uninstall") {
                continue;
            }
            let target = self.resolve_shortcut_target(sc, &comp_map, &dirs, &file_paths);
            let Some((target_vfs, target_dos)) = target else { continue };
            if !is_exe_path(&target_dos) {
                continue;
            }
            let args_text = self.format_text(&sc.arguments, &dirs, &file_paths);
            let mut argv = alloc::vec![target_vfs.clone()];
            argv.extend(winapps::split_command_line(&winapps::expand(&args_text)));
            let working_dir = sc
                .wk_dir
                .as_deref()
                .and_then(|d| dirs.get(d))
                .map(|d| d.vfs_path.clone())
                .or_else(|| target_vfs.rsplit_once('/').map(|(d, _)| d.to_string()));
            let description = self.format_text(&sc.description, &dirs, &file_paths);
            shortcuts.push(PlannedShortcut {
                id: winapps::id_of(sc_name),
                name: sc_name.to_string(),
                description,
                argv,
                target_vfs,
                target_dos,
                working_dir,
                icon_name: sc.icon.clone(),
                icon_index: sc.icon_index,
            });
        }

        // Fallback: if the MSI defines no `.exe` shortcuts, pick the main installed `.exe`.
        if shortcuts.is_empty()
            && let Some(main_exe) = self.pick_main_exe(&planned_files)
        {
            let app_name = self
                .property("ProductName")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| file_stem_exe(&main_exe.vfs_path));
            let working_dir = main_exe.vfs_path.rsplit_once('/').map(|(d, _)| d.to_string());
            shortcuts.push(PlannedShortcut {
                id: winapps::id_of(app_name),
                name: app_name.to_string(),
                description: self.property("ARPCOMMENTS").unwrap_or("").to_string(),
                argv: alloc::vec![main_exe.vfs_path.clone()],
                target_vfs: main_exe.vfs_path.clone(),
                target_dos: main_exe.dos_path.clone(),
                working_dir,
                icon_name: self.property("ARPPRODUCTICON").map(ToString::to_string),
                icon_index: 0,
            });
        }

        // Deduplicate shortcuts by argv and disambiguate colliding IDs.
        let shortcuts = dedup_shortcuts(shortcuts);

        Ok(InstallPlan { files: planned_files, shortcuts })
    }

    fn media_for_sequence(&self, seq: i32) -> Option<&MediaRow> {
        self.media.iter().find(|m| seq <= m.last_sequence)
    }

    fn resolve_source_dir(&self, dir_id: &str) -> Result<Vec<String>, Error> {
        let by_id: BTreeMap<&str, &DirectoryRow> = self.directories.iter().map(|d| (d.directory.as_str(), d)).collect();
        let mut visiting = BTreeSet::new();
        resolve_source_segments(dir_id, &by_id, &mut visiting)
    }

    fn resolve_shortcut_target(
        &self,
        sc: &ShortcutRow,
        comp_map: &BTreeMap<&str, &ComponentRow>,
        dirs: &BTreeMap<String, ResolvedDir>,
        file_paths: &BTreeMap<String, (String, String)>,
    ) -> Option<(String, String)> {
        let raw = sc.target.trim();
        if raw.contains('[') {
            // Non-advertised formatted target: `[#FileKey]`, `[!FileKey]`, `[DIRID]app.exe`, etc.
            if let Some(key) =
                raw.strip_prefix("[#").or_else(|| raw.strip_prefix("[!")).and_then(|s| s.strip_suffix(']'))
            {
                return file_paths.get(key).cloned();
            }
            let formatted = self.format_text(raw, dirs, file_paths);
            let expanded = winapps::expand(&formatted);
            let rest = expanded.strip_prefix("C:").or_else(|| expanded.strip_prefix("c:"))?;
            let mut segs = Vec::new();
            for part in rest.split(['\\', '/']).filter(|p| !p.is_empty() && *p != ".") {
                if validate_segment(part).is_err() {
                    return None;
                }
                segs.push(part.to_string());
            }
            if segs.is_empty() {
                return None;
            }
            return Some((join_vfs(&segs), join_dos(&segs)));
        }
        // Advertised shortcut: `Target` is a Feature name; target is Component.KeyPath (or first .exe in Component).
        let comp = comp_map.get(sc.component.as_str())?;
        if let Some(kp) = &comp.key_path
            && let Some(found) = file_paths.get(kp)
        {
            return Some(found.clone());
        }
        self.files
            .iter()
            .filter(|f| f.component == sc.component)
            .find_map(|f| file_paths.get(&f.file).filter(|(_, dos)| is_exe_path(dos)).cloned())
    }

    fn pick_main_exe<'a>(&self, files: &'a [PlannedFile]) -> Option<&'a PlannedFile> {
        let exes: Vec<&PlannedFile> = files
            .iter()
            .filter(|f| {
                is_exe_path(&f.dos_path) && !file_stem_exe(&f.vfs_path).to_ascii_lowercase().starts_with("unins")
            })
            .collect();
        if let Some(product) = self.property("ProductName") {
            let slug = appimage::slug(product);
            if let Some(m) = exes.iter().find(|f| appimage::slug(file_stem_exe(&f.vfs_path)) == slug) {
                return Some(m);
            }
        }
        exes.first().copied()
    }

    /// Evaluate MSI formatted strings (`[Prop]`, `[#File]`, `[!File]`, `[DIR]`).
    pub fn format_text(
        &self,
        text: &str,
        dirs: &BTreeMap<String, ResolvedDir>,
        file_paths: &BTreeMap<String, (String, String)>,
    ) -> String {
        self.format_depth(text, dirs, file_paths, 0)
    }

    fn format_depth(
        &self,
        text: &str,
        dirs: &BTreeMap<String, ResolvedDir>,
        file_paths: &BTreeMap<String, (String, String)>,
        depth: usize,
    ) -> String {
        if depth >= MAX_FORMAT_DEPTH || !text.contains('[') {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(open) = rest.find('[') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let Some(close) = after.find(']') else {
                out.push_str(&rest[open..]);
                return out;
            };
            let token = &after[..close];
            rest = &after[close + 1..];
            if let Some(file_key) = token.strip_prefix('#').or_else(|| token.strip_prefix('!')) {
                if let Some((_, dos)) = file_paths.get(file_key) {
                    out.push_str(dos);
                }
            } else if let Some(dir) = dirs.get(token) {
                out.push_str(&dir.dos_path);
                if !dir.dos_path.ends_with('\\') {
                    out.push('\\');
                }
            } else if let Some(val) = self.property(token) {
                let expanded = self.format_depth(val, dirs, file_paths, depth + 1);
                out.push_str(&expanded);
            }
        }
        out.push_str(rest);
        out
    }

    /// Extract every planned file from embedded CAB streams, external sibling
    /// CABs, or uncompressed loose sibling files, passing `(vfs_path, bytes)`
    /// to `on_file`.
    pub fn extract_files<E, F>(
        &self,
        plan: &InstallPlan,
        read_msi: ReadAt<'_>,
        mut read_external: E,
        mut on_file: F,
    ) -> Result<Vec<String>, Error>
    where
        E: FnMut(&str) -> Option<Vec<u8>>,
        F: FnMut(&str, &[u8]) -> Result<(), Error>,
    {
        let mut extracted = Vec::with_capacity(plan.files.len());
        let mut by_embedded: BTreeMap<&str, Vec<&PlannedFile>> = BTreeMap::new();
        let mut by_external: BTreeMap<&str, Vec<&PlannedFile>> = BTreeMap::new();
        let mut loose: Vec<&PlannedFile> = Vec::new();

        for pf in &plan.files {
            match &pf.source {
                FileSource::EmbeddedCab(name) => by_embedded.entry(name.as_str()).or_default().push(pf),
                FileSource::ExternalCab(name) => by_external.entry(name.as_str()).or_default().push(pf),
                FileSource::Loose { .. } => loose.push(pf),
            }
        }

        // 1. Embedded CAB streams inside the MSI compound file.
        for (stream_name, group) in by_embedded {
            let entry = self.cfb.find_stream(stream_name).ok_or(Error::Corrupt)?;
            let cab_bytes = self.cfb.read_stream(entry, read_msi, MAX_CAB_BYTES)?;
            extract_cab_group(&cab_bytes, &group, &mut on_file, &mut extracted)?;
        }

        // 2. External CAB files next to the `.msi`.
        for (cab_name, group) in by_external {
            let cab_bytes = read_external(cab_name).ok_or(Error::Truncated)?;
            if cab_bytes.len() > MAX_CAB_BYTES {
                return Err(Error::TooLarge);
            }
            extract_cab_group(&cab_bytes, &group, &mut on_file, &mut extracted)?;
        }

        // 3. Uncompressed loose files next to the `.msi`.
        for pf in loose {
            let FileSource::Loose { rel_path, file_name } = &pf.source else { continue };
            let bytes = read_external(rel_path).or_else(|| read_external(file_name)).ok_or(Error::Truncated)?;
            on_file(&pf.vfs_path, &bytes)?;
            extracted.push(pf.vfs_path.clone());
        }

        Ok(extracted)
    }
}

fn extract_cab_group<F>(
    cab_bytes: &[u8],
    group: &[&PlannedFile],
    on_file: &mut F,
    extracted: &mut Vec<String>,
) -> Result<(), Error>
where
    F: FnMut(&str, &[u8]) -> Result<(), Error>,
{
    let cab = Cabinet::parse(cab_bytes)?;
    let mut pending: BTreeMap<String, Vec<&PlannedFile>> = BTreeMap::new();
    for &pf in group {
        pending.entry(pf.file_key.to_ascii_lowercase()).or_default().push(pf);
    }
    let wanted: BTreeSet<String> = pending.keys().cloned().collect();
    cab.extract_matching(
        |name| wanted.contains(&name.to_ascii_lowercase()),
        |name, bytes| {
            let key = name.to_ascii_lowercase();
            if let Some(targets) = pending.remove(&key) {
                for pf in targets {
                    on_file(&pf.vfs_path, &bytes)?;
                    extracted.push(pf.vfs_path.clone());
                }
            }
            Ok(())
        },
    )?;
    if !pending.is_empty() {
        return Err(Error::Truncated);
    }
    Ok(())
}

/// Inspect an `.msi` package quickly (metadata and icon only; no CABs read).
pub fn inspect(file_name: &str, size: u64, read: ReadAt<'_>) -> Result<MsiInfo, Error> {
    let db = MsiDatabase::open(read)?;
    Ok(db.info(file_name, size, read))
}

/// A resolved directory under Poly's `C:` root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDir {
    /// Path components below `C:\` / `/nt/c`.
    pub segments: Vec<String>,
    /// Poly VFS path (`/nt/c/...`).
    pub vfs_path: String,
    /// Windows path (`C:\...`).
    pub dos_path: String,
}

impl ResolvedDir {
    fn from_segments(segments: &[String]) -> ResolvedDir {
        ResolvedDir { segments: segments.to_vec(), vfs_path: join_vfs(segments), dos_path: join_dos(segments) }
    }
}

/// Where a planned file's bytes come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileSource {
    /// Embedded CAB stream inside the `.msi` compound file (`Media.Cabinet = "#name"`).
    EmbeddedCab(String),
    /// External CAB file next to the `.msi` (`Media.Cabinet = "name.cab"`).
    ExternalCab(String),
    /// Uncompressed loose file next to the `.msi` (`Media.Cabinet` empty).
    Loose { rel_path: String, file_name: String },
}

/// One file resolved from `File` + `Component` + `Directory` + `Media`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedFile {
    pub file_key: String,
    pub component: String,
    pub vfs_path: String,
    pub dos_path: String,
    pub file_size: u64,
    pub source: FileSource,
}

/// One launcher entry planned from `Shortcut` (or the main `.exe` fallback).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedShortcut {
    pub id: String,
    pub name: String,
    pub description: String,
    pub argv: Vec<String>,
    pub target_vfs: String,
    pub target_dos: String,
    pub working_dir: Option<String>,
    pub icon_name: Option<String>,
    pub icon_index: i32,
}

/// The complete installation plan for an `.msi`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallPlan {
    pub files: Vec<PlannedFile>,
    pub shortcuts: Vec<PlannedShortcut>,
}

/// Parse MSI `short|long` name, returning the long name (or the whole string
/// when there is no `|`).
pub fn parse_short_long(name: &str) -> &str {
    match name.split_once('|') {
        Some((_, long)) if !long.is_empty() => long,
        Some((short, _)) => short,
        None => name,
    }
}

/// Parse `DefaultDir` (`target[:source]`), returning the target component's
/// long name (`"."` means same folder as parent).
pub fn parse_default_dir_target(default_dir: &str) -> &str {
    let target = match default_dir.split_once(':') {
        Some((t, _)) => t,
        None => default_dir,
    };
    parse_short_long(target)
}

/// Parse `DefaultDir` (`target[:source]`), returning the source component's
/// long name.
fn parse_default_dir_source(default_dir: &str) -> &str {
    let source = match default_dir.split_once(':') {
        Some((_, s)) if !s.is_empty() => s,
        Some((t, _)) => t,
        None => default_dir,
    };
    parse_short_long(source)
}

/// Validate a single relative path segment: rejects `..`, `.`, empty, path
/// separators, drive/colon syntax, NUL, and ASCII control characters.
pub fn validate_segment(seg: &str) -> Result<(), Error> {
    if seg.is_empty() || seg == "." || seg == ".." {
        return Err(Error::Corrupt);
    }
    if seg.bytes().any(|b| b == 0 || b == b'/' || b == b'\\' || b == b':' || b < 0x20) {
        return Err(Error::Corrupt);
    }
    Ok(())
}

/// Map standard MSI System Folder Properties (Microsoft Learn, "System Folder
/// Properties") to path segments under `C:` matching Poly's NT layout.
pub fn system_folder_segments(id: &str, all_users: bool) -> Option<Vec<String>> {
    let user = winapps::USER;
    let list: Vec<&str> = match id {
        "TARGETDIR" | "WindowsVolume" | "SourceDir" => return Some(Vec::new()),
        "ProgramFiles64Folder" => alloc::vec!["Program Files"],
        "ProgramFilesFolder" => alloc::vec!["Program Files (x86)"],
        "CommonFiles64Folder" => alloc::vec!["Program Files", "Common Files"],
        "CommonFilesFolder" => alloc::vec!["Program Files (x86)", "Common Files"],
        "System64Folder" => alloc::vec!["Windows", "System32"],
        "SystemFolder" => alloc::vec!["Windows", "SysWOW64"],
        "System16Folder" => alloc::vec!["Windows", "System"],
        "WindowsFolder" => alloc::vec!["Windows"],
        "FontsFolder" => alloc::vec!["Windows", "Fonts"],
        "CommonAppDataFolder" => alloc::vec!["ProgramData"],
        "AppDataFolder" => alloc::vec!["Users", user, "AppData", "Roaming"],
        "LocalAppDataFolder" => alloc::vec!["Users", user, "AppData", "Local"],
        "TempFolder" => alloc::vec!["Users", user, "AppData", "Local", "Temp"],
        "PersonalFolder" => alloc::vec!["Users", user, "Documents"],
        "MyPicturesFolder" => alloc::vec!["Users", user, "Pictures"],
        "FavoritesFolder" => alloc::vec!["Users", user, "Favorites"],
        "SendToFolder" => alloc::vec!["Users", user, "AppData", "Roaming", "Microsoft", "Windows", "SendTo"],
        "NetHoodFolder" => {
            alloc::vec!["Users", user, "AppData", "Roaming", "Microsoft", "Windows", "Network Shortcuts"]
        }
        "PrintHoodFolder" => {
            alloc::vec!["Users", user, "AppData", "Roaming", "Microsoft", "Windows", "Printer Shortcuts"]
        }
        "RecentFolder" => alloc::vec!["Users", user, "AppData", "Roaming", "Microsoft", "Windows", "Recent"],
        "TemplateFolder" => alloc::vec!["Users", user, "AppData", "Roaming", "Microsoft", "Windows", "Templates"],
        "DesktopFolder" => {
            if all_users {
                alloc::vec!["Users", "Public", "Desktop"]
            } else {
                alloc::vec!["Users", user, "Desktop"]
            }
        }
        "StartMenuFolder" => {
            if all_users {
                alloc::vec!["ProgramData", "Microsoft", "Windows", "Start Menu"]
            } else {
                alloc::vec!["Users", user, "AppData", "Roaming", "Microsoft", "Windows", "Start Menu"]
            }
        }
        "ProgramMenuFolder" => {
            if all_users {
                alloc::vec!["ProgramData", "Microsoft", "Windows", "Start Menu", "Programs"]
            } else {
                alloc::vec!["Users", user, "AppData", "Roaming", "Microsoft", "Windows", "Start Menu", "Programs"]
            }
        }
        "StartupFolder" => {
            if all_users {
                alloc::vec!["ProgramData", "Microsoft", "Windows", "Start Menu", "Programs", "Startup"]
            } else {
                alloc::vec![
                    "Users",
                    user,
                    "AppData",
                    "Roaming",
                    "Microsoft",
                    "Windows",
                    "Start Menu",
                    "Programs",
                    "Startup"
                ]
            }
        }
        "AdminToolsFolder" => {
            alloc::vec!["ProgramData", "Microsoft", "Windows", "Start Menu", "Programs", "Administrative Tools"]
        }
        _ => return None,
    };
    Some(list.into_iter().map(ToString::to_string).collect())
}

fn resolve_dir_segments(
    id: &str,
    by_id: &BTreeMap<&str, &DirectoryRow>,
    all_users: bool,
    visiting: &mut BTreeSet<String>,
    cache: &mut BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>, Error> {
    if let Some(existing) = cache.get(id) {
        return Ok(existing.clone());
    }
    if visiting.len() >= MAX_DIR_DEPTH || !visiting.insert(id.to_string()) {
        return Err(Error::Corrupt);
    }
    let resolved = if let Some(sys) = system_folder_segments(id, all_users) {
        sys
    } else {
        let row = by_id.get(id).ok_or(Error::Corrupt)?;
        if row.default_dir.contains(":\\") || row.default_dir.contains(":/") {
            return Err(Error::Corrupt);
        }
        let mut segs = match row.parent.as_deref() {
            Some(p) if !p.is_empty() && p != id => resolve_dir_segments(p, by_id, all_users, visiting, cache)?,
            _ => Vec::new(),
        };
        let target = parse_default_dir_target(&row.default_dir);
        if target != "." && !target.is_empty() {
            // Reject absolute prefixes or traversal even if separators appear.
            if target.starts_with('/') || target.starts_with('\\') || target.contains(':') {
                return Err(Error::Corrupt);
            }
            for part in target.split(['\\', '/']) {
                if part == "." {
                    continue;
                }
                validate_segment(part)?;
                segs.push(part.to_string());
            }
        }
        segs
    };
    visiting.remove(id);
    cache.insert(id.to_string(), resolved.clone());
    Ok(resolved)
}

fn resolve_source_segments(
    id: &str,
    by_id: &BTreeMap<&str, &DirectoryRow>,
    visiting: &mut BTreeSet<String>,
) -> Result<Vec<String>, Error> {
    if visiting.len() >= MAX_DIR_DEPTH || !visiting.insert(id.to_string()) {
        return Err(Error::Corrupt);
    }
    let Some(row) = by_id.get(id) else {
        visiting.remove(id);
        return Ok(Vec::new());
    };
    let mut segs = match row.parent.as_deref() {
        Some(p) if !p.is_empty() && p != id => resolve_source_segments(p, by_id, visiting)?,
        _ => Vec::new(),
    };
    let src = parse_default_dir_source(&row.default_dir);
    if src != "." && !src.is_empty() && !src.eq_ignore_ascii_case("SourceDir") {
        if src.starts_with('/') || src.starts_with('\\') || src.contains(':') {
            return Err(Error::Corrupt);
        }
        for part in src.split(['\\', '/']) {
            if part == "." {
                continue;
            }
            validate_segment(part)?;
            segs.push(part.to_string());
        }
    }
    visiting.remove(id);
    Ok(segs)
}

fn join_vfs(segments: &[String]) -> String {
    if segments.is_empty() {
        winapps::C_ROOT.to_string()
    } else {
        alloc::format!("{}/{}", winapps::C_ROOT, segments.join("/"))
    }
}

fn join_dos(segments: &[String]) -> String {
    if segments.is_empty() { "C:\\".to_string() } else { alloc::format!("C:\\{}", segments.join("\\")) }
}

fn is_exe_path(path: &str) -> bool {
    path.len() > 4 && path.is_char_boundary(path.len() - 4) && path[path.len() - 4..].eq_ignore_ascii_case(".exe")
}

fn file_stem_exe(path: &str) -> &str {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if is_exe_path(file) { &file[..file.len() - 4] } else { file }
}

fn dedup_shortcuts(entries: Vec<PlannedShortcut>) -> Vec<PlannedShortcut> {
    let mut out: Vec<PlannedShortcut> = Vec::new();
    for mut sc in entries {
        if out.iter().any(|o| o.argv == sc.argv) {
            continue;
        }
        let mut n = 2;
        let base = sc.id.trim_end_matches(".desktop").to_string();
        while out.iter().any(|o| o.id == sc.id) {
            sc.id = alloc::format!("{base}-{n}.desktop");
            n += 1;
        }
        out.push(sc);
    }
    out
}

/// Convert raw bytes from an MSI `Icon` stream (`.ico`, PNG, or PE `.exe`/`.dll`)
/// into PNG bytes for Poly's launcher.
pub fn icon_bytes_to_png(bytes: &[u8], icon_index: i32) -> Option<Vec<u8>> {
    if bytes.starts_with(PNG_SIGNATURE) {
        return Some(bytes.to_vec());
    }
    // PE executable / DLL icon stream.
    if bytes.starts_with(b"MZ") {
        let mut reader = |at: u64, buf: &mut [u8]| -> usize {
            let at = at as usize;
            if at >= bytes.len() {
                return 0;
            }
            let n = buf.len().min(bytes.len() - at);
            buf[..n].copy_from_slice(&bytes[at..at + n]);
            n
        };
        let mut image = program::Image::open(&mut reader).ok()?;
        return image.icon(icon_index);
    }
    // Windows ICO file (ICONDIR + ICONDIRENTRY array).
    if bytes.len() >= 6
        && u16::from_le_bytes([bytes[0], bytes[1]]) == 0
        && u16::from_le_bytes([bytes[2], bytes[3]]) == 1
    {
        let count = usize::from(u16::from_le_bytes([bytes[4], bytes[5]])).min(64);
        let mut best: Option<((u32, bool, u16), Vec<u8>)> = None;
        for i in 0..count {
            let Some(e) = bytes.get(6 + i * 16..6 + i * 16 + 16) else { break };
            let width = if e[0] == 0 { 256 } else { u32::from(e[0]) };
            let bits = u16::from_le_bytes([e[6], e[7]]);
            let size = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
            let Some(img) = bytes.get(off..off.checked_add(size)?) else { continue };
            let (key, png) = if img.starts_with(PNG_SIGNATURE) {
                ((width, true, bits), img.to_vec())
            } else {
                match program::dib_to_png(img) {
                    Some(png) => ((width, false, bits), png),
                    None => continue,
                }
            };
            if best.as_ref().is_none_or(|(k, _)| key > *k) {
                best = Some((key, png));
            }
        }
        return best.map(|(_, png)| png);
    }
    None
}

/// Generate a launcher `.desktop` file for an MSI-installed shortcut.
pub fn desktop_file(sc: &PlannedShortcut, icon: Option<&str>, msi_slug: &str, info: &MsiInfo) -> String {
    let clean = |v: &str| v.replace(['\n', '\r'], " ");
    let mut out = String::from("[Desktop Entry]\nType=Application\n");
    out.push_str(&alloc::format!("Name={}\n", clean(&sc.name)));
    let comment = if !sc.description.is_empty() {
        clean(&sc.description)
    } else if !info.summary.is_empty() {
        clean(&info.summary)
    } else {
        "A Windows app".to_string()
    };
    out.push_str(&alloc::format!("Comment={comment}\n"));
    let exec: Vec<String> = sc.argv.iter().map(|a| winapps::exec_arg(a)).collect();
    out.push_str(&alloc::format!("Exec={}\n", exec.join(" ")));
    if let Some(dir) = &sc.working_dir {
        out.push_str(&alloc::format!("Path={dir}\n"));
    }
    if let Some(icon) = icon {
        out.push_str(&alloc::format!("Icon={icon}\n"));
    }
    out.push_str("Keywords=windows;\n");
    out.push_str(&alloc::format!("{}={}\n", winapps::TARGET_KEY, clean(&sc.target_dos)));
    out.push_str(&alloc::format!("{MSI_ID_KEY}={}\n", clean(msi_slug)));
    if !info.manufacturer.is_empty() {
        out.push_str(&alloc::format!("{}={}\n", winapps::PUBLISHER_KEY, clean(&info.manufacturer)));
    }
    if !info.version.is_empty() {
        out.push_str(&alloc::format!("{}={}\n", winapps::VERSION_KEY, clean(&info.version)));
    }
    out
}

/// Where an installed MSI's manifest lives under `data_home`.
pub fn manifest_path(data_home: &str, slug: &str) -> String {
    alloc::format!("{}/poly/msi/{slug}.manifest", data_home.trim_end_matches('/'))
}

/// Whether `path` is a safe file or directory path strictly inside Poly's `C:` root.
pub fn is_safe_install_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix(winapps::C_ROOT).and_then(|r| r.strip_prefix('/')) else {
        return false;
    };
    if rest.is_empty() || path.contains('\0') {
        return false;
    }
    rest.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Whether `dir` is safe to remove (if empty) during MSI uninstallation:
/// strictly under `/nt/c/...` and never a shared Windows system folder.
pub fn is_removable_dir(dir: &str) -> bool {
    if !is_safe_install_path(dir) {
        return false;
    }
    let rel = dir.trim_start_matches(winapps::C_ROOT).trim_start_matches('/').to_ascii_lowercase();
    let user_lower = winapps::USER.to_ascii_lowercase();
    let protected = [
        "program files",
        "program files (x86)",
        "program files/common files",
        "program files (x86)/common files",
        "windows",
        "windows/system32",
        "windows/syswow64",
        "windows/system",
        "windows/fonts",
        "programdata",
        "programdata/microsoft",
        "programdata/microsoft/windows",
        "programdata/microsoft/windows/start menu",
        "programdata/microsoft/windows/start menu/programs",
        "users",
        "users/public",
        "users/public/desktop",
    ];
    if protected.contains(&rel.as_str()) {
        return false;
    }
    let user_prefix = alloc::format!("users/{user_lower}");
    let user_protected = [
        user_prefix.clone(),
        alloc::format!("{user_prefix}/desktop"),
        alloc::format!("{user_prefix}/documents"),
        alloc::format!("{user_prefix}/appdata"),
        alloc::format!("{user_prefix}/appdata/local"),
        alloc::format!("{user_prefix}/appdata/local/temp"),
        alloc::format!("{user_prefix}/appdata/roaming"),
        alloc::format!("{user_prefix}/appdata/roaming/microsoft"),
        alloc::format!("{user_prefix}/appdata/roaming/microsoft/windows"),
        alloc::format!("{user_prefix}/appdata/roaming/microsoft/windows/start menu"),
        alloc::format!("{user_prefix}/appdata/roaming/microsoft/windows/start menu/programs"),
    ];
    !user_protected.iter().any(|p| p == &rel)
}

/// Uninstall record persisted when an `.msi` package is installed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MsiManifest {
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub product_code: String,
    pub size: u64,
    /// Desktop file IDs (`poly-windows-*.desktop`).
    pub desktops: Vec<String>,
    /// Written PNG icon paths.
    pub icons: Vec<String>,
    /// Installed file paths under `/nt/c/...`.
    pub files: Vec<String>,
    /// Newly created directories under `/nt/c/...` (removed if empty on uninstall).
    pub dirs: Vec<String>,
}

impl MsiManifest {
    /// Serialize the manifest as a simple line-oriented `Key=Value` file.
    pub fn encode(&self) -> String {
        let clean = |v: &str| v.replace(['\n', '\r'], " ");
        let mut out = String::new();
        out.push_str(&alloc::format!("Name={}\n", clean(&self.name)));
        if !self.version.is_empty() {
            out.push_str(&alloc::format!("Version={}\n", clean(&self.version)));
        }
        if !self.publisher.is_empty() {
            out.push_str(&alloc::format!("Publisher={}\n", clean(&self.publisher)));
        }
        if !self.product_code.is_empty() {
            out.push_str(&alloc::format!("ProductCode={}\n", clean(&self.product_code)));
        }
        out.push_str(&alloc::format!("Size={}\n", self.size));
        for d in &self.desktops {
            out.push_str(&alloc::format!("Desktop={}\n", clean(d)));
        }
        for ic in &self.icons {
            out.push_str(&alloc::format!("Icon={}\n", clean(ic)));
        }
        for f in &self.files {
            out.push_str(&alloc::format!("File={}\n", clean(f)));
        }
        for dir in &self.dirs {
            out.push_str(&alloc::format!("Dir={}\n", clean(dir)));
        }
        out
    }

    /// Parse a manifest file written by [`MsiManifest::encode`].
    pub fn parse(text: &str) -> Option<MsiManifest> {
        let mut m = MsiManifest::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "Name" => m.name = v.to_string(),
                "Version" => m.version = v.to_string(),
                "Publisher" => m.publisher = v.to_string(),
                "ProductCode" => m.product_code = v.to_string(),
                "Size" => m.size = v.parse().unwrap_or(0),
                "Desktop" if winapps::is_windows_id(v) => m.desktops.push(v.to_string()),
                "Icon" if !v.is_empty() => m.icons.push(v.to_string()),
                "File" if is_safe_install_path(v) => m.files.push(v.to_string()),
                "Dir" if is_removable_dir(v) => m.dirs.push(v.to_string()),
                _ => {}
            }
        }
        (!m.name.is_empty()).then_some(m)
    }
}
