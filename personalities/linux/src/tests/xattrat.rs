//! setxattrat(2) and friends, file_getattr(2) / file_setattr(2),
//! name_to_handle_at(2) / open_by_handle_at(2), against a
//! [`LinuxFs`](crate::fs::LinuxFs) over a real `poly-vfs` tmpfs (the
//! fixture of `files.rs`: `/dir/file`, `/dir/sub/`, `/link` -> `dir/file`).

extern crate std;

use super::fake::*;
use super::files::{AT, env, s};
use crate::{Errno, nr};

const AT_SYMLINK_NOFOLLOW: u64 = 0x100;
const AT_SYMLINK_FOLLOW: u64 = 0x400;
const AT_EMPTY_PATH: u64 = 0x1000;
const O_RDONLY: u64 = 0;
const O_RDWR: u64 = 2;
const O_DIRECTORY: u64 = 0o200000;
const O_PATH: u64 = 0o10000000;

/// A `struct xattr_args` at scratch `at`.
fn args(k: &mut FakeKernel, at: u64, value: u64, size: u32, flags: u32) -> u64 {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&value.to_le_bytes());
    b[8..12].copy_from_slice(&size.to_le_bytes());
    b[12..].copy_from_slice(&flags.to_le_bytes());
    k.poke(scratch(at), &b);
    scratch(at)
}

#[test]
fn xattrat_set_get_list_remove() {
    let mut e = env();
    let k = &mut e.k;
    let file = s(k, 0, b"/dir/file");
    let link = s(k, 0x40, b"/link");
    let name = s(k, 0x80, b"user.k");
    let value = s(k, 0xc0, b"one");
    let buf = scratch(0x3000);
    let a = args(k, 0x400, value, 3, 0);
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, a, 16]), Ok(0));
    assert_eq!(sys(k, nr::GETXATTR, &[file, name, buf, 64]), Ok(3));
    let a = args(k, 0x400, value, 3, 1);
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, a, 16]), Err(Errno::EEXIST));
    // A user. name on a link itself is EPERM; following it reaches the file.
    let a = args(k, 0x400, value, 3, 0);
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, link, AT_SYMLINK_NOFOLLOW, name, a, 16]), Err(Errno::EPERM));
    let g = args(k, 0x440, buf, 64, 0);
    assert_eq!(sys(k, nr::GETXATTRAT, &[AT, link, 0, name, g, 16]), Ok(3));
    assert_eq!(k.peek(buf, 3), b"one");
    let q = args(k, 0x480, 0, 0, 0);
    assert_eq!(sys(k, nr::GETXATTRAT, &[AT, file, 0, name, q, 16]), Ok(3), "size 0 asks for the length");
    let small = args(k, 0x4c0, buf, 2, 0);
    assert_eq!(sys(k, nr::GETXATTRAT, &[AT, file, 0, name, small, 16]), Err(Errno::ERANGE));
    // By descriptor (AT_EMPTY_PATH, also with a NULL path) and relative to one.
    let dir = s(k, 0x100, b"/dir");
    let dfd = sys(k, nr::OPENAT, &[AT, dir, O_RDONLY | O_DIRECTORY, 0]).unwrap();
    let rel = s(k, 0x140, b"file");
    assert_eq!(sys(k, nr::GETXATTRAT, &[dfd, rel, 0, name, g, 16]), Ok(3));
    let fd = sys(k, nr::OPENAT, &[AT, file, O_RDONLY, 0]).unwrap();
    let empty = s(k, 0x180, b"");
    assert_eq!(sys(k, nr::GETXATTRAT, &[fd, empty, AT_EMPTY_PATH, name, g, 16]), Ok(3));
    assert_eq!(sys(k, nr::GETXATTRAT, &[fd, 0, AT_EMPTY_PATH, name, g, 16]), Ok(3));
    assert_eq!(sys(k, nr::GETXATTRAT, &[fd, empty, 0, name, g, 16]), Err(Errno::ENOENT));
    let opath = sys(k, nr::OPENAT, &[AT, file, O_PATH, 0]).unwrap();
    assert_eq!(sys(k, nr::GETXATTRAT, &[opath, empty, AT_EMPTY_PATH, name, g, 16]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::LISTXATTRAT, &[AT, file, 0, buf, 64]), Ok(7));
    assert_eq!(sys(k, nr::LISTXATTRAT, &[AT, file, 0, 0, 0]), Ok(7));
    assert_eq!(sys(k, nr::LISTXATTRAT, &[AT, file, 0, buf, 6]), Err(Errno::ERANGE));
    assert_eq!(sys(k, nr::LISTXATTRAT, &[AT, file, 0, buf, 1 << 20]), Ok(7));
    assert_eq!(sys(k, nr::LISTXATTRAT, &[AT, link, AT_SYMLINK_NOFOLLOW, buf, 64]), Ok(0));
    assert_eq!(sys(k, nr::REMOVEXATTRAT, &[AT, link, AT_SYMLINK_NOFOLLOW, name]), Err(Errno::EPERM));
    assert_eq!(sys(k, nr::REMOVEXATTRAT, &[fd, empty, AT_EMPTY_PATH, name]), Ok(0));
    assert_eq!(sys(k, nr::REMOVEXATTRAT, &[AT, file, 0, name]), Err(Errno::ENODATA));
}

