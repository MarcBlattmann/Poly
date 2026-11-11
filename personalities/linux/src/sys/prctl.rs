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
pub const PR_SET_SECUREBITS: u32 = 28;
pub const PR_SET_NO_NEW_PRIVS: u32 = 38;
pub const PR_GET_NO_NEW_PRIVS: u32 = 39;
pub const PR_CAP_AMBIENT: u32 = 47;
pub const PR_CAP_AMBIENT_IS_SET: u64 = 1;
pub const PR_CAP_AMBIENT_RAISE: u64 = 2;
pub const PR_CAP_AMBIENT_LOWER: u64 = 3;
pub const PR_CAP_AMBIENT_CLEAR_ALL: u64 = 4;
pub const PR_GET_SPECULATION_CTRL: u32 = 52;
pub const PR_SET_SPECULATION_CTRL: u32 = 53;
/// The speculation features (`PR_SPEC_STORE_BYPASS`,
/// `PR_SPEC_INDIRECT_BRANCH`, `PR_SPEC_L1D_FLUSH`) and their states.
pub const PR_SPEC_FEATURES: u64 = 3;
pub const PR_SPEC_PRCTL: u64 = 1 << 0;
pub const PR_SPEC_ENABLE: u64 = 1 << 1;
pub const PR_SPEC_DISABLE: u64 = 1 << 2;
pub const PR_SPEC_FORCE_DISABLE: u64 = 1 << 3;
pub const PR_SPEC_DISABLE_NOEXEC: u64 = 1 << 4;
pub const PR_SET_VMA: u32 = 0x5356_4d41;
pub const PR_SET_VMA_ANON_NAME: u64 = 0;
/// The longest name `PR_SET_VMA_ANON_NAME` takes, with its NUL
/// (`ANON_VMA_NAME_MAX_LEN`, 80; the prctl spec sheet: 79 bytes accepted,
/// 80 refused).
pub const ANON_VMA_NAME_MAX_LEN: usize = 80;
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
/// - `PR_GET_SECUREBITS` / `PR_SET_SECUREBITS` (`arg3`-`arg5` ignored):
///   the secure bits ([`cred::Creds::set_securebits`]); a refused change
///   is `EPERM`, never `EINVAL`. `PR_SET_KEEPCAPS` is `EPERM` while
///   `SECBIT_KEEP_CAPS` is locked.
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
        crate::sys::perf::PR_TASK_PERF_EVENTS_DISABLE | crate::sys::perf::PR_TASK_PERF_EVENTS_ENABLE => {
            let enable = a.int(0) as u32 == crate::sys::perf::PR_TASK_PERF_EVENTS_ENABLE;
            crate::sys::perf::task_events(k, enable);
            Ok(0)
        }
        PR_SET_DUMPABLE => {
            let dumpable = bool_arg(arg2)?;
            with_own(k, |p| p.dumpable = dumpable);
            Ok(0)
        }
        PR_GET_KEEPCAPS => Ok(u64::from(cred::current(k).keep_caps)),
        PR_SET_KEEPCAPS => {
            let keep = bool_arg(arg2)?;
            cred::change(k, |c| c.set_keep_caps(keep)).map(|()| 0)
        }
        PR_GET_SECUREBITS => Ok(u64::from(cred::current(k).securebits())),
        PR_SET_SECUREBITS => cred::change(k, |c| c.set_securebits(arg2)).map(|()| 0),
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
        PR_SET_VMA => set_vma(k, a),
        PR_GET_SPECULATION_CTRL => {
            require_zero(a, 2)?;
            let which = spec_feature(arg2)?;
            let tid = k.tid();
            let state = with_own(k, |p| p.thread(tid).spec_ctrl[which]);
            Ok(PR_SPEC_PRCTL | if state == 0 { PR_SPEC_ENABLE } else { u64::from(state) })
        }
        PR_SET_SPECULATION_CTRL => {
            require_zero(a, 3)?;
            let which = spec_feature(arg2)?;
            let set = a.ulong(2);
            if ![PR_SPEC_ENABLE, PR_SPEC_DISABLE, PR_SPEC_FORCE_DISABLE, PR_SPEC_DISABLE_NOEXEC].contains(&set) {
                return Err(Errno::ERANGE);
            }
            let tid = k.tid();
            with_own(k, |p| {
                let state = &mut p.thread_mut(tid).spec_ctrl[which];
                // "PR_SPEC_FORCE_DISABLE: Same as PR_SPEC_DISABLE, but
                // cannot be undone": EPERM for anything but itself after.
                if u64::from(*state) == PR_SPEC_FORCE_DISABLE && set != PR_SPEC_FORCE_DISABLE {
                    return Err(Errno::EPERM);
                }
                *state = set as u8;
                Ok(0)
            })
        }
        _ => Err(Errno::EINVAL),
    }
}

/// The feature of `PR_GET/SET_SPECULATION_CTRL` (`arg2`): `ENODEV` for
/// one the kernel does not know (prctl(2)).
fn spec_feature(v: u64) -> Result<usize, Errno> {
    if v < PR_SPEC_FEATURES { Ok(v as usize) } else { Err(Errno::ENODEV) }
}

/// `PR_SET_VMA` with `PR_SET_VMA_ANON_NAME` (the only sub-option, else
/// `EINVAL`): name the anonymous mappings of `[arg3, arg3 + arg4)` with
/// the string at `arg5`, or remove their names (`arg5` NULL). The name is
/// at most 79 bytes of printable ASCII without `\` `` ` `` `$` `[` `]`
/// (`EINVAL`), readable (`EFAULT`); the kernel checks the range
/// ([`LinuxKernel::name_anon`]: `EINVAL` for an unaligned start, `ENOMEM`
/// for a hole, `EBADF` for a file mapping).
fn set_vma(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ulong(1) != PR_SET_VMA_ANON_NAME {
        return Err(Errno::EINVAL);
    }
    let (addr, len) = (a.ulong(2), a.ulong(3));
    if addr % crate::abi::PAGE_SIZE != 0 {
        return Err(Errno::EINVAL);
    }
    let mut buf = [0u8; ANON_VMA_NAME_MAX_LEN];
    let name = if a.ptr(4).is_null() {
        None
    } else {
        let n = match k.strncpy_from_user(a.ptr(4), &mut buf) {
            Ok(n) => n,
            Err(Errno::ENAMETOOLONG) => return Err(Errno::EINVAL),
            Err(e) => return Err(e),
        };
        if !valid_anon_name(&buf[..n]) {
            return Err(Errno::EINVAL);
        }
        Some(&buf[..n])
    };
    k.name_anon(addr, len, name).map(|()| 0)
}

/// Whether `name` may name anonymous memory: printable ASCII, none of
/// `` \ ` $ [ ] `` (the prctl spec sheet, `vma_name_invalid`).
pub fn valid_anon_name(name: &[u8]) -> bool {
    name.iter().all(|&b| (0x20..0x7f).contains(&b) && !b"\\`$[]".contains(&b))
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
