//! In-kernel tests of fadvise64(2), copy_file_range(2), the file ioctls
//! (`FIONREAD` on a regular file, `FICLONE`) and the extended attribute
//! calls, made from a ring-3 process the way `files_tests` makes them
//! (its harness: the checks run in the process's one system call): on
//! tmpfs, and on ext4 (the RAM disk, formatted for the test and mounted
//! through the page cache), whose attributes must still be there for a
//! fresh mount.

use super::files_tests::{AT_FDCWD, BUF, Check, cstr, peek, read_file, run, setup, sys};
use crate::fs::disk::{self, DiskIo};
use crate::kernel_test;
use crate::process::Process;
use crate::user;
use alloc::vec::Vec;
use poly_personality_linux::{Errno, nr};
use poly_vfs::FileSystem;

/// Fails check `n`, logging what was expected.
macro_rules! check {
    ($n:expr, $cond:expr) => {
        if !$cond {
            crate::kprintln!("linux xattr test: check {} failed: {}", $n, stringify!($cond));
            return Err($n);
        }
    };
}

const O_WRONLY: u64 = 0o1;
const O_RDWR: u64 = 0o2;
const O_CREAT_TRUNC: u64 = 0o1100;
const FIONREAD: u64 = 0x541B;
const FICLONE: u64 = 0x4004_9409;
const POSIX_FADV_SEQUENTIAL: u64 = 2;
/// Scratch user memory for values and offsets (BUF is the harness's).
const VALUE: u64 = BUF + 0x4000;
const LIST: u64 = BUF + 0x5000;
const OFFSETS: u64 = BUF + 0x6000;
/// Where the test formats and mounts the RAM disk as ext4.
const EXT4_DIR: &str = "/tmp/xattr-ext4";

fn poke(at: u64, bytes: &[u8]) {
    let _ = user::copy_to_user(at, bytes);
}

fn create(p: &Process, path: &str, data: &[u8]) -> Result<u64, Errno> {
    let fd = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(6, path), O_RDWR | O_CREAT_TRUNC, 0o644])?;
    poke(VALUE, data);
    sys(p, nr::WRITE, &[fd, VALUE, data.len() as u64])?;
    sys(p, nr::LSEEK, &[fd, 0, 0])?;
    Ok(fd)
}

fn pipe(p: &Process) -> Result<(u64, u64), Errno> {
    sys(p, nr::PIPE2, &[BUF, 0])?;
    let b = peek(BUF, 8);
    let fd = |i: usize| u64::from(u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]));
    Ok((fd(0), fd(4)))
}

kernel_test! {
    fn fadvise64_and_the_file_ioctls() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            let fd = create(p, "/tmp/xa-advise", b"hello world").map_err(|_| 1u64)?;
            check!(2, sys(p, nr::FADVISE64, &[fd, 0, 0, POSIX_FADV_SEQUENTIAL]) == Ok(0));
            check!(3, sys(p, nr::FADVISE64, &[fd, 4096, 1 << 20, 4]) == Ok(0));
            check!(4, sys(p, nr::FADVISE64, &[fd, 0, 0, 6]) == Err(Errno::EINVAL));
            check!(5, sys(p, nr::FADVISE64, &[fd, 0, -1i64 as u64, 0]) == Err(Errno::EINVAL));
            check!(6, sys(p, nr::FADVISE64, &[999, 0, 0, 0]) == Err(Errno::EBADF));
            let (r, w) = pipe(p).map_err(|_| 7u64)?;
            check!(8, sys(p, nr::FADVISE64, &[r, 0, 0, POSIX_FADV_SEQUENTIAL]) == Err(Errno::ESPIPE));
            // FIONREAD: what is left from the file offset.
            check!(9, sys(p, nr::IOCTL, &[fd, FIONREAD, BUF]) == Ok(0) && peek(BUF, 4) == 11i32.to_le_bytes());
            check!(10, sys(p, nr::LSEEK, &[fd, 4, 0]) == Ok(4));
            check!(11, sys(p, nr::IOCTL, &[fd, FIONREAD, BUF]) == Ok(0) && peek(BUF, 4) == 7i32.to_le_bytes());
            // And on a pipe as before.
            check!(12, sys(p, nr::WRITE, &[w, cstr(1, "abc"), 3]) == Ok(3));
            check!(13, sys(p, nr::IOCTL, &[r, FIONREAD, BUF]) == Ok(0) && peek(BUF, 4) == 3i32.to_le_bytes());
            // FICLONE: tmpfs shares no extents.
            let dst = create(p, "/tmp/xa-clone", b"").map_err(|_| 14u64)?;
            check!(15, sys(p, nr::IOCTL, &[dst, FICLONE, fd]) == Err(Errno::EOPNOTSUPP));
            check!(16, sys(p, nr::IOCTL, &[w, FICLONE, fd]) == Err(Errno::EINVAL));
            check!(17, sys(p, nr::IOCTL, &[dst, FICLONE, 999]) == Err(Errno::EBADF));
            Ok(())
        })
    }
}

