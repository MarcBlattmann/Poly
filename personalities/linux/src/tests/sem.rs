//! System V semaphores: semget(2), and the semop(2) and semctl(2) its
//! tests observe sets with (`specs/linux/x86_64/semget.yaml` and its
//! visible tests).
//!
//! Each test runs in an IPC namespace of its own, so tests running at the
//! same time do not see each other's sets.

use super::fake::*;
use crate::ipc::sem::*;
use crate::ipc::{IPC_CREAT, IPC_EXCL, IPC_NOWAIT, IPC_RMID, IPC_STAT};
use crate::ns::IpcNs;
use crate::{Errno, nr};
use alloc::vec::Vec;

const CREAT: i64 = IPC_CREAT as i64;
const EXCL: i64 = IPC_EXCL as i64;
const NOWAIT: u16 = IPC_NOWAIT as u16;
const BUF: u64 = DATA + 0x1000;

/// A fake process in an IPC namespace of its own (pid 100, euid 1001,
/// egid 2001, no capabilities).
fn fresh() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.me().ns.ipc = Some(IpcNs::create(&None));
    k
}

/// Add process `pid` (uid `uid`) to the caller's IPC namespace.
fn another(k: &mut FakeKernel, pid: i32, uid: u32) {
    let ns = k.me().ns.ipc.clone();
    k.add_process(pid, uid, pid, pid).state.ns.ipc = ns;
}

fn semget(k: &mut FakeKernel, key: i64, nsems: i64, flags: i64) -> Result<i32, Errno> {
    sys(k, nr::SEMGET, &[key as u64, nsems as u64, flags as u64]).map(|id| id as i32)
}

fn semctl(k: &mut FakeKernel, id: i32, num: i32, cmd: u32, arg: u64) -> Result<u64, Errno> {
    sys(k, nr::SEMCTL, &[id as u64, num as u64, u64::from(cmd), arg])
}

fn rm(k: &mut FakeKernel, id: i32) -> Result<u64, Errno> {
    semctl(k, id, 0, IPC_RMID, 0)
}

fn semop(k: &mut FakeKernel, id: i32, ops: &[(u16, i16, u16)]) -> Result<u64, Errno> {
    let bytes: Vec<u8> = ops
        .iter()
        .flat_map(|(num, op, flg)| [num.to_ne_bytes(), op.to_ne_bytes(), flg.to_ne_bytes()].concat())
        .collect();
    k.poke(BUF, &bytes);
    sys(k, nr::SEMOP, &[id as u64, BUF, ops.len() as u64])
}

fn stat(k: &mut FakeKernel, id: i32) -> Result<SemidDs, Errno> {
    k.poke(BUF, &[0xa5; SemidDs::SIZE]);
    semctl(k, id, 0, IPC_STAT, BUF)?;
    let b = k.peek(BUF, SemidDs::SIZE);
    let u32_at = |at: usize| u32::from_ne_bytes(b[at..at + 4].try_into().unwrap());
    let u64_at = |at: usize| u64::from_ne_bytes(b[at..at + 8].try_into().unwrap());
    let zeros = [22..24, 26..48, 56..64, 72..80, 88..104];
    assert!(zeros.iter().all(|r| b[r.clone()].iter().all(|x| *x == 0)), "padding and reserved words are 0");
    Ok(SemidDs {
        key: u32_at(0) as i32,
        uid: u32_at(4),
        gid: u32_at(8),
        cuid: u32_at(12),
        cgid: u32_at(16),
        mode: u16::from_ne_bytes([b[20], b[21]]),
        seq: u16::from_ne_bytes([b[24], b[25]]),
        otime: u64_at(48) as i64,
        ctime: u64_at(64) as i64,
        nsems: u64_at(80),
    })
}

