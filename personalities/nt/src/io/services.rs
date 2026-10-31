//! The file I/O services: arguments copied in, [`super::IoState`] and the
//! object manager asked, results copied out. Each takes its arguments in
//! the order of the documented `Zw*` routine (Learn, WDK reference).
//!
//! # Synchronous and asynchronous file objects
//!
//! A file object opened with `FILE_SYNCHRONOUS_IO_ALERT` or
//! `FILE_SYNCHRONOUS_IO_NONALERT` keeps a current byte offset, and each
//! read, write or lock waits until it is done (Learn: "Using Files in a
//! Driver", "ZwReadFile"). On a volume the transfer runs right in the
//! calling thread, outside the NT lock; on a slow device (a pipe without
//! data, a lock that must wait) the request is an [`super::Irp`] and the
//! thread waits on the file object until it completes. Both alert flavours
//! wait non-alertably for now (to be confirmed by the spec agent what an
//! APC does to an alertable synchronous wait).
//!
//! Without those options the file object is asynchronous: `ByteOffset` is
//! required (`STATUS_INVALID_PARAMETER`), and a read or write becomes an
//! IRP queued on the volume's worker — the call returns `STATUS_PENDING`
//! and completes later through the `IO_STATUS_BLOCK`, the event (reset
//! when the request is made, as `ReadFile`'s overlapped event is), or the
//! file object itself when there is no event, and the APC
//! (`ApcRoutine(ApcContext, IoStatusBlock, 0)`, delivered at an alertable
//! wait). A request that can finish at once (a pipe that has data, a lock
//! that is free) completes at once and returns its status; its event, file
//! object and APC are signalled as for a later completion.
//!
//! The `IO_STATUS_BLOCK` is written when a request completes, not when it
//! fails its argument checks; a status returned before any request exists
//! (a bad handle, a bad parameter) leaves it alone (to be confirmed by the
//! spec agent per service).

use super::consts::*;
use super::create::{CreateHook, CreateParams};
use super::fileinfo::RenameTo;
use super::irp::{Irp, IrpOp, IrpState, iosb_bytes};
use super::locks::{self, ByteLock, LockOwner};
use super::{Target, dir, info, status_of};
use crate::abi::{self, Attributes};
use crate::kernel::{Apc, NtKernel, NtTimeout};
use crate::objects::return_handle;
use crate::system::NtSystem;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::access::EVENT_MODIFY_STATE;
use poly_ntobj::status::*;
use poly_ntobj::{Caller, Handle, Mode, NtStatus, ObjectId, ObjectType, Timeout, WaitOutcome, WaitType};
use poly_vfs::{Errno, FileType, OpenFile};

pub type Result<T> = core::result::Result<T, NtStatus>;

/// A synchronous transfer moves through the kernel in pieces this big.
const CHUNK: usize = 64 * 1024;
/// The most an asynchronous or pipe write copies in at once.
pub const MAX_CAPTURED_WRITE: usize = 16 << 20;

// ----- shared helpers

/// A file handle: its object and the object's cookie. The handle must have
/// `required` access. A file object the file I/O layer did not make (the
/// minimal ones of ADR 0063) is `STATUS_INVALID_DEVICE_REQUEST`.
pub(crate) fn file_handle(sys: &mut NtSystem, caller: &Caller, h: Handle, required: u32) -> Result<(ObjectId, u64)> {
    let id = sys.objects.object_by_handle(caller, h, required, Some(ObjectType::File))?;
    let cookie = sys.objects.cookie(id).ok_or(STATUS_INVALID_HANDLE)?;
    if sys.io.file(cookie).is_none() {
        return Err(STATUS_INVALID_DEVICE_REQUEST);
    }
    Ok((id, cookie))
}

/// The handle was granted at least one of `any` (kernel-mode callers
/// always are).
fn require_any(sys: &NtSystem, caller: &Caller, h: Handle, any: u32) -> Result<u32> {
    let granted = sys.objects.granted_access(caller, h)?;
    if caller.mode == Mode::User && granted & any == 0 {
        return Err(STATUS_ACCESS_DENIED);
    }
    Ok(granted)
}

/// The `Event`, `ApcRoutine`, `ApcContext` and `IoStatusBlock` arguments
/// every asynchronous service starts with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Notify {
    pub(crate) event: u64,
    pub(crate) apc_routine: u64,
    pub(crate) apc_context: u64,
    pub(crate) iosb: u64,
}

impl Notify {
    fn apc(&self) -> Option<Apc> {
        (self.apc_routine != 0).then_some(Apc {
            routine: self.apc_routine,
            arg1: self.apc_context,
            arg2: self.iosb,
            arg3: 0,
        })
    }

    /// Check the event handle now (so a bad one fails before any work).
    pub(crate) fn check_event(&self, sys: &NtSystem, caller: &Caller) -> Result<()> {
        if self.event != 0 {
            sys.objects.object_by_handle(caller, Handle(self.event), EVENT_MODIFY_STATE, Some(ObjectType::Event))?;
        }
        Ok(())
    }

