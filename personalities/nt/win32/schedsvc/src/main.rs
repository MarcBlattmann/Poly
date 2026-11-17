//! `schedsvc.exe` (ADR 0226): Poly's Task Scheduler, the process that plays Windows' `Schedule`
//! service. It owns the task store (`poly_taskschd::service::Service`, host-tested), keeps it in
//! `C:\Windows\System32\Tasks` (a file of registered XML per task, a directory per folder, and
//! Poly's state file), answers `taskschd.dll` on the pipe `\\.\pipe\poly-taskschd`, starts and
//! watches the tasks' processes, and runs the clock that fires their triggers.
//!
//! `taskschd.dll` starts it on demand; it runs until the machine stops. When B1's service host
//! exists it becomes the `Schedule` service (ADR 0226, section 4). Functions from Microsoft Learn
//! (prototypes) and the SDK headers named beside the constants.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use poly_taskschd::hr::{E_UNEXPECTED, Hresult, from_win32};
use poly_taskschd::parse::parse_task;
use poly_taskschd::persist::{self, STATE_FILE, TASKS_DIR, file_of};
use poly_taskschd::service::{Host, Service};
use poly_taskschd::wire::{PIPE, Reply, Request, frame, frame_len};

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
    fn ReadFile(file: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut c_void) -> i32;
    fn WriteFile(file: Handle, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: Handle) -> Handle;
    fn DeleteFileW(name: *const u16) -> i32;
    fn CreateDirectoryW(name: *const u16, sa: *const c_void) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn FindFirstFileW(pattern: *const u16, data: *mut FindData) -> Handle;
    fn FindNextFileW(find: Handle, data: *mut FindData) -> i32;
    fn FindClose(find: Handle) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn GetLastError() -> u32;
    fn Sleep(ms: u32);
    fn GetTickCount64() -> u64;
    fn GetSystemTimeAsFileTime(out: *mut u64);
    fn GetCurrentProcessId() -> u32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn CreateMutexW(sa: *const c_void, owner: i32, name: *const u16) -> Handle;
    fn ReleaseMutex(m: Handle) -> i32;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn ExpandEnvironmentStringsW(src: *const u16, dst: *mut u16, size: u32) -> u32;
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
    fn ExitProcess(code: u32) -> !;
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

/// `WIN32_FIND_DATAW` (592 bytes).
#[repr(C)]
struct FindData {
    attributes: u32,
    times: [u32; 6],
    size_high: u32,
    size_low: u32,
    reserved: [u32; 2],
    name: [u16; 260],
    alternate: [u16; 14],
}

// winbase.h / winnt.h / fileapi.h.
const PIPE_ACCESS_DUPLEX: u32 = 3;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const ERROR_PIPE_CONNECTED: u32 = 535;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_PARAMETER: u32 = 87;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const INFINITE: u32 = 0xFFFF_FFFF;
/// `CREATE_NO_WINDOW`: a task's program has no console of its own (session 0 on Windows).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Seconds from 1601 to 1970.
const FILETIME_UNIX_SECONDS: u64 = 11_644_473_600;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

// ------------------------------------------------------------------------------- the host

/// What the scheduler does to the system: time, processes, files.
struct Sys {
    /// The processes of running instances: the instance's GUID and the process handle.
    procs: Vec<(String, Handle)>,
    boot: i64,
}

fn now() -> i64 {
    let mut ft = 0u64;
    // SAFETY: an out parameter of 8 bytes.
    unsafe { GetSystemTimeAsFileTime(&mut ft) };
    (ft / 10_000_000).saturating_sub(FILETIME_UNIX_SECONDS) as i64
}

fn expand(s: &str) -> String {
    if !s.contains('%') {
        return String::from(s);
    }
    let src = wide(s);
    let mut out = alloc::vec![0u16; 4096];
    // SAFETY: a NUL-terminated source and a buffer of 4096 units.
    let n = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), out.as_mut_ptr(), 4096) } as usize;
    if n == 0 || n > out.len() {
        return String::from(s);
    }
    String::from_utf16_lossy(&out[..n.saturating_sub(1)])
}

