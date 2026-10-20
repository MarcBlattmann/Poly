//! Kernel tests of disk file systems and the page cache: ext2 and ext4 on
//! the RAM disk (every boot), and with `poly.test.ext2` / `poly.test.ext4`
//! the ext2 and ext4 test disks that `cargo xtask` builds, mounted at
//! [`TEST_MOUNT`] and [`TEST_EXT4_MOUNT`] by `poly.mount=`.

use super::cache::{self, FileSource};
use super::disk::{self, DiskIo};
use super::*;
use crate::frames::FRAMES;
use crate::kernel_test;
use crate::ktest::TestEnv;
use crate::user::memory::{Layout, PageSource, Placement, Prot};
use crate::user::{FileRef, Memory};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_pagecache::{Key, PAGE_SIZE};
use poly_vfs::FileSystem;

/// Kernel flag saying the ext2 test disk is attached and mounted at
/// [`TEST_MOUNT`] (`cargo xtask` passes it with `poly.mount=virtio2:/mnt`).
pub const TEST_EXT2_FLAG: &str = "poly.test.ext2";
/// Where the boot tests mount the ext2 test disk, and which disk it is.
pub const TEST_MOUNT: &str = "/mnt";
pub const TEST_DISK: &str = "virtio2";

/// `/hello.txt` on the test disk. Must match `tools/xtask/src/ext2disk.rs`.
const HELLO: &[u8] = b"Hello from ext2 on virtio-blk!\n";
/// Length of `/dir/big.bin`: past the 12 direct blocks of 4 KiB, not a
/// whole number of pages.
const BIG_LEN: usize = 300_000;

/// Byte `i` of `/dir/big.bin`. Must match `tools/xtask/src/ext2disk.rs`.
fn big(i: usize) -> u8 {
    (i.wrapping_mul(13) ^ (i / 4096)) as u8
}

fn e(err: Errno) -> &'static str {
    err.name()
}

/// A file name no other run of the tests has used (the shell's `test` can
/// run them again on the same disk).
fn scratch(dir: &str, what: &str) -> String {
    static RUN: AtomicU32 = AtomicU32::new(0);
    format!("{dir}/{what}-{}", RUN.fetch_add(1, Ordering::Relaxed))
}

fn write_file(files: &Files, path: &str, data: &[u8]) -> Result<()> {
    let fd = files.open(path.as_bytes(), OpenFlags::RDWR | OpenFlags::CREAT | OpenFlags::TRUNC, 0o644)?;
    let written = files.write(fd, data);
    files.close(fd)?;
    match written? {
        n if n == data.len() => Ok(()),
        _ => Err(Errno::EIO),
    }
}

fn read_file(files: &Files, path: &str) -> Result<Vec<u8>> {
    files.read_to_end(path.as_bytes(), 1 << 20).map(|(data, _)| data)
}

fn pattern(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed) ^ (i >> 12) as u8).collect()
}

/// The disk mount at [`TEST_MOUNT`], when the boot has the test disk.
fn test_mount(env: &TestEnv) -> core::result::Result<Option<Arc<disk::DiskMount>>, &'static str> {
    if !env.boot.has_flag(TEST_EXT2_FLAG) {
        return Ok(None);
    }
    let m = disk::mounts().into_iter().find(|m| m.path == TEST_MOUNT);
    let m = m.ok_or("poly.test.ext2 is set but nothing is mounted at /mnt")?;
    if m.disk.name() != TEST_DISK {
        return Err("/mnt is not the ext2 test disk (virtio2)");
    }
    Ok(Some(m))
}

