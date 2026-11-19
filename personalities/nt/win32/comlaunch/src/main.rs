//! `comlaunch.exe` (ADR 0220): Poly's COM activation process. It plays RPCSS's endpoint mapper
//! and the DCOM launcher: servers publish their class objects here (`Register`, `Revoke`), and a
//! client's `CoCreateInstance(CLSCTX_LOCAL_SERVER)` asks here (`Activate`) for the class object,
//! which is served at once, or after the server this process starts for it has registered.
//!
//! It starts when `ole32` first needs it, by the kernel and as LocalSystem in session 0
//! (`PolyStartSystemProcess`, ADR 0235), and runs until the machine stops; the servers it starts
//! run as the client that activates them. The decisions are
//! `poly_com::activation::Activator`'s (host-tested); this file is the pipe, the threads, starting
//! programs and services, and waiting. Functions from Microsoft Learn (prototypes) and the SDK
//! headers named beside the constants.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_com::activation::{Activator, Decision, Published};
use poly_com::hresult::{E_UNEXPECTED, Hresult, REGDB_E_CLASSNOTREG, S_OK, from_win32};
use poly_com::wire::{ACTIVATION_PIPE, Op, Plan, Reply, Request, frame, frame_len};

poly_win32_rt::process_heap_allocator!();

