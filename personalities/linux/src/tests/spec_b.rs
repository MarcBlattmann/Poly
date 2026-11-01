//! Behaviours of spec-linux-b's entries (the file-system calls): preadv,
//! pwritev, preadv2 and pwritev2, fallocate, flock's `LOCK_MAND` and its
//! order of errors, truncate's order of errors; against a [`LinuxFs`] over
//! a real `poly-vfs` tmpfs (the fixture of `files.rs`).
//!
//! [`LinuxFs`]: crate::fs::LinuxFs

extern crate std;

use super::fake::*;
use super::files::{AT, env, s};
use crate::kernel::LinuxKernel;
use crate::{Errno, nr};
use std::vec::Vec;

const O_RDONLY: u64 = 0;
const O_WRONLY: u64 = 1;
const O_RDWR: u64 = 2;
const O_APPEND: u64 = 0o2000;
const O_DIRECTORY: u64 = 0o200000;
const O_PATH: u64 = 0o10000000;
const SEEK_CUR: u64 = 1;
const BAD: u64 = 900;

fn open(k: &mut FakeKernel, path: &[u8], flags: u64) -> u64 {
    let p = s(k, 0x100, path);
    sys(k, nr::OPENAT, &[AT, p, flags, 0o644]).unwrap()
}

fn contents(k: &mut FakeKernel, path: &[u8]) -> Vec<u8> {
    let fd = open(k, path, O_RDONLY);
    let n = sys(k, nr::PREAD64, &[fd, scratch(0x4000), 0x1000, 0]).unwrap();
    k.peek(scratch(0x4000), n as usize)
}

fn pos(k: &mut FakeKernel, fd: u64) -> u64 {
    sys(k, nr::LSEEK, &[fd, 0, SEEK_CUR]).unwrap()
}

/// Three iovecs of 2, 3 and 1 bytes at scratch 0x3000.. and their array.
fn three(k: &mut FakeKernel) -> u64 {
    put_iovecs(k, scratch(0x2000), &[(scratch(0x3000), 2), (scratch(0x3100), 3), (scratch(0x3200), 1)])
}

// ---- preadv, pwritev -------------------------------------------------------

#[test]
fn preadv_scatters_at_an_offset_and_leaves_the_position() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDONLY);
    let iov = three(k);
    // "contents" from offset 1: "on", "ten", "t".
    assert_eq!(sys(k, nr::PREADV, &[fd, iov, 3, 1, 0x1234_5678]), Ok(6));
    assert_eq!(k.peek(scratch(0x3000), 2), b"on");
    assert_eq!(k.peek(scratch(0x3100), 3), b"ten");
    assert_eq!(k.peek(scratch(0x3200), 1), b"t");
    assert_eq!(pos(k, fd), 0);
    // Short at the end, 0 past it; iovcnt 0 (NULL array) is 0, on a
    // directory too.
    assert_eq!(sys(k, nr::PREADV, &[fd, iov, 3, 5, 0]), Ok(3));
    assert_eq!(sys(k, nr::PREADV, &[fd, iov, 3, 100, 0]), Ok(0));
    let dir = open(k, b"/dir", O_DIRECTORY);
    assert_eq!(sys(k, nr::PREADV, &[dir, 0, 0, 0, 0]), Ok(0));
}

