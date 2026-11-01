//! userfaultfd(2) and its ioctls (ioctl_userfaultfd(2)): page faults of
//! registered ranges handed to user space (ADR 0122).
//!
//! The object's state (the `UFFDIO_API` handshake, the features, the
//! queue of fault messages) is [`UffdState`], which the kernel keeps in
//! the file; registrations are per region in the kernel's region map
//! (`poly_vm`'s `Vma::uffd`). A fault on a missing page of a registered
//! range queues a message and blocks the faulting task until the page is
//! filled (`UFFDIO_COPY`, `UFFDIO_ZEROPAGE`), woken (`UFFDIO_WAKE`) or the
//! object is closed; with `UFFD_FEATURE_SIGBUS` it raises `SIGBUS`
//! instead. This module decodes the calls and the ioctl structures.
//!
//! Constants and structure layouts are from the UAPI header
//! `linux/userfaultfd.h`.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::flags::{O_CLOEXEC, O_NONBLOCK};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::table::{Handler, nr};
use alloc::collections::VecDeque;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::USERFAULTFD, userfaultfd)];

/// `linux/userfaultfd.h`.
pub mod uapi {
    /// The API version `UFFDIO_API` must name.
    pub const UFFD_API: u64 = 0xaa;
    /// userfaultfd(2) flag: only faults raised in user mode are handled.
    pub const UFFD_USER_MODE_ONLY: u32 = 1;

    pub const UFFD_FEATURE_EVENT_FORK: u64 = 1 << 1;
    pub const UFFD_FEATURE_SIGBUS: u64 = 1 << 7;
    pub const UFFD_FEATURE_THREAD_ID: u64 = 1 << 8;
    pub const UFFD_FEATURE_EXACT_ADDRESS: u64 = 1 << 11;
    /// Every feature bit up to `UFFD_FEATURE_MOVE` (1 << 16): what
    /// `UFFDIO_API` reports as supported (the spec sheet's 0x1ffff).
    pub const FEATURES: u64 = 0x1_ffff;
    /// Features that need `CAP_SYS_PTRACE`.
    pub const PRIVILEGED_FEATURES: u64 = UFFD_FEATURE_EVENT_FORK;

    /// `_UFFDIO_*`: ioctl numbers, also the bit numbers of the `ioctls`
    /// fields.
    pub const NR_REGISTER: u32 = 0x00;
    pub const NR_UNREGISTER: u32 = 0x01;
    pub const NR_WAKE: u32 = 0x02;
    pub const NR_COPY: u32 = 0x03;
    pub const NR_ZEROPAGE: u32 = 0x04;
    pub const NR_WRITEPROTECT: u32 = 0x06;
    pub const NR_API: u32 = 0x3f;

    /// `_IOWR(0xAA, nr, struct ...)` and `_IOR(0xAA, nr, struct ...)`.
    pub const UFFDIO_API: u32 = 0xc018_aa3f;
    pub const UFFDIO_REGISTER: u32 = 0xc020_aa00;
    pub const UFFDIO_UNREGISTER: u32 = 0x8010_aa01;
    pub const UFFDIO_WAKE: u32 = 0x8010_aa02;
    pub const UFFDIO_COPY: u32 = 0xc028_aa03;
    pub const UFFDIO_ZEROPAGE: u32 = 0xc020_aa04;

    /// The `ioctls` `UFFDIO_API` returns.
    pub const API_IOCTLS: u64 = 1 << NR_REGISTER | 1 << NR_UNREGISTER | 1 << NR_API;
    /// The `ioctls` `UFFDIO_REGISTER` returns for anonymous memory.
    pub const RANGE_IOCTLS: u64 = 1 << NR_WAKE | 1 << NR_COPY | 1 << NR_ZEROPAGE;

    pub const UFFDIO_REGISTER_MODE_MISSING: u64 = 1 << 0;
    pub const UFFDIO_REGISTER_MODE_WP: u64 = 1 << 1;
    pub const UFFDIO_COPY_MODE_DONTWAKE: u64 = 1 << 0;
    pub const UFFDIO_COPY_MODE_WP: u64 = 1 << 1;
    pub const UFFDIO_ZEROPAGE_MODE_DONTWAKE: u64 = 1 << 0;

