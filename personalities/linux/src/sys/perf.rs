//! perf_event_open(2), the `PERF_EVENT_IOC_*` ioctls, and the counters'
//! life with their process: prctl(2) `PR_TASK_PERF_EVENTS_DISABLE` /
//! `_ENABLE`, inheritance by children, children's counts at their exit
//! (ADR 0135). The counters are in [`crate::perf`].

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{CpuTimesOf, LinuxKernel, UserPtr};
use crate::perf::{self, Attr, Counter, bit, sw};
use crate::signal::deliver::with_own;
use crate::sys::procaccess;
use crate::table::{Handler, nr};
use alloc::sync::Arc;
use alloc::vec;

pub const CALLS: &[(u64, Handler)] = &[(nr::PERF_EVENT_OPEN, perf_event_open)];

/// `PERF_EVENT_IOC_*` (`linux/perf_event.h`).
pub mod ioc {
    pub const ENABLE: u32 = 0x2400;
    pub const DISABLE: u32 = 0x2401;
    pub const REFRESH: u32 = 0x2402;
    pub const RESET: u32 = 0x2403;
    pub const PERIOD: u32 = 0x4008_2404;
    pub const SET_OUTPUT: u32 = 0x2405;
    pub const SET_FILTER: u32 = 0x4008_2406;
    pub const ID: u32 = 0x8008_2407;
    pub const SET_BPF: u32 = 0x4004_2408;
    pub const PAUSE_OUTPUT: u32 = 0x4004_2409;
    pub const QUERY_BPF: u32 = 0xc008_240a;
    pub const MODIFY_ATTRIBUTES: u32 = 0x4008_240b;
    /// `PERF_IOC_FLAG_GROUP`.
    pub const FLAG_GROUP: u64 = 1;
}

/// prctl(2) options (`linux/prctl.h`).
pub const PR_TASK_PERF_EVENTS_DISABLE: u32 = 31;
pub const PR_TASK_PERF_EVENTS_ENABLE: u32 = 32;