    /// Reference the event for a request and reset it.
    fn take_event(&self, sys: &mut NtSystem, caller: &Caller) -> Result<Option<ObjectId>> {
        if self.event == 0 {
            return Ok(None);
        }
        let e =
            sys.objects.reference_by_handle(caller, Handle(self.event), EVENT_MODIFY_STATE, Some(ObjectType::Event))?;
        let _ = sys.objects.reset_event(e);
        Ok(Some(e))
    }

    /// The request a call makes.
    pub(crate) fn irp(
        &self,
        sys: &mut NtSystem,
        caller: &Caller,
        file: (ObjectId, u64),
        sync: bool,
        op: IrpOp,
    ) -> Result<Irp> {
        let event = self.take_event(sys, caller)?;
        let _ = sys.objects.set_signaled(file.0, false);
        Ok(Irp {
            id: sys.io.new_irp_id(),
            file: file.1,
            object: file.0,
            process: caller.process,
            thread: caller.thread,
            iosb: self.iosb,
            event,
            apc: self.apc(),
            sync,
            op,
            state: IrpState::Waiting,
        })
    }

    /// A call that finished in the calling thread: set its event and queue
    /// its APC like a completed request (its status block is written by the
    /// service itself).
    pub(crate) fn signal_done(
        &self,
        sys: &mut NtSystem,
        caller: &Caller,
        file: (ObjectId, u64),
        status: NtStatus,
        info: u64,
    ) {
        if self.event == 0 && self.apc_routine == 0 {
            return;
        }
        if let Ok(mut irp) = self.irp(sys, caller, file, false, IrpOp::Write { offset: None, data: Vec::new(), key: 0 })
        {
            irp.iosb = 0;
            sys.io.complete_now(irp, status, info, None);
        }
    }
}

pub(crate) fn write_iosb<K: NtKernel>(k: &mut K, iosb: u64, status: NtStatus, info: u64) -> Result<()> {
    abi::write(k, iosb, &iosb_bytes(status, info))
}

/// An optional `PLARGE_INTEGER` argument.
fn read_offset<K: NtKernel>(k: &mut K, ptr: u64) -> Result<Option<i64>> {
    if ptr == 0 { Ok(None) } else { Ok(Some(abi::read_u64(k, ptr)? as i64)) }
}

/// Wait for synchronous request `irp` on file object `obj` to complete.
pub(crate) fn wait_sync<K: NtKernel>(k: &mut K, obj: ObjectId, irp: u64) -> (NtStatus, u64) {
    let caller = k.caller();
    loop {
        let step = k.with_state(|sys, _, _| {
            if let Some(r) = sys.io.take_sync_result(irp) {
                return Ok(Err(r));
            }
            match sys.objects.wait_objects(caller.thread, &[obj], WaitType::Any, false, Timeout::Infinite) {
                Ok(WaitOutcome::Done(_)) => {
                    // Signalled by an earlier request: not ours yet.
                    let _ = sys.objects.set_signaled(obj, false);
                    Ok(Ok(None))
                }
                Ok(WaitOutcome::Blocked(id)) => Ok(Ok(Some(id))),
                Err(e) => Err(e),
            }
        });
        match step {
            Ok(Err(result)) => return result,
            Ok(Ok(None)) => continue,
            Ok(Ok(Some(id))) => {
                let s = k.wait(id, NtTimeout::Infinite, false);
                if s != STATUS_SUCCESS {
                    // The wait ended without the request (the thread is
                    // going): cancel it.
                    k.with_state(|sys, _, _| sys.io.cancel_where(|i| i.id == irp, true));
                    return (s, 0);
                }
            }
            Err(e) => {
                k.with_state(|sys, _, _| sys.io.cancel_where(|i| i.id == irp, true));
                return (e, 0);
            }
        }
    }
}

/// A read at `offset` (or at the stream's own position). Nothing to read
/// yet from a pipe or device of Poly's own (an inherited standard handle,
/// ADR 0080) is `STATUS_PENDING`, which the caller waits out.
fn read_at(open: &OpenFile, offset: u64, buf: &mut [u8]) -> Result<usize> {
    match open.read_at(offset, buf) {
        Err(Errno::ESPIPE) => open.read(buf),
        r => r,
    }
    .map_err(|e| if e == Errno::EAGAIN { STATUS_PENDING } else { status_of(e) })
}

fn write_at(open: &OpenFile, offset: u64, buf: &[u8]) -> Result<usize> {
    match open.write_at(offset, buf) {
        Err(Errno::ESPIPE) => open.write(buf),
        r => r,
    }
    .map_err(status_of)
}

// ----- NtCreateFile, NtOpenFile, NtDeleteFile, NtQuery(Full)AttributesFile

