//! The target's item ID list (`LinkTargetIDList`, MS-SHLLINK 2.2): what Save writes for a drive
//! path and how a list is read back into a path.
//!
//! A list is a run of items, each a `USHORT` size that counts itself and the item's bytes, ended
//! by a size of 0 (`shtypes.h`, `SHITEMID`). For `X:\dir\file` the sheet's `idlist` rule gives:
//!
//! - item type 0x1F, 20 bytes: the computer (`CLSID_MyComputer` {20D04FE0-3AEA-1069-A2D8-08002B30309D},
//!   `shlguid.h`);
//! - item type 0x2F, 25 bytes: the drive, `X:\` as ANSI text at offset 3;
//! - one file-entry item per component: type 0x31 for a directory, 0x32 for a file. Poly's layout
//!   (the sheet fixes only the name and the extension block): size (2), type (1), 0 (1), file
//!   size (4), DOS date (2), DOS time (2), attributes (2), the name as ANSI text with its NUL
//!   (padded to an even length), then the extension block: its size (2), version 9 (2), the
//!   signature 0xBEEF0004 (4), 38 bytes of times and reserved zeros, the name in UTF-16 with its
//!   NUL, and the offset of the block in the item (2). An item is so
//!   `14 + even(len + 1) + 48 + 2 * (len + 1)` bytes, the sizes the sheet lists.
//!
//! Poly's other shell functions (`ILCreateFromPathW`, `SHParseDisplayName`) make lists of one item
//! holding the path in UTF-16; [`path_of`] reads both.

use crate::{FileInfo, Guid, Text, text};
use alloc::vec::Vec;

/// `CLSID_MyComputer` (`shlguid.h`): {20D04FE0-3AEA-1069-A2D8-08002B30309D}.
pub const CLSID_MY_COMPUTER: Guid =
    Guid { data1: 0x20D0_4FE0, data2: 0x3AEA, data3: 0x1069, data4: [0xA2, 0xD8, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D] };

/// The item types the list of a drive path uses.
pub const ITEM_ROOT: u8 = 0x1F;
pub const ITEM_DRIVE: u8 = 0x2F;
pub const ITEM_FOLDER: u8 = 0x31;
pub const ITEM_FILE: u8 = 0x32;
/// The signature of the extension block of a file-entry item.
pub const EXTENSION_SIGNATURE: u32 = 0xBEEF_0004;

/// The size of the root (computer) item and of the drive item.
pub const ROOT_ITEM_SIZE: usize = 20;
pub const DRIVE_ITEM_SIZE: usize = 25;

/// The 16 bytes of a GUID as it is in memory (`guiddef.h`).
pub fn guid_bytes(g: &Guid) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&g.data1.to_le_bytes());
    b[4..6].copy_from_slice(&g.data2.to_le_bytes());
    b[6..8].copy_from_slice(&g.data3.to_le_bytes());
    b[8..16].copy_from_slice(&g.data4);
    b
}

/// A GUID from its 16 bytes in memory.
pub fn guid_from(b: &[u8]) -> Option<Guid> {
    let b: &[u8; 16] = b.get(..16)?.try_into().ok()?;
    Some(Guid {
        data1: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        data2: u16::from_le_bytes([b[4], b[5]]),
        data3: u16::from_le_bytes([b[6], b[7]]),
        data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
    })
}

/// A `FILETIME` (100 ns since 1601) as the DOS date and time of a file-entry item (0 for 0).
fn dos_date_time(filetime: u64) -> (u16, u16) {
    if filetime == 0 {
        return (0, 0);
    }
    // Days since 1601-01-01 and the time of day, then the civil date (proleptic Gregorian).
    let secs = filetime / 10_000_000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // 1601-01-01 is 134 774 days before 1970-01-01, which is 719 468 days after 0000-03-01.
    let z = days as i64 - 134_774 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    if !(1980..=2107).contains(&year) {
        return (0, 0);
    }
    let date = (((year - 1980) as u16) << 9) | ((month as u16) << 5) | day as u16;
    let time = (((rem / 3600) as u16) << 11) | ((((rem % 3600) / 60) as u16) << 5) | ((rem % 60) / 2) as u16;
    (date, time)
}

