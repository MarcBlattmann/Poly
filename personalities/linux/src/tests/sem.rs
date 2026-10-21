//! System V semaphores: semget(2), semop(2), semctl(2)
//! (`specs/linux/x86_64/semctl.yaml` and its visible tests).
//!
//! Each test runs in an IPC namespace of its own, so tests running at the
//! same time do not see each other's sets. Other processes share it; a
//! blocked semop(2) lets the steps queued with `meanwhile` run as them.

use super::fake::*;
use crate::ipc::sem::*;
use crate::ns::IpcNs;
use crate::{Errno, Linux, nr};
use alloc::vec::Vec;

/// A fake process in an IPC namespace of its own (euid 1001, egid 2001,
/// no capabilities), and process 300 (uid 1001) in the same namespace.
fn fresh() -> FakeKernel {
    let mut k = FakeKernel::new();
    let ns = IpcNs::create(&None);
    k.me().ns.ipc = Some(ns.clone());
    k.add_process(300, 1001, 300, 300).state.ns.ipc = Some(ns);
    k
}

fn semget(k: &mut FakeKernel, key: i64, nsems: i64, flags: i64) -> Result<i64, Errno> {
    sys(k, nr::SEMGET, &[key as u64, nsems as u64, flags as u64]).map(|id| id as i64)
}

fn ctl(k: &mut FakeKernel, id: i64, num: i64, cmd: i32, arg: u64) -> Result<i64, Errno> {
    sys(k, nr::SEMCTL, &[id as u64, num as u64, cmd as u64, arg]).map(|r| r as i64)
}

const OPS: u64 = DATA + 0x2000;
const BUF: u64 = DATA + 0x1000;
/// The last byte of the fake's data area.
const DATA_END: u64 = DATA + 0x10000;

fn semop(k: &mut FakeKernel, id: i64, ops: &[(u16, i16, i16)]) -> Result<u64, Errno> {
    let bytes: Vec<u8> =
        ops.iter().flat_map(|(n, o, f)| [n.to_ne_bytes(), o.to_ne_bytes(), f.to_ne_bytes()].concat()).collect();
    k.poke(OPS, &bytes);
    sys(k, nr::SEMOP, &[id as u64, OPS, ops.len() as u64])
}

fn stat(k: &mut FakeKernel, id: i64) -> Result<[u8; SemidDs::SIZE], Errno> {
    k.poke(BUF, &[0xa5; 160]);
    ctl(k, id, 0, IPC_STAT, BUF)?;
    assert!(k.peek(BUF + 104, 56).iter().all(|b| *b == 0xa5), "nothing past 104 bytes");
    Ok(k.peek(BUF, SemidDs::SIZE).try_into().unwrap())
}

fn field(b: &[u8], at: usize, len: usize) -> i64 {
    let mut raw = [0u8; 8];
    raw[..len].copy_from_slice(&b[at..at + len]);
    i64::from_le_bytes(raw)
}

fn set_perm(k: &mut FakeKernel, id: i64, uid: u32, gid: u32, mode: u16) -> Result<i64, Errno> {
    let mut b = [0x5au8; SemidDs::SIZE];
    b[4..8].copy_from_slice(&uid.to_ne_bytes());
    b[8..12].copy_from_slice(&gid.to_ne_bytes());
    b[20..22].copy_from_slice(&mode.to_ne_bytes());
    k.poke(BUF, &b);
    ctl(k, id, 0, IPC_SET, BUF)
}

fn put_values(k: &mut FakeKernel, values: &[u16]) {
    let b: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
    k.poke(BUF, &b);
}

