//! I/O completion ports (ADR 0169): `NtCreateIoCompletion`,
//! `NtSetIoCompletion`, `NtRemoveIoCompletion(Ex)`, `NtQueryIoCompletion`,
//! and the association of a file object with a port
//! (`NtSetInformationFile(FileCompletionInformation)`).
//!
//! A port is an opaque, waitable object (type `IoCompletion`) whose
//! cookie names a [`Port`] here: a FIFO of [`Packet`]s. The object is
//! signalled exactly while the queue is not empty, so a thread waiting on
//! it wakes when a packet arrives; every woken thread then tries to take
//! one, and the losers wait again. Packets come from two places: a service
//! call (`NtSetIoCompletion`, `PostQueuedCompletionStatus`) and an I/O
//! request on a file object associated with the port, which posts its
//! packet when it completes, after its buffer and status block are written
//! ([`IoState::finish`]). Both kinds share one queue, in the order they
//! were queued.
//!
//! The completion rules are Learn's ("I/O Completion Ports",
//! "GetQueuedCompletionStatus", "SetFileCompletionNotificationModes") and
//! the spec sheets': a packet carries the key of the association, the
//! `ApcContext` of the request (kernel32 passes the `OVERLAPPED` pointer),
//! the final status and the byte count; no packet is made when the
//! request had an APC routine (`ReadFileEx`), when `ApcContext` is NULL
//! (kernel32 passes NULL when the low bit of `OVERLAPPED.hEvent` is set),
//! or, with `FILE_SKIP_COMPLETION_PORT_ON_SUCCESS`, when the request
//! finished at once with `STATUS_SUCCESS`.

use super::IoState;
use super::consts::*;
use crate::abi;
use crate::kernel::{NtKernel, NtTimeout};
use crate::objects::return_handle;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::access::{IO_COMPLETION_MODIFY_STATE, IO_COMPLETION_QUERY_STATE};
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NewObject, NoParse, NtStatus, ObjectId, ObjectManager, ObjectType, WaitOutcome, WaitType};

pub type Result<T> = core::result::Result<T, NtStatus>;

/// One queued completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    /// `KeyContext`: the key of the association (or what was posted).
    pub key: u64,
    /// `ApcContext`: the request's context (the `OVERLAPPED` pointer).
    pub context: u64,
    /// `IoStatus.Status`.
    pub status: NtStatus,
    /// `IoStatus.Information`.
    pub information: u64,
}

/// A port's queue.
#[derive(Debug)]
pub struct Port {
    pub object: ObjectId,
    pub queue: VecDeque<Packet>,
    /// `NumberOfConcurrentThreads`: recorded, not enforced.
    pub concurrency: u32,
}

/// A file object's association with a port: it holds a pointer reference
/// on the port object until the file object goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortAssoc {
    pub object: ObjectId,
    pub cookie: u64,
    pub key: u64,
}

/// A packet for [`IoState::finish`] to queue once a request is complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketPost {
    pub port: PortAssoc,
    pub context: u64,
    pub status: NtStatus,
    pub information: u64,
}

/// The size of a `FILE_IO_COMPLETION_INFORMATION` record (`ntifs.h`):
/// `KeyContext`, `ApcContext`, `IoStatusBlock`.
pub const COMPLETION_RECORD: usize = 32;

impl IoState {
    /// A port object with cookie `cookie` was made.
    pub fn port_create(&mut self, cookie: u64, object: ObjectId, concurrency: u32) {
        self.ports.insert(cookie, Port { object, queue: VecDeque::new(), concurrency });
    }

    /// The port object was deleted: its queue goes.
    pub fn port_deleted(&mut self, cookie: u64) {
        self.ports.remove(&cookie);
    }

    /// Packets queued on port `cookie`.
    pub fn port_depth(&self, cookie: u64) -> usize {
        self.ports.get(&cookie).map_or(0, |p| p.queue.len())
    }

    /// Queue `packet` on port `cookie` and signal the port.
    pub fn port_post(&mut self, om: &mut ObjectManager, cookie: u64, packet: Packet) -> bool {
        let Some(p) = self.ports.get_mut(&cookie) else { return false };
        p.queue.push_back(packet);
        let object = p.object;
        if om.is_alive(object) {
            let _ = om.set_signaled(object, true);
        }
        true
    }

