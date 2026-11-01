//! Reaching into another process (ADR 0121): kcmp(2), pidfd_getfd(2),
//! process_vm_readv(2) and process_vm_writev(2).
//!
//! All four name a process and need ptrace(2) access to it, with the
//! caller's real credentials ([`may_access`]): its own process always;
//! another one when the caller's real uid and gid equal all three of the
//! target's user and group ids and the target is dumpable, or with
//! `CAP_SYS_PTRACE`. Poly has no Yama module, so nothing restricts that
//! further. kcmp needs "read" access and the other three "attach"
//! access; without ptrace(2) the two differ in nothing Poly checks.
//!
//! The kernel provides the objects ([`LinuxKernel::kcmp_id`],
//! [`LinuxKernel::take_fd`]) and the other process's memory
//! ([`LinuxKernel::process_vm_read`], [`LinuxKernel::process_vm_write`]);
//! the argument rules, the order of the checks and the scatter/gather walk
//! ([`pieces`]) are here.

use crate::abi::{IOV_MAX, Iovec, PAGE_SIZE};
use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::fileio::PidFdTarget;
use crate::kernel::{KcmpResource, LinuxKernel, ProcessInfo, Target, UserPtr};
use crate::ns::pid;
use crate::sys::io::load_iovecs;
use crate::sys::pipe::file_io;
use crate::table::{Handler, nr};
use alloc::vec::Vec;

pub const CALLS: &[(u64, Handler)] = &[
    (nr::PROCESS_VM_READV, process_vm_readv),
    (nr::PROCESS_VM_WRITEV, process_vm_writev),
    (nr::KCMP, kcmp),
    (nr::PIDFD_GETFD, pidfd_getfd),
];

// kcmp types (`linux/kcmp.h`).
pub const KCMP_FILE: u32 = 0;
pub const KCMP_VM: u32 = 1;
pub const KCMP_FILES: u32 = 2;
pub const KCMP_FS: u32 = 3;
pub const KCMP_SIGHAND: u32 = 4;
pub const KCMP_IO: u32 = 5;
pub const KCMP_SYSVSEM: u32 = 6;
pub const KCMP_EPOLL_TFD: u32 = 7;
/// `sizeof(struct kcmp_epoll_slot)`: `u32 efd, tfd, toff`.
pub const KCMP_EPOLL_SLOT_SIZE: usize = 12;

/// A process another process's call names, as found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    /// Its thread group id (global).
    pub pid: i32,
    /// Its ids and whether it has ended.
    pub info: ProcessInfo,
    /// Whether it is dumpable (prctl(2) `PR_SET_DUMPABLE`).
    pub dumpable: bool,
}

/// Find the process of thread or process `local` (the caller's pid
/// namespace's id): `ESRCH` when there is none. A zombie (ended, not yet
/// reaped) is found only with `zombies`.
pub fn find(k: &mut dyn LinuxKernel, local: i32, zombies: bool) -> Result<Found, Errno> {
    if local <= 0 {
        return Err(Errno::ESRCH);
    }
    let global = pid::find(k, local)?;
    let mut found = None;
    let mut look = |p: &mut crate::LinuxProcess, info: &ProcessInfo| {
        found = Some(Found { pid: info.pid, info: *info, dumpable: p.dumpable });
    };
    if k.with_process(Target::Thread(global), &mut look).is_err() && zombies {
        k.with_process(Target::Process(global), &mut look)?;
    }
    match found {
        Some(f) if zombies || !f.info.zombie => Ok(f),
        _ => Err(Errno::ESRCH),
    }
}

/// The ptrace(2) access check with real credentials ("PTRACE_MODE_
/// REALCREDS"): the caller's own process; another one if the caller's
/// real uid is the target's real, effective and saved uid, its real gid
/// the target's three gids, and the target is dumpable; or with
/// `CAP_SYS_PTRACE`.
pub fn may_access(me: i32, uid: u32, gid: u32, ptrace_cap: bool, target: &Found) -> bool {
    let t = &target.info;
    target.pid == me
        || ptrace_cap
        || ([t.uid, t.euid, t.suid].iter().all(|&id| id == uid)
            && [t.gid, t.egid, t.sgid].iter().all(|&id| id == gid)
            && target.dumpable)
}

/// [`may_access`] for the calling thread: `EPERM` without access.
fn check_access(k: &mut dyn LinuxKernel, target: &Found) -> Result<(), Errno> {
    let c = cred::current(k);
    if may_access(k.pid(), c.uid, c.gid, c.capable(cap::SYS_PTRACE), target) { Ok(()) } else { Err(Errno::EPERM) }
}

/// kcmp's `type`, with its descriptor arguments (only their low 32 bits
/// count). `EINVAL` for an unknown type. The epoll slot is read later.
fn resource(kind: u32, idx: u64) -> Result<Option<KcmpResource>, Errno> {
    Ok(Some(match kind {
        KCMP_FILE => KcmpResource::File(idx as u32 as i32),
        KCMP_VM => KcmpResource::Vm,
        KCMP_FILES => KcmpResource::Files,
        KCMP_FS => KcmpResource::Fs,
        KCMP_SIGHAND => KcmpResource::Sighand,
        KCMP_IO => KcmpResource::Io,
        KCMP_SYSVSEM => KcmpResource::SysvSem,
        KCMP_EPOLL_TFD => return Ok(None),
        _ => return Err(Errno::EINVAL),
    }))
}

