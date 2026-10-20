//! The jbd2 journal ("ext4 Data Structures and Algorithms", "Journal
//! (jbd2)"): replay at mount, and journaling of this driver's own changes.
//!
//! # Format
//!
//! The journal is the file of inode 8 (`s_journal_inum`), a circular log
//! of blocks after its superblock (block 0). Every log block starts with a
//! big-endian header: magic `0xC03B3998`, block type, transaction
//! sequence number. A transaction is one or more *descriptor* blocks, each
//! followed by the blocks it describes (tags: file system block number,
//! flags, and with checksum v3 a CRC-32C of the block; the first tag is
//! followed by the journal UUID), *revoke* blocks (file system blocks whose
//! earlier copies in the log must not be replayed), and a *commit* block.
//! A logged block that starts with the magic is "escaped" (its first four
//! bytes zeroed, the tag flagged). The superblock says where the oldest
//! transaction that may still need replaying starts (`s_start`, 0: the log
//! is empty) and its sequence number. Checksum v3 (`metadata_csum`
//! volumes) seeds CRC-32C with the journal UUID's CRC-32C and covers
//! descriptor and revoke blocks (a 4-byte tail), each logged block (in its
//! tag, after the sequence number) and the commit block.
//!
//! # Replay
//!
//! Three steps from `s_start`: scan the transactions with consecutive
//! sequence numbers that end in a valid commit block (a bad checksum or an
//! unexpected block ends the log), collect their revoke records (block ->
//! newest revoking transaction), then write each logged block of each
//! complete transaction home unless a transaction at or after it revoked
//! it. Then the log is marked empty. Replay supports logs without
//! checksums, with checksum v1 (not verified), and v3; not v2 or fast
//! commits.
//!
//! # Our transactions
//!
//! Metadata writes go into the *running transaction* (an in-memory copy of
//! each block, which reads see); file data is written in place. A commit
//! (at `sync`, when the transaction grows big, at unmount) writes the
//! revoke records, the descriptors and the blocks to the log, flushes the
//! device (the file data written so far is durable then too: "ordered"
//! mode), writes the commit block and flushes again; then it writes the
//! blocks home ("checkpoint") at once. The log tail in the superblock moves
//! lazily: it points at the first transaction since the log was last
//! empty, and the log restarts at its beginning when the next transaction
//! does not fit before its end (after a flush makes every checkpoint
//! durable and the superblock says so). Freeing a block that is in the log
//! since the tail adds a revoke record, so replay never writes old metadata
//! over what that block holds next; freed blocks are not reused before the
//! commit (`balloc.rs`).

use crate::layout::{FL_EXTENTS, JOURNAL_INO};
use crate::{BlockIo, Ctx, MountError, crc::crc32c};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Errno, FileType, Result};

pub const MAGIC: u32 = 0xC03B_3998;
const DESCRIPTOR: u32 = 1;
const COMMIT: u32 = 2;
const SUPERBLOCK_V1: u32 = 3;
const SUPERBLOCK_V2: u32 = 4;
const REVOKE: u32 = 5;

/// Journal feature flags.
const COMPAT_CHECKSUM: u32 = 0x1;
const INCOMPAT_REVOKE: u32 = 0x1;
const INCOMPAT_64BIT: u32 = 0x2;
const INCOMPAT_ASYNC_COMMIT: u32 = 0x4;
const INCOMPAT_CSUM_V2: u32 = 0x8;
const INCOMPAT_CSUM_V3: u32 = 0x10;
const INCOMPAT_KNOWN: u32 = INCOMPAT_REVOKE | INCOMPAT_64BIT | INCOMPAT_ASYNC_COMMIT | INCOMPAT_CSUM_V3;
/// `s_checksum_type` for CRC-32C.
const CSUM_CRC32C: u8 = 4;

/// Tag flags.
const FLAG_ESCAPE: u32 = 1;
const FLAG_SAME_UUID: u32 = 2;
const FLAG_LAST_TAG: u32 = 8;

