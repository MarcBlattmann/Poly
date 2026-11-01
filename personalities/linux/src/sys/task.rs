//! Creating, replacing and waiting for processes and threads: fork(2),
//! vfork(2), clone(2), clone3(2), execve(2), wait4(2), waitid(2), and
//! process groups and sessions: setpgid(2), getpgid(2), getpgrp(2),
//! setsid(2), getsid(2). The rules are in [`crate::task`]; see ADR 0041.

use crate::abi::{SigInfo, USER_ADDRESS_LIMIT};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr, Wakeup};
use crate::ns::pid::{find, find_or_none, view};
use crate::task::clone::{self, CLONE_NEWUSER, CloneArgs, CloneRequest};
use crate::task::exec::{self, ExecRequest};
use crate::task::tree::{Poll, Selector, WaitFilter, Waited};
use crate::task::wait;
use crate::task::with_tree;

/// `AT_FDCWD` (`linux/fcntl.h`).
const AT_FDCWD: i32 = -100;

fn pid_result(id: i32) -> SysResult {
    Ok(id as i64 as u64)
}

/// A global id as the caller's pid namespace sees it, as a result.
fn seen(k: &dyn LinuxKernel, global: i32) -> SysResult {
    pid_result(view(k, global))
}

/// Check what the kernel cannot (a TLS base must be a user address, as for
/// arch_prctl(2): `EPERM`; to be confirmed by the diff runner; a new user
/// namespace from a chroot, clone(2) `EPERM`) and create the child. Its id
/// is reported as the caller's namespace sees it.
fn spawn(k: &mut dyn LinuxKernel, req: &CloneRequest) -> SysResult {
    if req.tls.is_some_and(|tls| tls >= USER_ADDRESS_LIMIT) {
        return Err(Errno::EPERM);
    }
    if req.flags & CLONE_NEWUSER != 0 && k.chrooted() {
        return Err(Errno::EPERM);
    }
    // A SCHED_DEADLINE task cannot create tasks unless they are reset to
    // SCHED_OTHER (sched_setattr's spec sheet: EAGAIN).
    let tid = k.tid();
    let sched = crate::signal::deliver::with_own(k, |p| p.sched_of(tid));
    if sched.is_some_and(|s| s.policy == crate::task::sched::SCHED_DEADLINE && !s.reset_on_fork) {
        return Err(Errno::EAGAIN);
    }
    let child = k.clone_task(req)?;
    seen(k, child)
}

/// The caller's pid as its namespace sees it (`CLONE_PARENT` from an init
/// is `EINVAL`).
fn own_pid(k: &dyn LinuxKernel) -> i32 {
    view(k, k.pid())
}

/// A wait selector with the caller's ids made global.
fn global_selector(k: &dyn LinuxKernel, s: Selector) -> Selector {
    match s {
        Selector::Any => Selector::Any,
        Selector::Pid(p) => Selector::Pid(find_or_none(k, p)),
        Selector::Group(g) => Selector::Group(find_or_none(k, g)),
    }
}

/// fork(): clone(2) with only `SIGCHLD` (fork(2)).
pub fn fork(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    spawn(k, &CloneRequest::fork())
}

/// vfork(): the child shares the caller's memory and the caller waits
/// until it calls execve(2) or ends (vfork(2)).
pub fn vfork(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    spawn(k, &CloneRequest::vfork())
}

/// clone(flags, stack, parent_tid, child_tid, tls): the x86-64 order of
/// the raw system call (clone(2), "C library/kernel differences").
pub fn clone(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let req = clone::decode_clone(a.ulong(0), a.ulong(1), a.ptr(2), a.ptr(3), a.ulong(4), own_pid(k))?;
    spawn(k, &req)
}