#[test]
fn xattrat_argument_errors() {
    let mut e = env();
    let k = &mut e.k;
    let file = s(k, 0, b"/dir/file");
    let name = s(k, 0x80, b"user.k");
    let value = s(k, 0xc0, b"one");
    let a = args(k, 0x400, value, 3, 0);
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, a, 8]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, a, 4097]), Err(Errno::E2BIG));
    // A larger structure is fine with a zero tail, E2BIG with another.
    k.poke(scratch(0x410), &[0; 8]);
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, a, 24]), Ok(0));
    k.poke(scratch(0x417), &[1]);
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, a, 24]), Err(Errno::E2BIG));
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0x400, name, a, 16]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0x800, name, a, 16]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, 0, 16]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, UNMAPPED, 16]), Err(Errno::EFAULT));
    let bad = args(k, 0x440, value, 3, 4);
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, bad, 16]), Err(Errno::EINVAL));
    let unreadable = args(k, 0x480, UNMAPPED, 3, 0);
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, name, unreadable, 16]), Err(Errno::EFAULT));
    // The arguments come before the path: a bad name with a missing file.
    let empty = s(k, 0x100, b"");
    let nope = s(k, 0x140, b"/nope");
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, nope, 0, empty, a, 16]), Err(Errno::ERANGE));
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, nope, 0, name, a, 16]), Err(Errno::ENOENT));
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, 0, 0, name, a, 16]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::SETXATTRAT, &[99, empty, AT_EMPTY_PATH, name, a, 16]), Err(Errno::EBADF));
    let plain = s(k, 0x180, b"plain");
    assert_eq!(sys(k, nr::SETXATTRAT, &[AT, file, 0, plain, a, 16]), Err(Errno::EOPNOTSUPP));
    // getxattrat's flags must be 0; an unwritable buffer is EFAULT.
    let g = args(k, 0x4c0, scratch(0x3000), 16, 1);
    assert_eq!(sys(k, nr::GETXATTRAT, &[AT, file, 0, name, g, 16]), Err(Errno::EINVAL));
    let g = args(k, 0x4c0, UNMAPPED, 16, 0);
    assert_eq!(sys(k, nr::GETXATTRAT, &[AT, file, 0, name, g, 16]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::LISTXATTRAT, &[AT, file, 0, UNMAPPED, 64]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::LISTXATTRAT, &[AT, file, 0x400, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::REMOVEXATTRAT, &[AT, file, 0x400, name]), Err(Errno::EINVAL));
}

/// A `struct file_attr` with `xflags` at scratch `at`.
fn fattr(k: &mut FakeKernel, at: u64, xflags: u64) -> u64 {
    let mut b = [0u8; 24];
    b[..8].copy_from_slice(&xflags.to_le_bytes());
    k.poke(scratch(at), &b);
    scratch(at)
}

