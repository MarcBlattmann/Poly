//! The system services of named pipes and completion ports that are not
//! the generic file services (ADR 0169): `NtCreateNamedPipeFile`,
//! `NtFsControlFile` for the pipe control codes, the pipe information
//! classes of `NtQueryInformationFile` / `NtSetInformationFile`, and
//! `NtCancelSynchronousIoFile`. The logic is [`super::npfs`]'s.
//!
//! On an asynchronous handle a control code that succeeds at once still
//! returns `STATUS_PENDING` and completes through the event, the status
//! block and the completion port, as the spec sheet of `NtFsControlFile`
//! observed; one that fails at once returns its status and queues nothing.

use super::consts::*;
use super::irp::{Irp, IrpOp};
use super::npfs::{NpCreate, NpDone};
use super::services::{Notify, check_create, file_handle, open_in_with, wait_sync, write_iosb};
use super::{FileRecord, Target};
use crate::abi;
use crate::kernel::{NtKernel, NtTimeout};
use crate::objects::return_handle;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NtStatus, ObjectId};

pub type Result<T> = core::result::Result<T, NtStatus>;

/// `THREAD_TERMINATE` (`winnt.h`), the right `CancelSynchronousIo` needs.
const THREAD_TERMINATE: u32 = 1;

/// The default time-out of a pipe made with none: 50 ms, relative.
pub const DEFAULT_PIPE_TIMEOUT: i64 = -500_000;

/// `NtCreateNamedPipeFile(FileHandle*, DesiredAccess, ObjectAttributes*,
/// IoStatusBlock*, ShareAccess, CreateDisposition, CreateOptions,
/// NamedPipeType, ReadMode, CompletionMode, MaximumInstances, InboundQuota,
/// OutboundQuota, DefaultTimeout*)` (`ntifs.h`).
pub fn create_named_pipe_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let a = abi::args::<K, 14>(k, regs)?;
    let [out, desired, attrs, iosb, share, disposition, options, pipe_type, read_mode, completion] =
        [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9]];
    let [max_instances, inbound, outbound, timeout_ptr] = [a[10], a[11], a[12], a[13]];
    let attrs = abi::read_attributes(k, attrs)?;
    let params = check_create(desired as u32, share as u32, disposition as u32, options as u32, 0)?;
    let (pipe_type, read_mode, completion) = (pipe_type as u32, read_mode as u32, completion as u32);
    let max_instances = max_instances as u32;
    if pipe_type > FILE_PIPE_MESSAGE_TYPE | FILE_PIPE_REJECT_REMOTE_CLIENTS
        || read_mode > FILE_PIPE_MESSAGE_TYPE
        || completion > FILE_PIPE_COMPLETE_OPERATION
        || (pipe_type & FILE_PIPE_MESSAGE_TYPE == 0 && read_mode == FILE_PIPE_MESSAGE_TYPE)
        || max_instances == 0
    {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let timeout = if timeout_ptr == 0 { DEFAULT_PIPE_TIMEOUT } else { abi::read_u64(k, timeout_ptr)? as i64 };
    let np = NpCreate {
        share: params.share,
        disposition: params.disposition,
        pipe_type,
        read_mode,
        completion_mode: completion,
        max_instances,
        inbound: inbound as u32,
        outbound: outbound as u32,
        timeout,
    };
    let caller = k.caller();
    let (h, info, _) =
        k.with_state(|sys, _, b| open_in_with(sys, b, &caller, &attrs, desired as u32, params, Some(np)))?;
    return_handle(k, out, h)?;
    write_iosb(k, iosb, STATUS_SUCCESS, info)?;
    Ok(STATUS_SUCCESS)
}

/// What a control code turned into.
enum Out {
    /// Finished in the call with `status`; `data` for the output buffer.
    Done { status: NtStatus, data: Vec<u8> },
    /// The request waits.
    Pending { irp: u64, file: ObjectId, sync: bool },
    /// `FSCTL_PIPE_WAIT`: the pipe to wait for and for how long.
    Wait { key: Vec<u16>, timeout: Option<i64> },
}

