//! I/O request packets: how a read, write or lock that does not finish at
//! once is queued, carried out, cancelled and completed.
//!
//! An [`Irp`] is one request of one thread on one file object. It is
//!
//! - **queued** on its volume's device queue, for the kernel's per-device
//!   worker thread ([`IoState::next_work`] hands the worker a [`Work`] it
//!   runs without the NT lock, [`IoState::work_done`] completes it);
//! - **waiting** for something another request does: a pipe read for data
//!   ([`IoState::pipe_write`]), a lock for an unlock ([`IoState::retry_locks`]);
//! - or **running** (the worker has it; it can no longer be cancelled).
//!
//! Completing a request makes a [`Completion`]: what to copy into the
//! caller's buffer, the `IO_STATUS_BLOCK` to write, the event to set (or
//! the file object to signal), and the APC to queue. The kernel drains them
//! ([`IoState::pop_completion`]): it writes the caller's memory *first*,
//! then calls [`IoState::finish`] under the NT lock to signal, then queues
//! the APC — so a thread woken by the event or running the APC sees the
//! status block filled in (Learn: "ZwReadFile": "the IoStatusBlock is set
//! ... and the Event is set to the signaled state").
//!
//! Cancelling (`NtCancelIoFile`, the file object closing, its thread
//! ending) completes every queued or waiting request it covers with
//! `STATUS_CANCELLED` (Learn: "NtCancelIoFile", "Canceling IRPs"). A
//! request whose thread has ended completes without its APC.
//!
//! I/O completion ports are not modelled yet (no `NtCreateIoCompletion`);
//! a request completes through its event, its file object and its APC.

use super::consts::*;
use super::locks::{ByteLock, LockOwner};
use super::{IoState, NodeKey, Target};
use crate::kernel::Apc;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::result::Result;
use poly_ntobj::status::*;
use poly_ntobj::{NtStatus, ObjectId, ObjectManager, ProcessId, ThreadRef};
use poly_vfs::OpenFile;

/// What a request does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IrpOp {
    /// Read `len` bytes at `offset` into the caller's `buf`.
    Read { offset: u64, len: usize, buf: u64, key: u32 },
    /// Write `data` at `offset` (`None`: at the end of the file).
    Write { offset: Option<u64>, data: Vec<u8>, key: u32 },
    /// Take a byte-range lock once it can be granted.
    Lock { lock: ByteLock },
}

/// Where a request is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrpState {
    Queued(u32),
    Waiting,
    Running,
}

/// One I/O request packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Irp {
    pub id: u64,
    /// The file object's cookie and id.
    pub file: u64,
    pub object: ObjectId,
    pub process: ProcessId,
    pub thread: ThreadRef,
    /// The caller's `IO_STATUS_BLOCK`.
    pub iosb: u64,
    /// The caller's event (the request holds a reference on it).
    pub event: Option<ObjectId>,
    /// The caller's APC routine and context: called as
    /// `routine(ApcContext, IoStatusBlock, 0)` (`PIO_APC_ROUTINE`, `wdm.h`).
    pub apc: Option<Apc>,
    /// The caller waits for it (a synchronous file object).
    pub sync: bool,
    pub op: IrpOp,
    pub state: IrpState,
}

/// What the worker does for a queued request, outside the NT lock.
#[derive(Debug)]
pub struct Work {
    pub irp: u64,
    pub file: Arc<OpenFile>,
    pub op: WorkOp,
}

#[derive(Debug)]
pub enum WorkOp {
    Read { offset: u64, len: usize },
    Write { offset: Option<u64>, data: Vec<u8> },
}

/// What a worker's transfer produced: the bytes read, or the count written.
#[derive(Debug)]
pub struct WorkDone {
    pub data: Vec<u8>,
    pub count: u64,
}

/// The largest buffer one asynchronous read allocates in the kernel; a
/// longer read returns what fits (a short read, which callers handle).
pub const MAX_ASYNC_READ: usize = 1 << 20;

