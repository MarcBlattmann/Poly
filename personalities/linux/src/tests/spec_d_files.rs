//! spec-linux-d's file calls (ADR 0121): openat2(2) with its `resolve`
//! restrictions, fchmodat2(2), futimesat(2) with a null path, against the
//! tmpfs of [`super::files::env`].

extern crate std;

use super::fake::*;
use super::files::{AT, env, s};
use crate::sys::path::check_open_how;
use crate::{Errno, nr};
use poly_vfs::{At, Cred};

/// `struct open_how` at `offset` in the data area, padded to `size`.
fn how(k: &mut FakeKernel, offset: u64, flags: u64, mode: u64, resolve: u64, size: usize) -> u64 {
    let mut b = std::vec![0u8; size.max(24)];
    b[0..8].copy_from_slice(&flags.to_le_bytes());
    b[8..16].copy_from_slice(&mode.to_le_bytes());
    b[16..24].copy_from_slice(&resolve.to_le_bytes());
    k.poke(scratch(offset), &b);
    scratch(offset)
}

const O_RDONLY: u64 = 0;
const O_WRONLY: u64 = 1;
const O_CREAT: u64 = 0o100;
const O_TRUNC: u64 = 0o1000;
const O_DIRECTORY: u64 = 0o200000;
const O_NOFOLLOW: u64 = 0o400000;
const O_PATH: u64 = 0o10000000;
const O_TMPFILE: u64 = 0o20200000;
const NO_XDEV: u64 = 1;
const NO_MAGICLINKS: u64 = 2;
const NO_SYMLINKS: u64 = 4;
const BENEATH: u64 = 8;
const IN_ROOT: u64 = 0x10;
const CACHED: u64 = 0x20;

#[test]
fn open_how_checks() {
    assert_eq!(check_open_how(O_RDONLY, 0, 0), Ok(()));
    assert_eq!(check_open_how(3, 0, NO_XDEV | NO_MAGICLINKS | NO_SYMLINKS | BENEATH), Ok(()));
    for bad in [1 << 23, 1 << 31, 1 << 32, 1 << 63] {
        assert_eq!(check_open_how(bad, 0, 0), Err(Errno::EINVAL), "{bad:#x}");
    }
    assert_eq!(check_open_how(O_RDONLY, 0o644, 0), Err(Errno::EINVAL), "mode without O_CREAT");
    assert_eq!(check_open_how(O_CREAT | O_WRONLY, 0o10000, 0), Err(Errno::EINVAL));
    assert_eq!(check_open_how(O_CREAT | O_WRONLY, 0o7777, 0), Ok(()));
    assert_eq!(check_open_how(O_PATH | 2, 0, 0), Err(Errno::EINVAL));
    assert_eq!(check_open_how(O_PATH | 0o100000, 0, 0), Err(Errno::EINVAL), "O_PATH | O_LARGEFILE");
    assert_eq!(check_open_how(O_PATH | O_NOFOLLOW | 0o2000000 | O_DIRECTORY, 0, 0), Ok(()));
    assert_eq!(check_open_how(O_TMPFILE, 0o600, 0), Err(Errno::EINVAL), "O_TMPFILE read-only");
    assert_eq!(check_open_how(O_DIRECTORY | O_CREAT, 0o600, 0), Err(Errno::EINVAL));
    assert_eq!(check_open_how(O_RDONLY, 0, 0x40), Err(Errno::EINVAL));
    assert_eq!(check_open_how(O_RDONLY, 0, BENEATH | IN_ROOT), Err(Errno::EINVAL));
    assert_eq!(check_open_how(O_CREAT, 0, CACHED), Err(Errno::EAGAIN));
    assert_eq!(check_open_how(O_WRONLY | O_TRUNC, 0, CACHED), Err(Errno::EAGAIN));
    assert_eq!(check_open_how(O_TMPFILE | 2, 0, CACHED), Err(Errno::EAGAIN));
    assert_eq!(check_open_how(O_RDONLY, 0, CACHED), Ok(()));
}

#[test]
fn openat2_size_rules() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    let h = how(k, 0x100, O_RDONLY, 0, 0, 64);
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 23]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 4097]), Err(Errno::E2BIG));
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, 0, 24]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::OPENAT2, &[AT, 0, h, 8]), Err(Errno::EINVAL), "size before the path");
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 64]), Ok(3), "zero bytes past the struct");
    k.poke(scratch(0x100 + 40), &[1]);
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 64]), Err(Errno::E2BIG));
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 40]), Ok(4), "a byte beyond size is not looked at");
    assert_eq!(sys(k, nr::OPENAT2, &[99, 0, h, 24]), Err(Errno::EFAULT), "path before dirfd");
}

