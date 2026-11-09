//! I/O completion ports, overlapped I/O results and cancellation, and the
//! APC-completed `ReadFileEx` / `WriteFileEx` (Learn: "I/O Completion
//! Ports", "Synchronous and Asynchronous I/O", "GetQueuedCompletionStatus",
//! "ReadFileEx"; ADR 0169), over `NtCreateIoCompletion`,
//! `NtRemoveIoCompletion(Ex)`, `NtSetInformationFile`
//! (`FileCompletionInformation`, `FileIoCompletionNotificationInformation`),
//! `NtCancelIoFile(Ex)` and `NtCancelSynchronousIoFile`. Also the three
//! process and thread lookups the Chrome IPC code uses: `OpenProcess`,
//! `OpenThread`, `GetThreadId`, and `QueueUserAPC`.

use crate::base::*;
use crate::file::Overlapped;
use crate::nt::{self, Handle, IoStatusBlock};
use alloc::boxed::Box;
use poly_kernelbase::error::*;
use poly_kernelbase::namedpipe as np;
use poly_ntobj::NtStatus;

const FILE_COMPLETION_INFORMATION: u32 = 30;
const FILE_IO_COMPLETION_NOTIFICATION_INFORMATION: u32 = 41;
const IO_COMPLETION_ALL_ACCESS: u32 = 0x001F_0003;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_IO_COMPLETION: u32 = 192;
const STATUS_NOT_FOUND: NtStatus = NtStatus(0xC000_0225);

fn set_info(h: Handle, class: u32, data: &[u8]) -> Result<(), NtStatus> {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtSetInformationFile(h, &mut iosb, data.as_ptr(), data.len() as u32, class) };
    if st.is_success() { Ok(()) } else { Err(st) }
}

/// A new port.
fn new_port(threads: u32) -> Result<Handle, NtStatus> {
    let mut h: Handle = 0;
    // SAFETY: a valid out-pointer; no attributes: an unnamed port.
    let st = unsafe { nt::NtCreateIoCompletion(&mut h, IO_COMPLETION_ALL_ACCESS, core::ptr::null(), threads) };
    if st.is_success() { Ok(h) } else { Err(st) }
}

/// `CreateIoCompletionPort`. A new port (`INVALID_HANDLE_VALUE` for the file
/// and no port), or `file` associated with `port` (made on the way when
/// `port` is NULL), returning the port.
#[unsafe(no_mangle)]
pub extern "system" fn CreateIoCompletionPort(file: Handle, port: Handle, key: usize, threads: u32) -> Handle {
    if file == INVALID_HANDLE_VALUE {
        if port != 0 {
            return fail(ERROR_INVALID_PARAMETER, 0);
        }
        return match new_port(threads) {
            Ok(h) => h,
            Err(st) => fail_status(st, 0),
        };
    }
    if file == 0 || port == INVALID_HANDLE_VALUE {
        return fail(ERROR_INVALID_HANDLE, 0);
    }
    let (port, made) = if port == 0 {
        match new_port(threads) {
            Ok(h) => (h, true),
            Err(st) => return fail_status(st, 0),
        }
    } else {
        (port, false)
    };
    let mut info = [0u8; 16];
    info[0..8].copy_from_slice(&(port as u64).to_le_bytes());
    info[8..16].copy_from_slice(&(key as u64).to_le_bytes());
    match set_info(file, FILE_COMPLETION_INFORMATION, &info) {
        Ok(()) => port,
        Err(st) => {
            if made {
                close(port);
            }
            fail_status(st, 0)
        }
    }
}

/// `GetQueuedCompletionStatus`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetQueuedCompletionStatus(
    port: Handle,
    bytes: *mut u32,
    key: *mut usize,
    overlapped: *mut *mut Overlapped,
    ms: u32,
) -> i32 {
    let timeout = np::nt_timeout(ms);
    let tp = timeout.as_ref().map_or(core::ptr::null(), |t| t as *const i64);
    let (mut k, mut ctx) = (0usize, 0usize);
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtRemoveIoCompletion(port, &mut k, &mut ctx, &mut iosb, tp) };
    // SAFETY: the caller's out-pointers.
    unsafe {
        if st != nt::STATUS_TIMEOUT && !st.is_success() && st != nt::STATUS_USER_APC {
            if !overlapped.is_null() {
                overlapped.write(core::ptr::null_mut());
            }
            return fail_status(st, FALSE);
        }
        if st == nt::STATUS_TIMEOUT || st == nt::STATUS_USER_APC {
            if !overlapped.is_null() {
                overlapped.write(core::ptr::null_mut());
            }
            return fail(WAIT_TIMEOUT, FALSE);
        }
        if !bytes.is_null() {
            bytes.write(iosb.information as u32);
        }
        if !key.is_null() {
            key.write(k);
        }
        if !overlapped.is_null() {
            overlapped.write(ctx as *mut Overlapped);
        }
    }
    let status = NtStatus(iosb.status as u32);
    if (status.0 as i32) < 0 { fail_status(status, FALSE) } else { TRUE }
}