/// clone3(cl_args, size): the structure is read first (`EFAULT`), then
/// checked (clone(2)).
pub fn clone3(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let size = clone::check_clone3_size(a.ulong(1))?;
    let known = size.min(clone::CLONE_ARGS_SIZE_VER2);
    let mut bytes = [0u8; clone::CLONE_ARGS_SIZE_VER2];
    k.copy_from_user(a.ptr(0), &mut bytes[..known])?;
    let mut extra_nonzero = false;
    // The bytes past the fields we know, up to a page: read in small pieces,
    // not into a page-sized buffer, which would make this frame alone take
    // a quarter of a kernel thread stack (the clone path is the deepest
    // system call there is).
    let mut piece = [0u8; 64];
    let mut done = known;
    while done < size {
        let n = (size - done).min(piece.len());
        let at = a.ptr(0).offset(done as u64).ok_or(Errno::EFAULT)?;
        k.copy_from_user(at, &mut piece[..n])?;
        extra_nonzero |= piece[..n].iter().any(|&b| b != 0);
        done += n;
    }
    let req = clone::decode_clone3(&CloneArgs::from_bytes(&bytes[..known]), extra_nonzero, own_pid(k))?;
    spawn(k, &req)
}

/// execve(pathname, argv, envp): the path, then `argv`, then `envp` are
/// read (`EFAULT`, `ENAMETOOLONG`, `E2BIG`); the kernel does the rest.
/// On success the call "returns" into the new program, whose registers
/// the kernel has set.
pub fn execve(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = exec::read_path(k, a.ptr(0))?;
    let mut budget = exec::MAX_ARG_TOTAL;
    let argv = exec::read_strings(k, a.ptr(1), &mut budget)?;
    let envp = exec::read_strings(k, a.ptr(2), &mut budget)?;
    k.execve(&ExecRequest::new(&path, &argv, &envp))?;
    let pid = k.pid();
    let _ = with_tree(k, |t| t.exec(pid));
    Ok(0)
}

/// execveat(2) flags (`linux/fcntl.h`).
const AT_SYMLINK_NOFOLLOW: u32 = 0x100;
const AT_EMPTY_PATH: u32 = 0x1000;

/// execveat(dirfd, pathname, argv, envp, flags): execve(2) of `pathname`
/// relative to `dirfd`, or of `dirfd`'s own file with `AT_EMPTY_PATH` and
/// an empty path. In the order of execveat's spec sheet: the path is read
/// (`EFAULT`), the flags checked (`EINVAL` for bits other than
/// `AT_EMPTY_PATH` and `AT_SYMLINK_NOFOLLOW`), the file looked up and
/// checked ([`LinuxKernel::exec_lookup`]: `EBADF`, `ENOTDIR`, `ENOENT`,
/// `ELOOP`, `EACCES`, `ETXTBSY`), then `argv` and `envp` read (`EFAULT`)
/// and the program run. The new image is known as [`exec::exec_name`];
/// a script reached through a descriptor with `FD_CLOEXEC` could not be
/// opened by its interpreter and is `ENOENT`. An empty `argv` becomes
/// one empty string.
pub fn execveat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let dirfd = a.fd(0);
    let path = crate::sys::path::path_arg(k, a.ptr(1))?;
    let flags = a.uint(4);
    if flags & !(AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW) != 0 {
        return Err(Errno::EINVAL);
    }
    if path.is_empty() && flags & AT_EMPTY_PATH == 0 {
        return Err(Errno::ENOENT);
    }
    let name = exec::exec_name(dirfd, &path);
    let mut req = ExecRequest {
        dirfd,
        path: &path,
        argv: &[],
        envp: &[],
        follow: flags & AT_SYMLINK_NOFOLLOW == 0,
        name: &name,
        script_reachable: true,
    };
    k.exec_lookup(&req)?;
    // The descriptor the name goes through (the directory, or the file
    // itself) is closed by the exec when it has FD_CLOEXEC.
    let through_fd = dirfd != AT_FDCWD && path.first() != Some(&b'/');
    if through_fd {
        // The lookup has found the descriptor open already.
        req.script_reachable = !crate::sys::path::fs(k).and_then(|fs| fs.cloexec(dirfd)).unwrap_or(false);
    }
    let mut budget = exec::MAX_ARG_TOTAL;
    let mut argv = exec::read_strings(k, a.ptr(2), &mut budget)?;
    let envp = exec::read_strings(k, a.ptr(3), &mut budget)?;
    if argv.is_empty() {
        argv.push(alloc::vec::Vec::new());
    }
    req.argv = &argv;
    req.envp = &envp;
    k.execve(&req)?;
    let pid = k.pid();
    let _ = with_tree(k, |t| t.exec(pid));
    Ok(0)
}

