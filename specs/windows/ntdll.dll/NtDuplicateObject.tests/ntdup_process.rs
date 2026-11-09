//! NtDuplicateObject / DuplicateHandle between two processes: pushing a handle into
//! a child (same access and reduced access), pulling one out of a child with
//! DUPLICATE_CLOSE_SOURCE, the rights PROCESS_DUP_HANDLE needs. The child talks to
//! the test over a pipe (Learn: "DuplicateHandle function", "ZwDuplicateObject",
//! "Process Security and Access Rights").
#![no_std]
#![no_main]
#![allow(dead_code, non_snake_case, non_camel_case_types, unused_imports)]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use poly_wtest::*;

pub type BOOL = i32;

// --- winerror.h ---
pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_INVALID_FUNCTION: u32 = 1;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_GEN_FAILURE: u32 = 31;
pub const ERROR_HANDLE_EOF: u32 = 38;
pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_BROKEN_PIPE: u32 = 109;
pub const ERROR_SEM_TIMEOUT: u32 = 121;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const ERROR_BAD_PIPE: u32 = 230;
pub const ERROR_PIPE_BUSY: u32 = 231;
pub const ERROR_NO_DATA: u32 = 232;
pub const ERROR_PIPE_NOT_CONNECTED: u32 = 233;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_PIPE_CONNECTED: u32 = 535;
pub const ERROR_PIPE_LISTENING: u32 = 536;
pub const ERROR_ABANDONED_WAIT_0: u32 = 735;
pub const ERROR_OPERATION_ABORTED: u32 = 995;
pub const ERROR_IO_INCOMPLETE: u32 = 996;
pub const ERROR_IO_PENDING: u32 = 997;
pub const ERROR_NOACCESS: u32 = 998;
pub const ERROR_NOT_FOUND: u32 = 1168;
pub const ERROR_INVALID_USER_BUFFER: u32 = 1784;
pub const WAIT_OBJECT_0: u32 = 0;
pub const WAIT_IO_COMPLETION: u32 = 0xC0;
pub const WAIT_TIMEOUT: u32 = 258;
pub const WAIT_FAILED: u32 = 0xFFFF_FFFF;
pub const INFINITE: u32 = 0xFFFF_FFFF;
pub const STILL_ACTIVE: u32 = 259;