/// `OVERLAPPED_ENTRY` (`minwinbase.h`): the key, the `OVERLAPPED`, the
/// internal status and the byte count (32 bytes).
#[repr(C)]
pub struct OverlappedEntry {
    pub key: usize,
    pub overlapped: *mut Overlapped,
    pub internal: usize,
    pub bytes: u32,
}

/// `GetQueuedCompletionStatusEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetQueuedCompletionStatusEx(
    port: Handle,
    entries: *mut OverlappedEntry,
    count: u32,
    removed: *mut u32,
    ms: u32,
    alertable: i32,
) -> i32 {
    if count == 0 || entries.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let timeout = np::nt_timeout(ms);
    let tp = timeout.as_ref().map_or(core::ptr::null(), |t| t as *const i64);
    let mut raw = alloc::vec![0u8; count as usize * 32];
    let mut n = 0u32;
    // SAFETY: valid pointers; `raw` holds `count` records.
    let st = unsafe { nt::NtRemoveIoCompletionEx(port, raw.as_mut_ptr(), count, &mut n, tp, alertable as u8) };
    if st == nt::STATUS_TIMEOUT || st == nt::STATUS_USER_APC || !st.is_success() {
        if !removed.is_null() {
            // SAFETY: the caller's out-pointer.
            unsafe { removed.write(0) };
        }
        return if st == nt::STATUS_TIMEOUT {
            fail(WAIT_TIMEOUT, FALSE)
        } else if st == nt::STATUS_USER_APC {
            fail(WAIT_IO_COMPLETION, FALSE)
        } else {
            fail_status(st, FALSE)
        };
    }
    for i in 0..n as usize {
        let r = &raw[i * 32..i * 32 + 32];
        let q = |o: usize| u64::from_le_bytes(r[o..o + 8].try_into().expect("8 bytes")) as usize;
        // SAFETY: `i` < `count`: the caller's array.
        unsafe {
            entries.add(i).write(OverlappedEntry {
                key: q(0),
                overlapped: q(8) as *mut Overlapped,
                internal: u32::from_le_bytes(r[16..20].try_into().expect("4 bytes")) as usize,
                bytes: q(24) as u32,
            });
        }
    }
    if !removed.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { removed.write(n) };
    }
    TRUE
}

