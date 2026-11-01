//! prctl(2): the operations on the calling thread's credentials (ADR
//! 0045) and its name. Other options are `EINVAL`, as prctl(2) says for
//! "option is not recognized".

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::signal::deliver::with_own;

/// Options (UAPI `linux/prctl.h`).
pub const PR_SET_PDEATHSIG: u32 = 1;
pub const PR_GET_PDEATHSIG: u32 = 2;
pub const PR_GET_DUMPABLE: u32 = 3;
pub const PR_SET_DUMPABLE: u32 = 4;
pub const PR_GET_KEEPCAPS: u32 = 7;
pub const PR_SET_KEEPCAPS: u32 = 8;
pub const PR_SET_NAME: u32 = 15;
pub const PR_GET_NAME: u32 = 16;
pub const PR_CAPBSET_READ: u32 = 23;
pub const PR_CAPBSET_DROP: u32 = 24;
pub const PR_GET_SECUREBITS: u32 = 27;
pub const PR_SET_NO_NEW_PRIVS: u32 = 38;
pub const PR_GET_NO_NEW_PRIVS: u32 = 39;
pub const PR_CAP_AMBIENT: u32 = 47;
pub const PR_CAP_AMBIENT_IS_SET: u64 = 1;
pub const PR_CAP_AMBIENT_RAISE: u64 = 2;
pub const PR_CAP_AMBIENT_LOWER: u64 = 3;
pub const PR_CAP_AMBIENT_CLEAR_ALL: u64 = 4;
/// `SECBIT_KEEP_CAPS` (`1 << SECURE_KEEP_CAPS`, UAPI `linux/securebits.h`).
pub const SECBIT_KEEP_CAPS: u64 = 1 << 4;

/// The length of a thread's name with its NUL (`TASK_COMM_LEN`).
pub const COMM_LEN: usize = 16;

/// The name execve(2) gives a thread: the last component of the program's
/// path, cut to 15 bytes (proc(5), `/proc/[pid]/comm`: "the command name
/// associated with the process ... the filename of the executable").
pub fn comm_from_path(path: &[u8]) -> [u8; COMM_LEN] {
    let name = path.rsplit(|&b| b == b'/').next().unwrap_or(path);
    let mut comm = [0u8; COMM_LEN];
    let n = name.len().min(COMM_LEN - 1);
    comm[..n].copy_from_slice(&name[..n]);
    comm
}

fn bool_arg(v: u64) -> Result<bool, Errno> {
    match v {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Errno::EINVAL),
    }
}

fn require_zero(a: &Args, from: usize) -> Result<(), Errno> {
    if (from..5).any(|i| a.ulong(i) != 0) { Err(Errno::EINVAL) } else { Ok(()) }
}

fn cap_arg(v: u64) -> Result<u32, Errno> {
    u32::try_from(v).ok().filter(|&c| cap::valid(c)).ok_or(Errno::EINVAL)
}

