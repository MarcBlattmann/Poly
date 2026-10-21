//! sendfile(2) (ADR 0051), after sendfile's spec sheet and visible tests:
//! regular files over a real `poly-vfs` tmpfs (the fixture of `files.rs`),
//! pipes and `/dev/null` from the fake's own files, and the argument
//! checks ([`check`]) for every kind of descriptor.

extern crate std;

use super::fake::*;
use super::files::{AT, env, s};
use crate::abi::signal::SIGPIPE;
use crate::fs::rlimit::RLIMIT_FSIZE;
use crate::rlimit::Rlimit;
use crate::sys::sendfile::{Desc, FileKind, LAST_OFFSET, Source, check, check_range, pipe_share};
use crate::{Errno, nr};
use std::vec::Vec;

const O_RDONLY: u64 = 0;
const O_WRONLY: u64 = 1;
const O_RDWR: u64 = 2;
const O_CREAT: u64 = 0o100;
const O_TRUNC: u64 = 0o1000;
const O_APPEND: u64 = 0o2000;
const O_DIRECTORY: u64 = 0o200000;
const O_PATH: u64 = 0o10000000;

/// Where the offset argument lives.
const OFF: u64 = DATA + 0x800;
/// Descriptors of the fake's own files (the VFS hands out 3 and up).
const PIPE: u64 = 50;
const NULL: u64 = 51;

fn open(k: &mut FakeKernel, path: &[u8], flags: u64) -> u64 {
    let p = s(k, 0x100, path);
    sys(k, nr::OPENAT, &[AT, p, flags, 0o644]).unwrap()
}

/// A file at `path` holding `data`, open with `flags`.
fn file(k: &mut FakeKernel, path: &[u8], data: &[u8], flags: u64) -> u64 {
    let fd = open(k, path, O_RDWR | O_CREAT | O_TRUNC);
    k.poke(scratch(0x2000), data);
    assert_eq!(sys(k, nr::PWRITE64, &[fd, scratch(0x2000), data.len() as u64, 0]), Ok(data.len() as u64));
    // (The fake closes only its own files; this descriptor stays open.)
    open(k, path, flags)
}

fn sendfile(k: &mut FakeKernel, out: u64, input: u64, off: u64, count: u64) -> Result<u64, Errno> {
    sys(k, nr::SENDFILE, &[out, input, off, count])
}

/// sendfile with `*offset` = `at`; returns the result and `*offset` after.
fn sendfile_at(k: &mut FakeKernel, out: u64, input: u64, at: i64, count: u64) -> (Result<u64, Errno>, i64) {
    k.poke(OFF, &at.to_le_bytes());
    let r = sendfile(k, out, input, OFF, count);
    (r, i64::from_le_bytes(k.peek(OFF, 8).try_into().unwrap()))
}

fn pos(k: &mut FakeKernel, fd: u64) -> u64 {
    sys(k, nr::LSEEK, &[fd, 0, 1]).unwrap()
}

fn contents(k: &mut FakeKernel, path: &[u8]) -> Vec<u8> {
    let fd = open(k, path, O_RDONLY);
    let n = sys(k, nr::PREAD64, &[fd, scratch(0x4000), 0x1000, 0]).unwrap();
    k.peek(scratch(0x4000), n as usize)
}

fn size(k: &mut FakeKernel, fd: u64) -> u64 {
    assert_eq!(sys(k, nr::FSTAT, &[fd, scratch(0x3000)]), Ok(0));
    u64::from_le_bytes(k.peek(scratch(0x3000) + 48, 8).try_into().unwrap())
}