/// kcmp's result for two identities: 0 for the same object, else 1 or 2
/// by their order (kcmp(2): "less than" / "greater than").
pub const fn order(a: u64, b: u64) -> u64 {
    if a == b {
        0
    } else if a < b {
        1
    } else {
        2
    }
}

/// kcmp(pid1, pid2, type, idx1, idx2): checks in the spec sheet's order:
/// both processes (`ESRCH`; zombies count), access to both (`EPERM`),
/// the type (`EINVAL`), then the descriptors.
pub fn kcmp(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let p1 = find(k, a.int(0), true)?;
    let p2 = find(k, a.int(1), true)?;
    check_access(k, &p1)?;
    check_access(k, &p2)?;
    let kind = a.uint(2);
    let (what1, what2) = match resource(kind, a.ulong(3))? {
        Some(r1) => {
            let r2 = resource(kind, a.ulong(4))?.expect("same type");
            (r1, r2)
        }
        None => {
            let mut raw = [0u8; KCMP_EPOLL_SLOT_SIZE];
            k.copy_from_user(a.ptr(4), &mut raw)?;
            let word = |i: usize| u32::from_le_bytes(raw[i..i + 4].try_into().expect("4 bytes"));
            let slot = KcmpResource::EpollTarget { efd: word(0) as i32, tfd: word(4) as i32, toff: word(8) };
            (KcmpResource::File(a.ulong(3) as u32 as i32), slot)
        }
    };
    let id1 = k.kcmp_id(p1.pid, what1)?;
    let id2 = k.kcmp_id(p2.pid, what2)?;
    Ok(order(id1, id2))
}

/// pidfd_getfd(pidfd, targetfd, flags): checks in the spec sheet's
/// order: flags (must be 0), the pidfd (`EBADF` unless open and a
/// pidfd), the process alive (`ESRCH`), access (`EPERM`), then the
/// kernel's copy (`EBADF` for `targetfd`, `EMFILE`).
pub fn pidfd_getfd(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(2) != 0 {
        return Err(Errno::EINVAL);
    }
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let (global, thread) = match file_io(k)?.pidfd_target(fd)? {
        Some(PidFdTarget::PidFd { pid, thread }) => (pid, thread),
        _ => return Err(Errno::EBADF),
    };
    let target = if thread { Target::Thread(global) } else { Target::Process(global) };
    let mut found = None;
    let _ = k.with_process(target, &mut |p, info| {
        found = Some(Found { pid: info.pid, info: *info, dumpable: p.dumpable });
    });
    let dead = |k: &mut dyn LinuxKernel, pid: i32| {
        crate::task::with_tree(k, |t| !t.contains(pid) || t.is_zombie(pid)).unwrap_or(false)
    };
    let found = match found {
        Some(f) if !f.info.zombie && !dead(k, f.pid) => f,
        _ => return Err(Errno::ESRCH),
    };
    check_access(k, &found)?;
    k.take_fd(found.pid, a.fd(1)).map(|fd| fd as u64)
}

/// One piece of a process_vm transfer: `len` bytes between `local` and
/// `remote`, within one page on both sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub local: u64,
    pub remote: u64,
    pub len: usize,
}

/// The pieces that pair the bytes of `local` with those of `remote` in
/// order (the two lists' boundaries are independent; zero-length entries
/// count for nothing), cut at every page boundary of either side, so a
/// fault ends the transfer exactly at the first byte that cannot be
/// reached. Stops at the shorter total.
pub fn pieces(local: &[Iovec], remote: &[Iovec]) -> Vec<Piece> {
    let mut out = Vec::new();
    let (mut li, mut lo, mut ri, mut ro) = (0, 0u64, 0, 0u64);
    loop {
        while li < local.len() && lo == local[li].len {
            (li, lo) = (li + 1, 0);
        }
        while ri < remote.len() && ro == remote[ri].len {
            (ri, ro) = (ri + 1, 0);
        }
        if li == local.len() || ri == remote.len() {
            return out;
        }
        let l = local[li].base.wrapping_add(lo);
        let r = remote[ri].base.wrapping_add(ro);
        let to_page = |at: u64| PAGE_SIZE - at % PAGE_SIZE;
        let n = (local[li].len - lo).min(remote[ri].len - ro).min(to_page(l)).min(to_page(r));
        out.push(Piece { local: l, remote: r, len: n as usize });
        lo += n;
        ro += n;
    }
}

/// Which way process_vm data goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    /// From the other process to the caller.
    Read,
    /// From the caller to the other process.
    Write,
}

