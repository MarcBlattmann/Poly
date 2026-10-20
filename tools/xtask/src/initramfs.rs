//! The initramfs images `cargo xtask` boots the kernel with (QEMU's
//! `-initrd`): `newc` cpio archives written with `poly-cpio`, so the host
//! needs no `cpio`. Every byte is fixed (times, owners, inode numbers,
//! order), so the same sources always give the same image.
//!
//! - The default image (`cargo xtask run`): `/etc/motd`, `/etc/hostname`,
//!   and `/dev/console` as a Linux initramfs has it.
//! - The test image (the self-test boots, with `poly.test.initrd`): the same
//!   plus `/test`, a tree of every kind of entry the kernel's file system
//!   tests check (`kernel/src/fs/tests.rs`), and `/bin/hello`, the nolibc
//!   hello world the exec tests run (`programs.rs`).
//! - The programs image (boots with `poly.init=/bin/<name>`): the default
//!   image plus the hello worlds in `/bin`.

use poly_cpio::{Format, Writer};
use poly_vfs::{DeviceId, FileType};
use std::path::{Path, PathBuf};

/// Kernel flag saying the boot has the test image (`fs::TEST_INITRD_FLAG`).
pub const TEST_INITRD_FLAG: &str = "poly.test.initrd";

/// Modification time of every entry (2023-11-14 22:13:20 UTC). Must match
/// `INITRAMFS_MTIME` in `kernel/src/fs/tests.rs`.
pub const MTIME: u32 = 1_700_000_000;

/// `/etc/motd`.
pub const MOTD: &str = "Welcome to Poly.\n\
\n\
This is /etc/motd from the initramfs `cargo xtask` builds (tools/xtask/src/initramfs.rs),\n\
unpacked into the root tmpfs at boot. Try `ls /`, `ls /dev`, `cat /etc/hostname` and `mount`.\n";

/// Length of `/test/big.bin`: more than nine 4 KiB pages, not a multiple.
pub const BIG_BIN_LEN: usize = 40_000;

/// Byte `i` of `/test/big.bin`. Must match `big_bin` in `kernel/src/fs/tests.rs`.
pub fn big_bin(i: usize) -> u8 {
    (i * 7 + i / 251) as u8
}

fn writer() -> Writer {
    let mut w = Writer::new(Format::Newc);
    w.mtime = MTIME;
    w
}

fn base(w: &mut Writer) {
    w.dir(b".", 0o755);
    w.dir(b"etc", 0o755);
    w.file(b"etc/motd", 0o644, MOTD.as_bytes());
    w.file(b"etc/hostname", 0o644, b"poly\n");
    w.dir(b"dev", 0o755);
    w.node(b"dev/console", FileType::CharDevice, 0o600, DeviceId::new(5, 1));
}

/// The image `cargo xtask run` boots with.
pub fn default_image() -> Vec<u8> {
    let mut w = writer();
    base(&mut w);
    w.finish()
}

/// The test image without programs.
#[cfg(test)]
pub fn test_image() -> Vec<u8> {
    test_image_with(&[])
}

/// Programs (`name`, ELF bytes) for an image's `/bin`, mode 0755.
pub type Programs<'a> = [(&'a str, &'a [u8])];

fn bin(w: &mut Writer, programs: &Programs<'_>) {
    if programs.is_empty() {
        return;
    }
    w.dir(b"bin", 0o755);
    for (name, elf) in programs {
        w.file(format!("bin/{name}").as_bytes(), 0o755, elf);
    }
}

/// The default image with `programs` in `/bin`: what the boots that run a
/// program as the first process (`poly.init=/bin/<name>`) use.
pub fn programs_image(programs: &Programs<'_>) -> Vec<u8> {
    let mut w = writer();
    base(&mut w);
    bin(&mut w, programs);
    w.finish()
}

/// More of a file tree than `/bin`: files (path without the leading `/`,
/// mode, contents) and symbolic links (path, target). Parent directories
/// are made as needed (mode 0755).
#[derive(Debug, Default)]
pub struct Tree {
    pub files: Vec<(String, u16, Vec<u8>)>,
    pub symlinks: Vec<(String, String)>,
}