#[test]
fn openat2_in_root_and_beneath() {
    let mut e = env();
    let k = &mut e.k;
    let d = s(k, 0, b"/dir");
    let dirfd = sys(k, nr::OPEN, &[d, O_DIRECTORY, 0]).unwrap();
    let h = how(k, 0x100, O_RDONLY, 0, IN_ROOT, 24);
    // "/file" and "../file" are dirfd's file; "/dir" is not there.
    let p = s(k, 0, b"/file");
    assert_eq!(sys(k, nr::OPENAT2, &[dirfd, p, h, 24]), Ok(dirfd + 1));
    let p = s(k, 0, b"../../file");
    assert_eq!(sys(k, nr::OPENAT2, &[dirfd, p, h, 24]), Ok(dirfd + 2));
    let p = s(k, 0, b"/dir/file");
    assert_eq!(sys(k, nr::OPENAT2, &[dirfd, p, h, 24]), Err(Errno::ENOENT));
    // Beneath: '..' that stays inside works, leaving is EXDEV, absolute
    // paths too; a missing name before the escape is ENOENT.
    let h = how(k, 0x100, O_RDONLY, 0, BENEATH, 24);
    let p = s(k, 0, b"sub/../file");
    assert_eq!(sys(k, nr::OPENAT2, &[dirfd, p, h, 24]), Ok(dirfd + 3));
    let p = s(k, 0, b"../dir/file");
    assert_eq!(sys(k, nr::OPENAT2, &[dirfd, p, h, 24]), Err(Errno::EXDEV));
    let p = s(k, 0, b"/dir/file");
    assert_eq!(sys(k, nr::OPENAT2, &[dirfd, p, h, 24]), Err(Errno::EXDEV));
    let p = s(k, 0, b"nope/../..");
    assert_eq!(sys(k, nr::OPENAT2, &[dirfd, p, h, 24]), Err(Errno::ENOENT));
    // /link -> dir/file is relative and stays below /.
    let p = s(k, 0, b"link");
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 24]), Ok(dirfd + 4), "below the working directory, /");
    let p = s(k, 0, b"/link");
    let root = s(k, 0x200, b"/");
    let rootfd = sys(k, nr::OPEN, &[root, O_DIRECTORY, 0]).unwrap();
    let rel = s(k, 0x300, b"link");
    assert_eq!(sys(k, nr::OPENAT2, &[rootfd, rel, h, 24]), Ok(rootfd + 1));
    assert_eq!(sys(k, nr::OPENAT2, &[rootfd, p, h, 24]), Err(Errno::EXDEV));
}

#[test]
fn openat2_refuses_symlinks() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/link");
    let h = how(k, 0x100, O_RDONLY, 0, NO_SYMLINKS, 24);
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 24]), Err(Errno::ELOOP));
    let h = how(k, 0x100, O_RDONLY | O_NOFOLLOW, 0, NO_SYMLINKS, 24);
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 24]), Err(Errno::ELOOP));
    let h = how(k, 0x100, O_PATH | O_NOFOLLOW, 0, NO_SYMLINKS, 24);
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 24]), Ok(3), "the link itself");
    // Ordinary links are no magic links; one file system, no crossing.
    let h = how(k, 0x100, O_RDONLY, 0, NO_MAGICLINKS | NO_XDEV, 24);
    assert_eq!(sys(k, nr::OPENAT2, &[AT, p, h, 24]), Ok(4));
}

#[test]
fn fchmodat2_flags_and_symlinks() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/link");
    assert_eq!(sys(k, nr::FCHMODAT2, &[AT, 0, 0o600, 0x200]), Err(Errno::EINVAL), "flags first");
    assert_eq!(sys(k, nr::FCHMODAT2, &[AT, p, 0o600, 0x100]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(k, nr::FCHMODAT2, &[AT, p, 0o640, 0]), Ok(0), "followed");
    let ctx = e.vfs.root_context().unwrap();
    let st = e.vfs.stat(&ctx, &Cred::root(), At::Cwd, b"/dir/file", true).unwrap();
    assert_eq!(st.mode, 0o100640);
    let st = e.vfs.stat(&ctx, &Cred::root(), At::Cwd, b"/link", false).unwrap();
    assert_eq!(st.mode & 0o777, 0o777, "the link unchanged");
    // AT_EMPTY_PATH: the descriptor's file, also an O_PATH one; NULL is
    // still EFAULT.
    let f = s(k, 0, b"/dir/file");
    let fd = sys(k, nr::OPEN, &[f, O_PATH, 0]).unwrap();
    let empty = s(k, 0x100, b"");
    assert_eq!(sys(k, nr::FCHMODAT2, &[fd, empty, 0o604, 0x1000]), Ok(0));
    let st = e.vfs.stat(&ctx, &Cred::root(), At::Cwd, b"/dir/file", true).unwrap();
    assert_eq!(st.mode, 0o100604);
    assert_eq!(sys(&mut e.k, nr::FCHMODAT2, &[fd, 0, 0o604, 0x1000]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut e.k, nr::FCHMODAT2, &[AT, empty, 0o604, 0x100]), Err(Errno::ENOENT));
}

#[test]
fn futimesat_null_path() {
    let mut e = env();
    let k = &mut e.k;
    assert_eq!(sys(k, nr::FUTIMESAT, &[AT, 0, 0]), Err(Errno::EFAULT), "AT_FDCWD with a null path");
    let f = s(k, 0, b"/dir/file");
    let fd = sys(k, nr::OPEN, &[f, O_RDONLY, 0]).unwrap();
    let mut times = [0u8; 32];
    times[0..8].copy_from_slice(&5i64.to_le_bytes());
    times[16..24].copy_from_slice(&7i64.to_le_bytes());
    times[24..32].copy_from_slice(&3i64.to_le_bytes());
    k.poke(scratch(0x100), &times);
    assert_eq!(sys(k, nr::FUTIMESAT, &[fd, 0, scratch(0x100)]), Ok(0));
    let st = e.vfs.stat(&e.vfs.root_context().unwrap(), &Cred::root(), At::Cwd, b"/dir/file", true).unwrap();
    assert_eq!((st.atime.sec, st.mtime.sec, st.mtime.nsec), (5, 7, 3000));
    // The times are checked before the path: a bad tv_usec is EINVAL.
    times[8..16].copy_from_slice(&1_000_000i64.to_le_bytes());
    e.k.poke(scratch(0x100), &times);
    assert_eq!(sys(&mut e.k, nr::FUTIMESAT, &[AT, 0, scratch(0x100)]), Err(Errno::EINVAL));
}
