//! In-kernel tests of the administration calls (ADR 0051): mount(2) and
//! umount2(2) of the memory file systems, remounts and bind mounts,
//! mknod(2), chroot(2), sethostname(2) and the harmless reboot(2)
//! commands, made from a ring-3 process the way `files_tests` makes them
//! (its harness: the checks run in the process's one system call).

use super::files_tests::{AT_FDCWD, BUF, Check, cstr, read_file, readlink, run, setup, sys, text};
use crate::kernel_test;
use crate::process::Process;
use poly_personality_linux::{Errno, nr};

/// mount(2) flags (`linux/mount.h`).
const MS_RDONLY: u64 = 1;
const MS_NOSUID: u64 = 2;
const MS_NODEV: u64 = 4;
const MS_NOEXEC: u64 = 8;
const MS_REMOUNT: u64 = 32;
const MS_BIND: u64 = 4096;
const O_CREAT_WRONLY: u64 = 0o101;

/// Fails check `n`, logging what was expected.
macro_rules! check {
    ($n:expr, $cond:expr) => {
        if !$cond {
            crate::kprintln!("linux admin test: check {} failed: {}", $n, stringify!($cond));
            return Err($n);
        }
    };
}

fn mkdir(p: &Process, path: &str) -> Result<u64, Errno> {
    match sys(p, nr::MKDIRAT, &[AT_FDCWD, cstr(10, path), 0o755]) {
        Err(Errno::EEXIST) => Ok(0),
        r => r,
    }
}

fn mount(p: &Process, source: &str, target: &str, fstype: &str, flags: u64, data: &str) -> Result<u64, Errno> {
    let data = if data.is_empty() { 0 } else { cstr(4, data) };
    let fstype = if fstype.is_empty() { 0 } else { cstr(3, fstype) };
    sys(p, nr::MOUNT, &[cstr(1, source), cstr(2, target), fstype, flags, data])
}

fn umount(p: &Process, target: &str) -> Result<u64, Errno> {
    sys(p, nr::UMOUNT2, &[cstr(2, target), 0])
}

/// `st_mode` of `path` (newfstatat; `struct stat` has it at offset 24).
fn mode(p: &Process, path: &str) -> Result<u32, Errno> {
    sys(p, nr::NEWFSTATAT, &[AT_FDCWD, cstr(5, path), BUF, 0])?;
    let b = super::files_tests::peek(BUF + 24, 4);
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn create(p: &Process, path: &str, data: &str) -> Result<u64, Errno> {
    let fd = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(6, path), O_CREAT_WRONLY, 0o644])?;
    sys(p, nr::WRITE, &[fd, cstr(7, data), data.len() as u64])?;
    Ok(fd)
}

