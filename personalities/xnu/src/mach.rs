//! The Mach traps (`specs/macos/mach/*.yaml`): ports, Mach VM, the self
//! ports, mach_msg receive, semaphores, time.
//!
//! Each returns its `kern_return_t` (or a port name) for `rax` / `x0`.

use crate::kernel::{Fault, MachState, Wakeup, XnuKernel, unblock_all, with_mach};
use crate::mem::{self, Coverage};
use crate::wait::{after, block_for};
use poly_mach::consts::{kern::*, msg::*, trap::*, vm::*};
use poly_mach::{PortId, Received, Signal, WakeReason};

/// Run Mach trap `number` with arguments `a`; `None` if Poly does not have
/// it.
pub fn trap(k: &mut dyn XnuKernel, number: u64, a: &[u64; 8]) -> Option<u64> {
    let n32 = |i: usize| a[i] as u32;
    let r = match number {
        MACH_VM_ALLOCATE => u64::from(vm_allocate(k, n32(0), a[1], a[2], n32(3))),
        MACH_VM_DEALLOCATE => u64::from(vm_deallocate(k, n32(0), a[1], a[2])),
        MACH_VM_PROTECT => u64::from(vm_protect(k, n32(0), a[1], a[2], a[3] != 0, n32(4))),
        MACH_PORT_ALLOCATE => u64::from(port_allocate(k, n32(0), n32(1), a[2])),
        MACH_PORT_DEALLOCATE => u64::from(on_task(k, n32(0), |m, t, _| m.ipc.port_deallocate(t, n32(1)))),
        MACH_PORT_MOD_REFS => u64::from(port_mod_refs(k, n32(0), n32(1), n32(2), a[3] as i32)),
        MACH_PORT_MOVE_MEMBER => u64::from(on_task(k, n32(0), |m, t, _| m.ipc.port_move_member(t, n32(1), n32(2)))),
        MACH_PORT_INSERT_RIGHT => {
            let caller = k.task();
            u64::from(on_task(k, n32(0), |m, t, _| m.ipc.port_insert_right(caller, t, n32(1), n32(2), n32(3))))
        }
        MACH_PORT_TYPE => u64::from(port_type(k, n32(0), n32(1), a[2])),
        MACH_REPLY_PORT => u64::from(with_mach(k, |m, k| m.ipc.reply_port(k.task()))),
        THREAD_SELF => u64::from(with_mach(k, |m, k| m.ipc.thread_self(k.task(), k.thread()))),
        TASK_SELF => u64::from(with_mach(k, |m, k| m.ipc.task_self(k.task()))),
        HOST_SELF => u64::from(with_mach(k, |m, k| m.ipc.host_self(k.task()))),
        MACH_MSG => u64::from(mach_msg(k, a[0], n32(1), n32(3), n32(4), n32(5))),
        SEMAPHORE_SIGNAL => u64::from(semaphore_signal(k, n32(0), Signal::One)),
        SEMAPHORE_SIGNAL_ALL => u64::from(semaphore_signal(k, n32(0), Signal::All)),
        SEMAPHORE_SIGNAL_THREAD => u64::from(semaphore_signal_thread(k, n32(0), n32(1))),
        SEMAPHORE_WAIT => u64::from(semaphore_wait(k, n32(0), None, None)),
        SEMAPHORE_WAIT_SIGNAL => u64::from(semaphore_wait(k, n32(0), Some(n32(1)), None)),
        SEMAPHORE_TIMEDWAIT => u64::from(semaphore_wait(k, n32(0), None, Some((n32(1), n32(2) as i32)))),
        SEMAPHORE_TIMEDWAIT_SIGNAL => u64::from(semaphore_wait(k, n32(0), Some(n32(1)), Some((n32(2), n32(3) as i32)))),
        MACH_TIMEBASE_INFO => u64::from(timebase_info(k, a[0])),
        MACH_WAIT_UNTIL => u64::from(wait_until(k, a[0])),
        _ => return None,
    };
    Some(r)
}

