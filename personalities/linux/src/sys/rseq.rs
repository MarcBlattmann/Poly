//! Restartable sequences (ADR 0111): rseq(2) and what the kernel does for
//! a registered thread on its way back to user mode.
//!
//! A thread registers one `struct rseq` (`linux/rseq.h`: `u32
//! cpu_id_start` at 0, `u32 cpu_id` at 4, `u64 rseq_cs` at 8, `u32 flags`
//! at 16, `u32 node_id` at 20, `u32 mm_cid` at 24; 32 bytes, 32-byte
//! aligned) and a signature. From then on:
//!
//! - registering stores the CPU in `cpu_id_start` and `cpu_id`, the NUMA
//!   node (0) in `node_id` and the concurrency id (0) in `mm_cid`; a
//!   readable area that cannot be written is accepted and the thread gets
//!   `SIGSEGV`;
//! - whenever the thread was switched out (preempted, blocked, moved to
//!   another CPU) and before a signal handler's frame is built, [`fixup`]
//!   looks at `rseq_cs`: if it points to a `struct rseq_cs` (`u32
//!   version`, `u32 flags`, `u64 start_ip`, `u64 post_commit_offset`, `u64
//!   abort_ip`) whose section `[start_ip, start_ip + post_commit_offset)`
//!   holds the instruction pointer, the thread continues at `abort_ip`
//!   (whose preceding 4 bytes must be the signature); `rseq_cs` is cleared
//!   in any case and the CPU numbers are brought up to date. Anything the
//!   kernel cannot read or write, or a malformed `rseq_cs`, is `SIGSEGV`.
//!
//! The registration is per thread ([`crate::signal::LinuxThread::rseq`]):
//! a fork(2) child keeps it (its copy of the area is at the same address),
//! a task made with `CLONE_VM` starts without one, execve(2) drops it.
//! The kernel is told about registrations
//! ([`LinuxKernel::rseq_registered`]) so it only calls [`resume`] for
//! threads that have one.

use crate::abi::SigInfo;
use crate::abi::signal::{SI_KERNEL, SIGSEGV};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserContext, UserPtr};
use crate::signal::deliver::with_own;
use crate::table::{Handler, nr};

/// `RSEQ_FLAG_UNREGISTER` (`linux/rseq.h`).
pub const RSEQ_FLAG_UNREGISTER: u32 = 1;
/// `RSEQ_CPU_ID_UNINITIALIZED` (`linux/rseq.h`): `cpu_id` after
/// unregistering.
pub const RSEQ_CPU_ID_UNINITIALIZED: u32 = 0xffff_ffff;
/// The size and alignment of the original `struct rseq`.
pub const RSEQ_SIZE: u32 = 32;
/// Offsets in `struct rseq`.
const CPU_ID_START: u64 = 0;
const RSEQ_CS: u64 = 8;
const NODE_ID: u64 = 20;
/// `sizeof(struct rseq_cs)`.
const RSEQ_CS_SIZE: usize = 32;

pub const CALLS: &[(u64, Handler)] = &[(nr::RSEQ, rseq)];

/// A thread's registration: the area, the length it was registered with
/// and the signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration {
    pub area: UserPtr,
    pub len: u32,
    pub sig: u32,
}

/// What a valid rseq(2) call does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Register `Registration` (nothing is registered yet; the area still
    /// has to be readable).
    Register(Registration),
    /// Drop the registration.
    Unregister,
}

/// The argument checks of rseq(2), in the order the spec gives: flags;
/// then, with a registration, another area or length `EINVAL`, another
/// signature `EPERM`, the same again `EBUSY` (registering) or the
/// unregistration; without one, unregistering `EINVAL`, a length below 32
/// or an area not aligned to 32 bytes `EINVAL`. Whether the area can be
/// read is the caller's to check.
pub fn check(current: Option<Registration>, area: UserPtr, len: u32, flags: u32, sig: u32) -> Result<Request, Errno> {
    if flags & !RSEQ_FLAG_UNREGISTER != 0 {
        return Err(Errno::EINVAL);
    }
    let unregister = flags & RSEQ_FLAG_UNREGISTER != 0;
    match current {
        Some(r) => {
            if r.area != area || r.len != len {
                Err(Errno::EINVAL)
            } else if r.sig != sig {
                Err(Errno::EPERM)
            } else if unregister {
                Ok(Request::Unregister)
            } else {
                Err(Errno::EBUSY)
            }
        }
        None if unregister => Err(Errno::EINVAL),
        None => {
            if len < RSEQ_SIZE || !area.addr().is_multiple_of(u64::from(RSEQ_SIZE)) {
                return Err(Errno::EINVAL);
            }
            Ok(Request::Register(Registration { area, len, sig }))
        }
    }
}

/// `struct rseq_cs` (`linux/rseq.h`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CriticalSection {
    pub version: u32,
    pub flags: u32,
    pub start_ip: u64,
    pub post_commit_offset: u64,
    pub abort_ip: u64,
}

