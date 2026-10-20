//! The ext2 test disk: an 8 MiB ext2 volume made with `poly-ext2`'s
//! `format` and filled through the same driver the kernel runs, on the
//! host, so every byte is the same on every run and no e2fsprogs are
//! needed. The kernel mounts it at `/mnt` (`poly.mount=virtio2:/mnt`) and
//! its tests (`fs::disk_tests`, flag `poly.test.ext2`) check the files.
//!
//! | Path | What |
//! |---|---|
//! | `/hello.txt` | [`HELLO`], mode 0644 |
//! | `/empty` | an empty file, mode 0600 |
//! | `/dir/big.bin` | [`BIG_LEN`] bytes of [`big`], mode 0644 (past the direct blocks) |
//! | `/dir/link` | a symbolic link to `../hello.txt` |
//! | `/lost+found` | made by `format` |

use poly_block::RamDisk;
use poly_ext2::{DeviceIo, Ext2, FormatOptions, MountOptions};
use poly_vfs::{Clock, DeviceId, FileSystem, FileType, NewNode, Timespec};
use std::path::Path;
use std::sync::Arc;

/// The image's file name, next to the test disk image.
pub const FILE_NAME: &str = "poly-test-ext2.img";
/// Size of the image.
pub const BYTES: usize = 8 << 20;
/// Kernel flags for a boot that has the image as its third virtio-blk
/// device: mount it on `/mnt` and check it (`fs::disk_tests`).
pub const KERNEL_FLAGS: &str = "poly.mount=virtio2:/mnt poly.test.ext2";

/// `/hello.txt`. Must match `HELLO` in `kernel/src/fs/disk_tests.rs`.
pub const HELLO: &[u8] = b"Hello from ext2 on virtio-blk!\n";
/// Length of `/dir/big.bin`. Must match `BIG_LEN` in the kernel's tests.
pub const BIG_LEN: usize = 300_000;

/// Byte `i` of `/dir/big.bin`. Must match `big` in the kernel's tests.
pub fn big(i: usize) -> u8 {
    (i.wrapping_mul(13) ^ (i / 4096)) as u8
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
    let err = |what: &str, e: poly_vfs::Errno| format!("ext2 test disk: {what}: {}", e.name());
    {
        let disk = RamDisk::new(&mut mem, 512).ok_or("ext2 test disk: bad RAM disk")?;
        let io = DeviceIo::new(disk);
        let opts = FormatOptions {
            block_size: 4096,
            uuid: *b"poly-ext2-test-1",
            label: *b"poly-test\0\0\0\0\0\0\0",
            time: TIME,
            ..FormatOptions::default()
        };
        poly_ext2::format(&io, &opts).map_err(|e| err("format", e))?;
        let fs = Ext2::mount(&io, Arc::new(FixedClock), MountOptions::default())
            .map_err(|e| format!("ext2 test disk: mount: {e}"))?;
        let root = fs.root();
        let hello = fs.create(root, b"hello.txt", &node(0o644)).map_err(|e| err("hello.txt", e))?;
        fs.write(hello, 0, HELLO).map_err(|e| err("hello.txt", e))?;
        fs.create(root, b"empty", &node(0o600)).map_err(|e| err("empty", e))?;
        let dir = fs.mkdir(root, b"dir", 0o755, 0, 0).map_err(|e| err("dir", e))?;
        let big_bin = fs.create(dir, b"big.bin", &node(0o644)).map_err(|e| err("big.bin", e))?;
        let data: Vec<u8> = (0..BIG_LEN).map(big).collect();
        fs.write(big_bin, 0, &data).map_err(|e| err("big.bin", e))?;
        fs.symlink(dir, b"link", b"../hello.txt", 0, 0).map_err(|e| err("link", e))?;
        fs.unmount().map_err(|e| err("unmount", e))?;
    }
    Ok(mem)
}

/// Write the image next to `test_disk` (the test disk image) and return
/// its path. Rewritten before every boot, like the test disk.
pub fn write_next_to(test_disk: &Path) -> Result<std::path::PathBuf, String> {
    let path = test_disk.with_file_name(FILE_NAME);
    std::fs::write(&path, image()?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// `e2fsck -fn` on `image`: `Ok(Some(report))` if it passed, `Ok(None)` if
/// e2fsprogs are not installed, `Err` with its output if it found problems.
pub fn e2fsck(image: &Path) -> Result<Option<String>, String> {
    let out = match std::process::Command::new("e2fsck").arg("-fn").arg(image).output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("e2fsck: {e}")),
    };
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(Some(text)) } else { Err(format!("e2fsck -fn {}:\n{text}", image.display())) }
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
        let dir = fs.lookup(fs.root(), b"dir").unwrap();
        let big_bin = fs.lookup(dir, b"big.bin").unwrap();
        let mut buf = vec![0u8; BIG_LEN];
        assert_eq!(fs.read(big_bin, 0, &mut buf).unwrap(), BIG_LEN);
        assert!(buf.iter().enumerate().all(|(i, &b)| b == big(i)));
        assert_eq!(fs.readlink(fs.lookup(dir, b"link").unwrap()).unwrap(), b"../hello.txt");
    }

    #[test]
    fn e2fsck_accepts_the_image() {
        let dir = std::env::temp_dir().join(format!("poly-xtask-ext2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, image().unwrap()).unwrap();
        let checked = e2fsck(&path);
        let _ = std::fs::remove_dir_all(&dir);
        match checked {
            Ok(Some(_)) => {}
            Ok(None) => eprintln!("e2fsck not installed; skipped"),
            Err(e) => panic!("{e}"),
        }
    }
}
