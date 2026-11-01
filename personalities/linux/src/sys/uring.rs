//! io_uring_setup(2), io_uring_enter(2), io_uring_register(2): argument
//! checks in the order the calls report errors; the instance and the
//! requests are [`crate::uring`]'s.

use crate::abi::Timespec;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{Clock, LinuxKernel, UserPtr};
use crate::sys::poll::sigmask_arg;
use crate::table::{Handler, nr};
use crate::uring::abi::{self, Layout, Params, enter, reg, setup};
use crate::uring::exec::{self, uring};
use crate::uring::{FileHandle, Instance};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::IO_URING_SETUP, io_uring_setup),
    (nr::IO_URING_ENTER, io_uring_enter),
    (nr::IO_URING_REGISTER, io_uring_register),
];

/// `struct io_uring_getevents_arg` (24 bytes): `u64 sigmask; u32
/// sigmask_sz; u32 min_wait_usec; u64 ts`.
const GETEVENTS_ARG_SIZE: u64 = 24;
/// `IORING_REGISTER_FILES_SKIP`: leave the slot as it is.
const FILES_SKIP: i32 = -2;
const PAGE: u64 = 4096;

/// io_uring_setup(entries, params): `EFAULT` for the structure, then
/// `EINVAL` for reserved words, unknown flags, the entry counts and flag
/// combinations; with `IORING_SETUP_ATTACH_WQ` a `wq_fd` that is not open
/// is `ENXIO` and one that is no io_uring `EINVAL`.
pub fn io_uring_setup(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let ptr = a.ptr(1);
    if ptr.is_null() {
        return Err(Errno::EFAULT);
    }
    let mut raw = [0; Params::SIZE];
    k.copy_from_user(ptr, &mut raw)?;
    let p = Params::from_bytes(&raw);
    if p.resv.iter().any(|&w| w != 0) || p.flags & !setup::KNOWN != 0 {
        return Err(Errno::EINVAL);
    }
    let layout = Layout::for_setup(a.uint(0), &p)?;
    if p.flags & setup::SQ_AFF != 0 && p.flags & setup::SQPOLL == 0 {
        return Err(Errno::EINVAL);
    }
    // Rings in memory the caller provides are not supported.
    if p.flags & setup::NO_MMAP != 0 {
        return Err(Errno::EINVAL);
    }
    if p.flags & setup::ATTACH_WQ != 0 {
        match uring(k)?.ring_of(p.wq_fd as i32) {
            Err(Errno::EBADF) => return Err(Errno::ENXIO),
            Err(e) => return Err(e),
            Ok(None) => return Err(Errno::EINVAL),
            Ok(Some(_)) => {}
        }
    }
    let u = uring(k)?;
    let rings = u.ring_memory(layout.rings_len())?;
    let sqes = u.ring_memory(layout.sqes_len())?;
    let instance = Arc::new(Instance::new(layout, p.flags, rings, sqes));
    k.copy_to_user(ptr, &Params::result(&raw, &layout))?;
    uring(k)?.install_ring(instance).map(|fd| fd as u64)
}

/// The instance open as `fd`: `EBADF`, or `EOPNOTSUPP` for a descriptor
/// that is no io_uring.
fn instance(k: &mut dyn LinuxKernel, fd: i32) -> Result<Arc<Instance>, Errno> {
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    uring(k)?.ring_of(fd)?.ok_or(Errno::EOPNOTSUPP)
}