/// [`programs_image`] plus `tree`: the image of the dynamic linking boot
/// tests (`/bin` holds the programs, `tree` the dynamic linker and the
/// shared libraries, see `dynamic.rs`).
pub fn programs_image_with(programs: &Programs<'_>, tree: &Tree) -> Vec<u8> {
    let mut w = writer();
    base(&mut w);
    bin(&mut w, programs);
    let mut made: std::collections::BTreeSet<String> = ["etc", "dev", "bin"].map(String::from).into();
    let paths = tree.files.iter().map(|(p, _, _)| p).chain(tree.symlinks.iter().map(|(p, _)| p));
    let mut dirs = std::collections::BTreeSet::new();
    for path in paths {
        let mut parent = Path::new(path).parent();
        while let Some(dir) = parent.filter(|d| !d.as_os_str().is_empty()) {
            dirs.insert(dir.to_string_lossy().into_owned());
            parent = dir.parent();
        }
    }
    // BTreeSet order puts every directory after its parent.
    for dir in dirs {
        if made.insert(dir.clone()) {
            w.dir(dir.as_bytes(), 0o755);
        }
    }
    for (path, mode, data) in &tree.files {
        w.file(path.as_bytes(), *mode, data);
    }
    for (path, target) in &tree.symlinks {
        w.symlink(path.as_bytes(), target.as_bytes());
    }
    w.finish()
}

/// The test image with `programs` in `/bin` (the kernel's exec tests run
/// `/bin/hello` from it).
pub fn test_image_with(programs: &Programs<'_>) -> Vec<u8> {
    let mut w = writer();
    base(&mut w);
    bin(&mut w, programs);
    w.dir(b"test", 0o750);
    w.file(b"test/hello.txt", 0o644, b"Hello from the initramfs!\n");
    w.file(b"test/empty", 0o600, b"");
    let big: Vec<u8> = (0..BIG_BIN_LEN).map(big_bin).collect();
    w.file(b"test/big.bin", 0o644, &big);
    w.file(b"test/script.sh", 0o755, b"#!/bin/sh\necho hi\n");
    w.dir(b"test/tree", 0o755);
    w.dir(b"test/tree/a", 0o755);
    w.dir(b"test/tree/a/b", 0o755);
    w.dir(b"test/tree/a/b/c", 0o700);
    w.file(b"test/tree/a/b/c/leaf.txt", 0o444, b"leaf\n");
    w.symlink(b"test/link", b"hello.txt");
    w.symlink(b"test/abs-link", b"/etc/motd");
    w.symlink(b"test/dangling", b"nowhere");
    w.node(b"test/zero-node", FileType::CharDevice, 0o666, DeviceId::new(1, 5));
    w.hard_link(&[b"test/hard1", b"test/hard2"], 0o640, b"linked\n");
    w.finish()
}