#[test]
fn setval_and_getval() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 3, 0o600).unwrap();
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(0));
    assert_eq!(ctl(&mut k, id, 1, SETVAL, 42), Ok(0));
    assert_eq!(ctl(&mut k, id, 1, GETVAL, 0), Ok(42));
    assert_eq!(ctl(&mut k, id, 2, GETVAL, 0), Ok(0));
    assert_eq!(ctl(&mut k, id, 1, SETVAL, 32767), Ok(0));
    for bad in [32768, (-1i64) as u64, 65536, (1 << 32) - 1] {
        assert_eq!(ctl(&mut k, id, 1, SETVAL, bad), Err(Errno::ERANGE), "{bad:#x}");
    }
    assert_eq!(ctl(&mut k, id, 1, GETVAL, 0), Ok(32767));
    for num in [3, -1] {
        assert_eq!(ctl(&mut k, id, num, GETVAL, 0), Err(Errno::EINVAL));
        assert_eq!(ctl(&mut k, id, num, SETVAL, 1), Err(Errno::EINVAL));
    }
    assert_eq!(ctl(&mut k, id, 3, SETVAL, (-1i64) as u64), Err(Errno::ERANGE), "range before semnum");
    assert_eq!(ctl(&mut k, id, 0, IPC_RMID, 0), Ok(0));
    assert_eq!(ctl(&mut k, id, 0, IPC_RMID, 0), Err(Errno::EINVAL));
}

#[test]
fn argument_widths() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 2, 0o600).unwrap();
    let hi = 1i64 << 32;
    assert_eq!(ctl(&mut k, id | hi, 0, SETVAL, 3), Ok(0));
    assert_eq!(ctl(&mut k, id, 1 | hi, SETVAL, 4), Ok(0));
    assert_eq!(sys(&mut k, nr::SEMCTL, &[id as u64, 1, GETVAL as u64 | (1 << 32), 0]), Ok(4));
    assert_eq!(ctl(&mut k, id, 0, SETVAL, (hi + 5) as u64), Ok(0));
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(5));
    assert_eq!(ctl(&mut k, id, 0, SETVAL, 0x8000_0000_0000_0007), Ok(0));
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(7));
    assert_eq!(ctl(&mut k, id, 0, IPC_STAT, BUF | (1 << 47)), Err(Errno::EFAULT));
    assert_eq!(ctl(&mut k, id | hi, 0, IPC_RMID, 0), Ok(0));
}

#[test]
fn bad_commands_and_the_order_of_errors() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 2, 0o600).unwrap();
    for cmd in (4..=10).chain([21, 99, -1, GETVAL | 0x100, IPC_STAT | 0x100]) {
        assert_eq!(ctl(&mut k, id, 0, cmd, BUF), Err(Errno::EINVAL), "{cmd}");
    }
    assert_eq!(ctl(&mut k, -1, 0, IPC_SET, 1), Err(Errno::EINVAL));
    assert_eq!(ctl(&mut k, -1, 0, SETVAL, 40000), Err(Errno::EINVAL));
    assert_eq!(ctl(&mut k, -1, 0, IPC_INFO, BUF), Err(Errno::EINVAL));

    let gone = semget(&mut k, 0, 2, 0o600).unwrap();
    ctl(&mut k, gone, 0, IPC_RMID, 0).unwrap();
    assert_eq!(ctl(&mut k, gone, 0, IPC_SET, 1), Err(Errno::EFAULT));
    for cmd in [IPC_STAT, GETALL, SETALL] {
        assert_eq!(ctl(&mut k, gone, 0, cmd, 1), Err(Errno::EINVAL), "{cmd}");
    }
    assert_eq!(ctl(&mut k, gone, 0, SETVAL, 40000), Err(Errno::ERANGE));

    let z = semget(&mut k, 0, 2, 0).unwrap();
    put_values(&mut k, &[1, 40000]);
    assert_eq!(ctl(&mut k, z, 0, SETVAL, 40000), Err(Errno::ERANGE));
    assert_eq!(ctl(&mut k, z, 9, SETVAL, 1), Err(Errno::EINVAL));
    assert_eq!(ctl(&mut k, z, 9, GETVAL, 0), Err(Errno::EACCES));
    assert_eq!(ctl(&mut k, z, 0, SETALL, 1), Err(Errno::EACCES));
    assert_eq!(ctl(&mut k, z, 0, SETALL, BUF), Err(Errno::EACCES));
    assert_eq!(ctl(&mut k, z, 0, GETALL, 1), Err(Errno::EACCES));
    assert_eq!(ctl(&mut k, z, 0, IPC_STAT, 1), Err(Errno::EACCES));
    assert_eq!(ctl(&mut k, id, 0, IPC_RMID, 0), Ok(0));
    assert_eq!(ctl(&mut k, z, 0, IPC_RMID, 0), Ok(0), "the owner needs no mode bits");
}

