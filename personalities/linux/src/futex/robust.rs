//! What the kernel does for a Linux thread that exits: the robust futex
//! list and `clear_child_tid`.
//!
//! The robust list (`linux/futex.h`): the thread registers a
//! `struct robust_list_head { next; futex_offset; list_op_pending; }`
//! (set_robust_list(2)). Each lock the thread holds is linked into a
//! circular list through `next` ("Points back to itself if empty"); the
//! lock word is at `entry + futex_offset`; `list_op_pending` names a lock
//! being taken or released right now. At exit, for each such word whose
//! TID bits name the thread, the kernel sets `FUTEX_OWNER_DIED` and, if
//! `FUTEX_WAITERS` is set, wakes one waiter ("The kernel also does a
//! FUTEX_WAKE on such futexes, after setting the bit, to wake up any
//! possible waiters"). The walk stops after `ROBUST_LIST_LIMIT` entries
//! ("protects against a deliberately circular list") or at the first
//! entry it cannot read.

use crate::abi::futex::*;
use crate::kernel::{LinuxKernel, UserPtr};
use crate::signal::deliver::with_own;

/// What a robust list walk did (for tests and tracing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RobustWalk {
    /// List entries visited (not counting `list_op_pending`).
    pub visited: usize,
    /// Lock words marked `FUTEX_OWNER_DIED`.
    pub marked: usize,
    /// Waiters woken.
    pub woken: usize,
}

/// Walk the robust list at `head` for exiting thread `tid`.
///
/// Entry pointers are read with bit 0 masked off (entries are
/// pointer-aligned; user space may tag them there). The pending entry is
/// handled after the list, and skipped in the list if it is linked there
/// already. Each word: unaligned or unreadable words are skipped; the
/// owner check and the update are one compare-and-exchange, retried if
/// the word changes under it; the new value keeps `FUTEX_WAITERS`, sets
/// `FUTEX_OWNER_DIED` and clears the TID (to be confirmed by the diff
/// runner).
pub fn walk_robust_list(k: &mut dyn LinuxKernel, head: UserPtr, tid: i32) -> RobustWalk {
    let mut walk = RobustWalk::default();
    let mut raw = [0; ROBUST_LIST_HEAD_SIZE as usize];
    if k.copy_from_user(head, &mut raw).is_err() {
        return walk;
    }
    let word = |o: usize| u64::from_le_bytes(raw[o..o + 8].try_into().unwrap());
    let offset = word(8);
    let pending = word(16) & !1;
    let mut entry = word(0) & !1;
    while entry != head.0 && walk.visited < ROBUST_LIST_LIMIT {
        let mut next = [0; 8];
        if k.copy_from_user(UserPtr(entry), &mut next).is_err() {
            break;
        }
        if entry != pending {
            handle_dead_owner(k, UserPtr(entry.wrapping_add(offset)), tid, &mut walk);
        }
        entry = u64::from_le_bytes(next) & !1;
        walk.visited += 1;
    }
    if pending != 0 {
        handle_dead_owner(k, UserPtr(pending.wrapping_add(offset)), tid, &mut walk);
    }
    walk
}

fn handle_dead_owner(k: &mut dyn LinuxKernel, addr: UserPtr, tid: i32, walk: &mut RobustWalk) {
    if !addr.0.is_multiple_of(4) {
        return;
    }
    let Ok(mut value) = k.futex_load(addr) else { return };
    loop {
        if value & FUTEX_TID_MASK != tid as u32 & FUTEX_TID_MASK {
            return;
        }
        let new = (value & FUTEX_WAITERS) | FUTEX_OWNER_DIED;
        match k.futex_cmpxchg(addr, value, new) {
            Ok(found) if found == value => break,
            Ok(found) => value = found,
            Err(_) => return,
        }
    }
    walk.marked += 1;
    if value & FUTEX_WAITERS != 0 {
        walk.woken += super::kernel_wake(k, addr);
    }
}

/// Called by the kernel when the calling thread exits (exit, exit_group,
/// a fatal signal, execve's end of the other threads), before it stops
/// running user code for good, with `shares_memory` set if another
/// thread or process still uses its address space.
///
/// 1. The robust list is walked ([`walk_robust_list`]).
/// 2. set_tid_address(2): "When a thread whose clear_child_tid is not NULL
///    terminates, then, if the thread is sharing memory with other
///    threads, then 0 is written at the address specified in
///    clear_child_tid and the kernel performs the following operation:
///    futex(clear_child_tid, FUTEX_WAKE, 1, NULL, NULL, 0); ... Errors from
///    the futex wake operation are ignored."
///
/// The order (robust list first) is to be confirmed by the diff runner.
/// Both addresses are forgotten, so a second call does nothing.
pub fn thread_exit(k: &mut dyn LinuxKernel, shares_memory: bool) -> RobustWalk {
    let tid = k.tid();
    let (head, ctid) = with_own(k, |p| {
        if !p.has_thread(tid) {
            return (UserPtr::NULL, UserPtr::NULL);
        }
        let t = p.thread_mut(tid);
        (core::mem::take(&mut t.robust_list), core::mem::take(&mut t.clear_child_tid))
    });
    let walk = if head.is_null() { RobustWalk::default() } else { walk_robust_list(k, head, tid) };
    if shares_memory
        && !ctid.is_null()
        && k.copy_to_user(ctid, &0u32.to_le_bytes()).is_ok()
        && ctid.0.is_multiple_of(4)
    {
        super::kernel_wake(k, ctid);
    }
    walk
}
