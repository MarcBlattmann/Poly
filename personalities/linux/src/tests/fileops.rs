//! fadvise64(2), copy_file_range(2), the file ioctls (`FIONREAD` on a
//! regular file, `FICLONE`) and the extended attribute calls, against a
//! [`LinuxFs`](crate::fs::LinuxFs) over a real `poly-vfs` tmpfs (the
//! fixture of `files.rs`).

extern crate std;

use super::fake::*;
use super::files::{AT, env, s};
use crate::{Errno, nr};
use std::vec::Vec;

const O_RDONLY: u64 = 0;
const O_WRONLY: u64 = 1;
const O_RDWR: u64 = 2;
const O_CREAT: u64 = 0o100;
const O_APPEND: u64 = 0o2000;
const O_DIRECTORY: u64 = 0o200000;
const O_PATH: u64 = 0o10000000;

fn open(k: &mut FakeKernel, path: &[u8], flags: u64) -> u64 {
    let p = s(k, 0x100, path);
    sys(k, nr::OPENAT, &[AT, p, flags, 0o644]).unwrap()
}

fn pwrite(k: &mut FakeKernel, fd: u64, data: &[u8], at: u64) {
    k.poke(scratch(0x2000), data);
    assert_eq!(sys(k, nr::PWRITE64, &[fd, scratch(0x2000), data.len() as u64, at]), Ok(data.len() as u64));
}

fn contents(k: &mut FakeKernel, path: &[u8]) -> Vec<u8> {
    let fd = open(k, path, O_RDONLY);
    let n = sys(k, nr::PREAD64, &[fd, scratch(0x4000), 0x1000, 0]).unwrap();
    k.peek(scratch(0x4000), n as usize)
}

// ---- fadvise64 ------------------------------------------------------------

#[test]
fn fadvise64_takes_every_advice() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDONLY);
    for advice in 0..=5 {
        assert_eq!(sys(k, nr::FADVISE64, &[fd, 0, 0, advice]), Ok(0), "advice {advice}");
    }
    // Past the end, a directory: fine.
    assert_eq!(sys(k, nr::FADVISE64, &[fd, 1 << 40, 4096, 3]), Ok(0));
    let dir = open(k, b"/dir", O_DIRECTORY);
    assert_eq!(sys(k, nr::FADVISE64, &[dir, 0, 0, 2]), Ok(0));
    // Unknown advice, negative length.
    assert_eq!(sys(k, nr::FADVISE64, &[fd, 0, 0, 6]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FADVISE64, &[fd, 0, -1i64 as u64, 0]), Err(Errno::EINVAL));
    // Not open, negative, O_PATH.
    assert_eq!(sys(k, nr::FADVISE64, &[77, 0, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FADVISE64, &[-1i64 as u64, 0, 0, 0]), Err(Errno::EBADF));
    let path_fd = open(k, b"/dir/file", O_PATH);
    assert_eq!(sys(k, nr::FADVISE64, &[path_fd, 0, 0, 0]), Err(Errno::EBADF));
}

// ---- copy_file_range -----------------------------------------------------------

#[test]
fn copy_file_range_with_file_offsets() {
    let mut e = env();
    let k = &mut e.k;
    let src = open(k, b"/dir/file", O_RDONLY);
    let dst = open(k, b"/dir/copy", O_WRONLY | O_CREAT);
    // cat's call: everything there is, then 0 at the end.
    let huge = 9_223_372_035_781_033_984u64;
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, 0, dst, 0, huge, 0]), Ok(8));
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, 0, dst, 0, huge, 0]), Ok(0));
    assert_eq!(contents(k, b"/dir/copy"), b"contents");
    // Both file offsets moved.
    assert_eq!(sys(k, nr::LSEEK, &[src, 0, 1]), Ok(8));
    assert_eq!(sys(k, nr::LSEEK, &[dst, 0, 1]), Ok(8));
    // Length 0 copies nothing.
    assert_eq!(sys(k, nr::LSEEK, &[src, 0, 0]), Ok(0));
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, 0, dst, 0, 0, 0]), Ok(0));
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, 0, dst, 0, 3, 0]), Ok(3));
    assert_eq!(contents(k, b"/dir/copy"), b"contentscon");
}