#[test]
fn ipc_stat_layout() {
    let mut k = fresh();
    let key = 0x5100_1234;
    let id = semget(&mut k, key, 5, 0o640 | 0o1000 | 0o2000).unwrap();
    let b = stat(&mut k, id).unwrap();
    assert_eq!(field(&b, 0, 4), key);
    assert_eq!((field(&b, 4, 4), field(&b, 8, 4), field(&b, 12, 4), field(&b, 16, 4)), (1001, 2001, 1001, 2001));
    assert_eq!(field(&b, 20, 2), 0o640);
    assert_eq!(field(&b, 48, 8), 0, "otime");
    assert_eq!(field(&b, 64, 8), k.realtime.sec);
    assert_eq!(field(&b, 80, 8), 5);
    for (at, len) in [(22, 2), (26, 2), (28, 4), (32, 8), (40, 8), (56, 8), (72, 8), (88, 8), (96, 8)] {
        assert_eq!(field(&b, at, len), 0, "pad at {at}");
    }
    assert_eq!(ctl(&mut k, id, -9, IPC_STAT, BUF), Ok(0), "semnum ignored");
    ctl(&mut k, id, 1, SETVAL, 3).unwrap();
    assert_eq!(field(&stat(&mut k, id).unwrap(), 48, 8), 0, "semctl never sets otime");
    let p = semget(&mut k, 0, 1, 0o600).unwrap();
    assert_eq!(field(&stat(&mut k, p).unwrap(), 0, 4), 0);
}

#[test]
fn ipc_set() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 2, 0o600).unwrap();
    let before = stat(&mut k, id).unwrap();
    k.realtime.sec += 2;
    assert_eq!(set_perm(&mut k, id, 1001, 2001, 0o7640), Ok(0));
    let ds = stat(&mut k, id).unwrap();
    assert_eq!(field(&ds, 20, 2), 0o640);
    assert_eq!(field(&ds, 64, 8), k.realtime.sec);
    assert_eq!(field(&ds, 24, 2), field(&before, 24, 2), "seq unchanged");
    assert_eq!(field(&ds, 80, 8), 2);
    assert_eq!(set_perm(&mut k, id, 12345, 54321, 0o660), Ok(0));
    let ds = stat(&mut k, id).unwrap();
    assert_eq!((field(&ds, 4, 4), field(&ds, 8, 4), field(&ds, 12, 4)), (12345, 54321, 1001));
    assert_eq!(ctl(&mut k, id, 0, SETVAL, 4), Ok(0), "the creator is still in the owner class");
    assert_eq!(set_perm(&mut k, id, u32::MAX, 2001, 0o600), Err(Errno::EINVAL));
    assert_eq!(set_perm(&mut k, id, 1001, u32::MAX, 0o600), Err(Errno::EINVAL));
    assert_eq!(field(&stat(&mut k, id).unwrap(), 20, 2), 0o660);
    // Someone else is neither owner nor creator.
    let other = k.as_thread(300, |k| {
        k.creds_mut().euid = 4000;
        set_perm(k, id, 4000, 4000, 0o666)
    });
    assert_eq!(other, Err(Errno::EPERM));
    assert_eq!(k.as_thread(300, |k| ctl(k, id, 0, IPC_RMID, 0)), Err(Errno::EPERM));
}