/// Resolve a task port name and run `f` on that task's space; an invalid
/// task is `MACH_SEND_INVALID_DEST`.
fn on_task(k: &mut dyn XnuKernel, task: u32, f: impl FnOnce(&mut MachState, u64, &mut dyn XnuKernel) -> u32) -> u32 {
    with_mach(k, |m, k| match m.ipc.resolve_task(k.task(), task) {
        Ok(t) => f(m, t, k),
        Err(kr) => kr,
    })
}

/// The Mach VM traps work on the caller's own address space: a task port
/// that names the caller.
fn own_task(k: &mut dyn XnuKernel, task: u32) -> Result<(), u32> {
    let t = with_mach(k, |m, k| m.ipc.resolve_task(k.task(), task))?;
    if t == k.task() { Ok(()) } else { Err(KERN_INVALID_ARGUMENT) }
}

fn port_allocate(k: &mut dyn XnuKernel, task: u32, right: u32, name_ptr: u64) -> u32 {
    let made = with_mach(k, |m, k| {
        let t = m.ipc.resolve_task(k.task(), task)?;
        m.ipc.port_allocate(t, right).map(|n| (t, n))
    });
    let (t, name) = match made {
        Ok(x) => x,
        Err(kr) => return kr,
    };
    if k.copy_out(name_ptr, &name.to_le_bytes()).is_err() {
        // The name cannot be handed back: take the right away again.
        let woken = with_mach(k, |m, _| {
            let (_, w) = m.ipc.port_mod_refs(t, name, right, -1);
            w
        });
        unblock_all(k, woken);
        return KERN_MEMORY_ERROR;
    }
    KERN_SUCCESS
}

fn port_mod_refs(k: &mut dyn XnuKernel, task: u32, name: u32, right: u32, delta: i32) -> u32 {
    let mut woken = alloc::vec::Vec::new();
    let kr = on_task(k, task, |m, t, _| {
        let (kr, w) = m.ipc.port_mod_refs(t, name, right, delta);
        woken = w;
        kr
    });
    unblock_all(k, woken);
    kr
}

fn port_type(k: &mut dyn XnuKernel, task: u32, name: u32, ptr: u64) -> u32 {
    let ty = with_mach(k, |m, k| {
        let t = m.ipc.resolve_task(k.task(), task)?;
        m.ipc.port_type(t, name)
    });
    match ty {
        Ok(ty) if k.copy_out(ptr, &ty.to_le_bytes()).is_err() => KERN_MEMORY_ERROR,
        Ok(_) => KERN_SUCCESS,
        Err(kr) => kr,
    }
}

// ----- Mach VM -----

fn vm_allocate(k: &mut dyn XnuKernel, task: u32, addr_ptr: u64, size: u64, flags: u32) -> u32 {
    if let Err(kr) = own_task(k, task) {
        return kr;
    }
    if flags & !VM_FLAGS_USER_ALLOCATE != 0 {
        return KERN_INVALID_ARGUMENT;
    }
    let mut buf = [0u8; 8];
    if k.copy_in(addr_ptr, &mut buf).is_err() {
        return KERN_MEMORY_ERROR;
    }
    let hint = u64::from_le_bytes(buf);
    let addr = if size == 0 {
        0
    } else {
        let ps = k.page_size();
        let Some(len) = mem::page_up(size, ps) else { return KERN_NO_SPACE };
        let fixed = flags & VM_FLAGS_ANYWHERE == 0;
        if fixed && !hint.is_multiple_of(ps) {
            return KERN_INVALID_ADDRESS;
        }
        match k.map_anon(len, fixed.then_some(hint), flags & VM_FLAGS_OVERWRITE != 0) {
            Ok(a) => a,
            Err(Fault) => return KERN_NO_SPACE,
        }
    };
    if k.copy_out(addr_ptr, &addr.to_le_bytes()).is_err() {
        if addr != 0 {
            let ps = k.page_size();
            k.unmap(addr, addr + mem::page_up(size, ps).unwrap_or(0));
        }
        return KERN_MEMORY_ERROR;
    }
    KERN_SUCCESS
}

fn vm_deallocate(k: &mut dyn XnuKernel, task: u32, addr: u64, size: u64) -> u32 {
    if let Err(kr) = own_task(k, task) {
        return kr;
    }
    if size == 0 {
        return KERN_SUCCESS;
    }
    // The pages the range touches; an unaligned address is not refused.
    let Some((start, end)) = mem::touched(addr, size, k.page_size()) else { return KERN_INVALID_ARGUMENT };
    k.unmap(start, end);
    KERN_SUCCESS
}

