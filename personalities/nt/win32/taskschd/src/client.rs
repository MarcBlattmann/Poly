//! The pipe to the scheduler process (ADR 0226, section 4): one request and one reply per
//! connection. When the pipe is not there the scheduler is started (`schedsvc.exe` in System32,
//! under a named mutex so that only one process starts it), as `ole32` starts `comlaunch.exe`
//! (ADR 0220). When B1's service control manager exists this becomes `StartServiceW("Schedule")`.

use crate::{HRESULT, log};
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_taskschd::hr::{SERVER_UNAVAILABLE, from_win32};
use poly_taskschd::wire::{PIPE, Reply, Request, frame, frame_len};

type Handle = isize;
const INVALID_HANDLE: Handle = -1;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
/// `ERROR_FILE_NOT_FOUND`, `ERROR_PIPE_BUSY` (`winerror.h`).
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PIPE_BUSY: u32 = 231;
/// `CREATE_NO_WINDOW`.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: Handle) -> Handle;
    fn ReadFile(file: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut c_void) -> i32;
    fn WriteFile(file: Handle, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn GetLastError() -> u32;
    fn Sleep(ms: u32);
    fn WaitNamedPipeW(name: *const u16, ms: u32) -> i32;
    fn CreateMutexW(sa: *const c_void, owner: i32, name: *const u16) -> Handle;
    fn ReleaseMutex(m: Handle) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn GetSystemDirectoryW(buf: *mut u16, n: u32) -> u32;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        ps: *const c_void,
        ts: *const c_void,
        inherit: i32,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *const StartupInfo,
        pi: *mut ProcessInformation,
    ) -> i32;
}

/// `STARTUPINFOW`.
#[repr(C)]
struct StartupInfo {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    pos: [u32; 7],
    flags: u32,
    show_window: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std: [Handle; 3],
}

/// `PROCESS_INFORMATION`.
#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn write_all(h: Handle, mut b: &[u8]) -> bool {
    while !b.is_empty() {
        let mut n = 0u32;
        // SAFETY: a pipe handle; reads `b.len()` bytes.
        if unsafe { WriteFile(h, b.as_ptr(), b.len() as u32, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
            return false;
        }
        b = &b[n as usize..];
    }
    true
}

fn read_exact(h: Handle, buf: &mut [u8]) -> bool {
    let mut at = 0;
    while at < buf.len() {
        let mut n = 0u32;
        // SAFETY: a pipe handle; writes into the free part of `buf`.
        let ok = unsafe { ReadFile(h, buf.as_mut_ptr().add(at), (buf.len() - at) as u32, &mut n, core::ptr::null_mut()) };
        if ok == 0 || n == 0 {
            return false;
        }
        at += n as usize;
    }
    true
}

enum Failure {
    NoServer,
    Broken(HRESULT),
}

fn open(name: &[u16]) -> Result<Handle, Failure> {
    for _ in 0..400 {
        // SAFETY: a NUL-terminated name.
        let h = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0) };
        if h != INVALID_HANDLE {
            return Ok(h);
        }
        // SAFETY: no arguments.
        match unsafe { GetLastError() } {
            ERROR_PIPE_BUSY => {
                // SAFETY: a NUL-terminated name.
                if unsafe { WaitNamedPipeW(name.as_ptr(), 50) } == 0 {
                    // SAFETY: a plain sleep.
                    unsafe { Sleep(1) };
                }
            }
            ERROR_FILE_NOT_FOUND => return Err(Failure::NoServer),
            e => return Err(Failure::Broken(from_win32(e))),
        }
    }
    Err(Failure::Broken(from_win32(ERROR_PIPE_BUSY)))
}

fn transact(req: &[u8]) -> Result<Reply, Failure> {
    let name = wide(PIPE);
    let h = open(&name)?;
    let mut reply = None;
    if write_all(h, &frame(req)) {
        let mut head = [0u8; 4];
        if read_exact(h, &mut head)
            && let Some(n) = frame_len(head)
        {
            let mut v = alloc::vec![0u8; n];
            if read_exact(h, &mut v) {
                reply = Reply::decode(&v).ok();
            }
        }
    }
    // SAFETY: our own handle.
    unsafe { CloseHandle(h) };
    reply.ok_or(Failure::Broken(SERVER_UNAVAILABLE))
}

/// Send `req` to the scheduler (starting it if it does not run) and return its reply; a reply
/// with `SERVER_UNAVAILABLE` when it cannot be reached.
pub fn call(req: &Request) -> Reply {
    let bytes = req.encode();
    match transact(&bytes) {
        Ok(r) => return r,
        Err(Failure::Broken(hr)) => return Reply::hr(hr),
        Err(Failure::NoServer) => {}
    }
    if !start_scheduler() {
        return Reply::hr(SERVER_UNAVAILABLE);
    }
    for _ in 0..200 {
        match transact(&bytes) {
            Ok(r) => return r,
            Err(Failure::Broken(hr)) => return Reply::hr(hr),
            // SAFETY: a plain sleep.
            Err(Failure::NoServer) => unsafe { Sleep(10) },
        }
    }
    Reply::hr(SERVER_UNAVAILABLE)
}