/// io_uring_enter(fd, to_submit, min_complete, flags, argp, argsz):
/// unknown flags are `EINVAL`, then the descriptor (`EBADF`,
/// `EOPNOTSUPP`). Submits up to `to_submit` SQEs, then with
/// `IORING_ENTER_GETEVENTS` waits for `min_complete` CQEs; the wait
/// argument is read only when a wait is asked for. Returns the number of
/// SQEs consumed (also when the wait then fails, if it is not 0).
pub fn io_uring_enter(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (to_submit, min_complete, flags) = (a.uint(1), a.uint(2), a.uint(3));
    if flags & !enter::KNOWN != 0 || flags & (enter::REGISTERED_RING | enter::EXT_ARG_REG) != 0 {
        return Err(Errno::EINVAL);
    }
    let inst = instance(k, a.fd(0))?;
    if inst.disabled() {
        return Err(Errno::EBADFD);
    }
    // Without a polling thread, an SQPOLL ring submits everything queued
    // on every call.
    let max = if inst.setup_flags & setup::SQPOLL != 0 { u32::MAX } else { to_submit };
    let submitted = if max > 0 { exec::submit(k, &inst, max) } else { 0 };
    exec::progress(k, &inst);
    if flags & enter::GETEVENTS != 0 && min_complete > 0 {
        let (until, mask) = if flags & enter::EXT_ARG != 0 {
            if a.ulong(5) != GETEVENTS_ARG_SIZE {
                return Err(Errno::EINVAL);
            }
            let mut raw = [0; GETEVENTS_ARG_SIZE as usize];
            k.copy_from_user(a.ptr(4), &mut raw)?;
            let le64 = |at: usize| u64::from_le_bytes(raw[at..at + 8].try_into().expect("8 bytes"));
            let sz = u64::from(u32::from_le_bytes(raw[8..12].try_into().expect("4 bytes")));
            let mask = sigmask_arg(k, UserPtr(le64(0)), sz)?;
            let ts = le64(16);
            let until = if ts == 0 {
                None
            } else {
                let mut t = [0; Timespec::SIZE];
                k.copy_from_user(UserPtr(ts), &mut t)?;
                let t = Timespec::from_bytes(&t);
                if !t.is_valid() {
                    return Err(Errno::EINVAL);
                }
                let now = k.clock(Clock::Monotonic)?;
                Some(if flags & enter::ABS_TIMER != 0 { t } else { now.saturating_add(t) })
            };
            (until, mask)
        } else {
            (None, sigmask_arg(k, a.ptr(4), a.ulong(5))?)
        };
        let min = min_complete.min(inst.layout.cq_entries);
        if let Err(e) = exec::wait(k, &inst, a.fd(0), min, until, mask) {
            return if submitted > 0 { Ok(u64::from(submitted)) } else { Err(e) };
        }
    }
    Ok(u64::from(submitted))
}

/// io_uring_register(fd, opcode, arg, nr_args): `EBADF`, `EOPNOTSUPP`,
/// then per opcode (unknown ones `EINVAL`).
pub fn io_uring_register(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let inst = instance(k, a.fd(0))?;
    let (opcode, arg, n) = (a.uint(1), a.ptr(2), a.uint(3));
    let unregister_args = || if arg.is_null() && n == 0 { Ok(()) } else { Err(Errno::EINVAL) };
    match opcode {
        reg::REGISTER_BUFFERS => register_buffers(k, &inst, arg, n).map(|()| 0),
        reg::UNREGISTER_BUFFERS => {
            unregister_args()?;
            if inst.clear_buffers() { Ok(0) } else { Err(Errno::ENXIO) }
        }
        reg::REGISTER_FILES => register_files(k, &inst, arg, n).map(|()| 0),
        reg::UNREGISTER_FILES => {
            unregister_args()?;
            if inst.clear_files() { Ok(0) } else { Err(Errno::ENXIO) }
        }
        reg::REGISTER_EVENTFD | reg::REGISTER_EVENTFD_ASYNC => {
            if n != 1 {
                return Err(Errno::EINVAL);
            }
            let fd = read_ints(k, arg, 1)?[0];
            if !uring(k)?.is_eventfd(fd)? {
                return Err(Errno::EINVAL);
            }
            let h = uring(k)?.file_handle(fd)?;
            if inst.set_eventfd(h) { Ok(0) } else { Err(Errno::EBUSY) }
        }
        reg::UNREGISTER_EVENTFD => {
            unregister_args()?;
            if inst.clear_eventfd() { Ok(0) } else { Err(Errno::ENXIO) }
        }
        reg::REGISTER_FILES_UPDATE => update_files(k, &inst, arg, n),
        reg::REGISTER_PROBE => {
            if n > reg::MAX_PROBE_OPS {
                return Err(Errno::EINVAL);
            }
            let mut buf = vec![0u8; 16 + 8 * n as usize];
            k.copy_from_user(arg, &mut buf)?;
            if buf.iter().any(|&b| b != 0) {
                return Err(Errno::EINVAL);
            }
            abi::fill_probe(&mut buf, n as usize);
            k.copy_to_user(arg, &buf)?;
            Ok(0)
        }
        reg::REGISTER_ENABLE_RINGS => {
            unregister_args()?;
            if inst.enable() { Ok(0) } else { Err(Errno::EBADFD) }
        }
        _ => Err(Errno::EINVAL),
    }
}