fn vm_protect(k: &mut dyn XnuKernel, task: u32, addr: u64, size: u64, set_max: bool, prot: u32) -> u32 {
    if let Err(kr) = own_task(k, task) {
        return kr;
    }
    let ps = k.page_size();
    if !addr.is_multiple_of(ps) {
        return KERN_INVALID_ADDRESS;
    }
    if size == 0 {
        return KERN_SUCCESS;
    }
    let Some((start, end)) = mem::touched(addr, size, ps) else { return KERN_INVALID_ADDRESS };
    if mem::coverage(k, start, end) != Coverage::Full {
        return KERN_INVALID_ADDRESS;
    }
    let prot = prot & VM_PROT_ALL;
    let regions = k.regions(start, end);
    if set_max {
        if regions.iter().any(|r| prot & !r.max_prot != 0) {
            return KERN_PROTECTION_FAILURE;
        }
        k.set_max_prot(start, end, prot);
    } else {
        if regions.iter().any(|r| prot & !r.max_prot != 0) || mem::wx_refused(k, prot) {
            return KERN_PROTECTION_FAILURE;
        }
        k.protect(start, end, prot);
    }
    KERN_SUCCESS
}

// ----- mach_msg -----

/// mach_msg_trap(msg, option, send size, receive size, receive name,
/// timeout, notify). A send option is a guard violation that kills the
/// caller (the sheet: programs send with mach_msg2_trap); a receive takes a
/// message from a port or a set, waiting up to `timeout` ms with
/// `MACH_RCV_TIMEOUT`.
fn mach_msg(k: &mut dyn XnuKernel, msg: u64, option: u32, rcv_size: u32, rcv_name: u32, timeout_ms: u32) -> u32 {
    if option & MACH_SEND_MSG != 0 {
        k.kill_self(9);
        return MACH_SEND_INVALID_DATA;
    }
    if option & MACH_RCV_MSG == 0 {
        return MACH_MSG_SUCCESS;
    }
    let deadline = (option & MACH_RCV_TIMEOUT != 0).then(|| after(k, u64::from(timeout_ms) * 1_000_000));
    loop {
        let got = with_mach(k, |m, k| {
            let (r, woken) = m.ipc.receive(k.task(), rcv_name, rcv_size, option)?;
            if let Received::Empty(key) = r
                && deadline.is_none_or(|d| k.now_ns() < d)
            {
                m.ipc.wait_receive(key, k.thread());
            }
            Ok((r, woken))
        });
        let (r, woken) = match got {
            Ok(x) => x,
            Err(kr) => return kr,
        };
        unblock_all(k, woken);
        match r {
            Received::Message(bytes) => {
                return if k.copy_out(msg, &bytes).is_err() { MACH_RCV_INVALID_DATA } else { MACH_MSG_SUCCESS };
            }
            Received::TooLarge { header, .. } => {
                let _ = k.copy_out(msg, &header);
                return MACH_RCV_TOO_LARGE;
            }
            Received::Empty(_) => {
                if deadline.is_some_and(|d| k.now_ns() >= d) {
                    return MACH_RCV_TIMED_OUT;
                }
                match block_for(k, deadline, |m, t| m.ipc.wait_result(t), |m, t| m.ipc.cancel_waits(t)) {
                    Ok(WakeReason::Ready) => continue,
                    Ok(WakeReason::Died) => return MACH_RCV_PORT_DIED,
                    Err(Wakeup::Signal) => return MACH_RCV_INTERRUPTED,
                    Err(_) => return MACH_RCV_TIMED_OUT,
                }
            }
        }
    }
}

// ----- semaphores -----

fn semaphore_signal(k: &mut dyn XnuKernel, name: u32, how: Signal) -> u32 {
    let r = with_mach(k, |m, k| {
        let p = m.ipc.semaphore(k.task(), name)?;
        m.ipc.semaphore_signal(p, how)
    });
    match r {
        Ok(woken) => {
            unblock_all(k, woken);
            KERN_SUCCESS
        }
        Err(kr) => kr,
    }
}