/// Journal superblock field offsets (big-endian).
mod jsb {
    pub const BLOCKSIZE: usize = 0x0C;
    pub const MAXLEN: usize = 0x10;
    pub const FIRST: usize = 0x14;
    pub const SEQUENCE: usize = 0x18;
    pub const START: usize = 0x1C;
    pub const COMPAT: usize = 0x24;
    pub const INCOMPAT: usize = 0x28;
    pub const RO_COMPAT: usize = 0x2C;
    pub const UUID: usize = 0x30;
    pub const CHECKSUM_TYPE: usize = 0x50;
    pub const CHECKSUM: usize = 0xFC;
    pub const SIZE: usize = 1024;
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn put_be32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

fn put_be16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_be_bytes());
}

/// `a` is `b` or after it, in sequence numbers that wrap.
fn seq_ge(a: u32, b: u32) -> bool {
    a.wrapping_sub(b) as i32 >= 0
}

/// The journal of a mounted volume.
pub(crate) struct Journal {
    /// The journal file's blocks, as runs `(logical, physical, length)`.
    map: Vec<(u64, u64, u64)>,
    bs: usize,
    /// The superblock block as on disk.
    sb: Vec<u8>,
    first: u32,
    maxlen: u32,
    v3: bool,
    wide: bool,
    seed: u32,
    uuid: [u8; 16],
    /// Where the next transaction goes, and its sequence number.
    head: u32,
    seq: u32,
    /// The superblock on disk points at a transaction (`s_start != 0`).
    tail_on_disk: bool,
    /// The running transaction: block -> contents, and revoked blocks.
    running: BTreeMap<u64, Vec<u8>>,
    revoked: BTreeSet<u64>,
    /// Blocks in the log since the tail (revoke them when freed).
    logged: BTreeSet<u64>,
    /// Blocks a transaction may hold.
    max_trans: usize,
    pub commits: u64,
}

