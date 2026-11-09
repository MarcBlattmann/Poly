//! POSIX message queues: mq_open(2), mq_unlink(2), mq_timedsend(2),
//! mq_timedreceive(2), mq_notify(2), mq_getsetattr(2) (mq_overview(7)).
//!
//! The queues and their names are [`crate::ipc::mqueue`]'s; the kernel
//! gives a queue its descriptors ([`FileIo::mq_install`],
//! [`FileIo::mq_file`]: an open file on a regular-file inode whose read
//! gives the status line). These handlers check the arguments in the order
//! the spec sheets give, copy user memory, sleep, wake the threads a queue
//! operation completed and send the notification signal.
//!
//! [`FileIo::mq_install`]: crate::fileio::FileIo::mq_install
//! [`FileIo::mq_file`]: crate::fileio::FileIo::mq_file

use super::pipe::file_io;
use crate::abi::{SigInfo, Timespec};
use crate::args::Args;
use crate::cred::cap;
use crate::errno::{Errno, SysResult};
use crate::ipc::mqueue::{
    Caller, Done, MqAttr, MqFd, NAME_MAX, OpenRequest, Queue, ReceiveStep, Registration, SI_MESGQ, SIGEV_SIGNAL,
    SIGEV_THREAD, SendStep, SigEvent, check_name, oflag, with_names,
};
use crate::kernel::{Clock, Deadline, LinuxKernel, Target, UserPtr};
use crate::ns::user::ns_capable;
use crate::signal::Dest;
use crate::signal::deliver::{complete, with_own, with_target};
use crate::table::{Handler, nr};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::MQ_OPEN, mq_open),
    (nr::MQ_UNLINK, mq_unlink),
    (nr::MQ_TIMEDSEND, mq_timedsend),
    (nr::MQ_TIMEDRECEIVE, mq_timedreceive),
    (nr::MQ_NOTIFY, mq_notify),
    (nr::MQ_GETSETATTR, mq_getsetattr),
];

/// The caller's IPC namespace and who it is there.
fn caller(k: &mut dyn LinuxKernel) -> (Option<Arc<crate::ns::IpcNs>>, Caller) {
    let (ns, creds, _) = crate::sys::shm::caller(k);
    let owner = ns.as_ref().and_then(|n| n.owner().clone());
    let who = Caller {
        euid: creds.euid,
        egid: creds.egid,
        groups: creds.groups.clone(),
        dac_override: ns_capable(&creds, &owner, cap::DAC_OVERRIDE),
        fowner: ns_capable(&creds, &owner, cap::FOWNER),
        sys_resource: ns_capable(&creds, &owner, cap::SYS_RESOURCE),
    };
    (ns, who)
}