#[test]
fn info_and_sem_stat() {
    let mut k = fresh();
    let a = semget(&mut k, 0, 3, 0o640).unwrap();
    let z = semget(&mut k, 0, 2, 0).unwrap();
    let max = ctl(&mut k, 0, 0, SEM_INFO, BUF).unwrap();
    assert_eq!(max, z % 32768);
    let info = k.peek(BUF, 40);
    assert_eq!((field(&info, 28, 4), field(&info, 36, 4)), (2, 5), "sets and semaphores in use");
    assert_eq!(ctl(&mut k, a, 0, IPC_INFO, BUF), Ok(max), "semid ignored");
    let info = k.peek(BUF, 40);
    let fields: Vec<i64> = (0..10).map(|i| field(&info, i * 4, 4)).collect();
    assert_eq!(fields, [1_024_000_000, 32000, 1_024_000_000, 1_024_000_000, 32000, 500, 500, 20, 32767, 32767]);
    assert_eq!(ctl(&mut k, a % 32768, 0, SEM_STAT, BUF), Ok(a));
    assert_eq!(ctl(&mut k, a, 0, SEM_STAT, BUF), Ok(a), "the id is an index too");
    assert_eq!(ctl(&mut k, z % 32768, 0, SEM_STAT, BUF), Err(Errno::EACCES));
    assert_eq!(ctl(&mut k, z % 32768, 0, SEM_STAT_ANY, BUF), Ok(z));
    assert_eq!(field(&k.peek(BUF, 104), 80, 8), 2);
    assert_eq!(ctl(&mut k, a % 32768, 0, SEM_STAT, 1), Err(Errno::EFAULT));
    for cmd in [IPC_INFO, SEM_INFO] {
        assert_eq!(ctl(&mut k, 0, 0, cmd, 0), Err(Errno::EFAULT));
    }
    ctl(&mut k, a, 0, IPC_RMID, 0).unwrap();
    assert_eq!(ctl(&mut k, a % 32768, 0, SEM_STAT_ANY, BUF), Err(Errno::EINVAL));
}

#[test]
fn setall_getall_and_bad_pointers() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 3, 0o600).unwrap();
    k.poke(BUF, &[9; 8]);
    assert_eq!(ctl(&mut k, id, 0, GETALL, BUF), Ok(0));
    assert_eq!(k.peek(BUF, 8), [0, 0, 0, 0, 0, 0, 9, 9]);
    put_values(&mut k, &[1, 2, 32767]);
    assert_eq!(ctl(&mut k, id, 77, SETALL, BUF), Ok(0));
    assert_eq!(ctl(&mut k, id, 2, GETVAL, 0), Ok(32767));
    assert_eq!(ctl(&mut k, id, 2, GETPID, 0), Ok(100));
    for bad in [[10, 11, 32768], [65535, 11, 12]] {
        put_values(&mut k, &bad);
        assert_eq!(ctl(&mut k, id, 0, SETALL, BUF), Err(Errno::ERANGE));
    }
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(1));
    for p in [0, 1] {
        for cmd in [IPC_STAT, IPC_SET, GETALL, SETALL] {
            assert_eq!(ctl(&mut k, id, 0, cmd, p), Err(Errno::EFAULT), "{cmd} {p}");
        }
    }
    let edge = DATA_END - 2;
    k.poke(edge, &9u16.to_ne_bytes());
    assert_eq!(ctl(&mut k, id, 0, SETALL, edge), Err(Errno::EFAULT));
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(1));
    assert_eq!(ctl(&mut k, id, 0, GETALL, edge), Err(Errno::EFAULT));

    let big = semget(&mut k, 0, 400, 0o600).unwrap();
    let values: Vec<u16> = (0..400).map(|i| i * 7).collect();
    put_values(&mut k, &values);
    assert_eq!(ctl(&mut k, big, 0, SETALL, BUF), Ok(0));
    assert_eq!(ctl(&mut k, big, 399, GETVAL, 0), Ok(399 * 7));
}