// --- winbase.h ---
pub const PIPE_ACCESS_INBOUND: u32 = 1;
pub const PIPE_ACCESS_OUTBOUND: u32 = 2;
pub const PIPE_ACCESS_DUPLEX: u32 = 3;
pub const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
pub const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
pub const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
pub const WRITE_DAC: u32 = 0x0004_0000;
pub const WRITE_OWNER: u32 = 0x0008_0000;
pub const PIPE_WAIT: u32 = 0;
pub const PIPE_NOWAIT: u32 = 1;
pub const PIPE_READMODE_BYTE: u32 = 0;
pub const PIPE_READMODE_MESSAGE: u32 = 2;
pub const PIPE_TYPE_BYTE: u32 = 0;
pub const PIPE_TYPE_MESSAGE: u32 = 4;
pub const PIPE_ACCEPT_REMOTE_CLIENTS: u32 = 0;
pub const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
pub const PIPE_UNLIMITED_INSTANCES: u32 = 255;
pub const PIPE_CLIENT_END: u32 = 0;
pub const PIPE_SERVER_END: u32 = 1;
pub const NMPWAIT_USE_DEFAULT_WAIT: u32 = 0;
pub const NMPWAIT_WAIT_FOREVER: u32 = 0xFFFF_FFFF;
pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
pub const OPEN_EXISTING: u32 = 3;
pub const CREATE_NEW: u32 = 1;
pub const OPEN_ALWAYS: u32 = 4;
pub const FILE_SKIP_COMPLETION_PORT_ON_SUCCESS: u8 = 1;
pub const FILE_SKIP_SET_EVENT_ON_HANDLE: u8 = 2;
pub const HANDLE_FLAG_INHERIT: u32 = 1;
pub const HANDLE_FLAG_PROTECT_FROM_CLOSE: u32 = 2;
pub const DUPLICATE_CLOSE_SOURCE: u32 = 1;
pub const DUPLICATE_SAME_ACCESS: u32 = 2;
pub const CREATE_SUSPENDED: u32 = 4;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
pub const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
pub const STARTF_USESTDHANDLES: u32 = 0x100;
pub const PROCESS_ALL_ACCESS: u32 = 0x1F_FFFF;
pub const PROCESS_DUP_HANDLE: u32 = 0x40;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const PROCESS_TERMINATE: u32 = 1;
pub const EVENT_ALL_ACCESS: u32 = 0x1F_0003;
pub const FILE_MAP_READ: u32 = 4;
pub const FILE_MAP_WRITE: u32 = 2;
pub const FILE_MAP_ALL_ACCESS: u32 = 0xF_001F;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OVERLAPPED {
    pub internal: usize,
    pub internal_high: usize,
    pub offset: u32,
    pub offset_high: u32,
    pub event: HANDLE,
}
impl OVERLAPPED {
    pub const fn new() -> OVERLAPPED {
        OVERLAPPED { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event: core::ptr::null_mut() }
    }
    pub fn with_event(e: HANDLE) -> OVERLAPPED {
        let mut o = OVERLAPPED::new();
        o.event = e;
        o
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OVERLAPPED_ENTRY {
    pub key: usize,
    pub overlapped: *mut OVERLAPPED,
    pub internal: usize,
    pub bytes: u32,
}
impl OVERLAPPED_ENTRY {
    pub const fn new() -> OVERLAPPED_ENTRY {
        OVERLAPPED_ENTRY { key: 0, overlapped: core::ptr::null_mut(), internal: 0, bytes: 0 }
    }
}

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub length: u32,
    pub descriptor: *mut c_void,
    pub inherit: BOOL,
}

#[repr(C)]
pub struct STARTUPINFOW {
    pub cb: u32,
    pub reserved: *mut u16,
    pub desktop: *mut u16,
    pub title: *mut u16,
    pub x: u32,
    pub y: u32,
    pub x_size: u32,
    pub y_size: u32,
    pub x_count_chars: u32,
    pub y_count_chars: u32,
    pub fill_attribute: u32,
    pub flags: u32,
    pub show_window: u16,
    pub cb_reserved2: u16,
    pub reserved2: *mut u8,
    pub std_input: HANDLE,
    pub std_output: HANDLE,
    pub std_error: HANDLE,
}
#[repr(C)]
pub struct STARTUPINFOEXW {
    pub si: STARTUPINFOW,
    pub attribute_list: *mut c_void,
}
#[repr(C)]
pub struct PROCESS_INFORMATION {
    pub process: HANDLE,
    pub thread: HANDLE,
    pub pid: u32,
    pub tid: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetLastError() -> u32;
    pub fn SetLastError(e: u32);
    pub fn CloseHandle(h: HANDLE) -> BOOL;
    pub fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buf: u32,
        in_buf: u32,
        default_timeout: u32,
        sa: *const SECURITY_ATTRIBUTES,
    ) -> HANDLE;
    pub fn ConnectNamedPipe(h: HANDLE, ov: *mut OVERLAPPED) -> BOOL;
    pub fn DisconnectNamedPipe(h: HANDLE) -> BOOL;
    pub fn WaitNamedPipeW(name: *const u16, timeout: u32) -> BOOL;
    pub fn PeekNamedPipe(
        h: HANDLE,
        buf: *mut c_void,
        size: u32,
        read: *mut u32,
        avail: *mut u32,
        left: *mut u32,
    ) -> BOOL;
    pub fn GetNamedPipeInfo(h: HANDLE, flags: *mut u32, out: *mut u32, inp: *mut u32, max: *mut u32) -> BOOL;
    pub fn GetNamedPipeClientProcessId(h: HANDLE, pid: *mut u32) -> BOOL;
    pub fn GetNamedPipeServerProcessId(h: HANDLE, pid: *mut u32) -> BOOL;
    pub fn GetNamedPipeClientSessionId(h: HANDLE, id: *mut u32) -> BOOL;
    pub fn GetNamedPipeServerSessionId(h: HANDLE, id: *mut u32) -> BOOL;
    pub fn GetNamedPipeHandleStateW(
        h: HANDLE,
        state: *mut u32,
        cur: *mut u32,
        max_coll: *mut u32,
        coll_timeout: *mut u32,
        user: *mut u16,
        user_max: u32,
    ) -> BOOL;
    pub fn SetNamedPipeHandleState(h: HANDLE, mode: *const u32, max_coll: *const u32, coll_timeout: *const u32)
    -> BOOL;
    pub fn TransactNamedPipe(
        h: HANDLE,
        inb: *const c_void,
        in_size: u32,
        outb: *mut c_void,
        out_size: u32,
        read: *mut u32,
        ov: *mut OVERLAPPED,
    ) -> BOOL;
    pub fn CallNamedPipeW(
        name: *const u16,
        inb: *const c_void,
        in_size: u32,
        outb: *mut c_void,
        out_size: u32,
        read: *mut u32,
        timeout: u32,
    ) -> BOOL;
    pub fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const SECURITY_ATTRIBUTES,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    pub fn ReadFile(h: HANDLE, buf: *mut c_void, n: u32, read: *mut u32, ov: *mut OVERLAPPED) -> BOOL;
    pub fn WriteFile(h: HANDLE, buf: *const c_void, n: u32, written: *mut u32, ov: *mut OVERLAPPED) -> BOOL;
    pub fn ReadFileEx(
        h: HANDLE,
        buf: *mut c_void,
        n: u32,
        ov: *mut OVERLAPPED,
        routine: unsafe extern "system" fn(u32, u32, *mut OVERLAPPED),
    ) -> BOOL;
    pub fn WriteFileEx(
        h: HANDLE,
        buf: *const c_void,
        n: u32,
        ov: *mut OVERLAPPED,
        routine: unsafe extern "system" fn(u32, u32, *mut OVERLAPPED),
    ) -> BOOL;
    pub fn FlushFileBuffers(h: HANDLE) -> BOOL;
    pub fn GetFileType(h: HANDLE) -> u32;
    pub fn CreateEventW(sa: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    pub fn SetEvent(h: HANDLE) -> BOOL;
    pub fn ResetEvent(h: HANDLE) -> BOOL;
    pub fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    pub fn WaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32) -> u32;
    pub fn SleepEx(ms: u32, alertable: BOOL) -> u32;
    pub fn Sleep(ms: u32);
    pub fn GetOverlappedResult(h: HANDLE, ov: *mut OVERLAPPED, n: *mut u32, wait: BOOL) -> BOOL;
    pub fn GetOverlappedResultEx(h: HANDLE, ov: *mut OVERLAPPED, n: *mut u32, ms: u32, alertable: BOOL) -> BOOL;
    pub fn CancelIo(h: HANDLE) -> BOOL;
    pub fn CancelIoEx(h: HANDLE, ov: *mut OVERLAPPED) -> BOOL;
    pub fn CancelSynchronousIo(thread: HANDLE) -> BOOL;
    pub fn CreateIoCompletionPort(file: HANDLE, port: HANDLE, key: usize, threads: u32) -> HANDLE;
    pub fn GetQueuedCompletionStatus(
        port: HANDLE,
        bytes: *mut u32,
        key: *mut usize,
        ov: *mut *mut OVERLAPPED,
        ms: u32,
    ) -> BOOL;
    pub fn GetQueuedCompletionStatusEx(
        port: HANDLE,
        entries: *mut OVERLAPPED_ENTRY,
        count: u32,
        removed: *mut u32,
        ms: u32,
        alertable: BOOL,
    ) -> BOOL;
    pub fn PostQueuedCompletionStatus(port: HANDLE, bytes: u32, key: usize, ov: *mut OVERLAPPED) -> BOOL;
    pub fn SetFileCompletionNotificationModes(h: HANDLE, flags: u8) -> BOOL;
    pub fn CreateThread(
        sa: *const c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    pub fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
    pub fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    pub fn GetCurrentProcessId() -> u32;
    pub fn GetCurrentProcess() -> HANDLE;
    pub fn GetCurrentThreadId() -> u32;
    pub fn GetTickCount64() -> u64;
    pub fn QueueUserAPC(f: unsafe extern "system" fn(usize), thread: HANDLE, data: usize) -> u32;
    pub fn GetCurrentThread() -> HANDLE;
    pub fn WaitForSingleObjectEx(h: HANDLE, ms: u32, alertable: BOOL) -> u32;
    pub fn OpenThread(access: u32, inherit: BOOL, tid: u32) -> HANDLE;
    pub fn TerminateThread(h: HANDLE, code: u32) -> BOOL;
    pub fn GetCommandLineW() -> *const u16;
    pub fn GetModuleFileNameW(module: HANDLE, name: *mut u16, size: u32) -> u32;
    pub fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *const SECURITY_ATTRIBUTES,
        ta: *const SECURITY_ATTRIBUTES,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *const STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    pub fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    pub fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    pub fn DuplicateHandle(
        src_proc: HANDLE,
        src: HANDLE,
        dst_proc: HANDLE,
        dst: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
    pub fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
    pub fn SetHandleInformation(h: HANDLE, mask: u32, flags: u32) -> BOOL;
    pub fn GetProcessId(h: HANDLE) -> u32;
    pub fn GetThreadId(h: HANDLE) -> u32;
    pub fn GetStdHandle(which: u32) -> HANDLE;
    pub fn CreateFileMappingA(
        file: HANDLE,
        sa: *const SECURITY_ATTRIBUTES,
        protect: u32,
        size_high: u32,
        size_low: u32,
        name: *const u8,
    ) -> HANDLE;
    pub fn GetFileSize(h: HANDLE, high: *mut u32) -> u32;
    pub fn VirtualQuery(addr: *const c_void, info: *mut c_void, len: usize) -> usize;
    pub fn CreateFileMappingW(
        file: HANDLE,
        sa: *const SECURITY_ATTRIBUTES,
        protect: u32,
        size_high: u32,
        size_low: u32,
        name: *const u16,
    ) -> HANDLE;
    pub fn MapViewOfFile(h: HANDLE, access: u32, off_high: u32, off_low: u32, bytes: usize) -> *mut c_void;
    pub fn UnmapViewOfFile(p: *const c_void) -> BOOL;
    pub fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> BOOL;
    pub fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous: *mut c_void,
        return_size: *mut usize,
    ) -> BOOL;
    pub fn DeleteProcThreadAttributeList(list: *mut c_void);
    pub fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, protect: u32) -> *mut c_void;
    pub fn ResumeThread(h: HANDLE) -> u32;
    pub fn OpenJobObjectW(access: u32, inherit: BOOL, name: *const u16) -> HANDLE;
    pub fn CreateJobObjectW(sa: *const SECURITY_ATTRIBUTES, name: *const u16) -> HANDLE;
    pub fn AssignProcessToJobObject(job: HANDLE, process: HANDLE) -> BOOL;
    pub fn IsProcessInJob(process: HANDLE, job: HANDLE, result: *mut BOOL) -> BOOL;
    pub fn QueryInformationJobObject(job: HANDLE, class: u32, info: *mut c_void, size: u32, ret: *mut u32) -> BOOL;
    pub fn SetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, size: u32) -> BOOL;
}

// ---------------------------------------------------------------- helpers

/// A NUL-terminated UTF-16 string built from ASCII pieces and hex numbers.
#[derive(Clone, Copy)]
pub struct W16 {
    pub buf: [u16; 160],
    pub len: usize,
}
impl W16 {
    pub const fn new() -> W16 {
        W16 { buf: [0; 160], len: 0 }
    }
    pub fn s(mut self, s: &str) -> W16 {
        for b in s.bytes() {
            self.buf[self.len] = b as u16;
            self.len += 1;
        }
        self
    }
    pub fn hex(mut self, v: u32) -> W16 {
        const H: &[u8; 16] = b"0123456789abcdef";
        let mut i = 8;
        while i > 0 {
            i -= 1;
            self.buf[self.len] = H[((v >> (4 * i)) & 0xf) as usize] as u16;
            self.len += 1;
        }
        self
    }
    pub fn ptr(&self) -> *const u16 {
        self.buf.as_ptr()
    }
}

