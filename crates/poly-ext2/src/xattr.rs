//! Extended attributes ("ext4 Data Structures and Algorithms", "Extended
//! Attributes"): in the inode's extra space and in one attribute block
//! (`i_file_acl`).
//!
//! **In the inode** (large inodes): after the `i_extra_isize` bytes, the
//! magic `0xEA020000`, then entries; value offsets count from the first
//! entry; the space ends with the inode record. **In a block**: a 32-byte
//! header (magic, reference count, block count 1, hash, checksum), entries
//! from offset 32, value offsets from the start of the block. An entry is
//! name length, name index, value offset (16 bits), value inode (0: Poly
//! refuses `ea_inode` volumes), value size, hash, then the name without
//! its prefix, padded to 4 bytes; a zero word ends the list. Values are
//! padded to 4 bytes too and sit at the end of their space, growing down.
//!
//! The name index stands for the prefix: 1 `user.`, 2
//! `system.posix_acl_access`, 3 `system.posix_acl_default`, 4 `trusted.`,
//! 6 `security.`, 7 `system.`, 8 `system.richacl`. ACLs are stored in
//! ext4's own form (version 1, 4-byte entries for the owner, group, mask
//! and other, 8-byte ones with an id for named users and groups) and read
//! as the VFS's (`poly_vfs::xattr::acl`, version 2). `system.data` (inline
//! data) is the file's, not an attribute: hidden.
//!
//! Hashes as the format defines them: an entry's from its name (each byte:
//! rotate left 5, xor) and its value (each little-endian 32-bit word of
//! the padded value: rotate 16, xor); the block's from its entries'
//! (rotate 16, xor; 0 if any entry's is 0). With `metadata_csum` the
//! block's checksum covers its number and contents (`csum::xattr_block`).
//!
//! **Changing** an inode's attributes rewrites them all: sorted (index,
//! name length, name), each into the inode's space while it fits, the rest
//! into a block. A block only this inode uses is rewritten in place; a
//! shared one (reference count above 1) is left to its other users and a
//! new one allocated; a block no longer needed is released by reference
//! count. The first attribute written turns on the `ext_attr` feature.
//! Values that cannot fit in a block are `ENOSPC`.

use crate::layout::{COMPAT_EXT_ATTR, FL_APPEND, FL_IMMUTABLE, FL_INLINE_DATA, GOOD_OLD_INODE_SIZE, Inode};
use crate::layout::{XATTR_MAGIC, le16, le32, put16, put32};
use crate::{BlockIo, Ctx, csum};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::xattr::{SetMode, acl};
use poly_vfs::{Errno, Result};

const BLOCK_HEADER: usize = 32;
const ENTRY_HEADER: usize = 16;

const INDEX_USER: u8 = 1;
const INDEX_ACL_ACCESS: u8 = 2;
const INDEX_ACL_DEFAULT: u8 = 3;
const INDEX_TRUSTED: u8 = 4;
const INDEX_SECURITY: u8 = 6;
const INDEX_SYSTEM: u8 = 7;
const INDEX_RICHACL: u8 = 8;

/// ext4's ACL version.
const EXT4_ACL_VERSION: u32 = 1;

/// One attribute: name index, the name after its prefix, the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub index: u8,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl Attr {
    fn entry_len(&self) -> usize {
        pad(ENTRY_HEADER + self.name.len())
    }

    fn value_len(&self) -> usize {
        pad(self.value.len())
    }

    fn key(&self) -> (u8, usize, &[u8]) {
        (self.index, self.name.len(), &self.name)
    }
}

const fn pad(n: usize) -> usize {
    (n + 3) & !3
}

/// The index and stored name of a full attribute name; `None` for names
/// this format has no index for.
pub fn split_name(full: &[u8]) -> Option<(u8, &[u8])> {
    match full {
        b"system.posix_acl_access" => return Some((INDEX_ACL_ACCESS, b"")),
        b"system.posix_acl_default" => return Some((INDEX_ACL_DEFAULT, b"")),
        b"system.richacl" => return Some((INDEX_RICHACL, b"")),
        _ => {}
    }
    for (prefix, index) in [
        (&b"user."[..], INDEX_USER),
        (b"trusted.", INDEX_TRUSTED),
        (b"security.", INDEX_SECURITY),
        (b"system.", INDEX_SYSTEM),
    ] {
        if let Some(rest) = full.strip_prefix(prefix) {
            return Some((index, rest));
        }
    }
    None
}