/// Wait until a child matches `filter` (`None` at once with `nohang`).
/// `ECHILD` if no child matches at all. A signal for the caller ends the
/// wait; signal(7) lists wait(2) among the calls `SA_RESTART` restarts.
///
/// Returns the child and its id as the caller's pid namespace sees it,
/// taken while the child is still in the tree: once it is reaped, its
/// numbers in the namespaces are free again.
fn wait_for(k: &mut dyn LinuxKernel, filter: &WaitFilter, nohang: bool) -> Result<Option<(Waited, i32)>, Errno> {
    let me = k.pid();
    let ns = k.pid_ns();
    let local = |global: i32| match &ns {
        None => global,
        ns => crate::ns::pid::local_or_zero(ns, global),
    };
    loop {
        let (generation, poll, seen) = with_tree(k, |t| {
            let generation = t.generation();
            let poll = t.poll(me, filter);
            let seen = if let Poll::Ready(w) = &poll { local(w.pid) } else { 0 };
            (generation, poll, seen)
        })?;
        match poll {
            Poll::Ready(w) => return Ok(Some((w, seen))),
            Poll::NoChildren => return Err(Errno::ECHILD),
            Poll::Pending if nohang => return Ok(None),
            // A child is in the middle of ending (on another CPU): its exit
            // signal is about to be sent, then it is waitable. Not worth a
            // "nothing yet", even with WNOHANG, which a pending signal of
            // the caller does not end either (it is not a blocking wait).
            Poll::Exiting if nohang => {
                if k.wait_tree(generation) == Wakeup::Signal {
                    k.yield_cpu();
                }
            }
            Poll::Pending | Poll::Exiting => {
                if k.wait_tree(generation) == Wakeup::Signal {
                    return Err(Errno::RESTART_IF_SA_RESTART);
                }
            }
        }
    }
}

/// Resource usage of the child `pid` (global) the caller has just waited
/// for: its CPU time and that of its own waited-for children (ADR 0059);
/// the other fields are not accounted and stay 0.
fn write_rusage(k: &mut dyn LinuxKernel, ptr: UserPtr, pid: i32) -> Result<(), Errno> {
    use crate::time::abi::{Rusage, Timeval};
    let t = k.cpu_times(crate::kernel::CpuTimesOf::Reaped(pid)).unwrap_or_default();
    if ptr.is_null() {
        return Ok(());
    }
    let usage =
        Rusage { utime: Timeval::from_nanos(t.user_ns), stime: Timeval::from_nanos(t.system_ns), ..Rusage::default() };
    k.copy_to_user(ptr, &usage.to_bytes())
}

/// wait4(pid, wstatus, options, rusage): the child's pid and its status
/// word (0 with `WNOHANG` and nothing to report). The child is taken
/// before the status is stored, so a bad pointer loses it (`EFAULT`; to be
/// confirmed by the diff runner).
pub fn wait4(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let pgid = view(k, k.pgid());
    let (mut filter, nohang) = wait::wait4_filter(a.int(0), a.uint(2), pgid)?;
    filter.selector = global_selector(k, filter.selector);
    let Some((w, local)) = wait_for(k, &filter, nohang)? else { return Ok(0) };
    if !a.ptr(1).is_null() {
        k.copy_to_user(a.ptr(1), &wait::status_word(w.event).to_le_bytes())?;
    }
    write_rusage(k, a.ptr(3), w.pid)?;
    pid_result(local)
}