impl Journal {
    /// Reads the journal inode and superblock.
    pub fn open<D: BlockIo>(ctx: &mut Ctx<'_, D>) -> core::result::Result<Journal, MountError> {
        let bad = MountError::Corrupt("journal");
        let ino = ctx.st.sb.journal_inum();
        if ino != JOURNAL_INO && (ino < ctx.geo.first_ino || ino > ctx.geo.inodes) {
            return Err(MountError::Corrupt("journal inode number"));
        }
        let i = ctx.read_inode(ino).map_err(|_| MountError::Corrupt("journal inode"))?;
        if i.kind() != Some(FileType::Regular) || i.links == 0 {
            return Err(MountError::Corrupt("journal inode"));
        }
        let bs = ctx.geo.bs;
        let len = i.size / bs as u64;
        let mut map: Vec<(u64, u64, u64)> = Vec::new();
        let mut l = 0;
        while l < len {
            let (p, n) = if i.flags & FL_EXTENTS != 0 {
                match ctx.ext_lookup(&i, l).map_err(|_| bad)? {
                    crate::extent::Lookup::Mapped(m) => (m.pblk, m.run.min(len - l)),
                    crate::extent::Lookup::Hole { .. } => return Err(bad),
                }
            } else {
                (ctx.bmap(&i, l).map_err(|_| bad)?.ok_or(bad)?, 1)
            };
            match map.last_mut() {
                Some(last) if last.0 + last.2 == l && last.1 + last.2 == p => last.2 += n,
                _ => map.push((l, p, n)),
            }
            l += n;
        }
        if map.is_empty() {
            return Err(bad);
        }
        let mut sb = vec![0u8; bs];
        ctx.disk.read(map[0].1, &mut sb).map_err(MountError::Io)?;
        let btype = be32(&sb, 4);
        if be32(&sb, 0) != MAGIC || !(btype == SUPERBLOCK_V1 || btype == SUPERBLOCK_V2) {
            return Err(MountError::Corrupt("journal superblock"));
        }
        let (maxlen, first) = (be32(&sb, jsb::MAXLEN), be32(&sb, jsb::FIRST));
        if be32(&sb, jsb::BLOCKSIZE) as usize != bs || maxlen as u64 > len || first == 0 || first + 64 > maxlen {
            return Err(MountError::Corrupt("journal superblock"));
        }
        let (incompat, compat) =
            if btype == SUPERBLOCK_V2 { (be32(&sb, jsb::INCOMPAT), be32(&sb, jsb::COMPAT)) } else { (0, 0) };
        if incompat & INCOMPAT_CSUM_V2 != 0 {
            return Err(MountError::Unsupported("journal checksum v2"));
        }
        if incompat & !INCOMPAT_KNOWN != 0 {
            return Err(MountError::Unsupported("journal features"));
        }
        let _ = compat & COMPAT_CHECKSUM;
        let v3 = incompat & INCOMPAT_CSUM_V3 != 0;
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&sb[jsb::UUID..jsb::UUID + 16]);
        if v3 {
            let mut copy = sb[..jsb::SIZE].to_vec();
            copy[jsb::CHECKSUM..jsb::CHECKSUM + 4].fill(0);
            if crc32c(!0, &copy) != be32(&sb, jsb::CHECKSUM) {
                return Err(MountError::Corrupt("journal superblock checksum"));
            }
        }
        let start = be32(&sb, jsb::START);
        if start != 0 && (start < first || start >= maxlen) {
            return Err(MountError::Corrupt("journal start"));
        }
        let capacity = (maxlen - first) as usize;
        Ok(Journal {
            map,
            bs,
            first,
            maxlen,
            v3,
            wide: incompat & INCOMPAT_64BIT != 0,
            seed: crc32c(!0, &uuid),
            uuid,
            head: first,
            seq: be32(&sb, jsb::SEQUENCE),
            tail_on_disk: start != 0,
            running: BTreeMap::new(),
            revoked: BTreeSet::new(),
            logged: BTreeSet::new(),
            max_trans: (capacity / 4).max(32),
            commits: 0,
            sb,
        })
    }

    pub fn set_max_trans(&mut self, blocks: usize) {
        self.max_trans = blocks.clamp(16, (((self.maxlen - self.first) / 4) as usize).max(16));
    }

    /// Nothing in the running transaction.
    pub fn is_idle(&self) -> bool {
        self.running.is_empty() && self.revoked.is_empty()
    }

    pub fn needs_recovery(&self) -> bool {
        be32(&self.sb, jsb::START) != 0
    }

    /// The running transaction's copy of block `b`.
    pub fn get(&self, b: u64) -> Option<&[u8]> {
        self.running.get(&b).map(|v| v.as_slice())
    }

    /// The disk block of journal block `n`.
    fn phys(&self, n: u32) -> Result<u64> {
        let n = n as u64;
        let i = self.map.partition_point(|r| r.0 + r.2 <= n);
        match self.map.get(i) {
            Some(&(l, p, len)) if l <= n && n < l + len => Ok(p + (n - l)),
            _ => Err(Errno::EIO),
        }
    }

    fn next(&self, pos: u32) -> u32 {
        if pos + 1 >= self.maxlen { self.first } else { pos + 1 }
    }

    fn tag_size(&self) -> usize {
        if self.v3 {
            16
        } else if self.wide {
            12
        } else {
            8
        }
    }

    fn tail(&self) -> usize {
        if self.v3 { 4 } else { 0 }
    }

    /// Whether a descriptor or revoke block's tail checksum is right.
    fn block_tail_ok(&self, b: &[u8]) -> bool {
        if !self.v3 {
            return true;
        }
        let mut copy = b.to_vec();
        let n = copy.len();
        copy[n - 4..].fill(0);
        crc32c(self.seed, &copy) == be32(b, n - 4)
    }

    fn seal_tail(&self, b: &mut [u8]) {
        if self.v3 {
            let n = b.len();
            b[n - 4..].fill(0);
            let c = crc32c(self.seed, b);
            put_be32(b, n - 4, c);
        }
    }

    fn commit_ok(&self, b: &[u8]) -> bool {
        if !self.v3 {
            return true;
        }
        let mut copy = b.to_vec();
        copy[16..20].fill(0);
        crc32c(self.seed, &copy) == be32(b, 16)
    }

    fn tag_csum(&self, seq: u32, data: &[u8]) -> u32 {
        crc32c(crc32c(self.seed, &seq.to_be_bytes()), data)
    }

    fn header(&self, b: &mut [u8], kind: u32, seq: u32) {
        put_be32(b, 0, MAGIC);
        put_be32(b, 4, kind);
        put_be32(b, 8, seq);
    }

    /// Writes the superblock with `s_start` and `s_sequence`.
    fn write_sb<D: BlockIo>(&mut self, disk: &crate::io::Disk<D>, start: u32, seq: u32) -> Result<()> {
        put_be32(&mut self.sb, jsb::START, start);
        put_be32(&mut self.sb, jsb::SEQUENCE, seq);
        if be32(&self.sb, jsb::INCOMPAT) & INCOMPAT_CSUM_V3 != 0 && be32(&self.sb, 4) == SUPERBLOCK_V2 {
            self.sb[jsb::CHECKSUM..jsb::CHECKSUM + 4].fill(0);
            let c = crc32c(!0, &self.sb[..jsb::SIZE]);
            put_be32(&mut self.sb, jsb::CHECKSUM, c);
        }
        let p = self.phys(0)?;
        disk.write(p, &self.sb)
    }
}