/// Check `NtCreateFile`'s parameters (Learn: "NtCreateFile"): a
/// disposition above `FILE_OVERWRITE_IF`, unknown option, share or
/// attribute bits, `FILE_DIRECTORY_FILE` with `FILE_NON_DIRECTORY_FILE` or
/// with a disposition other than create/open/open-if, both synchronous
/// options, a synchronous option without `SYNCHRONIZE`, and
/// `FILE_DELETE_ON_CLOSE` without `DELETE` are `STATUS_INVALID_PARAMETER`.
/// `FILE_OPEN_BY_FILE_ID` is not supported (`STATUS_INVALID_PARAMETER`).
pub fn check_create(desired: u32, share: u32, disposition: u32, options: u32, attributes: u32) -> Result<CreateParams> {
    let mut access = ObjectType::File.generic_mapping().map(desired);
    if access & MAXIMUM_ALLOWED != 0 {
        access = (access & !MAXIMUM_ALLOWED) | FILE_ALL_ACCESS;
    }
    let sync = options & (FILE_SYNCHRONOUS_IO_ALERT | FILE_SYNCHRONOUS_IO_NONALERT);
    let bad = disposition > FILE_MAXIMUM_DISPOSITION
        || options & !FILE_VALID_OPTION_FLAGS != 0
        || share & !FILE_SHARE_VALID_FLAGS != 0
        || attributes & !FILE_ATTRIBUTE_VALID_FLAGS != 0
        || options & FILE_DIRECTORY_FILE != 0 && options & FILE_NON_DIRECTORY_FILE != 0
        || options & FILE_DIRECTORY_FILE != 0 && !matches!(disposition, FILE_CREATE | FILE_OPEN | FILE_OPEN_IF)
        || sync == FILE_SYNCHRONOUS_IO_ALERT | FILE_SYNCHRONOUS_IO_NONALERT
        || sync != 0 && access & SYNCHRONIZE == 0
        || options & FILE_DELETE_ON_CLOSE != 0 && access & DELETE == 0
        || options & FILE_OPEN_BY_FILE_ID != 0;
    if bad {
        return Err(STATUS_INVALID_PARAMETER);
    }
    Ok(CreateParams { access, share, disposition, options, attributes, allocation: 0 })
}

/// Open or create through the object manager with the file system's parse
/// procedure. Returns the handle, the `Information` and the cookie.
pub fn open_in(
    sys: &mut NtSystem,
    b: &mut dyn crate::Backend,
    caller: &Caller,
    attrs: &Attributes,
    desired: u32,
    params: CreateParams,
) -> Result<(Handle, u64, u64)> {
    if !sys.io.is_attached() {
        return Err(STATUS_OBJECT_PATH_NOT_FOUND);
    }
    let mut hook = CreateHook { io: &mut sys.io, params, thread: caller.thread, information: 0, made: None };
    let r = sys.objects.open_object(caller, &attrs.as_object_attributes(), desired, Some(ObjectType::File), &mut hook);
    let (information, made) = (hook.information, hook.made);
    sys.reap(b);
    let h = r?;
    Ok((h, information, made.unwrap_or(0)))
}

fn create_common<K: NtKernel>(
    k: &mut K,
    out: u64,
    desired: u32,
    attrs: &Attributes,
    iosb: u64,
    params: CreateParams,
) -> Result<NtStatus> {
    let caller = k.caller();
    let (h, info, _) = k.with_state(|sys, _, b| open_in(sys, b, &caller, attrs, desired, params))?;
    return_handle(k, out, h)?;
    write_iosb(k, iosb, STATUS_SUCCESS, info)?;
    Ok(STATUS_SUCCESS)
}

/// `NtCreateFile(FileHandle*, DesiredAccess, ObjectAttributes*,
/// IoStatusBlock*, AllocationSize*, FileAttributes, ShareAccess,
/// CreateDisposition, CreateOptions, EaBuffer, EaLength)`. Extended
/// attributes (`EaBuffer`) are `STATUS_EAS_NOT_SUPPORTED`.
pub fn create_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, desired, attrs, iosb, alloc_ptr, attributes, share, disposition, options, ea, ea_len] =
        abi::args::<K, 11>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let mut params = check_create(desired as u32, share as u32, disposition as u32, options as u32, attributes as u32)?;
    if ea != 0 && ea_len as u32 != 0 {
        return Err(STATUS_EAS_NOT_SUPPORTED);
    }
    if alloc_ptr != 0 {
        params.allocation = abi::read_u64(k, alloc_ptr)?;
    }
    create_common(k, out, desired as u32, &attrs, iosb, params)
}

/// `NtOpenFile(FileHandle*, DesiredAccess, ObjectAttributes*,
/// IoStatusBlock*, ShareAccess, OpenOptions)`: `NtCreateFile` with
/// `FILE_OPEN` and no attributes.
pub fn open_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, desired, attrs, iosb, share, options] = abi::args::<K, 6>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let params = check_create(desired as u32, share as u32, FILE_OPEN, options as u32, 0)?;
    create_common(k, out, desired as u32, &attrs, iosb, params)
}

/// Open by name for the services that take only `ObjectAttributes`, run
/// `f` on the file object, and close it again.
pub(crate) fn with_named<K: NtKernel, R>(
    k: &mut K,
    attrs: &Attributes,
    desired: u32,
    options: u32,
    f: impl FnOnce(&mut NtSystem, u64) -> Result<R>,
) -> Result<R> {
    let caller = k.caller();
    let all = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let params = check_create(desired, all, FILE_OPEN, options, 0)?;
    k.with_state(|sys, _, b| {
        let (h, _, cookie) = open_in(sys, b, &caller, attrs, desired, params)?;
        let r = f(sys, cookie);
        let _ = sys.objects.close(&caller, h);
        sys.reap(b);
        r
    })
}

