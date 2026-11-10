//! The squashfs 4 file system, read-only, from the format's public
//! description (ADR 0168).
//!
//! [`Squashfs`] mounts an image from any [`Source`] (a loop device, a
//! file at an offset) and implements [`poly_vfs::FileSystem`] on it.
//!
//! - **Compression**: gzip (zlib), xz, lzo, lz4 and zstd, for data
//!   blocks, fragments and metadata alike, and images stored without
//!   compression (the superblock's flags and the per-block bit). Legacy
//!   lzma (id 2) is refused, as the reference refuses it.
//! - **Layout**: block sizes 4 KiB to 1 MiB, fragments, sparse blocks,
//!   duplicate-removed files (inodes shared), basic and extended inodes,
//!   hard links, device nodes, FIFOs, sockets, symbolic links of any
//!   length and extended attributes (user, trusted, security).
//! - **Inode numbers** are the image's own (1 to the inode count). An
//!   inode is found again from a table of references remembered when its
//!   directory entry was read, so no scan of the inode table is needed.
//! - Every change is `EROFS`.
#![no_std]
extern crate alloc;

mod format;
#[cfg(test)]
mod tests;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use format::*;
use poly_compress as comp;
use poly_vfs::lock::{Lock, RawLock, RawSpinLock};
use poly_vfs::xattr::SetMode;
use poly_vfs::{
    Attr, DirEntry, Errno, FileSystem, FileType, FsInfo, FsStats, Ino, NewNode, RenameFlags, Result, SetAttr, Timespec,
};

/// Where the image's bytes come from.
pub trait Source: Send + Sync {
    /// The size of the image's device or file region in bytes.
    fn size(&self) -> u64;

    /// Fills `buf` from byte `offset`; an error is an I/O error.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<(), IoFailed>;
}

/// A [`Source`] read failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoFailed;

/// Why an image was not mounted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountError {
    /// Not squashfs, or a header value the format forbids (`EINVAL`).
    Invalid,
    /// The device is too small, a read failed or the metadata cannot be
    /// decompressed (`EIO`).
    Io,
}

impl MountError {
    pub const fn errno(self) -> Errno {
        match self {
            MountError::Invalid => Errno::EINVAL,
            MountError::Io => Errno::EIO,
        }
    }
}

/// The least a device must hold for a mount: the superblock's first
/// 1024-byte block (a smaller device, 512 bytes or none, is an I/O error).
pub const MIN_DEVICE: u64 = 1024;
/// The `f_type` of squashfs (`linux/magic.h`).
pub const SQUASHFS_MAGIC: u64 = 0x7371_7368;
/// Most bytes of decompressed data blocks kept.
const BLOCK_CACHE_BYTES: usize = 8 << 20;
const META_CACHE_BLOCKS: usize = 256;
const INODE_CACHE: usize = 4096;
const NO_FRAGMENT: u32 = 0xFFFF_FFFF;
const NO_XATTR: u32 = 0xFFFF_FFFF;

struct Meta {
    data: Vec<u8>,
    /// Absolute offset of the block after this one.
    next: u64,
}

/// A place in the metadata stream: a block's absolute offset and a byte in
/// its decompressed contents.
#[derive(Clone, Copy)]
struct Pos {
    block: u64,
    off: usize,
}

/// A parsed inode.
struct Inode {
    ino: u32,
    kind: FileType,
    perm: u16,
    uid: u32,
    gid: u32,
    mtime: u32,
    nlink: u32,
    size: u64,
    rdev: u32,
    xattr: u32,
    // Directories: the listing's place and length, the parent.
    dir_block: u32,
    dir_offset: u16,
    dir_len: u32,
    parent: u32,
    // Files.
    frag: u32,
    frag_off: u32,
    starts: Vec<u64>,
    sizes: Vec<u32>,
    sparse: u64,
    // Symbolic links.
    target: Vec<u8>,
}

struct Entry {
    name: Vec<u8>,
    ino: u32,
    kind: FileType,
}

#[derive(Default)]
struct Cache {
    meta: BTreeMap<u64, Arc<Meta>>,
    blocks: BTreeMap<u64, Arc<Vec<u8>>>,
    block_bytes: usize,
    inodes: BTreeMap<u32, Arc<Inode>>,
    refs: BTreeMap<u32, u64>,
    ids: BTreeMap<u32, u32>,
    id_blocks: Vec<u64>,
    frag_blocks: Vec<u64>,
    xattr_blocks: Vec<u64>,
    xattr_kv: u64,
    xattr_ids: u32,
}