    /// Take up to `max` packets off port `cookie`, oldest first; the port
    /// stops being signalled when it is empty.
    pub fn port_take(&mut self, om: &mut ObjectManager, cookie: u64, max: usize) -> Vec<Packet> {
        let Some(p) = self.ports.get_mut(&cookie) else { return Vec::new() };
        let n = max.min(p.queue.len());
        let out: Vec<Packet> = p.queue.drain(..n).collect();
        if p.queue.is_empty() && om.is_alive(p.object) {
            let _ = om.set_signaled(p.object, false);
        }
        out
    }

    /// The packet `post` describes goes onto its port (dropped when the
    /// port is gone).
    pub(crate) fn port_post_for(&mut self, om: &mut ObjectManager, post: &PacketPost) {
        if om.is_alive(post.port.object) {
            self.port_post(
                om,
                post.port.cookie,
                Packet {
                    key: post.port.key,
                    context: post.context,
                    status: post.status,
                    information: post.information,
                },
            );
        }
    }

    /// Associate file object `file` with a port (`FileCompletionInformation`).
    /// Only an asynchronous file object can be (Learn:
    /// "CreateIoCompletionPort": the handle must be opened for overlapped
    /// I/O), and only once.
    pub fn associate_port(
        &mut self,
        om: &mut ObjectManager,
        caller: &poly_ntobj::Caller,
        file: u64,
        port_handle: u64,
        key: u64,
    ) -> Result<()> {
        let rec = self.files.get(&file).ok_or(STATUS_INVALID_HANDLE)?;
        if rec.port.is_some() || rec.is_sync() {
            // Resolve the handle first so a bad one is reported as such.
            om.object_by_handle(caller, Handle(port_handle), 0, Some(ObjectType::IoCompletion))?;
            return Err(STATUS_INVALID_PARAMETER);
        }
        let object = om.reference_by_handle(
            caller,
            Handle(port_handle),
            IO_COMPLETION_MODIFY_STATE,
            Some(ObjectType::IoCompletion),
        )?;
        let cookie = om.cookie(object).ok_or(STATUS_INVALID_HANDLE)?;
        self.files.get_mut(&file).expect("checked").port = Some(PortAssoc { object, cookie, key });
        Ok(())
    }
}

// ----- the services

/// `NtCreateIoCompletion(IoCompletionHandle*, DesiredAccess,
/// ObjectAttributes*, NumberOfConcurrentThreads)`.
pub fn create_io_completion<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, attrs, concurrent] = abi::args::<K, 4>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let caller = k.caller();
    let created = k.with_state(|sys, _, b| {
        let cookie = sys.io.cookie();
        let new = NewObject::Opaque { kind: ObjectType::IoCompletion, cookie, signaled: Some(false), parse: false };
        let r = sys.objects.create_object(&caller, &attrs.as_object_attributes(), access as u32, new, &mut NoParse);
        if let Ok(c) = &r
            && sys.objects.cookie(c.object) == Some(cookie)
        {
            sys.io.port_create(cookie, c.object, concurrent as u32);
        }
        sys.reap(b);
        r
    })?;
    return_handle(k, out, created.handle)?;
    Ok(created.status)
}

/// The port behind handle `h`, which must have `access`.
fn port_of(sys: &mut crate::NtSystem, caller: &poly_ntobj::Caller, h: u64, access: u32) -> Result<(ObjectId, u64)> {
    let obj = sys.objects.object_by_handle(caller, Handle(h), access, Some(ObjectType::IoCompletion))?;
    let cookie = sys.objects.cookie(obj).ok_or(STATUS_INVALID_HANDLE)?;
    Ok((obj, cookie))
}

/// `NtSetIoCompletion(IoCompletionHandle, KeyContext, ApcContext,
/// IoStatus, IoStatusInformation)`.
pub fn set_io_completion<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, key, context, status, information] = abi::args::<K, 5>(k, regs)?;
    let caller = k.caller();
    k.with_state(|sys, _, _| -> Result<()> {
        let (_, cookie) = port_of(sys, &caller, h, IO_COMPLETION_MODIFY_STATE)?;
        sys.io.port_post(
            &mut sys.objects,
            cookie,
            Packet { key, context, status: NtStatus(status as u32), information },
        );
        Ok(())
    })?;
    Ok(STATUS_SUCCESS)
}

