//! Process descriptors: pidfd_open(2) and pidfd_send_signal(2). A pidfd
//! names a process (or with `PIDFD_THREAD` a thread) by its global id,
//! which Poly never hands out again, so a pidfd of a reaped process stays
//! a valid descriptor whose process is simply gone (`ESRCH`). The kernel
//! owns the file ([`FileIo::pidfd_open`](crate::fileio::FileIo::pidfd_open)):
//! readable (`POLLIN`) once the process has ended; clone3's `CLONE_PIDFD`
//! (`crate::task::clone`) and waitid(2)'s `P_PIDFD` (`sys::task`) use
//! the same files.

use crate::abi::SigInfo;
use crate::abi::signal::{SI_TKILL, SI_USER};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{PidFdTarget, flags};
use crate::kernel::{LinuxKernel, ProcessSelector, Target};
use crate::ns::pid;
use crate::signal::deliver::{send, send_to_all};
use crate::sys::pipe::file_io;
use crate::table::Handler;
use crate::table::nr;

/// `PIDFD_NONBLOCK` (`linux/pidfd.h`: `O_NONBLOCK`).
pub const PIDFD_NONBLOCK: u32 = flags::O_NONBLOCK;
/// `PIDFD_THREAD` (`linux/pidfd.h`: `O_EXCL`).
pub const PIDFD_THREAD: u32 = 0o200;

/// pidfd_send_signal's flags (`linux/pidfd.h`).
pub const PIDFD_SIGNAL_THREAD: u32 = 1;
pub const PIDFD_SIGNAL_THREAD_GROUP: u32 = 2;
pub const PIDFD_SIGNAL_PROCESS_GROUP: u32 = 4;

/// The calls of this module.
pub const CALLS: &[(u64, Handler)] = &[(nr::PIDFD_SEND_SIGNAL, pidfd_send_signal), (nr::PIDFD_OPEN, pidfd_open)];

/// What process `local` (the caller's pid namespace's id) is, for
/// pidfd_open: its global id and whether it leads its thread group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// A process (thread group leader), alive or a zombie.
    Process(i32),
    /// A thread that does not lead its group.
    Thread(i32),
}

/// Look `local` up: a process of the tree (also a zombie), else a thread;
/// `ESRCH` when there is neither.
pub fn find_task(k: &mut dyn LinuxKernel, local: i32) -> Result<Found, Errno> {
    let global = pid::find(k, local)?;
    if crate::task::with_tree(k, |t| t.contains(global))? {
        return Ok(Found::Process(global));
    }
    let mut tgid = None;
    k.with_process(Target::Thread(global), &mut |_, info| tgid = Some(info.pid)).map_err(|_| Errno::ESRCH)?;
    match tgid {
        Some(g) if g == global => Ok(Found::Process(global)),
        Some(_) => Ok(Found::Thread(global)),
        None => Err(Errno::ESRCH),
    }
}

