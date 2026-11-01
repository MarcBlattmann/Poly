//! The calls bash, coreutils, Python and gcc needed beyond the earlier
//! milestones (ADR 0059), against [`FakeKernel`].

extern crate std;

use super::fake::*;
use crate::{Errno, nr};

// ---- read ---------------------------------------------------------------

#[test]
fn reads_of_files_that_never_wait_are_whole() {
    let mut k = FakeKernel::new();
    let len = 3 * crate::sys::io::CHUNK + 100;
    // A user buffer that large: the fake's data area is 64 KiB, so map more.
    let buf = 0x5000_0000u64;
    k.map_range(buf, len as u64, crate::abi::prot::PROT_READ | crate::abi::prot::PROT_WRITE);
    for (fd, kind, want) in
        [(5, Kind::Regular, len), (6, Kind::NullDevice, len), (7, Kind::Pipe, crate::sys::io::CHUNK)]
    {
        let mut f = File::new(kind);
        f.input = (0..len).map(|i| i as u8).collect();
        k.files.insert(fd, f);
        assert_eq!(sys(&mut k, nr::READ, &[fd as u64, buf, len as u64]), Ok(want as u64), "{kind:?}");
        assert_eq!(k.peek(buf + want as u64 - 1, 1), [(want - 1) as u8]);
    }
}

// ---- record locks and flock ---------------------------------------------

/// A `struct flock` at `at`.
fn put_flock(k: &mut FakeKernel, at: u64, kind: i16, whence: i16, start: i64, len: i64, pid: i32) {
    let mut b = [0u8; 32];
    b[0..2].copy_from_slice(&kind.to_le_bytes());
    b[2..4].copy_from_slice(&whence.to_le_bytes());
    b[8..16].copy_from_slice(&start.to_le_bytes());
    b[16..24].copy_from_slice(&len.to_le_bytes());
    b[24..28].copy_from_slice(&pid.to_le_bytes());
    k.poke(at, &b);
}

/// `(l_type, l_whence, l_start, l_len, l_pid)` of the `struct flock` at `at`.
fn get_flock(k: &FakeKernel, at: u64) -> (i16, i16, i64, i64, i32) {
    let b = k.peek(at, 32);
    let i64_at = |o: usize| i64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    (
        i16::from_le_bytes([b[0], b[1]]),
        i16::from_le_bytes([b[2], b[3]]),
        i64_at(8),
        i64_at(16),
        i32::from_le_bytes(b[24..28].try_into().unwrap()),
    )
}

#[test]
fn fcntl_record_locks_decode_struct_flock() {
    let mut e = super::files::env();
    let k = &mut e.k;
    let p = super::files::s(k, 0, b"/dir/file");
    let rw = sys(k, nr::OPEN, &[p, 2, 0]).unwrap();
    let ro = sys(k, nr::OPEN, &[p, 0, 0]).unwrap();
    let fl = scratch(0x200);
    // A write lock from the end of the 8-byte file, start -2, length -2:
    // bytes 4 and 5 ("l_start+l_len up to and including l_start-1").
    put_flock(k, fl, 1, 2, -2, -2, 0);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 6, fl]), Ok(0));
    // The same process sees no conflict with itself.
    put_flock(k, fl, 1, 0, 0, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 5, fl]), Ok(0));
    assert_eq!(get_flock(k, fl).0, 2, "F_UNLCK: nothing in the way");
    // As an open file description, the process's lock is in the way.
    put_flock(k, fl, 0, 1, 0, 10, 0);
    assert_eq!(sys(k, nr::FCNTL, &[ro, 36, fl]), Ok(0));
    assert_eq!(get_flock(k, fl), (1, 0, 4, 2, 1));
    put_flock(k, fl, 0, 0, 0, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[ro, 37, fl]), Err(Errno::EAGAIN), "F_OFD_SETLK meets the lock");
    put_flock(k, fl, 0, 0, 0, 4, 0);
    assert_eq!(sys(k, nr::FCNTL, &[ro, 37, fl]), Ok(0), "below it");
    // Unlock, then the description's read lock is all that is left.
    put_flock(k, fl, 2, 0, 0, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 6, fl]), Ok(0));
    put_flock(k, fl, 1, 0, 0, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 5, fl]), Ok(0));
    assert_eq!(get_flock(k, fl), (0, 0, 0, 4, -1));
    // Errors.
    put_flock(k, fl, 1, 0, 0, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[ro, 6, fl]), Err(Errno::EBADF), "a write lock on a read-only descriptor");
    put_flock(k, fl, 3, 0, 0, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 6, fl]), Err(Errno::EINVAL), "l_type");
    put_flock(k, fl, 0, 3, 0, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 6, fl]), Err(Errno::EINVAL), "l_whence");
    put_flock(k, fl, 0, 0, -1, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 6, fl]), Err(Errno::EINVAL), "before the start of the file");
    put_flock(k, fl, 0, 0, 0, 0, 5);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 37, fl]), Err(Errno::EINVAL), "F_OFD_SETLK with an l_pid");
    put_flock(k, fl, 2, 0, 0, 0, 0);
    assert_eq!(sys(k, nr::FCNTL, &[rw, 5, fl]), Err(Errno::EINVAL), "F_GETLK of F_UNLCK");
}

