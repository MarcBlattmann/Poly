//! The credential, capability, prctl and file permission calls (ADR
//! 0045) through the dispatcher, against the fake kernel.

use super::fake::{DATA, FakeKernel, UNMAPPED, sys};
use crate::cred::{AttrChange, CapSet, Caps, Creds, FileTarget, FsIds, TimeChange, cap};
use crate::errno::Errno;
use crate::kernel::LinuxKernel;
use crate::nr;
use crate::sys::cred::{CAPABILITY_VERSION_1, CAPABILITY_VERSION_3};
use crate::sys::perm::{AT_EACCESS, AT_EMPTY_PATH, AT_FDCWD, AT_SYMLINK_NOFOLLOW, UTIME_NOW, UTIME_OMIT};
use crate::sys::prctl::*;
use alloc::vec::Vec;

/// A file target, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Path(i32, Vec<u8>, bool),
    Fd(i32),
    Empty(i32),
}

impl From<FileTarget<'_>> for Target {
    fn from(t: FileTarget<'_>) -> Target {
        match t {
            FileTarget::Path { dirfd, path, follow } => Target::Path(dirfd, path.to_vec(), follow),
            FileTarget::Fd(fd) => Target::Fd(fd),
            FileTarget::EmptyPath(fd) => Target::Empty(fd),
        }
    }
}

/// What the fake kernel saw of `change_attr`, `check_access` and `umask`.
#[derive(Debug)]
pub struct PermCalls {
    pub changes: Vec<(Target, AttrChange, FsIds)>,
    pub accesses: Vec<(Target, u32, FsIds)>,
    pub umask: u32,
    /// What the next calls return.
    pub result: Result<(), Errno>,
}

impl Default for PermCalls {
    fn default() -> Self {
        PermCalls { changes: Vec::new(), accesses: Vec::new(), umask: 0o022, result: Ok(()) }
    }
}

impl PermCalls {
    pub fn change(&mut self, file: FileTarget<'_>, change: &AttrChange, ids: &FsIds) -> Result<(), Errno> {
        self.changes.push((file.into(), *change, ids.clone()));
        self.result
    }

    pub fn access(&mut self, file: FileTarget<'_>, mode: u32, ids: &FsIds) -> Result<(), Errno> {
        self.accesses.push((file.into(), mode, ids.clone()));
        self.result
    }
}

const ALICE: u32 = 1000;
const P: u64 = DATA + 0x100;

fn root() -> FakeKernel {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    k
}

fn neg(v: i32) -> u64 {
    v as i64 as u64
}

fn resuid(k: &mut FakeKernel) -> [u32; 3] {
    sys(k, nr::GETRESUID, &[P, P + 4, P + 8]).unwrap();
    [k.peek_u32(P), k.peek_u32(P + 4), k.peek_u32(P + 8)]
}

fn resgid(k: &mut FakeKernel) -> [u32; 3] {
    sys(k, nr::GETRESGID, &[P, P + 4, P + 8]).unwrap();
    [k.peek_u32(P), k.peek_u32(P + 4), k.peek_u32(P + 8)]
}

