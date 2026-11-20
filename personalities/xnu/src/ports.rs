//! The Mach traps for the `mach_port_*` routines beyond the basic ones and
//! for tasks of other processes (ADR 0238, addendum): `_kernelrpc_mach_port_
//! {insert,extract}_member`, `construct`, `destruct`, `get_attributes`,
//! `guard`, `unguard`, `request_notification`, and task_name_for_pid,
//! task_for_pid and pid_for_task (`specs/macos/mach/*.yaml`,
//! `specs/macos/libsystem_kernel.dylib/mach_port_*.yaml`).
//!
//! The work is `poly-mach`'s; this decodes the arguments, copies user
//! memory, and ends the caller with SIGKILL for a port guard violation.

use crate::kernel::{XnuKernel, unblock_all, with_mach};
use crate::mach::SIGKILL;
use poly_mach::consts::{kern::*, msg::MACH_SEND_INVALID_DEST, port::*, trap::*};
use poly_mach::{Options, ThreadId};

/// Run trap `number`; `None` if it is not one of these.
pub fn trap(k: &mut dyn XnuKernel, number: u64, a: &[u64; 8]) -> Option<u64> {
    let n32 = |i: usize| a[i] as u32;
    let r = match number {
        MACH_PORT_INSERT_MEMBER => on_task(k, n32(0), |m, t| m.ipc.port_insert_member(t, n32(1), n32(2))),
        MACH_PORT_EXTRACT_MEMBER => on_task(k, n32(0), |m, t| m.ipc.port_extract_member(t, n32(1), n32(2))),
        MACH_PORT_CONSTRUCT => construct(k, n32(0), a[1], a[2], a[3]),
        MACH_PORT_DESTRUCT => {
            let kr = on_task_woken(k, n32(0), |m, t| m.ipc.port_destruct(t, n32(1), n32(2) as i32, a[3]));
            guarded(k, kr)
        }
        MACH_PORT_GET_ATTRIBUTES => get_attributes(k, n32(0), n32(1), n32(2), a[3], a[4]),
        MACH_PORT_GUARD => on_task(k, n32(0), |m, t| m.ipc.port_guard(t, n32(1), a[2], n32(3) != 0)),
        MACH_PORT_UNGUARD => {
            let kr = on_task(k, n32(0), |m, t| m.ipc.port_unguard(t, n32(1), a[2]));
            guarded(k, kr)
        }
        MACH_PORT_REQUEST_NOTIFICATION => request_notification(k, a),
        TASK_NAME_FOR_PID => for_pid(k, n32(0), n32(1) as i32, a[2], true),
        TASK_FOR_PID => for_pid(k, n32(0), n32(1) as i32, a[2], false),
        PID_FOR_TASK => pid_for_task(k, n32(0), a[1]),
        _ => return None,
    };
    Some(u64::from(r))
}

/// Resolve the task port name `task` and run `f` on that task.
fn on_task(k: &mut dyn XnuKernel, task: u32, f: impl FnOnce(&mut crate::MachState, u64) -> u32) -> u32 {
    with_mach(k, |m, k| match m.ipc.resolve_task(k.task(), task) {
        Ok(t) => f(m, t),
        Err(kr) => kr,
    })
}

/// [`on_task`] for calls that wake threads.
fn on_task_woken(
    k: &mut dyn XnuKernel,
    task: u32,
    f: impl FnOnce(&mut crate::MachState, u64) -> (u32, alloc::vec::Vec<ThreadId>),
) -> u32 {
    let mut woken = alloc::vec::Vec::new();
    let kr = on_task(k, task, |m, t| {
        let (kr, w) = f(m, t);
        woken = w;
        kr
    });
    unblock_all(k, woken);
    kr
}

/// A guard violation ends the caller with SIGKILL.
pub(crate) fn guarded(k: &mut dyn XnuKernel, kr: u32) -> u32 {
    if kr == POLY_GUARD_VIOLATION {
        k.kill_self(SIGKILL);
        return KERN_INVALID_RIGHT;
    }
    kr
}

fn rd32(k: &dyn XnuKernel, addr: u64) -> Option<u32> {
    let mut b = [0u8; 4];
    k.copy_in(addr, &mut b).ok().map(|()| u32::from_le_bytes(b))
}

