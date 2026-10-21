//! Futexes: waiting on and waking user-space words (futex(2), futex(7)),
//! robust futex lists (set_robust_list(2), `linux/futex.h`) and the
//! thread-exit duties (`clear_child_tid`, set_tid_address(2)).
//!
//! The wait queues are [`FutexTable`] (one per system, owned and locked by
//! the kernel). Waiting is "compare and block": with the table locked, the
//! word is read ([`LinuxKernel::futex_load`]), compared and the thread
//! queued; then the table is unlocked and the thread blocks
//! ([`LinuxKernel::block`]). A waker locks the table, dequeues waiters and
//! unblocks them. Because `unblock` before `block` is not lost, a wake
//! between the unlock and the block is not lost either, and futex(2)'s
//! promise holds: "the loading of the futex word's value, the comparison
//! of that value with the expected value, and the actual blocking will
//! happen atomically and will be totally ordered with respect to
//! concurrent operations performed by other threads on the same futex
//! word".
//!
//! Priority-inheritance futexes (`FUTEX_LOCK_PI` and friends) are not
//! implemented and return `ENOSYS`, which futex(2) documents for kernels
//! without PI support; C libraries then fall back or report the error for
//! PI mutexes only.

pub mod robust;
pub mod table;

pub use robust::thread_exit;
pub use table::{FutexKey, FutexTable, Waiter};

use crate::abi::futex::*;
use crate::errno::Errno;
use crate::kernel::{Deadline, LinuxKernel, UserPtr, Wakeup};
use alloc::vec::Vec;

/// Run `f` with the futex table locked and return its result.
pub fn with_futexes<R>(k: &mut dyn LinuxKernel, f: impl FnOnce(&mut FutexTable, &mut dyn LinuxKernel) -> R) -> R {
    let mut f = Some(f);
    let mut out = None;
    k.with_futexes(&mut |t, k| {
        if let Some(f) = f.take() {
            out = Some(f(t, k));
        }
    });
    out.expect("with_futexes ran the closure")
}

/// The key of the 4-byte aligned futex word at `addr` for an operation
/// with (`private`) or without `FUTEX_PRIVATE_FLAG`: the kernel's
/// [`LinuxKernel::futex_key`], with a shared futex in private memory made
/// [`FutexKey::Unshared`] so it never meets the private futex at the same
/// address. `EFAULT` if the word cannot be read (futex's spec sheet:
/// "EFAULT for an unmapped uaddr or uaddr2").
pub fn key(k: &mut dyn LinuxKernel, addr: UserPtr, private: bool) -> Result<FutexKey, Errno> {
    k.futex_load(addr)?;
    Ok(match k.futex_key(addr, private)? {
        FutexKey::Private { space, addr } if !private => FutexKey::Unshared { space, addr },
        key => key,
    })
}

/// The kernel's own `FUTEX_WAKE` of one waiter on the word at `addr`
/// (`clear_child_tid`, a robust lock whose owner died): a shared wake, as
/// set_tid_address(2) has it; when no shared waiter is there, a private
/// one is taken instead, so a thread library that waits on the word with
/// `FUTEX_PRIVATE_FLAG` is not left waiting. Errors are ignored.
pub fn kernel_wake(k: &mut dyn LinuxKernel, addr: UserPtr) -> usize {
    let mut n = key(k, addr, false).map_or(0, |key| wake(k, key, 1, FUTEX_BITSET_MATCH_ANY));
    if n == 0 {
        n = key(k, addr, true).map_or(0, |key| wake(k, key, 1, FUTEX_BITSET_MATCH_ANY));
    }
    n
}

/// How a wait ended, short of a wake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitError {
    /// An error to return as is (`EAGAIN`, `EFAULT`, `ETIMEDOUT`).
    Errno(Errno),
    /// A signal arrived; the caller picks the restart code.
    Interrupted,
}

impl From<Errno> for WaitError {
    fn from(e: Errno) -> Self {
        WaitError::Errno(e)
    }
}

