//! Unit tests: the `poly-block` adapters, mount checks, running out of
//! space, and read-only volumes. Images come from [`crate::format`]; the
//! integration tests check those and much more against e2fsprogs.

extern crate std;

use super::*;
use crate::layout::{RO_COMPAT_LARGE_FILE, put32};
use poly_block::{BufferCache, CacheSlot, DeviceSlot, IoBuf, Part, QueueStorage, RamDisk, RequestQueue, RequestSlot};
use poly_vfs::{DeviceId, FileSystem, FileType, NewNode, RenameFlags, SetAttr};
use std::vec::Vec;

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1_700_000_000, 5)
    }
}

fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock)
}

const REG: NewNode = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::new(0, 0) };

fn formatted(bytes: usize, sector: u32, opts: &FormatOptions) -> Vec<u8> {
    let mut mem = vec![0u8; bytes];
    let io = DeviceIo::new(RamDisk::new(&mut mem, sector).unwrap());
    format(&io, opts).unwrap();
    mem
}

fn k1() -> FormatOptions {
    FormatOptions { block_size: 1024, ..FormatOptions::default() }
}

fn read_all(fs: &dyn FileSystem, ino: Ino) -> Vec<u8> {
    let mut buf = vec![0u8; fs.getattr(ino).unwrap().size as usize];
    assert_eq!(fs.read(ino, 0, &mut buf).unwrap(), buf.len());
    buf
}

fn data(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31) ^ seed).collect()
}

use poly_vfs::Ino;

#[test]
fn through_a_request_queue_and_buffer_cache() {
    let mut mem = formatted(4 << 20, 512, &k1());
    let mut requests = vec![RequestSlot::EMPTY; 16];
    let mut device = vec![DeviceSlot::EMPTY; 4];
    let mut segments = vec![IoBuf::EMPTY; 16];
    let mut parts = vec![Part::EMPTY; 16];
    let mut slots = vec![CacheSlot::EMPTY; 64];
    let mut cache_mem = vec![0u8; 64 * 512];
    let mut disk = RamDisk::new(&mut mem, 512).unwrap();
    // Small device limits, so the queue splits.
    assert!(disk.set_limits(4, 2, 1024));
    let storage =
        QueueStorage { requests: &mut requests, device: &mut device, segments: &mut segments, parts: &mut parts };
    let queue = RequestQueue::new(disk, storage).unwrap();
    let cache = BufferCache::new(&mut slots, &mut cache_mem, 512).unwrap();
    let io = CachedIo::new(cache, queue).unwrap();
    let fs = Ext2::mount(&io, clock(), MountOptions::default()).unwrap();
    let f = fs.create(fs.root(), b"file", &REG).unwrap();
    fs.write(f, 0, &data(1, 300_000)).unwrap();
    fs.symlink(fs.root(), b"link", b"file", 0, 0).unwrap();
    assert_eq!(read_all(&fs, f), data(1, 300_000));
    // Written back only at sync.
    assert!(io.with(|c, _| c.dirty_count()) > 0);
    fs.unmount().unwrap();
    assert_eq!(io.with(|c, _| c.dirty_count()), 0);
    drop(fs);
    let (_, queue) = io.into_inner();
    let disk = queue.into_device();
    // What reached the RAM disk mounts and reads the same.
    let io = DeviceIo::new(disk);
    let fs = Ext2::mount(&io, clock(), MountOptions::default()).unwrap();
    assert!(fs.stats().was_clean);
    let f = fs.lookup(fs.root(), b"file").unwrap();
    assert_eq!(read_all(&fs, f), data(1, 300_000));
    assert_eq!(fs.readlink(fs.lookup(fs.root(), b"link").unwrap()).unwrap(), b"file");
}