kernel_test! {
    fn mount_and_umount_memory_file_systems() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            check!(1, mkdir(p, "/tmp/adm-m").is_ok());
            check!(2, mount(p, "scratch", "/tmp/adm-m", "tmpfs", MS_NOSUID | MS_NODEV, "mode=0710,size=1m") == Ok(0));
            check!(3, mode(p, "/tmp/adm-m") == Ok(0o40710));
            check!(4, text(p, "/proc/mounts").contains("scratch /tmp/adm-m tmpfs rw,nosuid,nodev,relatime,size=1024k,mode=710 0 0\n"));
            let fd = create(p, "/tmp/adm-m/f", "in tmpfs").map_err(|_| 5u64)?;
            // Busy while a file is open in it.
            check!(6, umount(p, "/tmp/adm-m") == Err(Errno::EBUSY));
            check!(7, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            check!(8, umount(p, "/tmp/adm-m") == Ok(0));
            check!(9, read_file(p, "/tmp/adm-m/f") == Err(Errno::ENOENT));
            check!(10, !text(p, "/proc/mounts").contains(" /tmp/adm-m "));
            // Not a mount point; an unknown type; options the type does not take.
            check!(11, umount(p, "/tmp/adm-m") == Err(Errno::EINVAL));
            check!(12, mount(p, "x", "/tmp/adm-m", "nosuchfs", 0, "") == Err(Errno::ENODEV));
            check!(13, mount(p, "x", "/tmp/adm-m", "tmpfs", 0, "bogus=1") == Err(Errno::EINVAL));
            check!(14, mount(p, "x", "/tmp/nowhere", "tmpfs", 0, "") == Err(Errno::ENOENT));
            // ext2 needs a block device.
            check!(15, mount(p, "/dev/null", "/tmp/adm-m", "ext2", 0, "") == Err(Errno::ENOTBLK));
            // procfs, sysfs and devpts, again.
            check!(16, mount(p, "proc", "/tmp/adm-m", "proc", MS_NOSUID | MS_NODEV | MS_NOEXEC, "") == Ok(0));
            check!(17, readlink(p, "/tmp/adm-m/self") == Ok(alloc::format!("{}", p.pid).into_bytes()));
            check!(18, umount(p, "/tmp/adm-m") == Ok(0));
            check!(19, mount(p, "sysfs", "/tmp/adm-m", "sysfs", 0, "") == Ok(0));
            check!(20, mode(p, "/tmp/adm-m/devices").is_ok_and(|m| m & 0o170000 == 0o040000));
            check!(21, umount(p, "/tmp/adm-m") == Ok(0));
            check!(22, mount(p, "devpts", "/tmp/adm-m", "devpts", 0, "gid=5,mode=620") == Ok(0));
            check!(23, umount(p, "/tmp/adm-m") == Ok(0));
            Ok(())
        })
    }
}

kernel_test! {
    fn remount_and_bind_mounts() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            check!(1, mkdir(p, "/tmp/adm-r").is_ok() && mkdir(p, "/tmp/adm-b").is_ok());
            check!(2, mount(p, "none", "/tmp/adm-r", "tmpfs", 0, "") == Ok(0));
            let fd = create(p, "/tmp/adm-r/f", "x").map_err(|_| 3u64)?;
            check!(4, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            check!(5, mount(p, "", "/tmp/adm-r", "", MS_REMOUNT | MS_RDONLY, "") == Ok(0));
            check!(6, create(p, "/tmp/adm-r/g", "y") == Err(Errno::EROFS));
            check!(7, text(p, "/proc/mounts").contains("none /tmp/adm-r tmpfs ro,relatime 0 0\n"));
            check!(8, mount(p, "", "/tmp/adm-r", "", MS_REMOUNT, "") == Ok(0));
            let fd = create(p, "/tmp/adm-r/g", "y").map_err(|_| 9u64)?;
            check!(10, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            // A bind mount shows the same files.
            check!(11, mount(p, "/tmp/adm-r", "/tmp/adm-b", "", MS_BIND, "") == Ok(0));
            check!(12, read_file(p, "/tmp/adm-b/f") == Ok(b"x".to_vec()));
            check!(13, umount(p, "/tmp/adm-b") == Ok(0));
            check!(14, umount(p, "/tmp/adm-r") == Ok(0));
            Ok(())
        })
    }
}

