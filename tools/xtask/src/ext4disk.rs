//! The ext4 test disk: a 16 MiB ext4 volume (4 KiB blocks, `metadata_csum`,
//! extents, a 4 MiB journal) made with `poly-ext2`'s `format` and filled
//! through the same driver the kernel runs, on the host, so every byte is
//! the same on every run and no e2fsprogs are needed. Attached as the
//! fourth virtio-blk device (`virtio3`) to every boot with test devices,
//! the kernel mounts it at `/ext4` (`poly.mount=...,virtio3:/ext4`) and its
//! tests (`fs::disk_tests`, flag `poly.test.ext4`) check the files.
//!
//! | Path | What |
//! |---|---|
//! | `/hello.txt` | [`HELLO`], mode 0644 |
//! | `/dir/big.bin` | [`BIG_LEN`] bytes of [`big`] (one extent) |
//! | `/dir/fragmented.bin` | [`FRAG_BLOCKS`] blocks of [`frag`] with a hole after each: an extent tree of depth 1 |
//! | `/dir/link` | a symbolic link to `../hello.txt` |
//! | `/lost+found` | made by `format` |
//!
//! Boot tests: a file written through the shell, `sync`, `reboot`,
//! `e2fsck -fn`, read in a second boot; and a "crash": a boot whose tests
//! write and sync a file, then end without unmounting, so the next mount
//! must replay the journal.

use crate::{Devices, SHELL_FLAG, Symbols, boot, boot_typing, console, ktest, test_disk};
use poly_block::RamDisk;
use poly_ext2::{DeviceIo, Ext2, FormatOptions, MountOptions};
use poly_vfs::{Clock, DeviceId, FileSystem, FileType, NewNode, Timespec};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The image's file name, next to the test disk image.
pub const FILE_NAME: &str = "poly-test-ext4.img";
/// Size of the image.
pub const BYTES: usize = 16 << 20;
/// Journal blocks.
const JOURNAL: u32 = 1024;

/// `/hello.txt`. Must match `HELLO4` in `kernel/src/fs/disk_tests.rs`.
pub const HELLO: &[u8] = b"Hello from ext4 on virtio-blk!\n";
/// Length of `/dir/big.bin`. Must match `BIG4_LEN` in the kernel's tests.
pub const BIG_LEN: usize = 300_000;
/// Data blocks of `/dir/fragmented.bin`. Must match the kernel's tests.
pub const FRAG_BLOCKS: usize = 200;

/// Byte `i` of `/dir/big.bin`. Must match `big4` in the kernel's tests.
pub fn big(i: usize) -> u8 {
    (i.wrapping_mul(7) ^ (i / 4096) ^ 0x5a) as u8
}

/// Byte `i` of data block `n` of `/dir/fragmented.bin` (at offset
/// `2 * n * 4096`). Must match `frag` in the kernel's tests.
pub fn frag(n: usize, i: usize) -> u8 {
    (n.wrapping_mul(31) ^ i.wrapping_mul(3)) as u8
}

/// Every timestamp in the image.
const TIME: Timespec = Timespec::new(1_700_000_000, 0);

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        TIME
    }
}

fn node(perm: u16) -> NewNode {
    NewNode { kind: FileType::Regular, perm, uid: 0, gid: 0, rdev: DeviceId::new(0, 0) }
}