#[test]
fn flock_takes_whole_file_locks_per_description() {
    let mut e = super::files::env();
    let k = &mut e.k;
    let p = super::files::s(k, 0, b"/dir/file");
    let (a, b) = (sys(k, nr::OPEN, &[p, 0, 0]).unwrap(), sys(k, nr::OPEN, &[p, 0, 0]).unwrap());
    assert_eq!(sys(k, nr::FLOCK, &[a, 2 | 4]), Ok(0), "LOCK_EX | LOCK_NB");
    assert_eq!(sys(k, nr::FLOCK, &[b, 1 | 4]), Err(Errno::EAGAIN), "another description");
    assert_eq!(sys(k, nr::FLOCK, &[a, 8]), Ok(0), "LOCK_UN");
    assert_eq!(sys(k, nr::FLOCK, &[b, 1]), Ok(0));
    assert_eq!(sys(k, nr::FLOCK, &[a, 1 | 4]), Ok(0), "two shared locks");
    assert_eq!(sys(k, nr::FLOCK, &[a, 3]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FLOCK, &[a, 4]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FLOCK, &[99, 1]), Err(Errno::EBADF));
}

// ---- extended attributes ------------------------------------------------

#[test]
fn xattrs_on_files_that_exist() {
    // tmpfs keeps attributes: none at first (ENODATA, an empty list), as
    // ls(1) expects; set, read, removed (tests/fileops.rs has more).
    let mut e = super::files::env();
    let k = &mut e.k;
    let file = super::files::s(k, 0, b"/dir/file");
    let link = super::files::s(k, 0x40, b"/link");
    let missing = super::files::s(k, 0x80, b"/nowhere");
    let name = super::files::s(k, 0xc0, b"user.comment");
    let empty = super::files::s(k, 0x100, b"");
    let long = super::files::s(k, 0x200, &[b'a'; 300]);
    let out = scratch(0x400);
    for nr in [nr::GETXATTR, nr::LGETXATTR] {
        assert_eq!(sys(k, nr, &[file, name, out, 64]), Err(Errno::ENODATA));
        assert_eq!(sys(k, nr, &[link, name, out, 64]), Err(Errno::ENODATA));
        assert_eq!(sys(k, nr, &[missing, name, out, 64]), Err(Errno::ENOENT));
        assert_eq!(sys(k, nr, &[file, empty, out, 64]), Err(Errno::ERANGE));
        assert_eq!(sys(k, nr, &[file, long, out, 64]), Err(Errno::ERANGE));
    }
    for nr in [nr::LISTXATTR, nr::LLISTXATTR] {
        assert_eq!(sys(k, nr, &[file, out, 64]), Ok(0));
        assert_eq!(sys(k, nr, &[missing, out, 64]), Err(Errno::ENOENT));
    }
    assert_eq!(sys(k, nr::SETXATTR, &[file, name, out, 4, 0]), Ok(0));
    assert_eq!(sys(k, nr::SETXATTR, &[file, name, out, 4, 3]), Err(Errno::EINVAL), "both flags");
    assert_eq!(sys(k, nr::SETXATTR, &[file, name, out, 4, 8]), Err(Errno::EINVAL), "an unknown flag");
    assert_eq!(sys(k, nr::REMOVEXATTR, &[file, name]), Ok(0));
    let fd = sys(k, nr::OPEN, &[file, 0, 0]).unwrap();
    assert_eq!(sys(k, nr::FGETXATTR, &[fd, name, out, 64]), Err(Errno::ENODATA));
    assert_eq!(sys(k, nr::FLISTXATTR, &[fd, out, 64]), Ok(0));
    assert_eq!(sys(k, nr::FSETXATTR, &[fd, name, out, 4, 1]), Ok(0));
    assert_eq!(sys(k, nr::FREMOVEXATTR, &[fd, name]), Ok(0));
    assert_eq!(sys(k, nr::FGETXATTR, &[99, name, out, 64]), Err(Errno::EBADF));
}

// ---- mremap -------------------------------------------------------------