impl CriticalSection {
    pub fn from_bytes(b: &[u8; RSEQ_CS_SIZE]) -> CriticalSection {
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let u64_at = |o: usize| u64::from(u32_at(o)) | (u64::from(u32_at(o + 4)) << 32);
        CriticalSection {
            version: u32_at(0),
            flags: u32_at(4),
            start_ip: u64_at(8),
            post_commit_offset: u64_at(16),
            abort_ip: u64_at(24),
        }
    }
}

/// What the instruction pointer `ip` makes of critical section `cs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Outside the section: `rseq_cs` is only cleared.
    Outside,
    /// Inside: continue at `abort_ip` once its signature is checked.
    Abort(u64),
    /// A section the kernel does not accept (version or flags not 0, a
    /// section that wraps around, `abort_ip` inside it): `SIGSEGV`.
    Invalid,
}

/// Judge `cs` for instruction pointer `ip`.
pub fn judge(cs: &CriticalSection, ip: u64) -> Verdict {
    if cs.version != 0 || cs.flags != 0 {
        return Verdict::Invalid;
    }
    let Some(end) = cs.start_ip.checked_add(cs.post_commit_offset) else { return Verdict::Invalid };
    if (cs.start_ip..end).contains(&cs.abort_ip) {
        return Verdict::Invalid;
    }
    if (cs.start_ip..end).contains(&ip) { Verdict::Abort(cs.abort_ip) } else { Verdict::Outside }
}

fn registration(k: &mut dyn LinuxKernel) -> Option<Registration> {
    let tid = k.tid();
    with_own(k, |p| p.thread(tid).rseq)
}

fn field(area: UserPtr, offset: u64) -> Result<UserPtr, Errno> {
    area.offset(offset).ok_or(Errno::EFAULT)
}

/// Store the CPU numbers (`cpu_id_start`, `cpu_id`), the node and the
/// concurrency id in the area.
fn store_ids(k: &mut dyn LinuxKernel, area: UserPtr) -> Result<(), Errno> {
    let cpu = k.current_cpu().to_le_bytes();
    let mut ids = [0u8; 8];
    ids[..4].copy_from_slice(&cpu);
    ids[4..].copy_from_slice(&cpu);
    k.copy_to_user(field(area, CPU_ID_START)?, &ids)?;
    // node_id 0 (one NUMA node), mm_cid 0.
    k.copy_to_user(field(area, NODE_ID)?, &[0u8; 8])
}

/// rseq(rseq, rseq_len, flags, sig): `rseq_len`, `flags` and `sig` are
/// 32-bit values.
pub fn rseq(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let area = a.ptr(0);
    match check(registration(k), area, a.uint(1), a.uint(2), a.uint(3))? {
        Request::Unregister => {
            let mut ids = [0u8; 8];
            ids[4..].copy_from_slice(&RSEQ_CPU_ID_UNINITIALIZED.to_le_bytes());
            k.copy_to_user(field(area, CPU_ID_START)?, &ids)?;
            let tid = k.tid();
            with_own(k, |p| p.thread_mut(tid).rseq = None);
            k.rseq_registered(false);
            Ok(0)
        }
        Request::Register(reg) => {
            let mut probe = [0u8; RSEQ_SIZE as usize];
            k.copy_from_user(area, &mut probe)?;
            let tid = k.tid();
            with_own(k, |p| p.thread_mut(tid).rseq = Some(reg));
            k.rseq_registered(true);
            // A readable area the kernel cannot write: accepted, and the
            // thread is killed on its way back to user mode.
            if store_ids(k, area).is_err() {
                segv(k);
            }
            Ok(0)
        }
    }
}

fn segv(k: &mut dyn LinuxKernel) {
    crate::signal::force_signal(k, SigInfo::new(SIGSEGV, SI_KERNEL));
}

/// Look at the calling thread's area for `ctx` (see the module docs):
/// `Ok(true)` if `ctx.rip` was moved to an abort handler, `Ok(false)` if
/// nothing about `ctx` changed (also without a registration), `Err` if the
/// thread must get `SIGSEGV`.
pub fn fixup(k: &mut dyn LinuxKernel, ctx: &mut UserContext) -> Result<bool, Errno> {
    let Some(reg) = registration(k) else { return Ok(false) };
    let mut moved = false;
    let mut cs_ptr = [0u8; 8];
    k.copy_from_user(field(reg.area, RSEQ_CS)?, &mut cs_ptr)?;
    let cs_addr = u64::from_le_bytes(cs_ptr);
    if cs_addr != 0 {
        let mut raw = [0u8; RSEQ_CS_SIZE];
        k.copy_from_user(UserPtr(cs_addr), &mut raw)?;
        match judge(&CriticalSection::from_bytes(&raw), ctx.rip) {
            Verdict::Invalid => return Err(Errno::EINVAL),
            Verdict::Outside => {}
            Verdict::Abort(abort_ip) => {
                let mut sig = [0u8; 4];
                let at = abort_ip.checked_sub(4).ok_or(Errno::EFAULT)?;
                k.copy_from_user(UserPtr(at), &mut sig)?;
                if u32::from_le_bytes(sig) != reg.sig {
                    return Err(Errno::EPERM);
                }
                ctx.rip = abort_ip;
                moved = true;
            }
        }
        k.copy_to_user(field(reg.area, RSEQ_CS)?, &[0u8; 8])?;
    }
    store_ids(k, reg.area)?;
    Ok(moved)
}