#[test]
fn file_attributes() {
    let mut e = env();
    let k = &mut e.k;
    let file = s(k, 0, b"/dir/file");
    let link = s(k, 0x40, b"/link");
    let out = scratch(0x3000);
    k.poke(out, &[0xee; 32]);
    assert_eq!(sys(k, nr::FILE_GETATTR, &[AT, file, out, 32, 0]), Ok(0));
    assert_eq!(k.peek(out, 32), [0; 32], "a new file: all zero, the tail cleared");
    let nodump = fattr(k, 0x400, 0x80);
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, nodump, 24, 0]), Ok(0));
    assert_eq!(sys(k, nr::FILE_GETATTR, &[AT, link, out, 24, 0]), Ok(0));
    assert_eq!(k.peek(out, 8), 0x80u64.to_le_bytes());
    assert_eq!(sys(k, nr::FILE_GETATTR, &[AT, link, out, 24, AT_SYMLINK_NOFOLLOW]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::FILE_GETATTR, &[AT, file, out, 16, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FILE_GETATTR, &[AT, file, out, 4097, 0]), Err(Errno::E2BIG));
    assert_eq!(sys(k, nr::FILE_GETATTR, &[AT, file, out, 24, 0x400]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FILE_GETATTR, &[AT, file, UNMAPPED, 24, 0]), Err(Errno::EFAULT));
    let empty = s(k, 0x100, b"");
    let fd = sys(k, nr::OPENAT, &[AT, file, O_RDONLY, 0]).unwrap();
    assert_eq!(sys(k, nr::FILE_GETATTR, &[fd, empty, out, 24, AT_EMPTY_PATH]), Ok(0));
    assert_eq!(sys(k, nr::FILE_GETATTR, &[fd, empty, out, 24, 0]), Err(Errno::ENOENT));
    // file_setattr: unknown bits, what tmpfs does not keep, a non-zero tail.
    let unknown = fattr(k, 0x440, 1 << 40);
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, unknown, 24, 0]), Err(Errno::EINVAL));
    let realtime = fattr(k, 0x480, 1);
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, realtime, 24, 0]), Err(Errno::EOPNOTSUPP));
    let clear = fattr(k, 0x4c0, 0);
    k.poke(scratch(0x4c0 + 16), &5u32.to_le_bytes());
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, clear, 24, 0]), Err(Errno::EOPNOTSUPP), "a project id");
    let clear = fattr(k, 0x500, 0);
    k.poke(scratch(0x518), &[0, 0, 0, 0, 0, 0, 0, 1]);
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, clear, 32, 0]), Err(Errno::E2BIG));
    k.poke(scratch(0x518), &[0; 8]);
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, clear, 32, 0]), Ok(0));
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, UNMAPPED, 24, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, link, clear, 24, AT_SYMLINK_NOFOLLOW]), Err(Errno::EOPNOTSUPP));
    // APPEND and IMMUTABLE take effect on opens.
    let append = fattr(k, 0x540, 0x10);
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, append, 24, 0]), Ok(0));
    assert_eq!(sys(k, nr::OPENAT, &[AT, file, O_RDWR, 0]), Err(Errno::EPERM));
    assert!(sys(k, nr::OPENAT, &[AT, file, O_RDWR | 0o2000, 0]).is_ok());
    assert_eq!(sys(k, nr::FILE_SETATTR, &[AT, file, clear, 24, 0]), Ok(0));
}

/// `struct file_handle` with room for `room` bytes at scratch `at`.
fn handle_buf(k: &mut FakeKernel, at: u64, room: u32) -> u64 {
    let mut b = std::vec![0u8; 8 + 128];
    b[..4].copy_from_slice(&room.to_le_bytes());
    k.poke(scratch(at), &b);
    scratch(at)
}