#[test]
fn mremap_checks_its_flags_and_rounds_the_sizes() {
    use crate::kernel::Remap;
    let mut k = FakeKernel::new();
    let a = 0x7000_0000u64;
    assert_eq!(sys(&mut k, nr::MREMAP, &[a, 4096, 8192, 0, 0]), Ok(a));
    assert_eq!(sys(&mut k, nr::MREMAP, &[a, 100, 5000, 1, 0]), Ok(a));
    assert_eq!(sys(&mut k, nr::MREMAP, &[a, 4096, 4096, 3, 0x6000_0000]), Ok(0x6000_0000));
    assert_eq!(
        k.remapped,
        [(a, 4096, 8192, Remap::InPlace), (a, 4096, 8192, Remap::MayMove), (a, 4096, 4096, Remap::Fixed(0x6000_0000))]
    );
    let bad: [(&[u64], &str); 7] = [
        (&[a, 4096, 4096, 8, 0], "an unknown flag"),
        (&[a, 4096, 4096, 2, 0x6000_0000], "MREMAP_FIXED without MREMAP_MAYMOVE"),
        (&[a, 4096, 4096, 4, 0], "MREMAP_DONTUNMAP without MREMAP_MAYMOVE"),
        (&[a, 4096, 4096, 5, 0], "MREMAP_DONTUNMAP"),
        (&[a + 1, 4096, 4096, 1, 0], "a misaligned address"),
        (&[a, 4096, 0, 1, 0], "a zero new size"),
        (&[a, 0, 4096, 0, 0], "a zero old size without MREMAP_MAYMOVE"),
    ];
    for (args, what) in bad {
        assert_eq!(sys(&mut k, nr::MREMAP, args), Err(Errno::EINVAL), "{what}");
    }
    assert_eq!(k.remapped.len(), 3, "refused calls never reach the kernel");
    // A zero old size asks for a second mapping of a shared mapping's pages:
    // the kernel's to do (or refuse for a private one).
    assert_eq!(sys(&mut k, nr::MREMAP, &[a, 0, 4096, 1, 0]), Ok(a));
    assert_eq!(k.remapped.last(), Some(&(a, 0, 4096, Remap::MayMove)));
}

// ---- msync --------------------------------------------------------------

#[test]
fn msync_checks_its_flags_and_the_range() {
    let mut k = FakeKernel::new();
    let a = scratch(0);
    assert_eq!(sys(&mut k, nr::MSYNC, &[a, 100, 4]), Ok(0), "MS_SYNC");
    assert_eq!(sys(&mut k, nr::MSYNC, &[a, 4096, 1 | 2]), Ok(0), "MS_ASYNC | MS_INVALIDATE");
    assert_eq!(sys(&mut k, nr::MSYNC, &[a, 0, 4]), Ok(0), "nothing to do");
    assert_eq!(k.synced_maps, [(a, 4096, true), (a, 4096, false)]);
    assert_eq!(sys(&mut k, nr::MSYNC, &[a + 1, 4096, 4]), Err(Errno::EINVAL), "misaligned");
    assert_eq!(sys(&mut k, nr::MSYNC, &[a, 4096, 5]), Err(Errno::EINVAL), "MS_SYNC | MS_ASYNC");
    assert_eq!(sys(&mut k, nr::MSYNC, &[a, 4096, 8]), Err(Errno::EINVAL), "an unknown flag");
    assert_eq!(sys(&mut k, nr::MSYNC, &[UNMAPPED, 4096, 4]), Err(Errno::ENOMEM));
}

// ---- copy_file_range ----------------------------------------------------

/// The text of `path` in `e`'s file system.
fn read_back(e: &super::files::Env, path: &[u8]) -> std::vec::Vec<u8> {
    let ctx = e.vfs.root_context().unwrap();
    let f = e.vfs.open(&ctx, &poly_vfs::Cred::root(), poly_vfs::At::Cwd, path, poly_vfs::OpenFlags::RDONLY, 0).unwrap();
    let mut buf = std::vec![0u8; 256];
    let n = f.read(&mut buf).unwrap();
    buf.truncate(n);
    buf
}