/// The full name of a stored attribute; `None` for unknown indexes and
/// for `system.data`.
pub fn full_name(index: u8, name: &[u8]) -> Option<Vec<u8>> {
    let prefix: &[u8] = match index {
        INDEX_USER => b"user.",
        INDEX_ACL_ACCESS => return name.is_empty().then(|| b"system.posix_acl_access".to_vec()),
        INDEX_ACL_DEFAULT => return name.is_empty().then(|| b"system.posix_acl_default".to_vec()),
        INDEX_RICHACL => return name.is_empty().then(|| b"system.richacl".to_vec()),
        INDEX_TRUSTED => b"trusted.",
        INDEX_SECURITY => b"security.",
        INDEX_SYSTEM if name == b"data" => return None,
        INDEX_SYSTEM => b"system.",
        _ => return None,
    };
    let mut full = prefix.to_vec();
    full.extend_from_slice(name);
    Some(full)
}

/// An entry's hash.
pub fn entry_hash(name: &[u8], value: &[u8]) -> u32 {
    let mut hash: u32 = 0;
    for &c in name {
        hash = (hash << 5) ^ (hash >> 27) ^ u32::from(c);
    }
    for word in value.chunks(4) {
        let mut w = [0u8; 4];
        w[..word.len()].copy_from_slice(word);
        hash = (hash << 16) ^ (hash >> 16) ^ u32::from_le_bytes(w);
    }
    hash
}

/// A block's hash from its entries' hashes.
pub fn block_hash(hashes: impl Iterator<Item = u32>) -> u32 {
    let mut hash: u32 = 0;
    for h in hashes {
        if h == 0 {
            return 0;
        }
        hash = (hash << 16) ^ (hash >> 16) ^ h;
    }
    hash
}

/// The entries of `area`, starting at `first`, values at `base` plus their
/// offset. `None` if one does not make sense.
pub fn parse(area: &[u8], first: usize, base: usize) -> Option<Vec<Attr>> {
    let mut out = Vec::new();
    let mut at = first;
    while at + 4 <= area.len() && le32(area, at) != 0 {
        if at + ENTRY_HEADER > area.len() {
            return None;
        }
        let name_len = area[at] as usize;
        let index = area[at + 1];
        let value_off = le16(area, at + 2) as usize;
        let value_inum = le32(area, at + 4);
        let value_size = le32(area, at + 8) as usize;
        let name = area.get(at + ENTRY_HEADER..at + ENTRY_HEADER + name_len)?;
        if value_inum != 0 {
            return None;
        }
        let value = if value_size == 0 {
            &[][..]
        } else {
            let start = base.checked_add(value_off)?;
            area.get(start..start.checked_add(value_size)?)?
        };
        out.push(Attr { index, name: name.to_vec(), value: value.to_vec() });
        at += pad(ENTRY_HEADER + name_len);
    }
    Some(out)
}

/// The bytes `attrs` take in a space whose entries start `first` bytes in:
/// entries, the end marker and values.
fn needed(first: usize, attrs: &[&Attr]) -> usize {
    first + attrs.iter().map(|a| a.entry_len() + a.value_len()).sum::<usize>() + 4
}

/// Writes `attrs` into `area` (zeroed first from `first` on): entries from
/// `first`, values from the end down, offsets from `base`. Returns the
/// entries' hashes. The caller has checked that they fit.
fn encode(area: &mut [u8], first: usize, base: usize, attrs: &[&Attr]) -> Vec<u32> {
    area[first..].fill(0);
    let mut at = first;
    let mut top = area.len();
    let mut hashes = Vec::new();
    for a in attrs {
        let value_off = if a.value.is_empty() {
            0
        } else {
            top -= a.value_len();
            area[top..top + a.value.len()].copy_from_slice(&a.value);
            top - base
        };
        let hash = entry_hash(&a.name, &a.value);
        area[at] = a.name.len() as u8;
        area[at + 1] = a.index;
        put16(area, at + 2, value_off as u16);
        put32(area, at + 4, 0);
        put32(area, at + 8, a.value.len() as u32);
        put32(area, at + 12, hash);
        area[at + ENTRY_HEADER..at + ENTRY_HEADER + a.name.len()].copy_from_slice(&a.name);
        at += a.entry_len();
        hashes.push(hash);
    }
    hashes
}