/// The kernel's call for a registered thread that was switched out, on
/// its way back to user mode: [`fixup`] on its registers. A thread
/// without a registration (any more) is reported to the kernel as such.
/// A failure queues `SIGSEGV`, acted on by the next
/// [`crate::Linux::exit_to_user`].
pub fn resume(k: &mut dyn LinuxKernel) {
    if registration(k).is_none() {
        k.rseq_registered(false);
        return;
    }
    let mut ctx = k.user_context();
    match fixup(k, &mut ctx) {
        Ok(true) => k.set_user_context(&ctx),
        Ok(false) => {}
        Err(_) => segv(k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIG: u32 = 0x5305_3053;
    const AREA: UserPtr = UserPtr(0x1000);

    fn reg() -> Registration {
        Registration { area: AREA, len: 32, sig: SIG }
    }

    #[test]
    fn flags_first() {
        assert_eq!(check(None, AREA, 32, 2, SIG), Err(Errno::EINVAL));
        assert_eq!(check(Some(reg()), AREA, 32, 3, SIG), Err(Errno::EINVAL));
        assert_eq!(check(None, UserPtr(8), 0, 0xffff_ffff, SIG), Err(Errno::EINVAL));
    }

    #[test]
    fn registering() {
        assert_eq!(check(None, AREA, 32, 0, SIG), Ok(Request::Register(reg())));
        assert_eq!(check(None, AREA, 4096, 0, SIG), Ok(Request::Register(Registration { len: 4096, ..reg() })));
        assert_eq!(check(None, AREA, 31, 0, SIG), Err(Errno::EINVAL));
        assert_eq!(check(None, AREA, 0, 0, SIG), Err(Errno::EINVAL));
        assert_eq!(check(None, UserPtr(0x1010), 64, 0, SIG), Err(Errno::EINVAL));
        // NULL is aligned: its fault is the caller's EFAULT.
        assert_eq!(
            check(None, UserPtr::NULL, 32, 0, SIG),
            Ok(Request::Register(Registration { area: UserPtr::NULL, ..reg() }))
        );
    }

    #[test]
    fn while_registered() {
        let r = Some(reg());
        assert_eq!(check(r, AREA, 32, 0, SIG), Err(Errno::EBUSY));
        assert_eq!(check(r, AREA, 32, 0, 1), Err(Errno::EPERM));
        assert_eq!(check(r, AREA, 32, 1, 1), Err(Errno::EPERM));
        assert_eq!(check(r, AREA, 64, 0, SIG), Err(Errno::EINVAL));
        assert_eq!(check(r, UserPtr(0x2000), 32, 1, SIG), Err(Errno::EINVAL));
        assert_eq!(check(r, UserPtr(0x2000), 32, 1, 1), Err(Errno::EINVAL), "area before signature");
        assert_eq!(check(r, AREA, 32, 1, SIG), Ok(Request::Unregister));
    }

    #[test]
    fn unregistering_nothing() {
        assert_eq!(check(None, AREA, 32, 1, SIG), Err(Errno::EINVAL));
        assert_eq!(check(None, UserPtr::NULL, 32, 1, SIG), Err(Errno::EINVAL));
    }

    #[test]
    fn critical_sections() {
        let cs = CriticalSection { start_ip: 0x100, post_commit_offset: 0x10, abort_ip: 0x200, ..Default::default() };
        assert_eq!(judge(&cs, 0x100), Verdict::Abort(0x200));
        assert_eq!(judge(&cs, 0x10f), Verdict::Abort(0x200));
        assert_eq!(judge(&cs, 0x110), Verdict::Outside, "the end is not in the section");
        assert_eq!(judge(&cs, 0xff), Verdict::Outside);
        assert_eq!(judge(&CriticalSection { version: 1, ..cs }, 0x100), Verdict::Invalid);
        assert_eq!(judge(&CriticalSection { flags: 1, ..cs }, 0x100), Verdict::Invalid);
        assert_eq!(judge(&CriticalSection { abort_ip: 0x108, ..cs }, 0x300), Verdict::Invalid);
        assert_eq!(judge(&CriticalSection { post_commit_offset: u64::MAX, ..cs }, 0x100), Verdict::Invalid);
        let empty = CriticalSection { post_commit_offset: 0, ..cs };
        assert_eq!(judge(&empty, 0x100), Verdict::Outside);
    }

    #[test]
    fn rseq_cs_layout() {
        let mut b = [0u8; RSEQ_CS_SIZE];
        b[8..16].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        b[16..24].copy_from_slice(&5u64.to_le_bytes());
        b[24..32].copy_from_slice(&0xabcdu64.to_le_bytes());
        let cs = CriticalSection::from_bytes(&b);
        assert_eq!((cs.start_ip, cs.post_commit_offset, cs.abort_ip), (0x1122_3344_5566_7788, 5, 0xabcd));
    }
}