#[test]
fn copy_file_range_with_offset_pointers() {
    let mut e = env();
    let k = &mut e.k;
    let src = open(k, b"/dir/file", O_RDONLY);
    let dst = open(k, b"/dir/copy", O_RDWR | O_CREAT);
    pwrite(k, dst, b"0123456789", 0);
    let (off_in, off_out) = (scratch(0x3000), scratch(0x3008));
    k.poke(off_in, &3u64.to_le_bytes());
    k.poke(off_out, &5u64.to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, off_in, dst, off_out, 4, 0]), Ok(4));
    assert_eq!(contents(k, b"/dir/copy"), b"01234tent9");
    assert_eq!(k.peek(off_in, 8), 7u64.to_le_bytes());
    assert_eq!(k.peek(off_out, 8), 9u64.to_le_bytes());
    // The file offsets were not used or moved.
    assert_eq!(sys(k, nr::LSEEK, &[src, 0, 1]), Ok(0));
    assert_eq!(sys(k, nr::LSEEK, &[dst, 0, 1]), Ok(0));
    // At the end of the source: 0.
    k.poke(off_in, &100u64.to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, off_in, dst, off_out, 4, 0]), Ok(0));
    // Negative offset, offset plus length past loff_t, a bad pointer.
    k.poke(off_in, &(-1i64).to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, off_in, dst, 0, 4, 0]), Err(Errno::EINVAL));
    k.poke(off_in, &1u64.to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, off_in, dst, 0, i64::MAX as u64, 0]), Err(Errno::EOVERFLOW));
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[src, UNMAPPED, dst, 0, 4, 0]), Err(Errno::EFAULT));
}

#[test]
fn copy_file_range_checks_its_descriptors() {
    let mut e = env();
    let k = &mut e.k;
    let src = open(k, b"/dir/file", O_RDONLY);
    let dst = open(k, b"/dir/copy", O_WRONLY | O_CREAT);
    let both = open(k, b"/dir/both", O_RDWR | O_CREAT);
    let append = open(k, b"/dir/app", O_WRONLY | O_CREAT | O_APPEND);
    let dir = open(k, b"/dir", O_DIRECTORY);
    let path_fd = open(k, b"/dir/file", O_PATH);
    let call = |k: &mut FakeKernel, i: u64, o: u64, flags: u64| sys(k, nr::COPY_FILE_RANGE, &[i, 0, o, 0, 10, flags]);
    assert_eq!(call(k, 99, dst, 0), Err(Errno::EBADF));
    assert_eq!(call(k, src, 99, 0), Err(Errno::EBADF));
    assert_eq!(call(k, -1i64 as u64, dst, 0), Err(Errno::EBADF));
    assert_eq!(call(k, dst, both, 0), Err(Errno::EBADF), "input not open for reading");
    assert_eq!(call(k, src, src, 0), Err(Errno::EBADF), "output not open for writing");
    assert_eq!(call(k, src, append, 0), Err(Errno::EBADF), "O_APPEND output");
    assert_eq!(call(k, path_fd, dst, 0), Err(Errno::EBADF));
    assert_eq!(call(k, src, dst, 1), Err(Errno::EINVAL));
    assert_eq!(call(k, dir, dst, 0), Err(Errno::EISDIR));
    // The same file: overlapping ranges are refused, others are fine.
    pwrite(k, both, b"abcdefgh", 0);
    let (off_in, off_out) = (scratch(0x3000), scratch(0x3008));
    k.poke(off_in, &0u64.to_le_bytes());
    k.poke(off_out, &2u64.to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[both, off_in, both, off_out, 4, 0]), Err(Errno::EINVAL));
    k.poke(off_out, &4u64.to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[both, off_in, both, off_out, 4, 0]), Ok(4));
    assert_eq!(contents(k, b"/dir/both"), b"abcdabcd");
}

#[test]
fn copy_file_range_without_file_system_services() {
    let mut k = FakeKernel::new();
    k.files.insert(5, File::new(Kind::Pipe));
    assert_eq!(sys(&mut k, nr::COPY_FILE_RANGE, &[5, 0, 5, 0, 1, 0]), Err(Errno::ENOSYS));
}

// ---- ioctl ----------------------------------------------------------------------