kernel_test! {
    /// The RAM disk, formatted as ext2 here, mounted through the page cache
    /// at a directory that does not exist yet: files written there are on
    /// the disk for a fresh mount; the volume is clean after unmounting.
    fn ext2_on_the_ram_disk_through_the_page_cache() -> TestResult {
        let ram = crate::block::ram().ok_or("no RAM disk")?;
        let opts = poly_ext2::FormatOptions { block_size: 1024, bytes_per_inode: 4096, ..Default::default() };
        poly_ext2::format(&DiskIo(ram), &opts).map_err(e)?;
        let at = scratch("/tmp", "ram-ext2");
        disk::mount("ram0", &at, false).map_err(|_| "mounting ext2 on ram0 failed")?;
        if disk::mount("ram0", "/tmp/again", false).map(|_| ()) != Err(disk::MountError::Busy) {
            return Err("ram0 mounted twice");
        }
        let f = Files::new();
        let data = pattern(1, 3 * PAGE_SIZE + 100);
        let result = (|| {
            f.mkdirat(AT_FDCWD, format!("{at}/d").as_bytes(), 0o755).map_err(e)?;
            write_file(&f, &format!("{at}/d/file"), &data).map_err(e)?;
            if read_file(&f, &format!("{at}/d/file")).map_err(e)? != data {
                return Err("a file on ext2 read back wrong through the cache");
            }
            Ok(())
        })();
        let unmounted = disk::unmount(&at);
        result?;
        unmounted.map_err(|_| "unmounting ram0 failed")?;
        // Straight from the disk, without the cache: the volume is clean
        // and has the file.
        let ext2 = poly_ext2::Ext2::mount(DiskIo(ram), clock(), poly_ext2::MountOptions::default())
            .map_err(|_| "the unmounted volume does not mount again")?;
        if !ext2.stats().was_clean {
            return Err("unmounting did not mark the volume clean");
        }
        let d = ext2.lookup(ext2.root(), b"d").map_err(e)?;
        let file = ext2.lookup(d, b"file").map_err(e)?;
        let mut back = vec![0u8; data.len()];
        if ext2.read(file, 0, &mut back).map_err(e)? != data.len() || back != data {
            return Err("the file is not on the disk");
        }
        ext2.unmount().map_err(e)
    }
}

kernel_test! {
    /// The test disk's files, as `cargo xtask` wrote them, read through the
    /// VFS and the page cache.
    fn ext2_test_disk_is_mounted_with_its_files(env: &TestEnv) -> TestResult {
        let Some(m) = test_mount(env)? else { return Ok(()) };
        let f = Files::new();
        if read_file(&f, "/mnt/hello.txt").map_err(e)? != HELLO {
            return Err("/mnt/hello.txt has the wrong contents");
        }
        let big_bin = read_file(&f, "/mnt/dir/big.bin").map_err(e)?;
        if big_bin.len() != BIG_LEN || big_bin.iter().enumerate().any(|(i, &b)| b != big(i)) {
            return Err("/mnt/dir/big.bin has the wrong contents");
        }
        if f.readlinkat(AT_FDCWD, b"/mnt/dir/link").map_err(e)? != b"../hello.txt" {
            return Err("/mnt/dir/link points elsewhere");
        }
        if read_file(&f, "/mnt/dir/link").map_err(e)? != HELLO {
            return Err("following /mnt/dir/link does not reach hello.txt");
        }
        let st = f.stat(b"/mnt/empty").map_err(e)?;
        if st.size != 0 || st.kind() != Some(FileType::Regular) || st.perm() != 0o600 {
            return Err("/mnt/empty is not an empty 0600 file");
        }
        let names: Vec<_> = f.read_dir(b"/mnt").map_err(e)?.into_iter().map(|(n, _, _)| n).collect();
        for want in [b"hello.txt".as_slice(), b"dir", b"empty", b"lost+found"] {
            if !names.iter().any(|n| n == want) {
                return Err("an entry of the test disk's root is missing");
            }
        }
        if m.read_only || !m.ext2.stats().was_clean {
            return Err("the test disk is not mounted read-write, or was not clean");
        }
        // Read through the cache: the big file's pages are cached now.
        let ino = f.stat(b"/mnt/dir/big.bin").map_err(e)?.ino;
        if m.cached.cached_page(ino, (BIG_LEN / PAGE_SIZE) as u64).is_none() {
            return Err("reading a file did not leave its pages in the page cache");
        }
        Ok(())
    }
}