/// A mounted squashfs image.
pub struct Squashfs<S: Source, L: RawLock + Default = RawSpinLock> {
    src: S,
    sb: Super,
    root_ino: u32,
    state: Lock<L, Cache>,
}

impl<S: Source, L: RawLock + Default> core::fmt::Debug for Squashfs<S, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Squashfs").field("inodes", &self.sb.inodes).finish_non_exhaustive()
    }
}

struct Ctx<'a, S: Source> {
    fs: &'a SquashfsCore<'a, S>,
    c: &'a mut Cache,
}

/// What an operation needs besides the cache.
struct SquashfsCore<'a, S: Source> {
    src: &'a S,
    sb: &'a Super,
}

fn io(_: IoFailed) -> Errno {
    Errno::EIO
}

impl<S: Source> SquashfsCore<'_, S> {
    fn read(&self, off: u64, len: usize) -> Result<Vec<u8>> {
        let end = off.checked_add(len as u64).ok_or(Errno::EIO)?;
        if end > self.src.size() {
            return Err(Errno::EIO);
        }
        let mut v = vec![0u8; len];
        self.src.read_at(off, &mut v).map_err(io)?;
        Ok(v)
    }

    /// Decompresses one compressed block of this image.
    fn unpack(&self, data: &[u8], max_out: usize) -> Result<Vec<u8>> {
        let r = match self.sb.compression {
            COMP_GZIP => comp::zlib(data, max_out),
            COMP_LZO => comp::lzo::decompress(data, max_out),
            COMP_XZ => comp::xz::decompress(data, max_out),
            COMP_LZ4 => comp::lz4::decompress(data, max_out),
            COMP_ZSTD => comp::zstd::decompress(data, max_out),
            _ => return Err(Errno::EIO),
        };
        r.map_err(|_| Errno::EIO)
    }
}