#[test]
fn create_needs_a_semaphore() {
    let mut k = fresh();
    let key = 0x5040_0064;
    assert_eq!(semget(&mut k, key, 0, 0o600 | CREAT), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, key, 0, 0o600 | CREAT | EXCL), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, key, 1, 0), Err(Errno::ENOENT));
    assert_eq!(semget(&mut k, 0, 0, 0o600), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, 0, 0, 0o600 | CREAT), Err(Errno::EINVAL));
    let id = semget(&mut k, key, 1, 0o600 | CREAT).unwrap();
    assert_eq!(semget(&mut k, key, 0, 0), Ok(id));
    assert_eq!(semget(&mut k, key, 0, CREAT), Ok(id));
    assert_eq!(rm(&mut k, id), Ok(0));
}

#[test]
fn existing_set_nsems_and_error_order() {
    let mut k = fresh();
    let key = 0x5040_0065;
    let id = semget(&mut k, key, 4, 0o600 | CREAT | EXCL).unwrap();
    assert_eq!(semget(&mut k, key, 4, 0), Ok(id));
    assert_eq!(semget(&mut k, key, 1, 0), Ok(id));
    assert_eq!(semget(&mut k, key, 5, 0), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, key, 5, 0o600 | CREAT), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, key, 5, 0o600 | CREAT | EXCL), Err(Errno::EEXIST));
    assert_eq!(semget(&mut k, key, i64::from(SEMMSL), 0), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, key, 5, 0o700), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, key, 4, 0o700), Err(Errno::EACCES));
    assert_eq!(semget(&mut k, key, 4, 0o700 | CREAT | EXCL), Err(Errno::EEXIST));
    assert_eq!(stat(&mut k, id).unwrap().nsems, 4);
    // Root (CAP_IPC_OWNER) passes the access check.
    *k.creds_mut() = crate::cred::Creds::root();
    assert_eq!(semget(&mut k, key, 4, 0o700), Ok(id));
}

#[test]
fn nsems_range_comes_first() {
    let mut k = fresh();
    let key = 0x5040_0066;
    for nsems in [-1, i64::from(SEMMSL) + 1, 0x7fff_ffff, 0xffff_ffff] {
        assert_eq!(semget(&mut k, key, nsems, 0), Err(Errno::EINVAL), "{nsems:#x}");
        assert_eq!(semget(&mut k, key, nsems, 0o600 | CREAT), Err(Errno::EINVAL));
    }
    assert_eq!(semget(&mut k, key, 1, 0), Err(Errno::ENOENT), "nothing made");
    let id = semget(&mut k, key, 1, 0o600 | CREAT | EXCL).unwrap();
    assert_eq!(semget(&mut k, key, -1, 0), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, key, i64::from(SEMMSL) + 1, CREAT | EXCL), Err(Errno::EINVAL));
    assert_eq!(semget(&mut k, key, -5, CREAT | EXCL), Err(Errno::EINVAL));
    rm(&mut k, id).unwrap();
    let id = semget(&mut k, 0, i64::from(SEMMSL), 0o600).unwrap();
    assert_eq!(stat(&mut k, id).unwrap().nsems, SEMMSL as u64);
    rm(&mut k, id).unwrap();
    assert_eq!(semget(&mut k, 0, i64::from(SEMMSL) + 1, 0o600), Err(Errno::EINVAL));
}

#[test]
fn arguments_use_the_low_32_bits() {
    let mut k = fresh();
    // nsems 2^32 + 2 is 2; 2^32 is 0.
    let id = semget(&mut k, 0, (1 << 32) + 2, 0o600).unwrap();
    assert_eq!(stat(&mut k, id).unwrap().nsems, 2);
    assert_eq!(semget(&mut k, 0, 1 << 32, 0o600), Err(Errno::EINVAL));
    // A key whose low half is 0 is IPC_PRIVATE: a new set every time.
    let a = semget(&mut k, 1 << 32, 1, 0o600).unwrap();
    let b = semget(&mut k, 1 << 32, 1, 0o600).unwrap();
    assert_ne!(a, b);
    assert_eq!(stat(&mut k, a).unwrap().key, 0);
    let id = semget(&mut k, 0, 1, 0x77_0000_0000 | 0o444).unwrap();
    assert_eq!(stat(&mut k, id).unwrap().mode, 0o444);
}