#[test]
fn preadv_errors_come_in_their_order() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDONLY);
    let wo = open(k, b"/dir/file", O_WRONLY);
    let path = open(k, b"/dir/file", O_PATH);
    let dir = open(k, b"/dir", O_DIRECTORY);
    let iov = three(k);
    let minus = |o: i64| o as u64;
    // The offset before the descriptor; -1 has no meaning here.
    assert_eq!(sys(k, nr::PREADV, &[BAD, iov, 3, minus(-1), 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::PREADV, &[BAD, iov, 3, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::PREADV, &[wo, iov, 3, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::PREADV, &[path, iov, 3, 0, 0]), Err(Errno::EBADF));
    // The array (count, unreadable) before EISDIR.
    assert_eq!(sys(k, nr::PREADV, &[dir, iov, 1025, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::PREADV, &[dir, UNMAPPED, 1, 0, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::PREADV, &[dir, iov, 3, 0, 0]), Err(Errno::EISDIR));
    // A negative length; a bad first buffer; a bad later one returns what
    // the earlier ones got.
    let neg = put_iovecs(k, scratch(0x2400), &[(scratch(0x3000), 2), (scratch(0x3100), u64::MAX)]);
    assert_eq!(sys(k, nr::PREADV, &[fd, neg, 2, 0, 0]), Err(Errno::EINVAL));
    let bad1 = put_iovecs(k, scratch(0x2400), &[(UNMAPPED, 2)]);
    assert_eq!(sys(k, nr::PREADV, &[fd, bad1, 1, 0, 0]), Err(Errno::EFAULT));
    let bad2 = put_iovecs(k, scratch(0x2400), &[(scratch(0x3000), 4), (1, 4)]);
    assert_eq!(sys(k, nr::PREADV, &[fd, bad2, 2, 0, 0]), Ok(4));
}

#[test]
fn pwritev_gathers_at_an_offset_and_appends_with_o_append() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDWR);
    k.poke(scratch(0x3000), b"ab");
    k.poke(scratch(0x3100), b"cde");
    k.poke(scratch(0x3200), b"f");
    let iov = three(k);
    assert_eq!(sys(k, nr::PWRITEV, &[fd, iov, 3, 2, 0]), Ok(6));
    assert_eq!(contents(k, b"/dir/file"), b"coabcdef");
    assert_eq!(pos(k, fd), 0);
    // Past the end: a gap of zeros.
    assert_eq!(sys(k, nr::PWRITEV, &[fd, iov, 1, 10, 0]), Ok(2));
    assert_eq!(contents(k, b"/dir/file"), b"coabcdef\0\0ab");
    // O_APPEND: the end, whatever the offset; the position stays.
    let ap = open(k, b"/dir/file", O_WRONLY | O_APPEND);
    assert_eq!(sys(k, nr::PWRITEV, &[ap, iov, 1, 0, 0]), Ok(2));
    assert_eq!(contents(k, b"/dir/file"), b"coabcdef\0\0abab");
    assert_eq!(pos(k, ap), 0);
    // Not open for writing; iovcnt 0 changes nothing.
    let ro = open(k, b"/dir/file", O_RDONLY);
    assert_eq!(sys(k, nr::PWRITEV, &[ro, iov, 3, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::PWRITEV, &[fd, 0, 0, 0, 0]), Ok(0));
    assert_eq!(sys(k, nr::PWRITEV, &[fd, iov, 3, -1i64 as u64, 0]), Err(Errno::EINVAL));
}

// ---- preadv2, pwritev2 -----------------------------------------------------

#[test]
fn preadv2_reads_at_the_position_with_minus_one_and_checks_its_flags() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDONLY);
    let iov = put_iovecs(k, scratch(0x2000), &[(scratch(0x3000), 4)]);
    let m1 = -1i64 as u64;
    assert_eq!(sys(k, nr::PREADV2, &[fd, iov, 1, m1, 0, 0]), Ok(4));
    assert_eq!(sys(k, nr::PREADV2, &[fd, iov, 1, m1, 0, 0]), Ok(4));
    assert_eq!(k.peek(scratch(0x3000), 4), b"ents");
    assert_eq!(pos(k, fd), 8);
    // Flags that change nothing about a read.
    for flags in [1, 2, 4, 0x10, 0x20, 0x17] {
        assert_eq!(sys(k, nr::PREADV2, &[fd, iov, 1, 0, 0, flags]), Ok(4), "flags {flags:#x}");
    }
    // RWF_NOWAIT on a tmpfs file; unknown flags, after the offset and the
    // descriptor, before EISDIR.
    assert_eq!(sys(k, nr::PREADV2, &[fd, iov, 1, 0, 0, 8]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::PREADV2, &[fd, iov, 1, 0, 0, 0x4000]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::PREADV2, &[fd, iov, 1, -2i64 as u64, 0, 0x4000]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::PREADV2, &[BAD, iov, 1, 0, 0, 0x4000]), Err(Errno::EBADF));
    let dir = open(k, b"/dir", O_DIRECTORY);
    assert_eq!(sys(k, nr::PREADV2, &[dir, iov, 1, 0, 0, 0x4000]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::PREADV2, &[dir, iov, 1, m1, 0, 0]), Err(Errno::EISDIR));
}

#[test]
fn pwritev2_appends_or_not_for_one_call() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDWR);
    k.poke(scratch(0x3000), b"XY");
    let iov = put_iovecs(k, scratch(0x2000), &[(scratch(0x3000), 2)]);
    let m1 = -1i64 as u64;
    // -1: at the position, which advances.
    assert_eq!(sys(k, nr::PWRITEV2, &[fd, iov, 1, m1, 0, 0]), Ok(2));
    assert_eq!(pos(k, fd), 2);
    assert_eq!(contents(k, b"/dir/file"), b"XYntents");
    // RWF_APPEND at an offset: the end, the position stays; with -1 the
    // position moves to the new end; the file does not become O_APPEND.
    assert_eq!(sys(k, nr::PWRITEV2, &[fd, iov, 1, 0, 0, 0x10]), Ok(2));
    assert_eq!(contents(k, b"/dir/file"), b"XYntentsXY");
    assert_eq!(pos(k, fd), 2);
    assert_eq!(sys(k, nr::PWRITEV2, &[fd, iov, 1, m1, 0, 0x10]), Ok(2));
    assert_eq!(pos(k, fd), 12);
    assert_eq!(sys(k, nr::FCNTL, &[fd, 3, 0]).unwrap() & O_APPEND, 0);
    // RWF_NOAPPEND on an O_APPEND descriptor: at the offset.
    let ap = open(k, b"/dir/file", O_WRONLY | O_APPEND);
    k.poke(scratch(0x3000), b"__");
    assert_eq!(sys(k, nr::PWRITEV2, &[ap, iov, 1, 4, 0, 0x20]), Ok(2));
    assert_eq!(contents(k, b"/dir/file"), b"XYnt__tsXYXY");
    // RWF_NOWAIT on a tmpfs file writes nothing; unknown flags.
    assert_eq!(sys(k, nr::PWRITEV2, &[fd, iov, 1, 0, 0, 8]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::PWRITEV2, &[fd, iov, 1, 0, 0, 0x8000_0000]), Err(Errno::EOPNOTSUPP));
    assert_eq!(contents(k, b"/dir/file"), b"XYnt__tsXYXY");
    assert_eq!(sys(k, nr::PWRITEV2, &[fd, iov, 1, -2i64 as u64, 0, 0]), Err(Errno::EINVAL));
}

