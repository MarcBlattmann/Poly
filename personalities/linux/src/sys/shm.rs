//! System V shared memory: shmget(2), shmat(2), shmdt(2), shmctl(2).
//!
//! The segments of the caller's IPC namespace are in its
//! [`ShmTable`](crate::ipc::shm::ShmTable), which decides under its lock;
//! the kernel makes, attaches and detaches the memory outside it
//! ([`LinuxKernel::shm_create`] and friends).

use crate::abi::{PAGE_SIZE, page_round_up, prot};
use crate::args::Args;
use crate::cred::Creds;
use crate::errno::{Errno, SysResult};
use crate::ipc::shm::{
    EXEC, IPC_64, IPC_RMID, IPC_SET, IPC_STAT, Lookup, READ, SHM_EXEC, SHM_NORESERVE, SHM_RDONLY, SHM_REMAP, SHM_RND,
    SHMLBA, Segment, ShmidDs, WRITE, Who, with_table,
};
use crate::kernel::{Clock, LinuxKernel, Placement, Prot};
use crate::ns::IpcNs;
use crate::ns::pid::view;
use crate::signal::deliver::with_own;
use crate::table::{Handler, nr};
use alloc::sync::Arc;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] =
    &[(nr::SHMGET, shmget), (nr::SHMAT, shmat), (nr::SHMCTL, shmctl), (nr::SHMDT, shmdt)];

/// The caller's IPC namespace (all a detach needs: an exiting thread has
/// already left its process's threads).
fn namespace(k: &mut dyn LinuxKernel) -> Option<Arc<IpcNs>> {
    with_own(k, |p| p.ns.ipc.clone())
}

/// The caller's IPC namespace, credentials and standing in the namespace.
pub(crate) fn caller(k: &mut dyn LinuxKernel) -> (Option<Arc<IpcNs>>, Creds, Who) {
    let tid = k.tid();
    let (ns, creds) = with_own(k, |p| (p.ns.ipc.clone(), p.thread(tid).creds.clone()));
    let owner = ns.as_ref().and_then(|n| n.owner().clone());
    let who = Who::new(&creds, &owner);
    (ns, creds, who)
}

/// The wall clock in seconds, for the segment (and semaphore set) times.
pub(crate) fn now(k: &dyn LinuxKernel) -> i64 {
    k.clock(Clock::Realtime).map_or(0, |t| t.sec)
}

/// `ENOMEM` for memory the system could never provide: more than its RAM
/// and swap together, unless `SHM_NORESERVE` asked for no reservation
/// (shmget(2): "ENOMEM No memory could be allocated for segment
/// overhead"; the memory itself is only taken page by page, when touched).
fn check_commit(k: &mut dyn LinuxKernel, bytes: u64) -> Result<(), Errno> {
    let Ok(info) = k.system_info() else { return Ok(()) };
    let total = info.totalram.saturating_add(info.totalswap).saturating_mul(u64::from(info.mem_unit.max(1)));
    if total != 0 && bytes > total {
        return Err(Errno::ENOMEM);
    }
    Ok(())
}

/// shmget(key, size, shmflg)
///
/// `key` and `shmflg` are `int`s (the upper halves of the registers are
/// ignored). The table decides ([`ShmTable::get`](crate::ipc::shm::ShmTable::get):
/// the order of the errors is there); a new segment gets memory from the
/// kernel and its creator's ids, mode `shmflg & 0777`, the size asked for
/// and the current time. Nothing is attached. If another process made a
/// segment with the same key meanwhile, the call looks again and finds it.
pub fn shmget(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (key, size, flags) = (a.int(0), a.ulong(1), a.uint(2));
    let (ns, _, who) = caller(k);
    let (pid, now) = (k.pid(), now(k));
    loop {
        let (lookup, dead) = with_table(&ns, |t| {
            let dead = t.purge();
            (t.get(key, size, flags, &who), dead)
        });
        drop(dead);
        if let Lookup::Found(id) = lookup? {
            return Ok(id as u64);
        }
        let bytes = page_round_up(size).ok_or(Errno::EINVAL)?;
        if flags & SHM_NORESERVE == 0 {
            check_commit(k, bytes)?;
        }
        let memory = k.shm_create(bytes)?;
        let segment = Segment::new(key, size, flags, &who, pid, now, memory);
        match with_table(&ns, |t| t.insert(segment)) {
            Ok(id) => return Ok(id as u64),
            Err(lost) => drop(lost),
        }
    }
}

