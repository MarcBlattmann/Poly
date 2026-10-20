//! read, write, readv, writev, close, fstat, ioctl.

extern crate std;

use super::fake::*;
use crate::abi::{FileStat, ioctl, le_u64, mode};
use crate::{Errno, nr};
use std::vec;
use std::vec::Vec;

const HELLO: &[u8] = b"Hello, world!\n";

#[test]
fn write_sends_user_bytes_to_the_file() {
    let mut k = FakeKernel::new();
    k.poke(scratch(0), HELLO);
    assert_eq!(sys(&mut k, nr::WRITE, &[1, scratch(0), 14]), Ok(14));
    assert_eq!(k.file(1).written, HELLO);
}

#[test]
fn write_errors() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::WRITE, &[9, scratch(0), 1]), Err(Errno::EBADF), "closed fd");
    assert_eq!(sys(&mut k, nr::WRITE, &[u64::MAX, scratch(0), 1]), Err(Errno::EBADF), "fd -1");
    assert_eq!(sys(&mut k, nr::WRITE, &[1, UNMAPPED, 1]), Err(Errno::EFAULT));
    k.file_mut(0).writable = false;
    assert_eq!(sys(&mut k, nr::WRITE, &[0, scratch(0), 1]), Err(Errno::EBADF), "not open for writing");
    k.file_mut(1).fail_after = Some((0, Errno::EPIPE));
    assert_eq!(sys(&mut k, nr::WRITE, &[1, scratch(0), 1]), Err(Errno::EPIPE));
    assert!(k.file(1).written.is_empty());
}

#[test]
fn zero_length_write_checks_the_fd_only() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::WRITE, &[1, UNMAPPED, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::WRITE, &[5, scratch(0), 0]), Err(Errno::EBADF));
}

#[test]
fn fd_arguments_use_the_low_32_bits() {
    let mut k = FakeKernel::new();
    k.poke(scratch(0), b"x");
    assert_eq!(sys(&mut k, nr::WRITE, &[0xdead_0000_0000_0001, scratch(0), 1]), Ok(1));
    assert_eq!(k.file(1).written, b"x");
}

#[test]
fn large_writes_go_out_in_chunks_and_complete() {
    let mut k = FakeKernel::new();
    let len = 150 * 1024;
    k.map_range(0x100_0000, len, RW);
    let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    k.poke(0x100_0000, &data);
    assert_eq!(sys(&mut k, nr::WRITE, &[1, 0x100_0000, len]), Ok(len));
    assert_eq!(k.file(1).written, data);
}

#[test]
fn short_writes_are_reported() {
    let mut k = FakeKernel::new();
    k.file_mut(1).max_write = Some(5);
    k.poke(scratch(0), HELLO);
    assert_eq!(sys(&mut k, nr::WRITE, &[1, scratch(0), 14]), Ok(5));
    assert_eq!(k.file(1).written, b"Hello");
}

#[test]
fn an_error_after_progress_returns_the_count() {
    let mut k = FakeKernel::new();
    let len = 100 * 1024;
    k.map_range(0x100_0000, len, RW);
    k.file_mut(1).fail_after = Some((64 * 1024, Errno::ENOSPC));
    assert_eq!(sys(&mut k, nr::WRITE, &[1, 0x100_0000, len]), Ok(64 * 1024));
}

#[test]
fn a_fault_after_progress_returns_the_count() {
    let mut k = FakeKernel::new();
    // 64 KiB mapped, then a hole.
    k.map_range(0x100_0000, 64 * 1024, RW);
    assert_eq!(sys(&mut k, nr::WRITE, &[1, 0x100_0000, 100 * 1024]), Ok(64 * 1024));
}

#[test]
fn read_copies_file_data_to_user_memory() {
    let mut k = FakeKernel::new();
    k.file_mut(0).input = b"input line\n".to_vec();
    assert_eq!(sys(&mut k, nr::READ, &[0, scratch(0), 4]), Ok(4));
    assert_eq!(k.peek(scratch(0), 4), b"inpu");
    assert_eq!(sys(&mut k, nr::READ, &[0, scratch(0), 100]), Ok(7));
    assert_eq!(k.peek(scratch(0), 7), b"t line\n");
    assert_eq!(sys(&mut k, nr::READ, &[0, scratch(0), 100]), Ok(0), "end of file");
}