// ---- fallocate -----------------------------------------------------------------

#[test]
fn fallocate_grows_keeps_and_punches() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDWR);
    assert_eq!(sys(k, nr::FALLOCATE, &[fd, 0, 0, 100]), Ok(0));
    let size = |k: &mut FakeKernel| k.fstat(fd as i32).unwrap().size;
    assert_eq!(size(k), 100);
    assert_eq!(sys(k, nr::FALLOCATE, &[fd, 1, 0, 1000]), Ok(0));
    assert_eq!(size(k), 100);
    assert_eq!(sys(k, nr::FALLOCATE, &[fd, 3, 0, 2]), Ok(0));
    assert_eq!(&contents(k, b"/dir/file")[..4], b"\0\0nt");
    assert_eq!(pos(k, fd), 0);
}

#[test]
fn fallocate_errors_come_in_their_order() {
    let mut e = env();
    let k = &mut e.k;
    let rw = open(k, b"/dir/file", O_RDWR);
    let ro = open(k, b"/dir/file", O_RDONLY);
    let path = open(k, b"/dir/file", O_PATH);
    let m1 = -1i64 as u64;
    assert_eq!(sys(k, nr::FALLOCATE, &[m1, 0, m1, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FALLOCATE, &[BAD, 0, 0, 10]), Err(Errno::EBADF));
    // O_PATH is refused like a closed descriptor, before the range.
    assert_eq!(sys(k, nr::FALLOCATE, &[path, 0, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FALLOCATE, &[ro, 0, m1, 10]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FALLOCATE, &[rw, 0, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FALLOCATE, &[rw, 0x4000_0000, m1, 10]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FALLOCATE, &[rw, 0x4000_0000, 0, 10]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::FALLOCATE, &[ro, 2, 0, 10]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::FALLOCATE, &[ro, 0, 0, 10]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FALLOCATE, &[rw, 0, 0x7fff_ffff_ffff_ff00, 0x1000]), Err(Errno::EFBIG));
    // A mode tmpfs does not support.
    assert_eq!(sys(k, nr::FALLOCATE, &[rw, 0x10, 0, 10]), Err(Errno::EOPNOTSUPP));
}

// ---- flock ---------------------------------------------------------------------

#[test]
fn flock_lock_mand_is_a_no_op_and_einval_comes_before_ebadf() {
    let mut e = env();
    let k = &mut e.k;
    let fd = open(k, b"/dir/file", O_RDONLY);
    let path = open(k, b"/dir/file", O_PATH);
    let m1 = -1i64 as u64;
    assert_eq!(sys(k, nr::FLOCK, &[fd, 32]), Ok(0));
    assert_eq!(sys(k, nr::FLOCK, &[m1, 32 | 2]), Ok(0));
    assert_eq!(sys(k, nr::FLOCK, &[m1, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FLOCK, &[m1, 2]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FLOCK, &[path, 2]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FLOCK, &[fd, 2]), Ok(0));
}

// ---- truncate --------------------------------------------------------------

#[test]
fn truncate_checks_the_length_before_the_path() {
    let mut e = env();
    let k = &mut e.k;
    let m1 = -1i64 as u64;
    assert_eq!(sys(k, nr::TRUNCATE, &[0, m1]), Err(Errno::EINVAL));
    let missing = s(k, 0x100, b"/nope");
    assert_eq!(sys(k, nr::TRUNCATE, &[missing, m1]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::TRUNCATE, &[missing, 1]), Err(Errno::ENOENT));
    assert_eq!(sys(k, nr::TRUNCATE, &[0, 1]), Err(Errno::EFAULT));
}