/// Read the caller's `struct perf_event_attr`: its `size` decides how
/// much (`E2BIG` for sizes out of range and for non-zero bytes past the
/// structure Poly knows; the kernel's size is then written to
/// `attr->size`).
fn read_attr(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<Attr, Errno> {
    if ptr.is_null() {
        return Err(Errno::EFAULT);
    }
    let mut head = [0u8; 8];
    k.copy_from_user(ptr, &mut head)?;
    let size = u32::from_le_bytes(head[4..8].try_into().expect("4 bytes"));
    let too_big = |k: &mut dyn LinuxKernel| {
        let _ = k.copy_to_user(UserPtr(ptr.0 + 4), &(perf::ATTR_SIZE as u32).to_le_bytes());
        Err(Errno::E2BIG)
    };
    let len = match perf::attr_len(size) {
        Ok(len) => len,
        Err(_) => return too_big(k),
    };
    let mut b = vec![0u8; len];
    k.copy_from_user(ptr, &mut b)?;
    if len > perf::ATTR_SIZE && b[perf::ATTR_SIZE..].iter().any(|&x| x != 0) {
        return too_big(k);
    }
    Ok(Attr::parse(&b))
}

/// perf_event_open(attr, pid, cpu, group_fd, flags): checks in the spec
/// sheet's order (flags, the attribute, pid and cpu, permission, the
/// group, the event, the task), then a new counter on a new descriptor.
pub fn perf_event_open(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (pid, cpu, group_fd, flags) = (a.int(1), a.int(2), a.int(3), a.ulong(4));
    if flags & !perf::FLAGS_ALL != 0 {
        return Err(Errno::EINVAL);
    }
    let attr = read_attr(k, a.ptr(0))?;
    attr.check()?;
    // Poly has no cgroup file system: pid is never a cgroup descriptor.
    if flags & perf::FLAG_PID_CGROUP != 0 || (pid == -1 && cpu == -1) || pid < -1 {
        return Err(Errno::EINVAL);
    }
    if cpu < -1 || cpu >= k.cpu_count() as i32 {
        return Err(Errno::EINVAL);
    }
    // kernel.perf_event_paranoid 2: without CAP_PERFMON (or
    // CAP_SYS_ADMIN) only user-space counting of one's own tasks.
    let c = cred::current(k);
    let privileged = c.capable(cap::PERFMON) || c.capable(cap::SYS_ADMIN);
    if !privileged && (!attr.has(bit::EXCLUDE_KERNEL) || pid == -1) {
        return Err(Errno::EACCES);
    }
    let leader = match group_fd {
        -1 => None,
        fd => {
            let l = k.perf_counter(fd)?.ok_or(Errno::EBADF)?;
            if l.is_member() {
                return Err(Errno::EINVAL);
            }
            Some(l)
        }
    };
    attr.check_event()?;
    let target = if pid > 0 {
        let found = procaccess::find(k, pid, true)?;
        if !privileged && !procaccess::may_access(k.pid(), c.uid, c.gid, false, &found) {
            return Err(Errno::EACCES);
        }
        found.pid
    } else {
        0
    };
    let source = k.perf_source(target)?;
    let counter = Counter::new(attr, source, leader.as_ref());
    let fd = k.perf_install(Arc::clone(&counter), flags & perf::FLAG_FD_CLOEXEC != 0)?;
    with_own(k, |p| p.perf.own(&counter, target == 0 && attr.has(bit::INHERIT)));
    Ok(fd as u64)
}

/// The `PERF_EVENT_IOC_*` requests on a perf event descriptor; `None` for
/// other descriptors.
pub fn ioctl(k: &mut dyn LinuxKernel, fd: i32, request: u32, arg: u64) -> Option<SysResult> {
    let counter = k.perf_counter(fd).ok().flatten()?;
    Some(ioctl_on(k, &counter, request, arg))
}

fn ioctl_on(k: &mut dyn LinuxKernel, c: &Arc<Counter>, request: u32, arg: u64) -> SysResult {
    let group = arg & ioc::FLAG_GROUP != 0;
    match request {
        // Other argument values than PERF_IOC_FLAG_GROUP are ignored.
        ioc::ENABLE => c.targets(group).iter().for_each(|c| c.enable()),
        ioc::DISABLE => c.targets(group).iter().for_each(|c| c.disable()),
        ioc::RESET => c.targets(group).iter().for_each(|c| c.reset()),
        ioc::REFRESH if !c.attr.sampling() => return Err(Errno::EINVAL),
        ioc::REFRESH => c.enable(),
        ioc::PERIOD => {
            let mut b = [0u8; 8];
            k.copy_from_user(UserPtr(arg), &mut b)?;
            let period = u64::from_le_bytes(b);
            if !c.attr.sampling() || period & (1 << 63) != 0 {
                return Err(Errno::EINVAL);
            }
            c.set_period(period);
        }
        ioc::SET_OUTPUT => {
            if arg == u64::MAX {
                c.set_output(None);
            } else {
                let to = k.perf_counter(arg as i32)?.ok_or(Errno::EINVAL)?;
                if Arc::ptr_eq(&to, c) || !to.has_ring() {
                    return Err(Errno::EINVAL);
                }
                c.set_output(Some(&to));
            }
        }
        // Filters are for tracepoints; BPF programs attach to them too.
        ioc::SET_FILTER | ioc::SET_BPF | ioc::QUERY_BPF => return Err(Errno::EINVAL),
        ioc::ID => k.copy_to_user(UserPtr(arg), &c.id.to_le_bytes())?,
        ioc::PAUSE_OUTPUT if !c.has_ring() => return Err(Errno::EINVAL),
        ioc::PAUSE_OUTPUT => c.pause(arg != 0),
        ioc::MODIFY_ATTRIBUTES => return Err(Errno::EOPNOTSUPP),
        _ => return Err(Errno::ENOTTY),
    }
    Ok(0)
}

/// prctl(2) `PR_TASK_PERF_EVENTS_DISABLE` / `_ENABLE`: every counter the
/// calling process opened stops or goes on.
pub fn task_events(k: &mut dyn LinuxKernel, enable: bool) {
    let owned = with_own(k, |p| p.perf.owned());
    for c in owned {
        if enable {
            c.enable();
        } else {
            c.disable();
        }
    }
}

/// The calling process is ending: the counters it inherited get its
/// counts.
pub fn process_exit(k: &mut dyn LinuxKernel) {
    let inherited = with_own(k, |p| p.perf.inherited());
    if inherited.is_empty() {
        return;
    }
    let times = k.cpu_times(CpuTimesOf::Process).unwrap_or_default();
    let faults = k.memory_counts(CpuTimesOf::Process).minor_faults;
    for c in inherited {
        let total = match c.attr.config {
            sw::CPU_CLOCK | sw::TASK_CLOCK => times.user_ns + times.system_ns,
            sw::PAGE_FAULTS | sw::PAGE_FAULTS_MIN => faults,
            _ => 0,
        };
        c.add_child(total);
    }
}