#[test]
fn null_offset_reads_and_moves_the_input_file_offset() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"0123456789", O_RDONLY);
    let out = open(k, b"/dir/out", O_RDWR | O_CREAT | O_TRUNC);
    assert_eq!(sendfile(k, out, input, 0, 4), Ok(4));
    assert_eq!((pos(k, input), pos(k, out)), (4, 4));
    assert_eq!(sendfile(k, out, input, 0, 3), Ok(3));
    assert_eq!(sendfile(k, out, input, 0, 100), Ok(3), "short at the end");
    assert_eq!(pos(k, input), 10);
    assert_eq!(sendfile(k, out, input, 0, 100), Ok(0), "at the end");
    assert_eq!(sendfile(k, out, input, 0, 0), Ok(0), "count 0");
    assert_eq!(pos(k, out), 10);
    assert_eq!(sys(k, nr::LSEEK, &[input, 2, 0]), Ok(2));
    assert_eq!(sendfile(k, out, input, 0, 2), Ok(2));
    assert_eq!(contents(k, b"/dir/out"), b"012345678923");
}

#[test]
fn a_large_file_moves_in_one_call() {
    let mut e = env();
    let k = &mut e.k;
    let big: Vec<u8> = (0..200_000u32).map(|i| b'a' + (i % 23) as u8).collect();
    let input = open(k, b"/dir/big", O_RDWR | O_CREAT | O_TRUNC);
    for (i, part) in big.chunks(0x1000).enumerate() {
        k.poke(scratch(0x2000), part);
        let at = (i * 0x1000) as u64;
        assert_eq!(sys(k, nr::PWRITE64, &[input, scratch(0x2000), part.len() as u64, at]), Ok(part.len() as u64));
    }
    let out = open(k, b"/dir/out", O_WRONLY | O_CREAT | O_TRUNC);
    assert_eq!(sendfile(k, out, input, 0, 200_000), Ok(200_000));
    assert_eq!(pos(k, input), 200_000);
    assert_eq!(size(k, out), 200_000);
    assert_eq!(sys(k, nr::PREAD64, &[input, scratch(0x2000), 10, 199_990]), Ok(10));
    let tail = k.peek(scratch(0x2000), 10);
    let fd = open(k, b"/dir/out", O_RDONLY);
    assert_eq!(sys(k, nr::PREAD64, &[fd, scratch(0x2000), 10, 199_990]), Ok(10));
    assert_eq!(k.peek(scratch(0x2000), 10), tail);
    assert_eq!(&tail[..], &big[199_990..]);
}

#[test]
fn an_offset_pointer_moves_instead_of_the_file_offset() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"abcdefghij", O_RDONLY);
    let out = open(k, b"/dir/out", O_RDWR | O_CREAT | O_TRUNC);
    assert_eq!(sys(k, nr::READ, &[input, scratch(0x2000), 1]), Ok(1));
    assert_eq!(sendfile_at(k, out, input, 6, 2), (Ok(2), 8));
    assert_eq!((pos(k, input), pos(k, out)), (1, 2));
    assert_eq!(sendfile_at(k, out, input, 8, 100), (Ok(2), 10));
    assert_eq!(sendfile_at(k, out, input, 10, 100), (Ok(0), 10), "at the end");
    assert_eq!(sendfile_at(k, out, input, 1 << 40, 5), (Ok(0), 1 << 40), "far past the end");
    assert_eq!(sendfile_at(k, out, input, 3, 0), (Ok(0), 3), "count 0");
    assert_eq!(sendfile_at(k, out, input, 0, 3), (Ok(3), 3));
    assert_eq!((pos(k, input), pos(k, out)), (1, 7));
    assert_eq!(contents(k, b"/dir/out"), b"ghijabc");
}