#[test]
fn device_blocks_larger_than_file_system_blocks() {
    // 1 KiB file system blocks on a disk of 4 KiB sectors: read-modify-write.
    let mut mem = formatted(4 << 20, 4096, &k1());
    let io = DeviceIo::new(RamDisk::new(&mut mem, 4096).unwrap());
    let fs = Ext2::mount(&io, clock(), MountOptions::default()).unwrap();
    let d = fs.mkdir(fs.root(), b"d", 0o755, 0, 0).unwrap();
    for i in 0..50u8 {
        let f = fs.create(d, &[b'f', b'0' + i / 10, b'0' + i % 10], &REG).unwrap();
        fs.write(f, i as u64 * 100, &data(i, 3000)).unwrap();
    }
    fs.unmount().unwrap();
    drop(fs);
    let fs = Ext2::mount(&io, clock(), MountOptions::default()).unwrap();
    let d = fs.lookup(fs.root(), b"d").unwrap();
    for i in 0..50u8 {
        let f = fs.lookup(d, &[b'f', b'0' + i / 10, b'0' + i % 10]).unwrap();
        let got = read_all(&fs, f);
        assert_eq!(&got[i as usize * 100..], &data(i, 3000)[..]);
        assert!(got[..i as usize * 100].iter().all(|&b| b == 0));
    }
}

#[test]
fn mount_checks() {
    let good = formatted(2 << 20, 512, &k1());
    let mount = |img: &mut Vec<u8>| {
        let io = DeviceIo::new(RamDisk::new(img, 512).unwrap());
        Ext2::mount(io, clock(), MountOptions::default()).map(|fs| fs.read_only())
    };
    assert_eq!(mount(&mut good.clone()), Ok(false));
    let mut bad = good.clone();
    bad[1024 + 56] = 0;
    assert_eq!(mount(&mut bad), Err(MountError::NotExt2));
    let mut bad = good.clone();
    put32(&mut bad, 1024 + 96, 0x2 | 0x40); // filetype + extents
    assert_eq!(mount(&mut bad), Err(MountError::IncompatFeatures(0x40)));
    assert_eq!(MountError::IncompatFeatures(0x40).errno(), Errno::EINVAL);
    let mut bad = good.clone();
    put32(&mut bad, 1024 + 76, 2);
    assert_eq!(mount(&mut bad), Err(MountError::Unsupported("revision")));
    let mut bad = good.clone();
    put32(&mut bad, 1024 + 24, 3);
    assert_eq!(mount(&mut bad), Err(MountError::Unsupported("block size")));
    let mut bad = good.clone();
    put32(&mut bad, 1024 + 4, 1 << 30);
    assert_eq!(mount(&mut bad), Err(MountError::TooSmall));
    let mut bad = good.clone();
    put32(&mut bad, 1024 + 32, 0);
    assert_eq!(mount(&mut bad), Err(MountError::Corrupt("group size")));
    // A descriptor pointing outside the volume.
    let mut bad = good.clone();
    put32(&mut bad, 2048 + 8, 1 << 30);
    assert_eq!(mount(&mut bad), Err(MountError::Corrupt("group descriptor")));
    // An unknown read-only-compatible feature: mounts, read-only.
    let mut ro = good.clone();
    put32(&mut ro, 1024 + 100, 0x1 | RO_COMPAT_LARGE_FILE | 0x400);
    assert_eq!(mount(&mut ro), Ok(true));
    assert_eq!(mount(&mut vec![0u8; 1024]).map(|_| ()), Err(MountError::TooSmall));
}

#[test]
fn read_only_volumes_change_nothing() {
    let mut mem = formatted(2 << 20, 512, &k1());
    {
        let io = DeviceIo::new(RamDisk::new(&mut mem, 512).unwrap());
        let fs = Ext2::mount(&io, clock(), MountOptions::default()).unwrap();
        let f = fs.create(fs.root(), b"f", &REG).unwrap();
        fs.write(f, 0, b"data").unwrap();
        fs.unmount().unwrap();
    }
    let before = mem.clone();
    {
        let mut disk = RamDisk::new(&mut mem, 512).unwrap();
        disk.set_read_only(true);
        let fs = Ext2::mount(DeviceIo::new(disk), clock(), MountOptions::default()).unwrap();
        assert!(fs.info().read_only);
        let f = fs.lookup(fs.root(), b"f").unwrap();
        assert_eq!(read_all(&fs, f), b"data");
        assert_eq!(fs.write(f, 0, b"x"), Err(Errno::EROFS));
        assert_eq!(fs.mkdir(fs.root(), b"d", 0o755, 0, 0), Err(Errno::EROFS));
        fs.sync().unwrap();
        fs.unmount().unwrap();
    }
    assert!(mem == before, "a read-only mount wrote to the disk");
}

