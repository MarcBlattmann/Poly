//! System V semaphores: semget(2), semop(2), semctl(2)
//! (`specs/linux/x86_64/semop.yaml` and its visible tests). The rules of
//! the operations themselves are tested on the table
//! (`crate::ipc::sem`); these tests cover the handlers: arguments, user
//! memory, sleeping, waking and signals.
//!
//! Each test runs in an IPC namespace of its own.

use super::fake::*;
use crate::abi::signal::SIGUSR1;
use crate::ipc::sem::*;
use crate::ipc::shm::{IPC_RMID, IPC_SET, IPC_STAT};
use crate::ns::IpcNs;
use crate::{Errno, Linux, nr};

/// A fake process (pid 100, euid 1001) in an IPC namespace of its own.
fn fresh() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.me().ns.ipc = Some(IpcNs::create(&None));
    k
}

/// Another process `pid` of the same user in the same IPC namespace.
fn other(k: &mut FakeKernel, pid: i32) {
    let ns = k.me().ns.ipc.clone();
    k.add_process(pid, 1001, pid, pid);
    k.proc_state(pid).ns.ipc = ns;
}

const SOPS: u64 = DATA + 0x800;
const BUF: u64 = DATA + 0x1000;

fn semget(k: &mut FakeKernel, nsems: u64) -> u64 {
    sys(k, nr::SEMGET, &[0, nsems, 0o600]).unwrap()
}

fn semop(k: &mut FakeKernel, id: u64, ops: &[(u16, i16, i16)]) -> Result<u64, Errno> {
    let bytes: alloc::vec::Vec<u8> =
        ops.iter().flat_map(|o| [o.0.to_ne_bytes(), o.1.to_ne_bytes(), o.2.to_ne_bytes()].concat()).collect();
    k.poke(SOPS, &bytes);
    sys(k, nr::SEMOP, &[id, SOPS, ops.len() as u64])
}

fn semctl(k: &mut FakeKernel, id: u64, num: u64, cmd: u32, arg: u64) -> Result<u64, Errno> {
    sys(k, nr::SEMCTL, &[id, num, u64::from(cmd), arg])
}

#[test]
fn argument_widths_and_early_errors() {
    // c_semop_bad_id; spec: nsops_zero, too_many, fault, nsops_width.
    let mut k = fresh();
    let id = semget(&mut k, 1);
    k.poke(SOPS, &[0, 0, 1, 0, 0, 0]);
    assert_eq!(sys(&mut k, nr::SEMOP, &[id | 1 << 32, SOPS, 1]), Ok(0), "upper half of semid ignored");
    assert_eq!(sys(&mut k, nr::SEMOP, &[id, SOPS, 1 << 32 | 1]), Ok(0), "upper half of nsops ignored");
    assert_eq!(semctl(&mut k, id, 0, GETVAL, 0), Ok(2));
    assert_eq!(sys(&mut k, nr::SEMOP, &[u64::MAX, 0, 0]), Err(Errno::EINVAL), "nsops 0 first");
    assert_eq!(sys(&mut k, nr::SEMOP, &[id, SOPS, 1 << 32]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SEMOP, &[u64::MAX, 0, 501]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::SEMOP, &[id, SOPS, u64::MAX]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::SEMOP, &[u64::MAX, 0, 1]), Err(Errno::EFAULT), "EFAULT before the id");
    assert_eq!(sys(&mut k, nr::SEMOP, &[id, UNMAPPED, 1]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SEMOP, &[u64::MAX, SOPS, 1]), Err(Errno::EINVAL));
    // 500 operations are fine (waits for zero on a zero semaphore).
    semctl(&mut k, id, 0, SETVAL, 0).unwrap();
    k.poke(SOPS, &[0u8; 500 * 6]);
    assert_eq!(sys(&mut k, nr::SEMOP, &[id, SOPS, 500]), Ok(0));
    assert_eq!(semctl(&mut k, id, 0, IPC_RMID, 0), Ok(0));
    assert_eq!(semop(&mut k, id, &[(0, 0, IPC_NOWAIT)]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SEMOP, &[id, 0, 1]), Err(Errno::EFAULT));
}