kernel_test! {
    fn mknod_and_chroot() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            // makedev(1, 3), /dev/null's number, in the call's encoding.
            let null = cstr(8, "/tmp/adm-null");
            let _ = sys(p, nr::UNLINKAT, &[AT_FDCWD, null, 0]);
            check!(1, sys(p, nr::MKNODAT, &[AT_FDCWD, null, 0o020666, 0x103]) == Ok(0));
            check!(2, mode(p, "/tmp/adm-null").is_ok_and(|m| m & 0o170000 == 0o020000));
            let fd = sys(p, nr::OPENAT, &[AT_FDCWD, null, 1, 0]).map_err(|_| 3u64)?;
            check!(4, sys(p, nr::WRITE, &[fd, cstr(7, "gone"), 4]) == Ok(4));
            check!(5, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            let fifo = cstr(9, "/tmp/adm-fifo");
            let _ = sys(p, nr::UNLINKAT, &[AT_FDCWD, fifo, 0]);
            check!(6, sys(p, nr::MKNOD, &[fifo, 0o010600, 0]) == Ok(0));
            check!(7, mode(p, "/tmp/adm-fifo") == Ok(0o010600));
            check!(8, sys(p, nr::MKNOD, &[cstr(9, "/tmp/adm-dir"), 0o040755, 0]) == Err(Errno::EPERM));
            // chroot: "/" is the new root from then on.
            check!(9, mkdir(p, "/tmp/adm-root").is_ok());
            let fd = create(p, "/tmp/adm-root/inside", "jailed").map_err(|_| 10u64)?;
            check!(11, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            check!(12, sys(p, nr::CHROOT, &[cstr(0, "/tmp/adm-root")]) == Ok(0));
            check!(13, read_file(p, "/inside") == Ok(b"jailed".to_vec()));
            check!(14, read_file(p, "/tmp/adm-root/inside") == Err(Errno::ENOENT));
            Ok(())
        })
    }
}

kernel_test! {
    fn host_name_and_harmless_reboot_commands() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            let before = super::admin::hostname();
            let name = cstr(0, "kt-host");
            check!(1, sys(p, nr::SETHOSTNAME, &[name, 7]) == Ok(0));
            check!(2, sys(p, nr::UNAME, &[BUF]) == Ok(0));
            let node = super::files_tests::peek(BUF + 65, 8);
            check!(3, node == b"kt-host\0");
            check!(4, text(p, "/proc/sys/kernel/hostname") == "kt-host\n");
            check!(5, sys(p, nr::SETHOSTNAME, &[name, 65]) == Err(Errno::EINVAL));
            let old = alloc::string::String::from_utf8_lossy(&before).into_owned();
            check!(6, sys(p, nr::SETHOSTNAME, &[cstr(0, &old), before.len() as u64]) == Ok(0));
            // reboot(2): wrong magic numbers, and the Ctrl-Alt-Del setting
            // (the commands that would take the machine down are left out).
            let magic1 = 0xfee1_dead_u64;
            check!(7, sys(p, nr::REBOOT, &[0x1234, 672_274_793, 0, 0]) == Err(Errno::EINVAL));
            check!(8, sys(p, nr::REBOOT, &[magic1, 1, 0, 0]) == Err(Errno::EINVAL));
            check!(9, sys(p, nr::REBOOT, &[magic1, 672_274_793, 0x89ab_cdef, 0]) == Ok(0));
            check!(10, sys(p, nr::REBOOT, &[magic1, 672_274_793, 0x4558_4543, 0]) == Err(Errno::EINVAL));
            Ok(())
        })
    }
}

/// More of `linux/mount.h` and `linux/sched.h` for the tests below.
const MS_REC: u64 = 16384;
const MS_MOVE: u64 = 8192;
const MS_PRIVATE: u64 = 1 << 18;
const MS_NOATIME: u64 = 1024;
const MNT_DETACH: u64 = 2;
const CLONE_NEWNS: u64 = 0x0002_0000;
const CLONE_NEWUSER: u64 = 0x1000_0000;

fn umount2(p: &Process, target: &str, flags: u64) -> Result<u64, Errno> {
    sys(p, nr::UMOUNT2, &[cstr(2, target), flags])
}

