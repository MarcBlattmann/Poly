//! The launcher of `cargo xtask chrome-win run` (ADR 0155): starts the
//! program a stage names the way a Windows user session would, with
//! `CreateProcessW`, waits for it, and says how it ended.
//!
//! The kernel's command line cannot carry a path with a space
//! (`C:\Program Files\Chrome\chrome.exe`), and an NT program cannot be
//! started from a Linux shell, so the stage is a file beside the launcher,
//! `stage<N>.cmd` (N is the launcher's one argument): line 1 the program (a DOS path), line 2 its command line
//! (the program quoted as `CreateProcessW` expects, then the arguments),
//! line 3 the current directory (optional), line 4 the time to wait in
//! seconds (optional, default 300). The program's standard output and
//! error go to `stdout.txt` and `stderr.txt` beside the launcher, standard
//! input is an empty `stdin.txt`; afterwards the launcher prints them to
//! its own standard output between marker lines, then
//! `LAUNCH-EXIT code=<hex>` (or `timeout`, `spawn-error=<error>`).
//!
//! Every call is on Microsoft Learn (`CreateProcessW`, `WaitForSingleObject`,
//! `GetExitCodeProcess`, `TerminateProcess`, `CreateFileW`, `ReadFile`,
//! `WriteFile`, `GetStdHandle`, `GetModuleFileNameW`); the layouts are from
//! the public SDK headers (`processthreadsapi.h`, `minwinbase.h`).
#![no_std]
#![no_main]

use core::cell::UnsafeCell;
use core::ffi::c_void;
use poly_wtest::*;

type BOOL = i32;

/// `SECURITY_ATTRIBUTES` (minwinbase.h).
#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: BOOL,
}

/// `STARTUPINFOW` (processthreadsapi.h), 104 bytes on x64.
#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std_input: HANDLE,
    std_output: HANDLE,
    std_error: HANDLE,
}

/// `PROCESS_INFORMATION` (processthreadsapi.h).
#[repr(C)]
struct ProcessInformation {
    process: HANDLE,
    thread: HANDLE,
    process_id: u32,
    thread_id: u32,
}

const OPEN_EXISTING: u32 = 3;
const CREATE_ALWAYS: u32 = 2;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_ALL: u32 = 7;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const STARTF_USESTDHANDLES: u32 = 0x100;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const SecurityAttributes,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn ReadFile(file: HANDLE, buffer: *mut u8, to_read: u32, read: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn WriteFile(file: HANDLE, buffer: *const u8, to_write: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *const SecurityAttributes,
        thread_attributes: *const SecurityAttributes,
        inherit_handles: BOOL,
        flags: u32,
        environment: *const c_void,
        current_directory: *const u16,
        startup_info: *const StartupInfoW,
        information: *mut ProcessInformation,
    ) -> BOOL;
    fn WaitForSingleObject(handle: HANDLE, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: HANDLE, code: *mut u32) -> BOOL;
    fn TerminateProcess(process: HANDLE, code: u32) -> BOOL;
    fn GetModuleFileNameW(module: HANDLE, name: *mut u16, size: u32) -> u32;
    fn GetStdHandle(which: u32) -> HANDLE;
    fn GetCommandLineW() -> *const u16;
    fn GetLastError() -> u32;
}

/// A buffer the single thread of the launcher owns.
struct Buffer<T>(UnsafeCell<T>);
// SAFETY: one thread; each buffer is used by one function at a time.
unsafe impl<T> Sync for Buffer<T> {}

const PATH_UNITS: usize = 520;
const CMD_BYTES: usize = 4096;
static SPEC: Buffer<[u8; CMD_BYTES]> = Buffer(UnsafeCell::new([0; CMD_BYTES]));
static CHUNK: Buffer<[u8; 8192]> = Buffer(UnsafeCell::new([0; 8192]));

/// The launcher's directory (with the trailing `\`) in UTF-16.
fn own_dir(out: &mut [u16; PATH_UNITS]) -> Option<usize> {
    let mut module = [0u16; PATH_UNITS];
    // SAFETY: the buffer holds PATH_UNITS units.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), module.as_mut_ptr(), PATH_UNITS as u32) } as usize;
    if n == 0 || n >= PATH_UNITS {
        return None;
    }
    let cut = module[..n].iter().rposition(|&c| c == u16::from(b'\\') || c == u16::from(b'/'))? + 1;
    out[..cut].copy_from_slice(&module[..cut]);
    Some(cut)
}

/// `<dir><name>` NUL-terminated.
fn beside(dir: &[u16; PATH_UNITS], dir_len: usize, name: &str) -> [u16; PATH_UNITS] {
    let mut p = [0u16; PATH_UNITS];
    p[..dir_len].copy_from_slice(&dir[..dir_len]);
    for (i, b) in name.bytes().enumerate() {
        p[dir_len + i] = u16::from(b);
    }
    p
}