kernel_test! {
    /// Data written to the test disk is there after unmounting it and
    /// mounting it again, with the disk's buffer cache dropped in between
    /// (so it comes from the device); a read-only mount refuses writes.
    fn ext2_writes_survive_a_remount(env: &TestEnv) -> TestResult {
        let Some(m) = test_mount(env)? else { return Ok(()) };
        let f = Files::new();
        let path = scratch(TEST_MOUNT, "remount");
        let data = pattern(2, 70_000);
        write_file(&f, &path, &data).map_err(e)?;
        disk::unmount(TEST_MOUNT).map_err(|_| "unmounting /mnt failed (busy?)")?;
        if m.disk.lock().stats().dirty != 0 {
            return Err("unmounting left dirty blocks in the disk's cache");
        }
        m.disk.lock().invalidate();
        drop(m);
        let ro = disk::mount(TEST_DISK, TEST_MOUNT, true).map_err(|_| "mounting read-only failed")?;
        let result = (|| {
            if !ro.read_only || read_file(&f, &path).map_err(e)? != data {
                return Err("the data did not survive unmounting");
            }
            if write_file(&f, &path, b"x") != Err(Errno::EROFS) {
                return Err("a read-only mount took a write");
            }
            Ok(())
        })();
        disk::unmount(TEST_MOUNT).map_err(|_| "unmounting the read-only mount failed")?;
        disk::mount(TEST_DISK, TEST_MOUNT, false).map_err(|_| "mounting /mnt again failed")?;
        result?;
        f.unlinkat(AT_FDCWD, path.as_bytes(), 0).map_err(e)
    }
}

kernel_test! {
    /// A file's page is one frame for `read` and for mappings: a private
    /// mapping maps the cache's frame read-only and copies it on a write
    /// (the file stays as it was); a shared mapping's write reaches the
    /// file.
    fn read_and_mappings_share_the_page_cache_page(env: &TestEnv) -> TestResult {
        let Some(m) = test_mount(env)? else { return Ok(()) };
        let f = Files::new();
        let path = scratch(TEST_MOUNT, "mapped");
        let data = pattern(3, 3 * PAGE_SIZE);
        write_file(&f, &path, &data).map_err(e)?;
        let fd = f.open(path.as_bytes(), OpenFlags::RDWR, 0).map_err(e)?;
        let result = (|| {
            let file = f.file(fd).map_err(e)?;
            let ino = file.node().ino();
            let mut buf = [0u8; 16];
            f.pread(fd, &mut buf, PAGE_SIZE as u64).map_err(e)?;
            let cached = m.cached.cached_page(ino, 1).ok_or("read did not cache the page")?;
            let source = Arc::new(FileSource::of(&file));
            let page = source.cached_page(PAGE_SIZE as u64).ok_or("no cache page for a file on ext2")?;
            let page = page.map_err(|_| "the cache page could not be read")?;
            if !page.same(&cached) || page.key() != Key::new(m.cached.id(), source.node().ino(), 1) {
                return Err("the mapping's page is not the page read() used");
            }
            let memory = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
            let len = data.len() as u64;
            let fixed = Placement::FixedNoReplace;
            let src: Arc<dyn PageSource> = source;
            memory
                .map_file(fixed(0x100_0000), len, Prot::RW, FileRef::new(Arc::clone(&src)), 0)
                .map_err(|_| "a private mapping of the file failed")?;
            memory
                .map_shared_file(fixed(0x200_0000), len, Prot::RW, FileRef::new(src), 0)
                .map_err(|_| "a shared mapping of the file failed")?;
            let mut seen = [0u8; 16];
            memory.read(0x100_0000 + PAGE_SIZE as u64, &mut seen).map_err(|_| "reading the mapping faulted")?;
            let phys = memory.space().translate(0x100_0000 + PAGE_SIZE as u64).ok_or("not mapped")?.phys;
            if phys != cached.frame().phys() || seen != buf {
                return Err("the private mapping does not map the cache's frame");
            }
            // Private: a write copies the page; the file does not change.
            memory.write(0x100_0000 + PAGE_SIZE as u64, b"private").map_err(|_| "writing the mapping faulted")?;
            let copied = memory.space().translate(0x100_0000 + PAGE_SIZE as u64).ok_or("not mapped")?;
            if copied.phys == cached.frame().phys() || !copied.owned {
                return Err("a write to a private mapping did not copy the page");
            }
            // Shared: the write lands in the cache's page, so read() sees
            // it at once and the file gets it.
            memory.write(0x200_0000 + 2 * PAGE_SIZE as u64, b"shared!").map_err(|_| "writing the shared mapping")?;
            let mut back = [0u8; 7];
            f.pread(fd, &mut back, 2 * PAGE_SIZE as u64).map_err(e)?;
            // The private mapping's page was copied; the shared one is the cache's.
            if &back != b"shared!" || memory.cached_pages() != 1 {
                return Err("a write through a shared mapping is not in the file");
            }
            f.pread(fd, &mut back, PAGE_SIZE as u64).map_err(e)?;
            if back != data[PAGE_SIZE..PAGE_SIZE + 7] {
                return Err("a private mapping's write reached the file");
            }
            drop(memory);
            disk::sync_all().map_err(e)?;
            let mut on_disk = [0u8; 7];
            m.ext2.read(ino, 2 * PAGE_SIZE as u64, &mut on_disk).map_err(e)?;
            if &on_disk != b"shared!" {
                return Err("the shared mapping's write was not written back");
            }
            Ok(())
        })();
        f.close(fd).map_err(e)?;
        f.unlinkat(AT_FDCWD, path.as_bytes(), 0).map_err(e)?;
        result
    }
}