/// `\\.\pipe\poly-ipc-<tag>-<pid>`: unique per test and run.
pub fn pipe_name(tag: &str) -> W16 {
    // SAFETY: no pointers involved.
    let pid = unsafe { GetCurrentProcessId() };
    W16::new().s("\\\\.\\pipe\\poly-ipc-").s(tag).s("-").hex(pid)
}
/// Same, with a suffix distinguishing a second pipe of the test.
pub fn pipe_name2(tag: &str, n: &str) -> W16 {
    // SAFETY: no pointers involved.
    let pid = unsafe { GetCurrentProcessId() };
    W16::new().s("\\\\.\\pipe\\poly-ipc-").s(tag).s(n).s("-").hex(pid)
}

pub fn le() -> u32 {
    // SAFETY: GetLastError has no preconditions.
    unsafe { GetLastError() }
}
pub fn clear_err() {
    // SAFETY: SetLastError has no preconditions.
    unsafe { SetLastError(0) }
}
pub fn is_invalid(h: HANDLE) -> bool {
    h == INVALID_HANDLE_VALUE || h.is_null()
}
pub fn close(h: HANDLE) -> bool {
    // SAFETY: closing a handle value we own; an invalid value only fails.
    unsafe { CloseHandle(h) != 0 }
}

pub fn create_pipe(name: &W16, open: u32, mode: u32, max: u32, out: u32, inp: u32, timeout: u32) -> HANDLE {
    clear_err();
    // SAFETY: `name` is NUL-terminated and outlives the call; no security attributes.
    unsafe { CreateNamedPipeW(name.ptr(), open, mode, max, out, inp, timeout, core::ptr::null()) }
}
pub fn open_pipe(name: &W16, access: u32, flags: u32) -> HANDLE {
    clear_err();
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe { CreateFileW(name.ptr(), access, 0, core::ptr::null(), OPEN_EXISTING, flags, core::ptr::null_mut()) }
}
pub fn open_pipe_share(name: &W16, access: u32, share: u32, flags: u32) -> HANDLE {
    clear_err();
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe { CreateFileW(name.ptr(), access, share, core::ptr::null(), OPEN_EXISTING, flags, core::ptr::null_mut()) }
}

/// Synchronous read: (ok, bytes, last error).
pub fn rd(h: HANDLE, buf: &mut [u8]) -> (bool, u32, u32) {
    let mut n = 0u32;
    clear_err();
    // SAFETY: `buf` and `n` are live for the call; no OVERLAPPED.
    let ok = unsafe { ReadFile(h, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n, core::ptr::null_mut()) } != 0;
    (ok, n, le())
}
pub fn wr(h: HANDLE, data: &[u8]) -> (bool, u32, u32) {
    let mut n = 0u32;
    clear_err();
    // SAFETY: `data` and `n` are live for the call; no OVERLAPPED.
    let ok = unsafe { WriteFile(h, data.as_ptr().cast(), data.len() as u32, &mut n, core::ptr::null_mut()) } != 0;
    (ok, n, le())
}
/// Overlapped read: (ok, bytes-as-reported-by-call, last error).
pub fn rd_ov(h: HANDLE, buf: &mut [u8], ov: &mut OVERLAPPED) -> (bool, u32, u32) {
    let mut n = 0u32;
    clear_err();
    // SAFETY: `buf`, `n` and `ov` outlive the call; the caller keeps `buf`/`ov` alive until completion.
    let ok = unsafe { ReadFile(h, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n, ov) } != 0;
    (ok, n, le())
}
pub fn wr_ov(h: HANDLE, data: &[u8], ov: &mut OVERLAPPED) -> (bool, u32, u32) {
    let mut n = 0u32;
    clear_err();
    // SAFETY: as in `rd_ov`.
    let ok = unsafe { WriteFile(h, data.as_ptr().cast(), data.len() as u32, &mut n, ov) } != 0;
    (ok, n, le())
}
pub fn connect(h: HANDLE, ov: Option<&mut OVERLAPPED>) -> (bool, u32) {
    clear_err();
    let p = match ov {
        Some(o) => o as *mut OVERLAPPED,
        None => core::ptr::null_mut(),
    };
    // SAFETY: `h` is a pipe handle; `p` is null or a live OVERLAPPED.
    let ok = unsafe { ConnectNamedPipe(h, p) } != 0;
    (ok, le())
}
pub fn event(manual: bool, initial: bool) -> HANDLE {
    // SAFETY: unnamed event, no security attributes.
    unsafe { CreateEventW(core::ptr::null(), manual as BOOL, initial as BOOL, core::ptr::null()) }
}
pub fn wait(h: HANDLE, ms: u32) -> u32 {
    // SAFETY: waiting on a handle value.
    unsafe { WaitForSingleObject(h, ms) }
}
/// (ok, bytes copied, bytes available, bytes left in message, last error)
pub fn peek(h: HANDLE, buf: &mut [u8]) -> (bool, u32, u32, u32, u32) {
    let (mut r, mut a, mut l) = (0u32, 0u32, 0u32);
    clear_err();
    // SAFETY: all out pointers are live locals / the buffer.
    let ok = unsafe { PeekNamedPipe(h, buf.as_mut_ptr().cast(), buf.len() as u32, &mut r, &mut a, &mut l) } != 0;
    (ok, r, a, l, le())
}
/// (ok, available, left in message, last error) without a data buffer.
pub fn peek_sizes(h: HANDLE) -> (bool, u32, u32, u32) {
    let (mut a, mut l) = (0u32, 0u32);
    clear_err();
    // SAFETY: no data buffer requested; out pointers are live.
    let ok = unsafe { PeekNamedPipe(h, core::ptr::null_mut(), 0, core::ptr::null_mut(), &mut a, &mut l) } != 0;
    (ok, a, l, le())
}
pub fn set_state(h: HANDLE, mode: Option<u32>) -> (bool, u32) {
    clear_err();
    let m = mode.unwrap_or(0);
    let p: *const u32 = if mode.is_some() { &m } else { core::ptr::null() };
    // SAFETY: `p` is null or points at `m`.
    let ok = unsafe { SetNamedPipeHandleState(h, p, core::ptr::null(), core::ptr::null()) } != 0;
    (ok, le())
}
/// (ok, state, instances, last error)
pub fn handle_state(h: HANDLE) -> (bool, u32, u32, u32) {
    let (mut s, mut c) = (0u32, 0u32);
    clear_err();
    // SAFETY: out pointers are live locals; the optional ones are null.
    let ok = unsafe {
        GetNamedPipeHandleStateW(
            h,
            &mut s,
            &mut c,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
        )
    } != 0;
    (ok, s, c, le())
}
/// (ok, flags, out, in, max, last error)
pub fn pipe_info(h: HANDLE) -> (bool, u32, u32, u32, u32, u32) {
    let (mut f, mut o, mut i, mut m) = (0u32, 0u32, 0u32, 0u32);
    clear_err();
    // SAFETY: out pointers are live locals.
    let ok = unsafe { GetNamedPipeInfo(h, &mut f, &mut o, &mut i, &mut m) } != 0;
    (ok, f, o, i, m, le())
}
/// (ok, bytes, last error) of GetOverlappedResult.
pub fn ov_result(h: HANDLE, ov: &mut OVERLAPPED, wait_: bool) -> (bool, u32, u32) {
    let mut n = 0u32;
    clear_err();
    // SAFETY: `ov` and `n` are live.
    let ok = unsafe { GetOverlappedResult(h, ov, &mut n, wait_ as BOOL) } != 0;
    (ok, n, le())
}
/// (ok, bytes, key, overlapped pointer value (0 or not), last error) of GetQueuedCompletionStatus.
pub fn gqcs(port: HANDLE, ms: u32) -> (bool, u32, usize, *mut OVERLAPPED, u32) {
    let mut n = 0u32;
    let mut key = 0usize;
    let mut ovp: *mut OVERLAPPED = core::ptr::null_mut();
    clear_err();
    // SAFETY: out pointers are live locals.
    let ok = unsafe { GetQueuedCompletionStatus(port, &mut n, &mut key, &mut ovp, ms) } != 0;
    (ok, n, key, ovp, le())
}
pub fn iocp_create(threads: u32) -> HANDLE {
    // SAFETY: creating a new port needs INVALID_HANDLE_VALUE and no existing port.
    unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, core::ptr::null_mut(), 0, threads) }
}
pub fn iocp_assoc(file: HANDLE, port: HANDLE, key: usize) -> HANDLE {
    clear_err();
    // SAFETY: plain handle values.
    unsafe { CreateIoCompletionPort(file, port, key, 0) }
}
pub fn post(port: HANDLE, bytes: u32, key: usize, ov: *mut OVERLAPPED) -> (bool, u32) {
    clear_err();
    // SAFETY: the packet's OVERLAPPED pointer is only stored, never dereferenced by the call.
    let ok = unsafe { PostQueuedCompletionStatus(port, bytes, key, ov) } != 0;
    (ok, le())
}