fn open(path: &[u16; PATH_UNITS], write: bool, inherit: bool) -> Option<HANDLE> {
    let sa = SecurityAttributes {
        length: core::mem::size_of::<SecurityAttributes>() as u32,
        security_descriptor: core::ptr::null_mut(),
        inherit_handle: BOOL::from(inherit),
    };
    // SAFETY: a NUL-terminated path and a valid SECURITY_ATTRIBUTES.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            if write { GENERIC_WRITE } else { GENERIC_READ },
            FILE_SHARE_ALL,
            &sa,
            if write { CREATE_ALWAYS } else { OPEN_EXISTING },
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(h)
}

/// Write `bytes` to the launcher's standard output.
fn say(bytes: &[u8]) {
    // SAFETY: the standard output handle (possibly null: the write fails).
    let out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    let mut done = 0u32;
    // SAFETY: a readable buffer.
    unsafe { WriteFile(out, bytes.as_ptr(), bytes.len() as u32, &mut done, core::ptr::null_mut()) };
}

/// Print the file at `path` between marker lines.
fn show(label: &str, path: &[u16; PATH_UNITS]) {
    say(b"LAUNCH-BEGIN ");
    say(label.as_bytes());
    say(b"\n");
    if let Some(h) = open(path, false, false) {
        let buf = CHUNK.0.get();
        loop {
            let mut got = 0u32;
            // SAFETY: the single thread owns CHUNK; the file handle is open.
            let ok = unsafe { ReadFile(h, (*buf).as_mut_ptr(), 8192, &mut got, core::ptr::null_mut()) };
            if ok == 0 || got == 0 {
                break;
            }
            // SAFETY: the first `got` bytes were just read.
            say(unsafe { core::slice::from_raw_parts(buf.cast::<u8>(), got as usize) });
        }
        // SAFETY: opened above.
        unsafe { CloseHandle(h) };
    }
    say(b"\nLAUNCH-END ");
    say(label.as_bytes());
    say(b"\n");
}

fn say_num(prefix: &str, v: u64, hex: bool) {
    let mut digits = [0u8; 24];
    let mut n = 0;
    let base = if hex { 16 } else { 10 };
    let mut x = v;
    loop {
        digits[n] = b"0123456789abcdef"[(x % base) as usize];
        n += 1;
        x /= base;
        if x == 0 {
            break;
        }
    }
    say(prefix.as_bytes());
    if hex {
        say(b"0x");
    }
    for i in (0..n).rev() {
        say(&digits[i..=i]);
    }
    say(b"\n");
}

/// The lines of `stage.cmd`: (start, end) of each in `SPEC`.
fn lines(spec: &[u8]) -> [(usize, usize); 4] {
    let mut out = [(0, 0); 4];
    let (mut start, mut k) = (0, 0);
    for (i, &c) in spec.iter().enumerate() {
        if c == b'\n' {
            if k < 4 {
                let mut end = i;
                if end > start && spec[end - 1] == b'\r' {
                    end -= 1;
                }
                out[k] = (start, end);
            }
            k += 1;
            start = i + 1;
        }
    }
    if start < spec.len() && k < 4 {
        out[k] = (start, spec.len());
    }
    out
}

/// ASCII bytes as NUL-terminated UTF-16 into `out`.
fn widen(bytes: &[u8], out: &mut [u16]) {
    for (i, &b) in bytes.iter().enumerate() {
        out[i] = u16::from(b);
    }
    out[bytes.len()] = 0;
}

poly_wtest::main!(launch);