#[test]
fn bytes_land_at_the_output_offset_without_truncating() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"0123456789", O_RDONLY);
    let out = file(k, b"/dir/out", b"abcdefghij", O_RDWR);
    assert_eq!(sys(k, nr::LSEEK, &[out, 3, 0]), Ok(3));
    assert_eq!(sendfile_at(k, out, input, 5, 4), (Ok(4), 9));
    assert_eq!(pos(k, out), 7);
    assert_eq!(contents(k, b"/dir/out"), b"abc5678hij");
    assert_eq!(sys(k, nr::LSEEK, &[out, 20, 0]), Ok(20));
    assert_eq!(sendfile_at(k, out, input, 0, 2), (Ok(2), 2));
    assert_eq!(pos(k, out), 22);
    assert_eq!(contents(k, b"/dir/out"), b"abc5678hij\0\0\0\0\0\0\0\0\0\x0001");
    // /dev/null takes everything.
    k.files.insert(NULL as i32, File::new(Kind::NullDevice));
    assert_eq!(sendfile_at(k, NULL, input, 0, 100), (Ok(10), 10));
    assert_eq!(sendfile(k, NULL, input, 0, 4), Ok(4));
    assert_eq!(pos(k, input), 4);
}

#[test]
fn one_descriptor_may_be_input_and_output() {
    let mut e = env();
    let k = &mut e.k;
    let fd = file(k, b"/dir/same", b"abcdefgh", O_RDWR);
    assert_eq!(sys(k, nr::LSEEK, &[fd, 8, 0]), Ok(8));
    assert_eq!(sendfile_at(k, fd, fd, 1, 4), (Ok(4), 5));
    assert_eq!(pos(k, fd), 12);
    assert_eq!(contents(k, b"/dir/same"), b"abcdefghbcde");
    // Without a pointer the bytes land on themselves; the offset moves once.
    assert_eq!(sys(k, nr::LSEEK, &[fd, 2, 0]), Ok(2));
    assert_eq!(sendfile(k, fd, fd, 0, 3), Ok(3));
    assert_eq!(pos(k, fd), 5);
    assert_eq!(contents(k, b"/dir/same"), b"abcdefghbcde");
}

#[test]
fn bad_descriptors_and_access_modes_are_ebadf() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"0123456789", O_RDONLY);
    let out = open(k, b"/dir/out", O_WRONLY | O_CREAT | O_TRUNC);
    k.poke(OFF, &0i64.to_le_bytes());
    assert_eq!(sendfile(k, out, 40, 0, 1), Err(Errno::EBADF));
    assert_eq!(sendfile(k, 41, input, 0, 1), Err(Errno::EBADF));
    assert_eq!(sendfile(k, out, 40, 0, 0), Err(Errno::EBADF), "count 0");
    assert_eq!(sendfile(k, 41, input, OFF, 0), Err(Errno::EBADF));
    assert_eq!(sendfile(k, out, -1i64 as u64, 0, 1), Err(Errno::EBADF));
    assert_eq!(sendfile(k, -1i64 as u64, input, 0, 1), Err(Errno::EBADF));
    let wo = open(k, b"/dir/in", O_WRONLY);
    assert_eq!(sendfile(k, out, wo, 0, 1), Err(Errno::EBADF), "input write-only");
    assert_eq!(sendfile(k, out, wo, OFF, 1), Err(Errno::EBADF));
    assert_eq!(sendfile(k, input, input, 0, 1), Err(Errno::EBADF), "output read-only");
    let pa = open(k, b"/dir/in", O_PATH);
    assert_eq!(sendfile(k, out, pa, 0, 1), Err(Errno::EBADF), "input O_PATH");
    assert_eq!(sendfile(k, pa, input, 0, 1), Err(Errno::EBADF), "output O_PATH");
    let dir = open(k, b"/dir", O_RDONLY | O_DIRECTORY);
    assert_eq!(sendfile(k, dir, input, 0, 1), Err(Errno::EBADF), "output a directory");
    assert_eq!(pos(k, input), 0);
    assert_eq!(k.peek(OFF, 8), [0; 8]);
    assert_eq!(size(k, out), 0);
}