impl Work {
    /// Carry out the transfer (the kernel's worker, or a host test).
    pub fn run(&self) -> Result<WorkDone, NtStatus> {
        match &self.op {
            WorkOp::Read { offset, len } => {
                let mut buf = alloc::vec![0u8; (*len).min(MAX_ASYNC_READ)];
                let n = self.file.read_at(*offset, &mut buf).map_err(super::status_of)?;
                if n == 0 && *len > 0 {
                    return Err(STATUS_END_OF_FILE);
                }
                buf.truncate(n);
                Ok(WorkDone { count: n as u64, data: buf })
            }
            WorkOp::Write { offset, data } => {
                let at = match offset {
                    Some(o) => *o,
                    None => self.file.stat().map_err(super::status_of)?.size,
                };
                let n = self.file.write_at(at, data).map_err(super::status_of)?;
                Ok(WorkDone { count: n as u64, data: Vec::new() })
            }
        }
    }
}

/// A finished request, for the kernel to deliver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub irp: u64,
    pub process: ProcessId,
    pub thread: ThreadRef,
    /// Where to write the `IO_STATUS_BLOCK` (0: nothing to write).
    pub iosb: u64,
    pub status: NtStatus,
    pub information: u64,
    /// Bytes to copy into the caller's memory first (a read's data).
    pub data: Option<(u64, Vec<u8>)>,
    pub event: Option<ObjectId>,
    /// The file object to signal (no event, or a synchronous file object).
    pub file_object: Option<ObjectId>,
    pub apc: Option<Apc>,
    pub sync: bool,
}

impl Completion {
    /// The `IO_STATUS_BLOCK` bytes: `Status` (a 32-bit `NTSTATUS` in the
    /// first pointer-sized slot) and `Information` at 8.
    pub fn iosb_bytes(&self) -> [u8; IO_STATUS_BLOCK_SIZE] {
        iosb_bytes(self.status, self.information)
    }
}

pub fn iosb_bytes(status: NtStatus, information: u64) -> [u8; IO_STATUS_BLOCK_SIZE] {
    let mut b = [0u8; IO_STATUS_BLOCK_SIZE];
    b[0..4].copy_from_slice(&status.0.to_le_bytes());
    b[8..16].copy_from_slice(&information.to_le_bytes());
    b
}

impl IoState {
    /// A fresh request id.
    pub fn new_irp_id(&mut self) -> u64 {
        let id = self.next_irp;
        self.next_irp += 1;
        id
    }

    /// Queue `irp` on device `device`'s worker.
    pub fn queue(&mut self, mut irp: Irp, device: u32) -> u64 {
        let id = irp.id;
        irp.state = IrpState::Queued(device);
        self.irps.insert(id, irp);
        self.queues.entry(device).or_default().push_back(id);
        self.kicks.insert(device);
        id
    }

    /// Keep `irp` waiting (a pipe read without data, a lock that must wait).
    pub fn park(&mut self, mut irp: Irp) -> u64 {
        let id = irp.id;
        irp.state = IrpState::Waiting;
        self.irps.insert(id, irp);
        id
    }

    pub fn irp(&self, id: u64) -> Option<&Irp> {
        self.irps.get(&id)
    }

    /// Requests not yet completed.
    pub fn pending_irps(&self) -> usize {
        self.irps.len()
    }

    /// Devices whose queue got work since the last call (the kernel wakes
    /// their workers).
    pub fn take_kicks(&mut self) -> Vec<u32> {
        core::mem::take(&mut self.kicks).into_iter().collect()
    }

    /// The next queued request of `device`, as work for its worker. A
    /// request that a byte-range lock forbids completes at once with
    /// `STATUS_FILE_LOCK_CONFLICT` (the check is made when the request
    /// starts, as the I/O manager's file-system dispatch would).
    pub fn next_work(&mut self, device: u32) -> Option<Work> {
        loop {
            let id = self.queues.get_mut(&device)?.pop_front()?;
            let Some(irp) = self.irps.get(&id) else { continue };
            let Some(rec) = self.files.get(&irp.file) else {
                self.complete(id, STATUS_CANCELLED, 0, None);
                continue;
            };
            let (Some(open), Some(key)) = (rec.disk().and_then(|d| d.open.clone()), rec.disk().map(|d| d.key)) else {
                self.complete(id, STATUS_INVALID_DEVICE_REQUEST, 0, None);
                continue;
            };
            let file = irp.file;
            let (op, conflict) = match &irp.op {
                IrpOp::Read { offset, len, key: k, .. } => (
                    WorkOp::Read { offset: *offset, len: *len },
                    !self.locks.may_read(key, LockOwner { file, key: *k }, *offset, *len as u64),
                ),
                IrpOp::Write { offset, data, key: k } => {
                    let at = match offset {
                        Some(o) => *o,
                        None => open.stat().map(|s| s.size).unwrap_or(0),
                    };
                    (
                        WorkOp::Write { offset: *offset, data: data.clone() },
                        !self.locks.may_write(key, LockOwner { file, key: *k }, at, data.len() as u64),
                    )
                }
                IrpOp::Lock { .. } => {
                    self.complete(id, STATUS_INVALID_DEVICE_REQUEST, 0, None);
                    continue;
                }
            };
            if conflict {
                self.complete(id, STATUS_FILE_LOCK_CONFLICT, 0, None);
                continue;
            }
            self.irps.get_mut(&id).expect("there").state = IrpState::Running;
            return Some(Work { irp: id, file: open, op });
        }
    }

