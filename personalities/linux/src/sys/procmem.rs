//! More memory calls: remap_file_pages(2), process_madvise(2) and
//! process_mrelease(2).
//!
//! remap_file_pages(2) is deprecated and emulated, as on Linux: the pages
//! of a shared mapping are mapped again to the same object at another
//! offset ([`LinuxKernel::remap_shared`]). process_madvise(2) is
//! madvise(2) (`sys::madvise`) for the process behind a pidfd; on another
//! process only advice that changes nothing is allowed, so Poly only
//! checks that the ranges are mapped there. process_mrelease(2) frees the
//! memory of a process that is being killed early; Poly frees it when the
//! process ends, so only the checks and the answer remain.

use crate::abi::signal::SIGKILL;
use crate::abi::{IOV_MAX, PAGE_SIZE, page_aligned, page_round_up};
use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::fileio::PidFdTarget;
use crate::kernel::{LinuxKernel, Target};
use crate::sys::io::load_iovecs;
use crate::sys::madvise::{self, Advice};
use crate::sys::pipe::file_io;
use crate::sys::procaccess::{self, Found};
use crate::table::{Handler, nr};

/// The calls of this module, merged into the table (`table::GROUPS`).
pub const CALLS: &[(u64, Handler)] = &[
    (nr::REMAP_FILE_PAGES, remap_file_pages),
    (nr::PROCESS_MADVISE, process_madvise),
    (nr::PROCESS_MRELEASE, process_mrelease),
];

/// remap_file_pages(start, size, prot, pgoff, flags): make the pages
/// `[start, start + size)` of a shared mapping show its object from page
/// `pgoff` on, with the mapping's own protection; the rest of the mapping
/// stays. `start` is rounded down to its page and `size` down to whole
/// pages. The range may span the regions a shared mapping of one object
/// was split into (by an earlier remap), but nothing else. `flags` is
/// ignored (`MAP_NONBLOCK` only skips bringing pages in).
///
/// `EINVAL` for a `prot` other than 0, a size below one page, a page
/// offset that overflows, and a range not inside the shared mappings of
/// one object (unmapped, private, or past its end).
pub fn remap_file_pages(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (start, size, prot, pgoff) = (a.ulong(0), a.ulong(1), a.ulong(2), a.ulong(3));
    if prot != 0 {
        return Err(Errno::EINVAL);
    }
    let start = start & !(PAGE_SIZE - 1);
    let size = size & !(PAGE_SIZE - 1);
    if size == 0 {
        return Err(Errno::EINVAL);
    }
    let pages = size / PAGE_SIZE;
    pgoff.checked_add(pages).ok_or(Errno::EINVAL)?;
    let offset = pgoff.checked_mul(PAGE_SIZE).ok_or(Errno::EINVAL)?;
    let end = start.checked_add(size).ok_or(Errno::EINVAL)?;
    let pid = k.pid();
    let regions = k.memory_regions(pid, start, end)?;
    let first = *regions.first().ok_or(Errno::EINVAL)?;
    if first.start > start || !first.shared || first.object == 0 {
        return Err(Errno::EINVAL);
    }
    let mut at = start;
    for r in &regions {
        if r.start > at || !r.shared || r.object != first.object || r.prot != first.prot {
            return Err(Errno::EINVAL);
        }
        at = r.end;
    }
    if at < end {
        return Err(Errno::EINVAL);
    }
    k.remap_shared(start, size, offset).map(|()| 0)
}

/// The process behind pidfd `fd`: its global id, whether that is a
/// thread's pidfd. `EBADF` for a descriptor that is not open or not a
/// pidfd.
fn pidfd_process(k: &mut dyn LinuxKernel, fd: i32) -> Result<(i32, bool), Errno> {
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    match file_io(k)?.pidfd_target(fd)? {
        Some(PidFdTarget::PidFd { pid, thread }) => Ok((pid, thread)),
        _ => Err(Errno::EBADF),
    }
}

/// What a pidfd's process is now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Running or sleeping, not being killed.
    Alive(Found),
    /// `SIGKILL` is pending: it is being killed and still has its memory.
    Dying,
    /// It has ended (a zombie, or reaped), or its exit is under way: its
    /// memory is gone.
    Gone,
}

fn state(k: &mut dyn LinuxKernel, pid: i32, thread: bool) -> State {
    let target = if thread { Target::Thread(pid) } else { Target::Process(pid) };
    let mut seen = None;
    let _ = k.with_process(target, &mut |p, info| {
        let killed = p.shared.iter().any(|s| s.signo == SIGKILL as i32)
            || p.thread_ids().any(|t| p.thread(t).pending.iter().any(|s| s.signo == SIGKILL as i32));
        seen = Some((Found { pid: info.pid, info: *info, dumpable: p.dumpable }, killed));
    });
    let Some((found, killed)) = seen else { return State::Gone };
    let ended = found.info.zombie
        || crate::task::with_tree(k, |t| !t.contains(found.pid) || t.is_zombie(found.pid)).unwrap_or(false)
        || k.memory_regions(found.pid, 0, 0) == Err(Errno::ESRCH);
    match (ended, killed) {
        (true, _) => State::Gone,
        (false, true) => State::Dying,
        (false, false) => State::Alive(found),
    }
}