/// Wait for packets: up to `max` of them, the first as soon as one is
/// there. `Ok(Err(status))` is the wait's own end (`STATUS_TIMEOUT`,
/// `STATUS_USER_APC`).
fn remove<K: NtKernel>(
    k: &mut K,
    h: u64,
    timeout: NtTimeout,
    alertable: bool,
    max: usize,
) -> Result<core::result::Result<Vec<Packet>, NtStatus>> {
    let caller = k.caller();
    loop {
        enum Step {
            Got(Vec<Packet>),
            Block(poly_ntobj::WaitId),
            End(NtStatus),
        }
        let step = k.with_state(|sys, _, _| -> Result<Step> {
            let (obj, cookie) = port_of(sys, &caller, h, IO_COMPLETION_MODIFY_STATE)?;
            let got = sys.io.port_take(&mut sys.objects, cookie, max);
            if !got.is_empty() {
                return Ok(Step::Got(got));
            }
            Ok(match sys.objects.wait_objects(caller.thread, &[obj], WaitType::Any, alertable, timeout.dispatch())? {
                WaitOutcome::Done(s) if s == STATUS_TIMEOUT || s == STATUS_USER_APC => Step::End(s),
                // Signalled between the two looks (or by an earlier waiter's
                // leftovers): look again.
                WaitOutcome::Done(_) => Step::End(STATUS_SUCCESS),
                WaitOutcome::Blocked(id) => Step::Block(id),
            })
        })?;
        match step {
            Step::Got(p) => return Ok(Ok(p)),
            Step::End(s) if s == STATUS_SUCCESS => continue,
            Step::End(s) => return Ok(Err(s)),
            Step::Block(id) => {
                let s = k.wait(id, timeout, alertable);
                if s == STATUS_SUCCESS {
                    continue;
                }
                return Ok(Err(s));
            }
        }
    }
}

/// `NtRemoveIoCompletion(IoCompletionHandle, KeyContext*, ApcContext*,
/// IoStatusBlock*, Timeout*)`.
pub fn remove_io_completion<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, key, context, iosb, timeout] = abi::args::<K, 5>(k, regs)?;
    let timeout = crate::sync::read_timeout(k, timeout)?;
    match remove(k, h, timeout, false, 1)? {
        Ok(packets) => {
            let p = packets[0];
            abi::write_u64(k, key, p.key)?;
            abi::write_u64(k, context, p.context)?;
            super::services::write_iosb(k, iosb, p.status, p.information)?;
            Ok(STATUS_SUCCESS)
        }
        Err(s) => Ok(s),
    }
}

/// `NtRemoveIoCompletionEx(IoCompletionHandle, IoCompletionInformation,
/// Count, NumEntriesRemoved*, Timeout*, Alertable)`.
pub fn remove_io_completion_ex<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, info, count, removed, timeout, alertable] = abi::args::<K, 6>(k, regs)?;
    let count = count as u32 as usize;
    if count == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let timeout = crate::sync::read_timeout(k, timeout)?;
    match remove(k, h, timeout, alertable as u8 != 0, count)? {
        Ok(packets) => {
            let mut bytes = Vec::with_capacity(packets.len() * COMPLETION_RECORD);
            for p in &packets {
                bytes.extend_from_slice(&p.key.to_le_bytes());
                bytes.extend_from_slice(&p.context.to_le_bytes());
                bytes.extend_from_slice(&super::irp::iosb_bytes(p.status, p.information));
            }
            abi::write(k, info, &bytes)?;
            abi::write_u32(k, removed, packets.len() as u32)?;
            Ok(STATUS_SUCCESS)
        }
        Err(s) => {
            if s == STATUS_USER_APC || s == STATUS_TIMEOUT {
                let _ = abi::write_u32(k, removed, 0);
            }
            Ok(s)
        }
    }
}

/// `NtQueryIoCompletion(IoCompletionHandle, IoCompletionInformationClass,
/// IoCompletionInformation, IoCompletionInformationLength, ReturnLength*)`:
/// class 0 (`IoCompletionBasicInformation`), a `LONG` depth.
pub fn query_io_completion<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, info, length, ret] = abi::args::<K, 5>(k, regs)?;
    let caller = k.caller();
    let depth = k.with_state(|sys, _, _| -> Result<usize> {
        let (_, cookie) = port_of(sys, &caller, h, IO_COMPLETION_QUERY_STATE)?;
        if class as u32 != IO_COMPLETION_BASIC_INFORMATION {
            return Err(STATUS_INVALID_INFO_CLASS);
        }
        if length as u32 != 4 {
            return Err(STATUS_INFO_LENGTH_MISMATCH);
        }
        Ok(sys.io.port_depth(cookie))
    })?;
    abi::write_u32(k, info, depth as u32)?;
    if ret != 0 {
        abi::write_u32(k, ret, 4)?;
    }
    Ok(STATUS_SUCCESS)
}