    /// `struct uffd_msg` event of a page fault.
    pub const UFFD_EVENT_PAGEFAULT: u8 = 0x12;
    pub const UFFD_PAGEFAULT_FLAG_WRITE: u64 = 1 << 0;
}

use uapi::*;

/// Size of `struct uffd_msg`.
pub const MSG_SIZE: usize = 32;

const PAGE: u64 = crate::abi::PAGE_SIZE;

/// The state of one userfaultfd object.
#[derive(Debug, Default)]
pub struct UffdState {
    /// `UFFDIO_API` succeeded.
    api: bool,
    /// The features it enabled.
    features: u64,
    /// Fault messages not read yet, oldest first.
    queue: VecDeque<[u8; MSG_SIZE]>,
    /// Counts `UFFDIO_WAKE`s, so a blocked task knows it was woken.
    wakes: u64,
}

impl UffdState {
    pub fn new() -> UffdState {
        UffdState::default()
    }

    /// `UFFDIO_API` with `api` and the wanted `features`: the supported
    /// features and the available ioctls. `EINVAL` a second time, for an
    /// unknown `api` (the handshake may be retried) or unknown feature
    /// bits; `EPERM` for privileged features without `CAP_SYS_PTRACE`.
    pub fn api(&mut self, api: u64, features: u64, privileged: bool) -> Result<(u64, u64), Errno> {
        if self.api || api != UFFD_API || features & !FEATURES != 0 {
            return Err(Errno::EINVAL);
        }
        if features & PRIVILEGED_FEATURES != 0 && !privileged {
            return Err(Errno::EPERM);
        }
        self.api = true;
        self.features = features;
        Ok((FEATURES, API_IOCTLS))
    }

    /// Whether the handshake is done (`EINVAL` for everything else before).
    pub fn ready(&self) -> Result<(), Errno> {
        if self.api { Ok(()) } else { Err(Errno::EINVAL) }
    }

    /// Whether faults raise `SIGBUS` instead of waiting.
    pub fn sigbus(&self) -> bool {
        self.features & UFFD_FEATURE_SIGBUS != 0
    }

    /// Queue the message for a fault at `addr` (a write if `write`) by
    /// thread `tid`.
    pub fn push_fault(&mut self, addr: u64, write: bool, tid: u32) {
        let mut m = [0u8; MSG_SIZE];
        m[0] = UFFD_EVENT_PAGEFAULT;
        let flags = if write { UFFD_PAGEFAULT_FLAG_WRITE } else { 0 };
        m[8..16].copy_from_slice(&flags.to_le_bytes());
        let addr = if self.features & UFFD_FEATURE_EXACT_ADDRESS != 0 { addr } else { addr & !(PAGE - 1) };
        m[16..24].copy_from_slice(&addr.to_le_bytes());
        let ptid = if self.features & UFFD_FEATURE_THREAD_ID != 0 { tid } else { 0 };
        m[24..28].copy_from_slice(&ptid.to_le_bytes());
        self.queue.push_back(m);
    }

    /// Whether a message waits to be read.
    pub fn pending(&self) -> bool {
        !self.queue.is_empty()
    }

    /// read(2) into `buf`: as many whole messages as fit. `EINVAL` before
    /// the handshake or for a buffer smaller than one message, `EAGAIN`
    /// when nothing is queued.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, Errno> {
        self.ready()?;
        if buf.len() < MSG_SIZE {
            return Err(Errno::EINVAL);
        }
        if self.queue.is_empty() {
            return Err(Errno::EAGAIN);
        }
        let mut n = 0;
        while n + MSG_SIZE <= buf.len()
            && let Some(m) = self.queue.pop_front()
        {
            buf[n..n + MSG_SIZE].copy_from_slice(&m);
            n += MSG_SIZE;
        }
        Ok(n)
    }

    /// `UFFDIO_WAKE`: blocked tasks look at their page again.
    pub fn wake(&mut self) {
        self.wakes = self.wakes.wrapping_add(1);
    }

    /// How many wakes there were.
    pub fn wakes(&self) -> u64 {
        self.wakes
    }
}