/// Wait on the word at `addr` (key `key`) while it holds `val`, until a
/// wake that matches `bitset`, `deadline`, or a signal. `EAGAIN` if the word
/// holds another value, `EFAULT` if it cannot be read, `ETIMEDOUT` at the
/// deadline. A wake that races with a timeout or a signal wins: the wait
/// then returns 0.
pub fn wait(
    k: &mut dyn LinuxKernel,
    addr: UserPtr,
    key: FutexKey,
    val: u32,
    bitset: u32,
    deadline: Option<Deadline>,
) -> Result<(), WaitError> {
    let tid = k.tid();
    // Touch the word unlocked first, so the locked read need not fault.
    k.futex_load(addr)?;
    with_futexes(k, |t, k| match k.futex_load(addr) {
        Err(e) => Err(e),
        Ok(v) if v != val => Err(Errno::EAGAIN),
        Ok(_) => {
            let queued = t.enqueue(key, tid, bitset);
            debug_assert!(queued, "a thread waits on one futex at a time");
            Ok(())
        }
    })?;
    loop {
        let why = k.block(deadline);
        let still_queued = with_futexes(k, |t, _| match why {
            Wakeup::Woken => t.key_of(tid).is_some(),
            Wakeup::TimedOut | Wakeup::Signal => t.remove(tid),
        });
        if !still_queued {
            return Ok(());
        }
        match why {
            Wakeup::Woken => {} // an old unblock: wait on
            Wakeup::TimedOut => return Err(Errno::ETIMEDOUT.into()),
            Wakeup::Signal => return Err(WaitError::Interrupted),
        }
    }
}

/// Wake up to `max` waiters on `key` that match `bitset`; returns how many.
pub fn wake(k: &mut dyn LinuxKernel, key: FutexKey, max: usize, bitset: u32) -> usize {
    let mut woken = Vec::new();
    let n = with_futexes(k, |t, _| t.wake(key, max, bitset, &mut |w| woken.push(w)));
    for w in woken {
        k.unblock(w);
    }
    n
}

/// Wake up to `max_wake` waiters on `from` and move up to `max_requeue`
/// of the others to `to`; with `cmp`, only if the word at `addr` still
/// holds it (`EAGAIN` otherwise). Returns `(woken, requeued)`.
pub fn requeue(
    k: &mut dyn LinuxKernel,
    addr: UserPtr,
    from: FutexKey,
    to: FutexKey,
    max_wake: usize,
    max_requeue: usize,
    cmp: Option<u32>,
) -> Result<(usize, usize), Errno> {
    if cmp.is_some() {
        k.futex_load(addr)?;
    }
    let mut woken = Vec::new();
    let r = with_futexes(k, |t, k| {
        if let Some(expected) = cmp
            && k.futex_load(addr)? != expected
        {
            return Err(Errno::EAGAIN);
        }
        Ok(t.requeue(from, to, max_wake, max_requeue, &mut |w| woken.push(w)))
    });
    for w in woken {
        k.unblock(w);
    }
    r
}

/// The operation encoded in `FUTEX_WAKE_OP`'s `val3` (futex(2)):
///
/// ```text
/// +---+---+-----------+-----------+
/// |op |cmp|   oparg   |  cmparg   |
/// +---+---+-----------+-----------+
///   4   4       12          12    <== # of bits
/// ```
///
/// `oparg` and `cmparg` are taken as signed 12-bit values (to be confirmed
/// by the diff runner); with `FUTEX_OP_OPARG_SHIFT` the operand is
/// `1 << oparg` (`oparg` taken modulo 32). Comparisons are signed, on the
/// old value as an `int`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WakeOp {
    pub op: u32,
    pub cmp: u32,
    pub operand: u32,
    pub cmparg: i32,
}

impl WakeOp {
    /// Decode `val3`. `ENOSYS` for an unknown operation or comparison
    /// (futex(2): "ENOSYS Invalid operation specified in futex_op"; to be
    /// confirmed by the diff runner).
    pub fn decode(val3: u32) -> Result<WakeOp, Errno> {
        let field = val3 >> 28;
        let op = field & !FUTEX_OP_OPARG_SHIFT;
        let cmp = (val3 >> 24) & 0xf;
        let sext12 = |v: u32| ((v << 20) as i32) >> 20;
        let oparg = sext12((val3 >> 12) & 0xfff);
        let cmparg = sext12(val3 & 0xfff);
        if op > FUTEX_OP_XOR || cmp > FUTEX_OP_CMP_GE {
            return Err(Errno::ENOSYS);
        }
        let operand = if field & FUTEX_OP_OPARG_SHIFT != 0 { 1u32 << (oparg as u32 & 31) } else { oparg as u32 };
        Ok(WakeOp { op, cmp, operand, cmparg })
    }

    /// The new value of the word that held `old`.
    pub const fn apply(&self, old: u32) -> u32 {
        match self.op {
            FUTEX_OP_SET => self.operand,
            FUTEX_OP_ADD => old.wrapping_add(self.operand),
            FUTEX_OP_OR => old | self.operand,
            FUTEX_OP_ANDN => old & !self.operand,
            _ => old ^ self.operand,
        }
    }