/// semaphore_signal_thread: the thread name is checked first (anything but
/// 0 or a thread port is `KERN_INVALID_ARGUMENT`), then the semaphore's.
fn semaphore_signal_thread(k: &mut dyn XnuKernel, name: u32, thread: u32) -> u32 {
    let r = with_mach(k, |m, k| {
        let t = m.ipc.thread_of(k.task(), thread)?;
        let p = m.ipc.semaphore(k.task(), name)?;
        m.ipc.semaphore_signal(p, Signal::Thread(t))
    });
    match r {
        Ok(woken) => {
            unblock_all(k, woken);
            KERN_SUCCESS
        }
        Err(kr) => kr,
    }
}

const NSEC_PER_SEC: i64 = 1_000_000_000;

/// semaphore_wait, _wait_signal, _timedwait, _timedwait_signal. Order of the
/// checks (the sheets): the signal name, then the time (nanoseconds below
/// 0 or from one second up are `KERN_INVALID_VALUE`), then the wait name.
/// Nothing is signalled or waited for when any of them is refused.
fn semaphore_wait(k: &mut dyn XnuKernel, wait: u32, signal: Option<u32>, time: Option<(u32, i32)>) -> u32 {
    let checked = with_mach(k, |m, k| -> Result<(PortId, Option<PortId>), u32> {
        let sig = match signal {
            Some(s) => Some(m.ipc.semaphore(k.task(), s)?),
            None => None,
        };
        if let Some((_, ns)) = time
            && !(0..NSEC_PER_SEC).contains(&i64::from(ns))
        {
            return Err(KERN_INVALID_VALUE);
        }
        Ok((m.ipc.semaphore(k.task(), wait)?, sig))
    });
    let (p, sig) = match checked {
        Ok(x) => x,
        Err(kr) => return kr,
    };
    let deadline = time.map(|(s, ns)| after(k, u64::from(s).saturating_mul(1_000_000_000).saturating_add(ns as u64)));
    let started = with_mach(k, |m, k| -> Result<(bool, alloc::vec::Vec<u64>), u32> {
        let woken = match sig {
            Some(s) => m.ipc.semaphore_signal(s, Signal::One)?,
            None => alloc::vec::Vec::new(),
        };
        Ok((m.ipc.semaphore_take(p, k.thread())?, woken))
    });
    let (got, woken) = match started {
        Ok(x) => x,
        Err(kr) => return kr,
    };
    unblock_all(k, woken);
    if got {
        return KERN_SUCCESS;
    }
    let ended = if deadline.is_some_and(|d| k.now_ns() >= d) {
        // A timeout of zero: no wait, unless a signal already came.
        with_mach(k, |m, k| match m.ipc.wait_result(k.thread()) {
            Some(r) => Ok(r),
            None => {
                m.ipc.cancel_waits(k.thread());
                Err(Wakeup::TimedOut)
            }
        })
    } else {
        block_for(k, deadline, |m, t| m.ipc.wait_result(t), |m, t| m.ipc.cancel_waits(t))
    };
    match ended {
        Ok(WakeReason::Ready) => KERN_SUCCESS,
        Ok(WakeReason::Died) => KERN_TERMINATED,
        Err(Wakeup::Signal) => KERN_ABORTED,
        Err(_) => KERN_OPERATION_TIMED_OUT,
    }
}

// ----- time -----

/// mach_timebase_info_trap: fill `{numer, denom}`; a pointer that cannot be
/// written is not reported.
fn timebase_info(k: &mut dyn XnuKernel, ptr: u64) -> u32 {
    let (n, d) = k.arch().timebase();
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&n.to_le_bytes());
    b[4..].copy_from_slice(&d.to_le_bytes());
    let _ = k.copy_out(ptr, &b);
    KERN_SUCCESS
}

/// mach_wait_until_trap: sleep until the deadline in absolute time units.
fn wait_until(k: &mut dyn XnuKernel, deadline: u64) -> u32 {
    let d = k.arch().abs_to_ns(deadline);
    while k.now_ns() < d {
        if k.block(Some(d)) == Wakeup::Signal {
            return KERN_ABORTED;
        }
    }
    KERN_SUCCESS
}