/// The mutex that lets one process start the scheduler.
const START_MUTEX: &str = "PolyTaskSchedulerStart";

fn running() -> bool {
    let name = wide(PIPE);
    // SAFETY: a NUL-terminated name.
    let h = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0) };
    if h != INVALID_HANDLE {
        // A connection with no request: the scheduler reads nothing and lets it go.
        // SAFETY: our own handle.
        unsafe { CloseHandle(h) };
        return true;
    }
    // SAFETY: no arguments.
    unsafe { GetLastError() == ERROR_PIPE_BUSY }
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> isize;
    fn OpenServiceW(scm: isize, name: *const u16, access: u32) -> isize;
    fn StartServiceW(service: isize, argc: u32, argv: *const *const u16) -> i32;
    fn CloseServiceHandle(h: isize) -> i32;
}

/// `SC_MANAGER_CONNECT`, `SERVICE_START` (winsvc.h); `ERROR_SERVICE_ALREADY_RUNNING`.
const SC_MANAGER_CONNECT: u32 = 1;
const SERVICE_START: u32 = 0x10;
const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;

/// Start the `Schedule` service through the service control manager (ADR 0221). Whether it was
/// started (or already ran); false when there is no such service or the caller may not start it.
fn start_service() -> bool {
    let name = wide("Schedule");
    // SAFETY: the documented calls (Learn: OpenSCManagerW, OpenServiceW, StartServiceW); every
    // handle is closed here.
    unsafe {
        let scm = OpenSCManagerW(core::ptr::null(), core::ptr::null(), SC_MANAGER_CONNECT);
        if scm == 0 {
            return false;
        }
        let svc = OpenServiceW(scm, name.as_ptr(), SERVICE_START);
        let ok = svc != 0 && (StartServiceW(svc, 0, core::ptr::null()) != 0 || GetLastError() == ERROR_SERVICE_ALREADY_RUNNING);
        if svc != 0 {
            CloseServiceHandle(svc);
        }
        CloseServiceHandle(scm);
        ok
    }
}

/// Start the scheduler unless another process did: the `Schedule` service, or (no service, no
/// right to start it) `C:\Windows\System32\schedsvc.exe` as a plain process; wait until its pipe
/// exists (10 s at most). Whether it runs.
fn start_scheduler() -> bool {
    let mname = wide(START_MUTEX);
    // SAFETY: a named mutex; closed below.
    let m = unsafe { CreateMutexW(core::ptr::null(), 0, mname.as_ptr()) };
    if m != 0 {
        // SAFETY: our own mutex handle.
        unsafe { WaitForSingleObject(m, 15_000) };
    }
    let mut up = running();
    if !up && start_service() {
        for _ in 0..1000 {
            if running() {
                up = true;
                break;
            }
            // SAFETY: a plain sleep.
            unsafe { Sleep(10) };
        }
    }
    if !up {
        let mut dir = [0u16; 260];
        // SAFETY: the buffer holds 260 units.
        let n = unsafe { GetSystemDirectoryW(dir.as_mut_ptr(), 260) } as usize;
        let mut cmd: Vec<u16> = Vec::new();
        cmd.push(u16::from(b'"'));
        cmd.extend_from_slice(&dir[..n.min(259)]);
        cmd.extend("\\schedsvc.exe\"".encode_utf16());
        cmd.push(0);
        let si = StartupInfo {
            cb: core::mem::size_of::<StartupInfo>() as u32,
            reserved: core::ptr::null_mut(),
            desktop: core::ptr::null_mut(),
            title: core::ptr::null_mut(),
            pos: [0; 7],
            flags: 0,
            show_window: 0,
            cb_reserved2: 0,
            reserved2: core::ptr::null_mut(),
            std: [0; 3],
        };
        let mut pi = ProcessInformation { process: 0, thread: 0, pid: 0, tid: 0 };
        // SAFETY: `cmd` is mutable and NUL-terminated; live structures.
        let ok = unsafe {
            CreateProcessW(
                core::ptr::null(),
                cmd.as_mut_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                0,
                CREATE_NO_WINDOW,
                core::ptr::null(),
                core::ptr::null(),
                &si,
                &mut pi,
            )
        };
        if ok != 0 {
            // SAFETY: the handles CreateProcessW returned, closed once.
            unsafe {
                CloseHandle(pi.thread);
                CloseHandle(pi.process);
            }
            for _ in 0..1000 {
                if running() {
                    up = true;
                    break;
                }
                // SAFETY: a plain sleep.
                unsafe { Sleep(10) };
            }
        } else {
            log("taskschd: cannot start schedsvc.exe: no task scheduler");
        }
    }
    if m != 0 {
        // SAFETY: the mutex this thread holds, then our handle.
        unsafe {
            ReleaseMutex(m);
            CloseHandle(m);
        }
    }
    up
}
