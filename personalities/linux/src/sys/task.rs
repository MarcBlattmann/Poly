//! Creating, replacing and waiting for processes and threads: fork(2),
//! vfork(2), clone(2), clone3(2), execve(2), wait4(2), waitid(2), and
//! process groups and sessions: setpgid(2), getpgid(2), getpgrp(2),
//! setsid(2), getsid(2). The rules are in [`crate::task`]; see ADR 0041.

use crate::abi::{SigInfo, USER_ADDRESS_LIMIT};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr, Wakeup};
use crate::task::clone::{self, CloneArgs, CloneRequest};
use crate::task::exec::{self, ExecRequest};
use crate::task::tree::{Poll, WaitFilter, Waited};
use crate::task::wait;
use crate::task::with_tree;

/// `AT_FDCWD` (`linux/fcntl.h`).
const AT_FDCWD: i32 = -100;

fn pid_result(id: i32) -> SysResult {
    Ok(id as i64 as u64)
}

/// Check what the kernel cannot (a TLS base must be a user address, as for
/// arch_prctl(2): `EPERM`; to be confirmed by the diff runner) and create
/// the child.
fn spawn(k: &mut dyn LinuxKernel, req: &CloneRequest) -> SysResult {
    if req.tls.is_some_and(|tls| tls >= USER_ADDRESS_LIMIT) {
        return Err(Errno::EPERM);
    }
    k.clone_task(req).and_then(pid_result)
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
    let req = clone::decode_clone(a.ulong(0), a.ulong(1), a.ptr(2), a.ptr(3), a.ulong(4), k.pid())?;
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
    if size > known {
        let mut rest = [0u8; crate::abi::PAGE_SIZE as usize];
        let at = a.ptr(0).offset(known as u64).ok_or(Errno::EFAULT)?;
        k.copy_from_user(at, &mut rest[..size - known])?;
        extra_nonzero = rest[..size - known].iter().any(|&b| b != 0);
    }
    let req = clone::decode_clone3(&CloneArgs::from_bytes(&bytes[..known]), extra_nonzero, k.pid())?;
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
    k.execve(&ExecRequest { dirfd: AT_FDCWD, path: &path, argv: &argv, envp: &envp })?;
    let pid = k.pid();
    let _ = with_tree(k, |t| t.exec(pid));
    Ok(0)
}

/// Wait until a child matches `filter` (`None` at once with `nohang`).
/// `ECHILD` if no child matches at all. A signal for the caller ends the
/// wait; signal(7) lists wait(2) among the calls `SA_RESTART` restarts.
fn wait_for(k: &mut dyn LinuxKernel, filter: &WaitFilter, nohang: bool) -> Result<Option<Waited>, Errno> {
    let me = k.pid();
    loop {
        let (generation, poll) = with_tree(k, |t| (t.generation(), t.poll(me, filter)))?;
        match poll {
            Poll::Ready(w) => return Ok(Some(w)),
            Poll::NoChildren => return Err(Errno::ECHILD),
            Poll::Pending if nohang => return Ok(None),
            Poll::Pending => {
                if k.wait_tree(generation) == Wakeup::Signal {
                    return Err(Errno::RESTART_IF_SA_RESTART);
                }
            }
        }
    }
}

/// Resource usage of a child: all zero (CPU time and the rest are not
/// accounted yet).
fn write_rusage(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<(), Errno> {
    if ptr.is_null() { Ok(()) } else { k.copy_to_user(ptr, &[0; wait::RUSAGE_SIZE]) }
}

/// wait4(pid, wstatus, options, rusage): the child's pid and its status
/// word (0 with `WNOHANG` and nothing to report). The child is taken
/// before the status is stored, so a bad pointer loses it (`EFAULT`; to be
/// confirmed by the diff runner).
pub fn wait4(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let pgid = k.pgid();
    let (filter, nohang) = wait::wait4_filter(a.int(0), a.uint(2), pgid)?;
    let Some(w) = wait_for(k, &filter, nohang)? else { return Ok(0) };
    if !a.ptr(1).is_null() {
        k.copy_to_user(a.ptr(1), &wait::status_word(w.event).to_le_bytes())?;
    }
    write_rusage(k, a.ptr(3))?;
    pid_result(w.pid)
}

/// waitid(idtype, id, infop, options, rusage): 0, with the child's
/// `siginfo_t` (`SIGCHLD`, `CLD_*`, pid, uid, status) in `infop`; with
/// `WNOHANG` and nothing to report, `infop` is zeroed (wait(2) tells
/// callers to check `si_pid` for 0).
pub fn waitid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let pgid = k.pgid();
    let (filter, nohang) = wait::waitid_filter(a.uint(0), a.int(1), a.uint(3), pgid)?;
    let found = wait_for(k, &filter, nohang)?;
    if !a.ptr(2).is_null() {
        let info = found.map_or([0; SigInfo::SIZE], |w| wait::sigchld(&w).to_bytes());
        k.copy_to_user(a.ptr(2), &info)?;
    }
    if found.is_some() {
        write_rusage(k, a.ptr(4))?;
    }
    Ok(0)
}

/// setpgid(pid, pgid) (setpgid(2)).
pub fn setpgid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let me = k.pid();
    with_tree(k, |t| t.setpgid(me, a.int(0), a.int(1)))??;
    Ok(0)
}

/// getpgid(pid): the process group of `pid` (0: the caller).
pub fn getpgid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let me = k.pid();
    with_tree(k, |t| t.getpgid(me, a.int(0)))?.and_then(pid_result)
}

/// getpgrp(): the caller's process group; "always successful".
pub fn getpgrp(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    pid_result(k.pgid())
}

/// setsid(): a new session led by the caller; returns its id.
pub fn setsid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    let me = k.pid();
    with_tree(k, |t| t.setsid(me))?.and_then(pid_result)
}

/// getsid(pid): the session of `pid` (0: the caller).
pub fn getsid(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let me = k.pid();
    with_tree(k, |t| t.getsid(me, a.int(0)))?.and_then(pid_result)
}