/// The image's bytes.
pub fn image() -> Result<Vec<u8>, String> {
    let mut mem = vec![0u8; BYTES];
    let err = |what: &str, e: poly_vfs::Errno| format!("ext4 test disk: {what}: {}", e.name());
    {
        let disk = RamDisk::new(&mut mem, 512).ok_or("ext4 test disk: bad RAM disk")?;
        let io = DeviceIo::new(disk);
        let opts = FormatOptions {
            uuid: *b"poly-ext4-test-1",
            label: *b"poly-ext4\0\0\0\0\0\0\0",
            time: TIME,
            ..FormatOptions::ext4(4096, JOURNAL)
        };
        poly_ext2::format(&io, &opts).map_err(|e| err("format", e))?;
        let fs = Ext2::mount(&io, Arc::new(FixedClock), MountOptions::default())
            .map_err(|e| format!("ext4 test disk: mount: {e}"))?;
        let root = fs.root();
        let hello = fs.create(root, b"hello.txt", &node(0o644)).map_err(|e| err("hello.txt", e))?;
        fs.write(hello, 0, HELLO).map_err(|e| err("hello.txt", e))?;
        let dir = fs.mkdir(root, b"dir", 0o755, 0, 0).map_err(|e| err("dir", e))?;
        let big_bin = fs.create(dir, b"big.bin", &node(0o644)).map_err(|e| err("big.bin", e))?;
        let data: Vec<u8> = (0..BIG_LEN).map(big).collect();
        fs.write(big_bin, 0, &data).map_err(|e| err("big.bin", e))?;
        // Backwards, so no two blocks are adjacent: one extent each.
        let f = fs.create(dir, b"fragmented.bin", &node(0o644)).map_err(|e| err("fragmented.bin", e))?;
        for n in (0..FRAG_BLOCKS).rev() {
            let block: Vec<u8> = (0..4096).map(|i| frag(n, i)).collect();
            fs.write(f, (2 * n * 4096) as u64, &block).map_err(|e| err("fragmented.bin", e))?;
        }
        fs.symlink(dir, b"link", b"../hello.txt", 0, 0).map_err(|e| err("link", e))?;
        fs.unmount().map_err(|e| err("unmount", e))?;
    }
    Ok(mem)
}