#[test]
fn the_offset_pointer_is_read_first_and_values_are_checked_last() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"0123456789", O_RDONLY);
    let out = open(k, b"/dir/out", O_WRONLY | O_CREAT | O_TRUNC);
    assert_eq!(sendfile(k, out, input, 8, 5), Err(Errno::EFAULT));
    assert_eq!(sendfile(k, out, input, 8, 0), Err(Errno::EFAULT));
    assert_eq!(sendfile(k, 41, 40, 8, 1), Err(Errno::EFAULT), "before the descriptors");
    assert_eq!(sendfile(k, out, input, 8, u64::MAX), Err(Errno::EFAULT));
    k.poke(OFF, &(-1i64).to_le_bytes());
    assert_eq!(sendfile(k, 41, 40, OFF, 1), Err(Errno::EBADF));
    assert_eq!(sendfile(k, out, 40, OFF, 1), Err(Errno::EBADF));
    assert_eq!(sendfile(k, out, 40, 0, u64::MAX), Err(Errno::EBADF));
    let wo = open(k, b"/dir/in", O_WRONLY);
    assert_eq!(sendfile(k, out, wo, 0, u64::MAX), Err(Errno::EBADF));
    assert_eq!(sendfile(k, out, input, OFF, 1), Err(Errno::EINVAL), "negative offset");
    assert_eq!(sendfile(k, out, input, OFF, 0), Err(Errno::EINVAL));
    assert_eq!(sendfile(k, out, input, 0, u64::MAX), Err(Errno::EINVAL), "negative count");
    // *offset + count past 2^63-1, or an offset past what a file allows.
    let at = 0x7fff_ffff_ffff_ff00;
    assert_eq!(sendfile_at(k, out, input, at, 0x100), (Err(Errno::EINVAL), at));
    assert_eq!(sendfile_at(k, out, input, at, 0xff), (Err(Errno::EOVERFLOW), at));
    assert_eq!(pos(k, input), 0);
    assert_eq!(size(k, out), 0);
}

#[test]
fn appending_outputs_and_inputs_without_a_position_are_refused() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"0123456789", O_RDONLY);
    let _ = file(k, b"/dir/out", b"xy", O_RDONLY);
    let ap = open(k, b"/dir/out", O_WRONLY | O_APPEND);
    assert_eq!(sendfile_at(k, ap, input, 0, 3), (Err(Errno::EINVAL), 0));
    assert_eq!(sendfile_at(k, ap, input, 0, 0), (Err(Errno::EINVAL), 0));
    assert_eq!(sendfile(k, ap, input, 0, 3), Err(Errno::EINVAL));
    let rwa = open(k, b"/dir/out", O_RDWR | O_APPEND);
    assert_eq!(sendfile(k, rwa, input, 0, 3), Err(Errno::EINVAL));
    assert_eq!(size(k, ap), 2);
    let out = open(k, b"/dir/out", O_WRONLY | O_TRUNC);
    let dir = open(k, b"/dir", O_RDONLY | O_DIRECTORY);
    assert_eq!(sendfile(k, out, dir, 0, 1), Err(Errno::EINVAL));
    assert_eq!(sendfile_at(k, out, dir, 0, 1), (Err(Errno::EINVAL), 0));
    // A pipe: EINVAL, ESPIPE with a pointer; its data stays.
    let mut pipe = File::new(Kind::Pipe);
    pipe.input = b"pqr".to_vec();
    k.files.insert(PIPE as i32, pipe);
    assert_eq!(sendfile(k, out, PIPE, 0, 3), Err(Errno::EINVAL));
    assert_eq!(sendfile_at(k, out, PIPE, 0, 3), (Err(Errno::ESPIPE), 0));
    k.files.insert(52, File::new(Kind::Pipe));
    assert_eq!(sendfile(k, 52, PIPE, 0, 3), Err(Errno::EINVAL), "pipe to pipe");
    assert_eq!(sendfile_at(k, 52, PIPE, 0, 3), (Err(Errno::ESPIPE), 0));
    assert_eq!(k.files[&(PIPE as i32)].input, b"pqr");
    assert_eq!(size(k, out), 0);
}