/// ext4's ACL form as the VFS's (`linux/posix_acl_xattr.h`); `None` if it
/// is malformed.
pub fn acl_from_disk(disk: &[u8]) -> Option<Vec<u8>> {
    if disk.len() < 4 || le32(disk, 0) != EXT4_ACL_VERSION {
        return None;
    }
    let mut out = acl::VERSION.to_le_bytes().to_vec();
    let mut at = 4;
    while at < disk.len() {
        let tag = le16(disk.get(at..at + 4)?, 0);
        let perm = le16(disk, at + 2);
        let id = match tag {
            acl::USER | acl::GROUP => {
                let id = le32(disk.get(at..at + 8)?, 4);
                at += 8;
                id
            }
            acl::USER_OBJ | acl::GROUP_OBJ | acl::MASK | acl::OTHER => {
                at += 4;
                u32::MAX
            }
            _ => return None,
        };
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&perm.to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
    }
    Some(out)
}

/// An inode's attributes as found: the record, those in it, those in its
/// block, and the block's reference count.
struct Found {
    raw: Vec<u8>,
    in_inode: Vec<Attr>,
    in_block: Vec<Attr>,
    refs: u32,
}

/// Where the in-inode attribute space starts in a record, if it has one.
fn inode_space(raw: &[u8]) -> Option<usize> {
    let start = GOOD_OLD_INODE_SIZE as usize + Inode::extra(raw);
    // Room for the magic, one entry and the end marker at least.
    (raw.len() > GOOD_OLD_INODE_SIZE as usize && start + 4 + ENTRY_HEADER + 4 <= raw.len()).then_some(start)
}