kernel_test! {
    /// ADR 0177: `MS_BIND | MS_REC` copies the mounts below the source as
    /// mounts of their own; a plain bind does not.
    fn a_recursive_bind_copies_the_mounts_below() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            check!(1, sys(p, nr::UNSHARE, &[CLONE_NEWNS]) == Ok(0));
            check!(2, mount(p, "", "/", "", MS_REC | MS_PRIVATE, "") == Ok(0));
            for d in ["/tmp/adm-s", "/tmp/adm-c", "/tmp/adm-p"] {
                check!(3, mkdir(p, d).is_ok());
            }
            check!(4, mount(p, "src", "/tmp/adm-s", "tmpfs", 0, "") == Ok(0));
            check!(5, mkdir(p, "/tmp/adm-s/a").is_ok());
            check!(6, mount(p, "sub", "/tmp/adm-s/a", "tmpfs", MS_NODEV, "") == Ok(0));
            let fd = create(p, "/tmp/adm-s/a/fa", "in-a").map_err(|_| 7u64)?;
            check!(8, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            // Recursive: the submount is there, listed, with its flags and source.
            check!(9, mount(p, "/tmp/adm-s", "/tmp/adm-c", "", MS_BIND | MS_REC, "") == Ok(0));
            check!(10, read_file(p, "/tmp/adm-c/a/fa") == Ok(b"in-a".to_vec()));
            let mounts = text(p, "/proc/self/mounts");
            check!(11, mounts.contains("src /tmp/adm-c tmpfs rw,relatime 0 0\n"));
            check!(12, mounts.contains("sub /tmp/adm-c/a tmpfs rw,nodev,relatime 0 0\n"));
            // Plain: the directory the submount covers.
            check!(13, mount(p, "/tmp/adm-s", "/tmp/adm-p", "", MS_BIND, "") == Ok(0));
            check!(14, read_file(p, "/tmp/adm-p/a/fa") == Err(Errno::ENOENT));
            check!(15, !text(p, "/proc/self/mounts").contains(" /tmp/adm-p/a "));
            check!(16, umount(p, "/tmp/adm-p") == Ok(0));
            // The copies unmount from the bottom up, or all at once.
            check!(17, umount(p, "/tmp/adm-c") == Err(Errno::EBUSY));
            check!(18, umount(p, "/tmp/adm-c/a") == Ok(0));
            check!(19, umount(p, "/tmp/adm-c") == Ok(0));
            check!(20, mount(p, "/tmp/adm-s", "/tmp/adm-c", "", MS_BIND | MS_REC, "") == Ok(0));
            check!(21, umount2(p, "/tmp/adm-c", MNT_DETACH) == Ok(0));
            check!(22, read_file(p, "/tmp/adm-c/a/fa") == Err(Errno::ENOENT));
            check!(23, read_file(p, "/tmp/adm-s/a/fa") == Ok(b"in-a".to_vec()));
            Ok(())
        })
    }
}