#[test]
fn checks_by_kind_of_descriptor() {
    let d = |kind, flags| Desc { kind, flags };
    let file = d(FileKind::Regular, 2);
    let pipe = d(FileKind::Pipe, 1);
    let sock = d(FileKind::Socket, 2);
    let event = d(FileKind::Other, 2);
    assert_eq!(check(file, file, None, 1), Ok(Source::Positioned));
    assert_eq!(check(d(FileKind::Memory, 0), pipe, Some(0), 1), Ok(Source::Positioned), "/dev/zero");
    // A socket feeds a pipe, nothing else.
    assert_eq!(check(sock, pipe, None, 1), Ok(Source::Stream));
    assert_eq!(check(sock, file, None, 1), Err(Errno::EINVAL));
    assert_eq!(check(sock, pipe, Some(0), 1), Err(Errno::ESPIPE));
    assert_eq!(check(sock, file, Some(0), 1), Err(Errno::ESPIPE));
    assert_eq!(check(event, file, None, 8), Err(Errno::EINVAL));
    assert_eq!(check(d(FileKind::Pipe, 0), pipe, None, 3), Err(Errno::EINVAL));
    assert_eq!(check(d(FileKind::Directory, 0), file, Some(0), 1), Err(Errno::EINVAL));
    // Access modes come before kinds; O_PATH is neither readable nor writable.
    assert_eq!(check(d(FileKind::Pipe, 1), file, None, 1), Err(Errno::EBADF));
    assert_eq!(check(file, d(FileKind::Regular, 0o10000000), None, 1), Err(Errno::EBADF));
    assert_eq!(check(file, d(FileKind::Directory, 0), None, 1), Err(Errno::EBADF));
    // Values.
    assert_eq!(check(file, d(FileKind::Regular, 0o2001), None, 0), Err(Errno::EINVAL), "O_APPEND");
    assert_eq!(check(file, file, None, 1 << 63), Err(Errno::EINVAL));
    assert_eq!(check(file, file, Some(-1), 0), Err(Errno::EINVAL));
    assert_eq!(check_range(i64::MAX as u64 - 5, 5), Err(Errno::EOVERFLOW));
    assert_eq!(check_range(LAST_OFFSET, 0xfff), Ok(()));
    assert_eq!(check_range(LAST_OFFSET + 1, 0), Err(Errno::EOVERFLOW));
    assert_eq!(check_range(1 << 40, 5), Ok(()));
    assert_eq!(check_range(0, i64::MAX as u64), Ok(()));
    assert_eq!(check_range(1, i64::MAX as u64), Err(Errno::EINVAL));
}

#[test]
fn pipes_take_whole_free_pages() {
    assert_eq!(pipe_share(300_000, 65536, 65536, true), Ok(65536));
    assert_eq!(pipe_share(100, 4096, 65536, true), Ok(100));
    assert_eq!(pipe_share(300_000, 0, 65536, true), Err(Errno::EAGAIN));
    assert_eq!(pipe_share(0, 0, 65536, true), Err(Errno::EAGAIN), "even count 0");
    assert_eq!(pipe_share(0, 4096, 65536, true), Ok(0));
    // Blocking: a pipe's worth, waiting for room.
    assert_eq!(pipe_share(300_000, 0, 65536, false), Ok(65536));
    assert_eq!(pipe_share(300_000, 8192, 65536, false), Ok(8192));
}