#[test]
fn copy_file_range_copies_at_file_offsets_and_at_pointers() {
    let mut e = super::files::env();
    let k = &mut e.k;
    let src = super::files::s(k, 0, b"/dir/file");
    let dst = super::files::s(k, 0x100, b"/dir/copy");
    let fd_in = sys(k, nr::OPEN, &[src, 0, 0]).unwrap();
    let fd_out = sys(k, nr::OPEN, &[dst, 0o101, 0o644]).unwrap();
    // Null pointers: the file offsets, which move.
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[fd_in, 0, fd_out, 0, 100, 0]), Ok(8));
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[fd_in, 0, fd_out, 0, 100, 0]), Ok(0), "at the end of the input");
    assert_eq!(read_back(&e, b"/dir/copy"), b"contents");
    let k = &mut e.k;
    // Pointers: they move, the file offsets do not.
    let (off_in, off_out) = (scratch(0x200), scratch(0x208));
    k.poke(off_in, &2u64.to_le_bytes());
    k.poke(off_out, &1u64.to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[fd_in, off_in, fd_out, off_out, 3, 0]), Ok(3));
    assert_eq!((k.peek_u64(off_in), k.peek_u64(off_out)), (5, 4));
    assert_eq!(sys(k, nr::LSEEK, &[fd_in, 0, 1]), Ok(8), "the input's offset stayed");
    assert_eq!(read_back(&e, b"/dir/copy"), b"cnteents", "\"nte\" over bytes 1-3");
    let k = &mut e.k;
    // Overlapping ranges of one file; flags; directions; directories.
    let both = sys(k, nr::OPEN, &[dst, 2, 0]).unwrap();
    k.poke(off_in, &0u64.to_le_bytes());
    k.poke(off_out, &2u64.to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[both, off_in, both, off_out, 4, 0]), Err(Errno::EINVAL));
    k.poke(off_out, &4u64.to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[both, off_in, both, off_out, 3, 0]), Ok(3), "no overlap");
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[fd_in, 0, fd_out, 0, 1, 1]), Err(Errno::EINVAL), "flags");
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[fd_out, 0, fd_in, 0, 1, 0]), Err(Errno::EBADF), "wrong directions");
    let append = sys(k, nr::OPEN, &[dst, 0o2001, 0]).unwrap();
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[fd_in, 0, append, 0, 1, 0]), Err(Errno::EBADF), "O_APPEND");
    let dir = super::files::s(k, 0x300, b"/dir");
    let dir_fd = sys(k, nr::OPEN, &[dir, 0, 0]).unwrap();
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[dir_fd, 0, fd_out, 0, 1, 0]), Err(Errno::EISDIR));
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[77, 0, fd_out, 0, 1, 0]), Err(Errno::EBADF));
    k.poke(off_in, &(-1i64).to_le_bytes());
    assert_eq!(sys(k, nr::COPY_FILE_RANGE, &[fd_in, off_in, fd_out, 0, 1, 0]), Err(Errno::EOVERFLOW), "negative");
}

// ---- close_range --------------------------------------------------------

#[test]
fn close_range_closes_marks_and_checks_its_flags() {
    let mut e = super::files::env();
    let k = &mut e.k;
    let p = super::files::s(k, 0, b"/dir/file");
    let fds: std::vec::Vec<u64> = (0..5).map(|_| sys(k, nr::OPEN, &[p, 0, 0]).unwrap()).collect();
    assert_eq!(fds, [3, 4, 5, 6, 7]);
    assert_eq!(sys(k, nr::CLOSE_RANGE, &[5, 3, 0]), Err(Errno::EINVAL), "first > last");
    assert_eq!(sys(k, nr::CLOSE_RANGE, &[3, 4, 1]), Err(Errno::EINVAL), "an unknown flag");
    // CLOSE_RANGE_CLOEXEC marks 4 and 5.
    assert_eq!(sys(k, nr::CLOSE_RANGE, &[4, 5, 4]), Ok(0));
    assert_eq!(sys(k, nr::FCNTL, &[4, 1, 0]), Ok(1));
    assert_eq!(sys(k, nr::FCNTL, &[6, 1, 0]), Ok(0));
    // Closing from 5 to ~0U, unsharing a table nobody else uses.
    assert_eq!(sys(k, nr::CLOSE_RANGE, &[5, u64::from(u32::MAX), 2]), Ok(0));
    assert_eq!(sys(k, nr::FCNTL, &[5, 1, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FCNTL, &[7, 1, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FCNTL, &[4, 1, 0]), Ok(1), "below the range: still open");
    assert_eq!(sys(k, nr::OPEN, &[p, 0, 0]), Ok(5), "the lowest free descriptor again");
}

// ---- fadvise64 ----------------------------------------------------------

#[test]
fn fadvise64_checks_the_descriptor_the_file_and_the_advice() {
    let mut k = FakeKernel::new();
    k.files.insert(5, File::new(Kind::Regular));
    k.files.insert(6, File::new(Kind::Pipe));
    for advice in 0..=5 {
        assert_eq!(sys(&mut k, nr::FADVISE64, &[5, 0, 0, advice]), Ok(0), "advice {advice}");
    }
    assert_eq!(sys(&mut k, nr::FADVISE64, &[5, 4096, (-1i64) as u64, 2]), Ok(0), "a negative length");
    assert_eq!(sys(&mut k, nr::FADVISE64, &[5, 0, 0, 6]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FADVISE64, &[6, 0, 0, 0]), Err(Errno::ESPIPE));
    assert_eq!(sys(&mut k, nr::FADVISE64, &[9, 0, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::FADVISE64, &[(-1i64) as u64, 0, 0, 0]), Err(Errno::EBADF));
}
