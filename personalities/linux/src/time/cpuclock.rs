//! CPU-time clocks named by id (ADR 0064): clock_getcpuclockid(3) and
//! pthread_getcpuclockid(3) hand out negative clock ids that name another
//! process's or thread's CPU-time clock. The encoding is an ABI fact
//! (clock_gettime(2) NOTES and the spec of `clock_getres`): `(~pid << 3) |
//! type`, where bit 2 of `type` says "thread" and its low two bits what is
//! counted (0: user and system time, "prof"; 1: user time, "virt"; 2: user
//! and system time, "sched"); type 3 (`(~fd << 3) | 3`) names a dynamic
//! clock behind a file descriptor, which Poly has none of. An id of 0 means
//! the caller (its process, or its thread).

use crate::errno::Errno;
use crate::kernel::{CpuTimes, CpuTimesOf, LinuxKernel};

/// What a CPU-time clock counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counts {
    /// User and system time.
    All,
    /// User time only.
    User,
}

/// A CPU-time clock a negative clock id names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuClock {
    pub of: CpuTimesOf,
    pub counts: Counts,
}

impl CpuClock {
    /// The clock's value in nanoseconds.
    pub fn read_ns(self, k: &dyn LinuxKernel) -> Result<u64, Errno> {
        let t: CpuTimes = k.cpu_times(self.of).ok_or(Errno::EINVAL)?;
        Ok(match self.counts {
            Counts::All => t.total_ns(),
            Counts::User => t.user_ns,
        })
    }
}

/// The CPU-time clock `clockid` names, if it is a negative (encoded) id:
/// `None` for the other ids; `EINVAL` for a dynamic (file descriptor)
/// clock, an invalid type, or a process or thread that does not exist (a
/// thread must be one of the caller's process).
pub fn decode(k: &dyn LinuxKernel, clockid: i32) -> Option<Result<CpuClock, Errno>> {
    if clockid >= 0 {
        return None;
    }
    let id = !(clockid >> 3);
    let kind = clockid & 7;
    let counts = match kind & 3 {
        0 | 2 => Counts::All,
        1 => Counts::User,
        _ => return Some(Err(Errno::EINVAL)),
    };
    let of = if kind & 4 != 0 {
        CpuTimesOf::Thread(if id == 0 { k.tid() } else { id })
    } else if id == 0 || id == k.pid() {
        CpuTimesOf::Process
    } else {
        CpuTimesOf::OtherProcess(id)
    };
    let clock = CpuClock { of, counts };
    Some(match k.cpu_times(of) {
        Some(_) => Ok(clock),
        None => Err(Errno::EINVAL),
    })
}