/// A request to the kernel's userfaultfd object, decoded and checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UffdOp<'a> {
    /// Is the descriptor a userfaultfd object at all?
    Probe,
    /// Is the handshake done ([`UffdState::ready`])?
    Ready,
    /// `UFFDIO_API` ([`UffdState::api`]).
    Api { api: u64, features: u64, privileged: bool },
    /// `UFFDIO_REGISTER` of a checked range ([`register_args`]).
    Register { start: u64, len: u64, mode: u64 },
    /// `UFFDIO_UNREGISTER` of a checked range.
    Unregister { start: u64, len: u64 },
    /// `UFFDIO_WAKE` of a checked range.
    Wake { start: u64, len: u64 },
    /// One page of `UFFDIO_COPY`: fill the missing page `dst` with `data`.
    Copy { dst: u64, data: &'a [u8] },
    /// `UFFDIO_ZEROPAGE` of a checked range: fill missing pages with zeros.
    ZeroPage { start: u64, len: u64 },
}

/// What the kernel answers to a [`UffdOp`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UffdReply {
    /// Done.
    Done,
    /// `UFFDIO_API`: the supported features and the available ioctls.
    Api { features: u64, ioctls: u64 },
    /// `UFFDIO_REGISTER`: the ioctls usable on the range.
    Ioctls(u64),
    /// `UFFDIO_COPY` / `UFFDIO_ZEROPAGE`: bytes filled before the first
    /// page that was present already (all of them if none was).
    Filled(u64),
}