kernel_test! {
    /// A page written by `write` is dirty in the cache until the writeback
    /// thread writes it to the file system.
    fn the_writeback_thread_writes_dirty_pages(env: &TestEnv) -> TestResult {
        let Some(m) = test_mount(env)? else { return Ok(()) };
        let f = Files::new();
        let path = scratch(TEST_MOUNT, "writeback");
        write_file(&f, &path, b"written back by the thread").map_err(e)?;
        let ino = f.stat(path.as_bytes()).map_err(e)?.ino;
        let page = m.cached.cached_page(ino, 0).ok_or("the written page is not cached")?;
        if !page.is_dirty() {
            return Err("a written page is not dirty");
        }
        // Two passes started after the write: one has seen the page.
        let start = cache::passes();
        let deadline = time::now_ns() + 30 * time::NANOS_PER_SEC;
        while cache::passes() < start + 2 {
            if time::now_ns() > deadline {
                return Err("the writeback thread made no pass in 30 s");
            }
            cache::kick();
            crate::thread::sleep(core::time::Duration::from_millis(10));
        }
        if page.is_dirty() {
            return Err("the writeback thread left the page dirty");
        }
        let mut back = [0u8; 26];
        if m.ext2.read(ino, 0, &mut back).map_err(e)? != 26 || &back != b"written back by the thread" {
            return Err("the file system does not have the page's data");
        }
        drop(page);
        f.unlinkat(AT_FDCWD, path.as_bytes(), 0).map_err(e)
    }
}

kernel_test! {
    /// Evicting clean pages gives their frames back to the frame
    /// allocator. Counted with interrupts off, so nothing else allocates
    /// meanwhile (eviction itself takes no sleeping lock).
    fn evicting_clean_pages_frees_their_frames(env: &TestEnv) -> TestResult {
        let Some(_m) = test_mount(env)? else { return Ok(()) };
        let f = Files::new();
        disk::sync_all().map_err(e)?;
        if read_file(&f, "/mnt/dir/big.bin").map_err(e)?.len() != BIG_LEN {
            return Err("/mnt/dir/big.bin has the wrong size");
        }
        let c = cache::cache();
        let _only = crate::smp::exclusive();
        // Frames that are free or the heap's: eviction's own bookkeeping may
        // move frames between the two.
        let free = || FRAMES.lock().as_ref().map_or(0, |f| f.free_frames()) + crate::heap::stats().pages_held as u64;
        let (free_before, held_before) = (free(), cache::frames());
        let evicted = c.shrink(0) as u64;
        let (free_after, held_after) = (free(), cache::frames());
        if evicted < BIG_LEN.div_ceil(PAGE_SIZE) as u64 {
            return Err("fewer pages were evicted than the big file has");
        }
        if held_after != held_before - evicted || free_after < free_before + evicted {
            return Err("evicted pages did not give their frames back");
        }
        Ok(())
    }
}