/// A transaction found in the log.
struct Found {
    seq: u32,
    /// File system block, log position, flags, tag checksum.
    tags: Vec<(u64, u32, u32, u32)>,
    revokes: Vec<u64>,
}

/// Replays the journal (the probe mount, before anything else reads the
/// volume). Returns the number of transactions replayed.
pub fn recover<D: BlockIo>(ctx: &mut Ctx<'_, D>) -> core::result::Result<u32, MountError> {
    let Some(mut j) = ctx.st.journal.take() else { return Ok(0) };
    let r = replay(ctx, &mut j);
    ctx.st.journal = Some(j);
    r
}

fn replay<D: BlockIo>(ctx: &mut Ctx<'_, D>, j: &mut Journal) -> core::result::Result<u32, MountError> {
    let io = MountError::Io;
    let start = be32(&j.sb, jsb::START);
    let mut seq = be32(&j.sb, jsb::SEQUENCE);
    if start == 0 {
        return Ok(0);
    }
    if be32(&j.sb, jsb::INCOMPAT) & 0x20 != 0 {
        return Err(MountError::Unsupported("journal fast commits"));
    }
    let bs = j.bs;
    let read = |j: &Journal, pos: u32| -> core::result::Result<Vec<u8>, MountError> {
        let mut b = vec![0u8; bs];
        ctx.disk.read(j.phys(pos).map_err(io)?, &mut b).map_err(io)?;
        Ok(b)
    };
    // 1. Scan.
    let mut done: Vec<Found> = Vec::new();
    let mut cur = Found { seq, tags: Vec::new(), revokes: Vec::new() };
    let mut pos = start;
    let mut budget = (j.maxlen - j.first) as u64 + 1;
    let tag_size = j.tag_size();
    let rec = if j.wide { 8 } else { 4 };
    'scan: while budget > 0 {
        budget -= 1;
        let b = read(j, pos)?;
        if be32(&b, 0) != MAGIC || be32(&b, 8) != seq {
            break;
        }
        match be32(&b, 4) {
            DESCRIPTOR => {
                if !j.block_tail_ok(&b) {
                    break;
                }
                let end = bs - j.tail();
                let mut off = 12;
                while off + tag_size <= end {
                    let lo = be32(&b, off) as u64;
                    let (flags, hi, csum) = if j.v3 {
                        (be32(&b, off + 4), be32(&b, off + 8) as u64, be32(&b, off + 12))
                    } else {
                        let hi = if j.wide { be32(&b, off + 8) as u64 } else { 0 };
                        (be16(&b, off + 6) as u32, hi, 0)
                    };
                    let block = if j.wide { lo | hi << 32 } else { lo };
                    off += tag_size;
                    if flags & FLAG_SAME_UUID == 0 {
                        off += 16;
                    }
                    pos = j.next(pos);
                    if budget == 0 {
                        break 'scan;
                    }
                    budget -= 1;
                    cur.tags.push((block, pos, flags, csum));
                    if flags & FLAG_LAST_TAG != 0 {
                        break;
                    }
                }
            }
            REVOKE => {
                if !j.block_tail_ok(&b) {
                    break;
                }
                let count = be32(&b, 12) as usize;
                if count < 16 || count > bs - j.tail() {
                    break;
                }
                let mut off = 16;
                while off + rec <= count {
                    let block = if j.wide {
                        (be32(&b, off) as u64) << 32 | be32(&b, off + 4) as u64
                    } else {
                        be32(&b, off) as u64
                    };
                    cur.revokes.push(block);
                    off += rec;
                }
            }
            COMMIT => {
                if !j.commit_ok(&b) {
                    break;
                }
                seq = seq.wrapping_add(1);
                done.push(core::mem::replace(&mut cur, Found { seq, tags: Vec::new(), revokes: Vec::new() }));
            }
            _ => break,
        }
        pos = j.next(pos);
    }
    // 2. Revoke records.
    let mut revoked: BTreeMap<u64, u32> = BTreeMap::new();
    for t in &done {
        for &b in &t.revokes {
            let e = revoked.entry(b).or_insert(t.seq);
            if seq_ge(t.seq, *e) {
                *e = t.seq;
            }
        }
    }
    // 3. Replay.
    for t in &done {
        for &(block, at, flags, csum) in &t.tags {
            if revoked.get(&block).is_some_and(|&r| seq_ge(r, t.seq)) {
                continue;
            }
            if block >= ctx.geo.blocks {
                ctx.corrupt();
                continue;
            }
            let mut data = read(j, at)?;
            if j.v3 && j.tag_csum(t.seq, &data) != csum {
                let mut unescaped = data.clone();
                unescaped[..4].copy_from_slice(&MAGIC.to_be_bytes());
                if flags & FLAG_ESCAPE == 0 || j.tag_csum(t.seq, &unescaped) != csum {
                    // A block that did not make it: leave its home copy.
                    ctx.corrupt();
                    continue;
                }
            }
            if flags & FLAG_ESCAPE != 0 {
                data[..4].copy_from_slice(&MAGIC.to_be_bytes());
            }
            ctx.disk.write(block, &data).map_err(io)?;
        }
    }
    ctx.disk.flush().map_err(io)?;
    j.write_sb(ctx.disk, 0, seq).map_err(io)?;
    ctx.disk.flush().map_err(io)?;
    j.seq = seq;
    j.head = j.first;
    j.tail_on_disk = false;
    Ok(done.len() as u32)
}