/// _kernelrpc_mach_port_construct_trap(task, options, context, name).
fn construct(k: &mut dyn XnuKernel, task: u32, options: u64, context: u64, name_ptr: u64) -> u32 {
    if with_mach(k, |m, k| m.ipc.resolve_task(k.task(), task)).is_err() {
        return MACH_SEND_INVALID_DEST;
    }
    // mach_port_options_t: flags, then mach_port_limits_t.
    let (Some(flags), Some(qlimit)) = (rd32(k, options), rd32(k, options + 4)) else { return KERN_MEMORY_ERROR };
    if rd32(k, name_ptr).is_none() {
        return KERN_MEMORY_ERROR;
    }
    let made = with_mach(k, |m, k| {
        let t = m.ipc.resolve_task(k.task(), task)?;
        m.ipc.port_construct(t, Options { flags, qlimit }, context).map(|n| (t, n))
    });
    let (t, name) = match made {
        Ok(x) => x,
        Err(kr) => return kr,
    };
    if k.copy_out(name_ptr, &name.to_le_bytes()).is_err() {
        let woken = with_mach(k, |m, _| {
            let (_, w) = m.ipc.port_destroy(t, name);
            w
        });
        unblock_all(k, woken);
        return KERN_MEMORY_ERROR;
    }
    KERN_SUCCESS
}

/// _kernelrpc_mach_port_get_attributes_trap(task, name, flavor, info,
/// count): `*count` words of room in, the words given out.
fn get_attributes(k: &mut dyn XnuKernel, task: u32, name: u32, flavor: u32, info: u64, count_ptr: u64) -> u32 {
    let Some(count) = rd32(k, count_ptr) else { return KERN_MEMORY_ERROR };
    let got = with_mach(k, |m, k| {
        let t = m.ipc.resolve_task(k.task(), task)?;
        m.ipc.port_get_attributes(t, name, flavor, count)
    });
    let words = match got {
        Ok(w) => w,
        Err(kr) => return kr,
    };
    let bytes: alloc::vec::Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    if k.copy_out(info, &bytes).is_err() || k.copy_out(count_ptr, &(words.len() as u32).to_le_bytes()).is_err() {
        return KERN_MEMORY_ERROR;
    }
    KERN_SUCCESS
}

/// _kernelrpc_mach_port_request_notification_trap(task, name, msgid,
/// sync, notify, notify disposition, previous).
fn request_notification(k: &mut dyn XnuKernel, a: &[u64; 8]) -> u32 {
    let n32 = |i: usize| a[i] as u32;
    let previous = a[6];
    if previous == 0 {
        return KERN_MEMORY_ERROR;
    }
    let got = with_mach(k, |m, k| {
        let t = m.ipc.resolve_task(k.task(), n32(0))?;
        m.ipc.port_request_notification(t, n32(1), n32(2), n32(3), n32(4), n32(5))
    });
    match got {
        Ok(prev) if k.copy_out(previous, &prev.to_le_bytes()).is_err() => KERN_MEMORY_ERROR,
        Ok(_) => KERN_SUCCESS,
        Err(kr) => kr,
    }
}

/// task_for_pid (`name` false) and task_name_for_pid (`name` true): a send
/// right to the task's port, only for the caller itself, or to its name
/// port, for the caller, its parent and its running children. The first
/// argument must be a task port; on failure the out name is
/// `MACH_PORT_NULL`.
fn for_pid(k: &mut dyn XnuKernel, target: u32, pid: i32, out: u64, name: bool) -> u32 {
    let me = k.task();
    let own = i64::from(pid) == me as i64;
    let allowed = own || name && pid > 1 && k.peer(pid).is_some_and(|p| p.running && p.related);
    let got = with_mach(k, |m, k| {
        if !m.ipc.is_task_port(k.task(), target) || !allowed {
            return None;
        }
        Some(if name { m.ipc.task_name_for(me, pid as u64) } else { m.ipc.task_self(me) })
    });
    let (kr, port) = match got {
        Some(p) => (KERN_SUCCESS, p),
        None => (KERN_FAILURE, MACH_PORT_NULL),
    };
    // A NULL out pointer is no error for a call that works.
    let _ = k.copy_out(out, &port.to_le_bytes());
    kr
}

/// pid_for_task(task, pid): the pid of a task port or name port; for
/// anything else `KERN_FAILURE` and -1.
fn pid_for_task(k: &mut dyn XnuKernel, task: u32, out: u64) -> u32 {
    let got = with_mach(k, |m, k| m.ipc.pid_for_task(k.task(), task));
    let (kr, pid) = match got {
        Ok(t) => (KERN_SUCCESS, t as i32),
        Err(kr) => (kr, -1),
    };
    if k.copy_out(out, &pid.to_le_bytes()).is_err() && kr == KERN_SUCCESS {
        return KERN_MEMORY_ERROR;
    }
    kr
}