/// The request a control code makes (its operation is only a placeholder;
/// the data comes back through the completion).
fn request(
    sys: &mut crate::NtSystem,
    n: &Notify,
    caller: &poly_ntobj::Caller,
    file: (ObjectId, u64),
    sync: bool,
    out_len: usize,
    out_buf: u64,
) -> Result<Irp> {
    n.irp(sys, caller, file, sync, IrpOp::Read { offset: 0, len: out_len, buf: out_buf, key: 0 })
}

/// Give a refused request back: its event reference goes.
fn refuse(sys: &mut crate::NtSystem, irp: Irp, status: NtStatus) -> Result<Out> {
    if let Some(e) = irp.event {
        let _ = sys.objects.dereference(e);
    }
    Ok(Out::Done { status, data: Vec::new() })
}

/// `NtFsControlFile(FileHandle, Event, ApcRoutine, ApcContext,
/// IoStatusBlock*, FsControlCode, InputBuffer, InputBufferLength,
/// OutputBuffer, OutputBufferLength)`: the pipe control codes.
pub fn fs_control_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, event, apc_routine, apc_context, iosb, code, input, input_len, output, output_len] =
        abi::args::<K, 10>(k, regs)?;
    let (code, in_len, out_len) = (code as u32, input_len as u32 as usize, output_len as u32 as usize);
    let n = Notify { event, apc_routine, apc_context, iosb };
    let mut data = Vec::new();
    if in_len > 0 && input != 0 {
        if in_len > super::services::MAX_CAPTURED_WRITE {
            return Err(STATUS_INSUFFICIENT_RESOURCES);
        }
        data = alloc::vec![0u8; in_len];
        abi::read(k, input, &mut data)?;
    }
    let caller = k.caller();
    let outcome = k.with_state(|sys, _, _| -> Result<Out> {
        let file = file_handle(sys, &caller, Handle(h), 0)?;
        n.check_event(sys, &caller)?;
        let granted = sys.objects.granted_access(&caller, Handle(h))?;
        let rec = sys.io.file(file.1).expect("checked");
        let sync = rec.is_sync();
        let has_port = rec.port.is_some();
        match rec.target {
            Target::PipeRoot if code == FSCTL_PIPE_WAIT => {
                // FILE_PIPE_WAIT_FOR_BUFFER (`ntifs.h`): Timeout, NameLength,
                // TimeoutSpecified, Name.
                if data.len() < 16 {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                let timeout = i64::from_le_bytes(data[0..8].try_into().expect("8 bytes"));
                let name_len = u32::from_le_bytes(data[8..12].try_into().expect("4 bytes")) as usize;
                let specified = data[12] != 0;
                if !name_len.is_multiple_of(2) || 16 + name_len > data.len() {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                let name: Vec<u16> =
                    data[16..16 + name_len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let name = name.strip_prefix(&[u16::from(b'\\')]).unwrap_or(&name).to_vec();
                if name.is_empty() {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                Ok(Out::Wait { key: super::names::fold(&name), timeout: specified.then_some(timeout) })
            }
            // An anonymous pipe (`CreatePipe`, a `\Device\PolyPipe` stream)
            // answers `FSCTL_PIPE_PEEK` (`PeekNamedPipe`) like a byte-mode
            // named pipe: the header, then what is there.
            Target::Pipe(ref key) if code == FSCTL_PIPE_PEEK => {
                if out_len < super::npfs::PEEK_HEADER {
                    return Err(STATUS_INFO_LENGTH_MISMATCH);
                }
                let pipe = sys.io.pipes.get(key).ok_or(STATUS_INVALID_HANDLE)?;
                if pipe.data.is_empty() && pipe.writers == 0 && pipe.had_writer {
                    return Err(STATUS_PIPE_BROKEN);
                }
                let max = out_len - super::npfs::PEEK_HEADER;
                let available = pipe.data.len();
                let mut data = Vec::with_capacity(super::npfs::PEEK_HEADER + max.min(available));
                for v in [FILE_PIPE_CONNECTED_STATE, available as u32, 0, 0] {
                    data.extend_from_slice(&v.to_le_bytes());
                }
                data.extend(pipe.data.iter().take(max).copied());
                let status = if available > max { STATUS_BUFFER_OVERFLOW } else { STATUS_SUCCESS };
                Ok(Out::Done { status, data })
            }
            Target::NamedPipe(_) => {
                // The access bits of the code (`CTL_CODE`): bit 14 read, 15 write.
                let need = (if code & (1 << 14) != 0 { FILE_READ_DATA } else { 0 })
                    | (if code & (1 << 15) != 0 { FILE_WRITE_DATA } else { 0 });
                if caller.mode == poly_ntobj::Mode::User && granted & need != need {
                    return Err(STATUS_ACCESS_DENIED);
                }
                let irp = request(sys, &n, &caller, file, sync, out_len, output)?;
                let result = match code {
                    FSCTL_PIPE_LISTEN => sys.io.np_listen(file.1, irp),
                    FSCTL_PIPE_TRANSCEIVE => {
                        if has_port && apc_routine != 0 {
                            return refuse(sys, irp, STATUS_INVALID_PARAMETER);
                        }
                        sys.io.np_transceive(file.1, irp, core::mem::take(&mut data))
                    }
                    FSCTL_PIPE_DISCONNECT => {
                        return match sys.io.np_disconnect(file.1) {
                            Ok(()) => finish_now(sys, irp, sync, STATUS_SUCCESS, Vec::new(), 0),
                            Err(s) => refuse(sys, irp, s),
                        };
                    }
                    FSCTL_PIPE_PEEK => {
                        if out_len < super::npfs::PEEK_HEADER {
                            return refuse(sys, irp, STATUS_INFO_LENGTH_MISMATCH);
                        }
                        let max = out_len - super::npfs::PEEK_HEADER;
                        return match sys.io.np_peek(file.1, max) {
                            Ok((bytes, truncated)) => {
                                let status = if truncated { STATUS_BUFFER_OVERFLOW } else { STATUS_SUCCESS };
                                let len = bytes.len() as u64;
                                finish_now(sys, irp, sync, status, bytes, len)
                            }
                            Err(s) => refuse(sys, irp, s),
                        };
                    }
                    FSCTL_POLY_PIPE_PROCESSES => {
                        return match sys.io.np_processes(file.1) {
                            Ok(bytes) => {
                                let len = bytes.len() as u64;
                                finish_now(sys, irp, sync, STATUS_SUCCESS, bytes, len)
                            }
                            Err(s) => refuse(sys, irp, s),
                        };
                    }
                    _ => return refuse(sys, irp, STATUS_NOT_SUPPORTED),
                };
                match result {
                    Ok(NpDone::Pending(id)) => Ok(Out::Pending { irp: id, file: file.0, sync }),
                    Ok(NpDone::Done(status)) => {
                        // Completed in the call (a reply already waiting).
                        Ok(Out::Done { status: if sync { status } else { STATUS_PENDING }, data: Vec::new() })
                    }
                    Err((s, irp)) => refuse(sys, irp, s),
                }
            }
            _ => Err(STATUS_INVALID_DEVICE_REQUEST),
        }
    })?;
    match outcome {
        Out::Done { status, data } => {
            if !data.is_empty() {
                if data.len() > out_len || output == 0 {
                    return Err(STATUS_BUFFER_TOO_SMALL);
                }
                abi::write(k, output, &data)?;
            }
            if status != STATUS_PENDING {
                write_iosb(k, iosb, status, data.len() as u64)?;
            }
            Ok(status)
        }
        Out::Pending { irp, file, sync: true } => Ok(wait_sync(k, file, irp).0),
        Out::Pending { .. } => Ok(STATUS_PENDING),
        Out::Wait { key, timeout } => wait_for_pipe(k, &key, timeout),
    }
}

/// An immediate success: on a synchronous handle the caller writes the
/// output and status block itself; on an asynchronous one the request
/// completes through the usual machinery (output, status block, event,
/// port) and the call returns `STATUS_PENDING`.
fn finish_now(
    sys: &mut crate::NtSystem,
    mut irp: Irp,
    sync: bool,
    status: NtStatus,
    data: Vec<u8>,
    information: u64,
) -> Result<Out> {
    if sync {
        if let Some(e) = irp.event {
            let _ = sys.objects.dereference(e);
        }
        return Ok(Out::Done { status, data });
    }
    let buf = match irp.op {
        IrpOp::Read { buf, .. } => buf,
        _ => 0,
    };
    irp.sync = false;
    sys.io.complete_now(irp, status, information, (!data.is_empty()).then_some((buf, data)));
    Ok(Out::Done { status: STATUS_PENDING, data: Vec::new() })
}

/// `FSCTL_PIPE_WAIT`: wait until an instance of pipe `key` listens, for
/// `timeout` (100 ns, negative relative; `i64::MIN` for ever; `None`: the
/// pipe's own default). Polled every millisecond.
fn wait_for_pipe<K: NtKernel>(k: &mut K, key: &[u16], timeout: Option<i64>) -> Result<NtStatus> {
    let start = k.monotonic_100ns();
    loop {
        let state = k.with_state(|sys, _, _| -> Result<(bool, i64)> {
            if !sys.io.np_exists(key) {
                return Err(STATUS_OBJECT_NAME_NOT_FOUND);
            }
            let default = sys.io.np_default_timeout(key).unwrap_or(DEFAULT_PIPE_TIMEOUT);
            Ok((sys.io.np_listening(key), default))
        })?;
        if state.0 {
            return Ok(STATUS_SUCCESS);
        }
        let limit = timeout.unwrap_or(state.1);
        if limit != i64::MIN {
            let allowed = if limit < 0 { limit.unsigned_abs() } else { 0 };
            if k.monotonic_100ns().saturating_sub(start) >= allowed {
                return Ok(STATUS_IO_TIMEOUT);
            }
        }
        let s = k.delay(NtTimeout::Relative(10_000), false);
        if s != STATUS_SUCCESS && s != STATUS_TIMEOUT {
            return Ok(s);
        }
    }
}

/// `NtQueryInformationFile` on a pipe: the pipe classes. `None` for any
/// other class (the generic code answers).
pub(crate) fn query_pipe_info(sys: &crate::NtSystem, cookie: u64, class: u32) -> Option<Result<Vec<u8>>> {
    sys.io.np_end(cookie)?;
    match class {
        FILE_PIPE_INFORMATION => Some(sys.io.np_handle_info(cookie)),
        FILE_PIPE_LOCAL_INFORMATION => Some(sys.io.np_local_info(cookie)),
        FILE_PIPE_REMOTE_INFORMATION => Some(Ok(alloc::vec![0u8; 16])),
        _ => None,
    }
}

/// The record behind a file object, for the generic services.
#[allow(dead_code, reason = "used by the tests")]
pub(crate) fn record(sys: &crate::NtSystem, cookie: u64) -> Option<&FileRecord> {
    sys.io.file(cookie)
}

/// `NtCancelSynchronousIoFile(ThreadHandle, IoRequestToCancel*,
/// IoStatusBlock*)`: cancel the synchronous requests a thread is waiting
/// on (Learn: "CancelSynchronousIo"). None is `STATUS_NOT_FOUND`.
pub fn cancel_synchronous_io_file<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, request, iosb] = abi::args::<K, 3>(k, regs)?;
    let thread = crate::threads::thread_of(k, Handle(h), THREAD_TERMINATE)?;
    let n = k.with_state(|sys, _, _| {
        sys.io.cancel_where(|i| i.sync && i.thread == thread && (request == 0 || i.iosb == request), false)
    });
    if n == 0 {
        if iosb != 0 {
            write_iosb(k, iosb, STATUS_NOT_FOUND, 0)?;
        }
        return Err(STATUS_NOT_FOUND);
    }
    if iosb != 0 {
        write_iosb(k, iosb, STATUS_SUCCESS, 0)?;
    }
    Ok(STATUS_SUCCESS)
}

/// Whether `class` is one of the information classes this module serves
/// for `NtQueryInformationFile`.
pub(crate) fn is_query_class(class: u32) -> bool {
    matches!(class, FILE_PIPE_INFORMATION | FILE_PIPE_LOCAL_INFORMATION | FILE_PIPE_REMOTE_INFORMATION)
}

/// Whether `class` is one of the classes `NtSetInformationFile` is served
/// here for: the pipe's modes, the completion port, the notification modes.
pub(crate) fn is_set_class(class: u32) -> bool {
    matches!(class, FILE_PIPE_INFORMATION | FILE_COMPLETION_INFORMATION | FILE_IO_COMPLETION_NOTIFICATION_INFORMATION)
}

/// `NtQueryInformationFile` for the pipe classes. A file object that is no
/// pipe is `STATUS_INVALID_PARAMETER`. The local information of a server end
/// needs `FILE_READ_ATTRIBUTES` (an outbound server handle has none).
pub fn query_pipe_information<K: NtKernel>(
    k: &mut K,
    h: u64,
    iosb: u64,
    out: u64,
    len: usize,
    class: u32,
) -> Result<NtStatus> {
    let caller = k.caller();
    let bytes = k.with_state(|sys, _, _| -> Result<Vec<u8>> {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), 0)?;
        let end = sys.io.np_end(cookie).ok_or(STATUS_INVALID_PARAMETER)?;
        let granted = sys.objects.granted_access(&caller, Handle(h))?;
        if class == FILE_PIPE_LOCAL_INFORMATION
            && end.server
            && caller.mode == poly_ntobj::Mode::User
            && granted & FILE_READ_ATTRIBUTES == 0
        {
            return Err(STATUS_ACCESS_DENIED);
        }
        query_pipe_info(sys, cookie, class).unwrap_or(Err(STATUS_INVALID_INFO_CLASS))
    })?;
    super::services::put_info(k, iosb, out, len, &bytes, bytes.len())
}