kernel_test! {
    fn copy_file_range_between_files_to_a_pipe_and_to_stdout() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| -> Check {
            let src = create(p, "/tmp/xa-src", b"0123456789").map_err(|_| 1u64)?;
            let dst = create(p, "/tmp/xa-dst", b"").map_err(|_| 2u64)?;
            let huge = 0x7fff_ffff_c000_0000u64;
            check!(3, sys(p, nr::COPY_FILE_RANGE, &[src, 0, dst, 0, huge, 0]) == Ok(10));
            check!(4, sys(p, nr::COPY_FILE_RANGE, &[src, 0, dst, 0, huge, 0]) == Ok(0));
            check!(5, read_file(p, "/tmp/xa-dst") == Ok(b"0123456789".to_vec()));
            // Offsets in memory: the file offsets stay.
            poke(OFFSETS, &2u64.to_le_bytes());
            poke(OFFSETS + 8, &10u64.to_le_bytes());
            check!(6, sys(p, nr::COPY_FILE_RANGE, &[src, OFFSETS, dst, OFFSETS + 8, 3, 0]) == Ok(3));
            check!(7, peek(OFFSETS, 8) == 5u64.to_le_bytes() && peek(OFFSETS + 8, 8) == 13u64.to_le_bytes());
            check!(8, read_file(p, "/tmp/xa-dst") == Ok(b"0123456789234".to_vec()));
            check!(9, sys(p, nr::LSEEK, &[src, 0, 1]) == Ok(10));
            // Not to a pipe, not into a file opened for appending.
            let (_, w) = pipe(p).map_err(|_| 10u64)?;
            check!(11, sys(p, nr::LSEEK, &[src, 0, 0]) == Ok(0));
            check!(12, sys(p, nr::COPY_FILE_RANGE, &[src, 0, w, 0, 10, 0]) == Err(Errno::EINVAL));
            let app = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(2, "/tmp/xa-dst"), O_WRONLY | 0o2000, 0]).map_err(|_| 13u64)?;
            check!(14, sys(p, nr::COPY_FILE_RANGE, &[src, 0, app, 0, 10, 0]) == Err(Errno::EBADF));
            check!(15, sys(p, nr::COPY_FILE_RANGE, &[src, 0, dst, 0, 10, 1]) == Err(Errno::EINVAL));
            // Another file system (the tmpfs on /dev/shm): no cross-file-system copies.
            let other = create(p, "/dev/shm/xa-other", b"").map_err(|_| 21u64)?;
            check!(22, sys(p, nr::COPY_FILE_RANGE, &[src, 0, other, 0, 10, 0]) == Err(Errno::EXDEV));
            check!(23, sys(p, nr::UNLINKAT, &[AT_FDCWD, cstr(2, "/dev/shm/xa-other"), 0]) == Ok(0));
            // stdout a regular file, as cat(1) has it in an app test.
            let out = create(p, "/tmp/xa-stdout", b"").map_err(|_| 16u64)?;
            check!(17, sys(p, nr::DUP3, &[out, 1, 0]) == Ok(1));
            check!(18, sys(p, nr::COPY_FILE_RANGE, &[src, 0, 1, 0, huge, 0]) == Ok(10));
            check!(19, sys(p, nr::COPY_FILE_RANGE, &[src, 0, 1, 0, huge, 0]) == Ok(0));
            check!(20, read_file(p, "/tmp/xa-stdout") == Ok(b"0123456789".to_vec()));
            Ok(())
        })
    }
}