/// `NtDeleteFile(ObjectAttributes*)`: open with `DELETE`, mark the file for
/// deletion and close, so it goes now or when the last other handle closes
/// (Learn: "ZwDeleteFile").
pub fn delete_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let ptr = abi::arg(k, regs, 0)?;
    let attrs = abi::read_attributes(k, ptr)?;
    with_named(k, &attrs, DELETE, FILE_OPEN_REPARSE_POINT, |sys, cookie| sys.io.set_disposition(cookie, true, false))?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryAttributesFile(ObjectAttributes*, FILE_BASIC_INFORMATION*)`.
pub fn query_attributes_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [attrs, out] = abi::args::<K, 2>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let bytes = with_named(k, &attrs, FILE_READ_ATTRIBUTES, 0, |sys, cookie| {
        sys.io.query_info(cookie, FILE_BASIC_INFORMATION, FILE_READ_ATTRIBUTES)
    })?;
    abi::write(k, out, &bytes.bytes)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryFullAttributesFile(ObjectAttributes*,
/// FILE_NETWORK_OPEN_INFORMATION*)`.
pub fn query_full_attributes_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [attrs, out] = abi::args::<K, 2>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let bytes = with_named(k, &attrs, FILE_READ_ATTRIBUTES, 0, |sys, cookie| {
        sys.io.query_info(cookie, FILE_NETWORK_OPEN_INFORMATION, FILE_READ_ATTRIBUTES)
    })?;
    abi::write(k, out, &bytes.bytes)?;
    Ok(STATUS_SUCCESS)
}

// ----- NtReadFile, NtWriteFile

/// What a read or write turned into.
enum Plan {
    /// A synchronous transfer on a volume, done by the caller now.
    Inline { open: Arc<OpenFile>, offset: u64, file: (ObjectId, u64), append: bool },
    /// A request that has completed (its completion is queued).
    Done(NtStatus),
    /// A request in flight: wait for it (synchronous) or return pending.
    Pending { irp: u64, file: ObjectId, sync: bool },
}

/// `NtReadFile(FileHandle, Event, ApcRoutine, ApcContext, IoStatusBlock*,
/// Buffer, Length, ByteOffset*, Key*)`. Reading at or past the end of a
/// file is `STATUS_END_OF_FILE` with nothing read (Learn: "ZwReadFile").
pub fn read_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, event, apc_routine, apc_context, iosb, buf, len, off_ptr, key_ptr] = abi::args::<K, 9>(k, regs)?;
    let len = len as u32 as usize;
    let offset = read_offset(k, off_ptr)?;
    let key = if key_ptr == 0 { 0 } else { abi::read_u32(k, key_ptr)? };
    let n = Notify { event, apc_routine, apc_context, iosb };
    // A socket: a receive (ADR 0095).
    if let Some(r) = crate::afd::services::read_file(k, h, n, buf, len) {
        return r;
    }
    let caller = k.caller();
    let plan = k.with_state(|sys, _, _| -> Result<Plan> {
        let file = file_handle(sys, &caller, Handle(h), FILE_READ_DATA)?;
        n.check_event(sys, &caller)?;
        let rec = sys.io.file(file.1).expect("checked");
        let sync = rec.is_sync();
        match &rec.target {
            Target::Pipe(name) => {
                let name = name.clone();
                let irp = n.irp(sys, &caller, file, sync, IrpOp::Read { offset: 0, len, buf, key })?;
                Ok(match sys.io.pipe_read(&name, irp)? {
                    None => Plan::Done(STATUS_SUCCESS),
                    Some(id) => Plan::Pending { irp: id, file: file.0, sync },
                })
            }
            // A console input handle: a read in the input code page (ADR 0080).
            Target::Console(t) => {
                let t = *t;
                let t = sys.io.console_target(file.1).map(|_| t)?;
                if t.kind != crate::console::ConsoleKind::Input {
                    return Err(STATUS_INVALID_HANDLE);
                }
                let irp = n.irp(sys, &caller, file, sync, IrpOp::Read { offset: 0, len, buf, key })?;
                let wait = crate::console::Wait::Text { unicode: false, max: len, buf };
                Ok(match sys.io.console_read(t.console, irp, wait)? {
                    None => Plan::Done(STATUS_SUCCESS),
                    Some(id) => Plan::Pending { irp: id, file: file.0, sync },
                })
            }
            Target::Host(open) => Ok(Plan::Inline { open: open.clone(), offset: rec.position, file, append: false }),
            // Sockets are read before this (`afd::services::read_file`).
            Target::Socket(_) => Err(STATUS_INVALID_DEVICE_REQUEST),
            Target::Disk(d) => {
                if rec.kind == FileType::Directory {
                    return Err(STATUS_INVALID_DEVICE_REQUEST);
                }
                let open = d.open.clone().filter(|o| !o.is_path_only()).ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
                let (volume, node, position) = (d.volume as u32, d.key, rec.position);
                let at = match offset {
                    Some(o) if o >= 0 => o as u64,
                    Some(FILE_USE_FILE_POINTER_POSITION) | None if sync => position,
                    _ => return Err(STATUS_INVALID_PARAMETER),
                };
                if sync {
                    if !sys.io.locks.may_read(node, LockOwner { file: file.1, key }, at, len as u64) {
                        return Err(STATUS_FILE_LOCK_CONFLICT);
                    }
                    return Ok(Plan::Inline { open, offset: at, file, append: false });
                }
                let irp = n.irp(sys, &caller, file, false, IrpOp::Read { offset: at, len, buf, key })?;
                let id = sys.io.queue(irp, volume);
                Ok(Plan::Pending { irp: id, file: file.0, sync: false })
            }
        }
    })?;
    match plan {
        Plan::Inline { open, offset, file, .. } => {
            let mut chunk = alloc::vec![0u8; len.min(CHUNK)];
            let mut total = 0usize;
            while total < len {
                let want = chunk.len().min(len - total);
                let got = match read_at(&open, offset + total as u64, &mut chunk[..want]) {
                    // A stream with nothing yet: wait for the first bytes,
                    // return what came once some did.
                    Err(STATUS_PENDING) if total == 0 => {
                        let s = k.delay(crate::kernel::NtTimeout::Relative(10_000), false);
                        if s != STATUS_SUCCESS && s != poly_ntobj::status::STATUS_TIMEOUT {
                            return Err(s);
                        }
                        continue;
                    }
                    Err(STATUS_PENDING) => break,
                    r => r?,
                };
                if got == 0 {
                    break;
                }
                abi::write(k, buf + total as u64, &chunk[..got])?;
                total += got;
                if got < want {
                    break;
                }
            }
            let status = if total == 0 && len > 0 { STATUS_END_OF_FILE } else { STATUS_SUCCESS };
            k.with_state(|sys, _, _| {
                if let Some(rec) = sys.io.file_mut(file.1) {
                    rec.position = offset + total as u64;
                }
                n.signal_done(sys, &caller, file, status, total as u64);
            });
            write_iosb(k, iosb, status, total as u64)?;
            Ok(status)
        }
        Plan::Done(status) => Ok(status),
        Plan::Pending { irp, file, sync: true } => Ok(wait_sync(k, file, irp).0),
        Plan::Pending { .. } => Ok(STATUS_PENDING),
    }
}