/// One file-entry item (see the module's description).
pub fn file_entry_item(name: &[u16], folder: bool, info: &FileInfo) -> Vec<u8> {
    let ansi = text::to_ansi(name);
    let ansi_len = (ansi.len() + 1 + 1) & !1;
    let block_len = 48 + 2 * (name.len() + 1);
    let size = 14 + ansi_len + block_len;
    let mut v = Vec::with_capacity(size);
    v.extend_from_slice(&(size as u16).to_le_bytes());
    v.push(if folder { ITEM_FOLDER } else { ITEM_FILE });
    v.push(0);
    v.extend_from_slice(&(if folder { 0 } else { info.size as u32 }).to_le_bytes());
    let (date, time) = dos_date_time(info.write);
    v.extend_from_slice(&date.to_le_bytes());
    v.extend_from_slice(&time.to_le_bytes());
    let attributes = if folder { crate::FILE_ATTRIBUTE_DIRECTORY } else { info.attributes };
    v.extend_from_slice(&(attributes as u16).to_le_bytes());
    v.extend_from_slice(&ansi);
    v.resize(14 + ansi_len, 0);
    let block_at = v.len();
    v.extend_from_slice(&(block_len as u16).to_le_bytes());
    v.extend_from_slice(&9u16.to_le_bytes());
    v.extend_from_slice(&EXTENSION_SIGNATURE.to_le_bytes());
    let (cdate, ctime) = dos_date_time(info.creation);
    let (adate, atime) = dos_date_time(info.access);
    for x in [cdate, ctime, adate, atime] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.resize(block_at + 46, 0);
    for &c in name {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v.extend_from_slice(&[0, 0]);
    v.extend_from_slice(&(block_at as u16).to_le_bytes());
    debug_assert_eq!(v.len(), size);
    v
}

/// What the list of a drive path needs to know of its components: for each, whether it is a
/// directory and its data (the last one is the target).
#[derive(Debug)]
pub struct Component<'a> {
    pub name: &'a [u16],
    pub folder: bool,
    pub info: FileInfo,
}

/// The item list (without its `IDListSize`, with the terminating 0) of a drive path; the sizes of
/// the items come with it (for the special-folder blocks' offsets).
pub fn build(drive: u16, parts: &[Component]) -> (Vec<u8>, Vec<usize>) {
    let mut v = Vec::new();
    let mut sizes = Vec::new();
    v.extend_from_slice(&(ROOT_ITEM_SIZE as u16).to_le_bytes());
    v.push(ITEM_ROOT);
    v.push(0x50);
    v.extend_from_slice(&guid_bytes(&CLSID_MY_COMPUTER));
    sizes.push(ROOT_ITEM_SIZE);
    v.extend_from_slice(&(DRIVE_ITEM_SIZE as u16).to_le_bytes());
    v.push(ITEM_DRIVE);
    v.extend_from_slice(&[drive as u8, b':', b'\\']);
    v.resize(ROOT_ITEM_SIZE + DRIVE_ITEM_SIZE, 0);
    sizes.push(DRIVE_ITEM_SIZE);
    for p in parts {
        let item = file_entry_item(p.name, p.folder, &p.info);
        sizes.push(item.len());
        v.extend_from_slice(&item);
    }
    v.extend_from_slice(&[0, 0]);
    (v, sizes)
}

/// The items of a list (each item's bytes with its size field), up to the terminator or the end
/// of `data` (a list cut short gives the items that are whole).
pub fn items(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 2 <= data.len() {
        let size = usize::from(u16::from_le_bytes([data[at], data[at + 1]]));
        if size < 2 || at + size > data.len() {
            break;
        }
        out.push(&data[at..at + size]);
        at += size;
    }
    out
}

/// The long name of a file-entry item: the UTF-16 name before the block offset at the end of the
/// item when the item has an extension block 0xBEEF0004, else the ANSI name at offset 14.
pub fn item_name(item: &[u8]) -> Text {
    if let Some(name) = extension_name(item) {
        return name;
    }
    let ansi = item.get(14..).unwrap_or(&[]);
    let end = ansi.iter().position(|&b| b == 0).unwrap_or(ansi.len());
    text::from_ansi(&ansi[..end])
}

fn extension_name(item: &[u8]) -> Option<Text> {
    let n = item.len();
    let block = usize::from(u16::from_le_bytes([*item.get(n.checked_sub(2)?)?, item[n - 1]]));
    let sig = item.get(block + 4..block + 8)?;
    if u32::from_le_bytes([sig[0], sig[1], sig[2], sig[3]]) != EXTENSION_SIGNATURE || block < 14 {
        return None;
    }
    // The name ends with its NUL right before the offset; walk back to the NUL before it.
    let end = n - 2;
    if end < block + 10 || item[end - 2..end] != [0, 0] {
        return None;
    }
    let mut start = end - 2;
    while start >= block + 8 + 2 && item[start - 2..start] != [0, 0] {
        start -= 2;
    }
    let name: Text = item[start..end - 2].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    (!name.is_empty()).then_some(name)
}