#[test]
fn key_lookup() {
    let mut k = fresh();
    let key = 0x5040_0067;
    assert_eq!(semget(&mut k, key, 1, 0), Err(Errno::ENOENT));
    assert_eq!(semget(&mut k, key, 0, 0), Err(Errno::ENOENT));
    assert_eq!(semget(&mut k, key, 1, 0o600 | EXCL), Err(Errno::ENOENT));
    let id = semget(&mut k, key, 2, 0o600 | CREAT).unwrap();
    let ds = stat(&mut k, id).unwrap();
    assert_eq!((ds.key, ds.nsems), (key as i32, 2));
    assert_eq!(semget(&mut k, key, 2, 0), Ok(id));
    assert_eq!(semget(&mut k, key, 1, 0o600 | CREAT), Ok(id));
    assert_eq!(semget(&mut k, key, 2, 0o600 | EXCL), Ok(id));
    assert_eq!(semget(&mut k, key, 2, 0o600 | CREAT | EXCL), Err(Errno::EEXIST));
    assert_eq!(semget(&mut k, key, 0, CREAT | EXCL), Err(Errno::EEXIST));
    let ds = stat(&mut k, id).unwrap();
    assert_eq!((ds.nsems, ds.mode), (2, 0o600));
}

#[test]
fn new_set_stat() {
    let mut k = fresh();
    let id = semget(&mut k, -7, 3, 0o640 | CREAT).unwrap();
    let expected = SemidDs {
        key: -7,
        uid: 1001,
        gid: 2001,
        cuid: 1001,
        cgid: 2001,
        mode: 0o640,
        seq: 0,
        otime: 0,
        ctime: k.realtime.sec,
        nsems: 3,
    };
    assert_eq!(stat(&mut k, id), Ok(expected));
}

#[test]
fn mode_bits() {
    let mut k = fresh();
    for (flags, mode) in [
        (0o777, 0o777),
        (0o7777, 0o777),
        (0o644 | i64::from(IPC_NOWAIT) | 0o10000 | 0o20000, 0o644),
        (0o600 | 0x7ff0_0000, 0o600),
    ] {
        let id = semget(&mut k, 0, 1, flags).unwrap();
        assert_eq!(stat(&mut k, id).unwrap().mode, mode, "{flags:o}");
        rm(&mut k, id).unwrap();
    }
    let key = 0x5040_0068;
    let id = semget(&mut k, key, 1, 0o460 | CREAT | EXCL | 0o40000).unwrap();
    // Mode 0460 gives the owner read only: IPC_STAT needs it.
    assert_eq!(stat(&mut k, id).unwrap().mode, 0o460);
    assert_eq!(semget(&mut k, key, 1, 0), Ok(id));
    let id = semget(&mut k, 0, 1, 0).unwrap();
    assert_eq!(stat(&mut k, id), Err(Errno::EACCES), "mode 0: no read for a normal user");
}

#[test]
fn ids_are_not_descriptors_and_not_reused_next() {
    let mut k = fresh();
    let files = k.files.len();
    let ids: Vec<i32> = (0..8).map(|_| semget(&mut k, 0, 1, 0o600).unwrap()).collect();
    for (i, id) in ids.iter().enumerate() {
        assert!(*id >= 0 && !ids[..i].contains(id));
    }
    assert_eq!(k.files.len(), files);
    for id in &ids {
        assert_eq!(rm(&mut k, *id), Ok(0));
    }
    let mut prev = semget(&mut k, 0, 1, 0o600).unwrap();
    for _ in 0..8 {
        rm(&mut k, prev).unwrap();
        let next = semget(&mut k, 0, 1, 0o600).unwrap();
        assert_ne!(next, prev);
        prev = next;
    }
    let a = semget(&mut k, 0x5040_0069, 1, 0o600 | CREAT).unwrap();
    assert_ne!(a, prev);
}