/// `PostQueuedCompletionStatus`: a packet of status success whose
/// information is the byte count.
#[unsafe(no_mangle)]
pub extern "system" fn PostQueuedCompletionStatus(
    port: Handle,
    bytes: u32,
    key: usize,
    overlapped: *mut Overlapped,
) -> i32 {
    // SAFETY: a handle value.
    let st = unsafe { nt::NtSetIoCompletion(port, key, overlapped as usize, NtStatus(0), bytes as usize) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `SetFileCompletionNotificationModes`: flags 0 look at nothing.
#[unsafe(no_mangle)]
pub extern "system" fn SetFileCompletionNotificationModes(file: Handle, flags: u8) -> i32 {
    if flags == 0 {
        return TRUE;
    }
    if let Err(e) = np::check_notification_flags(flags) {
        return fail(e, FALSE);
    }
    match set_info(file, FILE_IO_COMPLETION_NOTIFICATION_INFORMATION, &u32::from(flags).to_le_bytes()) {
        Ok(()) => TRUE,
        Err(st) => fail_status(st, FALSE),
    }
}

// ----- results and waiting

/// The handle or event `GetOverlappedResult` waits on: the `OVERLAPPED`'s
/// event (its low bit is a flag), else the file.
fn wait_object(file: Handle, ov: *mut Overlapped) -> Handle {
    // SAFETY: the caller's OVERLAPPED.
    let e = unsafe { (*ov).event } & !1;
    if e != 0 { e } else { file }
}

/// `GetOverlappedResult` and `Ex`: `ms` is the wait (`INFINITE` for ever,
/// 0 not at all).
unsafe fn overlapped_result(file: Handle, ov: *mut Overlapped, bytes: *mut u32, ms: u32, alertable: bool, ex: bool) -> i32 {
    if ov.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    loop {
        // SAFETY: the caller's OVERLAPPED, written by the kernel; read once.
        let status = unsafe { core::ptr::read_volatile(&(*ov).internal) } as u32;
        if status == np::STATUS_PENDING {
            if ms == 0 {
                return fail(ERROR_IO_INCOMPLETE, FALSE);
            }
            let timeout = np::nt_timeout(ms);
            let tp = timeout.as_ref().map_or(core::ptr::null(), |t| t as *const i64);
            // SAFETY: a handle value and a valid timeout.
            let st = unsafe { nt::NtWaitForSingleObject(wait_object(file, ov), alertable as u8, tp) };
            if st == nt::STATUS_TIMEOUT {
                return fail(if ex { WAIT_TIMEOUT } else { ERROR_IO_INCOMPLETE }, FALSE);
            }
            if st == nt::STATUS_USER_APC || st.0 == 0x101 {
                return fail(WAIT_IO_COMPLETION, FALSE);
            }
            if !st.is_success() {
                return fail_status(st, FALSE);
            }
            continue;
        }
        if alertable {
            // Learn: an alertable call runs queued APCs also when the request is done.
            // SAFETY: no arguments.
            unsafe { nt::NtTestAlert() };
        }
        // SAFETY: the OVERLAPPED is complete.
        let high = unsafe { (*ov).internal_high };
        if !bytes.is_null() {
            // SAFETY: the caller's out-pointer.
            unsafe { bytes.write(high as u32) };
        }
        return match np::overlapped_outcome(status, high) {
            Ok(_) => TRUE,
            Err((error, _)) => fail(error, FALSE),
        };
    }
}

/// `GetOverlappedResult`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetOverlappedResult(file: Handle, ov: *mut Overlapped, bytes: *mut u32, wait: i32) -> i32 {
    // SAFETY: the caller's pointers.
    unsafe { overlapped_result(file, ov, bytes, if wait != 0 { u32::MAX } else { 0 }, false, false) }
}

/// `GetOverlappedResultEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetOverlappedResultEx(
    file: Handle,
    ov: *mut Overlapped,
    bytes: *mut u32,
    ms: u32,
    alertable: i32,
) -> i32 {
    // SAFETY: the caller's pointers.
    unsafe { overlapped_result(file, ov, bytes, ms, alertable != 0, true) }
}

// ----- cancellation

/// `CancelIo`: the calling thread's requests on `file`.
#[unsafe(no_mangle)]
pub extern "system" fn CancelIo(file: Handle) -> i32 {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtCancelIoFile(file, &mut iosb) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `CancelIoEx`: the process's requests on `file`, all or the one of `ov`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CancelIoEx(file: Handle, ov: *mut Overlapped) -> i32 {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers; the OVERLAPPED is the request's status block.
    let st = unsafe { nt::NtCancelIoFileEx(file, ov as *const IoStatusBlock, &mut iosb) };
    if st.is_success() {
        TRUE
    } else if st == STATUS_NOT_FOUND {
        fail(ERROR_NOT_FOUND, FALSE)
    } else {
        fail_status(st, FALSE)
    }
}

/// `CancelSynchronousIo`: the synchronous request `thread` waits on.
#[unsafe(no_mangle)]
pub extern "system" fn CancelSynchronousIo(thread: Handle) -> i32 {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtCancelSynchronousIoFile(thread, core::ptr::null(), &mut iosb) };
    if st.is_success() {
        TRUE
    } else if st == STATUS_NOT_FOUND {
        fail(ERROR_NOT_FOUND, FALSE)
    } else {
        fail_status(st, FALSE)
    }
}

// ----- ReadFileEx, WriteFileEx

/// The routine of a `ReadFileEx` / `WriteFileEx` request and its
/// `OVERLAPPED`, the APC's context.
struct ApcRecord {
    routine: unsafe extern "system" fn(u32, u32, *mut Overlapped),
    overlapped: *mut Overlapped,
}

/// The APC the kernel queues when the request completes: `PIO_APC_ROUTINE`
/// (`ApcContext`, `IoStatusBlock`, reserved) calling the Win32 completion
/// routine with (error, bytes, `OVERLAPPED`).
unsafe extern "system" fn io_apc(context: usize, iosb: *mut IoStatusBlock, _reserved: usize) {
    // SAFETY: the record `read_write_ex` leaked; the status block is complete.
    unsafe {
        let rec = Box::from_raw(context as *mut ApcRecord);
        let status = NtStatus((*iosb).status as u32);
        let bytes = (*iosb).information as u32;
        let error = if status.0 == 0 { 0 } else { nt::RtlNtStatusToDosError(status) };
        (rec.routine)(error, bytes, rec.overlapped);
    }
}