#[test]
fn read_errors() {
    let mut k = FakeKernel::new();
    k.file_mut(0).input = b"abc".to_vec();
    assert_eq!(sys(&mut k, nr::READ, &[42, scratch(0), 3]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::READ, &[u64::MAX, scratch(0), 3]), Err(Errno::EBADF));
    let ro = read_only_page(&mut k);
    assert_eq!(sys(&mut k, nr::READ, &[0, ro, 3]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::READ, &[0, UNMAPPED, 0]), Ok(0), "zero-length read touches no memory");
    k.file_mut(1).readable = false;
    assert_eq!(sys(&mut k, nr::READ, &[1, scratch(0), 3]), Err(Errno::EBADF));
}

#[test]
fn writev_gathers_in_order() {
    let mut k = FakeKernel::new();
    k.poke(scratch(0x100), b"Hello");
    k.poke(scratch(0x200), b", world");
    k.poke(scratch(0x300), b"!\n");
    let iov =
        put_iovecs(&mut k, scratch(0), &[(scratch(0x100), 5), (UNMAPPED, 0), (scratch(0x200), 7), (scratch(0x300), 2)]);
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, iov, 4]), Ok(14));
    assert_eq!(k.file(1).written, HELLO);
}

#[test]
fn writev_errors() {
    let mut k = FakeKernel::new();
    let iov = put_iovecs(&mut k, scratch(0), &[(scratch(0x100), 1)]);
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, iov, u64::MAX]), Err(Errno::EINVAL), "count -1");
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, iov, 1025]), Err(Errno::EINVAL), "count above IOV_MAX");
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, UNMAPPED, 1]), Err(Errno::EFAULT), "unreadable iovec array");
    assert_eq!(sys(&mut k, nr::WRITEV, &[3, iov, 1]), Err(Errno::EBADF));
    let neg = put_iovecs(&mut k, scratch(0x40), &[(scratch(0x100), u64::MAX)]);
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, neg, 1]), Err(Errno::EINVAL), "length is negative as ssize_t");
    let over = put_iovecs(&mut k, scratch(0x80), &[(scratch(0x100), i64::MAX as u64), (scratch(0x100), 1)]);
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, over, 2]), Err(Errno::EINVAL), "sum overflows ssize_t");
    let bad = put_iovecs(&mut k, scratch(0xc0), &[(UNMAPPED, 4)]);
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, bad, 1]), Err(Errno::EFAULT), "unreadable buffer");
    assert!(k.file(1).written.is_empty());
}

#[test]
fn writev_accepts_iov_max_and_zero_vectors() {
    let mut k = FakeKernel::new();
    k.map_range(0x100_0000, 1024 * 16, RW);
    k.poke(scratch(0x100), b"z");
    let iovs: Vec<(u64, u64)> = (0..1024).map(|_| (scratch(0x100), 1)).collect();
    let iov = put_iovecs(&mut k, 0x100_0000, &iovs);
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, iov, 1024]), Ok(1024));
    assert_eq!(k.file(1).written, vec![b'z'; 1024]);
    assert_eq!(sys(&mut k, nr::WRITEV, &[1, UNMAPPED, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::WRITEV, &[7, UNMAPPED, 0]), Err(Errno::EBADF));
}

#[test]
fn readv_scatters_in_order() {
    let mut k = FakeKernel::new();
    k.file_mut(0).input = b"abcdefgh".to_vec();
    let iov = put_iovecs(&mut k, scratch(0), &[(scratch(0x100), 3), (scratch(0x200), 0), (scratch(0x300), 10)]);
    assert_eq!(sys(&mut k, nr::READV, &[0, iov, 3]), Ok(8));
    assert_eq!(k.peek(scratch(0x100), 3), b"abc");
    assert_eq!(k.peek(scratch(0x300), 5), b"defgh");
    assert_eq!(sys(&mut k, nr::READV, &[0, iov, 1025]), Err(Errno::EINVAL));
}