/// Check a page-aligned range: `EINVAL` for a misaligned start or length,
/// a zero length or a range that wraps around.
pub fn range_args(start: u64, len: u64) -> Result<(), Errno> {
    if !start.is_multiple_of(PAGE) || !len.is_multiple_of(PAGE) || len == 0 || start.checked_add(len).is_none() {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// Check `UFFDIO_REGISTER`'s arguments: a mode of `MISSING` and/or `WP`
/// (`EINVAL` for 0 or other bits), a page-aligned range.
pub fn register_args(start: u64, len: u64, mode: u64) -> Result<(), Errno> {
    let known = UFFDIO_REGISTER_MODE_MISSING | UFFDIO_REGISTER_MODE_WP;
    if mode == 0 || mode & !known != 0 {
        return Err(Errno::EINVAL);
    }
    range_args(start, len)
}

/// userfaultfd(flags): `O_CLOEXEC`, `O_NONBLOCK` and `UFFD_USER_MODE_ONLY`.
/// While `vm.unprivileged_userfaultfd` is 0 (Poly's fixed setting) a
/// caller without `CAP_SYS_PTRACE` must ask for `UFFD_USER_MODE_ONLY`
/// (`EPERM`, checked before the flags); unknown flags are `EINVAL`.
pub fn userfaultfd(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(0);
    let privileged = crate::cred::current(k).capable(crate::cred::cap::SYS_PTRACE);
    if !privileged && flags & UFFD_USER_MODE_ONLY == 0 {
        return Err(Errno::EPERM);
    }
    if flags & !(O_CLOEXEC | O_NONBLOCK | UFFD_USER_MODE_ONLY) != 0 {
        return Err(Errno::EINVAL);
    }
    k.userfaultfd(flags & O_NONBLOCK != 0, flags & O_CLOEXEC != 0).map(|fd| fd as u64)
}

fn word(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().expect("8 bytes"))
}

fn op(k: &mut dyn LinuxKernel, fd: i32, op: UffdOp<'_>) -> Result<UffdReply, Errno> {
    k.userfaultfd_op(fd, op).unwrap_or(Err(Errno::EINVAL))
}

/// The ioctls of a userfaultfd descriptor; `None` if `fd` is not one (the
/// caller goes on with the other ioctls).
pub fn ioctl(k: &mut dyn LinuxKernel, fd: i32, request: u32, arg: UserPtr) -> Option<SysResult> {
    if request >> 8 & 0xff != 0xaa {
        return None;
    }
    k.userfaultfd_op(fd, UffdOp::Probe)?.ok()?;
    Some(uffd_ioctl(k, fd, request, arg))
}

fn uffd_ioctl(k: &mut dyn LinuxKernel, fd: i32, request: u32, arg: UserPtr) -> SysResult {
    match request {
        UFFDIO_API => {
            let mut b = [0u8; 24];
            k.copy_from_user(arg, &mut b)?;
            let privileged = crate::cred::current(k).capable(crate::cred::cap::SYS_PTRACE);
            match op(k, fd, UffdOp::Api { api: word(&b, 0), features: word(&b, 1), privileged }) {
                Ok(UffdReply::Api { features, ioctls }) => {
                    b[8..16].copy_from_slice(&features.to_le_bytes());
                    b[16..24].copy_from_slice(&ioctls.to_le_bytes());
                    k.copy_to_user(arg, &b)?;
                    Ok(0)
                }
                Ok(_) => Err(Errno::EINVAL),
                Err(e) => {
                    // The features field reads 0 after a failed handshake.
                    let _ = k.copy_to_user(arg.offset(8).unwrap_or(arg), &0u64.to_le_bytes());
                    Err(e)
                }
            }
        }
        UFFDIO_REGISTER => {
            let mut b = [0u8; 32];
            k.copy_from_user(arg, &mut b)?;
            let (start, len, mode) = (word(&b, 0), word(&b, 1), word(&b, 2));
            ready(k, fd)?;
            register_args(start, len, mode)?;
            match op(k, fd, UffdOp::Register { start, len, mode })? {
                UffdReply::Ioctls(ioctls) => {
                    k.copy_to_user(arg.offset(24).ok_or(Errno::EFAULT)?, &ioctls.to_le_bytes())?;
                    Ok(0)
                }
                _ => Err(Errno::EINVAL),
            }
        }
        UFFDIO_UNREGISTER | UFFDIO_WAKE => {
            let mut b = [0u8; 16];
            k.copy_from_user(arg, &mut b)?;
            let (start, len) = (word(&b, 0), word(&b, 1));
            ready(k, fd)?;
            range_args(start, len)?;
            let o =
                if request == UFFDIO_WAKE { UffdOp::Wake { start, len } } else { UffdOp::Unregister { start, len } };
            op(k, fd, o).map(|_| 0)
        }
        UFFDIO_COPY => {
            let mut b = [0u8; 40];
            k.copy_from_user(arg, &mut b)?;
            let (dst, src, len, mode) = (word(&b, 0), word(&b, 1), word(&b, 2), word(&b, 3));
            let result_at = arg.offset(32).ok_or(Errno::EFAULT)?;
            ready(k, fd)?;
            if mode & !(UFFDIO_COPY_MODE_DONTWAKE | UFFDIO_COPY_MODE_WP) != 0 {
                return Err(Errno::EINVAL);
            }
            range_args(dst, len)?;
            let mut page = alloc::vec![0u8; PAGE as usize];
            let mut done = 0u64;
            let mut failed = None;
            while done < len {
                let from = src.checked_add(done).map(UserPtr).ok_or(Errno::EFAULT);
                if let Err(e) = from.and_then(|p| k.copy_from_user(p, &mut page)) {
                    failed = Some(e);
                    break;
                }
                match op(k, fd, UffdOp::Copy { dst: dst + done, data: &page }) {
                    Ok(UffdReply::Filled(n)) if n == PAGE => done += PAGE,
                    Ok(_) => {
                        failed = Some(Errno::EEXIST);
                        break;
                    }
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            filled(k, result_at, done, failed)
        }
        UFFDIO_ZEROPAGE => {
            let mut b = [0u8; 32];
            k.copy_from_user(arg, &mut b)?;
            let (start, len, mode) = (word(&b, 0), word(&b, 1), word(&b, 2));
            let result_at = arg.offset(24).ok_or(Errno::EFAULT)?;
            ready(k, fd)?;
            if mode & !UFFDIO_ZEROPAGE_MODE_DONTWAKE != 0 {
                return Err(Errno::EINVAL);
            }
            range_args(start, len)?;
            match op(k, fd, UffdOp::ZeroPage { start, len }) {
                Ok(UffdReply::Filled(n)) => filled(k, result_at, n, (n < len).then_some(Errno::EEXIST)),
                Ok(_) => Err(Errno::EINVAL),
                Err(e) => filled(k, result_at, 0, Some(e)),
            }
        }
        _ => Err(Errno::EINVAL),
    }
}

/// `EINVAL` until `UFFDIO_API` succeeded.
fn ready(k: &mut dyn LinuxKernel, fd: i32) -> Result<(), Errno> {
    op(k, fd, UffdOp::Ready).map(|_| ())
}

/// Store the result of a copy or zeropage (`done` bytes, or `-errno` if
/// nothing was filled) and return it: 0 when all was filled, the error
/// if nothing was, `EAGAIN` for a part (ioctl_userfaultfd(2)).
fn filled(k: &mut dyn LinuxKernel, at: UserPtr, done: u64, failed: Option<Errno>) -> SysResult {
    let value: i64 = match failed {
        Some(e) if done == 0 => -i64::from(e.get()),
        _ => done as i64,
    };
    k.copy_to_user(at, &value.to_le_bytes())?;
    match failed {
        None => Ok(0),
        Some(e) if done == 0 => Err(e),
        Some(_) => Err(Errno::EAGAIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake() {
        let mut s = UffdState::new();
        assert_eq!(s.ready(), Err(Errno::EINVAL));
        assert_eq!(s.read(&mut [0u8; 32]), Err(Errno::EINVAL), "read before the handshake");
        assert_eq!(s.api(0x55, 0, false), Err(Errno::EINVAL));
        assert_eq!(s.api(UFFD_API, 1 << 62, false), Err(Errno::EINVAL));
        assert_eq!(s.api(UFFD_API, UFFD_FEATURE_EVENT_FORK, false), Err(Errno::EPERM));
        assert_eq!(s.api(UFFD_API, UFFD_FEATURE_SIGBUS, false), Ok((FEATURES, API_IOCTLS)));
        assert!(s.sigbus());
        assert_eq!(s.api(UFFD_API, 0, false), Err(Errno::EINVAL), "a second time");
        assert_eq!(API_IOCTLS, 0x8000_0000_0000_0003);
    }

    #[test]
    fn messages() {
        let mut s = UffdState::new();
        s.api(UFFD_API, UFFD_FEATURE_THREAD_ID, false).unwrap();
        let mut buf = [0u8; 64];
        assert_eq!(s.read(&mut buf), Err(Errno::EAGAIN));
        assert_eq!(s.read(&mut buf[..8]), Err(Errno::EINVAL));
        s.push_fault(0x12345, true, 7);
        s.push_fault(0x20000, false, 8);
        s.push_fault(0x30000, false, 9);
        assert!(s.pending());
        assert_eq!(s.read(&mut buf), Ok(64), "whole messages that fit");
        assert_eq!(buf[0], UFFD_EVENT_PAGEFAULT);
        assert_eq!(word(&buf, 1), UFFD_PAGEFAULT_FLAG_WRITE);
        assert_eq!(word(&buf, 2), 0x12000, "page-aligned address");
        assert_eq!(u32::from_le_bytes(buf[24..28].try_into().unwrap()), 7);
        assert_eq!(word(&buf, 5), 0, "a read");
        assert_eq!(s.read(&mut buf[..40]), Ok(32));
        assert!(!s.pending());
        let w = s.wakes();
        s.wake();
        assert_ne!(s.wakes(), w);
    }

    #[test]
    fn ranges_and_modes() {
        assert_eq!(register_args(0x1000, 0x4000, UFFDIO_REGISTER_MODE_MISSING), Ok(()));
        assert_eq!(register_args(0x1001, 0x1000, UFFDIO_REGISTER_MODE_MISSING), Err(Errno::EINVAL));
        assert_eq!(register_args(0x1000, 0x1000, 0), Err(Errno::EINVAL));
        assert_eq!(register_args(0x1000, 0x1000, 0x100), Err(Errno::EINVAL));
        assert_eq!(register_args(0x1000, 0x1000, 4), Err(Errno::EINVAL), "minor faults: not for anonymous memory");
        assert_eq!(range_args(0x1000, 0), Err(Errno::EINVAL));
        assert_eq!(range_args(0xffff_ffff_ffff_f000, 0x2000), Err(Errno::EINVAL));
        assert_eq!(RANGE_IOCTLS, 0x1c);
    }
}