#[test]
fn user_ids_through_the_calls() {
    let mut k = root();
    assert_eq!(sys(&mut k, nr::SETRESUID, &[ALICE as u64, u64::MAX, u64::from(u32::MAX)]), Ok(0));
    assert_eq!(resuid(&mut k), [ALICE, 0, 0]);
    assert_eq!(sys(&mut k, nr::GETUID, &[]), Ok(ALICE as u64));
    assert_eq!(sys(&mut k, nr::GETEUID, &[]), Ok(0));
    // Drop the effective id, then all of them.
    assert_eq!(sys(&mut k, nr::SETREUID, &[u64::MAX, ALICE as u64]), Ok(0));
    assert_eq!(resuid(&mut k), [ALICE, ALICE, 0], "euid set to the old real uid: saved unchanged");
    assert_eq!(k.creds().caps.permitted, CapSet::ALL, "one id is still 0");
    assert_eq!(sys(&mut k, nr::SETRESUID, &[u64::MAX, u64::MAX, ALICE as u64]), Ok(0));
    assert_eq!(resuid(&mut k), [ALICE, ALICE, ALICE]);
    assert_eq!(sys(&mut k, nr::SETUID, &[0]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::SETUID, &[u64::from(u32::MAX)]), Err(Errno::EINVAL));
    assert!(k.creds().caps.permitted.is_empty());
    // setfsuid: returns the old value; -1 reads it.
    assert_eq!(sys(&mut k, nr::SETFSUID, &[u64::from(u32::MAX)]), Ok(ALICE as u64));
    assert_eq!(sys(&mut k, nr::SETFSUID, &[5]), Ok(ALICE as u64));
    assert_eq!(k.creds().fsuid, ALICE, "unprivileged: unchanged");
    // EFAULT for any pointer of getresuid.
    assert_eq!(sys(&mut k, nr::GETRESUID, &[P, UNMAPPED, P + 8]), Err(Errno::EFAULT));
    // setuid as root sets all three.
    let mut r = root();
    assert_eq!(sys(&mut r, nr::SETUID, &[42]), Ok(0));
    assert_eq!(resuid(&mut r), [42, 42, 42]);
}