/// process_vm_readv / process_vm_writev: checks in the spec sheets'
/// order: flags, both counts (at most `UIO_MAXIOV`), both arrays
/// (`EFAULT`), an empty list (0, the pid not looked up), the process
/// (`ESRCH`, also a zombie), access (`EPERM`); then the copy, which stops
/// at the first byte either side cannot reach and returns the count, or
/// `EFAULT` if that is 0.
fn process_vm(k: &mut dyn LinuxKernel, a: &Args, dir: Direction) -> SysResult {
    if a.ulong(5) != 0 {
        return Err(Errno::EINVAL);
    }
    let (lcount, rcount) = (a.ulong(2), a.ulong(4));
    if lcount > u64::from(IOV_MAX) || rcount > u64::from(IOV_MAX) {
        return Err(Errno::EINVAL);
    }
    let (local, ltotal) = load_iovecs(k, a.ptr(1), lcount as i32)?;
    let (remote, rtotal) = load_iovecs(k, a.ptr(3), rcount as i32)?;
    if ltotal == 0 || rtotal == 0 {
        return Ok(0);
    }
    let target = find(k, a.int(0), false)?;
    check_access(k, &target)?;
    let mut buf = alloc::vec![0u8; PAGE_SIZE as usize];
    let mut done = 0u64;
    for p in pieces(&local, &remote) {
        let chunk = &mut buf[..p.len];
        let ok = match dir {
            Direction::Read => {
                k.process_vm_read(target.pid, p.remote, chunk).is_ok()
                    && k.copy_to_user(UserPtr(p.local), chunk).is_ok()
            }
            Direction::Write => {
                k.copy_from_user(UserPtr(p.local), chunk).is_ok()
                    && k.process_vm_write(target.pid, p.remote, chunk).is_ok()
            }
        };
        if !ok {
            break;
        }
        done += p.len as u64;
    }
    if done == 0 { Err(Errno::EFAULT) } else { Ok(done) }
}

/// process_vm_readv(pid, local_iov, liovcnt, remote_iov, riovcnt, flags)
pub fn process_vm_readv(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    process_vm(k, a, Direction::Read)
}

/// process_vm_writev(pid, local_iov, liovcnt, remote_iov, riovcnt, flags)
pub fn process_vm_writev(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    process_vm(k, a, Direction::Write)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(base: u64, len: u64) -> Iovec {
        Iovec { base, len }
    }

    fn p(local: u64, remote: u64, len: usize) -> Piece {
        Piece { local, remote, len }
    }

    #[test]
    fn pieces_pair_independent_boundaries() {
        let local = [v(0x1000, 10), v(0, 0), v(0x3000, 20)];
        let remote = [v(0x5000, 4), v(0x6000, 100)];
        assert_eq!(pieces(&local, &remote), [p(0x1000, 0x5000, 4), p(0x1004, 0x6000, 6), p(0x3000, 0x6006, 20)]);
    }

    #[test]
    fn pieces_stop_at_the_shorter_total() {
        assert_eq!(pieces(&[v(0x1000, 100)], &[v(0x2000, 30)]), [p(0x1000, 0x2000, 30)]);
        assert!(pieces(&[v(0x1000, 100)], &[]).is_empty());
        assert!(pieces(&[v(0x1000, 0)], &[v(0x2000, 30)]).is_empty());
    }

    #[test]
    fn pieces_cut_at_page_boundaries_of_both_sides() {
        let local = [v(0x1ffa, 100)];
        let remote = [v(0x2ffe, 100)];
        assert_eq!(pieces(&local, &remote), [p(0x1ffa, 0x2ffe, 2), p(0x1ffc, 0x3000, 4), p(0x2000, 0x3004, 94)]);
    }

    fn found(pid: i32, uid: u32, gid: u32, dumpable: bool) -> Found {
        let info = ProcessInfo { pid, uid, euid: uid, suid: uid, gid, egid: gid, sgid: gid, ..ProcessInfo::default() };
        Found { pid, info, dumpable }
    }

    #[test]
    fn access_needs_matching_ids_or_the_capability() {
        assert!(may_access(5, 4242, 4242, false, &found(5, 0, 0, false)), "itself");
        assert!(may_access(5, 4242, 4242, false, &found(6, 4242, 4242, true)));
        assert!(!may_access(5, 4242, 4242, false, &found(6, 4242, 4242, false)), "not dumpable");
        assert!(!may_access(5, 4242, 4242, false, &found(6, 0, 0, true)), "root's");
        let mut mixed = found(6, 4242, 4242, true);
        mixed.info.suid = 0;
        assert!(!may_access(5, 4242, 4242, false, &mixed), "saved uid differs");
        assert!(may_access(5, 0, 0, true, &found(6, 4242, 4242, false)), "CAP_SYS_PTRACE");
    }

    #[test]
    fn kcmp_orders_consistently() {
        assert_eq!(order(7, 7), 0);
        assert_eq!(order(3, 7) + order(7, 3), 3);
        assert_eq!(resource(8, 0), Err(Errno::EINVAL));
        assert_eq!(resource(KCMP_FILE, (1 << 32) | 5), Ok(Some(KcmpResource::File(5))));
        assert_eq!(resource(KCMP_EPOLL_TFD, 0), Ok(None));
    }
}
