//! seccomp(2) and prctl(2)'s `PR_SET_SECCOMP` / `PR_GET_SECCOMP`: install
//! strict mode or a filter on the calling thread, ask which actions are
//! supported. The rules are in [`crate::seccomp`] (ADR 0054).
//!
//! The order of the checks of `SECCOMP_SET_MODE_FILTER` is Poly's (the
//! man page lists the errors without one), to be confirmed by the diff
//! runner: unknown or unsupported flags (`EINVAL`), the privilege
//! (`EACCES`), reading and checking the program (`EFAULT`, `EINVAL`), the
//! thread's mode (`EINVAL`), the path length (`ENOMEM`), the other threads
//! (`TSYNC`).

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::seccomp::abi::*;
use crate::seccomp::{self, Action, Installed};
use crate::table::{Handler, nr};

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

/// `SECCOMP_SET_MODE_FILTER`. A user notification listener
/// (`SECCOMP_FILTER_FLAG_NEW_LISTENER`, and `WAIT_KILLABLE_RECV`, which
/// needs one) is `EINVAL`: Poly has none. `SPEC_ALLOW` changes nothing
/// (no speculation mitigations to leave off).
fn set_filter(k: &mut dyn LinuxKernel, flags: u32, fprog: UserPtr) -> SysResult {
    if flags & !SECCOMP_FILTER_FLAGS != 0
        || flags & (SECCOMP_FILTER_FLAG_NEW_LISTENER | SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV) != 0
    {
        return Err(Errno::EINVAL);
    }
    // seccomp(2): "either the calling thread must have the CAP_SYS_ADMIN
    // capability in its user namespace, or the thread must already have
    // the no_new_privs bit set" (EACCES).
    let creds = cred::current(k);
    if !creds.no_new_privs && !creds.capable(cap::SYS_ADMIN) {
        return Err(Errno::EACCES);
    }
    let program = seccomp::read_program(k, fprog)?;
    let tsync = flags & SECCOMP_FILTER_FLAG_TSYNC != 0;
    match seccomp::install(k, program, flags & SECCOMP_FILTER_FLAG_LOG != 0, tsync)? {
        Installed::Done => Ok(0),
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
