//! Inline data (`inline_data`, "ext4 Data Structures and Algorithms",
//! "Inline Data"), read-only: a small file's bytes live in the inode, the
//! first 60 in `i_block`, the rest in the value of the extended attribute
//! `system.data` in the inode's extra space. An inline directory keeps its
//! parent's inode number in the first 4 bytes of `i_block` and directory
//! records (without `.` and `..`) in the rest and in the attribute.
//!
//! In-inode extended attributes (ext4 docs, "Extended Attributes"): after
//! the `i_extra_isize` bytes, a 4-byte magic `0xEA020000`, then entries of
//! name length, name index, value offset (from the first entry), value
//! inode, value size, hash and name, each padded to 4 bytes; a zero word
//! ends the list.

use crate::dir::Slot;
use crate::layout::{GOOD_OLD_INODE_SIZE, Inode, RawEntry, XATTR_MAGIC, dirent_type, le16, le32};
use crate::{BlockIo, Ctx};
use alloc::vec::Vec;
use poly_vfs::{FileType, Result};

/// `system.` attributes' name index.
const INDEX_SYSTEM: u8 = 7;

/// The value of in-inode attribute `system.data`, if the record has one.
/// `None` for a malformed attribute area.
fn system_data(raw: &[u8]) -> Option<&[u8]> {
    let start = GOOD_OLD_INODE_SIZE as usize + Inode::extra(raw);
    if raw.len() < start + 4 || le32(raw, start) != XATTR_MAGIC {
        return Some(&[]);
    }
    let base = start + 4;
    let mut at = base;
    while at + 16 <= raw.len() && le32(raw, at) != 0 {
        let name_len = raw[at] as usize;
        let index = raw[at + 1];
        let value_off = le16(raw, at + 2) as usize;
        let value_inum = le32(raw, at + 4);
        let value_size = le32(raw, at + 8) as usize;
        let name = raw.get(at + 16..at + 16 + name_len)?;
        if index == INDEX_SYSTEM && name == b"data" {
            if value_inum != 0 {
                return None;
            }
            return raw.get(base + value_off..base + value_off + value_size);
        }
        at += (16 + name_len + 3) & !3;
    }
    Some(&[])
}

impl<D: BlockIo> Ctx<'_, D> {
    /// All of an inline inode's bytes: `i_block`, then `system.data`.
    pub fn inline_data(&mut self, i: &Inode) -> Result<Vec<u8>> {
        let raw = self.read_inode_raw(i.ino)?;
        let mut data = i.block_bytes().to_vec();
        match system_data(&raw) {
            Some(more) => data.extend_from_slice(more),
            None => return Err(self.corrupt()),
        }
        Ok(data)
    }

    /// The entries of an inline directory, `.` and `..` first: inode, type
    /// code, name.
    pub fn inline_dir(&mut self, dir: &Inode) -> Result<Vec<(u32, u8, Vec<u8>)>> {
        let data = self.inline_data(dir)?;
        let ft = dirent_type(FileType::Directory);
        let mut out = alloc::vec![(dir.ino, ft, b".".to_vec()), (le32(&data, 0), ft, b"..".to_vec())];
        // Records in i_block after the parent, then in the attribute.
        for (start, end) in [(4, 60), (60, data.len())] {
            let mut off = start;
            while off < end {
                let e = RawEntry::decode(&data[..end], off, self.geo.filetype).ok_or_else(|| self.corrupt())?;
                if e.inode > self.geo.inodes {
                    return Err(self.corrupt());
                }
                if e.inode != 0 {
                    out.push((e.inode, e.file_type, e.name(&data, off).to_vec()));
                }
                off += e.rec_len;
            }
        }
        Ok(out)
    }

    /// A name in an inline directory (the slot is for reading only).
    pub fn inline_find(&mut self, dir: &Inode, name: &[u8]) -> Result<Option<Slot>> {
        let found = self.inline_dir(dir)?.into_iter().find(|(_, _, n)| n == name);
        Ok(found.map(|(ino, ft, n)| Slot {
            phys: 0,
            off: 0,
            prev: None,
            entry: RawEntry { inode: ino, rec_len: 0, name_len: n.len(), file_type: ft },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{put16, put32};

    #[test]
    fn finds_system_data() {
        let mut raw = [0u8; 256];
        put16(&mut raw, 128, 32);
        let start = 160;
        put32(&mut raw, start, XATTR_MAGIC);
        // One unrelated attribute ("user.x"), then system.data.
        let e1 = start + 4;
        raw[e1] = 1;
        raw[e1 + 1] = 1;
        put16(&mut raw, e1 + 2, 60);
        put32(&mut raw, e1 + 8, 2);
        raw[e1 + 16] = b'x';
        let e2 = e1 + 20;
        raw[e2] = 4;
        raw[e2 + 1] = INDEX_SYSTEM;
        put16(&mut raw, e2 + 2, 64);
        put32(&mut raw, e2 + 8, 5);
        raw[e2 + 16..e2 + 20].copy_from_slice(b"data");
        raw[start + 4 + 64..start + 4 + 69].copy_from_slice(b"hello");
        assert_eq!(system_data(&raw), Some(&b"hello"[..]));
        // No attribute area at all.
        assert_eq!(system_data(&[0u8; 128]), Some(&[][..]));
        // A value past the record.
        put16(&mut raw, e2 + 2, 200);
        assert_eq!(system_data(&raw), None);
    }
}