type Handle = isize;
const INVALID_HANDLE: Handle = -1;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateThread(
        security: *const c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        arg: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> Handle;
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        security: *const c_void,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn ReadFile(file: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut c_void) -> i32;
    fn WriteFile(file: Handle, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn GetLastError() -> u32;
    fn Sleep(ms: u32);
    fn GetTickCount64() -> u64;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn GetNamedPipeClientProcessId(pipe: Handle, pid: *mut u32) -> i32;
    fn ExpandEnvironmentStringsW(src: *const u16, dst: *mut u16, size: u32) -> u32;
    fn LoadLibraryW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
    fn ExitProcess(code: u32) -> !;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, access: u32, out: *mut isize) -> i32;
    fn RegQueryValueExW(
        key: isize,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: isize) -> i32;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn DuplicateTokenEx(
        token: Handle,
        access: u32,
        attributes: *const c_void,
        level: u32,
        kind: u32,
        new: *mut Handle,
    ) -> i32;
    fn CreateProcessAsUserW(
        token: Handle,
        app: *const u16,
        cmd: *mut u16,
        process_security: *const c_void,
        thread_security: *const c_void,
        inherit: i32,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        startup: *const StartupInfo,
        info: *mut ProcessInformation,
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

/// `PIPE_ACCESS_DUPLEX`; byte mode, blocking is 0; `PIPE_UNLIMITED_INSTANCES`.
const PIPE_ACCESS_DUPLEX: u32 = 3;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const ERROR_PIPE_CONNECTED: u32 = 535;
const ERROR_PIPE_BUSY: u32 = 231;
/// `RPC_E_DISCONNECTED` (`winerror.h`): an exporter's answer for an object it let go of.
const RPC_E_DISCONNECTED: Hresult = 0x8001_0108_u32 as i32;
/// `FILE_FLAG_FIRST_PIPE_INSTANCE` (`winbase.h`); `ERROR_ACCESS_DENIED`, `ERROR_INVALID_PARAMETER`.
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_PARAMETER: u32 = 87;
/// `SYNCHRONIZE`; `PROCESS_QUERY_LIMITED_INFORMATION` (`winnt.h`).
const SYNCHRONIZE: u32 = 0x0010_0000;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// `TOKEN_DUPLICATE`, `TOKEN_QUERY`, `MAXIMUM_ALLOWED` (`winnt.h`); `SecurityImpersonation` and
/// `TokenPrimary` (`SECURITY_IMPERSONATION_LEVEL`, `TOKEN_TYPE`).
const TOKEN_DUPLICATE: u32 = 0x0002;
const TOKEN_QUERY: u32 = 0x0008;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const SECURITY_IMPERSONATION: u32 = 2;
const TOKEN_PRIMARY: u32 = 1;
/// `HKEY_LOCAL_MACHINE`, `KEY_READ`.
const HKLM: isize = 0x8000_0002_u32 as i32 as isize;
const KEY_READ: u32 = 0x2_0019;
/// `ERROR_SERVICE_DOES_NOT_EXIST` (1060), `ERROR_SERVICE_ALREADY_RUNNING` (1056).
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;
/// `CO_E_SERVER_EXEC_FAILURE` (`winerror.h`).
const CO_E_SERVER_EXEC_FAILURE: Hresult = 0x8008_0005_u32 as i32;

/// A spin lock around the state (held for a few instructions, never across I/O).
struct Locked<T> {
    flag: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the flag serialises every access to the value.
unsafe impl<T> Sync for Locked<T> {}

impl<T> Locked<T> {
    const fn new(v: T) -> Locked<T> {
        Locked { flag: AtomicBool::new(false), value: UnsafeCell::new(v) }
    }
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held.
        let r = f(unsafe { &mut *self.value.get() });
        self.flag.store(false, Ordering::Release);
        r
    }
}

static STATE: Locked<Activator> = Locked::new(Activator::new());
/// The registering processes and a handle of each (to see when they end).
static PROCESSES: Locked<Vec<(u32, Handle)>> = Locked::new(Vec::new());

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn now() -> u64 {
    // SAFETY: no arguments.
    unsafe { GetTickCount64() }
}

fn read_exact(h: Handle, buf: &mut [u8]) -> bool {
    let mut at = 0;
    while at < buf.len() {
        let mut n = 0u32;
        // SAFETY: a pipe handle; writes into the free part of `buf`.
        let ok =
            unsafe { ReadFile(h, buf.as_mut_ptr().add(at), (buf.len() - at) as u32, &mut n, core::ptr::null_mut()) };
        if ok == 0 || n == 0 {
            return false;
        }
        at += n as usize;
    }
    true
}

fn write_all(h: Handle, mut bytes: &[u8]) -> bool {
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

/// Forget the registrations of processes that ended.
fn prune() {
    let gone: Vec<u32> = PROCESSES.with(|p| {
        let mut gone = Vec::new();
        p.retain(|&(pid, h)| {
            // SAFETY: a process handle this program opened.
            if unsafe { WaitForSingleObject(h, 0) } == 0 {
                // SAFETY: as above, closed once.
                unsafe { CloseHandle(h) };
                gone.push(pid);
                false
            } else {
                true
            }
        });
        gone
    });
    if !gone.is_empty() {
        STATE.with(|a| {
            for pid in gone {
                a.process_gone(pid);
            }
        });
    }
}

fn watch(pid: u32) {
    if PROCESSES.with(|p| p.iter().any(|e| e.0 == pid)) {
        return;
    }
    // SAFETY: a process id from a registration; the handle is kept until it ends.
    let h = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if h != 0 {
        PROCESSES.with(|p| p.push((pid, h)));
    }
}

/// A registry string value under `HKLM`.
fn hklm_value(path: &str, name: &str) -> Option<String> {
    let p = wide(path);
    let n = wide(name);
    let mut key = 0isize;
    // SAFETY: NUL-terminated strings and live out parameters; the key is closed below.
    unsafe {
        if RegOpenKeyExW(HKLM, p.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return None;
        }
        let mut buf = alloc::vec![0u16; 1024];
        let mut kind = 0u32;
        let mut len = (buf.len() * 2) as u32;
        let r = RegQueryValueExW(key, n.as_ptr(), core::ptr::null_mut(), &mut kind, buf.as_mut_ptr().cast(), &mut len);
        RegCloseKey(key);
        if r != 0 || (kind != 1 && kind != 2) {
            return None;
        }
        let units = &buf[..(len as usize / 2).min(buf.len())];
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        Some(String::from_utf16_lossy(&units[..end]))
    }
}

fn key_exists(path: &str) -> bool {
    let p = wide(path);
    let mut key = 0isize;
    // SAFETY: a NUL-terminated path; the key is closed at once.
    unsafe {
        if RegOpenKeyExW(HKLM, p.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return false;
        }
        RegCloseKey(key);
    }
    true
}

/// A primary token of the client at the other end of pipe `h`: the servers this process starts
/// for a client run as the client (ADR 0235; this process is LocalSystem, Windows' DCOM launcher
/// starts a server as the user that activates it). `None` when the client cannot be found.
fn client_token(h: Handle) -> Option<Handle> {
    let mut pid = 0u32;
    // SAFETY: our pipe handle and an out parameter; every handle opened here is closed here
    // except the token returned.
    unsafe {
        if GetNamedPipeClientProcessId(h, &mut pid) == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process == 0 {
            return None;
        }
        let mut token: Handle = 0;
        let opened = OpenProcessToken(process, TOKEN_DUPLICATE | TOKEN_QUERY, &mut token) != 0;
        CloseHandle(process);
        if !opened {
            return None;
        }
        let mut primary: Handle = 0;
        let ok = DuplicateTokenEx(
            token,
            MAXIMUM_ALLOWED,
            core::ptr::null(),
            SECURITY_IMPERSONATION,
            TOKEN_PRIMARY,
            &mut primary,
        ) != 0;
        CloseHandle(token);
        ok.then_some(primary)
    }
}

/// Start `cmd` as a process of the client on pipe `client`.
fn run(cmd: &str, client: Handle) -> Result<(), Hresult> {
    let Some(token) = client_token(client) else {
        return Err(CO_E_SERVER_EXEC_FAILURE);
    };
    let mut c = wide(cmd);
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
    // SAFETY: a mutable NUL-terminated command line and live structures; the token is ours.
    let ok = unsafe {
        CreateProcessAsUserW(
            token,
            core::ptr::null(),
            c.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
            core::ptr::null(),
            core::ptr::null(),
            &si,
            &mut pi,
        )
    };
    // SAFETY: our token handle, closed once.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return Err(REGDB_E_CLASSNOTREG);
    }
    // SAFETY: the handles CreateProcessW returned, closed once.
    unsafe {
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
    }
    Ok(())
}

/// A function of `advapi32.dll` (the service control manager's, B1), or 0.
fn advapi(name: &[u8]) -> usize {
    let lib = wide("advapi32.dll");
    // SAFETY: kernel32 calls with NUL-terminated names.
    unsafe { GetProcAddress(LoadLibraryW(lib.as_ptr()), name.as_ptr()) }
}

/// Start the service `name` with `args` through the service control manager. `None`: the manager
/// is not there (B1 not implemented): the caller falls back.
fn start_service(name: &str, args: &[String]) -> Option<Result<(), Hresult>> {
    type OpenScm = unsafe extern "system" fn(*const u16, *const u16, u32) -> usize;
    type OpenSvc = unsafe extern "system" fn(usize, *const u16, u32) -> usize;
    type Start = unsafe extern "system" fn(usize, u32, *const *const u16) -> i32;
    type CloseSvc = unsafe extern "system" fn(usize) -> i32;
    let (o, s, st, c) = (
        advapi(b"OpenSCManagerW\0"),
        advapi(b"OpenServiceW\0"),
        advapi(b"StartServiceW\0"),
        advapi(b"CloseServiceHandle\0"),
    );
    if o == 0 || s == 0 || st == 0 || c == 0 {
        return None;
    }
    // SAFETY: the documented prototypes (Learn: OpenSCManagerW, OpenServiceW, StartServiceW,
    // CloseServiceHandle); SC_MANAGER_CONNECT (1), SERVICE_START (0x10) | SERVICE_QUERY_STATUS (4).
    unsafe {
        let (o, s, st, c) = (
            core::mem::transmute::<usize, OpenScm>(o),
            core::mem::transmute::<usize, OpenSvc>(s),
            core::mem::transmute::<usize, Start>(st),
            core::mem::transmute::<usize, CloseSvc>(c),
        );
        let scm = o(core::ptr::null(), core::ptr::null(), 1);
        if scm == 0 {
            return None;
        }
        let n = wide(name);
        let svc = s(scm, n.as_ptr(), 0x14);
        if svc == 0 {
            let e = GetLastError();
            c(scm);
            return Some(Err(from_win32(e)));
        }
        let w: Vec<Vec<u16>> = args.iter().map(|a| wide(a)).collect();
        let ptrs: Vec<*const u16> = w.iter().map(|a| a.as_ptr()).collect();
        let ok = st(svc, ptrs.len() as u32, if ptrs.is_empty() { core::ptr::null() } else { ptrs.as_ptr() });
        let e = GetLastError();
        c(svc);
        c(scm);
        if ok != 0 || e == ERROR_SERVICE_ALREADY_RUNNING { Some(Ok(())) } else { Some(Err(from_win32(e))) }
    }
}

/// Start what `plan` says, for the client on pipe `client`.
fn launch(plan: &Plan, client: Handle) -> Result<(), Hresult> {
    match plan {
        Plan::Command(cmd) => run(cmd, client),
        Plan::Service { name, args } => {
            let key = alloc::format!(r"SYSTEM\CurrentControlSet\Services\{name}");
            if !key_exists(&key) {
                return Err(from_win32(ERROR_SERVICE_DOES_NOT_EXIST));
            }
            if let Some(r) = start_service(name, args) {
                return r;
            }
            // Until B1's service control manager exists: the service's program as a plain process.
            let image = hklm_value(&key, "ImagePath").ok_or(from_win32(ERROR_SERVICE_DOES_NOT_EXIST))?;
            let image = expand(&image);
            let mut cmd = image;
            for a in args {
                cmd.push(' ');
                cmd.push_str(a);
            }
            run(&cmd, client).map_err(|_| CO_E_SERVER_EXEC_FAILURE)
        }
    }
}

/// `%Name%` references of a `REG_EXPAND_SZ` value.
fn expand(s: &str) -> String {
    if !s.contains('%') {
        return String::from(s);
    }
    let src = wide(s);
    let mut out = alloc::vec![0u16; 2048];
    // SAFETY: NUL-terminated source, a buffer of 2048 units.
    let n = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), out.as_mut_ptr(), 2048) } as usize;
    if n == 0 || n > out.len() {
        return String::from(s);
    }
    String::from_utf16_lossy(&out[..n.saturating_sub(1)])
}