impl<D: BlockIo> Ctx<'_, D> {
    fn xattrs_of(&mut self, i: &Inode) -> Result<Found> {
        let raw = self.read_inode_raw(i.ino)?;
        let mut in_inode = Vec::new();
        if let Some(start) = inode_space(&raw)
            && le32(&raw, start) == XATTR_MAGIC
        {
            in_inode = parse(&raw[start..], 4, 4).ok_or_else(|| self.corrupt())?;
        }
        let (mut in_block, mut refs) = (Vec::new(), 0);
        if i.file_acl != 0 {
            let b = i.file_acl;
            if !self.block_ok(b) {
                return Err(self.corrupt());
            }
            let data = self.read_block(b)?;
            if le32(&data, 0) != XATTR_MAGIC || le32(&data, 8) != 1 {
                return Err(self.corrupt());
            }
            if let Some(seed) = self.geo.meta_seed()
                && csum::xattr_block(seed, b, &data) != le32(&data, 0x10)
            {
                return Err(self.bad_csum());
            }
            refs = le32(&data, 4);
            in_block = parse(&data, BLOCK_HEADER, 0).ok_or_else(|| self.corrupt())?;
        }
        Ok(Found { raw, in_inode, in_block, refs })
    }

    /// The value of attribute `full` of inode `ino` (`ENODATA` if none).
    pub fn xattr_get(&mut self, ino: u32, full: &[u8]) -> Result<Vec<u8>> {
        let i = self.live_inode(ino)?;
        let (index, name) = split_name(full).ok_or(Errno::EOPNOTSUPP)?;
        if index == INDEX_SYSTEM && name == b"data" {
            return Err(Errno::ENODATA);
        }
        let f = self.xattrs_of(&i)?;
        let a = f.in_inode.iter().chain(&f.in_block).find(|a| a.index == index && a.name == name);
        let a = a.ok_or(Errno::ENODATA)?;
        match index {
            INDEX_ACL_ACCESS | INDEX_ACL_DEFAULT => acl_from_disk(&a.value).ok_or_else(|| self.corrupt()),
            _ => Ok(a.value.clone()),
        }
    }

    /// The full names of inode `ino`'s attributes, each followed by a NUL.
    pub fn xattr_list(&mut self, ino: u32) -> Result<Vec<u8>> {
        let i = self.live_inode(ino)?;
        let f = self.xattrs_of(&i)?;
        let mut out = Vec::new();
        for a in f.in_inode.iter().chain(&f.in_block) {
            if let Some(n) = full_name(a.index, &a.name) {
                out.extend_from_slice(&n);
                out.push(0);
            }
        }
        Ok(out)
    }

    /// Sets (`Some(value)`) or removes (`None`) attribute `full` of inode
    /// `ino`, and its change time.
    pub fn xattr_set(&mut self, ino: u32, full: &[u8], value: Option<&[u8]>, how: SetMode) -> Result<()> {
        let mut i = self.live_inode(ino)?;
        let (index, name) = split_name(full).ok_or(Errno::EOPNOTSUPP)?;
        if i.flags & (FL_IMMUTABLE | FL_APPEND) != 0 {
            return Err(Errno::EPERM);
        }
        if i.flags & FL_INLINE_DATA != 0 || (index == INDEX_SYSTEM && name == b"data") {
            return Err(Errno::EROFS);
        }
        // ACLs are stored in ext4's form; the VFS only removes them.
        if value.is_some() && matches!(index, INDEX_ACL_ACCESS | INDEX_ACL_DEFAULT | INDEX_RICHACL) {
            return Err(Errno::EOPNOTSUPP);
        }
        let f = self.xattrs_of(&i)?;
        let mut all: Vec<Attr> = f.in_inode.into_iter().chain(f.in_block).collect();
        let pos = all.iter().position(|a| a.index == index && a.name == name);
        if value.is_none() && pos.is_none() {
            return Err(Errno::ENODATA);
        }
        how.check(pos.is_some())?;
        if let Some(p) = pos {
            all.remove(p);
        }
        if let Some(v) = value {
            let new = Attr { index, name: name.to_vec(), value: v.to_vec() };
            if needed(BLOCK_HEADER, &[&new]) > self.geo.bs {
                return Err(Errno::ENOSPC);
            }
            all.push(new);
        }
        all.sort_by(|a, b| a.key().cmp(&b.key()));
        // Into the inode while they fit, the rest into a block.
        let mut raw = f.raw;
        let space = inode_space(&raw);
        let (mut inode_part, mut block_part): (Vec<&Attr>, Vec<&Attr>) = (Vec::new(), Vec::new());
        for a in &all {
            let fits = space.is_some_and(|s| {
                let mut with = inode_part.clone();
                with.push(a);
                needed(4, &with) <= raw.len() - s
            });
            if fits { inode_part.push(a) } else { block_part.push(a) }
        }
        if needed(BLOCK_HEADER, &block_part) > self.geo.bs {
            return Err(Errno::ENOSPC);
        }
        let mut freed = Vec::new();
        let old = i.file_acl;
        if block_part.is_empty() {
            if old != 0 {
                if let Some(b) = self.release_xattr(old)? {
                    freed.push(b);
                }
                i.file_acl = 0;
                i.blocks = i.blocks.saturating_sub(u64::from(self.geo.spb));
            }
        } else {
            let mut data = vec![0u8; self.geo.bs];
            let hashes = encode(&mut data, BLOCK_HEADER, 0, &block_part);
            put32(&mut data, 0, XATTR_MAGIC);
            put32(&mut data, 4, 1);
            put32(&mut data, 8, 1);
            put32(&mut data, 0xC, block_hash(hashes.into_iter()));
            if old != 0 && f.refs <= 1 {
                self.seal_xattr_block(old, &mut data);
                self.write_meta(old, &data)?;
            } else {
                let goal = self.group_data_goal(self.geo.group_of_inode(ino));
                let b = self.alloc_block(goal)?;
                self.seal_xattr_block(b, &mut data);
                self.write_fresh(b, &data)?;
                if old != 0 {
                    // Shared: one reference fewer for the others.
                    if let Some(gone) = self.release_xattr(old)? {
                        freed.push(gone);
                    }
                } else {
                    i.blocks += u64::from(self.geo.spb);
                }
                i.file_acl = b;
            }
        }
        i.ctime = self.now;
        i.encode(&mut raw, &self.geo.ifmt);
        if let Some(s) = space {
            let area = &mut raw[s..];
            if inode_part.is_empty() {
                area.fill(0);
            } else {
                encode(area, 4, 4, &inode_part);
                put32(area, 0, XATTR_MAGIC);
            }
        }
        if !all.is_empty() && self.st.sb.feature_compat_raw() & COMPAT_EXT_ATTR == 0 {
            let sb = &mut self.st.sb;
            sb.set_feature_compat(sb.feature_compat_raw() | COMPAT_EXT_ATTR);
            self.st.meta_dirty = true;
            self.st.features_dirty = true;
        }
        self.put_inode_raw(ino, &mut raw)?;
        self.free_blocks(&freed)
    }

    fn seal_xattr_block(&self, b: u64, data: &mut [u8]) {
        if let Some(seed) = self.geo.meta_seed() {
            put32(data, 0x10, 0);
            let c = csum::xattr_block(seed, b, data);
            put32(data, 0x10, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(index: u8, name: &[u8], value: &[u8]) -> Attr {
        Attr { index, name: name.to_vec(), value: value.to_vec() }
    }

    #[test]
    fn names_and_indexes() {
        assert_eq!(split_name(b"user.mime"), Some((1, &b"mime"[..])));
        assert_eq!(split_name(b"system.posix_acl_access"), Some((2, &b""[..])));
        assert_eq!(split_name(b"system.posix_acl_default"), Some((3, &b""[..])));
        assert_eq!(split_name(b"trusted.t"), Some((4, &b"t"[..])));
        assert_eq!(split_name(b"security.selinux"), Some((6, &b"selinux"[..])));
        assert_eq!(split_name(b"system.data"), Some((7, &b"data"[..])));
        assert_eq!(split_name(b"os2.x"), None);
        assert_eq!(full_name(1, b"mime"), Some(b"user.mime".to_vec()));
        assert_eq!(full_name(2, b""), Some(b"system.posix_acl_access".to_vec()));
        assert_eq!(full_name(7, b"data"), None);
        assert_eq!(full_name(5, b"x"), None);
    }

    #[test]
    fn hashes() {
        // Name only: "ab" = ('a' << 5) ^ 'b'.
        assert_eq!(entry_hash(b"ab", b""), (0x61 << 5) ^ 0x62);
        // A value word: rotate the name hash by 16, xor the word.
        let h = entry_hash(b"a", b"\x01\x00\x00\x00");
        assert_eq!(h, (0x61u32 << 16) ^ 1);
        // A short value is padded with zeros.
        assert_eq!(entry_hash(b"a", b"\x01"), h);
        assert_eq!(block_hash([1u32, 2].into_iter()), (1 << 16) ^ 2);
        assert_eq!(block_hash([1u32, 0, 2].into_iter()), 0);
    }

    #[test]
    fn encode_and_parse_round_trip() {
        let a = attr(1, b"colour", b"blue");
        let b = attr(6, b"selinux", b"");
        let c = attr(4, b"odd", b"12345");
        let mut block = vec![0u8; 1024];
        encode(&mut block, BLOCK_HEADER, 0, &[&a, &b, &c]);
        assert_eq!(parse(&block, BLOCK_HEADER, 0), Some(vec![a.clone(), b.clone(), c.clone()]));
        // In an inode: offsets from the first entry, after the magic.
        let mut area = vec![0u8; 96];
        encode(&mut area, 4, 4, &[&a, &c]);
        assert_eq!(parse(&area, 4, 4), Some(vec![a.clone(), c.clone()]));
        assert_eq!(needed(4, &[&a, &c]), 4 + 24 + 4 + 20 + 8 + 4);
        // A value past the end.
        put16(&mut area, 4 + 2, 200);
        assert_eq!(parse(&area, 4, 4), None);
    }

    #[test]
    fn acls_in_ext4_form() {
        // Owner rwx, user 1000 r-x, group r-x, mask r-x, other none.
        let mut disk = 1u32.to_le_bytes().to_vec();
        for (tag, perm, id) in [(1u16, 7u16, None), (2, 5, Some(1000u32)), (4, 5, None), (0x10, 5, None), (0x20, 0, None)]
        {
            disk.extend_from_slice(&tag.to_le_bytes());
            disk.extend_from_slice(&perm.to_le_bytes());
            if let Some(id) = id {
                disk.extend_from_slice(&id.to_le_bytes());
            }
        }
        let vfs = acl_from_disk(&disk).unwrap();
        assert_eq!(vfs.len(), 4 + 5 * 8);
        assert_eq!(acl::parse(&vfs), Ok(acl::Summary { perm: 0o750, minimal: false }));
        assert_eq!(acl_from_disk(&disk[..disk.len() - 2]), None);
        assert_eq!(acl_from_disk(&2u32.to_le_bytes()), None);
    }
}