/// `NtWriteFile(FileHandle, Event, ApcRoutine, ApcContext, IoStatusBlock*,
/// Buffer, Length, ByteOffset*, Key*)`. A handle with `FILE_APPEND_DATA`
/// but not `FILE_WRITE_DATA`, or a `ByteOffset` of
/// `FILE_WRITE_TO_END_OF_FILE`, writes at the end of the file.
pub fn write_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, event, apc_routine, apc_context, iosb, buf, len, off_ptr, key_ptr] = abi::args::<K, 9>(k, regs)?;
    let len = len as u32 as usize;
    let offset = read_offset(k, off_ptr)?;
    let key = if key_ptr == 0 { 0 } else { abi::read_u32(k, key_ptr)? };
    let n = Notify { event, apc_routine, apc_context, iosb };
    // A socket: a send (ADR 0095).
    if let Some(r) = crate::afd::services::write_file(k, h, n, buf, len) {
        return r;
    }
    let caller = k.caller();
    // What kind of file it is decides whether the data is copied in whole
    // (a request that outlives the call) or in pieces.
    let whole = k.with_state(|sys, _, _| -> Result<bool> {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), 0)?;
        let rec = sys.io.file(cookie).expect("checked");
        Ok(!rec.is_sync() || matches!(rec.target, Target::Pipe(_) | Target::Console(_)))
    })?;
    let captured = if whole {
        if len > MAX_CAPTURED_WRITE {
            return Err(STATUS_INSUFFICIENT_RESOURCES);
        }
        let mut data = alloc::vec![0u8; len];
        abi::read(k, buf, &mut data).or_else(|e| if len == 0 { Ok(()) } else { Err(e) })?;
        Some(data)
    } else {
        None
    };
    let plan = k.with_state(|sys, _, _| -> Result<Plan> {
        let file = file_handle(sys, &caller, Handle(h), 0)?;
        let granted = require_any(sys, &caller, Handle(h), FILE_WRITE_DATA | FILE_APPEND_DATA)?;
        n.check_event(sys, &caller)?;
        let rec = sys.io.file(file.1).expect("checked");
        let sync = rec.is_sync();
        match &rec.target {
            Target::Pipe(name) => {
                let name = name.clone();
                let data = captured.unwrap_or_default();
                let written = sys.io.pipe_write(&name, &data)?;
                n.signal_done(sys, &caller, file, STATUS_SUCCESS, written);
                Ok(Plan::Done(STATUS_SUCCESS))
            }
            // A console output handle: text in the output code page (ADR 0080).
            Target::Console(t) => {
                let t = *t;
                let t = sys.io.console_target(file.1).map(|_| t)?;
                if t.kind != crate::console::ConsoleKind::Output {
                    return Err(STATUS_INVALID_HANDLE);
                }
                let data = captured.unwrap_or_default();
                let written = sys.io.console_write_bytes(t.console, &data)?;
                n.signal_done(sys, &caller, file, STATUS_SUCCESS, written);
                Ok(Plan::Done(STATUS_SUCCESS))
            }
            // Written at its end: a redirected output shared by two handles
            // (`2>&1`) interleaves rather than overwrites.
            Target::Host(open) => Ok(Plan::Inline { open: open.clone(), offset: rec.position, file, append: true }),
            Target::Socket(_) => Err(STATUS_INVALID_DEVICE_REQUEST),
            Target::Disk(d) => {
                if rec.kind == FileType::Directory {
                    return Err(STATUS_INVALID_DEVICE_REQUEST);
                }
                let open = d.open.clone().filter(|o| !o.is_path_only()).ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
                let (volume, node, position) = (d.volume as u32, d.key, rec.position);
                let append_only = caller.mode == Mode::User && granted & FILE_WRITE_DATA == 0;
                let at: Option<u64> = match offset {
                    _ if append_only => None,
                    Some(FILE_WRITE_TO_END_OF_FILE) => None,
                    Some(o) if o >= 0 => Some(o as u64),
                    Some(FILE_USE_FILE_POINTER_POSITION) | None if sync => Some(position),
                    _ => return Err(STATUS_INVALID_PARAMETER),
                };
                if sync {
                    let start = match at {
                        Some(a) => a,
                        None => open.stat().map_err(status_of)?.size,
                    };
                    if !sys.io.locks.may_write(node, LockOwner { file: file.1, key }, start, len as u64) {
                        return Err(STATUS_FILE_LOCK_CONFLICT);
                    }
                    return Ok(Plan::Inline { open, offset: start, file, append: at.is_none() });
                }
                let data = captured.unwrap_or_default();
                let irp = n.irp(sys, &caller, file, false, IrpOp::Write { offset: at, data, key })?;
                let id = sys.io.queue(irp, volume);
                Ok(Plan::Pending { irp: id, file: file.0, sync: false })
            }
        }
    })?;
    match plan {
        Plan::Inline { open, offset, file, append } => {
            let mut chunk = alloc::vec![0u8; len.min(CHUNK)];
            let mut total = 0usize;
            while total < len {
                let want = chunk.len().min(len - total);
                abi::read(k, buf + total as u64, &mut chunk[..want])?;
                let at = if append { open.stat().map_err(status_of)?.size } else { offset + total as u64 };
                let put = write_at(&open, at, &chunk[..want])?;
                total += put;
                if put < want {
                    break;
                }
            }
            let end = if append { open.stat().map(|s| s.size).unwrap_or(0) } else { offset + total as u64 };
            k.with_state(|sys, _, _| {
                if let Some(rec) = sys.io.file_mut(file.1) {
                    rec.position = end;
                }
                n.signal_done(sys, &caller, file, STATUS_SUCCESS, total as u64);
            });
            write_iosb(k, iosb, STATUS_SUCCESS, total as u64)?;
            Ok(STATUS_SUCCESS)
        }
        Plan::Done(status) => {
            write_iosb(k, iosb, status, len as u64)?;
            Ok(status)
        }
        Plan::Pending { irp, file, sync: true } => Ok(wait_sync(k, file, irp).0),
        Plan::Pending { .. } => Ok(STATUS_PENDING),
    }
}