#[test]
fn initial_values() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 5, 0o600).unwrap();
    k.poke(BUF, &[9; 12]);
    assert_eq!(semctl(&mut k, id, 0, GETALL, BUF), Ok(0));
    assert_eq!(k.peek(BUF, 12), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9, 9]);
    for i in 0..5 {
        for cmd in [GETVAL, GETPID, GETNCNT, GETZCNT] {
            assert_eq!(semctl(&mut k, id, i, cmd, 0), Ok(0));
        }
    }
    assert_eq!(semctl(&mut k, id, 5, GETVAL, 0), Err(Errno::EINVAL));
    assert_eq!(semctl(&mut k, id, -1, GETVAL, 0), Err(Errno::EINVAL));
    assert_eq!(semop(&mut k, id, &[(2, -1, NOWAIT)]), Err(Errno::EAGAIN));
    assert_eq!(semop(&mut k, id, &[(4, 0, 0)]), Ok(0));
    assert!(stat(&mut k, id).unwrap().otime != 0, "semop sets sem_otime");
}

#[test]
fn removed_key_is_free_at_once() {
    let mut k = fresh();
    let key = 0x5040_006a;
    let id = semget(&mut k, key, 2, 0o600 | CREAT | EXCL).unwrap();
    assert_eq!(semctl(&mut k, id, 1, SETVAL, 5), Ok(0));
    assert_eq!(semctl(&mut k, id, 1, GETVAL, 0), Ok(5));
    assert_eq!(semctl(&mut k, id, 1, GETPID, 0), Ok(100));
    assert_eq!(rm(&mut k, id), Ok(0));
    assert_eq!(semget(&mut k, key, 1, 0), Err(Errno::ENOENT));
    assert_eq!(stat(&mut k, id), Err(Errno::EINVAL));
    assert_eq!(semctl(&mut k, id, 1, GETVAL, 0), Err(Errno::EINVAL));
    let id2 = semget(&mut k, key, 2, 0o600 | CREAT | EXCL).unwrap();
    assert_ne!(id2, id);
    assert_eq!(semctl(&mut k, id2, 1, GETVAL, 0), Ok(0));
    assert_eq!(semget(&mut k, key, 0, 0), Ok(id2));
    assert_eq!(rm(&mut k, id2), Ok(0));
    assert_eq!(rm(&mut k, id2), Err(Errno::EINVAL));
}

#[test]
fn sets_are_shared_by_the_namespace() {
    let mut k = fresh();
    another(&mut k, 200, 1001);
    let key = 0x5040_006b;
    let id = k.as_thread(200, |k| {
        let id = semget(k, key, 3, 0o600 | CREAT | EXCL).unwrap();
        semctl(k, id, 2, SETVAL, 42).unwrap();
        id
    });
    assert_eq!(semget(&mut k, key, 0, 0o600), Ok(id));
    let ds = stat(&mut k, id).unwrap();
    assert_eq!((ds.nsems, ds.mode, ds.uid, ds.gid), (3, 0o600, 1001, 1001));
    assert_eq!(semctl(&mut k, id, 2, GETVAL, 0), Ok(42));
    assert_eq!(semctl(&mut k, id, 2, GETPID, 0), Ok(200));
}