/// Whether the exporter of `p`'s server still has its factory: a call that needs no apartment
/// (giving back no references) answers `RPC_E_DISCONNECTED` for a withdrawn one; a pipe that is
/// gone means the server is.
fn factory_alive(p: &Published) -> bool {
    let name = wide(&p.factory.endpoint);
    let mut h = INVALID_HANDLE;
    for _ in 0..100 {
        // SAFETY: a NUL-terminated name; GENERIC_READ | GENERIC_WRITE, OPEN_EXISTING; closed below.
        h = unsafe { CreateFileW(name.as_ptr(), 0xC000_0000, 0, core::ptr::null(), 3, 0, 0) };
        // SAFETY: no arguments.
        if h != INVALID_HANDLE || unsafe { GetLastError() } != ERROR_PIPE_BUSY {
            break;
        }
        // SAFETY: a plain sleep.
        unsafe { Sleep(5) };
    }
    if h == INVALID_HANDLE {
        return false;
    }
    let req = Request::Call { oid: p.factory.oid, op: Op::Release(0) };
    let reply = if write_all(h, &frame(&req.encode())) {
        let mut head = [0u8; 4];
        if read_exact(h, &mut head) {
            frame_len(head).and_then(|n| {
                let mut b = alloc::vec![0u8; n];
                read_exact(h, &mut b).then_some(b)
            })
        } else {
            None
        }
    } else {
        None
    };
    // SAFETY: our handle, closed once.
    unsafe { CloseHandle(h) };
    match reply.and_then(|b| Reply::decode(&b).ok()) {
        Some(r) => r.hr != RPC_E_DISCONNECTED,
        None => false,
    }
}