    /// The worker finished `irp`.
    pub fn work_done(&mut self, irp: u64, result: Result<WorkDone, NtStatus>) {
        let Some(r) = self.irps.get(&irp) else { return };
        match (result, &r.op) {
            (Ok(done), IrpOp::Read { buf, .. }) => {
                let buf = *buf;
                self.complete(irp, STATUS_SUCCESS, done.count, Some((buf, done.data)));
            }
            (Ok(done), _) => self.complete(irp, STATUS_SUCCESS, done.count, None),
            (Err(s), _) => self.complete(irp, s, 0, None),
        }
    }

    /// Finish request `id` with `status`: it leaves the table and a
    /// [`Completion`] is queued.
    pub fn complete(&mut self, id: u64, status: NtStatus, information: u64, data: Option<(u64, Vec<u8>)>) {
        self.complete_with(id, status, information, data, true);
    }

    fn complete_with(&mut self, id: u64, status: NtStatus, info: u64, data: Option<(u64, Vec<u8>)>, apc: bool) {
        let Some(irp) = self.irps.remove(&id) else { return };
        if let IrpState::Queued(dev) = irp.state
            && let Some(q) = self.queues.get_mut(&dev)
        {
            q.retain(|&i| i != id);
        }
        if let Some(Target::Pipe(key)) = self.files.get(&irp.file).map(|f| &f.target)
            && let Some(p) = self.pipes.get_mut(key)
        {
            p.readers.retain(|&i| i != id);
        }
        self.completions.push_back(Completion {
            irp: id,
            process: irp.process,
            thread: irp.thread,
            iosb: irp.iosb,
            status,
            information: info,
            data,
            event: irp.event,
            file_object: (irp.event.is_none() || irp.sync).then_some(irp.object),
            apc: if apc { irp.apc } else { None },
            sync: irp.sync,
        });
    }

    /// A request that finished as soon as it was made (no [`Irp`] kept):
    /// its event, file object and APC still go through the kernel.
    pub fn complete_now(&mut self, irp: Irp, status: NtStatus, information: u64, data: Option<(u64, Vec<u8>)>) {
        let id = irp.id;
        self.irps.insert(id, irp);
        self.complete(id, status, information, data);
    }

    /// Cancel every queued or waiting request `pred` selects (running ones
    /// finish on their own). `thread_gone` drops their APCs. Returns how
    /// many were cancelled.
    pub fn cancel_where(&mut self, mut pred: impl FnMut(&Irp) -> bool, thread_gone: bool) -> usize {
        let ids: Vec<u64> =
            self.irps.values().filter(|i| i.state != IrpState::Running && pred(i)).map(|i| i.id).collect();
        for &id in &ids {
            self.complete_with(id, STATUS_CANCELLED, 0, None, !thread_gone);
        }
        ids.len()
    }

    /// `NtCancelIoFile`: the calling thread's requests on one file object.
    pub fn cancel_file_thread(&mut self, file: u64, thread: ThreadRef) -> usize {
        self.cancel_where(|i| i.file == file && i.thread == thread, false)
    }

    /// `NtCancelIoFileEx`: the process's requests on one file object, all of
    /// them or the one with status block `iosb`.
    pub fn cancel_file_process(&mut self, file: u64, process: ProcessId, iosb: Option<u64>) -> usize {
        self.cancel_where(|i| i.file == file && i.process == process && iosb.is_none_or(|b| b == i.iosb), false)
    }

    /// A thread ended: its requests are cancelled ("all I/O requests a thread
    /// issued are cancelled when it exits", Learn: "CancelIo").
    pub fn cancel_thread(&mut self, thread: ThreadRef) -> usize {
        self.cancel_where(|i| i.thread == thread, true)
    }