kernel_test! {
    /// ADR 0177: the mounts a user namespace inherits are locked, the file
    /// systems made outside it are not its to change, and
    /// pivot_root(".", ".") hides the old root beneath the new one.
    fn inherited_mounts_are_locked_in_a_user_namespace() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            check!(1, sys(p, nr::UNSHARE, &[CLONE_NEWNS]) == Ok(0));
            check!(2, mount(p, "", "/", "", MS_REC | MS_PRIVATE, "") == Ok(0));
            for d in ["/tmp/adm-x", "/tmp/adm-y", "/tmp/adm-c1", "/tmp/adm-c2"] {
                check!(3, mkdir(p, d).is_ok());
            }
            check!(4, mount(p, "x", "/tmp/adm-x", "tmpfs", MS_NOEXEC | MS_NOATIME, "") == Ok(0));
            check!(5, mount(p, "y", "/tmp/adm-y", "tmpfs", 0, "") == Ok(0));
            check!(6, mkdir(p, "/tmp/adm-y/in").is_ok());
            check!(7, mount(p, "in", "/tmp/adm-y/in", "tmpfs", MS_NODEV, "") == Ok(0));
            check!(8, sys(p, nr::UNSHARE, &[CLONE_NEWUSER | CLONE_NEWNS]) == Ok(0));
            // In place.
            check!(9, umount(p, "/tmp/adm-x") == Err(Errno::EINVAL));
            check!(10, umount2(p, "/tmp/adm-y/in", MNT_DETACH) == Err(Errno::EINVAL));
            check!(11, umount2(p, "/tmp/adm-y", MNT_DETACH) == Err(Errno::EINVAL));
            check!(12, mount(p, "/tmp/adm-x", "/tmp/adm-c1", "", MS_MOVE, "") == Err(Errno::EINVAL));
            // Flags and the access time mode.
            let bind_remount = MS_REMOUNT | MS_BIND;
            check!(13, mount(p, "", "/tmp/adm-x", "", bind_remount | MS_NOEXEC, "") == Ok(0));
            check!(14, mount(p, "", "/tmp/adm-x", "", bind_remount | MS_NOATIME, "") == Err(Errno::EPERM));
            check!(15, mount(p, "", "/tmp/adm-x", "", bind_remount | MS_NOEXEC | MS_NOSUID, "") == Ok(0));
            check!(16, mount(p, "", "/tmp/adm-x", "", bind_remount | MS_NOEXEC, "") == Ok(0));
            check!(17, mount(p, "", "/tmp/adm-y/in", "", bind_remount, "") == Err(Errno::EPERM));
            check!(18, mount(p, "", "/tmp/adm-y", "", bind_remount | MS_NOATIME, "") == Err(Errno::EPERM));
            check!(19, mount(p, "", "/tmp/adm-y", "", bind_remount, "") == Ok(0));
            // The file system itself is the outer namespace's.
            check!(20, mount(p, "", "/tmp/adm-y", "", MS_REMOUNT | MS_RDONLY, "") == Err(Errno::EPERM));
            // Copies.
            check!(21, mount(p, "/tmp/adm-y", "/tmp/adm-c1", "", MS_BIND, "") == Err(Errno::EINVAL));
            check!(22, mount(p, "/tmp/adm-y", "/tmp/adm-c1", "", MS_BIND | MS_REC, "") == Ok(0));
            check!(23, umount(p, "/tmp/adm-c1/in") == Err(Errno::EINVAL));
            check!(24, mount(p, "", "/tmp/adm-c1/in", "", bind_remount, "") == Err(Errno::EPERM));
            check!(25, umount(p, "/tmp/adm-c1") == Err(Errno::EBUSY));
            check!(26, umount2(p, "/tmp/adm-c1", MNT_DETACH) == Ok(0));
            check!(27, mount(p, "/tmp/adm-x", "/tmp/adm-c2", "", MS_BIND, "") == Ok(0));
            check!(28, mount(p, "", "/tmp/adm-c2", "", bind_remount, "") == Err(Errno::EPERM));
            check!(29, umount(p, "/tmp/adm-c2") == Ok(0));
            // A mount of its own: on top of a locked one, changed and removed freely.
            check!(30, mount(p, "own", "/tmp/adm-x", "tmpfs", 0, "") == Ok(0));
            check!(31, mount(p, "", "/tmp/adm-x", "", MS_REMOUNT | MS_RDONLY, "") == Ok(0));
            check!(32, mount(p, "", "/tmp/adm-x", "", MS_REMOUNT, "") == Ok(0));
            check!(33, umount(p, "/tmp/adm-x") == Ok(0));
            check!(34, umount(p, "/tmp/adm-x") == Err(Errno::EINVAL));
            // The sandbox's last step: a new root, the old one beneath it,
            // then gone with everything it had.
            check!(35, mount(p, "root", "/tmp/adm-c1", "tmpfs", 0, "") == Ok(0));
            let fd = create(p, "/tmp/adm-c1/g", "y").map_err(|_| 36u64)?;
            check!(37, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            check!(38, sys(p, nr::CHDIR, &[cstr(2, "/tmp/adm-c1")]) == Ok(0));
            check!(39, sys(p, nr::PIVOT_ROOT, &[cstr(1, "."), cstr(2, ".")]) == Ok(0));
            check!(40, read_file(p, "/g") == Ok(b"y".to_vec()));
            check!(41, read_file(p, "g") == Ok(b"y".to_vec()));
            check!(42, mode(p, "/tmp") == Err(Errno::ENOENT));
            check!(43, sys(p, nr::PIVOT_ROOT, &[cstr(1, "."), cstr(2, ".")]) == Err(Errno::EBUSY));
            check!(44, umount2(p, ".", MNT_DETACH) == Ok(0));
            check!(45, read_file(p, "/g") == Ok(b"y".to_vec()));
            check!(46, mode(p, "/tmp") == Err(Errno::ENOENT));
            Ok(())
        })
    }
}