fn write_file(path: &str, bytes: &[u8]) -> bool {
    let p = wide(path);
    // SAFETY: a NUL-terminated path; the handle is closed below.
    let h = unsafe { CreateFileW(p.as_ptr(), GENERIC_WRITE, FILE_SHARE_READ, core::ptr::null(), CREATE_ALWAYS, 0x80, 0) };
    if h == INVALID_HANDLE {
        return false;
    }
    let mut at = 0;
    let mut ok = true;
    while at < bytes.len() {
        let mut n = 0u32;
        // SAFETY: our file handle; reads the rest of `bytes`.
        if unsafe { WriteFile(h, bytes[at..].as_ptr(), (bytes.len() - at) as u32, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
            ok = false;
            break;
        }
        at += n as usize;
    }
    // SAFETY: our own handle.
    unsafe { CloseHandle(h) };
    ok
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    let p = wide(path);
    // SAFETY: a NUL-terminated path; the handle is closed below.
    let h = unsafe { CreateFileW(p.as_ptr(), GENERIC_READ, FILE_SHARE_READ, core::ptr::null(), OPEN_EXISTING, 0x80, 0) };
    if h == INVALID_HANDLE {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut n = 0u32;
        // SAFETY: our file handle; writes into `buf`.
        if unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    // SAFETY: our own handle.
    unsafe { CloseHandle(h) };
    Some(out)
}

/// A task file's text: UTF-16LE (with or without a byte order mark) or UTF-8.
fn decode_text(b: &[u8]) -> String {
    if b.len() >= 2 && b[0] == 0xFF && b[1] == 0xFE {
        let u: Vec<u16> = b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&u);
    }
    let s = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    String::from_utf8_lossy(s).into_owned()
}

fn make_dirs(dir: &str) {
    // Each component in turn (an existing one is fine).
    let mut at = 0;
    while let Some(i) = dir[at..].find('\\') {
        let end = at + i;
        if end > 2 {
            let p = wide(&dir[..end]);
            // SAFETY: a NUL-terminated path.
            unsafe { CreateDirectoryW(p.as_ptr(), core::ptr::null()) };
        }
        at = end + 1;
    }
    let p = wide(dir);
    // SAFETY: a NUL-terminated path.
    unsafe { CreateDirectoryW(p.as_ptr(), core::ptr::null()) };
}

impl Host for Sys {
    fn now(&self) -> i64 {
        now()
    }

    fn boot(&self) -> i64 {
        self.boot
    }

    fn spawn(&mut self, guid: &str, command_line: &str, working_directory: Option<&str>) -> Result<u32, Hresult> {
        let mut cmd = wide(&expand(command_line));
        let dir = working_directory.map(|d| wide(&expand(d)));
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
        let mut start = |dir: *const u16, cmd: &mut Vec<u16>, pi: &mut ProcessInformation| {
            // SAFETY: a mutable NUL-terminated command line, live structures.
            unsafe {
                CreateProcessW(
                    core::ptr::null(),
                    cmd.as_mut_ptr(),
                    core::ptr::null(),
                    core::ptr::null(),
                    0,
                    CREATE_NO_WINDOW,
                    core::ptr::null(),
                    dir,
                    &si,
                    pi,
                )
            }
        };
        let mut ok = start(dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr()), &mut cmd, &mut pi);
        if ok == 0 && dir.is_some() {
            // A working directory that does not exist: start it in ours.
            ok = start(core::ptr::null(), &mut cmd, &mut pi);
        }
        if ok == 0 {
            // SAFETY: no arguments.
            return Err(from_win32(unsafe { GetLastError() }));
        }
        // SAFETY: the thread handle CreateProcessW returned, closed once.
        unsafe { CloseHandle(pi.thread) };
        self.procs.push((String::from(guid), pi.process));
        Ok(pi.pid)
    }

    fn terminate(&mut self, guid: &str) {
        if let Some(i) = self.procs.iter().position(|(g, _)| g == guid) {
            let (_, h) = self.procs.remove(i);
            // SAFETY: a process handle of ours, closed once. The exit code is what a terminated
            // task's last result says (SCHED_S_TASK_TERMINATED).
            unsafe {
                TerminateProcess(h, 0x0004_1306);
                WaitForSingleObject(h, 2000);
                CloseHandle(h);
            }
        }
    }

    fn forget(&mut self, guid: &str) {
        if let Some(i) = self.procs.iter().position(|(g, _)| g == guid) {
            let (_, h) = self.procs.remove(i);
            // SAFETY: a process handle of ours, closed once; the process keeps running.
            unsafe { CloseHandle(h) };
        }
    }

    fn write_task(&mut self, path: &str, xml: &str) {
        let file = file_of(path);
        if let Some(i) = file.rfind('\\') {
            make_dirs(&file[..i]);
        }
        let mut bytes = alloc::vec![0xFF, 0xFE];
        for u in xml.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        if !write_file(&file, &bytes) {
            poly_taskschd_log("schedsvc: cannot write the task file");
        }
    }

    fn delete_task(&mut self, path: &str) {
        let p = wide(&file_of(path));
        // SAFETY: a NUL-terminated path.
        unsafe { DeleteFileW(p.as_ptr()) };
    }

    fn make_folder(&mut self, path: &str) {
        make_dirs(&file_of(path));
    }

    fn remove_folder(&mut self, path: &str) {
        let p = wide(&file_of(path));
        // SAFETY: a NUL-terminated path.
        unsafe { RemoveDirectoryW(p.as_ptr()) };
    }

    fn save_state(&mut self, text: &str) {
        write_file(STATE_FILE, text.as_bytes());
    }
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
}

