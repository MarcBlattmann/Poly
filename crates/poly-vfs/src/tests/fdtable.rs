//! Descriptor tables: `dup(2)`, `dup2`, `dup3`, `fcntl(2)` `F_DUPFD`,
//! `F_GETFD`/`F_SETFD`, close-on-exec, lowest-free allocation, limits.

use super::Env;
use crate::*;
use alloc::sync::Arc;
use alloc::vec::Vec;

fn file(env: &Env) -> Arc<OpenFile> {
    env.open("/f", OpenFlags::RDWR | OpenFlags::CREAT).unwrap()
}

#[test]
fn lowest_free_descriptor() {
    let env = Env::new();
    let mut t = FdTable::new();
    for want in 0..5 {
        assert_eq!(t.install(file(&env), false), Ok(want));
    }
    t.close(1).unwrap();
    t.close(3).unwrap();
    assert_eq!(t.install(file(&env), false), Ok(1));
    assert_eq!(t.install(file(&env), false), Ok(3));
    assert_eq!(t.install(file(&env), false), Ok(5));
    assert_eq!(t.count(), 6);
    assert_eq!(t.close(1).map(|_| ()), Ok(()));
    assert_eq!(t.close(1).map(|_| ()), Err(Errno::EBADF));
    assert_eq!(t.close(-1).map(|_| ()), Err(Errno::EBADF));
    assert_eq!(t.close(1000).map(|_| ()), Err(Errno::EBADF));
    assert_eq!(t.get(1).map(|_| ()), Err(Errno::EBADF));
    let fds: Vec<Fd> = t.iter().map(|(fd, _, _)| fd).collect();
    assert_eq!(fds, [0, 2, 3, 4, 5]);
}

#[test]
fn dup_and_fdupfd() {
    let env = Env::new();
    let mut t = FdTable::new();
    let a = t.install(file(&env), true).unwrap();
    let b = t.dup(a).unwrap();
    assert_eq!(b, 1);
    assert!(Arc::ptr_eq(t.get(a).unwrap(), t.get(b).unwrap()));
    // The copy does not inherit close-on-exec.
    assert_eq!(t.cloexec(a), Ok(true));
    assert_eq!(t.cloexec(b), Ok(false));
    // F_DUPFD: lowest free at or above the minimum.
    assert_eq!(t.dup_from(a, 10, false), Ok(10));
    assert_eq!(t.dup_from(a, 10, true), Ok(11));
    assert_eq!(t.cloexec(11), Ok(true));
    assert_eq!(t.dup_from(a, 2, false), Ok(2));
    assert_eq!(t.dup_from(a, -1, false), Err(Errno::EINVAL));
    assert_eq!(t.dup_from(a, DEFAULT_FD_LIMIT as Fd, false), Err(Errno::EINVAL));
    assert_eq!(t.dup(7), Err(Errno::EBADF));
}