kernel_test! {
    /// Mount errors: a disk that is not there, one mounted already, an
    /// unmount of something that is not a disk mount.
    fn disk_mount_errors() -> TestResult {
        if disk::mount("virtio9", "/tmp/nine", false).map(|_| ()) != Err(disk::MountError::NoDisk) {
            return Err("mounting a missing disk did not fail with NoDisk");
        }
        if disk::mount("ram0", "relative", false).map(|_| ()) != Err(disk::MountError::Vfs(Errno::EINVAL)) {
            return Err("a relative mount point was accepted");
        }
        if disk::unmount("/tmp") != Err(Errno::EINVAL) {
            return Err("unmounting a directory that is not a disk mount did not fail");
        }
        Ok(())
    }
}

// ----- ext4 (ADR 0052) -----

/// Kernel flag saying the ext4 test disk is attached and mounted at
/// [`TEST_EXT4_MOUNT`] (`cargo xtask` passes it with
/// `poly.mount=...,virtio3:/ext4`).
pub const TEST_EXT4_FLAG: &str = "poly.test.ext4";
pub const TEST_EXT4_MOUNT: &str = "/ext4";
pub const TEST_EXT4_DISK: &str = "virtio3";
/// Kernel flag for the boot that ends without unmounting (`cargo xtask`'s
/// journal crash test).
const TEST_EXT4_CRASH_FLAG: &str = "poly.test.ext4.crash";

/// `/hello.txt` on the ext4 test disk. Must match `tools/xtask/src/ext4disk.rs`,
/// like the lengths and contents below.
const HELLO4: &[u8] = b"Hello from ext4 on virtio-blk!\n";
const BIG4_LEN: usize = 300_000;
const FRAG_BLOCKS: usize = 200;

fn big4(i: usize) -> u8 {
    (i.wrapping_mul(7) ^ (i / 4096) ^ 0x5a) as u8
}

fn frag(n: usize, i: usize) -> u8 {
    (n.wrapping_mul(31) ^ i.wrapping_mul(3)) as u8
}

/// The disk mount at [`TEST_EXT4_MOUNT`], when the boot has the ext4 test disk.
fn ext4_mount(env: &TestEnv) -> core::result::Result<Option<Arc<disk::DiskMount>>, &'static str> {
    if !env.boot.has_flag(TEST_EXT4_FLAG) {
        return Ok(None);
    }
    let m = disk::mounts().into_iter().find(|m| m.path == TEST_EXT4_MOUNT);
    let m = m.ok_or("poly.test.ext4 is set but nothing is mounted at /ext4")?;
    if m.disk.name() != TEST_EXT4_DISK || m.ext2.fs_type() != "ext4" {
        return Err("/ext4 is not the ext4 test disk (virtio3)");
    }
    Ok(Some(m))
}

kernel_test! {
    /// The RAM disk formatted as ext4 (extents, checksums, no journal: it
    /// is too small for one), mounted through the page cache: files written
    /// there are on the disk for a fresh mount, which finds it clean.
    fn ext4_on_the_ram_disk_through_the_page_cache() -> TestResult {
        let ram = crate::block::ram().ok_or("no RAM disk")?;
        let opts = poly_ext2::FormatOptions { bytes_per_inode: 4096, ..poly_ext2::FormatOptions::ext4(1024, 0) };
        poly_ext2::format(&DiskIo(ram), &opts).map_err(e)?;
        let at = scratch("/tmp", "ram-ext4");
        disk::mount("ram0", &at, false).map_err(|_| "mounting ext4 on ram0 failed")?;
        let f = Files::new();
        let data = pattern(4, 5 * PAGE_SIZE + 10);
        let result = (|| {
            write_file(&f, &format!("{at}/file"), &data).map_err(e)?;
            if read_file(&f, &format!("{at}/file")).map_err(e)? != data {
                return Err("a file on ext4 read back wrong through the cache");
            }
            Ok(())
        })();
        let unmounted = disk::unmount(&at);
        result?;
        unmounted.map_err(|_| "unmounting ram0 failed")?;
        let fs = poly_ext2::Ext2::mount(DiskIo(ram), clock(), poly_ext2::MountOptions::default())
            .map_err(|_| "the unmounted ext4 volume does not mount again")?;
        if !fs.stats().was_clean || fs.fs_type() != "ext4" {
            return Err("unmounting did not leave a clean ext4 volume");
        }
        let file = fs.lookup(fs.root(), b"file").map_err(e)?;
        let mut back = vec![0u8; data.len()];
        if fs.read(file, 0, &mut back).map_err(e)? != data.len() || back != data {
            return Err("the file is not on the disk");
        }
        fs.unmount().map_err(e)
    }
}