fn launch(t: &mut Test) {
    let mut dir = [0u16; PATH_UNITS];
    let Some(dir_len) = own_dir(&mut dir) else {
        say(b"LAUNCH-EXIT driver-error no-module-name\n");
        return;
    };
    // The stage number: the last run of digits of the command line.
    let mut number = 1u32;
    {
        // SAFETY: the process's NUL-terminated command line.
        let mut p = unsafe { GetCommandLineW() };
        let (mut run, mut seen) = (0u32, false);
        while !p.is_null() {
            // SAFETY: up to the terminating NUL.
            let c = unsafe { *p };
            if c == 0 {
                break;
            }
            if (u16::from(b'0')..=u16::from(b'9')).contains(&c) {
                run = run.saturating_mul(10).saturating_add(u32::from(c - u16::from(b'0')));
                seen = true;
            } else if seen {
                number = run;
                (run, seen) = (0, false);
            }
            // SAFETY: still inside the string.
            p = unsafe { p.add(1) };
        }
        if seen {
            number = run;
        }
    }
    let mut name = [0u8; 24];
    let n = {
        let base = b"stage";
        name[..5].copy_from_slice(base);
        let mut digits = [0u8; 10];
        let (mut k, mut v) = (0, number);
        loop {
            digits[k] = b'0' + (v % 10) as u8;
            k += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for i in 0..k {
            name[5 + i] = digits[k - 1 - i];
        }
        name[5 + k..5 + k + 4].copy_from_slice(b".cmd");
        5 + k + 4
    };
    let spec_path = beside(&dir, dir_len, core::str::from_utf8(&name[..n]).unwrap_or("stage1.cmd"));
    let Some(h) = open(&spec_path, false, false) else {
        say(b"LAUNCH-EXIT driver-error no-stage-cmd\n");
        return;
    };
    let spec = SPEC.0.get();
    let mut len = 0u32;
    // SAFETY: the single thread owns SPEC; the file is open.
    unsafe {
        ReadFile(h, (*spec).as_mut_ptr(), (CMD_BYTES - 1) as u32, &mut len, core::ptr::null_mut());
        CloseHandle(h);
    }
    // SAFETY: the first `len` bytes were just read.
    let spec: &[u8] = unsafe { core::slice::from_raw_parts(spec.cast::<u8>(), len as usize) };
    let l = lines(spec);
    let line = |k: usize| &spec[l[k].0..l[k].1];
    let (app, cmdline, cwd) = (line(0), line(1), line(2));
    let seconds = {
        let mut n = 0u32;
        for &c in line(3) {
            if c.is_ascii_digit() {
                n = n * 10 + u32::from(c - b'0');
            }
        }
        if n == 0 { 300 } else { n }
    };
    let mut app_w = [0u16; 1024];
    let mut cmd_w = [0u16; 2048];
    let mut cwd_w = [0u16; 1024];
    if app.len() >= 1024 || cmdline.len() >= 2048 || cwd.len() >= 1024 {
        say(b"LAUNCH-EXIT driver-error stage-too-long\n");
        return;
    }
    widen(app, &mut app_w);
    widen(cmdline, &mut cmd_w);
    widen(cwd, &mut cwd_w);

    let (stdin_p, stdout_p, stderr_p) = (
        beside(&dir, dir_len, "stdin.txt"),
        beside(&dir, dir_len, "stdout.txt"),
        beside(&dir, dir_len, "stderr.txt"),
    );
    let (Some(si_h), Some(so_h), Some(se_h)) = (open(&stdin_p, true, true), open(&stdout_p, true, true), open(&stderr_p, true, true))
    else {
        say(b"LAUNCH-EXIT driver-error standard-handle-files\n");
        return;
    };
    // SAFETY: zeroed is a valid STARTUPINFOW (integers and null pointers).
    let mut si: StartupInfoW = unsafe { core::mem::zeroed() };
    si.cb = core::mem::size_of::<StartupInfoW>() as u32;
    si.flags = STARTF_USESTDHANDLES;
    si.std_input = si_h;
    si.std_output = so_h;
    si.std_error = se_h;
    let mut pi = ProcessInformation { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), process_id: 0, thread_id: 0 };
    say(b"LAUNCH-START ");
    say(app);
    say(b"\n");
    // SAFETY: valid strings (the command line writable), STARTUPINFOW and
    // PROCESS_INFORMATION; the standard handles are inheritable.
    let created = unsafe {
        CreateProcessW(
            app_w.as_ptr(),
            cmd_w.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            1,
            0,
            core::ptr::null(),
            if cwd.is_empty() { core::ptr::null() } else { cwd_w.as_ptr() },
            &si,
            &mut pi,
        )
    };
    // SAFETY: no preconditions.
    let create_error = if created == 0 { unsafe { GetLastError() } } else { 0 };
    for h in [si_h, so_h, se_h] {
        // SAFETY: the launcher's copies, closed once.
        unsafe { CloseHandle(h) };
    }
    if created == 0 {
        say_num("LAUNCH-EXIT spawn-error=", u64::from(create_error), false);
        t.check("spawned", false, i64::from(create_error));
        return;
    }
    // SAFETY: the child's process handle.
    let waited = unsafe { WaitForSingleObject(pi.process, seconds.saturating_mul(1000)) };
    let mut code = 0u32;
    if waited == WAIT_OBJECT_0 {
        // SAFETY: the child's process handle; `code` is writable.
        unsafe { GetExitCodeProcess(pi.process, &mut code) };
    } else {
        // SAFETY: the child's process handle.
        unsafe {
            TerminateProcess(pi.process, 1);
            WaitForSingleObject(pi.process, 10_000);
        }
    }
    // SAFETY: the handles CreateProcessW returned, closed once.
    unsafe {
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
    }
    show("stdout", &stdout_p);
    show("stderr", &stderr_p);
    if waited == WAIT_OBJECT_0 {
        say_num("LAUNCH-EXIT code=", u64::from(code), true);
        t.check("exited", true, i64::from(code));
    } else if waited == WAIT_TIMEOUT {
        say_num("LAUNCH-EXIT timeout seconds=", u64::from(seconds), false);
        t.check("exited", false, 0);
    } else {
        say_num("LAUNCH-EXIT wait-failed=", u64::from(waited), true);
        t.check("exited", false, i64::from(waited));
    }
}