/// Answer one request from the client on pipe `client`.
fn handle(req: Request, client: Handle) -> Reply {
    match req {
        Request::Register { clsid, pid, cookie, factory, single } => {
            watch(pid);
            STATE.with(|a| a.register(Published { clsid, pid, cookie, factory, single }));
            Reply::hr(S_OK)
        }
        Request::Revoke { pid, cookie } => {
            STATE.with(|a| a.revoke(pid, cookie));
            Reply::hr(S_OK)
        }
        Request::Activate { clsid, plan } => {
            let mut started: Option<u64> = None;
            // A registration that arrives while this request waits is fresh: it is handed out at
            // once, without the check below (a server may leave soon after it registered; Windows
            // serves the waiting caller within that time).
            let mut waited = false;
            loop {
                prune();
                let t = now();
                match STATE.with(|a| a.activate(&clsid, plan.as_ref(), t, started)) {
                    Decision::Serve(p) => {
                        // A server withdraws without waiting, and may end before its withdrawal
                        // arrives: a factory its exporter no longer has is dropped, and the class
                        // asked for again (which starts a server when there is none).
                        if waited || factory_alive(&p) {
                            return Reply { hr: S_OK, obj: Some(p.factory), data: Vec::new() };
                        }
                        STATE.with(|a| a.revoke(p.pid, p.cookie));
                    }
                    Decision::NotRegistered => return Reply::hr(REGDB_E_CLASSNOTREG),
                    Decision::TimedOut => return Reply::hr(CO_E_SERVER_EXEC_FAILURE),
                    Decision::Wait => {
                        waited = true;
                        // SAFETY: a plain sleep (1 ms: the caller is answered as soon as the server
                        // registers).
                        unsafe { Sleep(1) };
                    }
                    Decision::Launch => {
                        waited = true;
                        started = Some(t);
                        if let Some(p) = &plan
                            && let Err(hr) = launch(p, client)
                        {
                            STATE.with(|a| a.launch_failed(&clsid, p));
                            return Reply::hr(hr);
                        }
                    }
                }
            }
        }
        Request::Call { .. } => Reply::hr(E_UNEXPECTED),
    }
}