unsafe fn read_write_ex(
    file: Handle,
    buffer: *mut u8,
    len: u32,
    ov: *mut Overlapped,
    routine: Option<unsafe extern "system" fn(u32, u32, *mut Overlapped)>,
    write: bool,
) -> i32 {
    let Some(routine) = routine else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    if ov.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let record = Box::into_raw(Box::new(ApcRecord { routine, overlapped: ov }));
    // SAFETY: the caller's OVERLAPPED.
    let offset = unsafe {
        (*ov).internal = nt::STATUS_PENDING.0 as u64;
        poly_kernelbase::file::overlapped_offset((*ov).offset, (*ov).offset_high)
    };
    let apc = io_apc as unsafe extern "system" fn(usize, *mut IoStatusBlock, usize) as *const u8;
    // SAFETY: the caller's buffer of `len` bytes; the record outlives the request.
    let st = unsafe {
        if write {
            nt::NtWriteFile(file, 0, apc, record as *const u8, ov as *mut IoStatusBlock, buffer, len, &offset, core::ptr::null())
        } else {
            nt::NtReadFile(file, 0, apc, record as *const u8, ov as *mut IoStatusBlock, buffer, len, &offset, core::ptr::null())
        }
    };
    if st == nt::STATUS_PENDING || st.is_success() || st == nt::STATUS_END_OF_FILE {
        return TRUE;
    }
    // SAFETY: the request did not start: nobody else has the record.
    drop(unsafe { Box::from_raw(record) });
    fail_status(st, FALSE)
}

/// `ReadFileEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReadFileEx(
    file: Handle,
    buffer: *mut u8,
    len: u32,
    ov: *mut Overlapped,
    routine: Option<unsafe extern "system" fn(u32, u32, *mut Overlapped)>,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { read_write_ex(file, buffer, len, ov, routine, false) }
}

/// `WriteFileEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WriteFileEx(
    file: Handle,
    buffer: *const u8,
    len: u32,
    ov: *mut Overlapped,
    routine: Option<unsafe extern "system" fn(u32, u32, *mut Overlapped)>,
) -> i32 {
    // SAFETY: the caller's arguments; a write only reads the buffer.
    unsafe { read_write_ex(file, buffer as *mut u8, len, ov, routine, true) }
}

// ----- processes and threads by id

/// `OpenProcess`.
#[unsafe(no_mangle)]
pub extern "system" fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle {
    let oa = nt::ObjectAttributes::new(None, if inherit != 0 { nt::OBJ_INHERIT } else { 0 });
    let client = [pid as usize, 0usize];
    let mut h: Handle = 0;
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenProcess(&mut h, access, &oa, &client) };
    if st.is_success() {
        h
    } else if st.0 == 0xC000_000B {
        // STATUS_INVALID_CID: Learn: ERROR_INVALID_PARAMETER.
        fail(ERROR_INVALID_PARAMETER, 0)
    } else {
        fail_status(st, 0)
    }
}

/// `OpenThread`.
#[unsafe(no_mangle)]
pub extern "system" fn OpenThread(access: u32, inherit: i32, tid: u32) -> Handle {
    let oa = nt::ObjectAttributes::new(None, if inherit != 0 { nt::OBJ_INHERIT } else { 0 });
    let client = [0usize, tid as usize];
    let mut h: Handle = 0;
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenThread(&mut h, access, &oa, &client) };
    if st.is_success() {
        h
    } else if st.0 == 0xC000_000B {
        fail(ERROR_INVALID_PARAMETER, 0)
    } else {
        fail_status(st, 0)
    }
}

/// `GetThreadId`: `THREAD_BASIC_INFORMATION.ClientId.UniqueThread`.
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadId(thread: Handle) -> u32 {
    let mut b = [0u8; 48];
    let mut ret = 0u32;
    // SAFETY: valid pointers; THREAD_BASIC_INFORMATION is 48 bytes.
    let st = unsafe { nt::NtQueryInformationThread(thread, 0, b.as_mut_ptr(), 48, &mut ret) };
    if st.is_success() {
        u64::from_le_bytes(b[24..32].try_into().expect("8 bytes")) as u32
    } else {
        fail_status(st, 0)
    }
}

/// `QueueUserAPC`: the routine runs on `thread` in an alertable wait.
#[unsafe(no_mangle)]
pub extern "system" fn QueueUserAPC(routine: usize, thread: Handle, data: usize) -> u32 {
    // SAFETY: a handle value; the routine is the caller's.
    let st = unsafe { nt::NtQueueApcThread(thread, routine, data, 0, 0) };
    if st.is_success() { 1 } else { fail_status(st, 0) }
}
