//! seccomp(2) and prctl(2)'s `PR_SET_SECCOMP` / `PR_GET_SECCOMP`: install
//! strict mode or a filter on the calling thread, ask which actions are
//! supported. The rules are in [`crate::seccomp`] (ADR 0054).
//!
//! The order of the checks of `SECCOMP_SET_MODE_FILTER` is the spec
//! sheet's: the operation and flags (`EINVAL`), reading and checking the
//! program (`EFAULT`, `EINVAL`), the privilege (`EACCES`), a listener
//! already there (`EBUSY`), the thread's mode (`EINVAL`), the path length
//! (`ENOMEM`), the other threads (`TSYNC`).

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr, Wakeup};
use crate::seccomp::abi::*;
use crate::seccomp::{self, Action, Installed};
use crate::table::{Handler, nr};
use alloc::sync::Arc;

/// The calls of this module, registered with the table's groups.
pub const CALLS: &[(u64, Handler)] = &[(nr::SECCOMP, seccomp)];

/// seccomp(operation, flags, args).
pub fn seccomp(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(1);
    let args = a.ptr(2);
    match a.uint(0) {
        SECCOMP_SET_MODE_STRICT => {
            // seccomp(2): "the value of flags must be 0 and args must be NULL".
            if flags != 0 || !args.is_null() {
                return Err(Errno::EINVAL);
            }
            seccomp::set_strict(k).map(|()| 0)
        }
        SECCOMP_SET_MODE_FILTER => set_filter(k, flags, args),
        SECCOMP_GET_ACTION_AVAIL => {
            if flags != 0 {
                return Err(Errno::EINVAL);
            }
            let mut b = [0u8; 4];
            k.copy_from_user(args, &mut b)?;
            if Action::available(u32::from_le_bytes(b)) { Ok(0) } else { Err(Errno::EOPNOTSUPP) }
        }
        SECCOMP_GET_NOTIF_SIZES => {
            if flags != 0 {
                return Err(Errno::EINVAL);
            }
            // struct seccomp_notif_sizes { __u16 seccomp_notif;
            // __u16 seccomp_notif_resp; __u16 seccomp_data; }
            let mut b = [0u8; 6];
            b[0..2].copy_from_slice(&SECCOMP_NOTIF_SIZE.to_le_bytes());
            b[2..4].copy_from_slice(&SECCOMP_NOTIF_RESP_SIZE.to_le_bytes());
            b[4..6].copy_from_slice(&(SECCOMP_DATA_SIZE as u16).to_le_bytes());
            k.copy_to_user(args, &b)?;
            Ok(0)
        }
        _ => Err(Errno::EINVAL),
    }
}