/// A connected pair: server instance made with `open`/`mode` (4 instances, 4096-byte buffers), the
/// client opened with `access`/`flags`, and the server's connect done (already-connected is fine).
pub fn pair(name: &W16, open: u32, mode: u32, access: u32, flags: u32) -> (HANDLE, HANDLE) {
    let s = create_pipe(name, open, mode, 4, 4096, 4096, 0);
    let c = open_pipe(name, access, flags);
    if open & FILE_FLAG_OVERLAPPED != 0 {
        let ev = event(true, false);
        let mut ov = OVERLAPPED::with_event(ev);
        connect(s, Some(&mut ov));
        close(ev);
    } else {
        connect(s, None);
    }
    (s, c)
}
pub const RW: u32 = 0xC000_0000; // GENERIC_READ | GENERIC_WRITE

/// Read that never blocks forever: polls PeekNamedPipe for up to `ms` for data, then reads.
/// (false, 0, ERROR_SEM_TIMEOUT) when nothing arrived.
pub fn rd_poll(h: HANDLE, buf: &mut [u8], ms: u32) -> (bool, u32, u32) {
    let mut waited = 0;
    loop {
        let (ok, a, _, e) = peek_sizes(h);
        if !ok {
            return (false, 0, e);
        }
        if a > 0 {
            return rd(h, buf);
        }
        if waited >= ms {
            return (false, 0, ERROR_SEM_TIMEOUT);
        }
        sleep(5);
        waited += 5;
    }
}
pub fn tick() -> u64 {
    // SAFETY: no arguments.
    unsafe { GetTickCount64() }
}

pub struct Job {
    pub f: fn(usize) -> u32,
    pub arg: usize,
}
unsafe extern "system" fn tramp(p: *mut c_void) -> u32 {
    // SAFETY: `p` is the `Job` the spawner keeps alive until the thread is joined.
    let j = unsafe { &*(p as *const Job) };
    (j.f)(j.arg)
}
/// Start a thread running `job`; the caller keeps `job` alive until `join`.
pub fn spawn(job: &Job) -> HANDLE {
    // SAFETY: `job` outlives the thread by the contract above.
    unsafe { CreateThread(core::ptr::null(), 0, tramp, job as *const Job as *mut c_void, 0, core::ptr::null_mut()) }
}
pub fn join(h: HANDLE) -> u32 {
    let mut code = 0u32;
    // SAFETY: waiting for and reading the exit code of a thread handle we own.
    unsafe {
        WaitForSingleObject(h, INFINITE);
        GetExitCodeThread(h, &mut code);
        CloseHandle(h);
    }
    code
}
pub fn sleep(ms: u32) {
    // SAFETY: plain sleep.
    unsafe { Sleep(ms) }
}

/// Stage numbers shared between a test's threads.
pub static STAGE: AtomicU32 = AtomicU32::new(0);
pub static SHARED: AtomicUsize = AtomicUsize::new(0);
pub fn wait_stage(n: u32) -> bool {
    let mut i = 0;
    while STAGE.load(Ordering::SeqCst) < n {
        sleep(2);
        i += 1;
        if i > 5000 {
            return false;
        }
    }
    true
}

// ----------------------------------------------------- child processes

pub type Args = ([u8; 160], usize);

/// The ASCII arguments after `--child` on this process's command line.
pub fn child_args() -> Option<Args> {
    // SAFETY: GetCommandLineW returns a NUL-terminated string valid for the process lifetime.
    let p = unsafe { GetCommandLineW() };
    let mut units = [0u16; 400];
    let mut n = 0;
    // SAFETY: reads up to the terminating NUL (bounded by `units`).
    unsafe {
        while n < units.len() - 1 && *p.add(n) != 0 {
            units[n] = *p.add(n);
            n += 1;
        }
    }
    let key = b" --child ";
    let mut i = 0;
    while i + key.len() <= n {
        let mut ok = true;
        let mut j = 0;
        while j < key.len() {
            if units[i + j] != key[j] as u16 {
                ok = false;
                break;
            }
            j += 1;
        }
        if ok {
            let mut out = [0u8; 160];
            let mut m = 0;
            let mut k = i + key.len();
            while k < n && m < out.len() {
                out[m] = units[k] as u8;
                m += 1;
                k += 1;
            }
            return Some((out, m));
        }
        i += 1;
    }
    None
}

pub struct CmdLine {
    pub buf: [u16; 400],
}
pub fn exe_path() -> ([u16; 400], usize) {
    let mut b = [0u16; 400];
    // SAFETY: buffer of 400 units; null module = the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), b.as_mut_ptr(), 399) } as usize;
    (b, n)
}
/// `"<exe>" --child <args>`, mutable and NUL-terminated for CreateProcessW.
pub fn child_cmdline(args: &str) -> CmdLine {
    let (exe, n) = exe_path();
    let mut c = CmdLine { buf: [0; 400] };
    let mut m = 0;
    c.buf[m] = b'"' as u16;
    m += 1;
    for &u in &exe[..n] {
        c.buf[m] = u;
        m += 1;
    }
    c.buf[m] = b'"' as u16;
    m += 1;
    for b in " --child ".bytes().chain(args.bytes()) {
        c.buf[m] = b as u16;
        m += 1;
    }
    c
}
pub fn new_startupinfo() -> STARTUPINFOW {
    STARTUPINFOW {
        cb: core::mem::size_of::<STARTUPINFOW>() as u32,
        reserved: core::ptr::null_mut(),
        desktop: core::ptr::null_mut(),
        title: core::ptr::null_mut(),
        x: 0,
        y: 0,
        x_size: 0,
        y_size: 0,
        x_count_chars: 0,
        y_count_chars: 0,
        fill_attribute: 0,
        flags: 0,
        show_window: 0,
        cb_reserved2: 0,
        reserved2: core::ptr::null_mut(),
        std_input: core::ptr::null_mut(),
        std_output: core::ptr::null_mut(),
        std_error: core::ptr::null_mut(),
    }
}
/// Start the test's own executable as a child with `args`: (process, thread, pid), or None.
/// `startup` is a STARTUPINFOW / STARTUPINFOEXW pointer (null = default).
pub fn start_child_raw(
    args: &str,
    inherit: bool,
    flags: u32,
    startup: *const STARTUPINFOW,
) -> Option<(HANDLE, HANDLE, u32)> {
    let mut cmd = child_cmdline(args);
    let (exe, _) = exe_path();
    let mut pi =
        PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    let default_si = new_startupinfo();
    let sip = if startup.is_null() { &default_si as *const STARTUPINFOW } else { startup };
    clear_err();
    // SAFETY: `exe`/`cmd` are NUL-terminated buffers that live through the call; `sip` and `pi` are live.
    let ok = unsafe {
        CreateProcessW(
            exe.as_ptr(),
            cmd.buf.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            inherit as BOOL,
            flags,
            core::ptr::null(),
            core::ptr::null(),
            sip,
            &mut pi,
        )
    } != 0;
    if ok { Some((pi.process, pi.thread, pi.pid)) } else { None }
}
pub fn start_child(args: &str, inherit: bool) -> Option<(HANDLE, HANDLE, u32)> {
    start_child_raw(args, inherit, 0, core::ptr::null())
}
/// Wait (up to `ms`) for a process and return its exit code (0xFFFF_FFFE on timeout).
pub fn wait_exit(p: HANDLE, ms: u32) -> u32 {
    let mut code = 0u32;
    // SAFETY: waiting on and reading a process handle we own.
    unsafe {
        if WaitForSingleObject(p, ms) != WAIT_OBJECT_0 {
            return 0xFFFF_FFFE;
        }
        GetExitCodeProcess(p, &mut code);
    }
    code
}
pub fn arg_is(a: &Args, s: &str) -> bool {
    &a.0[..a.1] == s.as_bytes()
}
pub fn arg_starts(a: &Args, s: &str) -> bool {
    a.0[..a.1].starts_with(s.as_bytes())
}
/// A hex number (lowercase digits) at `off` in the child arguments, and where it ends.
pub fn arg_hex(a: &Args, off: usize) -> usize {
    let mut v = 0usize;
    let mut i = off;
    while i < a.1 {
        let c = a.0[i];
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => break,
        };
        v = v * 16 + d as usize;
        i += 1;
    }
    v
}
/// Append `h` as 16 hex digits to `s` (child arguments carry handle values this way).
pub fn hex16(mut s: [u8; 100], mut len: usize, v: usize) -> ([u8; 100], usize) {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut i = 16;
    while i > 0 {
        i -= 1;
        s[len] = H[(v >> (4 * i)) & 0xf];
        len += 1;
    }
    (s, len)
}
/// `"<word> <hex handle>"` as a command argument string.
pub fn args_with_handle(word: &str, v: usize) -> ([u8; 100], usize) {
    let mut s = [0u8; 100];
    let mut len = 0;
    for b in word.bytes() {
        s[len] = b;
        len += 1;
    }
    s[len] = b' ';
    len += 1;
    hex16(s, len, v)
}
pub fn as_str(a: &([u8; 100], usize)) -> &str {
    core::str::from_utf8(&a.0[..a.1]).unwrap_or("")
}