#[test]
fn running_out_of_space_and_inodes() {
    let mut mem = formatted(1 << 20, 512, &FormatOptions { bytes_per_inode: 16384, ..k1() });
    let io = DeviceIo::new(RamDisk::new(&mut mem, 512).unwrap());
    let fs = Ext2::mount(&io, clock(), MountOptions::default()).unwrap();
    let r = fs.root();
    let start = fs.stats();
    // Fill the disk: the write that runs out reports what fit.
    let f = fs.create(r, b"fill", &REG).unwrap();
    let mut total = 0;
    loop {
        match fs.write(f, total as u64, &[7u8; 50_000]) {
            Ok(n) => {
                total += n;
                if n < 50_000 {
                    break;
                }
            }
            Err(e) => {
                assert_eq!(e, Errno::ENOSPC);
                break;
            }
        }
    }
    assert!(total > 700_000, "{total}");
    assert_eq!(fs.write(f, total as u64, b"x"), Err(Errno::ENOSPC));
    assert_eq!(fs.mkdir(r, b"d", 0o755, 0, 0), Err(Errno::ENOSPC));
    // Freed blocks come back after the deletion is durable, without a sync
    // by the caller.
    fs.unlink(r, b"fill").unwrap();
    let g = fs.create(r, b"again", &REG).unwrap();
    assert_eq!(fs.write(g, 0, &vec![1u8; total]).unwrap(), total);
    fs.unlink(r, b"again").unwrap();
    fs.sync().unwrap();
    let end = fs.stats();
    assert_eq!((end.free_blocks, end.free_inodes), (start.free_blocks, start.free_inodes));
    // Inodes run out too.
    let mut n = 0;
    while fs.create(r, std::format!("i{n}").as_bytes(), &REG).is_ok() {
        n += 1;
    }
    assert_eq!(n as u64, start.free_inodes);
    assert_eq!(fs.create(r, b"one-more", &REG), Err(Errno::ENOSPC));
    for i in 0..n {
        fs.unlink(r, std::format!("i{i}").as_bytes()).unwrap();
    }
    fs.sync().unwrap();
    assert_eq!(fs.stats().free_inodes, start.free_inodes);
    fs.unmount().unwrap();
}

