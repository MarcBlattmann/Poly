//! memfd_create(2) and fcntl's file seals, against a
//! [`LinuxFs`](crate::fs::LinuxFs) over a real `poly-vfs` (the fixture of
//! `files.rs`).

extern crate std;

use super::fake::*;
use super::files::{env, s};
use crate::fs::fcntl::{F_ADD_SEALS, F_GET_SEALS, F_GETFD, F_GETFL};
use crate::sys::memfd::{MFD_ALLOW_SEALING, MFD_CLOEXEC, MFD_EXEC, MFD_NOEXEC_SEAL};
use crate::{Errno, nr};

const OUT: u64 = 0x8000;
const F_SEAL_SEAL: u64 = 1;
const F_SEAL_SHRINK: u64 = 2;
const F_SEAL_GROW: u64 = 4;
const F_SEAL_WRITE: u64 = 8;
const F_SEAL_EXEC: u64 = 0x20;

fn mode(k: &mut FakeKernel, fd: u64) -> u32 {
    assert_eq!(sys(k, nr::FSTAT, &[fd, scratch(OUT)]), Ok(0));
    k.peek_u32(scratch(OUT) + 24)
}

#[test]
fn memfd_create_makes_an_unlinked_file_on_the_lowest_descriptor() {
    let mut e = env();
    let k = &mut e.k;
    let name = s(k, 0, b"basic");
    assert_eq!(sys(k, nr::UMASK, &[0o077]), Ok(0o022));
    assert_eq!(sys(k, nr::MEMFD_CREATE, &[name, 0]), Ok(3));
    assert_eq!(sys(k, nr::FCNTL, &[3, u64::from(F_GETFL)]), Ok(0o100002), "O_RDWR | O_LARGEFILE");
    assert_eq!(sys(k, nr::FCNTL, &[3, u64::from(F_GETFD)]), Ok(0));
    assert_eq!(mode(k, 3), 0o100777, "the umask does not apply");
    assert_eq!(k.peek_u32(scratch(OUT) + 16), 0, "st_nlink");
    // Bits 32..63 of flags are ignored.
    assert_eq!(sys(k, nr::MEMFD_CREATE, &[name, 1 << 32 | u64::from(MFD_CLOEXEC)]), Ok(4));
    assert_eq!(sys(k, nr::FCNTL, &[4, u64::from(F_GETFD)]), Ok(1));
    assert_eq!(sys(k, nr::MEMFD_CREATE, &[name, u64::from(MFD_NOEXEC_SEAL)]), Ok(5));
    assert_eq!(mode(k, 5), 0o100666);
    // Read, write, truncate like any file.
    k.poke(scratch(0x100), b"hello");
    assert_eq!(sys(k, nr::PWRITE64, &[3, scratch(0x100), 5, 0]), Ok(5));
    assert_eq!(sys(k, nr::FTRUNCATE, &[3, 10000]), Ok(0));
    assert_eq!(sys(k, nr::PREAD64, &[3, scratch(OUT), 8, 0]), Ok(8));
    assert_eq!(k.peek(scratch(OUT), 8), b"hello\0\0\0");
}

#[test]
fn memfd_create_checks_flags_then_name() {
    let mut e = env();
    let k = &mut e.k;
    for bad in [0x20, 0x40, 0x8000_0000, 21 << 26, u64::from(MFD_EXEC | MFD_NOEXEC_SEAL)] {
        assert_eq!(sys(k, nr::MEMFD_CREATE, &[0, bad]), Err(Errno::EINVAL), "{bad:#x} before the name");
    }
    assert_eq!(sys(k, nr::MEMFD_CREATE, &[0, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::MEMFD_CREATE, &[UNMAPPED, 0]), Err(Errno::EFAULT));
    let longest = s(k, 0, &[b'n'; 249]);
    assert_eq!(sys(k, nr::MEMFD_CREATE, &[longest, 0]), Ok(3));
    let too_long = s(k, 0, &[b'n'; 250]);
    assert_eq!(sys(k, nr::MEMFD_CREATE, &[too_long, 0]), Err(Errno::EINVAL));
    let empty = s(k, 0, b"");
    assert_eq!(sys(k, nr::MEMFD_CREATE, &[empty, 0]), Ok(4));
}

#[test]
fn seals_start_sealed_unless_sealing_is_allowed() {
    let mut e = env();
    let k = &mut e.k;
    let name = s(k, 0, b"seals");
    let fd = sys(k, nr::MEMFD_CREATE, &[name, 0]).unwrap();
    assert_eq!(sys(k, nr::FCNTL, &[fd, u64::from(F_GET_SEALS)]), Ok(F_SEAL_SEAL));
    assert_eq!(sys(k, nr::FCNTL, &[fd, u64::from(F_ADD_SEALS), F_SEAL_WRITE]), Err(Errno::EPERM));

    let fd = sys(k, nr::MEMFD_CREATE, &[name, u64::from(MFD_ALLOW_SEALING)]).unwrap();
    assert_eq!(sys(k, nr::FCNTL, &[fd, u64::from(F_GET_SEALS)]), Ok(0));
    k.poke(scratch(0x100), b"abcdef");
    assert_eq!(sys(k, nr::WRITE, &[fd, scratch(0x100), 6]), Ok(6));
    assert_eq!(sys(k, nr::FCNTL, &[fd, u64::from(F_ADD_SEALS), F_SEAL_SHRINK | F_SEAL_GROW]), Ok(0));
    assert_eq!(sys(k, nr::FTRUNCATE, &[fd, 3]), Err(Errno::EPERM));
    assert_eq!(sys(k, nr::FTRUNCATE, &[fd, 7]), Err(Errno::EPERM));
    assert_eq!(sys(k, nr::FTRUNCATE, &[fd, 6]), Ok(0));
    assert_eq!(sys(k, nr::PWRITE64, &[fd, scratch(0x100), 2, 5]), Err(Errno::EPERM));
    assert_eq!(sys(k, nr::FCNTL, &[fd, u64::from(F_ADD_SEALS), F_SEAL_WRITE]), Ok(0));
    assert_eq!(sys(k, nr::PWRITE64, &[fd, scratch(0x100), 1, 0]), Err(Errno::EPERM));
    assert_eq!(sys(k, nr::FCNTL, &[fd, u64::from(F_ADD_SEALS), 0x40]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FCNTL, &[fd, u64::from(F_GET_SEALS)]), Ok(F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE));

    // Not a memfd.
    let path = s(k, 0, b"/dir/file");
    let file = sys(k, nr::OPEN, &[path, 2, 0]).unwrap();
    assert_eq!(sys(k, nr::FCNTL, &[file, u64::from(F_GET_SEALS)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FCNTL, &[file, u64::from(F_ADD_SEALS), F_SEAL_WRITE]), Err(Errno::EINVAL));
}

#[test]
fn the_exec_seal_keeps_the_execute_bits() {
    let mut e = env();
    let k = &mut e.k;
    let name = s(k, 0, b"exec");
    let fd = sys(k, nr::MEMFD_CREATE, &[name, u64::from(MFD_NOEXEC_SEAL)]).unwrap();
    assert_eq!(sys(k, nr::FCNTL, &[fd, u64::from(F_GET_SEALS)]), Ok(F_SEAL_EXEC));
    assert_eq!(sys(k, nr::FCHMOD, &[fd, 0o766]), Err(Errno::EPERM));
    assert_eq!(sys(k, nr::FCHMOD, &[fd, 0o600]), Ok(0));
    assert_eq!(mode(k, fd), 0o100600);
}