/// process_mrelease(pidfd, flags): free the memory of a process that is
/// being killed. The checks in the sheet's order: `flags` (`EINVAL`),
/// the descriptor (`EBADF`), the process: its memory already gone
/// (`ESRCH`), alive and not being killed (`EINVAL`). Poly frees the
/// memory as the process ends, so a dying process gets 0 at once.
pub fn process_mrelease(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(1) != 0 {
        return Err(Errno::EINVAL);
    }
    let (pid, thread) = pidfd_process(k, a.fd(0))?;
    match state(k, pid, thread) {
        State::Gone => Err(Errno::ESRCH),
        State::Dying => Ok(0),
        State::Alive(_) => Err(Errno::EINVAL),
    }
}

/// Advice process_madvise(2) allows on another process: what changes
/// nothing the process could see.
const REMOTE_ADVICE: [u32; 4] =
    [madvise::MADV_WILLNEED, madvise::MADV_COLD, madvise::MADV_PAGEOUT, madvise::MADV_COLLAPSE];

/// madvise(2) of one range for process_madvise: the caller's own memory
/// through the kernel's madvise, another process's by checking that the
/// range is mapped there (the advice it may get changes nothing).
fn advise(k: &mut dyn LinuxKernel, remote: Option<i32>, base: u64, length: u64, advice: u32) -> Result<(), Errno> {
    let what = Advice::from_raw(advice).ok_or(Errno::EINVAL)?;
    if !page_aligned(base) {
        return Err(Errno::EINVAL);
    }
    let len = page_round_up(length).ok_or(Errno::EINVAL)?;
    let end = base.checked_add(len).ok_or(Errno::EINVAL)?;
    if len == 0 {
        return Ok(());
    }
    let Some(pid) = remote else { return k.madvise(base, len, what) };
    let mut at = base;
    for r in k.memory_regions(pid, base, end)? {
        if r.start > at {
            return Err(Errno::ENOMEM);
        }
        at = at.max(r.end);
    }
    if at < end { Err(Errno::ENOMEM) } else { Ok(()) }
}

/// process_madvise(pidfd, iovec, vlen, advice, flags): madvise(2) each
/// range of `iovec` in the memory of the process behind `pidfd`, in
/// order; returns the sum of the lengths advised (as given, not rounded).
/// When a range fails after others worked the call returns what was
/// done; when the first fails, its error.
///
/// Checks, in this order (Poly's where the sheet gives none): `flags`
/// (`EINVAL`), `vlen` above `UIO_MAXIOV` (`EINVAL`), the array
/// (`EFAULT`), the descriptor (`EBADF`), a process that has ended
/// (`ESRCH`); for another process the advice (only `MADV_WILLNEED`,
/// `MADV_COLD`, `MADV_PAGEOUT`, `MADV_COLLAPSE`: `EINVAL`), the right to
/// inspect it and `CAP_SYS_NICE` (`EPERM`). Then each range as madvise(2)
/// checks it.
pub fn process_madvise(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(4) != 0 {
        return Err(Errno::EINVAL);
    }
    let vlen = a.ulong(2);
    if vlen > u64::from(IOV_MAX) {
        return Err(Errno::EINVAL);
    }
    let (iovs, _) = load_iovecs(k, a.ptr(1), vlen as i32)?;
    let (pid, thread) = pidfd_process(k, a.fd(0))?;
    let found = match state(k, pid, thread) {
        State::Gone => return Err(Errno::ESRCH),
        State::Alive(f) => Some(f),
        State::Dying => None,
    };
    let advice = a.uint(3);
    let me = k.pid();
    let remote = match found {
        Some(f) if f.pid == me => None,
        Some(f) => Some(f),
        None if pid == me => None,
        // Being killed: its memory is still there to check.
        None => Some(procaccess::find(k, pid, true)?),
    };
    if let Some(f) = &remote {
        if !REMOTE_ADVICE.contains(&advice) {
            return Err(Errno::EINVAL);
        }
        let c = cred::current(k);
        if !procaccess::may_access(me, c.uid, c.gid, c.capable(cap::SYS_PTRACE), f) || !c.capable(cap::SYS_NICE) {
            return Err(Errno::EPERM);
        }
    }
    let remote = remote.map(|f| f.pid);
    let mut done = 0u64;
    for v in iovs {
        if let Err(e) = advise(k, remote, v.base, v.len, advice) {
            return if done > 0 { Ok(done) } else { Err(e) };
        }
        done = done.saturating_add(v.len);
    }
    Ok(done)
}