/// waitid(idtype, id, infop, options, rusage): 0, with the child's
/// `siginfo_t` (`SIGCHLD`, `CLD_*`, pid, uid, status) in `infop`; with
/// `WNOHANG` and nothing to report, `infop` is zeroed (wait(2) tells
/// callers to check `si_pid` for 0).
///
/// `P_PIDFD` (pidfd_open(2)): `id` is a pidfd and the child it names is
/// waited for (`EBADF` for a descriptor that is not open, `EINVAL` for one
/// that is no pidfd); once that child is reaped the wait is `ECHILD`.
pub fn waitid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let pgid = view(k, k.pgid());
    let (filter, nohang) = if a.uint(0) == wait::P_PIDFD {
        // The options are checked as for a pid (any positive id will do).
        let (filter, nohang) = wait::waitid_filter(wait::P_PID, 1, a.uint(3), pgid)?;
        let fd = a.int(1);
        if fd < 0 {
            return Err(Errno::EBADF);
        }
        let pid = match crate::sys::pipe::file_io(k)?.pidfd_target(fd)? {
            Some(crate::fileio::PidFdTarget::PidFd { pid, .. }) => pid,
            _ => return Err(Errno::EINVAL),
        };
        (WaitFilter { selector: Selector::Pid(pid), ..filter }, nohang)
    } else {
        let (mut filter, nohang) = wait::waitid_filter(a.uint(0), a.int(1), a.uint(3), pgid)?;
        filter.selector = global_selector(k, filter.selector);
        (filter, nohang)
    };
    let waited = wait_for(k, &filter, nohang)?;
    let found = waited.map(|(w, local)| Waited { pid: local, ..w });
    if !a.ptr(2).is_null() {
        let info = found.map_or([0; SigInfo::SIZE], |w| wait::sigchld(&w).to_bytes());
        k.copy_to_user(a.ptr(2), &info)?;
    }
    if let Some((w, _)) = waited {
        write_rusage(k, a.ptr(4), w.pid)?;
    }
    Ok(0)
}

/// A process id argument where 0 means the caller: the global id, `ESRCH`
/// for one the caller's pid namespace does not have.
fn pid_arg(k: &dyn LinuxKernel, pid: i32) -> Result<i32, Errno> {
    if pid <= 0 { Ok(pid) } else { find(k, pid) }
}

/// setpgid(pid, pgid) (setpgid(2)). A group the caller's namespace does
/// not have cannot exist in its session either (`EPERM`).
pub fn setpgid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let me = k.pid();
    let pid = pid_arg(k, a.int(0))?;
    let pgid = match a.int(1) {
        g if g > 0 => find(k, g).map_err(|_| Errno::EPERM)?,
        g => g,
    };
    with_tree(k, |t| t.setpgid(me, pid, pgid))??;
    Ok(0)
}

/// getpgid(pid): the process group of `pid` (0: the caller).
pub fn getpgid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let me = k.pid();
    let pid = pid_arg(k, a.int(0))?;
    let pid = thread_group_of(k, pid);
    let g = with_tree(k, |t| t.getpgid(me, pid))??;
    seen(k, g)
}

/// getpgrp(): the caller's process group; "always successful".
pub fn getpgrp(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    seen(k, k.pgid())
}

/// setsid(): a new session led by the caller; returns its id.
pub fn setsid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    let me = k.pid();
    let s = with_tree(k, |t| t.setsid(me))??;
    seen(k, s)
}

/// getsid(pid): the session of `pid` (0: the caller).
pub fn getsid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let me = k.pid();
    let pid = pid_arg(k, a.int(0))?;
    let pid = thread_group_of(k, pid);
    let s = with_tree(k, |t| t.getsid(me, pid))??;
    seen(k, s)
}

/// The process of thread `pid` (a global id; a process's own id, 0 and
/// negative ids stay as they are): getpgid and getsid of a thread id
/// answer for its process (the spec sheets of getpgid and getsid). A
/// zombie has no threads left and is found by its pid.
fn thread_group_of(k: &mut dyn LinuxKernel, pid: i32) -> i32 {
    if pid <= 0 {
        return pid;
    }
    let mut tgid = pid;
    let _ = k.with_process(crate::kernel::Target::Thread(pid), &mut |_, info| tgid = info.pid);
    tgid
}