impl<D: BlockIo> Ctx<'_, D> {
    /// Puts a metadata block into the running transaction.
    pub fn journal_put(&mut self, b: u64, data: Vec<u8>) {
        if let Some(j) = self.st.journal.as_mut() {
            j.revoked.remove(&b);
            j.running.insert(b, data);
        }
    }

    /// A block is being freed: out of the running transaction, revoked if
    /// the log has it.
    pub fn journal_forget(&mut self, b: u64) {
        if let Some(j) = self.st.journal.as_mut() {
            j.running.remove(&b);
            if j.logged.contains(&b) {
                j.revoked.insert(b);
            }
        }
    }

    fn journal_load(&self) -> usize {
        self.st.journal.as_ref().map_or(0, |j| j.running.len() + j.revoked.len() / 128)
    }

    /// Whether the running transaction is nearly as big as one may be.
    pub fn journal_tight(&self) -> bool {
        self.st.journal.as_ref().is_some_and(|j| self.journal_load() + j.max_trans / 4 > j.max_trans)
    }

    /// Before an operation: commits if the transaction is getting big, or
    /// if freed space the operation may need is held until a commit.
    pub fn journal_reserve(&mut self) -> Result<()> {
        let Some(j) = self.st.journal.as_ref() else { return Ok(()) };
        let big = self.journal_load() + j.max_trans / 2 > j.max_trans;
        let blocks_held = !self.st.held_blocks.is_empty() && self.free_block_count() < 4096;
        let free_inodes: u64 = self.st.groups.iter().map(|d| d.free_inodes as u64).sum();
        let inodes_held = !self.st.held_inodes.is_empty() && free_inodes < 64 + self.st.held_inodes.len() as u64;
        if big || blocks_held || inodes_held { self.sync() } else { Ok(()) }
    }

    /// Mounting read-write: sets the journal's features for our
    /// transactions (the log is empty).
    pub fn journal_prepare(&mut self, wide: bool, v3: bool) -> Result<()> {
        let Some(j) = self.st.journal.as_mut() else { return Ok(()) };
        put_be32(&mut j.sb, 4, SUPERBLOCK_V2);
        put_be32(&mut j.sb, jsb::COMPAT, 0);
        let incompat = INCOMPAT_REVOKE | if wide { INCOMPAT_64BIT } else { 0 } | if v3 { INCOMPAT_CSUM_V3 } else { 0 };
        put_be32(&mut j.sb, jsb::INCOMPAT, incompat);
        put_be32(&mut j.sb, jsb::RO_COMPAT, 0);
        j.sb[jsb::CHECKSUM_TYPE] = if v3 { CSUM_CRC32C } else { 0 };
        j.v3 = v3;
        j.wide = wide;
        j.head = j.first;
        j.tail_on_disk = false;
        let seq = j.seq;
        j.write_sb(self.disk, 0, seq)?;
        self.disk.flush()
    }

    /// Commits the running transaction (callers flush their caches and the
    /// superblock into it first) and checkpoints it.
    pub fn commit(&mut self) -> Result<()> {
        let Some(mut j) = self.st.journal.take() else { return Ok(()) };
        let r = self.commit_with(&mut j);
        self.st.journal = Some(j);
        r?;
        self.release_held();
        Ok(())
    }

    fn commit_with(&mut self, j: &mut Journal) -> Result<()> {
        if j.running.is_empty() && j.revoked.is_empty() {
            return Ok(());
        }
        let bs = j.bs;
        let tag_size = j.tag_size();
        let avail = bs - 12 - j.tail();
        let per_desc = 1 + (avail - tag_size - 16) / tag_size;
        let rec = if j.wide { 8 } else { 4 };
        let per_revoke = (bs - 16 - j.tail()) / rec;
        let blocks: Vec<(u64, Vec<u8>)> = core::mem::take(&mut j.running).into_iter().collect();
        let revoked: Vec<u64> = core::mem::take(&mut j.revoked).into_iter().collect();
        let need = blocks.len().div_ceil(per_desc) + blocks.len() + revoked.len().div_ceil(per_revoke) + 1;
        if need as u32 > j.maxlen - j.first {
            return Err(self.corrupt());
        }
        let disk = self.disk;
        if j.head + need as u32 > j.maxlen {
            // Restart the log: every checkpoint durable, then the tail.
            disk.flush()?;
            j.head = j.first;
            j.logged.clear();
            let (h, s) = (j.head, j.seq);
            j.write_sb(disk, h, s)?;
            disk.flush()?;
            j.tail_on_disk = true;
        } else if !j.tail_on_disk {
            // Made durable by the flush before the commit block.
            let (h, s) = (j.head, j.seq);
            j.write_sb(disk, h, s)?;
            j.tail_on_disk = true;
        }
        let seq = j.seq;
        let mut pos = j.head;
        for chunk in revoked.chunks(per_revoke) {
            let mut b = vec![0u8; bs];
            j.header(&mut b, REVOKE, seq);
            put_be32(&mut b, 12, (16 + chunk.len() * rec) as u32);
            for (k, &blk) in chunk.iter().enumerate() {
                let at = 16 + k * rec;
                if j.wide {
                    put_be32(&mut b, at, (blk >> 32) as u32);
                    put_be32(&mut b, at + 4, blk as u32);
                } else {
                    put_be32(&mut b, at, blk as u32);
                }
            }
            j.seal_tail(&mut b);
            disk.write(j.phys(pos)?, &b)?;
            pos += 1;
        }
        for chunk in blocks.chunks(per_desc) {
            let mut d = vec![0u8; bs];
            j.header(&mut d, DESCRIPTOR, seq);
            let mut off = 12;
            let desc_pos = pos;
            pos += 1;
            for (k, (blk, data)) in chunk.iter().enumerate() {
                let mut logged = data.clone();
                let mut flags = 0;
                if be32(&logged, 0) == MAGIC {
                    logged[..4].fill(0);
                    flags |= FLAG_ESCAPE;
                }
                if k > 0 {
                    flags |= FLAG_SAME_UUID;
                }
                if k + 1 == chunk.len() {
                    flags |= FLAG_LAST_TAG;
                }
                put_be32(&mut d, off, *blk as u32);
                if j.v3 {
                    put_be32(&mut d, off + 4, flags);
                    put_be32(&mut d, off + 8, (*blk >> 32) as u32);
                    let c = j.tag_csum(seq, &logged);
                    put_be32(&mut d, off + 12, c);
                } else {
                    put_be16(&mut d, off + 6, flags as u16);
                    if j.wide {
                        put_be32(&mut d, off + 8, (*blk >> 32) as u32);
                    }
                }
                off += tag_size;
                if k == 0 {
                    d[off..off + 16].copy_from_slice(&j.uuid);
                    off += 16;
                }
                disk.write(j.phys(pos)?, &logged)?;
                pos += 1;
            }
            j.seal_tail(&mut d);
            disk.write(j.phys(desc_pos)?, &d)?;
        }
        // Everything before the commit block (and the file data written so
        // far) durable first.
        disk.flush()?;
        let mut c = vec![0u8; bs];
        j.header(&mut c, COMMIT, seq);
        c[0x30..0x38].copy_from_slice(&(self.now.sec.max(0) as u64).to_be_bytes());
        put_be32(&mut c, 0x38, self.now.nsec);
        if j.v3 {
            let sum = crc32c(j.seed, &c);
            put_be32(&mut c, 16, sum);
        }
        disk.write(j.phys(pos)?, &c)?;
        disk.flush()?;
        // Checkpoint.
        for (blk, data) in &blocks {
            disk.write(*blk, data)?;
            j.logged.insert(*blk);
        }
        j.head = pos + 1;
        j.seq = seq.wrapping_add(1);
        j.commits += 1;
        Ok(())
    }

    /// Unmounting (after a commit): every checkpoint durable, the log
    /// marked empty.
    pub fn journal_clean(&mut self) -> Result<()> {
        let Some(j) = self.st.journal.as_mut() else { return Ok(()) };
        self.disk.flush()?;
        let seq = j.seq;
        j.write_sb(self.disk, 0, seq)?;
        self.disk.flush()?;
        j.tail_on_disk = false;
        j.head = j.first;
        j.logged.clear();
        Ok(())
    }
}