/// The path a list names: a drive list (the computer, the drive, file-entry items) spelled with
/// the items' long names, or one of Poly's one-item lists holding a path in UTF-16. `None` for a
/// list that names no file system path.
pub fn path_of(data: &[u8]) -> Option<Text> {
    let items = items(data);
    let first = items.first()?;
    if first.get(2) == Some(&ITEM_ROOT) {
        let mut out: Text = Vec::new();
        for item in &items[1..] {
            match item.get(2).copied()? {
                t if t & 0x70 == 0x20 => {
                    // A drive item: `X:\` at offset 3.
                    let letter = text::drive_letter(&[u16::from(*item.get(3)?), u16::from(*item.get(4)?)])?;
                    out = alloc::vec![letter, u16::from(b':'), u16::from(b'\\')];
                }
                t if t & 0x70 == 0x30 => {
                    if out.is_empty() {
                        return None;
                    }
                    if out.last() != Some(&u16::from(b'\\')) {
                        out.push(u16::from(b'\\'));
                    }
                    out.extend(item_name(item));
                }
                _ => return None,
            }
        }
        return (!out.is_empty()).then_some(out);
    }
    // Poly's own list: one item, the path in UTF-16.
    if items.len() != 1 || first.len() < 4 || first.len() % 2 != 0 {
        return None;
    }
    let path: Text = first[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    (text::is_drive_path(&path) || text::is_unc(&path)).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::wide;

    fn sizes(path: &str) -> Vec<usize> {
        let p = wide(path);
        let parts: Vec<Component> = p[3..]
            .split(|&c| c == u16::from(b'\\'))
            .map(|name| Component { name, folder: true, info: FileInfo::default() })
            .collect();
        build(p[0], &parts).1
    }

    #[test]
    fn item_sizes_are_the_sheets() {
        assert_eq!(sizes("C:\\Windows\\System32\\kernel32.dll"), [20, 25, 86, 90, 102]);
        assert_eq!(sizes("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"), [20, 25, 104, 84, 84, 98, 96]);
        let p = wide("C:\\Windows\\System32\\kernel32.dll");
        let parts: Vec<Component> = p[3..]
            .split(|&c| c == u16::from(b'\\'))
            .map(|name| Component { name, folder: false, info: FileInfo::default() })
            .collect();
        // 323 bytes of items and the terminator: the sheet's IDList size 325.
        assert_eq!(build(p[0], &parts).0.len(), 325);
    }

    #[test]
    fn the_name_is_at_offset_14_and_in_the_extension_block() {
        let item = file_entry_item(&wide("Program Files"), true, &FileInfo::default());
        assert_eq!(&item[14..27], b"Program Files");
        assert_eq!(item[2], ITEM_FOLDER);
        assert!(item.windows(4).any(|w| w == [0x04, 0x00, 0xEF, 0xBE]));
        assert_eq!(item_name(&item), wide("Program Files"));
    }

    #[test]
    fn a_list_reads_back_as_its_path() {
        let p = wide("C:\\Program Files\\Google\\chrome.exe");
        let parts: Vec<Component> = p[3..]
            .split(|&c| c == u16::from(b'\\'))
            .map(|name| Component { name, folder: false, info: FileInfo::default() })
            .collect();
        assert_eq!(path_of(&build(p[0], &parts).0), Some(p));
    }

    #[test]
    fn items_without_the_extension_block_give_the_ansi_names() {
        // As pf_built builds them: size, type, 0, size, date, time, attributes, name, NUL, pad.
        let mut list = Vec::new();
        list.extend_from_slice(&20u16.to_le_bytes());
        list.push(0x1F);
        list.push(0x50);
        list.extend_from_slice(&guid_bytes(&CLSID_MY_COMPUTER));
        list.extend_from_slice(&25u16.to_le_bytes());
        list.push(0x2F);
        list.extend_from_slice(b"C:\\");
        list.resize(45, 0);
        for (name, ty) in [("Windows", 0x31u8), ("ntdll.dll", 0x32)] {
            let len = (14 + name.len() + 1 + 1) & !1;
            list.extend_from_slice(&(len as u16).to_le_bytes());
            list.push(ty);
            list.push(0);
            list.extend_from_slice(&[0, 0, 0, 0, 0x21, 0x5A, 0, 0x60, 0x10, 0]);
            list.extend_from_slice(name.as_bytes());
            list.push(0);
            if (14 + name.len() + 1) % 2 == 1 {
                list.push(0);
            }
        }
        list.extend_from_slice(&[0, 0]);
        assert_eq!(path_of(&list), Some(wide("C:\\Windows\\ntdll.dll")));
        // Cut inside the last item: the whole items are read.
        assert_eq!(path_of(&list[..70]), Some(wide("C:\\Windows")));
    }

    #[test]
    fn polys_one_item_lists_are_read_too() {
        let p = wide("C:\\Windows\\notepad.exe");
        let mut list = Vec::new();
        list.extend_from_slice(&((2 + 2 * p.len()) as u16).to_le_bytes());
        for c in &p {
            list.extend_from_slice(&c.to_le_bytes());
        }
        list.extend_from_slice(&[0, 0]);
        assert_eq!(path_of(&list), Some(p));
        assert_eq!(path_of(&[0, 0]), None);
    }

    #[test]
    fn dos_times() {
        // 2000-02-29 12:34:56 UTC as a FILETIME (Unix time 951 827 696).
        let ft = (951_827_696u64 + 11_644_473_600) * 10_000_000;
        let (d, t) = dos_date_time(ft);
        assert_eq!(((d >> 9) + 1980, (d >> 5) & 15, d & 31), (2000, 2, 29));
        assert_eq!((t >> 11, (t >> 5) & 63, (t & 31) * 2), (12, 34, 56));
        assert_eq!(dos_date_time(0), (0, 0));
    }
}
