//! futex(2), set_robust_list(2), get_robust_list(2).

use crate::abi::Timespec;
use crate::abi::futex::*;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::futex::{self, WaitError, WakeOp};
use crate::kernel::{Clock, Deadline, LinuxKernel, Target, UserPtr};
use crate::signal::deliver::{with_own, with_target};
use crate::signal::restart::RestartBlock;

/// futex(uaddr, futex_op, val, timeout | val2, uaddr2, val3)
///
/// The operation is `futex_op` without `FUTEX_PRIVATE_FLAG` (the futex is
/// private to the address space) and `FUTEX_CLOCK_REALTIME` (timeouts on
/// `CLOCK_REALTIME` instead of `CLOCK_MONOTONIC`). futex(2):
///
/// - `FUTEX_WAIT`: block while `*uaddr == val`, `timeout` relative (may be
///   null). `EAGAIN` if the value differs, `ETIMEDOUT`, `EINTR`.
/// - `FUTEX_WAIT_BITSET`: the same with the bitset `val3` (`EINVAL` if 0)
///   and an absolute `timeout`.
/// - `FUTEX_WAKE` / `FUTEX_WAKE_BITSET`: wake up to `val` waiters (whose
///   bitset shares a bit with `val3`); returns how many.
/// - `FUTEX_REQUEUE`: wake up to `val`, move up to `val2` of the rest to
///   `uaddr2`; returns the woken. `FUTEX_CMP_REQUEUE`: only if
///   `*uaddr == val3` (else `EAGAIN`); returns woken plus requeued.
/// - `FUTEX_WAKE_OP`: see [`WakeOp`]; returns the total woken.
/// - The PI operations and `FUTEX_FD`: `ENOSYS` (PI futexes are not
///   implemented; `FUTEX_FD` was removed from Linux in 2.6.26).
///
/// Errors common to all: `EINVAL` if `uaddr` (or `uaddr2`, where used) is
/// not 4-byte aligned, or a timeout is invalid (negative seconds,
/// nanoseconds outside 0-999,999,999); `ENOSYS` for an unknown operation
/// or `FUTEX_CLOCK_REALTIME` with an operation other than the waits
/// ("FUTEX_WAIT, FUTEX_WAIT_BITSET, FUTEX_WAIT_REQUEUE_PI, FUTEX_LOCK_PI2");
/// `EFAULT` for unreadable words or timeouts. Negative counts wake or
/// requeue nobody for `FUTEX_WAKE` and are `EINVAL` for the requeues (to
/// be confirmed by the diff runner, as is the order of the checks).
///
/// Interrupted waits restart after a handler with `SA_RESTART` (signal(7)
/// lists `FUTEX_WAIT` and `FUTEX_WAIT_BITSET` there) and when no handler
/// runs. A `FUTEX_WAIT` with a (relative) timeout is the exception: without
/// a handler it continues through restart_syscall(2) with the time left;
/// with one it fails with `EINTR` ([`Errno::RESTART_WITH_BLOCK`]).
pub fn futex(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let uaddr = a.ptr(0);
    let op = a.int(1) as u32;
    let val = a.uint(2);
    let cmd = op & FUTEX_CMD_MASK;
    let private = op & FUTEX_PRIVATE_FLAG != 0;
    let realtime = op & FUTEX_CLOCK_REALTIME != 0;
    if realtime && !matches!(cmd, FUTEX_WAIT | FUTEX_WAIT_BITSET | FUTEX_WAIT_REQUEUE_PI | FUTEX_LOCK_PI2) {
        return Err(Errno::ENOSYS);
    }
    let clock = if realtime { Clock::Realtime } else { Clock::Monotonic };
    // The fourth argument is a timeout for the waits and a count (val2)
    // for the requeues and FUTEX_WAKE_OP.
    let val2 = a.int(3);
    let uaddr2 = a.ptr(4);
    let val3 = a.uint(5);
    match cmd {
        FUTEX_WAIT | FUTEX_WAIT_BITSET => {
            let bitset = if cmd == FUTEX_WAIT { FUTEX_BITSET_MATCH_ANY } else { val3 };
            if bitset == 0 {
                return Err(Errno::EINVAL);
            }
            aligned(uaddr)?;
            let deadline = match read_timeout(k, a.ptr(3))? {
                None => None,
                Some(t) if cmd == FUTEX_WAIT => Some(Deadline { clock, at: k.clock(clock)?.saturating_add(t) }),
                Some(t) => Some(Deadline { clock, at: t }),
            };
            // A relative timeout keeps counting after a restart.
            let with_block = cmd == FUTEX_WAIT && deadline.is_some();
            wait_with_restart(k, uaddr, private, val, bitset, deadline, with_block)
        }
        FUTEX_WAKE | FUTEX_WAKE_BITSET => {
            let bitset = if cmd == FUTEX_WAKE { FUTEX_BITSET_MATCH_ANY } else { val3 };
            if bitset == 0 {
                return Err(Errno::EINVAL);
            }
            aligned(uaddr)?;
            let key = k.futex_key(uaddr, private)?;
            Ok(futex::wake(k, key, count(val as i32), bitset) as u64)
        }
        FUTEX_REQUEUE | FUTEX_CMP_REQUEUE => {
            aligned(uaddr)?;
            aligned(uaddr2)?;
            if (val as i32) < 0 || val2 < 0 {
                return Err(Errno::EINVAL);
            }
            let from = k.futex_key(uaddr, private)?;
            let to = k.futex_key(uaddr2, private)?;
            let cmp = (cmd == FUTEX_CMP_REQUEUE).then_some(val3);
            let (woken, requeued) = futex::requeue(k, uaddr, from, to, val as usize, val2 as usize, cmp)?;
            Ok(if cmp.is_some() { woken + requeued } else { woken } as u64)
        }
        FUTEX_WAKE_OP => {
            aligned(uaddr)?;
            aligned(uaddr2)?;
            let op = WakeOp::decode(val3)?;
            let key1 = k.futex_key(uaddr, private)?;
            let key2 = k.futex_key(uaddr2, private)?;
            Ok(futex::wake_op(k, key1, uaddr2, key2, count(val as i32), count(val2), op)? as u64)
        }
        _ => Err(Errno::ENOSYS),
    }
}

