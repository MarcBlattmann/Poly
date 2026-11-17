//! One request and one reply over a named pipe (ADR 0220), and starting the activation process.

use super::*;
use alloc::vec::Vec;
use poly_com::hresult::from_win32;
use poly_com::wire::{ACTIVATION_PIPE, Reply, Request, frame, frame_len};

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PIPE_BUSY: u32 = 231;
/// `PIPE_ACCESS_DUPLEX`; `PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT` is 0.
pub(crate) const PIPE_ACCESS_DUPLEX: u32 = 3;
pub(crate) const PIPE_UNLIMITED_INSTANCES: u32 = 255;
pub(crate) const ERROR_PIPE_CONNECTED: u32 = 535;

/// Why a transaction failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Nobody listens on that pipe (the process is gone, or the activation process not started).
    NoServer,
    /// The connection broke or the reply did not parse.
    Broken(Hresult),
}

impl Failure {
    pub(crate) fn hr(self) -> Hresult {
        match self {
            Failure::NoServer => RPC_S_SERVER_UNAVAILABLE,
            Failure::Broken(hr) => hr,
        }
    }
}

/// Write all of `bytes`.
pub(crate) fn write_all(h: Handle, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        let mut n = 0u32;
        // SAFETY: a pipe handle; reads `bytes.len()` bytes.
        if unsafe { WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
            return false;
        }
        bytes = &bytes[n as usize..];
    }
    true
}

/// Read exactly `buf.len()` bytes.
pub(crate) fn read_exact(h: Handle, buf: &mut [u8]) -> bool {
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

/// Read one frame's payload.
pub(crate) fn read_frame(h: Handle) -> Option<Vec<u8>> {
    let mut head = [0u8; 4];
    if !read_exact(h, &mut head) {
        return None;
    }
    let n = frame_len(head)?;
    let mut v = alloc::vec![0u8; n];
    read_exact(h, &mut v).then_some(v)
}

/// Open the client end of `name`, waiting while every instance is busy.
fn open(name: &[u16]) -> Result<Handle, Failure> {
    for _ in 0..200 {
        // SAFETY: a NUL-terminated name.
        let h = unsafe {
            CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0)
        };
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

/// Send `req` to the pipe `endpoint` and read the reply.
pub(crate) fn transact(endpoint: &str, req: &Request) -> Result<Reply, Failure> {
    let name = wide(endpoint);
    let h = open(&name)?;
    let ok = write_all(h, &frame(&req.encode()));
    let reply = if ok { read_frame(h) } else { None };
    close(h);
    let reply = reply.ok_or(Failure::Broken(RPC_S_SERVER_UNAVAILABLE))?;
    Reply::decode(&reply).map_err(|_| Failure::Broken(RPC_S_SERVER_UNAVAILABLE))
}

/// Send `req` to the activation process; with `start`, start it first when it does not run.
pub(crate) fn activation(req: &Request, start: bool) -> Result<Reply, Failure> {
    match transact(ACTIVATION_PIPE, req) {
        Err(Failure::NoServer) if start => {}
        r => return r,
    }
    if !start_activation_process() {
        return Err(Failure::NoServer);
    }
    // It is listening by now (a connection that finds every instance busy waits in `open`).
    for _ in 0..100 {
        match transact(ACTIVATION_PIPE, req) {
            Err(Failure::NoServer) => {
                // SAFETY: a plain sleep.
                unsafe { Sleep(10) }
            }
            r => return r,
        }
    }
    Err(Failure::NoServer)
}

/// The name of the mutex that lets one process start `comlaunch.exe`.
const START_MUTEX: &str = "PolyComActivationStart";

/// Start `C:\Windows\System32\comlaunch.exe` unless another process did; wait until its pipe exists
/// (10 s at most). Whether it runs.
fn start_activation_process() -> bool {
    let name = wide(START_MUTEX);
    // SAFETY: a named mutex; the handle is closed below.
    let m = unsafe { CreateMutexW(core::ptr::null(), 0, name.as_ptr()) };
    if m != 0 {
        // SAFETY: our own mutex handle.
        unsafe { crate::WaitForSingleObject(m as usize, 15_000) };
    }
    let pipe = wide(ACTIVATION_PIPE);
    let running = || {
        // SAFETY: a NUL-terminated name.
        let h = unsafe {
            CreateFileW(pipe.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0)
        };
        if h != INVALID_HANDLE {
            // A connection with no request: the activation process reads nothing and lets it go.
            close(h);
            return true;
        }
        // SAFETY: no arguments.
        (unsafe { GetLastError() }) == ERROR_PIPE_BUSY
    };
    let mut up = running();
    if !up {
        let mut dir = [0u16; 260];
        // SAFETY: the buffer holds 260 units.
        let n = unsafe { GetSystemDirectoryW(dir.as_mut_ptr(), 260) } as usize;
        let mut cmd: Vec<u16> = Vec::new();
        cmd.push(u16::from(b'"'));
        cmd.extend_from_slice(&dir[..n.min(259)]);
        cmd.extend("\\comlaunch.exe\"".encode_utf16());
        cmd.push(0);
        let si = StartupInfo::new();
        let mut pi = ProcessInformation { process: 0, thread: 0, pid: 0, tid: 0 };
        // SAFETY: `cmd` is mutable and NUL-terminated; live structures. CREATE_NO_WINDOW (0x0800_0000).
        let ok = unsafe {
            CreateProcessW(
                core::ptr::null(),
                cmd.as_mut_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                0,
                0x0800_0000,
                core::ptr::null(),
                core::ptr::null(),
                &si,
                &mut pi,
            )
        };
        if ok != 0 {
            close(pi.thread);
            close(pi.process);
            for _ in 0..1000 {
                if running() {
                    up = true;
                    break;
                }
                // SAFETY: a plain sleep.
                unsafe { Sleep(10) };
            }
        } else {
            crate::log("ole32: cannot start comlaunch.exe: no COM activation across processes");
        }
    }
    if m != 0 {
        // SAFETY: the mutex this thread holds.
        unsafe { ReleaseMutex(m) };
        close(m);
    }
    up
}