kernel_test! {
    /// ADR 0177, decision 9: a shared writable mapping of a file of an
    /// overlay copies the file up and maps the upper copy, so what is
    /// written through it is in the upper layer and the merged view at
    /// once, and the lower layer keeps its own.
    fn a_shared_mapping_of_an_overlay_file_writes_the_upper_layer() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            check!(1, sys(p, nr::UNSHARE, &[CLONE_NEWNS]) == Ok(0));
            check!(2, mkdir(p, "/tmp/adm-ov").is_ok());
            check!(3, mount(p, "ov", "/tmp/adm-ov", "tmpfs", 0, "") == Ok(0));
            for d in ["/tmp/adm-ov/l", "/tmp/adm-ov/u", "/tmp/adm-ov/w", "/tmp/adm-ov/m"] {
                check!(4, mkdir(p, d).is_ok());
            }
            let fd = create(p, "/tmp/adm-ov/l/mm", "0123456789").map_err(|_| 5u64)?;
            check!(6, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            let layers = "lowerdir=/tmp/adm-ov/l,upperdir=/tmp/adm-ov/u,workdir=/tmp/adm-ov/w";
            check!(7, mount(p, "overlay", "/tmp/adm-ov/m", "overlay", 0, layers) == Ok(0));
            // O_RDWR: the file is copied up at open.
            let fd = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(6, "/tmp/adm-ov/m/mm"), 2, 0]).map_err(|_| 8u64)?;
            check!(9, read_file(p, "/tmp/adm-ov/u/mm") == Ok(b"0123456789".to_vec()));
            // PROT_READ | PROT_WRITE, MAP_SHARED.
            let at = sys(p, nr::MMAP, &[0, 4096, 3, 1, fd, 0]).map_err(|_| 10u64)?;
            check!(11, super::files_tests::peek(at, 4) == b"0123");
            check!(12, crate::user::copy_to_user(at + 2, b"XY").is_ok());
            // The upper layer and the merged view see it before any sync.
            check!(13, read_file(p, "/tmp/adm-ov/u/mm") == Ok(b"01XY456789".to_vec()));
            check!(14, read_file(p, "/tmp/adm-ov/m/mm") == Ok(b"01XY456789".to_vec()));
            // MS_SYNC.
            check!(15, sys(p, nr::MSYNC, &[at, 4096, 4]) == Ok(0));
            check!(16, sys(p, nr::MUNMAP, &[at, 4096]) == Ok(0));
            check!(17, sys(p, nr::CLOSE, &[fd]) == Ok(0));
            check!(18, read_file(p, "/tmp/adm-ov/u/mm") == Ok(b"01XY456789".to_vec()));
            check!(19, read_file(p, "/tmp/adm-ov/m/mm") == Ok(b"01XY456789".to_vec()));
            check!(20, read_file(p, "/tmp/adm-ov/l/mm") == Ok(b"0123456789".to_vec()));
            check!(21, umount(p, "/tmp/adm-ov/m") == Ok(0));
            check!(22, umount(p, "/tmp/adm-ov") == Ok(0));
            Ok(())
        })
    }
}
