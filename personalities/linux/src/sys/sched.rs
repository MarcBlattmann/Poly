//! Scheduling calls: sched_yield(2), sched_setaffinity(2) and
//! sched_getaffinity(2) (ADR 0053).
//!
//! An affinity mask is a bit set of CPU numbers in memory, `cpusetsize`
//! bytes long, CPU `n` in bit `n % 8` of byte `n / 8` (CPU_SET(3)). The
//! kernel's mask is [`CPU_MASK_BYTES`] bytes (64 CPUs):
//!
//! - **sched_setaffinity(pid, cpusetsize, mask)**: `pid` 0 is the calling
//!   thread, otherwise a thread id. The first `cpusetsize` bytes are read
//!   (`EFAULT`); bits beyond the kernel's mask are ignored. A mask with no
//!   CPU the system has is `EINVAL` ("the affinity bit mask mask contains no
//!   processors that are currently physically on the system and permitted
//!   to the thread"), an unknown thread `ESRCH`, a thread the caller may not
//!   change `EPERM` (the kernel decides).
//! - **sched_getaffinity(pid, cpusetsize, mask)**: `EINVAL` when
//!   `cpusetsize` is smaller than the kernel's mask ("cpusetsize is smaller
//!   than the size of the affinity mask used by the kernel"); writes the
//!   kernel's mask and returns its size in bytes (the man page's "C
//!   library/kernel differences": the raw call returns the number of bytes
//!   copied; the C library clears the rest of the caller's buffer).
//! - **sched_yield()**: gives up the CPU; always 0.
//!
//! Points the man pages leave open, ours to be confirmed by the diff
//! runner: a negative `pid` is `ESRCH`; the order of the checks (size, then
//! the thread, then reading the mask).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

/// Bytes of the kernel's CPU mask: 64 CPUs.
pub const CPU_MASK_BYTES: usize = 8;

/// sched_yield(): let another thread run.
pub fn sched_yield(k: &mut dyn LinuxKernel, _a: &Args) -> SysResult {
    k.yield_cpu();
    Ok(0)
}

/// The thread a `pid` argument names: 0 is the caller.
fn target(k: &dyn LinuxKernel, pid: i32) -> Result<i32, Errno> {
    match pid {
        0 => Ok(k.tid()),
        p if p < 0 => Err(Errno::ESRCH),
        p => Ok(p),
    }
}

/// sched_setaffinity(pid, cpusetsize, mask).
pub fn sched_setaffinity(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let tid = target(k, a.int(0))?;
    let len = (a.ulong(1) as usize).min(CPU_MASK_BYTES);
    let mut bytes = [0u8; CPU_MASK_BYTES];
    k.copy_from_user(a.ptr(2), &mut bytes[..len])?;
    let mask = u64::from_le_bytes(bytes);
    if mask == 0 {
        return Err(Errno::EINVAL);
    }
    k.set_cpu_affinity(tid, mask)?;
    Ok(0)
}

/// sched_getaffinity(pid, cpusetsize, mask).
pub fn sched_getaffinity(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if (a.ulong(1) as usize) < CPU_MASK_BYTES {
        return Err(Errno::EINVAL);
    }
    let tid = target(k, a.int(0))?;
    let mask = k.cpu_affinity(tid)?;
    k.copy_to_user(a.ptr(2), &mask.to_le_bytes())?;
    Ok(CPU_MASK_BYTES as u64)
}

pub const CALLS: &[(u64, Handler)] = &[
    (nr::SCHED_YIELD, sched_yield),
    (nr::SCHED_SETAFFINITY, sched_setaffinity),
    (nr::SCHED_GETAFFINITY, sched_getaffinity),
];