#[test]
fn getpid_names_the_last_changer() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 3, 0o600).unwrap();
    assert_eq!(ctl(&mut k, id, 0, GETPID, 0), Ok(0));
    ctl(&mut k, id, 1, SETVAL, 5).unwrap();
    assert_eq!(ctl(&mut k, id, 1, GETPID, 0), Ok(100));
    assert_eq!(ctl(&mut k, id, 0, GETPID, 0), Ok(0));
    k.as_thread(300, |k| {
        ctl(k, id, 1, GETVAL, 0).unwrap();
        ctl(k, id, 2, SETVAL, 6).unwrap();
        ctl(k, id, 1, SETVAL, 40000).unwrap_err();
    });
    assert_eq!(ctl(&mut k, id, 2, GETPID, 0), Ok(300));
    assert_eq!(ctl(&mut k, id, 1, GETPID, 0), Ok(100));
    assert_eq!(k.as_thread(300, |k| semop(k, id, &[(0, 1, 0), (1, 0, IPC_NOWAIT)])), Err(Errno::EAGAIN));
    assert_eq!(k.as_thread(300, |k| semop(k, id, &[(0, 1, 0), (1, -5, 0)])), Ok(0));
    assert_eq!((ctl(&mut k, id, 0, GETPID, 0), ctl(&mut k, id, 1, GETPID, 0)), (Ok(300), Ok(300)));
    assert_eq!(field(&stat(&mut k, id).unwrap(), 48, 8), k.realtime.sec, "semop sets otime");
}

#[test]
fn waiters_are_counted_and_woken_by_setval() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 2, 0o600).unwrap();
    // Process 300 waits to take 1 from semaphore 0; meanwhile process 100
    // sees it counted and sets the value.
    k.meanwhile(move |k| {
        k.as_thread(100, |k| {
            assert_eq!(ctl(k, id, 0, GETNCNT, 0), Ok(1));
            assert_eq!(ctl(k, id, 0, GETZCNT, 0), Ok(0));
            assert_eq!(ctl(k, id, 0, SETVAL, 1), Ok(0));
        })
    });
    assert_eq!(k.as_thread(300, |k| semop(k, id, &[(0, -1, 0)])), Ok(0));
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(0));
    assert_eq!(ctl(&mut k, id, 0, GETPID, 0), Ok(300), "the woken waiter changed it last");
    assert_eq!(ctl(&mut k, id, 0, GETNCNT, 0), Ok(0));

    // A wait for zero; SETVAL 2 is not enough for a -3, SETVAL 3 is.
    ctl(&mut k, id, 1, SETVAL, 1).unwrap();
    k.meanwhile(move |k| {
        k.as_thread(100, |k| {
            assert_eq!(ctl(k, id, 1, GETZCNT, 0), Ok(1));
            ctl(k, id, 1, SETVAL, 0).unwrap();
        })
    });
    assert_eq!(k.as_thread(300, |k| semop(k, id, &[(1, 0, 0)])), Ok(0));
    k.meanwhile(move |k| k.as_thread(100, |k| ctl(k, id, 0, SETVAL, 2).map(drop).unwrap()));
    k.meanwhile(move |k| {
        k.as_thread(100, |k| {
            assert_eq!(ctl(k, id, 0, GETNCNT, 0), Ok(1), "still blocked");
            ctl(k, id, 0, SETVAL, 3).unwrap();
        })
    });
    assert_eq!(k.as_thread(300, |k| semop(k, id, &[(0, -3, 0)])), Ok(0));
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(0));
}