#[test]
fn fionread_on_a_regular_file_is_what_is_left() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDONLY);
    let out = scratch(0x3000);
    assert_eq!(sys(k, nr::IOCTL, &[fd, 0x541B, out]), Ok(0));
    assert_eq!(k.peek(out, 4), 8i32.to_le_bytes());
    assert_eq!(sys(k, nr::LSEEK, &[fd, 5, 0]), Ok(5));
    assert_eq!(sys(k, nr::IOCTL, &[fd, 0x541B, out]), Ok(0));
    assert_eq!(k.peek(out, 4), 3i32.to_le_bytes());
    assert_eq!(sys(k, nr::LSEEK, &[fd, 100, 0]), Ok(100));
    assert_eq!(sys(k, nr::IOCTL, &[fd, 0x541B, out]), Ok(0));
    assert_eq!(k.peek(out, 4), 0i32.to_le_bytes());
    assert_eq!(sys(k, nr::IOCTL, &[fd, 0x541B, UNMAPPED]), Err(Errno::EFAULT));
}

#[test]
fn ficlone_is_not_supported() {
    let mut e = env();
    let k = &mut e.k;
    let src = open(k, b"/dir/file", O_RDONLY);
    let dst = open(k, b"/dir/copy", O_WRONLY | O_CREAT);
    let dir = open(k, b"/dir", O_DIRECTORY);
    const FICLONE: u64 = 0x4004_9409;
    const FICLONERANGE: u64 = 0x4020_940d;
    const FIDEDUPERANGE: u64 = 0xc018_9436;
    // cp's probe: ioctl(dest, FICLONE, src).
    assert_eq!(sys(k, nr::IOCTL, &[dst, FICLONE, src]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::IOCTL, &[src, FICLONE, dst]), Err(Errno::EBADF), "wrong way round");
    assert_eq!(sys(k, nr::IOCTL, &[dst, FICLONE, 99]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::IOCTL, &[dst, FICLONE, dir]), Err(Errno::EISDIR));
    let range = scratch(0x3000);
    let mut raw = (src as i64).to_le_bytes().to_vec();
    raw.extend_from_slice(&[0; 24]);
    k.poke(range, &raw);
    assert_eq!(sys(k, nr::IOCTL, &[dst, FICLONERANGE, range]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::IOCTL, &[dst, FICLONERANGE, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::IOCTL, &[src, FIDEDUPERANGE, range]), Err(Errno::EOPNOTSUPP));
}

// ---- extended attributes -----------------------------------------------------

#[test]
fn xattrs_through_the_calls() {
    let mut e = env();
    let k = &mut e.k;
    let file = s(k, 0, b"/dir/file");
    let link = s(k, 0x40, b"/link");
    let name = s(k, 0x80, b"user.colour");
    let value = scratch(0x1000);
    let buf = scratch(0x3000);
    k.poke(value, b"blue");
    // None yet: a file system that keeps them answers ENODATA and 0.
    let selinux = s(k, 0xc0, b"security.selinux");
    assert_eq!(sys(k, nr::LGETXATTR, &[file, selinux, buf, 255]), Err(Errno::ENODATA));
    assert_eq!(sys(k, nr::LISTXATTR, &[file, buf, 1024]), Ok(0));
    assert_eq!(sys(k, nr::SETXATTR, &[file, name, value, 4, 0]), Ok(0));
    // The size first, then the value; too small a buffer is ERANGE.
    assert_eq!(sys(k, nr::GETXATTR, &[file, name, 0, 0]), Ok(4));
    assert_eq!(sys(k, nr::GETXATTR, &[file, name, buf, 3]), Err(Errno::ERANGE));
    assert_eq!(sys(k, nr::GETXATTR, &[link, name, buf, 64]), Ok(4));
    assert_eq!(k.peek(buf, 4), b"blue");
    // lgetxattr does not follow the link (and a link has no user. ones).
    assert_eq!(sys(k, nr::LGETXATTR, &[link, name, buf, 64]), Err(Errno::ENODATA));
    assert_eq!(sys(k, nr::LISTXATTR, &[link, 0, 0]), Ok(12));
    assert_eq!(sys(k, nr::LISTXATTR, &[link, buf, 11]), Err(Errno::ERANGE));
    assert_eq!(sys(k, nr::LISTXATTR, &[file, buf, 64]), Ok(12));
    assert_eq!(k.peek(buf, 12), b"user.colour\0");
    assert_eq!(sys(k, nr::LLISTXATTR, &[link, buf, 64]), Ok(0));
    // Through a descriptor.
    let fd = sys(k, nr::OPENAT, &[AT, file, 0, 0]).unwrap();
    assert_eq!(sys(k, nr::FGETXATTR, &[fd, name, buf, 64]), Ok(4));
    assert_eq!(sys(k, nr::FLISTXATTR, &[fd, buf, 64]), Ok(12));
    assert_eq!(sys(k, nr::FSETXATTR, &[fd, name, value, 2, 1]), Err(Errno::EEXIST));
    assert_eq!(sys(k, nr::FSETXATTR, &[fd, name, value, 2, 2]), Ok(0));
    assert_eq!(sys(k, nr::GETXATTR, &[file, name, buf, 64]), Ok(2));
    assert_eq!(sys(k, nr::FREMOVEXATTR, &[fd, name]), Ok(0));
    assert_eq!(sys(k, nr::REMOVEXATTR, &[file, name]), Err(Errno::ENODATA));
    assert_eq!(sys(k, nr::LREMOVEXATTR, &[file, name]), Err(Errno::ENODATA));
    // An empty value; sed -i's minimal ACL on a new file.
    assert_eq!(sys(k, nr::LSETXATTR, &[file, name, 0, 0, 0]), Ok(0));
    assert_eq!(sys(k, nr::GETXATTR, &[file, name, buf, 64]), Ok(0));
    let acl = s(k, 0x100, b"system.posix_acl_access");
    assert_eq!(sys(k, nr::FGETXATTR, &[fd, acl, buf, 64]), Err(Errno::ENODATA));
    k.poke(value, &poly_vfs::xattr::acl::from_mode(0o640));
    assert_eq!(sys(k, nr::FSETXATTR, &[fd, acl, value, 28, 0]), Ok(0));
    let st = e.vfs.stat(&e.vfs.root_context().unwrap(), &poly_vfs::Cred::root(), poly_vfs::At::Cwd, b"/dir/file", true);
    assert_eq!(st.map(|s| s.mode), Ok(0o100640));
}

#[test]
fn xattr_arguments() {
    let mut e = env();
    let k = &mut e.k;
    let file = s(k, 0, b"/dir/file");
    let name = s(k, 0x80, b"user.x");
    let buf = scratch(0x3000);
    let empty = s(k, 0xc0, b"");
    let mut long = b"user.".to_vec();
    long.resize(256, b'a');
    let long = s(k, 0x200, &long);
    assert_eq!(sys(k, nr::GETXATTR, &[file, empty, buf, 64]), Err(Errno::ERANGE));
    assert_eq!(sys(k, nr::GETXATTR, &[file, long, buf, 64]), Err(Errno::ERANGE));
    assert_eq!(sys(k, nr::GETXATTR, &[file, UNMAPPED, buf, 64]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::GETXATTR, &[empty, name, buf, 64]), Err(Errno::ENOENT));
    let nope = s(k, 0x300, b"/nope");
    assert_eq!(sys(k, nr::GETXATTR, &[nope, name, buf, 64]), Err(Errno::ENOENT));
    let other = s(k, 0x340, b"other.x");
    assert_eq!(sys(k, nr::GETXATTR, &[file, other, buf, 64]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::SETXATTR, &[file, name, buf, 4, 4]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::SETXATTR, &[file, name, buf, 65537, 0]), Err(Errno::E2BIG));
    assert_eq!(sys(k, nr::SETXATTR, &[file, name, UNMAPPED, 4, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::FGETXATTR, &[-1i64 as u64, name, buf, 64]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FLISTXATTR, &[99, buf, 64]), Err(Errno::EBADF));
    let path_fd = sys(k, nr::OPENAT, &[AT, file, O_PATH, 0]).unwrap();
    assert_eq!(sys(k, nr::FLISTXATTR, &[path_fd, buf, 64]), Err(Errno::EBADF));
    // The value buffer must be writable once there is something to copy.
    assert_eq!(sys(k, nr::SETXATTR, &[file, name, buf, 4, 0]), Ok(0));
    assert_eq!(sys(k, nr::GETXATTR, &[file, name, UNMAPPED, 64]), Err(Errno::EFAULT));
    // Without kernel services.
    let mut bare = FakeKernel::new();
    let p = s(&mut bare, 0, b"/x");
    let n = s(&mut bare, 0x80, b"user.x");
    assert_eq!(sys(&mut bare, nr::GETXATTR, &[p, n, 0, 0]), Err(Errno::ENOSYS));
}
