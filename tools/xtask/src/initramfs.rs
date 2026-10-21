//! The initramfs images `cargo xtask` boots the kernel with (QEMU's
//! `-initrd`): `newc` cpio archives written with `poly-cpio`, so the host
//! needs no `cpio`. Every byte is fixed (times, owners, inode numbers,
//! order), so the same sources always give the same image.
//!
//! - The default image (`cargo xtask run`): `/etc/motd`, `/etc/hostname`,
//!   `/etc/hosts` and `/etc/resolv.conf` (name resolution is the C
//!   library's: QEMU's user-networking DNS server at 10.0.2.3, which
//!   `poly.net=qemu` sets up), a minimal user database (`/etc/passwd`,
//!   `/etc/group`: root and nobody, for the C libraries and BusyBox later;
//!   the kernel never reads them), and `/dev/console` as a Linux
//!   initramfs has it.
//! - The test image (the self-test boots, with `poly.test.initrd`): the same
//!   plus `/test`, a tree of every kind of entry the kernel's file system
//!   tests check (`kernel/src/fs/tests.rs`), and `/bin/hello`, the nolibc
//!   hello world the exec tests run (`programs.rs`).
//! - The programs image (boots with `poly.init=/bin/<name>`): the default
//!   image plus the hello worlds in `/bin`.
//! - The BusyBox image (`cargo xtask run --busybox`, the BusyBox boot test):
//!   the default image plus `/bin/busybox`, a link per applet, and what
//!   BusyBox's init and shell read: `/etc/inittab`, `/etc/init.d/rcS`,
//!   `/etc/profile`, `/etc/fstab`, `/etc/shells` (see `busybox.rs`).

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

/// `/etc/hosts` (hosts(5)).
pub const HOSTS: &str = "127.0.0.1\tlocalhost\n::1\tlocalhost ip6-localhost ip6-loopback\n127.0.1.1\tpoly\n";

/// `/etc/resolv.conf` (resolv.conf(5)): QEMU's user-networking DNS
/// server (QEMU documentation, "Network options").
pub const RESOLV_CONF: &str = "nameserver 10.0.2.3\n";

/// `/etc/passwd` (passwd(5): name, password, uid, gid, comment, home,
/// shell; `x`: the password lives elsewhere).
pub const PASSWD: &str = "root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/nonexistent:/bin/false\n";

/// `/etc/group` (group(5): name, password, gid, members).
pub const GROUP: &str = "root:x:0:\nnogroup:x:65534:\n";

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
    w.file(b"etc/hosts", 0o644, HOSTS.as_bytes());
    w.file(b"etc/resolv.conf", 0o644, RESOLV_CONF.as_bytes());
    w.file(b"etc/passwd", 0o644, PASSWD.as_bytes());
    w.file(b"etc/group", 0o644, GROUP.as_bytes());
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

/// `/etc/inittab` for BusyBox's init: `<id>:<runlevels>:<action>:<process>`
/// (no id: the console). `rcS` once at boot; then a login shell on the
/// console that `cttyhack` gives a controlling terminal (so it has job
/// control), started again whenever it exits; at reboot or power-off the
/// file systems are unmounted (or made read-only).
pub const INITTAB: &str = "\
# BusyBox init(8) table for Poly (tools/xtask/src/initramfs.rs)
::sysinit:/etc/init.d/rcS
::respawn:/bin/cttyhack /bin/sh -l
::ctrlaltdel:/sbin/reboot
::shutdown:/bin/umount -a -r
";