#[test]
fn removal_wakes_waiters_with_eidrm_and_signals_end_the_wait() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 1, 0o600).unwrap();
    k.meanwhile(move |k| k.as_thread(100, |k| ctl(k, id, 0, IPC_RMID, 0).map(drop).unwrap()));
    assert_eq!(k.as_thread(300, |k| semop(k, id, &[(0, -1, 0)])), Err(Errno::EIDRM));
    assert_eq!(k.as_thread(300, |k| semop(k, id, &[(0, -1, 0)])), Err(Errno::EINVAL));

    let id = semget(&mut k, 0, 1, 0o600).unwrap();
    k.notified.insert(300);
    assert!(k.as_thread(300, |k| semop(k, id, &[(0, -1, 0)])).is_err());
    k.notified.remove(&300);
    assert_eq!(ctl(&mut k, id, 0, GETNCNT, 0), Ok(0), "no longer waiting");
}

#[test]
fn setval_and_setall_clear_undo_adjustments() {
    let mut k = fresh();
    let id = semget(&mut k, 0, 2, 0o600).unwrap();
    let exit_300 = |k: &mut FakeKernel| k.as_thread(300, |k| Linux::process_exit(k));
    ctl(&mut k, id, 0, SETVAL, 1).unwrap();
    k.as_thread(300, |k| semop(k, id, &[(0, 2, SEM_UNDO)])).unwrap();
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(3));
    ctl(&mut k, id, 0, SETVAL, 10).unwrap();
    exit_300(&mut k);
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(10));

    k.as_thread(300, |k| semop(k, id, &[(0, 2, SEM_UNDO)])).unwrap();
    exit_300(&mut k);
    assert_eq!(ctl(&mut k, id, 0, GETVAL, 0), Ok(10), "the exit undid the raise");

    k.as_thread(300, |k| semop(k, id, &[(1, 3, SEM_UNDO)])).unwrap();
    put_values(&mut k, &[7, 8]);
    ctl(&mut k, id, 0, SETALL, BUF).unwrap();
    exit_300(&mut k);
    assert_eq!(ctl(&mut k, id, 1, GETVAL, 0), Ok(8));

    k.as_thread(300, |k| semop(k, id, &[(0, 1, SEM_UNDO), (1, 1, SEM_UNDO)])).unwrap();
    ctl(&mut k, id, 0, SETVAL, 20).unwrap();
    exit_300(&mut k);
    assert_eq!((ctl(&mut k, id, 0, GETVAL, 0), ctl(&mut k, id, 1, GETVAL, 0)), (Ok(20), Ok(8)));
}

#[test]
fn semget_and_semop_argument_checks() {
    let mut k = fresh();
    assert_eq!(semget(&mut k, 0x77, 1, 0o600), Err(Errno::ENOENT));
    assert_eq!(semget(&mut k, 0, 0, 0o600), Err(Errno::EINVAL));
    let id = semget(&mut k, 0x77, 2, 0o600 | 0o1000).unwrap();
    assert_eq!(semget(&mut k, 0x77, 2, 0), Ok(id));
    assert_eq!(semget(&mut k, 0x77, 2, 0o1000 | 0o2000), Err(Errno::EEXIST));
    assert_eq!(sys(&mut k, nr::SEMOP, &[id as u64, OPS, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SEMOP, &[id as u64, OPS, 501]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::SEMOP, &[id as u64, 1, 1]), Err(Errno::EFAULT));
    assert_eq!(semop(&mut k, id, &[(2, 1, 0)]), Err(Errno::EFBIG));
    // semtimedop(2): a wait that times out is EAGAIN and leaves no waiter.
    let mut timeout = [0u8; 16];
    timeout[8] = 1;
    k.poke(BUF, &timeout);
    k.poke(OPS, &[0, 0, 0xff, 0xff, 0, 0]);
    assert_eq!(sys(&mut k, nr::SEMTIMEDOP, &[id as u64, OPS, 1, BUF]), Err(Errno::EAGAIN));
    assert_eq!(ctl(&mut k, id, 0, GETNCNT, 0), Ok(0));
    timeout[8] = 0;
    timeout[15] = 0x80;
    k.poke(BUF, &timeout);
    assert_eq!(sys(&mut k, nr::SEMTIMEDOP, &[id as u64, OPS, 1, BUF]), Err(Errno::EINVAL), "a bad timeout");
}