/// `NtSetInformationFile` for the pipe modes, the port association and the
/// completion notification modes.
pub fn set_special_information<K: NtKernel>(
    k: &mut K,
    h: u64,
    iosb: u64,
    input: u64,
    len: usize,
    class: u32,
) -> Result<NtStatus> {
    let size = match class {
        FILE_PIPE_INFORMATION => 8,
        FILE_COMPLETION_INFORMATION => 16,
        _ => 4,
    };
    if len < size {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let mut data = alloc::vec![0u8; size];
    abi::read(k, input, &mut data)?;
    let caller = k.caller();
    let q = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().expect("8 bytes"));
    let d = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().expect("4 bytes"));
    k.with_state(|sys, _, b| -> Result<()> {
        let (_, cookie) = file_handle(sys, &caller, Handle(h), 0)?;
        let r = match class {
            FILE_PIPE_INFORMATION => {
                if sys.io.np_end(cookie).is_none() {
                    Err(STATUS_INVALID_PARAMETER)
                } else {
                    sys.io.np_set_handle_info(cookie, d(0), d(4))
                }
            }
            FILE_COMPLETION_INFORMATION => sys.io.associate_port(&mut sys.objects, &caller, cookie, q(0), q(8)),
            _ => {
                let flags = d(0);
                if flags & !(FILE_SKIP_COMPLETION_PORT_ON_SUCCESS | FILE_SKIP_SET_EVENT_ON_HANDLE) != 0 {
                    Err(STATUS_INVALID_PARAMETER)
                } else {
                    let rec = sys.io.file_mut(cookie).expect("checked");
                    rec.skip_port = flags & FILE_SKIP_COMPLETION_PORT_ON_SUCCESS != 0;
                    rec.skip_event = flags & FILE_SKIP_SET_EVENT_ON_HANDLE != 0;
                    Ok(())
                }
            }
        };
        sys.reap(b);
        r
    })?;
    write_iosb(k, iosb, STATUS_SUCCESS, 0)?;
    Ok(STATUS_SUCCESS)
}