/// `/etc/init.d/rcS`, run by init once at boot. The kernel mounts procfs,
/// sysfs, devpts and the tmpfs on `/dev/shm` before the first program
/// (ADR 0043, 0046), so each is mounted only if it is not there yet; the
/// tmpfs on `/tmp` always is.
pub const RCS: &str = "\
#!/bin/sh
# Run once by init at boot (/etc/inittab).
mounted() { grep -q \" $1 \" /proc/mounts 2>/dev/null; }
mounted /proc || mount -t proc -o nosuid,nodev,noexec proc /proc
mounted /sys || mount -t sysfs -o nosuid,nodev,noexec sysfs /sys
mkdir -p /dev/pts /dev/shm
mounted /dev/pts || mount -t devpts -o nosuid,noexec devpts /dev/pts
mounted /dev/shm || mount -t tmpfs -o nosuid,nodev tmpfs /dev/shm
mount -t tmpfs -o nosuid,nodev,mode=1777 tmpfs /tmp
hostname -F /etc/hostname
echo \"rcS: $(uname -sr) up, file systems mounted\"
";

/// `/etc/profile`, read by the login shell.
pub const PROFILE: &str = "\
# Read by login shells (tools/xtask/src/initramfs.rs).
export PATH=/sbin:/usr/sbin:/bin:/usr/bin
export HOME=/root
export TERM=${TERM:-linux}
export PS1='\\u@\\h:\\w\\$ '
cd \"$HOME\"
";

/// `/etc/fstab` (fstab(5)), for `mount -a` and `mount <dir>`.
pub const FSTAB: &str = "\
# <file system> <dir> <type> <options> <dump> <pass>
proc /proc proc nosuid,nodev,noexec 0 0
sysfs /sys sysfs nosuid,nodev,noexec 0 0
devpts /dev/pts devpts nosuid,noexec 0 0
tmpfs /tmp tmpfs nosuid,nodev,mode=1777 0 0
";

/// The applets the BusyBox image links to `/bin/busybox`, by directory.
/// An applet a BusyBox binary was built without says "applet not found".
pub const BUSYBOX_APPLETS: &[(&str, &[&str])] = &[
    (
        "bin",
        &[
            "ash",
            "base64",
            "cat",
            "chgrp",
            "chmod",
            "chown",
            "cp",
            "cttyhack",
            "date",
            "dd",
            "df",
            "dmesg",
            "echo",
            "egrep",
            "false",
            "fgrep",
            "grep",
            "gunzip",
            "gzip",
            "hostname",
            "ionice",
            "kill",
            "link",
            "ln",
            "login",
            "ls",
            "mkdir",
            "mknod",
            "mktemp",
            "more",
            "mount",
            "mountpoint",
            "mv",
            "nice",
            "pidof",
            "printenv",
            "ps",
            "pwd",
            "rm",
            "rmdir",
            "sed",
            "setpriv",
            "sh",
            "sleep",
            "stat",
            "stty",
            "su",
            "sync",
            "tar",
            "touch",
            "true",
            "umount",
            "uname",
            "usleep",
            "vi",
            "watch",
            "zcat",
        ],
    ),
    (
        "sbin",
        &[
            "blkid",
            "chroot",
            "getty",
            "halt",
            "hwclock",
            "init",
            "losetup",
            "mdev",
            "mkdosfs",
            "mke2fs",
            "pivot_root",
            "poweroff",
            "reboot",
            "runlevel",
            "start-stop-daemon",
            "switch_root",
            "sysctl",
        ],
    ),
    (
        "usr/bin",
        &[
            "[",
            "[[",
            "awk",
            "basename",
            "bc",
            "cksum",
            "clear",
            "cmp",
            "comm",
            "cut",
            "dc",
            "diff",
            "dirname",
            "du",
            "env",
            "expand",
            "expr",
            "factor",
            "find",
            "flock",
            "fold",
            "free",
            "fuser",
            "head",
            "hexdump",
            "id",
            "install",
            "killall",
            "less",
            "logname",
            "md5sum",
            "mesg",
            "mkfifo",
            "nl",
            "nohup",
            "nproc",
            "od",
            "paste",
            "patch",
            "printf",
            "pstree",
            "readlink",
            "realpath",
            "renice",
            "reset",
            "resize",
            "script",
            "seq",
            "setsid",
            "sha1sum",
            "sha256sum",
            "sha512sum",
            "shuf",
            "sort",
            "split",
            "strings",
            "sum",
            "tac",
            "tail",
            "tee",
            "test",
            "time",
            "timeout",
            "top",
            "tr",
            "truncate",
            "tty",
            "uniq",
            "unlink",
            "uptime",
            "uudecode",
            "uuencode",
            "wc",
            "which",
            "whoami",
            "xargs",
            "xxd",
            "yes",
        ],
    ),
    ("usr/sbin", &["chpasswd", "crond", "nologin", "rdate"]),
];

/// The BusyBox image: the default image plus `busybox` as `/bin/busybox`
/// with a link to it for every applet ([`BUSYBOX_APPLETS`]; `/sbin/init`
/// among them) and the files BusyBox's init and shell read.
pub fn busybox_image(busybox: &[u8]) -> Vec<u8> {
    busybox_image_with(busybox, INITTAB, &[])
}

/// `/etc/inittab` of the root disk image ([`disk_root_image`]): after
/// `rcS`, `rootdisk` mounts the disk; the console's shell is the disk's
/// bash, chrooted.
pub const DISK_ROOT_INITTAB: &str = "\
# BusyBox init(8) table for Poly's root disk (tools/xtask/src/rootdisk.rs)
::sysinit:/etc/init.d/rcS
::sysinit:/etc/init.d/rootdisk
::respawn:/bin/cttyhack /sbin/chroot /mnt/root /bin/bash -l
::ctrlaltdel:/sbin/reboot
::shutdown:/bin/umount -a -r
";

/// `/etc/init.d/rootdisk`: mount the root disk (the first virtio-blk
/// disk) on `/mnt/root` and bind the kernel's file systems into it.
pub const ROOTDISK_SCRIPT: &str = "\
#!/bin/sh
# Run once by init at boot, after rcS (/etc/inittab).
mkdir -p /mnt/root
if ! mount -t ext4 /dev/vda /mnt/root; then
    echo \"rootdisk: cannot mount /dev/vda on /mnt/root\"
    exit 1
fi
for d in dev dev/pts dev/shm proc sys tmp; do
    mkdir -p /mnt/root/$d && mount -o bind /$d /mnt/root/$d
done
echo \"rootdisk: /dev/vda mounted on /mnt/root\"
";

/// The BusyBox image booting into the root disk's bash (`cargo xtask run
/// --disk-root`, the root disk boot test; `rootdisk.rs`).
pub fn disk_root_image(busybox: &[u8]) -> Vec<u8> {
    busybox_image_with(busybox, DISK_ROOT_INITTAB, &[("etc/init.d/rootdisk", ROOTDISK_SCRIPT)])
}

/// [`busybox_image`] with another `/etc/inittab` and more scripts in it.
fn busybox_image_with(busybox: &[u8], inittab: &str, scripts: &[(&str, &str)]) -> Vec<u8> {
    let mut w = writer();
    base(&mut w);
    w.file(b"etc/inittab", 0o644, inittab.as_bytes());
    w.dir(b"etc/init.d", 0o755);
    w.file(b"etc/init.d/rcS", 0o755, RCS.as_bytes());
    for (path, text) in scripts {
        w.file(path.as_bytes(), 0o755, text.as_bytes());
    }
    w.file(b"etc/profile", 0o644, PROFILE.as_bytes());
    w.file(b"etc/fstab", 0o644, FSTAB.as_bytes());
    w.file(b"etc/shells", 0o644, b"/bin/sh\n/bin/ash\n");
    for dir in ["bin", "sbin", "usr", "usr/bin", "usr/sbin", "mnt", "home", "var", "var/log", "run"] {
        w.dir(dir.as_bytes(), 0o755);
    }
    w.symlink(b"var/run", b"../run");
    w.dir(b"root", 0o700);
    w.dir(b"tmp", 0o1777);
    w.file(b"bin/busybox", 0o755, busybox);
    for (dir, applets) in BUSYBOX_APPLETS {
        for applet in *applets {
            w.symlink(format!("{dir}/{applet}").as_bytes(), b"/bin/busybox");
        }
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
    fn the_busybox_image_has_init_its_applets_and_configuration() {
        let image = busybox_image(b"\x7fELF busybox");
        assert_eq!(image, busybox_image(b"\x7fELF busybox"));
        let fs = Arc::new(Tmpfs::new(Arc::new(Zero)));
        poly_cpio::unpack_fs(&image, fs.clone()).unwrap();
        let vfs = Vfs::new(fs);
        let ctx = vfs.root_context().unwrap();
        let root = Cred::root();
        let link = |p: &str| vfs.readlink(&ctx, &root, At::Cwd, p.as_bytes()).unwrap();
        for p in ["/sbin/init", "/bin/sh", "/bin/cttyhack", "/bin/mount", "/usr/bin/seq", "/sbin/poweroff"] {
            assert_eq!(link(p), b"/bin/busybox", "{p}");
        }
        assert_eq!(link("/var/run"), b"../run");
        let stat = |p: &str| vfs.stat(&ctx, &root, At::Cwd, p.as_bytes(), false).unwrap();
        assert_eq!(stat("/bin/busybox").perm(), 0o755);
        assert_eq!(stat("/etc/init.d/rcS").perm(), 0o755);
        assert_eq!((stat("/tmp").perm(), stat("/root").perm()), (0o1777, 0o700));
        let read = |p: &str| {
            let f = vfs.open(&ctx, &root, At::Cwd, p.as_bytes(), OpenFlags::RDONLY, 0).unwrap();
            let mut buf = vec![0; 4096];
            let n = f.read(&mut buf).unwrap();
            buf.truncate(n);
            String::from_utf8(buf).unwrap()
        };
        assert!(read("/etc/inittab").contains("::sysinit:/etc/init.d/rcS\n"));
        assert!(read("/etc/inittab").contains("::respawn:/bin/cttyhack /bin/sh -l\n"));
        assert!(read("/etc/init.d/rcS").starts_with("#!/bin/sh\n"));
        assert!(read("/etc/profile").contains("PS1="));
        // Every inittab line is <id>:<runlevels>:<action>:<process>.
        assert!(INITTAB.lines().filter(|l| !l.starts_with('#')).all(|l| l.split(':').count() == 4));
        // No applet twice.
        let mut all: Vec<&str> = BUSYBOX_APPLETS.iter().flat_map(|(_, a)| a.iter().copied()).collect();
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n);
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
        assert_eq!(read("/etc/passwd"), PASSWD.as_bytes());
        assert_eq!(read("/etc/group"), GROUP.as_bytes());
        assert_eq!((stat("/etc/passwd").perm(), stat("/etc/passwd").uid), (0o644, 0));
        assert!(PASSWD.lines().all(|l| l.split(':').count() == 7) && GROUP.lines().all(|l| l.split(':').count() == 4));
    }
}