#[test]
fn attributes_and_flags() {
    let mut mem = formatted(2 << 20, 512, &FormatOptions { inode_size: 128, ..k1() });
    let io = DeviceIo::new(RamDisk::new(&mut mem, 512).unwrap());
    let fs = Ext2::mount(&io, clock(), MountOptions::default()).unwrap();
    let r = fs.root();
    let f = fs.create(r, b"f", &NewNode { uid: 70_000, gid: 80_000, ..REG }).unwrap();
    let a = fs.getattr(f).unwrap();
    assert_eq!((a.uid, a.gid, a.nlink, a.size, a.blksize), (70_000, 80_000, 1, 0, 1024));
    // 128-byte inodes have no nanoseconds.
    assert_eq!(a.mtime, Timespec::new(1_700_000_000, 0));
    let a = fs
        .setattr(
            f,
            &SetAttr {
                size: Some(5000),
                mtime: Some(poly_vfs::SetTime::At(Timespec::new(-7, 0))),
                ..SetAttr::default()
            },
        )
        .unwrap();
    assert_eq!((a.size, a.blocks, a.mtime.sec), (5000, 0, -7));
    assert_eq!(fs.setattr(r, &SetAttr { size: Some(0), ..SetAttr::default() }), Err(Errno::EISDIR));
    let fifo = fs.create(r, b"p", &NewNode { kind: FileType::Fifo, ..REG }).unwrap();
    assert_eq!(fs.truncate(fifo, 0), Err(Errno::EINVAL));
    assert_eq!(fs.read(fifo, 0, &mut [0; 4]), Err(Errno::EINVAL));
    assert_eq!(fs.write(r, 0, b"x"), Err(Errno::EISDIR));
    assert_eq!(fs.readlink(f), Err(Errno::EINVAL));
    assert_eq!(fs.symlink(r, b"s", b"", 0, 0), Err(Errno::ENOENT));
    assert_eq!(fs.symlink(r, b"s", &[b'x'; 1024], 0, 0), Err(Errno::ENAMETOOLONG));
    assert_eq!(fs.create(r, &[b'n'; 256], &REG), Err(Errno::ENAMETOOLONG));
    assert_eq!(fs.lookup(f, b"x"), Err(Errno::ENOTDIR));
    assert_eq!(fs.getattr(0), Err(Errno::ENOENT));
    assert_eq!(fs.getattr(1 << 40), Err(Errno::ENOENT));
    // Immutable and append-only files (flags set by other tools).
    let g = fs.create(r, b"g", &REG).unwrap();
    fs.write(g, 0, b"abc").unwrap();
    let set_flags = |flags: u32| {
        let mut st = fs.state.lock();
        let mut c = fs.ctx(&mut st);
        let mut i = c.read_inode(g as u32).unwrap();
        i.flags = flags;
        c.write_inode(g as u32, &i).unwrap();
    };
    set_flags(layout::FL_APPEND);
    assert_eq!(fs.write(g, 0, b"x"), Err(Errno::EPERM));
    assert_eq!(fs.append(g, b"def"), Ok((3, 3)));
    assert_eq!(fs.truncate(g, 0), Err(Errno::EPERM));
    assert_eq!(fs.unlink(r, b"g"), Err(Errno::EPERM));
    set_flags(layout::FL_IMMUTABLE);
    assert_eq!(fs.append(g, b"x").map(|_| ()), Err(Errno::EPERM));
    assert_eq!(fs.setattr(g, &SetAttr { perm: Some(0), ..SetAttr::default() }).map(|_| ()), Err(Errno::EPERM));
    assert_eq!(fs.link(r, b"g2", g), Err(Errno::EPERM));
    assert_eq!(fs.rename(r, b"g", r, b"g3", RenameFlags::default()), Err(Errno::EPERM));
    set_flags(0);
    assert_eq!(read_all(&fs, g), b"abcdef");
    fs.unmount().unwrap();
}

#[test]
fn unordered_mounts_take_no_barriers() {
    struct Counting<'a> {
        inner: DeviceIo<RamDisk<'a>>,
        flushes: core::sync::atomic::AtomicUsize,
    }
    impl BlockIo for Counting<'_> {
        fn block_size(&self) -> u32 {
            self.inner.block_size()
        }
        fn blocks(&self) -> u64 {
            self.inner.blocks()
        }
        fn read(&self, first: u64, buf: &mut [u8]) -> core::result::Result<(), poly_block::IoError> {
            self.inner.read(first, buf)
        }
        fn write(&self, first: u64, buf: &[u8]) -> core::result::Result<(), poly_block::IoError> {
            self.inner.write(first, buf)
        }
        fn flush(&self) -> core::result::Result<(), poly_block::IoError> {
            self.flushes.fetch_add(1, Ordering::Relaxed);
            self.inner.flush()
        }
    }
    for ordered in [true, false] {
        let mut mem = formatted(4 << 20, 512, &k1());
        let io = Counting {
            inner: DeviceIo::new(RamDisk::new(&mut mem, 512).unwrap()),
            flushes: core::sync::atomic::AtomicUsize::new(0),
        };
        let fs = Ext2::mount(&io, clock(), MountOptions { ordered, ..MountOptions::default() }).unwrap();
        let before = io.flushes.load(Ordering::Relaxed);
        let r = fs.root();
        for i in 0..20u8 {
            let d = fs.mkdir(r, &[b'd', b'a' + i], 0o755, 0, 0).unwrap();
            let f = fs.create(d, b"f", &REG).unwrap();
            fs.write(f, 0, &data(i, 100_000)).unwrap();
            fs.rename(d, b"f", r, &[b'f', b'a' + i], RenameFlags::default()).unwrap();
        }
        let flushes = io.flushes.load(Ordering::Relaxed) - before;
        if ordered {
            assert!(flushes >= 60, "{flushes} flushes");
        } else {
            assert_eq!(flushes, 0);
        }
        fs.unmount().unwrap();
    }
}