#[test]
fn a_pipe_gets_what_fits_and_the_offset_moves_by_that() {
    let mut e = env();
    let k = &mut e.k;
    let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
    let input = file(k, b"/dir/in", &data[..0x1000], O_RDWR);
    for (i, part) in data.chunks(0x1000).enumerate().skip(1) {
        k.poke(scratch(0x2000), part);
        let at = (i * 0x1000) as u64;
        assert_eq!(sys(k, nr::PWRITE64, &[input, scratch(0x2000), part.len() as u64, at]), Ok(part.len() as u64));
    }
    k.files.insert(PIPE as i32, File::new(Kind::Pipe));
    k.io.pipe_room.insert(PIPE as i32, 8192);
    assert_eq!(sendfile_at(k, PIPE, input, 0, 20_000), (Ok(8192), 8192));
    assert_eq!(sendfile(k, PIPE, input, 0, 20_000), Ok(8192), "the file offset too");
    assert_eq!(pos(k, input), 8192);
    // A pipe that takes less than offered: the offset moves by what went in.
    k.io.pipe_room.insert(PIPE as i32, 65536);
    k.files.get_mut(&(PIPE as i32)).unwrap().max_write = Some(1000);
    assert_eq!(sendfile(k, PIPE, input, 0, 20_000), Ok(1000));
    assert_eq!(pos(k, input), 9192);
    assert_eq!(k.files[&(PIPE as i32)].written, [&data[..8192], &data[..9192]].concat());
}

#[test]
fn epipe_raises_sigpipe_and_moves_nothing() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"0123456789", O_RDONLY);
    let mut pipe = File::new(Kind::Pipe);
    pipe.fail_after = Some((0, Errno::EPIPE));
    k.files.insert(PIPE as i32, pipe);
    assert_eq!(sendfile_at(k, PIPE, input, 2, 5), (Err(Errno::EPIPE), 2));
    let tid = k.tid;
    assert!(k.me().pending(tid).contains(SIGPIPE));
    assert_eq!(sendfile(k, PIPE, input, 0, 5), Err(Errno::EPIPE));
    assert_eq!(pos(k, input), 0);
    // An error after some bytes went out: the count.
    let mut pipe = File::new(Kind::Pipe);
    pipe.max_write = Some(3);
    pipe.fail_after = Some((3, Errno::EPIPE));
    k.files.insert(PIPE as i32, pipe);
    assert_eq!(sendfile(k, PIPE, input, 0, 10), Ok(3));
}

#[test]
fn the_file_size_limit_cuts_the_transfer_then_refuses_it() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"0123456789", O_RDONLY);
    let out = open(k, b"/dir/out", O_WRONLY | O_CREAT | O_TRUNC);
    k.poke(scratch(0x300), &Rlimit { cur: 6, max: 6 }.to_bytes());
    assert_eq!(sys(k, nr::SETRLIMIT, &[u64::from(RLIMIT_FSIZE), scratch(0x300)]), Ok(0));
    assert_eq!(sendfile_at(k, out, input, 0, 10), (Ok(6), 6));
    assert!(!k.me().pending(100).contains(crate::sys::fsize::SIGXFSZ));
    assert_eq!(sendfile_at(k, out, input, 6, 10), (Err(Errno::EFBIG), 6));
    assert!(k.me().pending(100).contains(crate::sys::fsize::SIGXFSZ), "SIGXFSZ raised");
    assert_eq!(sendfile_at(k, out, input, 6, 0), (Ok(0), 6), "count 0 at the limit");
    assert_eq!(size(k, out), 6);
}

#[test]
fn an_offset_that_cannot_be_written_back_is_efault_after_the_copy() {
    let mut e = env();
    let k = &mut e.k;
    let input = file(k, b"/dir/in", b"0123456789", O_RDONLY);
    let out = open(k, b"/dir/out", O_WRONLY | O_CREAT | O_TRUNC);
    let ro = read_only_page(k);
    k.poke(ro, &2i64.to_le_bytes());
    assert_eq!(sendfile(k, out, input, ro, 3), Err(Errno::EFAULT));
    assert_eq!(k.peek(ro, 8), 2i64.to_le_bytes());
    assert_eq!((pos(k, out), size(k, out)), (3, 3), "the bytes moved");
    assert_eq!(contents(k, b"/dir/out"), b"234");
}