    /// Grant waiting lock requests on `key` that can now be granted, oldest
    /// first.
    pub fn retry_locks(&mut self, key: NodeKey) {
        let waiting: Vec<(u64, ByteLock)> = self
            .irps
            .values()
            .filter(|i| i.state == IrpState::Waiting)
            .filter_map(|i| match &i.op {
                IrpOp::Lock { lock } => {
                    let k = self.files.get(&i.file).and_then(|f| f.disk()).map(|d| d.key)?;
                    (k == key).then_some((i.id, *lock))
                }
                _ => None,
            })
            .collect();
        for (id, lock) in waiting {
            if self.locks.lock(key, lock).is_ok() {
                self.complete(id, STATUS_SUCCESS, 0, None);
            }
        }
    }

    /// Next completion for the kernel to deliver.
    pub fn pop_completion(&mut self) -> Option<Completion> {
        self.completions.pop_front()
    }

    pub fn has_completions(&self) -> bool {
        !self.completions.is_empty()
    }

    /// The kernel wrote `c`'s data and status block: set its event, signal
    /// its file object and note a synchronous request's result. Returns the
    /// APC to queue, and on which thread.
    pub fn finish(&mut self, om: &mut ObjectManager, c: &Completion) -> Option<(ThreadRef, Apc)> {
        if let Some(e) = c.event {
            let _ = om.set_event(e);
            let _ = om.dereference(e);
        }
        if let Some(f) = c.file_object
            && om.is_alive(f)
        {
            let _ = om.set_signaled(f, true);
        }
        if c.sync {
            self.sync_done.insert(c.irp, (c.status, c.information));
        }
        c.apc.map(|a| (c.thread, a))
    }

    /// A synchronous request's result, once [`Self::finish`] saw it.
    pub fn take_sync_result(&mut self, irp: u64) -> Option<(NtStatus, u64)> {
        self.sync_done.remove(&irp)
    }

    // ----- \Device\PolyPipe

    /// Read from a pipe: at once if it has data, else wait; with no data
    /// and no writer left (after there was one), `STATUS_PIPE_BROKEN`.
    pub fn pipe_read(&mut self, key: &[u16], mut irp: Irp) -> Result<Option<u64>, NtStatus> {
        let pipe = self.pipes.get_mut(key).ok_or(STATUS_INVALID_HANDLE)?;
        let IrpOp::Read { len, buf, .. } = irp.op else { return Err(STATUS_INVALID_PARAMETER) };
        if pipe.readers.is_empty() && !pipe.data.is_empty() {
            // Done at once: nobody waits for it.
            irp.sync = false;
            let n = len.min(pipe.data.len());
            let data: Vec<u8> = pipe.data.drain(..n).collect();
            self.complete_now(irp, STATUS_SUCCESS, n as u64, Some((buf, data)));
            return Ok(None);
        }
        if len == 0 {
            irp.sync = false;
            self.complete_now(irp, STATUS_SUCCESS, 0, None);
            return Ok(None);
        }
        if pipe.data.is_empty() && pipe.had_writer && pipe.writers == 0 {
            // Every writer is gone: the end of the stream.
            return Err(STATUS_PIPE_BROKEN);
        }
        pipe.readers.push_back(irp.id);
        Ok(Some(self.park(irp)))
    }

    /// Write to a pipe and hand the data to waiting reads, oldest first.
    pub fn pipe_write(&mut self, key: &[u16], data: &[u8]) -> Result<u64, NtStatus> {
        let pipe = self.pipes.get_mut(key).ok_or(STATUS_INVALID_HANDLE)?;
        pipe.data.extend(data.iter().copied());
        let mut done = Vec::new();
        while !pipe.data.is_empty() {
            let Some(id) = pipe.readers.pop_front() else { break };
            let Some(IrpOp::Read { len, buf, .. }) = self.irps.get(&id).map(|i| &i.op) else { continue };
            let n = (*len).min(pipe.data.len());
            let buf = *buf;
            done.push((id, buf, pipe.data.drain(..n).collect::<Vec<u8>>()));
        }
        for (id, buf, bytes) in done {
            let n = bytes.len() as u64;
            self.complete(id, STATUS_SUCCESS, n, Some((buf, bytes)));
        }
        Ok(data.len() as u64)
    }

    /// Bytes buffered in a pipe (tests, debugging).
    pub fn pipe_buffered(&self, key: &[u16]) -> Option<usize> {
        self.pipes.get(key).map(|p| p.data.len())
    }
}