/// Write `image` to `target/<name>` and return its path. Rewritten on every
/// boot, like the test disk.
pub fn write(root: &Path, name: &str, image: &[u8]) -> Result<PathBuf, String> {
    let path = root.join("target").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(&path, image).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_vfs::{At, Clock, Cred, OpenFlags, Stat, Timespec, Tmpfs, Vfs};
    use std::sync::Arc;

    struct Zero;
    impl Clock for Zero {
        fn now(&self) -> Timespec {
            Timespec::default()
        }
    }

    #[test]
    fn images_are_reproducible() {
        assert_eq!(test_image(), test_image());
        assert_eq!(default_image(), default_image());
        // The test image is the default one (less its trailer) and more.
        let default = default_image();
        assert!(test_image().starts_with(&default[..default.len() - 128]));
    }

    #[test]
    fn programs_go_to_bin_executable() {
        let image = programs_image(&[("hello", b"\x7fELF hello"), ("hello-libc", b"\x7fELF libc")]);
        assert_eq!(image, programs_image(&[("hello", b"\x7fELF hello"), ("hello-libc", b"\x7fELF libc")]));
        let fs = Arc::new(Tmpfs::new(Arc::new(Zero)));
        poly_cpio::unpack_fs(&image, fs.clone()).unwrap();
        let vfs = Vfs::new(fs);
        let ctx = vfs.root_context().unwrap();
        let stat = vfs.stat(&ctx, &Cred::root(), At::Cwd, b"/bin/hello-libc", false).unwrap();
        assert_eq!((stat.perm(), stat.size), (0o755, 9));
        assert_eq!(vfs.stat(&ctx, &Cred::root(), At::Cwd, b"/bin", false).unwrap().perm(), 0o755);
        // No programs, no /bin: the test image is unchanged by this.
        assert_eq!(test_image_with(&[]), test_image());
    }

    #[test]
    fn trees_get_their_directories_files_and_links() {
        let tree = Tree {
            files: vec![
                ("lib64/ld.so".into(), 0o755, b"\x7fELF ld".to_vec()),
                ("usr/lib/x/libc.so.6".into(), 0o644, b"\x7fELF libc".to_vec()),
            ],
            symlinks: vec![("lib/libc.so.6".into(), "/usr/lib/x/libc.so.6".into())],
        };
        let image = programs_image_with(&[("prog", b"\x7fELF prog")], &tree);
        assert_eq!(image, programs_image_with(&[("prog", b"\x7fELF prog")], &tree));
        let fs = Arc::new(Tmpfs::new(Arc::new(Zero)));
        poly_cpio::unpack_fs(&image, fs.clone()).unwrap();
        let vfs = Vfs::new(fs);
        let ctx = vfs.root_context().unwrap();
        let stat = |p: &str, follow| vfs.stat(&ctx, &Cred::root(), At::Cwd, p.as_bytes(), follow).unwrap();
        assert_eq!(stat("/usr/lib/x", false).perm(), 0o755);
        assert_eq!((stat("/lib64/ld.so", false).perm(), stat("/lib64/ld.so", false).size), (0o755, 7));
        assert_eq!(stat("/lib/libc.so.6", true).size, 9, "the link leads to the library");
        assert_eq!(stat("/bin/prog", false).perm(), 0o755);
        assert_eq!(stat("/etc/motd", false).size, MOTD.len() as u64);
    }

    #[test]
    fn the_test_image_unpacks_as_the_kernel_tests_expect() {
        let fs = Arc::new(Tmpfs::new(Arc::new(Zero)));
        let stats = poly_cpio::unpack_fs(&test_image(), fs.clone()).unwrap();
        assert_eq!((stats.symlinks, stats.nodes, stats.hard_links), (3, 2, 1));
        let vfs = Vfs::new(fs);
        let ctx = vfs.root_context().unwrap();
        let root = Cred::root();
        let stat = |p: &str| -> Stat { vfs.stat(&ctx, &root, At::Cwd, p.as_bytes(), false).unwrap() };
        let read = |p: &str| {
            let f = vfs.open(&ctx, &root, At::Cwd, p.as_bytes(), OpenFlags::RDONLY, 0).unwrap();
            let mut buf = vec![0; 100_000];
            let n = f.read(&mut buf).unwrap();
            buf.truncate(n);
            buf
        };
        assert_eq!(read("/test/hello.txt"), b"Hello from the initramfs!\n");
        assert_eq!(read("/test/abs-link"), MOTD.as_bytes());
        assert_eq!(read("/test/big.bin"), (0..BIG_BIN_LEN).map(big_bin).collect::<Vec<_>>());
        assert_eq!((stat("/test").perm(), stat("/test/tree/a/b/c").perm()), (0o750, 0o700));
        assert_eq!((stat("/test/hard1").ino, stat("/test/hard1").nlink), (stat("/test/hard2").ino, 2));
        assert_eq!(stat("/dev/console").rdev, DeviceId::new(5, 1));
        assert_eq!(stat("/test/zero-node").rdev, DeviceId::new(1, 5));
        assert_eq!(stat("/test/hello.txt").mtime.sec, i64::from(MTIME));
        assert_eq!(stat("/test/script.sh").size, 18);
    }
}