#[test]
fn handles() {
    let mut e = env();
    let k = &mut e.k;
    let file = s(k, 0, b"/dir/file");
    let link = s(k, 0x40, b"/link");
    let mid = scratch(0x3000);
    let h = handle_buf(k, 0x400, 128);
    assert_eq!(sys(k, nr::NAME_TO_HANDLE_AT, &[AT, file, h, mid, 0]), Ok(0));
    let first = k.peek(h, 8 + 12);
    assert_eq!(&first[..4], &12u32.to_le_bytes());
    assert_eq!(&first[4..8], &1i32.to_le_bytes());
    assert_eq!(k.peek(mid, 4), 1i32.to_le_bytes(), "the mount id");
    // The link itself, unless AT_SYMLINK_FOLLOW.
    let hl = handle_buf(k, 0x500, 128);
    assert_eq!(sys(k, nr::NAME_TO_HANDLE_AT, &[AT, link, hl, mid, 0]), Ok(0));
    assert_ne!(k.peek(hl, 20), first);
    assert_eq!(sys(k, nr::NAME_TO_HANDLE_AT, &[AT, link, hl, mid, AT_SYMLINK_FOLLOW | 1]), Ok(0));
    assert_eq!(k.peek(hl, 20), first);
    assert_eq!(k.peek(mid, 8), crate::mountapi::unique_id(1).to_le_bytes(), "AT_HANDLE_MNT_ID_UNIQUE: a u64");
    // Too little room: EOVERFLOW and the size needed; too much asked: EINVAL.
    let small = handle_buf(k, 0x600, 0);
    assert_eq!(sys(k, nr::NAME_TO_HANDLE_AT, &[AT, file, small, mid, 0]), Err(Errno::EOVERFLOW));
    assert_eq!(k.peek(small, 4), 12u32.to_le_bytes());
    let big = handle_buf(k, 0x700, 129);
    assert_eq!(sys(k, nr::NAME_TO_HANDLE_AT, &[AT, file, big, mid, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::NAME_TO_HANDLE_AT, &[AT, file, h, mid, 0x8000]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::NAME_TO_HANDLE_AT, &[AT, file, UNMAPPED, mid, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::NAME_TO_HANDLE_AT, &[AT, file, h, UNMAPPED, 0]), Err(Errno::EFAULT));
    // Open by handle (CAP_DAC_READ_SEARCH, after the size: EPERM without
    // it): read-only, O_PATH, a corrupt handle, bad sizes.
    let root = s(k, 0x100, b"/");
    assert_eq!(sys(k, nr::OPEN_BY_HANDLE_AT, &[AT, h, O_RDONLY]), Err(Errno::EPERM));
    k.poke(scratch(0x800), &0u32.to_le_bytes());
    assert_eq!(sys(k, nr::OPEN_BY_HANDLE_AT, &[AT, scratch(0x800), O_RDONLY]), Err(Errno::EINVAL));
    *k.creds_mut() = crate::cred::Creds::root();
    let mfd = sys(k, nr::OPENAT, &[AT, root, O_RDONLY | O_DIRECTORY, 0]).unwrap();
    let fd = sys(k, nr::OPEN_BY_HANDLE_AT, &[mfd, h, O_RDONLY]).unwrap();
    assert_eq!(sys(k, nr::PREAD64, &[fd, scratch(0x3100), 64, 0]), Ok(8));
    assert_eq!(k.peek(scratch(0x3100), 8), b"contents");
    assert!(sys(k, nr::OPEN_BY_HANDLE_AT, &[mfd, h, O_PATH]).is_ok());
    let mut corrupt = k.peek(h, 20);
    for b in &mut corrupt[8..] {
        *b ^= 0xff;
    }
    k.poke(scratch(0x800), &corrupt);
    assert_eq!(sys(k, nr::OPEN_BY_HANDLE_AT, &[mfd, scratch(0x800), O_RDONLY]), Err(Errno::ESTALE));
    k.poke(scratch(0x800), &0u32.to_le_bytes());
    assert_eq!(sys(k, nr::OPEN_BY_HANDLE_AT, &[mfd, scratch(0x800), O_RDONLY]), Err(Errno::EINVAL));
    k.poke(scratch(0x800), &129u32.to_le_bytes());
    assert_eq!(sys(k, nr::OPEN_BY_HANDLE_AT, &[mfd, scratch(0x800), O_RDONLY]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::OPEN_BY_HANDLE_AT, &[mfd, UNMAPPED, O_RDONLY]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::OPEN_BY_HANDLE_AT, &[99, h, O_RDONLY]), Err(Errno::EBADF));
}