/// The checks of pidfd_open(pid, flags), in the spec sheet's order: the
/// flags (only `PIDFD_NONBLOCK` and `PIDFD_THREAD`), then the pid (not
/// positive: `EINVAL`). Both are 32 bits wide.
pub fn check_open(pid: i32, flags: u32) -> Result<(), Errno> {
    if flags & !(PIDFD_NONBLOCK | PIDFD_THREAD) != 0 || pid <= 0 {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// pidfd_open(pid, flags): a pidfd for `pid` (spec sheet): `ESRCH` when
/// there is no such process, `ENOENT` for a thread that does not lead its
/// group unless `PIDFD_THREAD` asks for one thread.
pub fn pidfd_open(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (local, bits) = (a.int(0), a.uint(1));
    check_open(local, bits)?;
    let thread = bits & PIDFD_THREAD != 0;
    let pid = match find_task(k, local)? {
        Found::Process(p) => p,
        Found::Thread(t) if thread => t,
        Found::Thread(_) => return Err(Errno::ENOENT),
    };
    file_io(k)?.pidfd_open(pid, thread, bits & PIDFD_NONBLOCK != 0).map(|fd| fd as u64)
}

/// Where pidfd_send_signal's flags send the signal of a pidfd naming
/// `pid` (`thread`: a `PIDFD_THREAD` pidfd).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The thread `pid`.
    Thread(i32),
    /// The process of thread (or process) `pid`.
    Process(i32),
    /// The process group `pid` leads.
    Group(i32),
}

/// pidfd_send_signal's `flags`: 0 (the pidfd's own scope: its thread for a
/// thread pidfd, else the process), or exactly one of the three scopes;
/// anything else is `EINVAL`.
pub fn scope(flags: u32, pid: i32, thread: bool) -> Result<Scope, Errno> {
    match flags {
        0 if thread => Ok(Scope::Thread(pid)),
        0 | PIDFD_SIGNAL_THREAD_GROUP => Ok(Scope::Process(pid)),
        PIDFD_SIGNAL_THREAD => Ok(Scope::Thread(pid)),
        PIDFD_SIGNAL_PROCESS_GROUP => Ok(Scope::Group(pid)),
        _ => Err(Errno::EINVAL),
    }
}

/// The caller's siginfo for another process or itself (`to_self`):
/// `si_signo` must be `sig` (`EINVAL`); to another process `si_code` must
/// be negative and not `SI_TKILL` (`EPERM`), as for rt_sigqueueinfo(2).
pub fn check_info(info: &SigInfo, sig: u32, to_self: bool) -> Result<(), Errno> {
    if info.signo != sig as i32 {
        return Err(Errno::EINVAL);
    }
    if !to_self && (info.code >= 0 || info.code == SI_TKILL) {
        return Err(Errno::EPERM);
    }
    Ok(())
}

/// pidfd_send_signal(pidfd, sig, info, flags): checks in the spec sheet's
/// order: flags, the descriptor (`EBADF` unless a pidfd or a `/proc/<pid>`
/// directory), the signal number (0..64), the siginfo (`EFAULT`,
/// `EINVAL`, `EPERM`), then the target as kill(2) finds it (a zombie takes
/// any signal and nothing happens; reaped: `ESRCH`).
pub fn pidfd_send_signal(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let bits = a.uint(3);
    if !matches!(bits, 0 | PIDFD_SIGNAL_THREAD | PIDFD_SIGNAL_THREAD_GROUP | PIDFD_SIGNAL_PROCESS_GROUP) {
        return Err(Errno::EINVAL);
    }
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let (pid, thread) = match file_io(k)?.pidfd_target(fd)? {
        Some(PidFdTarget::PidFd { pid, thread }) => (pid, thread),
        Some(PidFdTarget::ProcDir(pid)) => (pid, false),
        None => return Err(Errno::EBADF),
    };
    let sig = match a.int(1) {
        s @ 0..=64 => s as u32,
        _ => return Err(Errno::EINVAL),
    };
    let to_self = pid == k.pid() || pid == k.tid();
    let info = if a.ptr(2).is_null() {
        SigInfo::from_process(sig, SI_USER, k.pid(), k.credentials().uid)
    } else {
        let mut raw = [0; SigInfo::SIZE];
        k.copy_from_user(a.ptr(2), &mut raw)?;
        let info = SigInfo::from_bytes(&raw);
        check_info(&info, sig, to_self)?;
        info
    };
    match scope(bits, pid, thread)? {
        Scope::Thread(tid) => send(k, Target::Thread(tid), Some(tid), None, info)?,
        Scope::Process(p) if thread => send(k, Target::Thread(p), None, None, info)?,
        Scope::Process(p) => send(k, Target::Process(p), None, None, info)?,
        Scope::Group(g) => {
            if crate::task::with_tree(k, |t| t.pgid(g))? != Some(g) {
                return Err(Errno::ESRCH);
            }
            let pids = k.process_ids(ProcessSelector::Group(g));
            send_to_all(k, &pids, info)?;
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::signal::{SI_KERNEL, SI_QUEUE, SIGUSR1, SIGUSR2};

    #[test]
    fn open_checks_flags_before_the_pid() {
        assert_eq!(check_open(5, 0), Ok(()));
        assert_eq!(check_open(5, PIDFD_NONBLOCK | PIDFD_THREAD), Ok(()));
        for bad in [flags::O_CLOEXEC, 1, 2, 0x8000_0000, PIDFD_NONBLOCK | 1] {
            assert_eq!(check_open(5, bad), Err(Errno::EINVAL), "{bad:#x}");
            assert_eq!(check_open(0x3fff_fff0, bad), Err(Errno::EINVAL));
        }
        assert_eq!(check_open(0, 0), Err(Errno::EINVAL));
        assert_eq!(check_open(-1, 0), Err(Errno::EINVAL));
    }

    #[test]
    fn signal_scopes() {
        assert_eq!(scope(0, 7, false), Ok(Scope::Process(7)));
        assert_eq!(scope(0, 7, true), Ok(Scope::Thread(7)));
        assert_eq!(scope(PIDFD_SIGNAL_THREAD, 7, false), Ok(Scope::Thread(7)));
        assert_eq!(scope(PIDFD_SIGNAL_THREAD_GROUP, 7, true), Ok(Scope::Process(7)));
        assert_eq!(scope(PIDFD_SIGNAL_PROCESS_GROUP, 7, false), Ok(Scope::Group(7)));
        for bad in [3, 6, 7, 8, 0x8000_0000] {
            assert_eq!(scope(bad, 7, false), Err(Errno::EINVAL), "{bad:#x}");
        }
    }

    #[test]
    fn siginfo_rules() {
        let info = SigInfo::new;
        assert_eq!(check_info(&info(SIGUSR2, SI_QUEUE), SIGUSR1, false), Err(Errno::EINVAL));
        assert_eq!(check_info(&info(SIGUSR1, SI_USER), SIGUSR1, false), Err(Errno::EPERM));
        assert_eq!(check_info(&info(SIGUSR1, SI_KERNEL), SIGUSR1, false), Err(Errno::EPERM));
        assert_eq!(check_info(&info(SIGUSR1, SI_TKILL), SIGUSR1, false), Err(Errno::EPERM));
        assert_eq!(check_info(&info(SIGUSR1, SI_QUEUE), SIGUSR1, false), Ok(()));
        assert_eq!(check_info(&info(SIGUSR1, SI_KERNEL), SIGUSR1, true), Ok(()), "any code to oneself");
        assert_eq!(check_info(&info(SIGUSR2, SI_KERNEL), SIGUSR1, true), Err(Errno::EINVAL));
    }
}