#[test]
fn group_ids_and_supplementary_groups() {
    let mut k = root();
    assert_eq!(sys(&mut k, nr::SETRESGID, &[1, 2, 3]), Ok(0));
    assert_eq!(resgid(&mut k), [1, 2, 3]);
    assert_eq!(sys(&mut k, nr::GETGID, &[]), Ok(1));
    assert_eq!(sys(&mut k, nr::GETEGID, &[]), Ok(2));
    assert_eq!(sys(&mut k, nr::SETREGID, &[4, u64::MAX]), Ok(0));
    assert_eq!(resgid(&mut k), [4, 2, 2]);
    assert_eq!(sys(&mut k, nr::SETGID, &[9]), Ok(0));
    assert_eq!(resgid(&mut k), [9, 9, 9]);
    assert_eq!(sys(&mut k, nr::SETFSGID, &[7]), Ok(9));
    assert_eq!(k.creds().fsgid, 7);
    // Groups: set, count, read back.
    let list: Vec<u8> = [10u32, 20, 30].iter().flat_map(|g| g.to_le_bytes()).collect();
    k.poke(P, &list);
    assert_eq!(sys(&mut k, nr::SETGROUPS, &[3, P]), Ok(0));
    assert_eq!(sys(&mut k, nr::GETGROUPS, &[0, 0]), Ok(3), "size 0: the count, list untouched");
    assert_eq!(sys(&mut k, nr::GETGROUPS, &[2, P + 0x40]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::GETGROUPS, &[neg(-1), P + 0x40]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::GETGROUPS, &[8, P + 0x40]), Ok(3));
    assert_eq!(k.peek(P + 0x40, 12), list);
    assert_eq!(sys(&mut k, nr::GETGROUPS, &[3, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SETGROUPS, &[65537, P]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SETGROUPS, &[1, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SETGROUPS, &[0, 0]), Ok(0), "an empty list clears them");
    assert_eq!(sys(&mut k, nr::GETGROUPS, &[0, 0]), Ok(0));
    // Unprivileged: EPERM.
    let mut u = FakeKernel::new();
    assert_eq!(sys(&mut u, nr::SETGROUPS, &[1, P]), Err(Errno::EPERM));
    assert_eq!(sys(&mut u, nr::SETGID, &[5]), Err(Errno::EPERM));
}

/// Write a capability header at `P` and room for data at `P + 16`.
fn header(k: &mut FakeKernel, version: u32, pid: i32) {
    k.poke(P, &version.to_le_bytes());
    k.poke(P + 4, &pid.to_le_bytes());
}

fn data(words: [[u32; 3]; 2]) -> Vec<u8> {
    words.iter().flatten().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn capget_and_capset() {
    let mut k = root();
    // An unknown version: EINVAL, the preferred version written back.
    header(&mut k, 0, 0);
    assert_eq!(sys(&mut k, nr::CAPGET, &[P, 0]), Err(Errno::EINVAL));
    assert_eq!(k.peek_u32(P), CAPABILITY_VERSION_3);
    // Version 3: two structs, low words first.
    assert_eq!(sys(&mut k, nr::CAPGET, &[P, P + 16]), Ok(0));
    assert_eq!(k.peek(P + 16, 24), data([[u32::MAX, u32::MAX, 0], [0x1ff, 0x1ff, 0]]));
    // Version 1: one struct.
    k.poke(P + 16, &[0xee; 24]);
    header(&mut k, CAPABILITY_VERSION_1, 0);
    assert_eq!(sys(&mut k, nr::CAPGET, &[P, P + 16]), Ok(0));
    assert_eq!(k.peek(P + 16, 12), data([[u32::MAX, u32::MAX, 0], [0; 3]])[..12].to_vec());
    assert_eq!(k.peek(P + 28, 4), [0xee; 4], "nothing past one struct");
    // Drop CAP_DAC_OVERRIDE from effective and permitted, CAP_SYS_TIME
    // from effective only.
    header(&mut k, CAPABILITY_VERSION_3, 0);
    let eff = !(1u32 << cap::DAC_OVERRIDE) & !(1 << cap::SYS_TIME);
    let perm = !(1u32 << cap::DAC_OVERRIDE);
    k.poke(P + 16, &data([[eff, perm, 0], [0x1ff, 0x1ff, 0]]));
    assert_eq!(sys(&mut k, nr::CAPSET, &[P, P + 16]), Ok(0));
    let c = k.creds().caps;
    assert!(!c.effective.contains(cap::DAC_OVERRIDE) && !c.permitted.contains(cap::DAC_OVERRIDE));
    assert!(!c.effective.contains(cap::SYS_TIME) && c.permitted.contains(cap::SYS_TIME));
    // Adding back to permitted: EPERM.
    k.poke(P + 16, &data([[eff, u32::MAX, 0], [0x1ff, 0x1ff, 0]]));
    assert_eq!(sys(&mut k, nr::CAPSET, &[P, P + 16]), Err(Errno::EPERM));
    // Another thread: EPERM for capset, readable with capget.
    k.add_thread(101);
    header(&mut k, CAPABILITY_VERSION_3, 101);
    assert_eq!(sys(&mut k, nr::CAPSET, &[P, P + 16]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::CAPGET, &[P, P + 16]), Ok(0));
    assert_eq!(k.peek_u32(P + 16), eff, "a copy of the creator's");
    header(&mut k, CAPABILITY_VERSION_3, 999);
    assert_eq!(sys(&mut k, nr::CAPGET, &[P, P + 16]), Err(Errno::ESRCH));
    header(&mut k, CAPABILITY_VERSION_3, -1);
    assert_eq!(sys(&mut k, nr::CAPGET, &[P, P + 16]), Err(Errno::EINVAL));
    header(&mut k, CAPABILITY_VERSION_3, 0);
    assert_eq!(sys(&mut k, nr::CAPGET, &[P, 0]), Ok(0), "a null data pointer only checks the header");
    assert_eq!(sys(&mut k, nr::CAPGET, &[UNMAPPED, P + 16]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::CAPSET, &[P, UNMAPPED]), Err(Errno::EFAULT));
    // The caller's own tid is fine for capset.
    header(&mut k, CAPABILITY_VERSION_1, 100);
    k.poke(P + 16, &data([[0, 0, 0], [0; 3]]));
    assert_eq!(sys(&mut k, nr::CAPSET, &[P, P + 16]), Ok(0));
    assert_eq!(k.creds().caps.permitted, CapSet::EMPTY, "version 1: the high words are 0");
}

#[test]
fn prctl_capabilities_and_flags() {
    let mut k = root();
    let p = |k: &mut FakeKernel, args: &[u64]| sys(k, nr::PRCTL, args);
    assert_eq!(p(&mut k, &[PR_GET_KEEPCAPS.into()]), Ok(0));
    assert_eq!(p(&mut k, &[PR_SET_KEEPCAPS.into(), 2]), Err(Errno::EINVAL));
    assert_eq!(p(&mut k, &[PR_SET_KEEPCAPS.into(), 1]), Ok(0));
    assert_eq!(p(&mut k, &[PR_GET_KEEPCAPS.into()]), Ok(1));
    assert_eq!(p(&mut k, &[PR_GET_SECUREBITS.into()]), Ok(SECBIT_KEEP_CAPS));
    // keep_caps: dropping every uid keeps the permitted set.
    assert_eq!(sys(&mut k, nr::SETRESUID, &[ALICE as u64; 3]), Ok(0));
    assert_eq!(k.creds().caps.permitted, CapSet::ALL);
    assert_eq!(k.creds().caps.effective, CapSet::EMPTY);
    // The bounding set.
    let mut r = root();
    assert_eq!(p(&mut r, &[PR_CAPBSET_READ.into(), cap::NET_RAW.into()]), Ok(1));
    assert_eq!(p(&mut r, &[PR_CAPBSET_DROP.into(), cap::NET_RAW.into()]), Ok(0));
    assert_eq!(p(&mut r, &[PR_CAPBSET_READ.into(), cap::NET_RAW.into()]), Ok(0));
    assert_eq!(p(&mut r, &[PR_CAPBSET_READ.into(), 41]), Err(Errno::EINVAL));
    assert_eq!(p(&mut r, &[PR_CAPBSET_DROP.into(), u64::MAX]), Err(Errno::EINVAL));
    let mut u = FakeKernel::new();
    assert_eq!(p(&mut u, &[PR_CAPBSET_DROP.into(), cap::NET_RAW.into()]), Err(Errno::EPERM));
    // no_new_privs: set once, never unset.
    assert_eq!(p(&mut u, &[PR_GET_NO_NEW_PRIVS.into()]), Ok(0));
    assert_eq!(p(&mut u, &[PR_SET_NO_NEW_PRIVS.into(), 0]), Err(Errno::EINVAL));
    assert_eq!(p(&mut u, &[PR_SET_NO_NEW_PRIVS.into(), 1, 1]), Err(Errno::EINVAL));
    assert_eq!(p(&mut u, &[PR_SET_NO_NEW_PRIVS.into(), 1]), Ok(0));
    assert_eq!(p(&mut u, &[PR_GET_NO_NEW_PRIVS.into()]), Ok(1));
    assert_eq!(p(&mut u, &[PR_GET_NO_NEW_PRIVS.into(), 1]), Err(Errno::EINVAL));
    // Ambient capabilities.
    let amb = PR_CAP_AMBIENT.into();
    let nbs = u64::from(cap::NET_BIND_SERVICE);
    assert_eq!(p(&mut r, &[amb, PR_CAP_AMBIENT_RAISE, nbs]), Err(Errno::EPERM), "not inheritable");
    r.creds_mut().caps.inheritable = CapSet::only(cap::NET_BIND_SERVICE);
    assert_eq!(p(&mut r, &[amb, PR_CAP_AMBIENT_RAISE, nbs]), Ok(0));
    assert_eq!(p(&mut r, &[amb, PR_CAP_AMBIENT_IS_SET, nbs]), Ok(1));
    assert_eq!(p(&mut r, &[amb, PR_CAP_AMBIENT_LOWER, nbs]), Ok(0));
    assert_eq!(p(&mut r, &[amb, PR_CAP_AMBIENT_IS_SET, nbs]), Ok(0));
    assert_eq!(p(&mut r, &[amb, PR_CAP_AMBIENT_RAISE, nbs, 1]), Err(Errno::EINVAL));
    assert_eq!(p(&mut r, &[amb, PR_CAP_AMBIENT_CLEAR_ALL, 1]), Err(Errno::EINVAL));
    assert_eq!(p(&mut r, &[amb, PR_CAP_AMBIENT_CLEAR_ALL]), Ok(0));
    assert_eq!(p(&mut r, &[amb, 9]), Err(Errno::EINVAL));
    assert_eq!(p(&mut r, &[9999]), Err(Errno::EINVAL));
}

#[test]
fn prctl_names() {
    let mut k = FakeKernel::new();
    k.poke(P, b"worker\0");
    assert_eq!(sys(&mut k, nr::PRCTL, &[PR_SET_NAME.into(), P]), Ok(0));
    assert_eq!(sys(&mut k, nr::PRCTL, &[PR_GET_NAME.into(), P + 0x40]), Ok(0));
    assert_eq!(k.peek(P + 0x40, 16), b"worker\0\0\0\0\0\0\0\0\0\0");
    // Longer: cut to 15 bytes and a NUL.
    k.poke(P, b"a-name-longer-than-sixteen\0");
    assert_eq!(sys(&mut k, nr::PRCTL, &[PR_SET_NAME.into(), P]), Ok(0));
    assert_eq!(k.thread_state().comm, *b"a-name-longer-t\0");
    assert_eq!(sys(&mut k, nr::PRCTL, &[PR_SET_NAME.into(), UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::PRCTL, &[PR_GET_NAME.into(), UNMAPPED]), Err(Errno::EFAULT));
    // Per thread; a new thread copies its creator's.
    k.add_thread(101);
    k.poke(P, b"other\0");
    k.as_thread(101, |k| sys(k, nr::PRCTL, &[PR_SET_NAME.into(), P]).unwrap());
    assert_eq!(k.thread_state().comm, *b"a-name-longer-t\0");
    assert_eq!(comm_from_path(b"/usr/bin/a-very-long-program-name"), *b"a-very-long-pro\0");
    assert_eq!(comm_from_path(b"hello"), *b"hello\0\0\0\0\0\0\0\0\0\0\0");
}

#[test]
fn credentials_are_per_thread() {
    let mut k = root();
    k.add_thread(101);
    k.as_thread(101, |k| sys(k, nr::SETRESUID, &[ALICE as u64; 3]).unwrap());
    assert_eq!(sys(&mut k, nr::GETUID, &[]), Ok(0), "the other thread is unchanged");
    assert_eq!(k.as_thread(101, |k| sys(k, nr::GETUID, &[])), Ok(ALICE as u64));
    // A fork copies the calling thread's.
    let child = k.me().fork(101, 300);
    assert_eq!(child.thread(300).creds.uid, ALICE);
    assert_eq!(child.process_creds(300).map(|c| c.uid), Some(ALICE));
    // exec keeps the exec'ing thread's.
    let mut p = k.me().clone();
    p.exec(101);
    assert_eq!(p.thread(101).creds.uid, ALICE);
    assert_eq!(p.process_creds(100).map(|c| c.uid), Some(ALICE), "the oldest thread stands in for the leader");
    // A zombie keeps the last thread's.
    p.remove_thread(101);
    assert_eq!(p.process_creds(100).map(|c| c.uid), Some(ALICE));
}

#[test]
fn kill_uses_the_target_threads_credentials() {
    let mut k = FakeKernel::new(); // real 1000, effective 1001
    k.add_process(300, 2000, 300, 300);
    let sigterm = u64::from(crate::abi::signal::SIGTERM);
    assert_eq!(sys(&mut k, nr::KILL, &[300, sigterm]), Err(Errno::EPERM));
    // The target's saved uid becomes 1001 (a set-user-ID program).
    k.proc_state(300).thread_mut(300).creds.suid = 1001;
    assert_eq!(sys(&mut k, nr::KILL, &[300, 0]), Ok(0));
    // Root without CAP_KILL is just another uid.
    let mut r = root();
    r.creds_mut().caps.effective = CapSet::ALL.without(cap::KILL);
    r.add_process(400, 4000, 400, 400);
    assert_eq!(sys(&mut r, nr::KILL, &[400, 0]), Err(Errno::EPERM));
    r.creds_mut().caps.effective = Caps::ROOT.effective;
    assert_eq!(sys(&mut r, nr::KILL, &[400, 0]), Ok(0));
}

fn path_at(k: &mut FakeKernel, path: &[u8]) -> u64 {
    let mut p = path.to_vec();
    p.push(0);
    k.poke(P + 0x200, &p);
    P + 0x200
}

#[test]
fn chmod_and_chown_requests() {
    let mut k = FakeKernel::new();
    let path = path_at(&mut k, b"/etc/f");
    assert_eq!(sys(&mut k, nr::CHMOD, &[path, 0o104755]), Ok(0));
    assert_eq!(sys(&mut k, nr::FCHMOD, &[3, 0o600]), Ok(0));
    assert_eq!(sys(&mut k, nr::FCHMODAT, &[neg(AT_FDCWD), path, 0o2711]), Ok(0));
    assert_eq!(sys(&mut k, nr::CHOWN, &[path, 5, u64::from(u32::MAX)]), Ok(0));
    assert_eq!(sys(&mut k, nr::LCHOWN, &[path, u64::MAX, 6]), Ok(0));
    assert_eq!(sys(&mut k, nr::FCHOWN, &[4, 7, 8]), Ok(0));
    assert_eq!(sys(&mut k, nr::FCHOWNAT, &[9, path, 1, 2, AT_SYMLINK_NOFOLLOW.into()]), Ok(0));
    let empty = path_at(&mut k, b"");
    assert_eq!(sys(&mut k, nr::FCHOWNAT, &[9, empty, 1, 2, AT_EMPTY_PATH.into()]), Ok(0));
    assert_eq!(sys(&mut k, nr::FCHOWNAT, &[9, empty, 1, 2, 0]), Err(Errno::ENOENT));
    assert_eq!(sys(&mut k, nr::FCHOWNAT, &[9, path, 1, 2, 0x800]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::CHMOD, &[UNMAPPED, 0o644]), Err(Errno::EFAULT));
    let p = |follow| Target::Path(AT_FDCWD, b"/etc/f".to_vec(), follow);
    let own = |uid, gid| AttrChange::Owner { uid, gid };
    let expected = [
        (p(true), AttrChange::Mode(0o4755)),
        (Target::Fd(3), AttrChange::Mode(0o600)),
        (p(true), AttrChange::Mode(0o2711)),
        (p(true), own(Some(5), None)),
        (p(false), own(None, Some(6))),
        (Target::Fd(4), own(Some(7), Some(8))),
        (Target::Path(9, b"/etc/f".to_vec(), false), own(Some(1), Some(2))),
        (Target::Empty(9), own(Some(1), Some(2))),
    ];
    let got: Vec<(Target, AttrChange)> = k.perm.changes.iter().map(|(t, c, _)| (t.clone(), *c)).collect();
    assert_eq!(got, expected);
    // Checked as the file system ids, with the effective capabilities.
    let ids = &k.perm.changes[0].2;
    assert_eq!((ids.uid, ids.gid, ids.caps), (1001, 2001, CapSet::EMPTY));
    // The kernel's answer comes back.
    k.perm.result = Err(Errno::EPERM);
    assert_eq!(sys(&mut k, nr::FCHMOD, &[3, 0o600]), Err(Errno::EPERM));
}

#[test]
fn access_uses_real_ids_unless_eaccess() {
    let mut k = root();
    sys(&mut k, nr::SETRESUID, &[ALICE as u64, u64::MAX, u64::MAX]).unwrap();
    let path = path_at(&mut k, b"f");
    assert_eq!(sys(&mut k, nr::ACCESS, &[path, 4]), Ok(0));
    assert_eq!(sys(&mut k, nr::ACCESS, &[path, 8]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FACCESSAT, &[3, path, 2]), Ok(0));
    assert_eq!(sys(&mut k, nr::FACCESSAT2, &[3, path, 1, AT_EACCESS.into()]), Ok(0));
    assert_eq!(sys(&mut k, nr::FACCESSAT2, &[3, path, 1, AT_SYMLINK_NOFOLLOW.into()]), Ok(0));
    assert_eq!(sys(&mut k, nr::FACCESSAT2, &[3, path, 1, 0x4000]), Err(Errno::EINVAL));
    let a = &k.perm.accesses;
    assert_eq!(a.len(), 4);
    assert_eq!((a[0].0.clone(), a[0].1), (Target::Path(AT_FDCWD, b"f".to_vec(), true), 4));
    assert_eq!((a[0].2.uid, a[0].2.caps), (ALICE, CapSet::EMPTY), "real uid, no capabilities");
    assert_eq!((a[1].0.clone(), a[1].1), (Target::Path(3, b"f".to_vec(), true), 2));
    assert_eq!((a[2].2.uid, a[2].2.caps), (0, CapSet::ALL), "AT_EACCESS: effective ids and capabilities");
    assert_eq!(a[3].0, Target::Path(3, b"f".to_vec(), false));
}

#[test]
fn umask_returns_the_old_mask() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::UMASK, &[0o077]), Ok(0o022));
    assert_eq!(sys(&mut k, nr::UMASK, &[0o7777]), Ok(0o077));
    assert_eq!(k.perm.umask, 0o777, "only the permission bits");
}

fn times(k: &mut FakeKernel, a: (i64, i64), m: (i64, i64)) -> u64 {
    let b: Vec<u8> = [a.0, a.1, m.0, m.1].iter().flat_map(|v| v.to_le_bytes()).collect();
    k.poke(P + 0x300, &b);
    P + 0x300
}

#[test]
fn utimensat_requests() {
    let mut k = FakeKernel::new();
    let path = path_at(&mut k, b"/f");
    let now = Some(TimeChange::Now);
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[neg(AT_FDCWD), path, 0, 0]), Ok(0));
    let t = times(&mut k, (5, 6), (0, UTIME_OMIT));
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[neg(AT_FDCWD), path, t, AT_SYMLINK_NOFOLLOW.into()]), Ok(0));
    let t = times(&mut k, (9, UTIME_NOW), (1, UTIME_OMIT));
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[7, 0, t, 0]), Ok(0), "a null path: the file fd 7 is");
    let omit = times(&mut k, (0, UTIME_OMIT), (0, UTIME_OMIT));
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[neg(AT_FDCWD), path, omit, 0]), Ok(0));
    assert_eq!(k.perm.changes.len(), 3, "both omitted: nothing asked of the kernel");
    let bad = times(&mut k, (0, 1_000_000_000), (0, 0));
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[neg(AT_FDCWD), path, bad, 0]), Err(Errno::EINVAL));
    let bad = times(&mut k, (0, 0), (0, -1));
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[neg(AT_FDCWD), path, bad, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[neg(AT_FDCWD), path, 0, 0x2]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[neg(AT_FDCWD), 0, 0, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::UTIMENSAT, &[neg(AT_FDCWD), path, UNMAPPED, 0]), Err(Errno::EFAULT));
    let c = &k.perm.changes;
    let at = |s, n| Some(TimeChange::At(crate::abi::Timespec { sec: s, nsec: n }));
    assert_eq!(c[0].1, AttrChange::Times { atime: now, mtime: now });
    assert_eq!(
        (c[1].0.clone(), c[1].1),
        (Target::Path(AT_FDCWD, b"/f".to_vec(), false), AttrChange::Times { atime: at(5, 6), mtime: None })
    );
    assert_eq!((c[2].0.clone(), c[2].1), (Target::Fd(7), AttrChange::Times { atime: now, mtime: None }));
}

#[test]
fn the_fake_kernel_has_root_and_user_credentials() {
    let k = root();
    assert!(k.creds().capable(cap::SETUID));
    assert_eq!(k.credentials().euid, 0);
    let u = FakeKernel::new();
    assert_eq!(u.credentials().uid, 1000);
}