kernel_test! {
    /// The ext4 test disk's files, as `cargo xtask` wrote them (one of them
    /// in an extent tree of depth 1), read through the VFS and the page
    /// cache.
    fn ext4_test_disk_is_mounted_with_its_files(env: &TestEnv) -> TestResult {
        let Some(m) = ext4_mount(env)? else { return Ok(()) };
        let f = Files::new();
        if read_file(&f, "/ext4/hello.txt").map_err(e)? != HELLO4 {
            return Err("/ext4/hello.txt has the wrong contents");
        }
        let big_bin = read_file(&f, "/ext4/dir/big.bin").map_err(e)?;
        if big_bin.len() != BIG4_LEN || big_bin.iter().enumerate().any(|(i, &b)| b != big4(i)) {
            return Err("/ext4/dir/big.bin has the wrong contents");
        }
        let fragmented = f.read_to_end(b"/ext4/dir/fragmented.bin", 2 << 20).map_err(e)?.0;
        if fragmented.len() != (2 * FRAG_BLOCKS - 1) * 4096 {
            return Err("/ext4/dir/fragmented.bin has the wrong size");
        }
        for (n, block) in fragmented.chunks(4096).enumerate() {
            let want = |i: usize| if n % 2 == 0 { frag(n / 2, i) } else { 0 };
            if block.iter().enumerate().any(|(i, &b)| b != want(i)) {
                return Err("/ext4/dir/fragmented.bin has the wrong contents");
            }
        }
        if read_file(&f, "/ext4/dir/link").map_err(e)? != HELLO4 {
            return Err("following /ext4/dir/link does not reach hello.txt");
        }
        let s = m.ext2.stats();
        if m.read_only || !s.journaled || s.errors {
            return Err("the ext4 test disk is not mounted read-write with its journal");
        }
        Ok(())
    }
}

kernel_test! {
    /// Writing to the ext4 test disk commits journal transactions; the data
    /// is there after unmounting and mounting it again.
    fn ext4_test_disk_journals_and_survives_a_remount(env: &TestEnv) -> TestResult {
        let Some(m) = ext4_mount(env)? else { return Ok(()) };
        let f = Files::new();
        let path = scratch(TEST_EXT4_MOUNT, "journaled");
        let data = pattern(5, 90_000);
        let commits = m.ext2.stats().commits;
        write_file(&f, &path, &data).map_err(e)?;
        disk::sync_all().map_err(e)?;
        if m.ext2.stats().commits <= commits {
            return Err("sync did not commit a journal transaction");
        }
        disk::unmount(TEST_EXT4_MOUNT).map_err(|_| "unmounting /ext4 failed (busy?)")?;
        m.disk.lock().invalidate();
        drop(m);
        let again = disk::mount(TEST_EXT4_DISK, TEST_EXT4_MOUNT, false).map_err(|_| "mounting /ext4 again failed")?;
        if !again.ext2.stats().was_clean || read_file(&f, &path).map_err(e)? != data {
            return Err("the data did not survive unmounting the ext4 disk");
        }
        f.unlinkat(AT_FDCWD, path.as_bytes(), 0).map_err(e)
    }
}

kernel_test! {
    /// With `poly.test.ext4.crash` only: writes a file and syncs (a journal
    /// commit), then another that is not synced; the boot then ends without
    /// unmounting, and `cargo xtask` checks that the next mount replays.
    fn ext4_journal_crash_writes_and_commits(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(TEST_EXT4_CRASH_FLAG) {
            return Ok(());
        }
        let Some(_m) = ext4_mount(env)? else { return Err("the crash test needs the ext4 test disk") };
        let f = Files::new();
        write_file(&f, "/ext4/committed.txt", b"committed before the crash\n").map_err(e)?;
        disk::sync_all().map_err(e)?;
        write_file(&f, "/ext4/uncommitted.txt", b"maybe").map_err(e)
    }
}