#[test]
fn setval_and_setall() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 2, 0o600).unwrap();
    assert_eq!(semctl(&mut k, id, 0, SETVAL, SEMVMX as u64 + 1), Err(Errno::ERANGE));
    assert_eq!(semctl(&mut k, id, 0, SETVAL, u64::MAX), Err(Errno::ERANGE));
    assert_eq!(semctl(&mut k, id, 2, SETVAL, 1), Err(Errno::EINVAL));
    // The value is an int: the upper half of the register is ignored.
    assert_eq!(semctl(&mut k, id, 0, SETVAL, (1 << 32) | 3), Ok(0));
    assert_eq!(semctl(&mut k, id, 0, GETVAL, 0), Ok(3));
    k.poke(BUF, &[7, 0, 8, 0]);
    assert_eq!(semctl(&mut k, id, 0, SETALL, BUF), Ok(0));
    assert_eq!((semctl(&mut k, id, 0, GETVAL, 0), semctl(&mut k, id, 1, GETVAL, 0)), (Ok(7), Ok(8)));
    assert_eq!(semctl(&mut k, id, 0, SETALL, 0), Err(Errno::EFAULT));
    assert_eq!(semctl(&mut k, id, 0, 99, 0), Err(Errno::EINVAL));
    // Read-only: SETVAL needs write.
    let ro = semget(&mut k, 0, 1, 0o400).unwrap();
    assert_eq!(semctl(&mut k, ro, 0, SETVAL, 1), Err(Errno::EACCES));
    assert_eq!(semctl(&mut k, ro, 0, GETVAL, 0), Ok(0));
}

#[test]
fn semop_errors() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 2, 0o600).unwrap();
    assert_eq!(sys(&mut k, nr::SEMOP, &[id as u64, BUF, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SEMOP, &[id as u64, BUF, SEMOPM + 1]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::SEMOP, &[id as u64, 0, 1]), Err(Errno::EFAULT));
    assert_eq!(semop(&mut k, id + 1, &[(0, 1, 0)]), Err(Errno::EINVAL));
    assert_eq!(semop(&mut k, id, &[(2, 1, 0)]), Err(Errno::EFBIG));
    let ro = semget(&mut k, 0, 1, 0o400).unwrap();
    assert_eq!(semop(&mut k, ro, &[(0, 1, 0)]), Err(Errno::EACCES));
    assert_eq!(semop(&mut k, ro, &[(0, 0, 0)]), Ok(0), "waiting for zero needs read only");
}

#[test]
fn semop_waits_until_another_process_changes_the_set() {
    let mut k = fresh();
    another(&mut k, 200, 1001);
    let id = semget(&mut k, 0, 1, 0o600).unwrap();
    k.meanwhile(move |k| {
        k.as_thread(200, |k| {
            assert_eq!(semctl(k, id, 0, GETNCNT, 0), Ok(1));
            assert_eq!(semctl(k, id, 0, SETVAL, 2), Ok(0));
        });
    });
    assert_eq!(semop(&mut k, id, &[(0, -1, 0)]), Ok(0));
    assert_eq!(semctl(&mut k, id, 0, GETVAL, 0), Ok(1));
    assert_eq!(semctl(&mut k, id, 0, GETPID, 0), Ok(100));
    assert_eq!(semctl(&mut k, id, 0, GETNCNT, 0), Ok(0));

    // Waiting for zero; removal wakes the waiter with EIDRM.
    k.meanwhile(move |k| {
        k.as_thread(200, |k| {
            assert_eq!(semctl(k, id, 0, GETZCNT, 0), Ok(1));
            assert_eq!(rm(k, id), Ok(0));
        });
    });
    assert_eq!(semop(&mut k, id, &[(0, 0, 0)]), Err(Errno::EIDRM));
}

#[test]
fn semop_wait_is_interrupted_by_a_signal() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 1, 0o600).unwrap();
    k.meanwhile(|k| {
        k.notified.insert(100);
    });
    assert_eq!(semop(&mut k, id, &[(0, -1, 0)]), Err(Errno::EINTR));
    k.notified.clear();
    assert_eq!(semctl(&mut k, id, 0, GETNCNT, 0), Ok(0), "no longer waiting");
}

#[test]
fn stat_needs_a_writable_buffer() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 1, 0o600).unwrap();
    assert_eq!(semctl(&mut k, id, 0, IPC_STAT, 0), Err(Errno::EFAULT));
    assert_eq!(semctl(&mut k, id, 0, GETALL, 0), Err(Errno::EFAULT));
}