/// `n` `int`s at `ptr` (`EFAULT`).
fn read_ints(k: &mut dyn LinuxKernel, ptr: UserPtr, n: usize) -> Result<Vec<i32>, Errno> {
    let mut raw = vec![0u8; 4 * n];
    k.copy_from_user(ptr, &mut raw)?;
    Ok(raw.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

/// Whether every page of `[base, base + len)` is mapped (one byte of each
/// is readable).
fn mapped(k: &mut dyn LinuxKernel, base: u64, len: u64) -> bool {
    let Some(end) = base.checked_add(len) else { return false };
    let mut at = base;
    while at < end {
        let mut b = [0u8];
        if k.copy_from_user(UserPtr(at), &mut b).is_err() {
            return false;
        }
        at = (at / PAGE + 1) * PAGE;
    }
    let mut b = [0u8];
    k.copy_from_user(UserPtr(end - 1), &mut b).is_ok()
}

/// `IORING_REGISTER_BUFFERS`: `EBUSY` if some are registered, `EINVAL` for
/// 0 or more than 16384, `EFAULT` for the array, a null base, a zero length
/// or an unmapped range.
fn register_buffers(k: &mut dyn LinuxKernel, inst: &Instance, arg: UserPtr, n: u32) -> Result<(), Errno> {
    if inst.has_buffers() {
        return Err(Errno::EBUSY);
    }
    if n == 0 || n > reg::MAX_BUFFERS {
        return Err(Errno::EINVAL);
    }
    let mut raw = vec![0u8; 16 * n as usize];
    k.copy_from_user(arg, &mut raw)?;
    let mut bufs = Vec::with_capacity(n as usize);
    for c in raw.chunks_exact(16) {
        let base = u64::from_le_bytes(c[..8].try_into().expect("8 bytes"));
        let len = u64::from_le_bytes(c[8..].try_into().expect("8 bytes"));
        if base == 0 || len == 0 || !mapped(k, base, len) {
            return Err(Errno::EFAULT);
        }
        bufs.push((base, len));
    }
    if inst.set_buffers(bufs) { Ok(()) } else { Err(Errno::EBUSY) }
}

/// A descriptor for a file table slot: `-1` empty, else a reference
/// (`EBADF` if it is not open or is an io_uring).
fn slot(k: &mut dyn LinuxKernel, fd: i32) -> Result<Option<FileHandle>, Errno> {
    if fd == -1 {
        return Ok(None);
    }
    let u = uring(k)?;
    let h = u.file_handle(fd)?;
    if u.handle_is_ring(&h) {
        return Err(Errno::EBADF);
    }
    Ok(Some(h))
}

/// `IORING_REGISTER_FILES`: `EBUSY` if a table is registered, `EINVAL`
/// for 0 or too many, `EFAULT`, `EBADF` per descriptor.
fn register_files(k: &mut dyn LinuxKernel, inst: &Instance, arg: UserPtr, n: u32) -> Result<(), Errno> {
    if inst.files_len().is_some() {
        return Err(Errno::EBUSY);
    }
    if n == 0 || n > reg::MAX_FILES {
        return Err(Errno::EINVAL);
    }
    let fds = read_ints(k, arg, n as usize)?;
    let mut table = Vec::with_capacity(fds.len());
    for fd in fds {
        table.push(slot(k, fd)?);
    }
    if inst.set_files(table) { Ok(()) } else { Err(Errno::EBUSY) }
}

/// `IORING_REGISTER_FILES_UPDATE` (`struct io_uring_files_update { u32
/// offset; u32 resv; u64 fds; }`): `ENXIO` without a table, `EINVAL` for
/// reserved bits or a range past its end; returns the slots updated.
fn update_files(k: &mut dyn LinuxKernel, inst: &Instance, arg: UserPtr, n: u32) -> SysResult {
    let Some(len) = inst.files_len() else { return Err(Errno::ENXIO) };
    let mut raw = [0u8; 16];
    k.copy_from_user(arg, &mut raw)?;
    let offset = u32::from_le_bytes(raw[0..4].try_into().expect("4 bytes")) as usize;
    let resv = u32::from_le_bytes(raw[4..8].try_into().expect("4 bytes"));
    let fds = u64::from_le_bytes(raw[8..16].try_into().expect("8 bytes"));
    if resv != 0 || offset.saturating_add(n as usize) > len {
        return Err(Errno::EINVAL);
    }
    let fds = read_ints(k, UserPtr(fds), n as usize)?;
    let mut done = 0;
    for (i, fd) in fds.into_iter().enumerate() {
        if fd == FILES_SKIP {
            done += 1;
            continue;
        }
        match slot(k, fd) {
            Ok(h) => inst.set_file(offset + i, h),
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
        done += 1;
    }
    Ok(done)
}