    /// Whether `old` passes the comparison (so `uaddr2` is woken too).
    pub const fn compare(&self, old: u32) -> bool {
        let old = old as i32;
        match self.cmp {
            FUTEX_OP_CMP_EQ => old == self.cmparg,
            FUTEX_OP_CMP_NE => old != self.cmparg,
            FUTEX_OP_CMP_LT => old < self.cmparg,
            FUTEX_OP_CMP_LE => old <= self.cmparg,
            FUTEX_OP_CMP_GT => old > self.cmparg,
            _ => old >= self.cmparg,
        }
    }
}

/// Atomically apply `op` to the word at `addr` (a compare-and-exchange
/// loop) and return the old value.
pub fn fetch_op(k: &mut dyn LinuxKernel, addr: UserPtr, op: &WakeOp) -> Result<u32, Errno> {
    let mut old = k.futex_load(addr)?;
    loop {
        let found = k.futex_cmpxchg(addr, old, op.apply(old))?;
        if found == old {
            return Ok(old);
        }
        old = found;
    }
}

/// `FUTEX_WAKE_OP`: change the word at `addr2` with `op`, wake up to
/// `max1` waiters on `key1`, and if the old value passes the comparison up
/// to `max2` waiters on `key2`. Returns the total woken.
pub fn wake_op(
    k: &mut dyn LinuxKernel,
    key1: FutexKey,
    addr2: UserPtr,
    key2: FutexKey,
    max1: usize,
    max2: usize,
    op: WakeOp,
) -> Result<usize, Errno> {
    k.futex_load(addr2)?;
    let mut woken = Vec::new();
    let r = with_futexes(k, |t, k| {
        let old = fetch_op(k, addr2, &op)?;
        let mut n = t.wake(key1, max1, FUTEX_BITSET_MATCH_ANY, &mut |w| woken.push(w));
        if op.compare(old) {
            n += t.wake(key2, max2, FUTEX_BITSET_MATCH_ANY, &mut |w| woken.push(w));
        }
        Ok(n)
    });
    for w in woken {
        k.unblock(w);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// futex(2)'s `FUTEX_OP` macro.
    const fn futex_op(op: u32, oparg: u32, cmp: u32, cmparg: u32) -> u32 {
        ((op & 0xf) << 28) | ((cmp & 0xf) << 24) | ((oparg & 0xfff) << 12) | (cmparg & 0xfff)
    }

    #[test]
    fn wake_op_encoding() {
        let op = WakeOp::decode(futex_op(FUTEX_OP_ADD, 5, FUTEX_OP_CMP_GT, 1)).unwrap();
        assert_eq!(op, WakeOp { op: FUTEX_OP_ADD, cmp: FUTEX_OP_CMP_GT, operand: 5, cmparg: 1 });
        assert_eq!(op.apply(10), 15);
        assert!(op.compare(2) && !op.compare(1) && !op.compare(u32::MAX), "signed: -1 > 1 is false");
        let set = WakeOp::decode(futex_op(FUTEX_OP_SET, 0, FUTEX_OP_CMP_EQ, 0)).unwrap();
        assert_eq!(set.apply(77), 0);
        assert!(set.compare(0) && !set.compare(1));
        let shift = WakeOp::decode(futex_op(FUTEX_OP_OR | FUTEX_OP_OPARG_SHIFT, 4, FUTEX_OP_CMP_NE, 0)).unwrap();
        assert_eq!(shift.apply(1), 0x11);
        let andn = WakeOp::decode(futex_op(FUTEX_OP_ANDN, 0b110, FUTEX_OP_CMP_LE, 0)).unwrap();
        assert_eq!(andn.apply(0b111), 0b001);
        let xor = WakeOp::decode(futex_op(FUTEX_OP_XOR, 0xfff, FUTEX_OP_CMP_GE, 0xfff)).unwrap();
        assert_eq!(xor.operand, u32::MAX, "oparg -1 sign-extended");
        assert_eq!(xor.cmparg, -1);
        assert_eq!(xor.apply(0), u32::MAX);
        assert!(xor.compare(0) && xor.compare(u32::MAX));
        let lt = WakeOp::decode(futex_op(FUTEX_OP_SET, 1, FUTEX_OP_CMP_LT, 0x800)).unwrap();
        assert_eq!(lt.cmparg, -2048);
        assert!(!lt.compare(0) && lt.compare(-3000i32 as u32));
        assert_eq!(WakeOp::decode(futex_op(5, 0, 0, 0)), Err(Errno::ENOSYS));
        assert_eq!(WakeOp::decode(futex_op(0, 0, 6, 0)), Err(Errno::ENOSYS));
    }
}
