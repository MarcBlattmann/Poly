//! timerfd_create(2), timerfd_settime(2), timerfd_gettime(2). The timer
//! (expirations, reads, readiness) is the kernel's
//! (`poly_vfs::timerfd`); here are the argument rules.

use super::pipe::file_io;
use crate::abi::clock;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{ItimerSpec, NewFd, flags};
use crate::kernel::{Clock, LinuxKernel};

/// `TFD_TIMER_ABSTIME` (`linux/timerfd.h`).
pub const TFD_TIMER_ABSTIME: u32 = 1 << 0;
/// `TFD_TIMER_CANCEL_ON_SET`.
pub const TFD_TIMER_CANCEL_ON_SET: u32 = 1 << 1;

/// `CLOCK_REALTIME_ALARM` and `CLOCK_BOOTTIME_ALARM` (`linux/time.h`).
const CLOCK_REALTIME_ALARM: i32 = 8;
const CLOCK_BOOTTIME_ALARM: i32 = 9;

/// timerfd_create(clockid, flags): timerfd_create(2) lists
/// `CLOCK_REALTIME`, `CLOCK_MONOTONIC`, `CLOCK_BOOTTIME` and the two alarm
/// clocks; anything else, or flags beside `TFD_NONBLOCK` and
/// `TFD_CLOEXEC`, is `EINVAL`. Nothing suspends, so `CLOCK_BOOTTIME` is
/// the monotonic clock and the alarm clocks are their plain ones (the
/// `EPERM` for callers without `CAP_WAKE_ALARM` needs capabilities: root
/// passes).
pub fn timerfd_create(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let which = match a.int(0) {
        clock::CLOCK_REALTIME | CLOCK_REALTIME_ALARM => Clock::Realtime,
        clock::CLOCK_MONOTONIC | clock::CLOCK_BOOTTIME | CLOCK_BOOTTIME_ALARM => Clock::Monotonic,
        _ => return Err(Errno::EINVAL),
    };
    let bits = a.uint(1);
    if bits & !(flags::O_CLOEXEC | flags::O_NONBLOCK) != 0 {
        return Err(Errno::EINVAL);
    }
    file_io(k)?.timerfd_create(which, NewFd::from_flags(bits)).map(|fd| fd as u64)
}

/// timerfd_settime(fd, flags, new_value, old_value): `EINVAL` for unknown
/// flags or a `new_value` whose `tv_nsec` is outside 0..999,999,999 (or
/// negative seconds); `EFAULT`. `TFD_TIMER_CANCEL_ON_SET` is accepted;
/// the wall clock cannot be set yet, so such a timer is never cancelled.
/// `old_value` (if not null) gets the old setting.
pub fn timerfd_settime(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    let bits = a.uint(1);
    if bits & !(TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET) != 0 {
        return Err(Errno::EINVAL);
    }
    let mut raw = [0; ItimerSpec::SIZE];
    k.copy_from_user(a.ptr(2), &mut raw)?;
    let new = ItimerSpec::from_bytes(&raw);
    if !new.value.is_valid() || !new.interval.is_valid() {
        return Err(Errno::EINVAL);
    }
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let old = file_io(k)?.timerfd_settime(fd, bits & TFD_TIMER_ABSTIME != 0, &new)?;
    if !a.ptr(3).is_null() {
        k.copy_to_user(a.ptr(3), &old.to_bytes())?;
    }
    Ok(0)
}

/// timerfd_gettime(fd, curr_value)
pub fn timerfd_gettime(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let cur = file_io(k)?.timerfd_gettime(fd)?;
    k.copy_to_user(a.ptr(1), &cur.to_bytes())?;
    Ok(0)
}