#[test]
fn dup2_semantics() {
    let env = Env::new();
    let mut t = FdTable::new();
    let a = t.install(file(&env), true).unwrap();
    let other = env.open("/g", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    let b = t.install(other.clone(), false).unwrap();
    // dup2 onto itself: checks the descriptor, changes nothing (cloexec stays).
    assert_eq!(t.dup2(a, a), Ok(a));
    assert_eq!(t.cloexec(a), Ok(true));
    assert_eq!(t.dup2(9, 9), Err(Errno::EBADF));
    // dup2 onto an open descriptor closes it silently.
    assert_eq!(Arc::strong_count(&other), 2);
    assert_eq!(t.dup2(a, b), Ok(b));
    assert_eq!(Arc::strong_count(&other), 1);
    assert!(Arc::ptr_eq(t.get(a).unwrap(), t.get(b).unwrap()));
    assert_eq!(t.cloexec(b), Ok(false));
    // A gap is fine; out of range is EBADF.
    assert_eq!(t.dup2(a, 100), Ok(100));
    assert_eq!(t.install(file(&env), false), Ok(2));
    assert_eq!(t.dup2(a, -1), Err(Errno::EBADF));
    assert_eq!(t.dup2(a, DEFAULT_FD_LIMIT as Fd), Err(Errno::EBADF));
    assert_eq!(t.dup2(55, 3), Err(Errno::EBADF));
}

#[test]
fn dup3_semantics() {
    let env = Env::new();
    let mut t = FdTable::new();
    let a = t.install(file(&env), false).unwrap();
    assert_eq!(t.dup3(a, a, OpenFlags::RDONLY), Err(Errno::EINVAL));
    assert_eq!(t.dup3(a, a, OpenFlags::CLOEXEC), Err(Errno::EINVAL));
    assert_eq!(t.dup3(a, 4, OpenFlags::APPEND), Err(Errno::EINVAL));
    assert_eq!(t.dup3(a, 4, OpenFlags::CLOEXEC), Ok(4));
    assert_eq!(t.cloexec(4), Ok(true));
    assert_eq!(t.dup3(a, 5, OpenFlags::RDONLY), Ok(5));
    assert_eq!(t.cloexec(5), Ok(false));
    assert_eq!(t.dup3(3, 6, OpenFlags::RDONLY), Err(Errno::EBADF));
}

#[test]
fn close_on_exec_and_fork() {
    let env = Env::new();
    let mut t = FdTable::new();
    let keep = t.install(file(&env), false).unwrap();
    let drop_me = t.install(file(&env), true).unwrap();
    let flip = t.install(file(&env), false).unwrap();
    t.set_cloexec(flip, true).unwrap();
    assert_eq!(t.set_cloexec(77, true), Err(Errno::EBADF));
    assert_eq!(t.cloexec(77), Err(Errno::EBADF));

    // fork: the child shares descriptions, flags included.
    let mut child = t.clone();
    assert!(Arc::ptr_eq(child.get(keep).unwrap(), t.get(keep).unwrap()));
    assert_eq!(child.cloexec(drop_me), Ok(true));

    child.close_on_exec();
    assert!(child.get(keep).is_ok());
    assert_eq!(child.get(drop_me).map(|_| ()), Err(Errno::EBADF));
    assert_eq!(child.get(flip).map(|_| ()), Err(Errno::EBADF));
    assert_eq!(child.count(), 1);
    // The parent is unaffected.
    assert_eq!(t.count(), 3);
    // New descriptors fill the holes again.
    assert_eq!(child.install(file(&env), false), Ok(1));
    child.close_all();
    assert_eq!(child.count(), 0);
}

#[test]
fn limits() {
    let env = Env::new();
    let mut t = FdTable::with_limit(3);
    for _ in 0..3 {
        t.install(file(&env), false).unwrap();
    }
    assert_eq!(t.install(file(&env), false), Err(Errno::EMFILE));
    assert_eq!(t.dup(0), Err(Errno::EMFILE));
    t.set_limit(2);
    // Existing descriptors above the limit stay usable.
    assert!(t.get(2).is_ok());
    t.close(1).unwrap();
    assert_eq!(t.install(file(&env), false), Ok(1));
    assert_eq!(t.dup_from(0, 2, false), Err(Errno::EINVAL));
}

#[test]
fn dirfd_resolution() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/x", b"x");
    let mut t = FdTable::new();
    let d = t.install(env.open("/d", OpenFlags::RDONLY | OpenFlags::DIRECTORY).unwrap(), false).unwrap();
    let at = t.at_for(d, b"x").unwrap();
    assert_eq!(env.vfs.stat(&env.ctx, &env.root, at, b"x", true).unwrap().size, 1);
    assert!(matches!(t.at(AT_FDCWD), Ok(At::Cwd)));
    assert_eq!(t.at(42).map(|_| ()), Err(Errno::EBADF));
    // Absolute paths ignore even a bad descriptor.
    assert!(matches!(t.at_for(42, b"/d/x"), Ok(At::Cwd)));
    // A descriptor on a file: ENOTDIR for relative paths.
    let f = t.install(env.open("/d/x", OpenFlags::RDONLY).unwrap(), false).unwrap();
    let at = t.at_for(f, b"y").unwrap();
    assert_eq!(env.vfs.stat(&env.ctx, &env.root, at, b"y", true), Err(Errno::ENOTDIR));
}

#[test]
fn close_range_closes_or_marks_a_range() {
    let env = Env::new();
    let mut t = FdTable::new();
    for _ in 0..8 {
        t.install(file(&env), false).unwrap();
    }
    t.close(4).unwrap();
    // Marking: 2..=3 and 5 get close-on-exec, nothing closes.
    assert!(t.close_range(2, 5, true).is_empty());
    let flags: Vec<(Fd, bool)> = t.iter().map(|(fd, _, c)| (fd, c)).collect();
    assert_eq!(flags, [(0, false), (1, false), (2, true), (3, true), (5, true), (6, false), (7, false)]);
    // Closing 3 to "all": the files come back to the caller.
    assert_eq!(t.close_range(3, u32::MAX, false).len(), 4);
    assert_eq!(t.iter().map(|(fd, _, _)| fd).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(t.install(file(&env), false), Ok(3), "the table shrank");
    // A range past the end, and a range of one.
    assert!(t.close_range(10, 20, false).is_empty());
    assert_eq!(t.close_range(1, 1, false).len(), 1);
    assert_eq!(t.iter().map(|(fd, _, _)| fd).collect::<Vec<_>>(), [0, 2, 3]);
}