/// `NtFlushBuffersFile(FileHandle, IoStatusBlock*)`: needs write access
/// (Learn: "ZwFlushBuffersFile").
pub fn flush_buffers_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, iosb] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let open = k.with_state(|sys, _, _| -> Result<Option<Arc<OpenFile>>> {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), 0)?;
        require_any(sys, &caller, Handle(h), FILE_WRITE_DATA | FILE_APPEND_DATA)?;
        Ok(sys.io.file(cookie).and_then(|f| f.disk()).and_then(|d| d.open.clone()))
    })?;
    if let Some(open) = open
        && open.kind() == FileType::Regular
    {
        open.sync().map_err(status_of)?;
    }
    write_iosb(k, iosb, STATUS_SUCCESS, 0)?;
    Ok(STATUS_SUCCESS)
}

// ----- information

/// Copy a variable-length answer into a buffer of `len` bytes.
fn put_info<K: NtKernel>(k: &mut K, iosb: u64, out: u64, len: usize, bytes: &[u8], fixed: usize) -> Result<NtStatus> {
    if len < fixed {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let (n, status) = if bytes.len() > len { (len, STATUS_BUFFER_OVERFLOW) } else { (bytes.len(), STATUS_SUCCESS) };
    abi::write(k, out, &bytes[..n])?;
    write_iosb(k, iosb, status, n as u64)?;
    Ok(status)
}

/// `NtQueryInformationFile(FileHandle, IoStatusBlock*, FileInformation,
/// Length, FileInformationClass)`.
pub fn query_information_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, iosb, out, len, class] = abi::args::<K, 5>(k, regs)?;
    let class = class as u32;
    let caller = k.caller();
    let need = super::IoState::query_access(class).ok_or(STATUS_INVALID_INFO_CLASS)?;
    let answer = k.with_state(|sys, _, _| {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), need)?;
        let granted = sys.objects.granted_access(&caller, Handle(h))?;
        sys.io.query_info(cookie, class, granted)
    })?;
    put_info(k, iosb, out, len as u32 as usize, &answer.bytes, answer.fixed)
}