/// Write the image next to `test_disk` (the test disk image) and return
/// its path. Rewritten before every boot, like the test disk.
pub fn write_next_to(test_disk: &Path) -> Result<PathBuf, String> {
    let path = test_disk.with_file_name(FILE_NAME);
    std::fs::write(&path, image()?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// How many journal transactions mounting `image` replays (on a copy).
fn replays(image: &Path) -> Result<u32, String> {
    let mut mem = std::fs::read(image).map_err(|e| format!("{}: {e}", image.display()))?;
    let io = DeviceIo::new(RamDisk::new(&mut mem, 512).ok_or("bad image size")?);
    let opts = MountOptions { read_only: true, ..MountOptions::default() };
    let fs = Ext2::mount(&io, Arc::new(FixedClock), opts).map_err(|e| format!("{}: {e}", image.display()))?;
    Ok(fs.stats().replayed)
}

/// The boot tests of the ext4 test disk (see the module documentation).
pub fn boot_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result<(), String> {
    eprintln!("==> boot test: a file written to the ext4 test disk in one boot is there in the next");
    persistence(kernel, syms)?;
    eprintln!("==> boot test: the ext4 journal is replayed after a boot that ended without unmounting");
    crash(kernel, syms)
}

fn persistence(kernel: &Path, syms: &Arc<Symbols>) -> Result<(), String> {
    const TEXT: &str = "persisted on ext4 across boots";
    let disk = test_disk()?;
    let image = disk.with_file_name(FILE_NAME);
    let devices = Devices::Test(&disk);
    let append = format!("poly.qemu_exit {SHELL_FLAG} poly.test=ext4_test_disk {}", devices.flags());
    let first = [
        (console::PROMPT, format!("write /ext4/persist.txt {TEXT}\r")),
        (console::PROMPT, "sync\r".to_string()),
        (console::PROMPT, "reboot\r".to_string()),
    ];
    let steps: Vec<_> = first.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let b = boot_typing(kernel, syms, &append, devices, &[], &steps)?;
    ktest::parse(b.output.split(console::PROMPT).next().unwrap_or_default()).check()?;
    let ex = console::exchanges(&b.output);
    let out = |i: usize| ex.get(i).map_or("", |e| e.output);
    if ex.len() < 3 || !out(0).is_empty() || !out(1).contains("every disk synced") {
        return Err(format!("writing to the ext4 test disk failed (status {:?}): {:?}", b.status, ex));
    }
    if !out(2).contains("fs: /ext4 (virtio3) unmounted clean") || b.status != Some(0) {
        return Err(format!("reboot did not unmount the ext4 test disk clean (status {:?})", b.status));
    }
    match crate::ext2disk::e2fsck(&image)? {
        Some(_) => eprintln!("==> e2fsck -fn passes on the ext4 image after the writing boot"),
        None => eprintln!("==> e2fsck not installed; skipped checking the image"),
    }
    let second = ["cat /ext4/persist.txt\r", "mount\r", "reboot\r"];
    let steps: Vec<_> = second.iter().map(|c| (console::PROMPT, *c)).collect();
    let b = boot_typing(kernel, syms, &append, devices, &[], &steps)?;
    ktest::parse(b.output.split(console::PROMPT).next().unwrap_or_default()).check()?;
    let ex = console::exchanges(&b.output);
    let out = |i: usize| ex.get(i).map_or("", |e| e.output);
    if out(0) != format!("{TEXT}\n") {
        return Err(format!("the file written in the first boot reads {:?} in the second", out(0)));
    }
    if !out(1).contains("virtio3 on /ext4 type ext4 (rw)") || b.status != Some(0) {
        return Err(format!("the second boot did not mount the ext4 test disk (status {:?})", b.status));
    }
    Ok(())
}

fn crash(kernel: &Path, syms: &Arc<Symbols>) -> Result<(), String> {
    let disk = test_disk()?;
    let image = disk.with_file_name(FILE_NAME);
    let devices = Devices::Test(&disk);
    // No shell: QEMU exits after the tests, with nothing unmounted.
    let append = format!("poly.qemu_exit poly.test=ext4_journal_crash poly.test.ext4.crash {}", devices.flags());
    let b = boot(kernel, syms, crate::DEFAULT_MEMORY, &append, devices)?;
    let report = b.tests(&b.output)?;
    if !report.passed.iter().any(|t| t.ends_with("ext4_journal_crash_writes_and_commits")) {
        return Err("the crash boot did not run its ext4 test".into());
    }
    let n = replays(&image)?;
    if n == 0 {
        return Err("the image of the crashed boot needs no journal replay".into());
    }
    eprintln!("==> the crashed image has {n} journal transaction(s) to replay");
    let append = format!("poly.qemu_exit {SHELL_FLAG} poly.test=ext4_test_disk {}", devices.flags());
    let steps = [(console::PROMPT, "cat /ext4/committed.txt\r"), (console::PROMPT, "reboot\r")];
    let b = boot_typing(kernel, syms, &append, devices, &[], &steps)?;
    let before_shell = b.output.split(console::PROMPT).next().unwrap_or_default();
    ktest::parse(before_shell).check()?;
    if !before_shell.contains("fs: virtio3: journal replayed") {
        return Err("mounting the crashed ext4 disk did not replay its journal".into());
    }
    let ex = console::exchanges(&b.output);
    if ex.first().map(|e| e.output) != Some("committed before the crash\n") || b.status != Some(0) {
        return Err(format!("the committed file is not there after the replay (status {:?}): {:?}", b.status, ex));
    }
    match crate::ext2disk::e2fsck(&image)? {
        Some(_) => eprintln!("==> e2fsck -fn passes on the ext4 image after the replay"),
        None => eprintln!("==> e2fsck not installed; skipped checking the image"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_is_the_same_every_time_and_mounts_clean() {
        let a = image().unwrap();
        assert_eq!(a, image().unwrap());
        let mut mem = a.clone();
        let io = DeviceIo::new(RamDisk::new(&mut mem, 512).unwrap());
        let fs = Ext2::mount(&io, Arc::new(FixedClock), MountOptions { read_only: true, ordered: true }).unwrap();
        assert!(fs.stats().was_clean);
        assert_eq!(fs.info().name, "ext4");
        let dir = fs.lookup(fs.root(), b"dir").unwrap();
        let big_bin = fs.lookup(dir, b"big.bin").unwrap();
        let mut buf = vec![0u8; BIG_LEN];
        assert_eq!(fs.read(big_bin, 0, &mut buf).unwrap(), BIG_LEN);
        assert!(buf.iter().enumerate().all(|(i, &b)| b == big(i)));
        let f = fs.lookup(dir, b"fragmented.bin").unwrap();
        let mut buf = vec![0u8; 8192];
        assert_eq!(fs.read(f, 2 * 17 * 4096, &mut buf).unwrap(), 8192);
        assert!((0..4096).all(|i| buf[i] == frag(17, i)) && buf[4096..].iter().all(|&b| b == 0));
        assert_eq!(fs.readlink(fs.lookup(dir, b"link").unwrap()).unwrap(), b"../hello.txt");
    }

    #[test]
    fn e2fsck_accepts_the_image() {
        let dir = std::env::temp_dir().join(format!("poly-xtask-ext4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, image().unwrap()).unwrap();
        let checked = crate::ext2disk::e2fsck(&path);
        let _ = std::fs::remove_dir_all(&dir);
        match checked {
            Ok(Some(_)) => {}
            Ok(None) => eprintln!("e2fsck not installed; skipped"),
            Err(e) => panic!("{e}"),
        }
    }
}