unsafe extern "system" fn connection(arg: *mut c_void) -> u32 {
    let h = arg as Handle;
    let mut head = [0u8; 4];
    let payload = if read_exact(h, &mut head) { frame_len(head) } else { None }.and_then(|n| {
        let mut payload = alloc::vec![0u8; n];
        read_exact(h, &mut payload).then_some(payload)
    });
    if let Some(payload) = payload {
        let reply = match Request::decode(&payload) {
            Ok(r) => handle(r, h),
            Err(_) => Reply::hr(E_UNEXPECTED),
        };
        if write_all(h, &frame(&reply.encode())) {
            // Wait for the client to read the reply and close its end.
            let mut b = [0u8; 1];
            read_exact(h, &mut b);
        }
    }
    // SAFETY: the connection's handle, closed once.
    unsafe { CloseHandle(h) };
    0
}

/// A listening instance of the activation pipe; `first` adds `FILE_FLAG_FIRST_PIPE_INSTANCE`.
fn new_instance(name: &[u16], first: u32) -> Handle {
    // SAFETY: a NUL-terminated name; byte mode, blocking.
    unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | first,
            0,
            PIPE_UNLIMITED_INSTANCES,
            65536,
            65536,
            0,
            core::ptr::null(),
        )
    }
}

/// The entry point: serve the activation pipe until the machine stops.
///
/// A listening instance exists at every moment: the next one is made before a connected one is
/// handed to its thread, so a client never finds the pipe gone while this process runs (a client
/// takes "no pipe" for "no activation process", ADR 0220).
#[unsafe(no_mangle)]
pub extern "system" fn comlaunch_main(_peb: *mut u8) -> u32 {
    let name = wide(ACTIVATION_PIPE);
    let mut failures = 0;
    // The first instance must be ours alone (`FILE_FLAG_FIRST_PIPE_INSTANCE`): a second activation
    // process (two clients that started one each) leaves at once.
    let mut first = FILE_FLAG_FIRST_PIPE_INSTANCE;
    let mut h = new_instance(&name, first);
    loop {
        if h == INVALID_HANDLE {
            // SAFETY: no arguments.
            match unsafe { GetLastError() } {
                // Another activation process has the pipe.
                ERROR_ACCESS_DENIED if first != 0 => {
                    // SAFETY: ends this process.
                    unsafe { ExitProcess(0) }
                }
                // A pipe implementation without the flag: go on without it.
                ERROR_INVALID_PARAMETER if first != 0 => {
                    first = 0;
                    h = new_instance(&name, first);
                    continue;
                }
                _ => {}
            }
            failures += 1;
            if failures > 100 {
                // The pipe cannot be made: nothing to do.
                // SAFETY: ends this process.
                unsafe { ExitProcess(1) };
            }
            // SAFETY: a plain sleep.
            unsafe { Sleep(20) };
            h = new_instance(&name, first);
            continue;
        }
        failures = 0;
        first = 0;
        // SAFETY: a pipe instance of ours.
        let ok = unsafe { ConnectNamedPipe(h, core::ptr::null_mut()) } != 0
            // SAFETY: no arguments.
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !ok {
            // SAFETY: our own handle.
            unsafe { CloseHandle(h) };
            h = new_instance(&name, 0);
            continue;
        }
        let connected = h;
        h = new_instance(&name, 0);
        let mut tid = 0u32;
        // SAFETY: the connection thread owns the handle from here.
        let t = unsafe { CreateThread(core::ptr::null(), 0, connection, connected as *mut c_void, 0, &mut tid) };
        // SAFETY: handles of ours, closed once.
        unsafe {
            if t == 0 {
                CloseHandle(connected);
            } else {
                CloseHandle(t);
            }
        }
    }
}