/// prctl(option, arg2, arg3, arg4, arg5).
///
/// - `PR_SET_KEEPCAPS` (`arg2` 0 or 1, else `EINVAL`) / `PR_GET_KEEPCAPS`:
///   the "keep capabilities" flag ([`cred::Creds::keep_caps`]).
/// - `PR_CAPBSET_READ` (1 if `arg2` is in the bounding set) /
///   `PR_CAPBSET_DROP` (`CAP_SETPCAP`, `EPERM`); `EINVAL` "if arg2 is not
///   a valid capability".
/// - `PR_SET_NO_NEW_PRIVS`: `arg2` must be 1 ("Once set, ... cannot be
///   unset") and `arg3`-`arg5` 0, else `EINVAL`; `PR_GET_NO_NEW_PRIVS`
///   with `arg2`-`arg5` 0.
/// - `PR_CAP_AMBIENT` with `IS_SET`, `RAISE`, `LOWER` (`arg4`, `arg5` 0)
///   and `CLEAR_ALL` (`arg3`-`arg5` 0).
/// - `PR_GET_SECUREBITS`: only `SECBIT_KEEP_CAPS` can be set in Poly.
/// - `PR_SET_SECCOMP` / `PR_GET_SECCOMP`: [`super::seccomp`] (ADR 0054).
/// - `PR_SET_NAME` / `PR_GET_NAME`: the thread's name, 16 bytes with the
///   NUL ("silently truncated" if longer); `EFAULT`.
/// - `PR_SET_PDEATHSIG` (a signal number 0..=64, else `EINVAL`) /
///   `PR_GET_PDEATHSIG` (stored as an `int` at `arg2`): the calling
///   thread's parent-death signal ([`crate::task::parent_died`]).
/// - `PR_GET_DUMPABLE` (the result) / `PR_SET_DUMPABLE` (0 or 1, else
///   `EINVAL`): the process's dumpable flag.
pub fn prctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let arg2 = a.ulong(1);
    match a.int(0) as u32 {
        PR_SET_PDEATHSIG => {
            let signal = u32::try_from(arg2).ok().filter(|&s| s <= crate::abi::signal::NSIG).ok_or(Errno::EINVAL)?;
            let tid = k.tid();
            with_own(k, |p| p.thread_mut(tid).pdeathsig = signal);
            Ok(0)
        }
        PR_GET_PDEATHSIG => {
            let tid = k.tid();
            let signal = with_own(k, |p| p.thread(tid).pdeathsig);
            k.copy_to_user(a.ptr(1), &(signal as i32).to_le_bytes())?;
            Ok(0)
        }
        PR_GET_DUMPABLE => Ok(u64::from(with_own(k, |p| p.dumpable))),
        PR_SET_DUMPABLE => {
            let dumpable = bool_arg(arg2)?;
            with_own(k, |p| p.dumpable = dumpable);
            Ok(0)
        }
        PR_GET_KEEPCAPS => Ok(u64::from(cred::current(k).keep_caps)),
        PR_SET_KEEPCAPS => {
            let keep = bool_arg(arg2)?;
            cred::change(k, |c| c.keep_caps = keep);
            Ok(0)
        }
        PR_GET_SECUREBITS => Ok(if cred::current(k).keep_caps { SECBIT_KEEP_CAPS } else { 0 }),
        PR_CAPBSET_READ => {
            let c = cap_arg(arg2)?;
            Ok(u64::from(cred::current(k).caps.bounding.contains(c)))
        }
        PR_CAPBSET_DROP => {
            let c = cap_arg(arg2)?;
            cred::change(k, |cr| cr.drop_bounding(c)).map(|()| 0)
        }
        PR_SET_NO_NEW_PRIVS => {
            if arg2 != 1 {
                return Err(Errno::EINVAL);
            }
            require_zero(a, 2)?;
            cred::change(k, |c| c.no_new_privs = true);
            Ok(0)
        }
        PR_GET_NO_NEW_PRIVS => {
            require_zero(a, 1)?;
            Ok(u64::from(cred::current(k).no_new_privs))
        }
        PR_CAP_AMBIENT => ambient(k, a),
        crate::seccomp::abi::PR_GET_SECCOMP => super::seccomp::prctl_get(k),
        crate::seccomp::abi::PR_SET_SECCOMP => super::seccomp::prctl_set(k, a),
        PR_SET_NAME => {
            let mut buf = [0u8; COMM_LEN];
            let n = match k.strncpy_from_user(a.ptr(1), &mut buf) {
                Ok(n) => n,
                Err(Errno::ENAMETOOLONG) => COMM_LEN - 1,
                Err(e) => return Err(e),
            };
            let mut comm = [0u8; COMM_LEN];
            comm[..n].copy_from_slice(&buf[..n]);
            let tid = k.tid();
            with_own(k, |p| p.thread_mut(tid).comm = comm);
            Ok(0)
        }
        PR_GET_NAME => {
            let tid = k.tid();
            let comm = with_own(k, |p| p.thread(tid).comm);
            k.copy_to_user(a.ptr(1), &comm)?;
            Ok(0)
        }
        super::subreaper::PR_SET_CHILD_SUBREAPER => super::subreaper::set(k, arg2),
        super::subreaper::PR_GET_CHILD_SUBREAPER => super::subreaper::get(k, a.ptr(1)),
        _ => Err(Errno::EINVAL),
    }
}

/// `PR_CAP_AMBIENT`: the operation in `arg2`, the capability in `arg3`.
fn ambient(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    require_zero(a, 3)?;
    match a.ulong(1) {
        PR_CAP_AMBIENT_CLEAR_ALL => {
            if a.ulong(2) != 0 {
                return Err(Errno::EINVAL);
            }
            cred::change(k, |c| c.caps.ambient = cred::CapSet::EMPTY);
            Ok(0)
        }
        PR_CAP_AMBIENT_IS_SET => {
            let c = cap_arg(a.ulong(2))?;
            Ok(u64::from(cred::current(k).caps.ambient.contains(c)))
        }
        PR_CAP_AMBIENT_RAISE => {
            let c = cap_arg(a.ulong(2))?;
            cred::change(k, |cr| cr.raise_ambient(c)).map(|()| 0)
        }
        PR_CAP_AMBIENT_LOWER => {
            let c = cap_arg(a.ulong(2))?;
            cred::change(k, |cr| cr.lower_ambient(c)).map(|()| 0)
        }
        _ => Err(Errno::EINVAL),
    }
}