/// shmat(shmid, shmaddr, shmflg)
///
/// shmat(2): `shmaddr` 0 lets the kernel choose; otherwise it is rounded
/// down to `SHMLBA` with `SHM_RND` or must be a multiple of it (`EINVAL`),
/// and the attachment goes exactly there: over what is mapped with
/// `SHM_REMAP`, else only into a free range (`EINVAL` if it is not).
/// `SHM_REMAP` without an address (also one `SHM_RND` rounds down to 0)
/// is `EINVAL`. Then the id must name a segment (`EINVAL`; a removed one
/// that is still attached may be attached again) whose mode grants read,
/// and write unless `SHM_RDONLY`, and execute with `SHM_EXEC` (`EACCES`,
/// unless `CAP_IPC_OWNER`). Last the placement: a range that wraps past
/// the top of the address space is `EINVAL` (`ENOMEM` with `SHM_REMAP`),
/// one past the end of user space `ENOMEM`, one below `vm.mmap_min_addr`
/// `EPERM`. The attachment covers the segment's whole pages; `shm_atime`
/// and `shm_lpid` record it.
pub fn shmat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, addr, flags) = (a.int(0), a.ulong(1), a.uint(2));
    let remap = flags & SHM_REMAP != 0;
    let fixed = if remap { Placement::Fixed } else { Placement::FixedNoReplace };
    let (at, placement) = if addr == 0 {
        (0, Placement::Hint)
    } else if flags & SHM_RND != 0 {
        (addr & !(SHMLBA - 1), fixed)
    } else if !addr.is_multiple_of(SHMLBA) {
        return Err(Errno::EINVAL);
    } else {
        (addr, fixed)
    };
    if remap && at == 0 {
        return Err(Errno::EINVAL);
    }
    let (mut access, mut bits) = (READ, prot::PROT_READ);
    if flags & SHM_RDONLY == 0 {
        access |= WRITE;
        bits |= prot::PROT_WRITE;
    }
    if flags & SHM_EXEC != 0 {
        access |= EXEC;
        bits |= prot::PROT_EXEC;
    }
    let (ns, _, who) = caller(k);
    let (pid, now) = (k.pid(), now(k));
    let (found, dead) = with_table(&ns, |t| {
        let dead = t.purge();
        let found = match t.segment(id) {
            None => Err(Errno::EINVAL),
            Some(s) if !s.allows(&who, access) => Err(Errno::EACCES),
            Some(s) => Ok((s.memory.clone(), s.pages() * PAGE_SIZE)),
        };
        (found, dead)
    });
    drop(dead);
    let (memory, len) = found?;
    if placement != Placement::Hint && at.checked_add(len).is_none() {
        return Err(if remap { Errno::ENOMEM } else { Errno::EINVAL });
    }
    let addr = k.shm_attach(&memory, at, placement, len, Prot(bits))?;
    let serial = memory.serial();
    with_table(&ns, |t| {
        if let Some(s) = t.by_serial(serial) {
            s.atime = now;
            s.lpid = pid;
        }
    });
    drop(memory);
    Ok(addr)
}

/// Record a detach of the memory `serial` by process `pid` at `now`, and
/// drop segments that are gone with it.
fn detached(ns: &Option<Arc<IpcNs>>, serials: &[u64], pid: i32, now: i64) {
    let dead = with_table(ns, |t| {
        for serial in serials {
            if let Some(s) = t.by_serial(*serial) {
                s.dtime = now;
                s.lpid = pid;
            }
        }
        t.purge()
    });
    drop(dead);
}

/// munmap(2) took pages of attachments of the memories `serials` (one per
/// attachment it touched): `shm_dtime` and `shm_lpid` record it, and a
/// removed segment goes with its last attachment.
pub fn unmapped(k: &mut dyn LinuxKernel, serials: &[u64]) {
    let ns = namespace(k);
    let (pid, now) = (k.pid(), now(k));
    detached(&ns, serials, pid, now);
}

/// shmdt(shmaddr)
///
/// shmdt(2): `shmaddr` must be where an attachment starts (`EINVAL`
/// otherwise, a misaligned address too); the attachment is unmapped and
/// `shm_dtime` and `shm_lpid` record it. A removed segment goes with its
/// last attachment.
pub fn shmdt(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(0);
    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(Errno::EINVAL);
    }
    let serial = k.shm_detach(addr)?;
    let ns = namespace(k);
    let (pid, now) = (k.pid(), now(k));
    detached(&ns, &[serial], pid, now);
    Ok(0)
}