fn poly_taskschd_log(text: &str) {
    // SAFETY: the service reads `text.len()` bytes.
    unsafe { PolyDebugPrint(text.as_ptr(), text.len()) };
}

// ------------------------------------------------------------------------------- the state

struct State {
    svc: Service,
    sys: Sys,
}

/// The scheduler's state behind a kernel mutex (held across process starts and file writes, so a
/// spin lock would not do).
struct Guarded {
    mutex: UnsafeCell<Handle>,
    state: UnsafeCell<Option<State>>,
}

// SAFETY: `state` is reached only with `mutex` held.
unsafe impl Sync for Guarded {}

static STATE: Guarded = Guarded { mutex: UnsafeCell::new(0), state: UnsafeCell::new(None) };

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    // SAFETY: the mutex handle is set once before any thread but the first runs.
    let m = unsafe { *STATE.mutex.get() };
    // SAFETY: a mutex handle of ours.
    unsafe { WaitForSingleObject(m, INFINITE) };
    // SAFETY: the mutex is held.
    let r = unsafe { (*STATE.state.get()).as_mut() }.map(f);
    // SAFETY: as above.
    unsafe { ReleaseMutex(m) };
    r
}

/// The files of `dir` (the store's folder `path`) into the service, recursively.
fn load(svc: &mut Service, dir: &str, path: &str, now: i64, boot: i64) {
    let pattern = wide(&alloc::format!("{dir}\\*"));
    let mut fd = FindData { attributes: 0, times: [0; 6], size_high: 0, size_low: 0, reserved: [0; 2], name: [0; 260], alternate: [0; 14] };
    // SAFETY: a NUL-terminated pattern and a live structure; closed below.
    let h = unsafe { FindFirstFileW(pattern.as_ptr(), &mut fd) };
    if h == INVALID_HANDLE {
        return;
    }
    loop {
        let n = fd.name.iter().position(|&u| u == 0).unwrap_or(260);
        let name = String::from_utf16_lossy(&fd.name[..n]);
        if !name.starts_with('.') {
            let full = alloc::format!("{dir}\\{name}");
            let p = if path == "\\" { alloc::format!("\\{name}") } else { alloc::format!("{path}\\{name}") };
            if fd.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                svc.store.ensure_folder(&p);
                load(svc, &full, &p, now, boot);
            } else if let Some(b) = read_file(&full)
                && let Ok(d) = parse_task(&decode_text(&b))
            {
                svc.store.insert(&p, d, now, boot);
            }
        }
        // SAFETY: our find handle and structure.
        if unsafe { FindNextFileW(h, &mut fd) } == 0 {
            break;
        }
    }
    // SAFETY: our find handle.
    unsafe { FindClose(h) };
}

// ------------------------------------------------------------------------------- the clock

unsafe extern "system" fn clock(_arg: *mut c_void) -> u32 {
    loop {
        // SAFETY: a plain sleep.
        unsafe { Sleep(100) };
        with(|st| {
            let mut ended = Vec::new();
            st.sys.procs.retain(|(g, h)| {
                // SAFETY: a process handle of ours.
                if unsafe { WaitForSingleObject(*h, 0) } == 0 {
                    let mut code = 0u32;
                    // SAFETY: as above; closed once here.
                    unsafe {
                        GetExitCodeProcess(*h, &mut code);
                        CloseHandle(*h);
                    }
                    ended.push((g.clone(), code));
                    false
                } else {
                    true
                }
            });
            for (g, code) in ended {
                st.svc.ended(&g, code, &mut st.sys);
            }
            st.svc.tick(&mut st.sys);
        });
    }
}

// ------------------------------------------------------------------------------- the pipe

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