/// Check `SECCOMP_SET_MODE_FILTER`'s flags (seccomp(2), the spec sheet):
/// only known bits; `WAIT_KILLABLE_RECV` needs `NEW_LISTENER`; `TSYNC`
/// with `NEW_LISTENER` needs `TSYNC_ESRCH` (its result could not be both a
/// thread id and a descriptor).
pub fn check_filter_flags(flags: u32) -> Result<(), Errno> {
    let has = |f: u32| flags & f != 0;
    if flags & !SECCOMP_FILTER_FLAGS != 0
        || has(SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV) && !has(SECCOMP_FILTER_FLAG_NEW_LISTENER)
        || has(SECCOMP_FILTER_FLAG_TSYNC)
            && has(SECCOMP_FILTER_FLAG_NEW_LISTENER)
            && !has(SECCOMP_FILTER_FLAG_TSYNC_ESRCH)
    {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// `SECCOMP_SET_MODE_FILTER`. With `SECCOMP_FILTER_FLAG_NEW_LISTENER` the
/// result is a new notification descriptor
/// ([`LinuxKernel::seccomp_listener`]); only one filter of a thread's
/// chain may have one (`EBUSY`). The calls the filter answers with
/// `SECCOMP_RET_USER_NOTIF` wait for whoever holds it ([`ioctl`]).
/// `SPEC_ALLOW` changes nothing (no speculation mitigations to leave off).
fn set_filter(k: &mut dyn LinuxKernel, flags: u32, fprog: UserPtr) -> SysResult {
    check_filter_flags(flags)?;
    let fprog = seccomp::read_fprog(k, fprog)?;
    // seccomp(2): "either the calling thread must have the CAP_SYS_ADMIN
    // capability in its user namespace, or the thread must already have
    // the no_new_privs bit set" (EACCES).
    let creds = cred::current(k);
    if !creds.no_new_privs && !creds.capable(cap::SYS_ADMIN) {
        return Err(Errno::EACCES);
    }
    let program = seccomp::read_program(k, fprog)?;
    let listener = flags & SECCOMP_FILTER_FLAG_NEW_LISTENER != 0;
    if listener && seccomp::has_listener(k) {
        return Err(Errno::EBUSY);
    }
    let tsync = flags & SECCOMP_FILTER_FLAG_TSYNC != 0;
    let opts = seccomp::FilterOptions { log: flags & SECCOMP_FILTER_FLAG_LOG != 0, listener };
    let (fd, made) = match listener {
        true => {
            let (fd, l) = k.seccomp_listener()?;
            (Some(fd), Some(l))
        }
        false => (None, None),
    };
    match seccomp::install_with(k, program, seccomp::NewFilter { opts, listener: made }, tsync)? {
        Installed::Done => Ok(fd.map_or(0, |fd| fd as u64)),
        // "the call will not attach the new seccomp filter, and will fail,
        // returning the first thread ID found that cannot synchronize",
        // or ESRCH with SECCOMP_FILTER_FLAG_TSYNC_ESRCH.
        Installed::Blocked(_) if flags & SECCOMP_FILTER_FLAG_TSYNC_ESRCH != 0 => Err(Errno::ESRCH),
        Installed::Blocked(tid) => Ok(tid as i64 as u64),
    }
}

/// `PR_GET_SECCOMP`: the calling thread's mode (in strict mode prctl(2)
/// itself is not allowed, so this reports 0 or 2).
pub fn prctl_get(k: &mut dyn LinuxKernel) -> SysResult {
    let tid = k.tid();
    Ok(u64::from(crate::signal::deliver::with_own(k, |p| p.thread(tid).seccomp.mode())))
}

/// `PR_SET_SECCOMP`: `arg2` `SECCOMP_MODE_STRICT`, or
/// `SECCOMP_MODE_FILTER` with the `struct sock_fprog *` in `arg3`, which
/// is seccomp(2) with no flags; anything else is `EINVAL`.
pub fn prctl_set(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    match a.ulong(1) {
        m if m == u64::from(SECCOMP_MODE_STRICT) => seccomp::set_strict(k).map(|()| 0),
        m if m == u64::from(SECCOMP_MODE_FILTER) => set_filter(k, 0, a.ptr(2)),
        _ => Err(Errno::EINVAL),
    }
}

/// The ioctls of a notification descriptor (seccomp_unotify(2)); `None`
/// if `fd` is not one (the caller goes on with the other ioctls).
pub fn ioctl(k: &mut dyn LinuxKernel, fd: i32, request: u32, arg: UserPtr) -> Option<SysResult> {
    if request >> 8 & 0xff != SECCOMP_IOC_MAGIC {
        return None;
    }
    let listener = k.seccomp_listener_of(fd)?;
    Some(match request {
        SECCOMP_IOCTL_NOTIF_RECV => notif_recv(k, &listener, arg),
        SECCOMP_IOCTL_NOTIF_SEND => notif_send(k, &listener, arg),
        SECCOMP_IOCTL_NOTIF_ID_VALID => {
            let mut id = [0u8; 8];
            k.copy_from_user(arg, &mut id)
                .and_then(|()| if listener.valid(u64::from_le_bytes(id)) { Ok(0) } else { Err(Errno::ENOENT) })
        }
        _ => Err(Errno::EINVAL),
    })
}

/// `SECCOMP_IOCTL_NOTIF_RECV`: waits for the next stopped call and fills
/// the caller's `struct seccomp_notif` (which must be all zero: `EINVAL`)
/// with its id, the thread's id as the caller's pid namespace names it,
/// and the call's `struct seccomp_data`.
fn notif_recv(k: &mut dyn LinuxKernel, listener: &Arc<seccomp::notify::Listener>, arg: UserPtr) -> SysResult {
    let mut b = [0u8; SECCOMP_NOTIF_SIZE as usize];
    k.copy_from_user(arg, &mut b)?;
    if b.iter().any(|&x| x != 0) {
        return Err(Errno::EINVAL);
    }
    let n = loop {
        let mut got = None;
        let woke = k.seccomp_wait(listener, &mut || {
            got = listener.receive();
            got.is_some()
        });
        match got {
            Some(n) => break n,
            None if woke == Wakeup::Signal => return Err(Errno::RESTART_IF_SA_RESTART),
            None => {}
        }
    };
    b[0..8].copy_from_slice(&n.id.to_le_bytes());
    b[8..12].copy_from_slice(&(crate::ns::pid::view(k, n.tid) as u32).to_le_bytes());
    for (i, w) in n.data.words().iter().enumerate() {
        b[16 + 4 * i..20 + 4 * i].copy_from_slice(&w.to_le_bytes());
    }
    if let Err(e) = k.copy_to_user(arg, &b) {
        // Not delivered: the next RECV gets it.
        listener.unreceive(n.id);
        return Err(e);
    }
    Ok(0)
}

/// `SECCOMP_IOCTL_NOTIF_SEND`: the answer in a `struct seccomp_notif_resp`.
fn notif_send(k: &mut dyn LinuxKernel, listener: &Arc<seccomp::notify::Listener>, arg: UserPtr) -> SysResult {
    let mut b = [0u8; SECCOMP_NOTIF_RESP_SIZE as usize];
    k.copy_from_user(arg, &mut b)?;
    let word = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"));
    let id = u64::from_le_bytes(b[0..8].try_into().expect("8 bytes"));
    let val = i64::from_le_bytes(b[8..16].try_into().expect("8 bytes"));
    let reply = seccomp::notify::Reply::of(val, word(16) as i32, word(20))?;
    listener.send(id, reply).map(|()| 0)
}