impl<S: Source> Ctx<'_, S> {
    fn meta(&mut self, abs: u64) -> Result<Arc<Meta>> {
        if let Some(m) = self.c.meta.get(&abs) {
            return Ok(m.clone());
        }
        let h = self.fs.read(abs, 2)?;
        let w = u16_at(&h, 0);
        let size = usize::from(w & 0x7fff);
        if size == 0 || size > METADATA_SIZE {
            return Err(Errno::EIO);
        }
        let raw = self.fs.read(abs + 2, size)?;
        let data = if w & 0x8000 != 0 { raw } else { self.fs.unpack(&raw, METADATA_SIZE)? };
        let m = Arc::new(Meta { data, next: abs + 2 + size as u64 });
        if self.c.meta.len() >= META_CACHE_BLOCKS {
            self.c.meta.clear();
        }
        self.c.meta.insert(abs, m.clone());
        Ok(m)
    }

    /// Reads `out.len()` bytes of the metadata stream from `pos` on, across
    /// block boundaries, and moves `pos` past them.
    fn read_meta(&mut self, pos: &mut Pos, out: &mut [u8]) -> Result<()> {
        let mut done = 0;
        while done < out.len() {
            let m = self.meta(pos.block)?;
            if pos.off >= m.data.len() {
                pos.block = m.next;
                pos.off = 0;
                if pos.block >= self.fs.sb.bytes_used {
                    return Err(Errno::EIO);
                }
                continue;
            }
            let n = (m.data.len() - pos.off).min(out.len() - done);
            out[done..done + n].copy_from_slice(&m.data[pos.off..pos.off + n]);
            done += n;
            pos.off += n;
        }
        Ok(())
    }

    fn take(&mut self, pos: &mut Pos, n: usize) -> Result<Vec<u8>> {
        let mut v = vec![0u8; n];
        self.read_meta(pos, &mut v)?;
        Ok(v)
    }

    /// An id from the id table.
    fn id(&mut self, idx: u16) -> Result<u32> {
        let idx = u32::from(idx);
        if let Some(&v) = self.c.ids.get(&idx) {
            return Ok(v);
        }
        if idx >= u32::from(self.fs.sb.ids) {
            return Err(Errno::EIO);
        }
        if self.c.id_blocks.is_empty() {
            let n = usize::from(self.fs.sb.ids).div_ceil(2048);
            let raw = self.fs.read(self.fs.sb.id_table, n * 8)?;
            self.c.id_blocks = (0..n).map(|i| u64_at(&raw, i * 8)).collect();
        }
        let block = *self.c.id_blocks.get(idx as usize / 2048).ok_or(Errno::EIO)?;
        let mut pos = Pos { block, off: (idx as usize % 2048) * 4 };
        let b = self.take(&mut pos, 4)?;
        let v = u32_at(&b, 0);
        self.c.ids.insert(idx, v);
        Ok(v)
    }

    fn inode_by_ref(&mut self, iref: u64) -> Result<Arc<Inode>> {
        let mut pos = Pos { block: self.fs.sb.inode_table + (iref >> 16), off: (iref & 0xffff) as usize };
        let h = self.take(&mut pos, 16)?;
        let t = u16_at(&h, 0);
        let kind = kind_of(t).ok_or(Errno::EIO)?;
        let mut i = Inode {
            ino: u32_at(&h, 12),
            kind,
            perm: u16_at(&h, 2) & 0o7777,
            uid: 0,
            gid: 0,
            mtime: u32_at(&h, 8),
            nlink: 1,
            size: 0,
            rdev: 0,
            xattr: NO_XATTR,
            dir_block: 0,
            dir_offset: 0,
            dir_len: 0,
            parent: 0,
            frag: NO_FRAGMENT,
            frag_off: 0,
            starts: Vec::new(),
            sizes: Vec::new(),
            sparse: 0,
            target: Vec::new(),
        };
        i.uid = self.id(u16_at(&h, 4))?;
        i.gid = self.id(u16_at(&h, 6))?;
        let bs = u64::from(self.fs.sb.block_size);
        match t {
            1 => {
                let b = self.take(&mut pos, 16)?;
                i.dir_block = u32_at(&b, 0);
                i.nlink = u32_at(&b, 4);
                i.size = u64::from(u16_at(&b, 8));
                i.dir_offset = u16_at(&b, 10);
                i.parent = u32_at(&b, 12);
                i.dir_len = (i.size as u32).saturating_sub(3);
            }
            8 => {
                let b = self.take(&mut pos, 24)?;
                i.nlink = u32_at(&b, 0);
                i.size = u64::from(u32_at(&b, 4));
                i.dir_block = u32_at(&b, 8);
                i.parent = u32_at(&b, 12);
                i.dir_offset = u16_at(&b, 18);
                i.xattr = u32_at(&b, 20);
                i.dir_len = (i.size as u32).saturating_sub(3);
            }
            2 | 9 => {
                let blocks_start;
                if t == 2 {
                    let b = self.take(&mut pos, 16)?;
                    blocks_start = u64::from(u32_at(&b, 0));
                    i.frag = u32_at(&b, 4);
                    i.frag_off = u32_at(&b, 8);
                    i.size = u64::from(u32_at(&b, 12));
                } else {
                    let b = self.take(&mut pos, 40)?;
                    blocks_start = u64_at(&b, 0);
                    i.size = u64_at(&b, 8);
                    i.sparse = u64_at(&b, 16);
                    i.nlink = u32_at(&b, 24);
                    i.frag = u32_at(&b, 28);
                    i.frag_off = u32_at(&b, 32);
                    i.xattr = u32_at(&b, 36);
                }
                let n = if i.frag == NO_FRAGMENT { i.size.div_ceil(bs) } else { i.size / bs } as usize;
                if n > (1 << 24) {
                    return Err(Errno::EIO);
                }
                let raw = self.take(&mut pos, n * 4)?;
                let mut at = blocks_start;
                for k in 0..n {
                    let w = u32_at(&raw, k * 4);
                    i.sizes.push(w);
                    i.starts.push(at);
                    at += u64::from(w & 0x00ff_ffff);
                }
            }
            3 | 10 => {
                let b = self.take(&mut pos, 8)?;
                i.nlink = u32_at(&b, 0);
                let n = u32_at(&b, 4) as usize;
                if n > 1 << 20 {
                    return Err(Errno::EIO);
                }
                i.target = self.take(&mut pos, n)?;
                i.size = n as u64;
                if t == 10 {
                    let x = self.take(&mut pos, 4)?;
                    i.xattr = u32_at(&x, 0);
                }
                i.perm = 0o777;
            }
            4 | 5 | 11 | 12 => {
                let b = self.take(&mut pos, 8)?;
                i.nlink = u32_at(&b, 0);
                i.rdev = u32_at(&b, 4);
                if t >= 11 {
                    let x = self.take(&mut pos, 4)?;
                    i.xattr = u32_at(&x, 0);
                }
            }
            _ => {
                let b = self.take(&mut pos, 4)?;
                i.nlink = u32_at(&b, 0);
                if t >= 13 {
                    let x = self.take(&mut pos, 4)?;
                    i.xattr = u32_at(&x, 0);
                }
            }
        }
        let a = Arc::new(i);
        if self.c.inodes.len() >= INODE_CACHE {
            self.c.inodes.clear();
        }
        self.c.refs.insert(a.ino, iref);
        self.c.inodes.insert(a.ino, a.clone());
        Ok(a)
    }

    fn inode(&mut self, ino: Ino) -> Result<Arc<Inode>> {
        let ino = u32::try_from(ino).map_err(|_| Errno::ENOENT)?;
        if let Some(i) = self.c.inodes.get(&ino) {
            return Ok(i.clone());
        }
        let iref = *self.c.refs.get(&ino).ok_or(Errno::ENOENT)?;
        self.inode_by_ref(iref)
    }

    fn attr(&self, i: &Inode) -> Attr {
        let t = Timespec::new(i64::from(i.mtime), 0);
        Attr {
            ino: Ino::from(i.ino),
            kind: i.kind,
            perm: i.perm,
            nlink: i.nlink,
            uid: i.uid,
            gid: i.gid,
            size: i.size,
            rdev: if matches!(i.kind, FileType::BlockDevice | FileType::CharDevice) {
                decode_rdev(i.rdev)
            } else {
                Default::default()
            },
            atime: t,
            mtime: t,
            ctime: t,
            blocks: if i.kind == FileType::Regular { i.size.div_ceil(512) } else { 0 },
            blksize: 1024,
        }
    }

    /// Walks the entries of directory `d` in the image's order; `f` returns
    /// `true` to stop.
    fn scan_dir(&mut self, d: &Inode, f: &mut dyn FnMut(Entry) -> bool) -> Result<()> {
        if d.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let mut pos = Pos { block: self.fs.sb.dir_table + u64::from(d.dir_block), off: usize::from(d.dir_offset) };
        let mut left = d.dir_len as usize;
        while left >= 12 {
            let h = self.take(&mut pos, 12)?;
            left -= 12;
            let count = u32_at(&h, 0) as usize + 1;
            let start = u64::from(u32_at(&h, 4));
            let base = u32_at(&h, 8);
            if count > 256 {
                return Err(Errno::EIO);
            }
            for _ in 0..count {
                if left < 8 {
                    return Err(Errno::EIO);
                }
                let e = self.take(&mut pos, 8)?;
                let off = u64::from(u16_at(&e, 0));
                let delta = i32::from(u16_at(&e, 2) as i16);
                let t = u16_at(&e, 4);
                let n = usize::from(u16_at(&e, 6)) + 1;
                if left < 8 + n {
                    return Err(Errno::EIO);
                }
                left -= 8 + n;
                let name = self.take(&mut pos, n)?;
                let ino = (i64::from(base) + i64::from(delta)) as u32;
                let kind = kind_of(t).ok_or(Errno::EIO)?;
                let iref = (start << 16) | off;
                self.c.refs.insert(ino, iref);
                if f(Entry { name, ino, kind }) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// The decompressed data block at `start` with the on-disk size word
    /// `word`; sparse blocks are `want` zeros.
    fn data_block(&mut self, start: u64, word: u32, want: usize) -> Result<Arc<Vec<u8>>> {
        let size = (word & 0x00ff_ffff) as usize;
        if size == 0 {
            return Ok(Arc::new(vec![0u8; want]));
        }
        if let Some(b) = self.c.blocks.get(&start) {
            return Ok(b.clone());
        }
        let raw = self.fs.read(start, size)?;
        let bs = self.fs.sb.block_size as usize;
        let data = if word & 0x0100_0000 != 0 { raw } else { self.fs.unpack(&raw, bs)? };
        if self.c.block_bytes + data.len() > BLOCK_CACHE_BYTES {
            self.c.blocks.clear();
            self.c.block_bytes = 0;
        }
        self.c.block_bytes += data.len();
        let a = Arc::new(data);
        self.c.blocks.insert(start, a.clone());
        Ok(a)
    }

    fn fragment(&mut self, idx: u32) -> Result<Arc<Vec<u8>>> {
        if idx >= self.fs.sb.fragments {
            return Err(Errno::EIO);
        }
        if self.c.frag_blocks.is_empty() {
            let n = (self.fs.sb.fragments as usize).div_ceil(512);
            let raw = self.fs.read(self.fs.sb.frag_table, n * 8)?;
            self.c.frag_blocks = (0..n).map(|i| u64_at(&raw, i * 8)).collect();
        }
        let block = *self.c.frag_blocks.get(idx as usize / 512).ok_or(Errno::EIO)?;
        let mut pos = Pos { block, off: (idx as usize % 512) * 16 };
        let e = self.take(&mut pos, 16)?;
        let (start, word) = (u64_at(&e, 0), u32_at(&e, 8));
        let bs = self.fs.sb.block_size as usize;
        self.data_block(start, word, bs)
    }

    fn read_file(&mut self, i: &Inode, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset >= i.size {
            return Ok(0);
        }
        let want = (buf.len() as u64).min(i.size - offset) as usize;
        let bs = u64::from(self.fs.sb.block_size);
        let mut done = 0usize;
        while done < want {
            let pos = offset + done as u64;
            let bi = (pos / bs) as usize;
            let within = (pos % bs) as usize;
            let block_len = (i.size - bi as u64 * bs).min(bs) as usize;
            let data;
            let from;
            if bi < i.sizes.len() {
                data = self.data_block(i.starts[bi], i.sizes[bi], block_len)?;
                from = within;
            } else if i.frag != NO_FRAGMENT {
                data = self.fragment(i.frag)?;
                from = i.frag_off as usize + within;
            } else {
                return Err(Errno::EIO);
            }
            let avail = block_len - within;
            let n = avail.min(want - done);
            let chunk = data.get(from..from + n).ok_or(Errno::EIO)?;
            buf[done..done + n].copy_from_slice(chunk);
            done += n;
        }
        Ok(done)
    }

    /// The extended attributes of `i`: full name and value.
    fn xattrs(&mut self, i: &Inode) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut out = Vec::new();
        if i.xattr == NO_XATTR || self.fs.sb.xattr_table == u64::MAX {
            return Ok(out);
        }
        if self.c.xattr_blocks.is_empty() {
            let h = self.fs.read(self.fs.sb.xattr_table, 16)?;
            self.c.xattr_kv = u64_at(&h, 0);
            self.c.xattr_ids = u32_at(&h, 8);
            let n = (self.c.xattr_ids as usize).div_ceil(512);
            let raw = self.fs.read(self.fs.sb.xattr_table + 16, n * 8)?;
            self.c.xattr_blocks = (0..n).map(|k| u64_at(&raw, k * 8)).collect();
        }
        if i.xattr >= self.c.xattr_ids {
            return Err(Errno::EIO);
        }
        let block = *self.c.xattr_blocks.get(i.xattr as usize / 512).ok_or(Errno::EIO)?;
        let mut pos = Pos { block, off: (i.xattr as usize % 512) * 16 };
        let e = self.take(&mut pos, 16)?;
        let (xref, count) = (u64_at(&e, 0), u32_at(&e, 8));
        let kv = self.c.xattr_kv;
        let mut kp = Pos { block: kv + (xref >> 16), off: (xref & 0xffff) as usize };
        for _ in 0..count {
            let h = self.take(&mut kp, 4)?;
            let t = u16_at(&h, 0);
            let n = usize::from(u16_at(&h, 2));
            let name = self.take(&mut kp, n)?;
            let vs = self.take(&mut kp, 4)?;
            let vlen = u32_at(&vs, 0) as usize;
            if vlen > 1 << 20 {
                return Err(Errno::EIO);
            }
            let value = if t & 0x100 != 0 {
                let r = self.take(&mut kp, 8)?;
                let vref = u64_at(&r, 0);
                let mut vp = Pos { block: kv + (vref >> 16), off: (vref & 0xffff) as usize };
                let l = self.take(&mut vp, 4)?;
                self.take(&mut vp, u32_at(&l, 0) as usize)?
            } else {
                self.take(&mut kp, vlen)?
            };
            let prefix: &[u8] = match t & 0xff {
                0 => b"user.",
                1 => b"trusted.",
                2 => b"security.",
                _ => return Err(Errno::EIO),
            };
            let mut full = prefix.to_vec();
            full.extend_from_slice(&name);
            out.push((full, value));
        }
        Ok(out)
    }
}

impl<S: Source, L: RawLock + Default> Squashfs<S, L> {
    /// Mounts the image `src` holds.
    pub fn mount(src: S) -> core::result::Result<Self, MountError> {
        let size = src.size();
        if size < MIN_DEVICE {
            return Err(MountError::Io);
        }
        let mut h = [0u8; SUPER_SIZE];
        src.read_at(0, &mut h).map_err(|_| MountError::Io)?;
        if u32_at(&h, 0) != MAGIC {
            return Err(MountError::Invalid);
        }
        let sb = Super::parse(&h);
        if sb.major != 4 {
            return Err(MountError::Invalid);
        }
        let bs = sb.block_size;
        if !bs.is_power_of_two() || !(4096..=1 << 20).contains(&bs) || u32::from(sb.block_log) != bs.trailing_zeros() {
            return Err(MountError::Invalid);
        }
        if !matches!(sb.compression, COMP_GZIP | COMP_LZO | COMP_XZ | COMP_LZ4 | COMP_ZSTD) {
            return Err(MountError::Invalid);
        }
        if sb.bytes_used < SUPER_SIZE as u64 || sb.bytes_used > size {
            return Err(MountError::Invalid);
        }
        for t in [sb.inode_table, sb.dir_table] {
            if t >= sb.bytes_used {
                return Err(MountError::Invalid);
            }
        }
        let mut fs = Squashfs { src, sb, root_ino: 0, state: Lock::new(Cache::default()) };
        // The root inode proves the metadata can be read and decoded.
        let root = {
            let core = SquashfsCore { src: &fs.src, sb: &fs.sb };
            let mut cache = Cache::default();
            let mut ctx = Ctx { fs: &core, c: &mut cache };
            let i = ctx.inode_by_ref(fs.sb.root).map_err(|_| MountError::Io)?;
            if i.kind != FileType::Directory {
                return Err(MountError::Invalid);
            }
            let ino = i.ino;
            let refs = (ino, fs.sb.root);
            *fs.state.get_mut() = cache;
            refs
        };
        fs.root_ino = root.0;
        Ok(fs)
    }

    /// The superblock's block size.
    pub fn block_size(&self) -> u32 {
        self.sb.block_size
    }

    fn with<R>(&self, f: impl FnOnce(&mut Ctx<'_, S>) -> Result<R>) -> Result<R> {
        let core = SquashfsCore { src: &self.src, sb: &self.sb };
        let mut cache = self.state.lock();
        let mut ctx = Ctx { fs: &core, c: &mut cache };
        f(&mut ctx)
    }
}

impl<S: Source, L: RawLock + Default> FileSystem for Squashfs<S, L> {
    fn info(&self) -> FsInfo {
        FsInfo { name: "squashfs", read_only: true, cache_lookups: true }
    }

    fn root(&self) -> Ino {
        Ino::from(self.root_ino)
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        self.with(|c| {
            let d = c.inode(dir)?;
            if d.kind != FileType::Directory {
                return Err(Errno::ENOTDIR);
            }
            if name == b".." {
                return Ok(if d.ino == self.root_ino { Ino::from(self.root_ino) } else { Ino::from(d.parent) });
            }
            let mut found = None;
            c.scan_dir(&d, &mut |e| {
                if e.name == name {
                    found = Some(e.ino);
                    true
                } else {
                    // Entries are sorted: past the name there is no match.
                    e.name.as_slice() > name
                }
            })?;
            found.map(Ino::from).ok_or(Errno::ENOENT)
        })
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        self.with(|c| {
            let i = c.inode(ino)?;
            Ok(c.attr(&i))
        })
    }

    fn setattr(&self, _ino: Ino, _set: &SetAttr) -> Result<Attr> {
        Err(Errno::EROFS)
    }

    fn create(&self, _dir: Ino, _name: &[u8], _node: &NewNode) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn mkdir(&self, _dir: Ino, _name: &[u8], _perm: u16, _uid: u32, _gid: u32) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn symlink(&self, _dir: Ino, _name: &[u8], _target: &[u8], _uid: u32, _gid: u32) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn link(&self, _dir: Ino, _name: &[u8], _ino: Ino) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn unlink(&self, _dir: Ino, _name: &[u8]) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn rmdir(&self, _dir: Ino, _name: &[u8]) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn rename(&self, _od: Ino, _on: &[u8], _nd: Ino, _nn: &[u8], _flags: RenameFlags) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        self.with(|c| {
            let i = c.inode(ino)?;
            if i.kind != FileType::Symlink {
                return Err(Errno::EINVAL);
            }
            Ok(i.target.clone())
        })
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.with(|c| {
            let i = c.inode(ino)?;
            match i.kind {
                FileType::Regular => c.read_file(&i, offset, buf),
                FileType::Directory => Err(Errno::EISDIR),
                _ => Err(Errno::EINVAL),
            }
        })
    }

    fn write(&self, _ino: Ino, _offset: u64, _buf: &[u8]) -> Result<usize> {
        Err(Errno::EROFS)
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        // Positions: 0 is ".", 1 is "..", 2 + k is the k-th entry.
        let (d, parent) = self.with(|c| {
            let d = c.inode(dir)?;
            if d.kind != FileType::Directory {
                return Err(Errno::ENOTDIR);
            }
            let parent = if d.ino == self.root_ino { self.root_ino } else { d.parent };
            Ok((d, parent))
        })?;
        if pos == 0 && !emit(&DirEntry { ino: Ino::from(d.ino), kind: FileType::Directory, name: b".", next: 1 }) {
            return Ok(());
        }
        if pos <= 1 && !emit(&DirEntry { ino: Ino::from(parent), kind: FileType::Directory, name: b"..", next: 2 }) {
            return Ok(());
        }
        let mut skip = pos.saturating_sub(2) as usize;
        let mut index = skip as u64;
        loop {
            // A batch under the lock, then the callback without it.
            let mut batch: Vec<Entry> = Vec::new();
            let mut seen = 0usize;
            let first = skip;
            self.with(|c| {
                c.scan_dir(&d, &mut |e| {
                    if seen >= first {
                        batch.push(e);
                    }
                    seen += 1;
                    batch.len() >= 64
                })
            })?;
            if batch.is_empty() {
                return Ok(());
            }
            let n = batch.len();
            for e in batch {
                index += 1;
                let de = DirEntry { ino: Ino::from(e.ino), kind: e.kind, name: &e.name, next: 2 + index };
                if !emit(&de) {
                    return Ok(());
                }
            }
            skip += n;
            if n < 64 {
                return Ok(());
            }
        }
    }

    fn statfs(&self) -> Result<FsStats> {
        let bs = u64::from(self.sb.block_size);
        Ok(FsStats {
            magic: SQUASHFS_MAGIC,
            block_size: bs,
            blocks: self.sb.bytes_used.div_ceil(bs),
            files: u64::from(self.sb.inodes),
            name_max: 256,
            fragment_size: bs,
            ..FsStats::default()
        })
    }

    fn getxattr(&self, ino: Ino, name: &[u8]) -> Result<Vec<u8>> {
        self.with(|c| {
            let i = c.inode(ino)?;
            c.xattrs(&i)?.into_iter().find(|(n, _)| n == name).map(|(_, v)| v).ok_or(Errno::ENODATA)
        })
    }

    fn setxattr(&self, _ino: Ino, _name: &[u8], _value: &[u8], _how: SetMode) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn listxattr(&self, ino: Ino) -> Result<Vec<u8>> {
        self.with(|c| {
            let i = c.inode(ino)?;
            let mut out = Vec::new();
            for (n, _) in c.xattrs(&i)? {
                out.extend_from_slice(&n);
                out.push(0);
            }
            Ok(out)
        })
    }

    fn removexattr(&self, _ino: Ino, _name: &[u8]) -> Result<()> {
        Err(Errno::EROFS)
    }
}
