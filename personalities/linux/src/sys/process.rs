//! Process and thread state: exit(2), exit_group(2), the id getters,
//! set_tid_address(2), arch_prctl(2).

use crate::abi::{USER_ADDRESS_LIMIT, arch_prctl as op};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::signal::deliver::with_own;

/// exit(status): ends the calling thread only (_exit(2): "the raw Linux
/// _exit() system call terminates only the calling thread"); the parent
/// sees `status & 0377`.
pub fn exit(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    k.exit_thread(a.int(0) as u8);
    Ok(0)
}

/// exit_group(status): "terminates all threads in the calling process's
/// thread group" (exit_group(2)); the status is `status & 0377` as for
/// exit.
pub fn exit_group(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    k.exit_group(a.int(0) as u8);
    Ok(0)
}

/// A `pid_t` result (sign-extended, as the ABI returns `int` values).
fn pid_result(id: i32) -> SysResult {
    Ok(id as i64 as u64)
}

/// getpid(): the thread group id. getpid(2): "always successful".
pub fn getpid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    pid_result(k.pid())
}

/// gettid(): the caller's thread id. gettid(2): "always successful".
pub fn gettid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    pid_result(k.tid())
}

/// getppid(): the parent's process id.
pub fn getppid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    pid_result(k.ppid())
}

/// getuid(): the real user id. getuid(2): "always successful".
pub fn getuid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    Ok(k.credentials().uid as u64)
}

/// geteuid(): the effective user id.
pub fn geteuid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    Ok(k.credentials().euid as u64)
}

/// getgid(): the real group id. getgid(2): "always successful".
pub fn getgid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    Ok(k.credentials().gid as u64)
}

/// getegid(): the effective group id.
pub fn getegid(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    Ok(k.credentials().egid as u64)
}

/// set_tid_address(tidptr): stores `tidptr` as the thread's
/// `clear_child_tid` and "always returns the caller's thread ID"
/// (set_tid_address(2)). The pointer is not checked here: it is only used
/// when the thread exits ([`crate::futex::thread_exit`]), and errors then
/// are ignored.
pub fn set_tid_address(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let tid = k.tid();
    let ptr = a.ptr(0);
    with_own(k, |p| p.thread_mut(tid).clear_child_tid = ptr);
    pid_result(tid)
}

/// arch_prctl(op, addr)
///
/// arch_prctl(2): `ARCH_SET_FS` / `ARCH_SET_GS` set the segment base to
/// `addr`, `EPERM` if "addr is outside the process address space" (here:
/// at or above [`USER_ADDRESS_LIMIT`], where a base could only point into
/// the kernel or be non-canonical); `ARCH_GET_FS` / `ARCH_GET_GS` store the
/// base in the `unsigned long` at `addr`, `EFAULT` if that cannot be
/// written; any other `op` is `EINVAL`. (The CPUID-faulting, AMX and vDSO
/// operations are not supported yet and also get `EINVAL`.)
pub fn arch_prctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(1);
    match a.uint(0) {
        op::ARCH_SET_FS | op::ARCH_SET_GS if addr >= USER_ADDRESS_LIMIT => Err(Errno::EPERM),
        op::ARCH_SET_FS => {
            k.set_fs_base(addr);
            Ok(0)
        }
        op::ARCH_SET_GS => {
            k.set_gs_base(addr);
            Ok(0)
        }
        op::ARCH_GET_FS => {
            let base = k.fs_base();
            k.copy_to_user(a.ptr(1), &base.to_le_bytes())?;
            Ok(0)
        }
        op::ARCH_GET_GS => {
            let base = k.gs_base();
            k.copy_to_user(a.ptr(1), &base.to_le_bytes())?;
            Ok(0)
        }
        _ => Err(Errno::EINVAL),
    }
}