// ------------------------------------------------------------ ntdll

pub const STATUS_ALERTED: i32 = 0x101;
pub const STATUS_USER_APC: i32 = 0xC0;
pub const STATUS_CANCELLED: i32 = 0xC000_0120_u32 as i32;
pub const STATUS_NOT_FOUND: i32 = 0xC000_0225_u32 as i32;
pub const STATUS_NOT_MAPPED_VIEW: i32 = 0xC000_0019_u32 as i32;
pub const STATUS_INVALID_VIEW_SIZE: i32 = 0xC000_001F_u32 as i32;
pub const STATUS_MAPPED_ALIGNMENT: i32 = 0xC000_0220_u32 as i32;
pub const STATUS_SECTION_PROTECTION: i32 = 0xC000_004E_u32 as i32;
pub const STATUS_OBJECT_TYPE_MISMATCH: i32 = 0xC000_0024_u32 as i32;
pub const STATUS_INVALID_INFO_CLASS: i32 = 0xC000_0003_u32 as i32;
pub const STATUS_SECTION_NOT_IMAGE: i32 = 0xC000_0256_u32 as i32;
pub const STATUS_INSTANCE_NOT_AVAILABLE: i32 = 0xC000_00AB_u32 as i32;
pub const STATUS_PIPE_NOT_AVAILABLE: i32 = 0xC000_00AC_u32 as i32;
pub const STATUS_INVALID_PIPE_STATE: i32 = 0xC000_00AD_u32 as i32;
pub const STATUS_PIPE_BUSY: i32 = 0xC000_00AE_u32 as i32;
pub const STATUS_PIPE_DISCONNECTED: i32 = 0xC000_00B0_u32 as i32;
pub const STATUS_PIPE_CLOSING: i32 = 0xC000_00B1_u32 as i32;
pub const STATUS_PIPE_CONNECTED: i32 = 0xC000_00B2_u32 as i32;
pub const STATUS_PIPE_LISTENING: i32 = 0xC000_00B3_u32 as i32;
pub const STATUS_PIPE_BROKEN: i32 = 0xC000_014B_u32 as i32;
pub const STATUS_OBJECT_NAME_EXISTS: i32 = 0x4000_0000;
pub const IO_COMPLETION_ALL_ACCESS: u32 = 0x1F_0003;
pub const SECTION_ALL_ACCESS: u32 = 0xF_001F;
pub const SEC_COMMIT: u32 = 0x0800_0000;
pub const SEC_RESERVE: u32 = 0x0400_0000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileIoCompletionInformation {
    pub key: usize,
    pub apc: usize,
    pub status: i32,
    pub pad: i32,
    pub information: usize,
}
impl FileIoCompletionInformation {
    pub const fn new() -> FileIoCompletionInformation {
        FileIoCompletionInformation { key: 0, apc: 0, status: 0, pad: 0, information: 0 }
    }
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn NtCreateIoCompletion(h: *mut HANDLE, access: u32, attrs: *const ObjectAttributes, threads: u32) -> NTSTATUS;
    pub fn NtSetIoCompletion(h: HANDLE, key: usize, apc: usize, status: NTSTATUS, info: usize) -> NTSTATUS;
    pub fn NtRemoveIoCompletion(h: HANDLE, key: *mut usize, apc: *mut usize, iosb: *mut IoStatusBlock, timeout: *const i64) -> NTSTATUS;
    pub fn NtRemoveIoCompletionEx(
        h: HANDLE,
        info: *mut FileIoCompletionInformation,
        count: u32,
        removed: *mut u32,
        timeout: *const i64,
        alertable: u8,
    ) -> NTSTATUS;
    pub fn NtQueryIoCompletion(h: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> NTSTATUS;
    pub fn NtCancelIoFile(h: HANDLE, iosb: *mut IoStatusBlock) -> NTSTATUS;
    pub fn NtCancelIoFileEx(h: HANDLE, to_cancel: *mut IoStatusBlock, iosb: *mut IoStatusBlock) -> NTSTATUS;
    pub fn NtCreateNamedPipeFile(
        h: *mut HANDLE,
        access: u32,
        attrs: *const ObjectAttributes,
        iosb: *mut IoStatusBlock,
        share: u32,
        disposition: u32,
        options: u32,
        pipe_type: u32,
        read_mode: u32,
        completion_mode: u32,
        max_instances: u32,
        inbound_quota: u32,
        outbound_quota: u32,
        default_timeout: *const i64,
    ) -> NTSTATUS;
    pub fn NtFsControlFile(
        h: HANDLE,
        event: HANDLE,
        apc: *const c_void,
        apc_context: *const c_void,
        iosb: *mut IoStatusBlock,
        code: u32,
        input: *const c_void,
        input_len: u32,
        output: *mut c_void,
        output_len: u32,
    ) -> NTSTATUS;
    pub fn NtDuplicateObject(
        src_process: HANDLE,
        src: HANDLE,
        dst_process: HANDLE,
        dst: *mut HANDLE,
        access: u32,
        attributes: u32,
        options: u32,
    ) -> NTSTATUS;
    pub fn NtCreateSection(
        h: *mut HANDLE,
        access: u32,
        attrs: *const ObjectAttributes,
        max_size: *const i64,
        protection: u32,
        allocation: u32,
        file: HANDLE,
    ) -> NTSTATUS;
    pub fn NtMapViewOfSection(
        section: HANDLE,
        process: HANDLE,
        base: *mut *mut c_void,
        zero_bits: usize,
        commit: usize,
        offset: *mut i64,
        view_size: *mut usize,
        inherit: u32,
        allocation_type: u32,
        protect: u32,
    ) -> NTSTATUS;
    pub fn NtUnmapViewOfSection(process: HANDLE, base: *mut c_void) -> NTSTATUS;
    pub fn NtQuerySection(h: HANDLE, class: u32, info: *mut c_void, len: usize, ret: *mut usize) -> NTSTATUS;
    pub fn NtQueryObject(h: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> NTSTATUS;
    pub fn NtWaitForSingleObject(h: HANDLE, alertable: u8, timeout: *const i64) -> NTSTATUS;
    pub fn NtCreateEvent(h: *mut HANDLE, access: u32, attrs: *const ObjectAttributes, ty: u32, signalled: u8) -> NTSTATUS;
}

pub const CUR_PROC: HANDLE = -1isize as HANDLE;
/// A relative timeout in milliseconds as the 100 ns negative value NT wants.
pub fn rel_ms(ms: i64) -> i64 {
    -ms * 10_000
}
/// OBJECT_BASIC_INFORMATION (winternl.h PUBLIC_OBJECT_BASIC_INFORMATION): (granted access, handle count, pointer count)
pub fn obj_basic(h: HANDLE) -> Option<(u32, u32, u32)> {
    let mut b = [0u32; 14];
    let mut ret = 0u32;
    // SAFETY: `b` is a 56-byte buffer, `ret` a live out-parameter.
    let st = unsafe { NtQueryObject(h, 0, b.as_mut_ptr().cast(), 56, &mut ret) };
    if st == 0 { Some((b[1], b[2], b[3])) } else { None }
}
/// NtDuplicateObject within this process: (status, new handle)
pub fn nt_dup(src: HANDLE, access: u32, attributes: u32, options: u32) -> (NTSTATUS, HANDLE) {
    let mut d: HANDLE = core::ptr::null_mut();
    // SAFETY: handle values and a live out-parameter.
    let st = unsafe { NtDuplicateObject(CUR_PROC, src, CUR_PROC, &mut d, access, attributes, options) };
    (st, d)
}

/// An anonymous (page file backed) section: (status, handle)
pub fn nt_section(size: i64, protection: u32, alloc: u32) -> (NTSTATUS, HANDLE) {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: live out-parameter and size.
    let st = unsafe { NtCreateSection(&mut h, SECTION_ALL_ACCESS, core::ptr::null(), &size, protection, alloc, core::ptr::null_mut()) };
    (st, h)
}
/// Map a section into `process`: (status, base address, view size)
pub fn nt_map(section: HANDLE, process: HANDLE, offset: i64, size: usize, protect: u32) -> (NTSTATUS, *mut c_void, usize) {
    let mut base: *mut c_void = core::ptr::null_mut();
    let mut vs = size;
    let mut off = offset;
    // SAFETY: live out-parameters; ViewShare (1).
    let st = unsafe { NtMapViewOfSection(section, process, &mut base, 0, 0, &mut off, &mut vs, 1, 0, protect) };
    (st, base, vs)
}
pub fn nt_unmap(base: *mut c_void) -> NTSTATUS {
    // SAFETY: base is a value the test got from a map call (or deliberately not).
    unsafe { NtUnmapViewOfSection(CUR_PROC, base) }
}
/// VirtualQuery: (State, Protect, Type, RegionSize, AllocationBase == addr rounded?), or None
pub fn vq(addr: *const c_void) -> Option<(u32, u32, u32, usize)> {
    let mut b = [0u64; 6];
    // SAFETY: 48-byte MEMORY_BASIC_INFORMATION buffer.
    let n = unsafe { VirtualQuery(addr, b.as_mut_ptr().cast(), 48) };
    if n == 0 {
        return None;
    }
    // layout: BaseAddress, AllocationBase, AllocationProtect(u32)+pad, RegionSize, State(u32), Protect(u32), Type(u32)
    let state = (b[4] & 0xFFFF_FFFF) as u32;
    let protect = (b[4] >> 32) as u32;
    let ty = (b[5] & 0xFFFF_FFFF) as u32;
    Some((state, protect, ty, b[3] as usize))
}
pub const PAGE_READONLY_: u32 = 0x02;
pub const PAGE_READWRITE_: u32 = 0x04;
pub const MEM_FREE: u32 = 0x10000;
pub const MEM_MAPPED: u32 = 0x40000;
pub const FILE_MAP_COPY: u32 = 1;
pub const SECTION_QUERY: u32 = 1;
pub const SECTION_MAP_WRITE: u32 = 2;
pub const SECTION_MAP_READ: u32 = 4;
// ----------------------------------------- parent/child message link

/// The parent's end: a byte pipe `poly-ipc-<tag>-<parent pid>` (overlapped server handle).
pub fn link_server(tag: &str) -> HANDLE {
    // SAFETY: no arguments.
    let me = unsafe { GetCurrentProcessId() };
    let name = W16::new().s("\\\\.\\pipe\\poly-ipc-").s(tag).s("-").hex(me);
    create_pipe(&name, PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE, 1, 4096, 4096, 0)
}
/// Wait up to `ms` for the child to connect.
pub fn link_accept(s: HANDLE, ms: u32) -> bool {
    let ev = event(true, false);
    let mut ov = OVERLAPPED::with_event(ev);
    let (ok, e) = connect(s, Some(&mut ov));
    let r = if ok || e == ERROR_PIPE_CONNECTED {
        true
    } else if e == ERROR_IO_PENDING {
        wait(ev, ms) == WAIT_OBJECT_0
    } else {
        false
    };
    close(ev);
    r
}
/// The child's end: open the parent's pipe (the parent's pid comes from the arguments).
pub fn link_client(tag: &str, parent: u32) -> HANDLE {
    let name = W16::new().s("\\\\.\\pipe\\poly-ipc-").s(tag).s("-").hex(parent);
    let mut i = 0;
    loop {
        let h = open_pipe(&name, RW, 0);
        if !is_invalid(h) || i > 200 {
            return h;
        }
        // SAFETY: `name` is NUL-terminated.
        unsafe { WaitNamedPipeW(name.ptr(), 20) };
        sleep(10);
        i += 1;
    }
}
pub fn send_bytes(h: HANDLE, b: &[u8]) -> bool {
    let (ok, k, _) = wr(h, b);
    ok && k as usize == b.len()
}
pub fn send_u64(h: HANDLE, v: u64) -> bool {
    send_bytes(h, &v.to_le_bytes())
}
pub fn recv_exact(h: HANDLE, out: &mut [u8], ms: u32) -> bool {
    let mut got = 0;
    while got < out.len() {
        let (ok, k, _) = rd_poll(h, &mut out[got..], ms);
        if !ok || k == 0 {
            return false;
        }
        got += k as usize;
    }
    true
}
pub fn recv_u64(h: HANDLE, ms: u32) -> Option<u64> {
    let mut b = [0u8; 8];
    if recv_exact(h, &mut b, ms) { Some(u64::from_le_bytes(b)) } else { None }
}
pub fn recv_byte(h: HANDLE, ms: u32) -> Option<u8> {
    let mut b = [0u8; 1];
    if recv_exact(h, &mut b, ms) { Some(b[0]) } else { None }
}
/// The command-line argument "<word> <pid hex 8>" for a child that talks back over a link.
pub fn pid_args(word: &str) -> ([u8; 40], usize) {
    // SAFETY: no arguments.
    let me = unsafe { GetCurrentProcessId() };
    let mut a = [0u8; 40];
    a[..word.len()].copy_from_slice(word.as_bytes());
    let mut len = word.len();
    a[len] = b' ';
    len += 1;
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut i = 8;
    while i > 0 {
        i -= 1;
        a[len] = H[((me >> (4 * i)) & 0xf) as usize];
        len += 1;
    }
    (a, len)
}
/// The parent's pid in "<word> <pid hex 8>".
pub fn parent_pid(a: &Args) -> u32 {
    let mut i = 0;
    while i < a.1 && a.0[i] != b' ' {
        i += 1;
    }
    arg_hex(a, i + 1) as u32
}

// ---------------------------------------------------- attribute lists, jobs

pub const PROC_THREAD_ATTRIBUTE_PARENT_PROCESS: usize = 0x0002_0000;
pub const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
pub const PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY: usize = 0x0002_0007;
pub const PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES: usize = 0x0002_0009;
pub const PROC_THREAD_ATTRIBUTE_JOB_LIST: usize = 0x0002_000D;
pub const PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY: usize = 0x0002_000E;
pub const ERROR_OBJECT_NAME_EXISTS: u32 = 698;
pub const ERROR_BAD_LENGTH: u32 = 24;
pub const ERROR_NOT_ENOUGH_QUOTA: u32 = 1816;
pub const ERROR_DYNAMIC_CODE_BLOCKED: u32 = 1655;

#[repr(C, align(16))]
pub struct AttrBuf(pub [u8; 1024]);
impl AttrBuf {
    pub const fn new() -> AttrBuf {
        AttrBuf([0; 1024])
    }
}
/// Size InitializeProcThreadAttributeList wants for `count` attributes (0 if it does not say).
pub fn attr_list_size(count: u32) -> usize {
    let mut size = 0usize;
    clear_err();
    // SAFETY: a NULL list asks for the size; `size` is a live out-parameter.
    unsafe { InitializeProcThreadAttributeList(core::ptr::null_mut(), count, 0, &mut size) };
    size
}
/// Initialise `buf` as an attribute list for `count` attributes; the list pointer or null.
pub fn init_list(buf: &mut AttrBuf, count: u32) -> *mut c_void {
    let mut size = buf.0.len();
    clear_err();
    // SAFETY: `buf` is a 1024-byte aligned buffer, `size` its length.
    let ok = unsafe { InitializeProcThreadAttributeList(buf.0.as_mut_ptr().cast(), count, 0, &mut size) } != 0;
    if ok { buf.0.as_mut_ptr().cast() } else { core::ptr::null_mut() }
}
/// UpdateProcThreadAttribute with no flags and no previous value: (ok, last error).
pub fn update_attr(list: *mut c_void, attribute: usize, value: *const c_void, size: usize) -> (bool, u32) {
    clear_err();
    // SAFETY: `list` is an initialised list (or the test's deliberate misuse); `value` points at `size` bytes
    // that outlive the list's use.
    let ok = unsafe {
        UpdateProcThreadAttribute(list, 0, attribute, value, size, core::ptr::null_mut(), core::ptr::null_mut())
    } != 0;
    (ok, le())
}
/// Start the test image as a child with an attribute list (or none) and `bInheritHandles`.
pub fn launch_ex(args: &str, inherit: bool, list: *mut c_void) -> Option<(HANDLE, HANDLE, u32)> {
    let mut si = STARTUPINFOEXW { si: new_startupinfo(), attribute_list: list };
    si.si.cb = core::mem::size_of::<STARTUPINFOEXW>() as u32;
    start_child_raw(args, inherit, EXTENDED_STARTUPINFO_PRESENT, &si.si)
}
pub fn inheritable_event(signalled: bool) -> HANDLE {
    let sa = SECURITY_ATTRIBUTES {
        length: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        descriptor: core::ptr::null_mut(),
        inherit: 1,
    };
    // SAFETY: `sa` outlives the call.
    unsafe { CreateEventW(&sa, 1, signalled as BOOL, core::ptr::null()) }
}
pub fn handle_flags(h: HANDLE) -> (bool, u32, u32) {
    let mut f = 0xAAAAu32;
    clear_err();
    // SAFETY: `f` is a live out-parameter.
    let ok = unsafe { GetHandleInformation(h, &mut f) } != 0;
    (ok, f, le())
}
/// "<word> <h1> <h2> ..." command-line arguments from a word and handle values (16 hex digits each).
pub fn handle_args(word: &str, hs: &[usize]) -> ([u8; 100], usize) {
    let mut s = [0u8; 100];
    let mut len = 0;
    for b in word.bytes() {
        s[len] = b;
        len += 1;
    }
    for &h in hs {
        s[len] = b' ';
        len += 1;
        let (s2, l2) = hex16(s, len, h);
        s = s2;
        len = l2;
    }
    (s, len)
}
/// The n-th (0-based) space-separated hex number after the first word of the child arguments.
pub fn arg_handle(a: &Args, n: usize) -> usize {
    let mut i = 0;
    let mut seen = 0;
    while i < a.1 && a.0[i] != b' ' {
        i += 1;
    }
    while i < a.1 {
        if a.0[i] == b' ' {
            if seen == n {
                return arg_hex(a, i + 1);
            }
            seen += 1;
        }
        i += 1;
    }
    0
}
/// A child "valid handle" test: the handle is an event that is signalled.
pub fn is_signalled_event(h: usize) -> bool {
    // SAFETY: polling a handle value with a zero timeout never blocks.
    unsafe { WaitForSingleObject(h as HANDLE, 0) == WAIT_OBJECT_0 }
}
pub const JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION: u32 = 1;
pub const JOB_OBJECT_BASIC_LIMIT_INFORMATION: u32 = 2;
pub const JOB_OBJECT_BASIC_PROCESS_ID_LIST: u32 = 3;
pub const JOB_OBJECT_ASSOCIATE_COMPLETION_PORT: u32 = 7;
pub const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
pub const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x8;
pub const JOB_OBJECT_LIMIT_BREAKAWAY_OK: u32 = 0x800;
pub const JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK: u32 = 0x1000;
pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

#[unsafe(no_mangle)]
pub extern "system" fn mainCRTStartup() -> ! {
    if let Some(a) = child_args() {
        let code = child(&a);
        exit(code)
    }
    if open_output().is_err() {
        exit(EXIT_NO_OUTPUT);
    }
    let mut t = Test::new();
    test(&mut t);
    let code = t.finish();
    exit(code)
}


const SAME_ACCESS: u32 = 0x2;
const CLOSE_SOURCE: u32 = 0x1;
const SYNCHRONIZE_: u32 = 0x0010_0000;

fn child(a: &Args) -> u32 {
    let parent = parent_pid(a);
    let c = link_client(if arg_starts(a, "recv") { "ntdpr" } else { "ntdps" }, parent);
    if is_invalid(c) {
        return 91;
    }
    if arg_starts(a, "recv") {
        // two handle values from the parent: a full-access event and a SYNCHRONIZE-only one
        let Some(h1) = recv_u64(c, 5000) else { return 92 };
        let Some(h2) = recv_u64(c, 5000) else { return 93 };
        let (h1, h2) = (h1 as usize as HANDLE, h2 as usize as HANDLE);
        let mut mask = 0u8;
        // not signalled yet
        if wait(h1, 0) == WAIT_TIMEOUT {
            mask |= 1;
        }
        send_bytes(c, &[b'r']);
        // the parent signals its handle and says go
        if recv_byte(c, 5000) != Some(b'g') {
            return 94;
        }
        if wait(h1, 2000) == WAIT_OBJECT_0 {
            mask |= 2;
        }
        // SAFETY: handle values the parent put into this process.
        if unsafe { ResetEvent(h1) } != 0 {
            mask |= 4;
        }
        clear_err();
        // SAFETY: as above; the SYNCHRONIZE-only handle cannot reset the event.
        let ok = unsafe { ResetEvent(h2) } != 0;
        if !ok && le() == ERROR_ACCESS_DENIED {
            mask |= 8;
        }
        // the event is reset now: the other handle sees it
        if wait(h2, 0) == WAIT_TIMEOUT {
            mask |= 16;
        }
        send_bytes(c, &[mask]);
        return 0;
    }
    // "send": create an event and hand its handle value to the parent, who takes it with CLOSE_SOURCE
    let ev = event(true, false);
    let ev2 = event(true, false);
    send_u64(c, ev as usize as u64);
    send_u64(c, ev2 as usize as u64);
    if recv_byte(c, 5000) != Some(b'g') {
        return 95;
    }
    let (open, _, e) = handle_flags(ev);
    let (open2, _, e2) = handle_flags(ev2);
    send_bytes(c, &[if open { 1 } else { 0 }, (e & 0xFF) as u8, if open2 { 1 } else { 0 }, (e2 & 0xFF) as u8]);
    // the parent's duplicate keeps the object alive; give it time to check, then leave
    let _ = recv_byte(c, 3000);
    0
}

fn test(t: &mut Test) {
    // ---- push handles into the child
    let s = link_server("ntdpr");
    let (args, len) = pid_args("recv");
    let Some((p, th, pid)) = start_child(core::str::from_utf8(&args[..len]).unwrap_or(""), false) else {
        t.check("recv_child_started", false, le() as i64);
        return;
    };
    close(th);
    t.check("recv_child_connected", link_accept(s, 8000), 0);
    // SAFETY: plain values.
    let target = unsafe { OpenProcess(PROCESS_DUP_HANDLE | 0x0010_0000 | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    t.check("opened_child_for_dup", !target.is_null(), le() as i64);
    let ev = event(true, false);
    let mut full: HANDLE = core::ptr::null_mut();
    let mut weak: HANDLE = core::ptr::null_mut();
    // SAFETY: handles from above, live out-parameters.
    let st = unsafe { NtDuplicateObject(CUR_PROC, ev, target, &mut full, 0, 0, SAME_ACCESS) };
    t.expect_status("push_same_access_status", st, STATUS_SUCCESS);
    // SAFETY: as above.
    let st = unsafe { NtDuplicateObject(CUR_PROC, ev, target, &mut weak, SYNCHRONIZE_, 0, 0) };
    t.expect_status("push_reduced_status", st, STATUS_SUCCESS);
    t.check("pushed_values_are_nonzero_and_differ", !full.is_null() && !weak.is_null() && full != weak, 0);
    send_u64(s, full as usize as u64);
    send_u64(s, weak as usize as u64);
    t.expect("child_saw_unsignalled", recv_byte(s, 5000).map_or(-1, |b| b as i64), b'r' as i64);
    // SAFETY: handle of our event.
    unsafe { SetEvent(ev) };
    send_bytes(s, &[b'g']);
    let mask = recv_byte(s, 5000).map_or(-1, |b| b as i64);
    t.expect("child_mask_all_five_bits", mask, 31);
    t.expect("child_exit", wait_exit(p, 5000) as i64, 0);
    // the child's reset is visible here: same object
    t.expect("child_reset_seen_by_parent", wait(ev, 0) as i64, WAIT_TIMEOUT as i64);
    // the child is gone: its handle values are gone with it
    let mut late: HANDLE = core::ptr::null_mut();
    // SAFETY: target is an exited process; live out-parameter.
    let st = unsafe { NtDuplicateObject(CUR_PROC, ev, target, &mut late, 0, 0, SAME_ACCESS) };
    t.value_hex("push_into_exited_process_status", st as u32 as u64);
    close(target);
    close(p);
    close(ev);
    close(s);

    // ---- without PROCESS_DUP_HANDLE the push is refused, kernel32 and ntdll alike
    let s = link_server("ntdpr");
    let (args, len) = pid_args("recv");
    let Some((p, th, pid)) = start_child(core::str::from_utf8(&args[..len]).unwrap_or(""), false) else {
        t.check("recv_child_2_started", false, le() as i64);
        return;
    };
    close(th);
    link_accept(s, 8000);
    // SAFETY: plain values.
    let nodup = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    let ev = event(true, false);
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: handles from above, live out-parameter.
    let st = unsafe { NtDuplicateObject(CUR_PROC, ev, nodup, &mut out, 0, 0, SAME_ACCESS) };
    t.expect_status("push_without_dup_right_status", st, STATUS_ACCESS_DENIED);
    clear_err();
    // SAFETY: as above.
    let ok = unsafe { DuplicateHandle(GetCurrentProcess(), ev, nodup, &mut out, 0, 0, DUPLICATE_SAME_ACCESS) } != 0;
    t.check("duplicate_handle_without_dup_right_fails", !ok, 0);
    t.expect("duplicate_handle_without_dup_right_error", le() as i64, ERROR_ACCESS_DENIED as i64);
    // let the child end: it waits for handle values; closing the pipe ends its receive
    close(s);
    t.value("recv_child_2_exit_after_pipe_closed", wait_exit(p, 8000) as i64);
    close(nodup);
    close(p);
    close(ev);

    // ---- pull a handle out of the child, closing it there
    let s = link_server("ntdps");
    let (args, len) = pid_args("send");
    let Some((p, th, pid)) = start_child(core::str::from_utf8(&args[..len]).unwrap_or(""), false) else {
        t.check("send_child_started", false, le() as i64);
        return;
    };
    close(th);
    t.check("send_child_connected", link_accept(s, 8000), 0);
    // SAFETY: plain values.
    let source = unsafe { OpenProcess(PROCESS_DUP_HANDLE | 0x0010_0000, 0, pid) };
    t.check("opened_child_as_source", !source.is_null(), le() as i64);
    let remote = recv_u64(s, 5000).unwrap_or(0) as usize as HANDLE;
    let remote2 = recv_u64(s, 5000).unwrap_or(0) as usize as HANDLE;
    t.check("child_sent_a_handle_value", !remote.is_null() && !remote2.is_null(), 0);
    // taking a copy first leaves the child's handle open
    let mut copy: HANDLE = core::ptr::null_mut();
    // SAFETY: handles from above, live out-parameter.
    let st = unsafe { NtDuplicateObject(source, remote, CUR_PROC, &mut copy, 0, 0, SAME_ACCESS) };
    t.expect_status("pull_copy_status", st, STATUS_SUCCESS);
    t.expect("pulled_event_unsignalled", wait(copy, 0) as i64, WAIT_TIMEOUT as i64);
    t.expect("pulled_event_has_two_handles", obj_basic(copy).map_or(-1, |b| b.1 as i64), 2);
    // now move it: DUPLICATE_CLOSE_SOURCE closes the child's handle
    let mut moved: HANDLE = core::ptr::null_mut();
    // SAFETY: handles from above, live out-parameter.
    let st = unsafe { NtDuplicateObject(source, remote, CUR_PROC, &mut moved, 0, 0, SAME_ACCESS | CLOSE_SOURCE) };
    t.expect_status("pull_close_source_status", st, STATUS_SUCCESS);
    t.expect("event_has_two_handles_again", obj_basic(copy).map_or(-1, |b| b.1 as i64), 2);
    // closing a handle in the child without taking it: no target process, no target handle
    // SAFETY: handles from above; NULL target process and handle are allowed with CLOSE_SOURCE.
    let st = unsafe { NtDuplicateObject(source, remote2, core::ptr::null_mut(), core::ptr::null_mut(), 0, 0, CLOSE_SOURCE) };
    t.expect_status("remote_close_without_target_status", st, STATUS_SUCCESS);
    send_bytes(s, &[b'g']);
    let r = recv_byte(s, 5000).unwrap_or(9);
    let e = recv_byte(s, 5000).unwrap_or(0);
    let r2 = recv_byte(s, 5000).unwrap_or(9);
    let e2 = recv_byte(s, 5000).unwrap_or(0);
    t.expect("child_handle_is_closed", r as i64, 0);
    t.expect("child_handle_error_is_invalid_handle", e as i64, ERROR_INVALID_HANDLE as i64);
    t.expect("remote_close_without_target_closed_the_childs_handle", r2 as i64, 0);
    t.expect("remote_close_without_target_error_is_invalid_handle", e2 as i64, ERROR_INVALID_HANDLE as i64);
    // the closed handle cannot be pulled again
    let mut again: HANDLE = core::ptr::null_mut();
    // SAFETY: handles from above, live out-parameter.
    let st = unsafe { NtDuplicateObject(source, remote, CUR_PROC, &mut again, 0, 0, SAME_ACCESS) };
    t.expect_status("pull_closed_handle_status", st, STATUS_INVALID_HANDLE);
    clear_err();
    // SAFETY: as above.
    let ok = unsafe { DuplicateHandle(source, remote, GetCurrentProcess(), &mut again, 0, 0, DUPLICATE_SAME_ACCESS) } != 0;
    t.check("duplicate_handle_closed_handle_fails", !ok, 0);
    t.expect("duplicate_handle_closed_handle_error", le() as i64, ERROR_INVALID_HANDLE as i64);
    // the object outlives the child's handle
    // SAFETY: handle of the pulled event.
    t.check("pulled_event_works", unsafe { SetEvent(moved) } != 0, le() as i64);
    t.expect("pulled_event_signalled_through_other_handle", wait(copy, 0) as i64, WAIT_OBJECT_0 as i64);
    send_bytes(s, &[b'x']);
    t.expect("send_child_exit", wait_exit(p, 5000) as i64, 0);
    close(copy);
    close(moved);
    close(source);
    close(p);
    close(s);
}