#[test]
fn close_releases_the_fd_once() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::CLOSE, &[2]), Ok(0));
    assert_eq!(sys(&mut k, nr::CLOSE, &[2]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::CLOSE, &[u64::MAX]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::WRITE, &[2, scratch(0), 1]), Err(Errno::EBADF));
}

#[test]
fn fstat_writes_struct_stat() {
    let mut k = FakeKernel::new();
    k.poke(scratch(0x100), &[0xaa; FileStat::SIZE + 8]);
    assert_eq!(sys(&mut k, nr::FSTAT, &[1, scratch(0x100)]), Ok(0));
    let b = k.peek(scratch(0x100), FileStat::SIZE + 8);
    let st_mode = u32::from_le_bytes(b[24..28].try_into().unwrap());
    assert_eq!(st_mode & mode::S_IFMT, mode::S_IFIFO);
    assert_eq!(le_u64(&b[8..]), 1001, "st_ino");
    assert_eq!(le_u64(&b[56..]), 4096, "st_blksize");
    assert_eq!(&b[FileStat::SIZE..], &[0xaa; 8], "nothing written past the struct");
}

#[test]
fn fstat_errors() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::FSTAT, &[8, scratch(0)]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::FSTAT, &[1, UNMAPPED]), Err(Errno::EFAULT));
    let ro = read_only_page(&mut k);
    assert_eq!(sys(&mut k, nr::FSTAT, &[1, ro]), Err(Errno::EFAULT));
}

#[test]
fn tcgets_is_enotty_for_files_that_are_not_terminals() {
    let mut k = FakeKernel::new();
    let tcgets = ioctl::TCGETS as u64;
    assert_eq!(sys(&mut k, nr::IOCTL, &[1, tcgets, scratch(0)]), Err(Errno::ENOTTY), "pipe");
    k.files.insert(3, File::new(Kind::Regular));
    assert_eq!(sys(&mut k, nr::IOCTL, &[3, tcgets, scratch(0)]), Err(Errno::ENOTTY), "regular file");
    assert!(k.ioctls.is_empty(), "answered without asking the kernel");
    k.files.insert(4, File::new(Kind::NullDevice));
    assert_eq!(sys(&mut k, nr::IOCTL, &[4, tcgets, scratch(0)]), Err(Errno::ENOTTY), "/dev/null");
    assert_eq!(k.ioctls, [(4, ioctl::TCGETS, scratch(0))], "character devices ask the kernel");
}

#[test]
fn tcgets_on_a_terminal_reaches_the_kernel() {
    let mut k = FakeKernel::new();
    k.poke(scratch(0), &[0xff; 36]);
    assert_eq!(sys(&mut k, nr::IOCTL, &[0, ioctl::TCGETS as u64, scratch(0)]), Ok(0));
    assert_eq!(k.peek(scratch(0), 36), [0; 36]);
    assert_eq!(sys(&mut k, nr::IOCTL, &[0, ioctl::TCGETS as u64, UNMAPPED]), Err(Errno::EFAULT));
}

#[test]
fn ioctl_errors_and_generic_requests() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::IOCTL, &[9, ioctl::TCGETS as u64, scratch(0)]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::IOCTL, &[u64::MAX, ioctl::TCGETS as u64, scratch(0)]), Err(Errno::EBADF));
    // FIONREAD has type 'T' but applies to every file: it goes to the kernel.
    k.file_mut(1).input = b"12345".to_vec();
    assert_eq!(sys(&mut k, nr::IOCTL, &[1, ioctl::FIONREAD as u64, scratch(0)]), Ok(0));
    assert_eq!(k.peek(scratch(0), 4), 5u32.to_le_bytes());
    // Requests the file does not know: ENOTTY from the kernel.
    assert_eq!(sys(&mut k, nr::IOCTL, &[1, 0x8000_1234, scratch(0)]), Err(Errno::ENOTTY));
    assert!(ioctl::is_terminal_request(ioctl::TIOCGWINSZ));
    assert!(!ioctl::is_terminal_request(ioctl::FIOCLEX));
    assert!(ioctl::is_terminal_request(0x8004_5430), "TIOCGPTN, an _IOR('T', 0x30) request");
}