#[test]
fn semctl_values_pids_and_stat() {
    // c_semop_sempid_otime, c_semop_efbig (IPC_SET)
    let mut k = fresh();
    let id = semget(&mut k, 3);
    assert_eq!(semctl(&mut k, id, 0, IPC_STAT, BUF), Ok(0));
    assert_eq!(k.peek_u64(BUF + 48), 0, "sem_otime of a new set");
    assert_eq!(k.peek_u64(BUF + 80), 3, "sem_nsems");
    assert_eq!(semctl(&mut k, id, 2, GETPID, 0), Ok(0));
    assert_eq!(semop(&mut k, id, &[(2, 0, 0)]), Ok(0));
    assert_eq!(semctl(&mut k, id, 2, GETPID, 0), Ok(100));
    assert_eq!(semctl(&mut k, id, 3, GETVAL, 0), Err(Errno::EINVAL));
    assert_eq!(semctl(&mut k, id, 1, SETVAL, 32768), Err(Errno::ERANGE));
    assert_eq!(semctl(&mut k, id, 1, SETVAL, 1 << 32 | 9), Ok(0), "the value is an int");
    assert_eq!(semctl(&mut k, id, 1, GETVAL, 0), Ok(9));
    k.poke(BUF, &[1, 0, 2, 0, 3, 0]);
    assert_eq!(semctl(&mut k, id, 0, SETALL, BUF), Ok(0));
    k.poke(BUF, &[0; 6]);
    assert_eq!(semctl(&mut k, id, 0, GETALL, BUF), Ok(0));
    assert_eq!(k.peek(BUF, 6), [1, 0, 2, 0, 3, 0]);

    semctl(&mut k, id, 0, IPC_STAT, BUF).unwrap();
    k.poke(BUF + 20, &0u16.to_ne_bytes());
    assert_eq!(semctl(&mut k, id, 0, IPC_SET, BUF), Ok(0));
    assert_eq!(semop(&mut k, id, &[(3, 1, 0)]), Err(Errno::EFBIG));
    assert_eq!(semop(&mut k, id, &[(0, 1, 0)]), Err(Errno::EACCES));
    assert_eq!(semctl(&mut k, id, 0, GETVAL, 0), Err(Errno::EACCES));
    assert_eq!(semctl(&mut k, id, 0, 99, 0), Err(Errno::EINVAL));
}

#[test]
fn a_sleeper_is_completed_by_the_waker() {
    // c_semop_block_wake
    let mut k = fresh();
    let id = semget(&mut k, 1);
    other(&mut k, 200);
    k.meanwhile(move |k| {
        k.as_thread(200, |k| {
            assert_eq!(semctl(k, id, 0, GETNCNT, 0), Ok(1));
            assert_eq!(semop(k, id, &[(0, 1, 0)]), Ok(0));
            assert_eq!(semctl(k, id, 0, GETVAL, 0), Ok(0), "taken at once");
            assert_eq!(semctl(k, id, 0, GETNCNT, 0), Ok(0));
        });
    });
    assert_eq!(semop(&mut k, id, &[(0, -1, 0)]), Ok(0));
    assert_eq!(semctl(&mut k, id, 0, GETPID, 0), Ok(100), "the sleeper's pid");
}

#[test]
fn a_signal_ends_the_sleep_with_eintr() {
    let mut k = fresh();
    let id = semget(&mut k, 1);
    k.add_thread(101);
    k.meanwhile(move |k| {
        k.as_thread(101, |k| {
            assert_eq!(semctl(k, id, 0, GETNCNT, 0), Ok(1));
            sys(k, nr::TGKILL, &[100, 100, u64::from(SIGUSR1)]).unwrap();
        });
    });
    assert_eq!(semop(&mut k, id, &[(0, -1, 0)]), Err(Errno::EINTR));
    assert_eq!(semctl(&mut k, id, 0, GETNCNT, 0), Ok(0), "left the count");
}

#[test]
fn removal_ends_the_sleep_with_eidrm() {
    let mut k = fresh();
    let id = semget(&mut k, 1);
    other(&mut k, 200);
    k.meanwhile(move |k| {
        k.as_thread(200, |k| assert_eq!(semctl(k, id, 0, IPC_RMID, 0), Ok(0)));
    });
    assert_eq!(semop(&mut k, id, &[(0, 0, 0), (0, -1, 0)]), Err(Errno::EIDRM));
}

#[test]
fn exit_undoes_adjustments() {
    // c_semop_undo_exit
    let mut k = fresh();
    let id = semget(&mut k, 1);
    semctl(&mut k, id, 0, SETVAL, 5).unwrap();
    other(&mut k, 200);
    k.as_thread(200, |k| {
        assert_eq!(semop(k, id, &[(0, -2, SEM_UNDO)]), Ok(0));
        assert_eq!(semop(k, id, &[(0, 3, SEM_UNDO)]), Ok(0));
        assert_eq!(semop(k, id, &[(0, -1, 0)]), Ok(0));
        Linux::process_exit(k);
    });
    assert_eq!(semctl(&mut k, id, 0, GETVAL, 0), Ok(4));
    assert_eq!(semctl(&mut k, id, 0, GETPID, 0), Ok(200));
}