/// The calling process is exiting: "after an exit(2) all attached shared
/// memory segments are detached from the process" (shmat(2)).
pub fn process_exit(k: &mut dyn LinuxKernel) {
    let serials = k.shm_detach_all();
    if serials.is_empty() {
        return;
    }
    let ns = namespace(k);
    let (pid, now) = (k.pid(), now(k));
    detached(&ns, &serials, pid, now);
}

/// shmctl(shmid, cmd, buf)
///
/// shmctl(2), for `IPC_STAT`, `IPC_SET` and `IPC_RMID` (`IPC_64` in `cmd`
/// is ignored: x86-64 has one layout of `struct shmid_ds`):
///
/// - `IPC_STAT`: needs read permission (`EACCES`); writes the segment's
///   `struct shmid64_ds` to `buf` (`EFAULT`), ids and pids as the caller
///   sees them, `shm_nattch` as the kernel counts it.
/// - `IPC_SET`: reads `buf` (`EFAULT`); the owner, the creator or a
///   caller with `CAP_SYS_ADMIN` (`EPERM`) sets the owner's uid and gid
///   (`EINVAL` if the caller's user namespace has no such id) and the
///   permission bits; `shm_ctime` is now.
/// - `IPC_RMID`: the same permission; the key is released at once and the
///   segment goes now, or with its last detach (`SHM_DEST`).
///
/// An id that names no segment is `EINVAL`, and so is every other command
/// (`IPC_INFO`, `SHM_INFO`, `SHM_STAT`, `SHM_LOCK` and `SHM_UNLOCK` are
/// not implemented).
pub fn shmctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, cmd, buf) = (a.int(0), a.uint(1) & !IPC_64, a.ptr(2));
    let (ns, creds, who) = caller(k);
    match cmd {
        IPC_STAT => {
            let (found, dead) = with_table(&ns, |t| {
                let dead = t.purge();
                let found = match t.segment(id) {
                    None => Err(Errno::EINVAL),
                    Some(s) if !s.allows(&who, READ) => Err(Errno::EACCES),
                    Some(s) => Ok(stat(s)),
                };
                (found, dead)
            });
            drop(dead);
            let mut ds = found?;
            ds.uid = creds.uid_view(ds.uid);
            ds.gid = creds.gid_view(ds.gid);
            ds.cuid = creds.uid_view(ds.cuid);
            ds.cgid = creds.gid_view(ds.cgid);
            ds.cpid = if ds.cpid == 0 { 0 } else { view(k, ds.cpid) };
            ds.lpid = if ds.lpid == 0 { 0 } else { view(k, ds.lpid) };
            k.copy_to_user(buf, &ds.to_bytes())?;
            Ok(0)
        }
        IPC_SET => {
            let mut bytes = [0u8; ShmidDs::SIZE];
            k.copy_from_user(buf, &mut bytes)?;
            let (uid, gid, mode) = ShmidDs::perm_from_bytes(&bytes);
            let now = now(k);
            with_table(&ns, |t| {
                let s = t.segment_mut(id).ok_or(Errno::EINVAL)?;
                if !s.owned_by(&who) {
                    return Err(Errno::EPERM);
                }
                let (uid, gid) = (creds.kuid(uid)?, creds.kgid(gid)?);
                s.uid = uid;
                s.gid = gid;
                s.mode = (s.mode & !0o777) | (mode & 0o777);
                s.ctime = now;
                Ok(0)
            })
        }
        IPC_RMID => {
            let gone = with_table(&ns, |t| {
                let s = t.segment(id).ok_or(Errno::EINVAL)?;
                if !s.owned_by(&who) {
                    return Err(Errno::EPERM);
                }
                Ok(t.remove(id))
            });
            drop(gone?);
            Ok(0)
        }
        _ => Err(Errno::EINVAL),
    }
}

/// What `IPC_STAT` reports of `s`, with kernel ids and global pids.
fn stat(s: &Segment) -> ShmidDs {
    ShmidDs {
        key: s.key,
        uid: s.uid,
        gid: s.gid,
        cuid: s.cuid,
        cgid: s.cgid,
        mode: s.mode,
        seq: 0,
        segsz: s.segsz,
        atime: s.atime,
        dtime: s.dtime,
        ctime: s.ctime,
        cpid: s.cpid,
        lpid: s.lpid,
        nattch: s.nattch(),
    }
}