/// A wake count: negative counts wake nobody.
fn count(n: i32) -> usize {
    n.max(0) as usize
}

fn aligned(addr: UserPtr) -> Result<(), Errno> {
    if addr.0.is_multiple_of(4) { Ok(()) } else { Err(Errno::EINVAL) }
}

/// Read and validate a `struct timespec` timeout; `None` for a null
/// pointer.
fn read_timeout(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<Option<Timespec>, Errno> {
    if ptr.is_null() {
        return Ok(None);
    }
    let mut raw = [0; Timespec::SIZE];
    k.copy_from_user(ptr, &mut raw)?;
    let t = Timespec::from_bytes(&raw);
    if !t.is_valid() {
        return Err(Errno::EINVAL);
    }
    Ok(Some(t))
}

/// Wait, and if a signal interrupts, arrange the restart: through
/// restart_syscall(2) with the deadline saved (`with_block`), or by
/// running the same call again.
pub fn wait_with_restart(
    k: &mut dyn LinuxKernel,
    uaddr: UserPtr,
    private: bool,
    val: u32,
    bitset: u32,
    deadline: Option<Deadline>,
    with_block: bool,
) -> SysResult {
    let key = k.futex_key(uaddr, private)?;
    match futex::wait(k, uaddr, key, val, bitset, deadline) {
        Ok(()) => Ok(0),
        Err(WaitError::Errno(e)) => Err(e),
        Err(WaitError::Interrupted) => match deadline {
            Some(deadline) if with_block => {
                let tid = k.tid();
                let block = RestartBlock::FutexWait { addr: uaddr.0, private, val, bitset, deadline };
                with_own(k, |p| p.thread_mut(tid).restart_block = Some(block));
                Err(Errno::RESTART_WITH_BLOCK)
            }
            _ => Err(Errno::RESTART_IF_SA_RESTART),
        },
    }
}

/// set_robust_list(head, len): record the calling thread's robust list
/// head. set_robust_list(2): `EINVAL` "The len value specified to
/// set_robust_list() doesn't match the size of structure struct
/// robust_list_head expected by kernel" (24 on x86-64). The head is not
/// checked here; a bad one is skipped at exit.
pub fn set_robust_list(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ulong(1) != ROBUST_LIST_HEAD_SIZE {
        return Err(Errno::EINVAL);
    }
    let tid = k.tid();
    let head = a.ptr(0);
    with_own(k, |p| p.thread_mut(tid).robust_list = head);
    Ok(0)
}

/// get_robust_list(pid, head_ptr, len_ptr): the robust list head of thread
/// `pid` (0: the caller) goes to `*head_ptr`, its size (24) to `*len_ptr`.
/// get_robust_list(2): `ESRCH` "The thread with the thread ID pid could not
/// be found", `EPERM` without permission (a ptrace(2) "read, real
/// credentials" check: the same process, or the caller's real uid equal to
/// the target's real, effective and saved uids and its real gid equal to
/// the target's real, effective and saved gids, or effective uid 0 for
/// `CAP_SYS_PTRACE`), `EFAULT` for either pointer. Negative `pid`s name no
/// thread (`ESRCH`; to be confirmed by the diff runner).
pub fn get_robust_list(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let pid = a.int(0);
    let head = if pid == 0 {
        let tid = k.tid();
        with_own(k, |p| p.thread(tid).robust_list)
    } else if pid < 0 {
        return Err(Errno::ESRCH);
    } else {
        let me = k.pid();
        let creds = k.credentials();
        with_target(k, Target::Thread(pid), |p, t| {
            if !p.has_thread(pid) {
                return Err(Errno::ESRCH);
            }
            let same_ids = [t.uid, t.euid, t.suid].iter().all(|&id| id == creds.uid)
                && [t.gid, t.egid, t.sgid].iter().all(|&id| id == creds.gid);
            if t.pid != me && !same_ids && creds.euid != 0 {
                return Err(Errno::EPERM);
            }
            Ok(p.thread(pid).robust_list)
        })??
    };
    k.copy_to_user(a.ptr(1), &head.0.to_le_bytes())?;
    k.copy_to_user(a.ptr(2), &ROBUST_LIST_HEAD_SIZE.to_le_bytes())?;
    Ok(0)
}