unsafe extern "system" fn connection(arg: *mut c_void) -> u32 {
    let h = arg as Handle;
    let mut head = [0u8; 4];
    let payload = if read_exact(h, &mut head) { frame_len(head) } else { None }.and_then(|n| {
        let mut p = alloc::vec![0u8; n];
        read_exact(h, &mut p).then_some(p)
    });
    if let Some(p) = payload {
        let reply = match Request::decode(&p) {
            Ok(req) => with(|st| st.svc.handle(req, &mut st.sys)).unwrap_or(Reply::hr(E_UNEXPECTED)),
            Err(()) => Reply::hr(E_UNEXPECTED),
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

/// The entry point: load the store, start the clock, serve the pipe until the machine stops.
#[unsafe(no_mangle)]
pub extern "system" fn schedsvc_main(_peb: *mut u8) -> u32 {
    let name = wide(PIPE);
    // The first instance must be ours alone: a second scheduler leaves at once.
    let mut first = FILE_FLAG_FIRST_PIPE_INSTANCE;
    let mut pending = {
        // SAFETY: a NUL-terminated name; byte mode, blocking.
        let h = unsafe {
            CreateNamedPipeW(name.as_ptr(), PIPE_ACCESS_DUPLEX | first, 0, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, core::ptr::null())
        };
        if h == INVALID_HANDLE {
            // SAFETY: no arguments.
            match unsafe { GetLastError() } {
                // SAFETY: ends this process (another scheduler has the pipe).
                ERROR_ACCESS_DENIED => unsafe { ExitProcess(0) },
                ERROR_INVALID_PARAMETER => {}
                // SAFETY: as above.
                _ => unsafe { ExitProcess(1) },
            }
        }
        h
    };
    first = 0;
    // SAFETY: an unnamed mutex, kept for the life of the process; set before other threads run.
    unsafe { *STATE.mutex.get() = CreateMutexW(core::ptr::null(), 0, core::ptr::null()) };
    let t = now();
    // SAFETY: no arguments.
    let up = (unsafe { GetTickCount64() } / 1000) as i64;
    let boot = (t - up) / 10 * 10;
    // SAFETY: no arguments.
    let seed = (t as u64) ^ (u64::from(unsafe { GetCurrentProcessId() }) << 32) ^ up as u64;
    let mut svc = Service::new(seed);
    make_dirs(&alloc::format!("{TASKS_DIR}\\Microsoft\\Windows"));
    load(&mut svc, TASKS_DIR, "\\", t, boot);
    let saved = read_file(STATE_FILE).map(|b| persist::decode(&decode_text(&b))).unwrap_or_default();
    let mut sys = Sys { procs: Vec::new(), boot };
    svc.start(&saved, &mut sys);
    // SAFETY: no other thread runs yet.
    unsafe { *STATE.state.get() = Some(State { svc, sys }) };
    let mut tid = 0u32;
    // SAFETY: the clock runs for the life of the process.
    let c = unsafe { CreateThread(core::ptr::null(), 0, clock, core::ptr::null_mut(), 0, &mut tid) };
    if c != 0 {
        // SAFETY: our thread handle.
        unsafe { CloseHandle(c) };
    }
    let mut failures = 0;
    loop {
        let h = if pending != INVALID_HANDLE {
            core::mem::replace(&mut pending, INVALID_HANDLE)
        } else {
            // SAFETY: a NUL-terminated name; byte mode, blocking.
            unsafe {
                CreateNamedPipeW(name.as_ptr(), PIPE_ACCESS_DUPLEX | first, 0, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, core::ptr::null())
            }
        };
        if h == INVALID_HANDLE {
            failures += 1;
            if failures > 100 {
                // SAFETY: ends this process.
                unsafe { ExitProcess(1) };
            }
            // SAFETY: a plain sleep.
            unsafe { Sleep(20) };
            continue;
        }
        failures = 0;
        // SAFETY: a pipe instance of ours.
        let ok = unsafe { ConnectNamedPipe(h, core::ptr::null_mut()) } != 0
            // SAFETY: no arguments.
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !ok {
            // SAFETY: our own handle.
            unsafe { CloseHandle(h) };
            continue;
        }
        let mut tid = 0u32;
        // SAFETY: the connection thread owns the handle from here.
        let t = unsafe { CreateThread(core::ptr::null(), 0, connection, h as *mut c_void, 0, &mut tid) };
        // SAFETY: handles of ours, closed once.
        unsafe {
            if t == 0 {
                CloseHandle(h);
            } else {
                CloseHandle(t);
            }
        }
    }
}