/// `NtSetInformationFile(FileHandle, IoStatusBlock*, FileInformation,
/// Length, FileInformationClass)`.
pub fn set_information_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, iosb, input, len, class] = abi::args::<K, 5>(k, regs)?;
    let class = class as u32;
    let len = len as u32 as usize;
    let (need, size) = super::IoState::set_access(class).ok_or(STATUS_INVALID_INFO_CLASS)?;
    if len < size {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let mut data = alloc::vec![0u8; size];
    abi::read(k, input, &mut data)?;
    // The rename target's name follows its fixed part.
    let rename = if class == FILE_RENAME_INFORMATION {
        let name_len = u32::from_le_bytes(data[16..20].try_into().expect("4 bytes")) as usize;
        if !name_len.is_multiple_of(2) || name_len == 0 || RENAME_INFO_FIXED + name_len > len.max(RENAME_INFO_FIXED) {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let mut raw = alloc::vec![0u8; name_len];
        abi::read(k, input + RENAME_INFO_FIXED as u64, &mut raw)?;
        let name: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let root = u64::from_le_bytes(data[8..16].try_into().expect("8 bytes"));
        Some((data[0] & 1 != 0, root, name))
    } else {
        None
    };
    let caller = k.caller();
    k.with_state(|sys, _, b| -> Result<()> {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), need)?;
        let q = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().expect("8 bytes"));
        let r = match class {
            FILE_BASIC_INFORMATION => sys.io.set_basic(cookie, &info::parse_basic(&data)),
            FILE_POSITION_INFORMATION => {
                let pos = q(0);
                if (pos as i64) < 0 {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                sys.io.file_mut(cookie).expect("checked").position = pos;
                Ok(())
            }
            FILE_END_OF_FILE_INFORMATION => sys.io.set_end_of_file(cookie, q(0), false),
            FILE_ALLOCATION_INFORMATION => sys.io.set_end_of_file(cookie, q(0), true),
            FILE_DISPOSITION_INFORMATION => sys.io.set_disposition(cookie, data[0] != 0, false),
            FILE_DISPOSITION_INFORMATION_EX => {
                let flags = u32::from_le_bytes(data[0..4].try_into().expect("4 bytes"));
                let ignore = flags & FILE_DISPOSITION_IGNORE_READONLY_ATTRIBUTE != 0;
                sys.io.set_disposition(cookie, flags & FILE_DISPOSITION_DELETE != 0, ignore)
            }
            FILE_RENAME_INFORMATION => {
                let (replace, root, name) = rename.expect("decoded");
                let to = if root != 0 {
                    let (_, dir) = file_handle(sys, &caller, Handle(root), 0)?;
                    RenameTo::Relative { dir, path: name }
                } else if name.first() == Some(&(b'\\' as u16)) {
                    let attrs =
                        Attributes { root: None, name: Some(name), attributes: poly_ntobj::OBJ_CASE_INSENSITIVE };
                    let (volume, path) =
                        sys.io.resolve_device_path(&mut sys.objects, &caller, &attrs.as_object_attributes())?;
                    RenameTo::Volume { volume, path }
                } else {
                    RenameTo::SameDirectory(name)
                };
                sys.io.rename(cookie, to, replace)
            }
            _ => Err(STATUS_INVALID_INFO_CLASS),
        };
        sys.reap(b);
        r
    })?;
    write_iosb(k, iosb, STATUS_SUCCESS, 0)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryVolumeInformationFile(FileHandle, IoStatusBlock*,
/// FsInformation, Length, FsInformationClass)`.
pub fn query_volume_information_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, iosb, out, len, class] = abi::args::<K, 5>(k, regs)?;
    let caller = k.caller();
    let answer = k.with_state(|sys, _, _| {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), 0)?;
        sys.io.query_volume(cookie, class as u32)
    })?;
    put_info(k, iosb, out, len as u32 as usize, &answer.bytes, answer.fixed)
}

/// `NtQueryDirectoryFile(FileHandle, Event, ApcRoutine, ApcContext,
/// IoStatusBlock*, FileInformation, Length, FileInformationClass,
/// ReturnSingleEntry, FileName*, RestartScan)`. The scan runs in the
/// calling thread and completes at once (also on an asynchronous handle).
pub fn query_directory_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, event, apc_routine, apc_context, iosb, out, len, class, single, name_ptr, restart] =
        abi::args::<K, 11>(k, regs)?;
    let pattern = if name_ptr == 0 { None } else { Some(abi::read_unicode(k, name_ptr)?) };
    let n = Notify { event, apc_routine, apc_context, iosb };
    let caller = k.caller();
    let q = dir::Query {
        class: class as u32,
        length: len as u32 as usize,
        single: single as u8 != 0,
        pattern: pattern.as_deref(),
        restart: restart as u8 != 0,
    };
    let answer = k.with_state(|sys, _, _| {
        let file = file_handle(sys, &caller, Handle(h), FILE_LIST_DIRECTORY).map_err(|s| (s, false))?;
        n.check_event(sys, &caller).map_err(|s| (s, false))?;
        let r = sys.io.query_directory(file.1, &q);
        let (status, info) = match &r {
            Ok(a) => (a.status, a.bytes.len() as u64),
            Err(s) => (*s, 0),
        };
        if r.is_ok() {
            n.signal_done(sys, &caller, file, status, info);
        }
        r.map_err(|s| (s, matches!(s, STATUS_NO_MORE_FILES | STATUS_NO_SUCH_FILE)))
    });
    match answer {
        Ok(a) => {
            abi::write(k, out, &a.bytes)?;
            write_iosb(k, iosb, a.status, a.bytes.len() as u64)?;
            Ok(a.status)
        }
        Err((s, completed)) => {
            if completed {
                write_iosb(k, iosb, s, 0)?;
            }
            Err(s)
        }
    }
}