/// A queue name: `EFAULT` if unreadable, `ENAMETOOLONG` above
/// [`NAME_MAX`] bytes.
fn read_name(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<Vec<u8>, Errno> {
    let mut buf = vec![0u8; NAME_MAX + 1];
    let len = k.strncpy_from_user(ptr, &mut buf)?;
    buf.truncate(len);
    Ok(buf)
}

/// The descriptor `fd` as a queue descriptor: `EBADF` if it is not open
/// or no queue.
fn queue_fd(k: &mut dyn LinuxKernel, fd: i32) -> Result<MqFd, Errno> {
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    file_io(k)?.mq_file(fd)?.ok_or(Errno::EBADF)
}

/// An `abs_timeout` argument: none if null; `EFAULT` if unreadable,
/// `EINVAL` for a negative `tv_sec` or a `tv_nsec` outside 0..1e9.
fn read_timeout(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<Option<Timespec>, Errno> {
    if ptr.is_null() {
        return Ok(None);
    }
    let mut b = [0u8; 16];
    k.copy_from_user(ptr, &mut b)?;
    let t = Timespec {
        sec: i64::from_le_bytes(b[0..8].try_into().unwrap_or([0; 8])),
        nsec: i64::from_le_bytes(b[8..16].try_into().unwrap_or([0; 8])),
    };
    valid_timeout(t)?;
    Ok(Some(t))
}

/// `EINVAL` for a negative second count or nanoseconds outside 0..1e9.
pub fn valid_timeout(t: Timespec) -> Result<(), Errno> {
    if t.sec < 0 || !(0..1_000_000_000).contains(&t.nsec) {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// Wait for `ticket` of `q` to complete, until the absolute
/// `CLOCK_REALTIME` time `deadline` (`ETIMEDOUT`; one already past expires
/// at once) or a signal (restarted under `SA_RESTART` with the same
/// absolute deadline, else `EINTR`). A result that came meanwhile wins.
fn sleep(k: &mut dyn LinuxKernel, q: &Queue, ticket: u64, deadline: Option<Timespec>) -> Result<Done, Errno> {
    let tid = k.tid();
    loop {
        k.clear_signal_notification();
        if let Some(done) = q.poll(ticket) {
            return Ok(done);
        }
        if with_own(k, |p| p.has_deliverable(tid)) {
            return q.cancel(ticket).ok_or(Errno::RESTART_IF_SA_RESTART);
        }
        if let Some(at) = deadline {
            let now = k.clock(Clock::Realtime)?;
            if (now.sec, now.nsec) >= (at.sec, at.nsec) {
                return q.cancel(ticket).ok_or(Errno::ETIMEDOUT);
            }
        }
        k.block(deadline.map(|at| Deadline { clock: Clock::Realtime, at }));
    }
}

fn wake(k: &mut dyn LinuxKernel, tids: &[i32]) {
    for &tid in tids {
        k.unblock(tid);
    }
}

/// Send the notification of `reg` for a message the caller sent:
/// `SIGEV_SIGNAL` with a signal sends it to the registered process with
/// `si_code` `SI_MESGQ`, `si_value` the registration's and the sender's
/// pid and real uid (mq_notify(3)); permissions do not apply.
fn notify(k: &mut dyn LinuxKernel, reg: Registration) {
    if reg.event.notify != SIGEV_SIGNAL || reg.event.signo == 0 {
        return;
    }
    let uid = crate::cred::current(k).uid;
    let mut info = SigInfo::from_process(reg.event.signo as u32, SI_MESGQ, k.pid(), uid);
    info.fields[8..16].copy_from_slice(&reg.event.value.to_le_bytes());
    if let Ok(Ok((pid, sent))) =
        with_target(k, Target::Process(reg.pid), |p, t| p.send(info, Dest::Process, false).map(|s| (t.pid, s)))
    {
        complete(k, pid, &sent);
    }
}

/// Whether process `pid` is still running (not gone, not a zombie).
fn alive(k: &mut dyn LinuxKernel, pid: i32) -> bool {
    with_target(k, Target::Process(pid), |_, t| !t.zombie).unwrap_or(false)
}

/// mq_open(name, oflag, mode, attr)
///
/// In order: the name (`EFAULT`, `ENAMETOOLONG`), `attr` when `O_CREAT`
/// asks for one (`EFAULT`, read also when the queue exists), the name's
/// syntax (`EACCES`, `ENOENT`), then the namespace's names
/// ([`MqTable::open`](crate::ipc::mqueue::MqTable::open)). The descriptor
/// is always close-on-exec; its status flags are the access mode and
/// `O_NONBLOCK`.
pub fn mq_open(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (flags, mode, attr_ptr) = (a.uint(1), a.uint(2), a.ptr(3));
    let name = read_name(k, a.ptr(0))?;
    let attr = if flags & oflag::O_CREAT != 0 && !attr_ptr.is_null() {
        let mut b = [0u8; MqAttr::SIZE];
        k.copy_from_user(attr_ptr, &mut b)?;
        Some(MqAttr::from_bytes(&b))
    } else {
        None
    };
    check_name(&name)?;
    let umask = match k.fs() {
        Some(fs) => {
            let m = fs.umask(0);
            fs.umask(m);
            m
        }
        None => 0o022,
    };
    let (ns, who) = caller(k);
    let req =
        OpenRequest { name: &name, flags, mode: (mode & !umask & 0o777) as u16, attr, uid: who.euid, gid: who.egid };
    let queue = with_names(&ns, |t| t.open(&req, &who))?;
    let bits = flags & (oflag::O_ACCMODE | oflag::O_NONBLOCK);
    file_io(k)?.mq_install(queue, bits).map(|fd| fd as u64)
}

/// mq_unlink(name): the name (`EFAULT`, `ENAMETOOLONG`), its syntax
/// (`EACCES`, `ENOENT`), then `ENOENT` / `EPERM`
/// ([`MqTable::unlink`](crate::ipc::mqueue::MqTable::unlink)).
pub fn mq_unlink(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let name = read_name(k, a.ptr(0))?;
    check_name(&name)?;
    let (ns, who) = caller(k);
    with_names(&ns, |t| t.unlink(&name, &who)).map(|()| 0)
}

/// mq_timedsend(mqdes, msg_ptr, msg_len, msg_prio, abs_timeout)
///
/// In order: `abs_timeout` (`EFAULT`, `EINVAL`), `msg_prio` (`EINVAL` from
/// 32768), the descriptor (`EBADF`, also if not open for writing),
/// `msg_len` (`EMSGSIZE` above `mq_msgsize`), the message (`EFAULT`), then
/// the queue: full is `EAGAIN` with `O_NONBLOCK`, else a sleep.
pub fn mq_timedsend(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd, msg, len, prio) = (a.fd(0), a.ptr(1), a.ulong(2), a.uint(3));
    let deadline = read_timeout(k, a.ptr(4))?;
    if prio >= crate::ipc::mqueue::MQ_PRIO_MAX {
        return Err(Errno::EINVAL);
    }
    let f = queue_fd(k, fd)?;
    if !f.writable() {
        return Err(Errno::EBADF);
    }
    if len > f.queue.msgsize as u64 {
        return Err(Errno::EMSGSIZE);
    }
    let mut data = vec![0u8; len as usize];
    if len > 0 {
        k.copy_from_user(msg, &mut data)?;
    }
    let tid = k.tid();
    match f.queue.send(prio, data, f.nonblock(), tid)? {
        SendStep::Done { woken, fired } => {
            wake(k, &woken);
            if let Some(reg) = fired {
                notify(k, reg);
            }
            Ok(0)
        }
        SendStep::Sleep(ticket) => sleep(k, &f.queue, ticket, deadline).map(|_| 0),
    }
}

/// mq_timedreceive(mqdes, msg_ptr, msg_len, msg_prio, abs_timeout)
///
/// In order: `abs_timeout` (`EFAULT`, `EINVAL`), the descriptor (`EBADF`,
/// also if not open for reading), `msg_len` (`EMSGSIZE` below
/// `mq_msgsize`), then the queue (empty: `EAGAIN` with `O_NONBLOCK`, else
/// a sleep). The message has left the queue when it is copied out: an
/// unwritable `msg_ptr` or `msg_prio` loses it (`EFAULT`).
pub fn mq_timedreceive(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd, msg, len, prio_ptr) = (a.fd(0), a.ptr(1), a.ulong(2), a.ptr(3));
    let deadline = read_timeout(k, a.ptr(4))?;
    let f = queue_fd(k, fd)?;
    if !f.readable() {
        return Err(Errno::EBADF);
    }
    if len < f.queue.msgsize as u64 {
        return Err(Errno::EMSGSIZE);
    }
    let tid = k.tid();
    let (prio, data) = match f.queue.receive(f.nonblock(), tid)? {
        ReceiveStep::Done { prio, data, woken } => {
            wake(k, &woken);
            (prio, data)
        }
        ReceiveStep::Sleep(ticket) => match sleep(k, &f.queue, ticket, deadline)? {
            Done::Received { prio, data } => (prio, data),
            Done::Sent => return Err(Errno::EINVAL),
        },
    };
    k.copy_to_user(msg, &data)?;
    if !prio_ptr.is_null() {
        k.copy_to_user(prio_ptr, &prio.to_le_bytes())?;
    }
    Ok(data.len() as u64)
}

/// mq_notify(mqdes, sevp)
///
/// In order: `sevp` (`EFAULT`), its fields (`EINVAL`:
/// [`SigEvent::validate`]), the descriptor (`EBADF`), then the queue's one
/// registration: `EBUSY` while a running process holds it. A null `sevp`
/// removes the caller's registration (success also when there is none).
/// `SIGEV_THREAD` (a netlink socket in `sigev_signo`, a cookie at
/// `sigev_value`) is not supported: the cookie is read (`EFAULT`), then
/// `EBADF` as for a descriptor that is no netlink socket.
pub fn mq_notify(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd, sevp) = (a.fd(0), a.ptr(1));
    let event = if sevp.is_null() {
        None
    } else {
        let mut b = [0u8; SigEvent::SIZE];
        k.copy_from_user(sevp, &mut b)?;
        let e = SigEvent::from_bytes(&b);
        e.validate()?;
        if e.notify == SIGEV_THREAD {
            let mut cookie = [0u8; 32];
            k.copy_from_user(UserPtr(e.value), &mut cookie)?;
        }
        Some(e)
    };
    let f = queue_fd(k, fd)?;
    let pid = k.pid();
    let Some(event) = event else {
        f.queue.unregister(pid);
        return Ok(0);
    };
    if event.notify == SIGEV_THREAD {
        return Err(Errno::EBADF);
    }
    // A registration whose process is gone went with it.
    if let Some(r) = f.queue.registered()
        && r.pid != pid
        && !alive(k, r.pid)
    {
        f.queue.unregister(r.pid);
    }
    f.queue.register(Registration { pid, event, file: f.file }).map(|()| 0)
}

/// mq_getsetattr(mqdes, newattr, oldattr)
///
/// In order: `newattr` (`EFAULT`), its `mq_flags` (`EINVAL` for anything
/// but `O_NONBLOCK`), the descriptor (`EBADF`), then `oldattr` as before
/// the change (`EFAULT`). Only the open file's `O_NONBLOCK` changes.
pub fn mq_getsetattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd, new_ptr, old_ptr) = (a.fd(0), a.ptr(1), a.ptr(2));
    let new = if new_ptr.is_null() {
        None
    } else {
        let mut b = [0u8; MqAttr::SIZE];
        k.copy_from_user(new_ptr, &mut b)?;
        let attr = MqAttr::from_bytes(&b);
        if attr.flags & !i64::from(oflag::O_NONBLOCK) != 0 {
            return Err(Errno::EINVAL);
        }
        Some(attr)
    };
    let f = queue_fd(k, fd)?;
    let old = MqAttr {
        flags: i64::from(f.flags & oflag::O_NONBLOCK),
        maxmsg: f.queue.maxmsg as i64,
        msgsize: f.queue.msgsize as i64,
        curmsgs: f.queue.curmsgs() as i64,
    };
    if let Some(new) = new {
        let flags = (f.flags & !oflag::O_NONBLOCK) | new.flags as u32;
        k.fs().ok_or(Errno::ENOSYS)?.set_status_flags(fd, flags)?;
    }
    if !old_ptr.is_null() {
        k.copy_to_user(old_ptr, &old.to_bytes())?;
    }
    Ok(0)
}

/// The calling process is exiting: its notification registrations go.
pub fn process_exit(k: &mut dyn LinuxKernel) {
    crate::ipc::mqueue::process_exit(k.pid());
}

#[cfg(test)]
mod tests {
    use super::valid_timeout;
    use crate::abi::Timespec;
    use crate::errno::Errno;

    #[test]
    fn timeouts() {
        assert_eq!(valid_timeout(Timespec { sec: 0, nsec: 0 }), Ok(()));
        assert_eq!(valid_timeout(Timespec { sec: 1, nsec: 999_999_999 }), Ok(()));
        assert_eq!(valid_timeout(Timespec { sec: 0, nsec: 1_000_000_000 }), Err(Errno::EINVAL));
        assert_eq!(valid_timeout(Timespec { sec: -1, nsec: 0 }), Err(Errno::EINVAL));
        assert_eq!(valid_timeout(Timespec { sec: 0, nsec: -1 }), Err(Errno::EINVAL));
    }
}