/// Set, get, list and remove attributes of a file in directory `dir`
/// (checks numbered from `base`), leaving `user.kept` = "persists".
fn xattr_round_trip(p: &Process, dir: &str, base: u64) -> Check {
    let path = alloc::format!("{dir}/xa-file");
    let fd = create(p, &path, b"data").map_err(|_| base)?;
    let file = cstr(3, &path);
    let (colour, kept, selinux) = (cstr(4, "user.colour"), cstr(5, "user.kept"), cstr(7, "security.selinux"));
    // A new file has none: ENODATA and an empty list, as ls(1) expects.
    check!(base + 1, sys(p, nr::LGETXATTR, &[file, selinux, VALUE, 255]) == Err(Errno::ENODATA));
    check!(base + 2, sys(p, nr::LISTXATTR, &[file, LIST, 256]) == Ok(0));
    poke(VALUE, b"blue");
    check!(base + 3, sys(p, nr::SETXATTR, &[file, colour, VALUE, 4, 0]) == Ok(0));
    poke(VALUE, b"persists");
    check!(base + 4, sys(p, nr::FSETXATTR, &[fd, kept, VALUE, 8, 1]) == Ok(0));
    check!(base + 5, sys(p, nr::FSETXATTR, &[fd, kept, VALUE, 8, 1]) == Err(Errno::EEXIST));
    check!(base + 6, sys(p, nr::GETXATTR, &[file, colour, 0, 0]) == Ok(4));
    check!(base + 7, sys(p, nr::GETXATTR, &[file, colour, VALUE, 2]) == Err(Errno::ERANGE));
    check!(base + 8, sys(p, nr::FGETXATTR, &[fd, colour, VALUE, 64]) == Ok(4) && peek(VALUE, 4) == b"blue");
    let n = sys(p, nr::FLISTXATTR, &[fd, LIST, 256]).map_err(|_| base + 9)?;
    let mut names: Vec<&[u8]> = Vec::new();
    let list = peek(LIST, n as usize);
    names.extend(list.split(|&b| b == 0).filter(|s| !s.is_empty()));
    names.sort();
    check!(base + 10, names == [&b"user.colour"[..], &b"user.kept"[..]]);
    check!(base + 11, sys(p, nr::REMOVEXATTR, &[file, colour]) == Ok(0));
    check!(base + 12, sys(p, nr::FREMOVEXATTR, &[fd, colour]) == Err(Errno::ENODATA));
    check!(base + 13, sys(p, nr::LISTXATTR, &[file, LIST, 256]) == Ok(10) && peek(LIST, 10) == b"user.kept\0");
    // A minimal ACL is the permission bits.
    let acl = cstr(8, "system.posix_acl_access");
    poke(VALUE, &poly_vfs::xattr::acl::from_mode(0o640));
    check!(base + 14, sys(p, nr::FSETXATTR, &[fd, acl, VALUE, 28, 0]) == Ok(0));
    check!(base + 15, sys(p, nr::FGETXATTR, &[fd, acl, VALUE, 64]) == Err(Errno::ENODATA));
    check!(base + 16, sys(p, nr::NEWFSTATAT, &[AT_FDCWD, file, BUF, 0]) == Ok(0));
    check!(base + 17, peek(BUF + 24, 4) == 0o100640u32.to_le_bytes());
    check!(base + 18, sys(p, nr::CLOSE, &[fd]) == Ok(0));
    Ok(())
}

kernel_test! {
    fn extended_attributes_on_tmpfs_and_ext4() -> TestResult {
        let ram = crate::block::ram().ok_or("no RAM disk")?;
        let opts = poly_ext2::FormatOptions { bytes_per_inode: 4096, ..poly_ext2::FormatOptions::ext4(1024, 0) };
        poly_ext2::format(&DiskIo(ram), &opts).map_err(|_| "formatting ram0 failed")?;
        disk::mount("ram0", EXT4_DIR, false).map_err(|_| "mounting ext4 on ram0 failed")?;
        let (process, memory) = setup()?;
        let result = run(&process, &memory, |p| -> Check {
            xattr_round_trip(p, "/tmp", 100)?;
            xattr_round_trip(p, EXT4_DIR, 200)?;
            // tmpfs directory sizes: 40 plus 20 per entry.
            let made = sys(p, nr::MKDIRAT, &[AT_FDCWD, cstr(0, "/tmp/xa-dir"), 0o755]);
            check!(1, made.is_ok() || made == Err(Errno::EEXIST));
            check!(2, create(p, "/tmp/xa-dir/one", b"").is_ok());
            check!(3, sys(p, nr::NEWFSTATAT, &[AT_FDCWD, cstr(0, "/tmp/xa-dir"), BUF, 0]) == Ok(0));
            check!(4, peek(BUF + 48, 8) == 60u64.to_le_bytes() && peek(BUF + 64, 8) == 0u64.to_le_bytes());
            Ok(())
        });
        let unmounted = disk::unmount(EXT4_DIR);
        result?;
        unmounted.map_err(|_| "unmounting ram0 failed")?;
        // On the disk for a fresh mount.
        let fs = poly_ext2::Ext2::mount(DiskIo(ram), crate::fs::clock(), poly_ext2::MountOptions::default())
            .map_err(|_| "the ext4 volume does not mount again")?;
        let ino = fs.lookup(fs.root(), b"xa-file").map_err(|_| "xa-file is gone")?;
        if fs.getxattr(ino, b"user.kept").as_deref() != Ok(&b"persists"[..]) {
            return Err("user.kept is not on the disk");
        }
        if fs.getattr(ino).map(|a| a.perm) != Ok(0o640) {
            return Err("the ACL's permission bits are not on the disk");
        }
        fs.unmount().map_err(|_| "unmounting the fresh mount failed")
    }
}