// ----- locks

/// `NtLockFile(FileHandle, Event, ApcRoutine, ApcContext, IoStatusBlock*,
/// ByteOffset*, Length*, Key, FailImmediately, ExclusiveLock)`. The handle
/// needs read or write access (Learn: "LockFileEx").
pub fn lock_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, event, apc_routine, apc_context, iosb, off_ptr, len_ptr, key, fail_now, exclusive] =
        abi::args::<K, 10>(k, regs)?;
    let offset = abi::read_u64(k, off_ptr)?;
    let length = abi::read_u64(k, len_ptr)?;
    let n = Notify { event, apc_routine, apc_context, iosb };
    let caller = k.caller();
    let plan = k.with_state(|sys, _, _| -> Result<Plan> {
        let file = file_handle(sys, &caller, Handle(h), 0)?;
        require_any(sys, &caller, Handle(h), FILE_READ_DATA | FILE_WRITE_DATA)?;
        n.check_event(sys, &caller)?;
        let rec = sys.io.file(file.1).expect("checked");
        let sync = rec.is_sync();
        let node = rec.disk().filter(|_| rec.kind != FileType::Directory).ok_or(STATUS_INVALID_DEVICE_REQUEST)?.key;
        let (offset, length) = locks::check_range(offset, length)?;
        let lock = ByteLock {
            owner: LockOwner { file: file.1, key: key as u32 },
            offset,
            length,
            exclusive: exclusive as u8 != 0,
        };
        match sys.io.locks.lock(node, lock) {
            Ok(()) => {
                n.signal_done(sys, &caller, file, STATUS_SUCCESS, 0);
                Ok(Plan::Done(STATUS_SUCCESS))
            }
            Err(s) if fail_now as u8 != 0 => Ok(Plan::Done(s)),
            Err(_) => {
                let irp = n.irp(sys, &caller, file, sync, IrpOp::Lock { lock })?;
                let id = sys.io.park(irp);
                Ok(Plan::Pending { irp: id, file: file.0, sync })
            }
        }
    })?;
    match plan {
        Plan::Done(status) => {
            write_iosb(k, iosb, status, 0)?;
            if status == STATUS_SUCCESS { Ok(status) } else { Err(status) }
        }
        Plan::Pending { irp, file, sync: true } => Ok(wait_sync(k, file, irp).0),
        _ => Ok(STATUS_PENDING),
    }
}

/// `NtUnlockFile(FileHandle, IoStatusBlock*, ByteOffset*, Length*, Key)`.
pub fn unlock_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, iosb, off_ptr, len_ptr, key] = abi::args::<K, 5>(k, regs)?;
    let offset = abi::read_u64(k, off_ptr)?;
    let length = abi::read_u64(k, len_ptr)?;
    let caller = k.caller();
    k.with_state(|sys, _, _| -> Result<()> {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), 0)?;
        let node = sys.io.file(cookie).and_then(|f| f.disk()).ok_or(STATUS_INVALID_DEVICE_REQUEST)?.key;
        let (offset, length) = locks::check_range(offset, length)?;
        sys.io.locks.unlock(node, LockOwner { file: cookie, key: key as u32 }, offset, length)?;
        sys.io.retry_locks(node);
        Ok(())
    })?;
    write_iosb(k, iosb, STATUS_SUCCESS, 0)?;
    Ok(STATUS_SUCCESS)
}

// ----- cancellation

/// `NtCancelIoFile(FileHandle, IoStatusBlock*)`: cancel the calling
/// thread's pending requests on the file (Learn: "NtCancelIoFile"); each
/// completes with `STATUS_CANCELLED`.
pub fn cancel_io_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, iosb] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    k.with_state(|sys, _, _| -> Result<usize> {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), 0)?;
        Ok(sys.io.cancel_file_thread(cookie, caller.thread))
    })?;
    write_iosb(k, iosb, STATUS_SUCCESS, 0)?;
    Ok(STATUS_SUCCESS)
}

/// `NtCancelIoFileEx(FileHandle, IoRequestToCancel*, IoStatusBlock*)`: the
/// process's requests on the file, all or the one whose status block is
/// `IoRequestToCancel`; none is `STATUS_NOT_FOUND` (Learn: "CancelIoEx").
pub fn cancel_io_file_ex<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, request, iosb] = abi::args::<K, 3>(k, regs)?;
    let caller = k.caller();
    let n = k.with_state(|sys, _, _| -> Result<usize> {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), 0)?;
        Ok(sys.io.cancel_file_process(cookie, caller.process, (request != 0).then_some(request)))
    })?;
    if n == 0 {
        return Err(STATUS_NOT_FOUND);
    }
    write_iosb(k, iosb, STATUS_SUCCESS, 0)?;
    Ok(STATUS_SUCCESS)
}
