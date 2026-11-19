//! `polyopen.exe` (ADR 0230): start a program the way the Windows shell opens it.
//!
//! ```text
//! polyopen PROGRAM [ARGUMENTS...]
//! ```
//!
//! A double click in Windows' shell runs `ShellExecuteEx` on the file: the program's manifest is
//! read, and a program that requests `requireAdministrator` (or `highestAvailable`, for an
//! administrator) gets the consent prompt before it runs (Learn, "How User Account Control
//! works"; ADR 0222: `CreateProcessW` refuses it with `ERROR_ELEVATION_REQUIRED`, and
//! `ShellExecuteExW` then asks the consent broker). Poly's installer window is a native program
//! and cannot call `ShellExecuteExW`; it starts this program instead, which calls it with the
//! default verb, waits for the program, and ends with its exit code.
//!
//! It waits for **everything the program started** too, exactly: before it opens the program it
//! puts itself into a new job object, which every process it and its descendants create joins
//! (ADR 0169; Learn, "Job Objects": a child process is associated with its parent's job). When
//! the program has ended it waits until it is the job's only process left
//! (`JobObjectBasicAccountingInformation.ActiveProcesses`), and says how many it waits for on
//! its standard output each time that changes: `polyopen: the program has ended; waiting for N
//! it started`. An installer that starts a helper, or an elevated copy of itself, is waited for
//! whole, however briefly a process in between lives.
//!
//! If `ShellExecuteExW` fails, it writes one line to its standard output,
//! `polyopen: ShellExecuteExW failed: <error>` (1223, `ERROR_CANCELLED`, when the user said no),
//! and ends with 1. No installer detection by name is done: Windows' rule for that (32-bit images
//! without a manifest whose names say "setup" or "install") has no spec sheet.
//!
//! Every call is on Microsoft Learn (`GetCommandLineW`, `ShellExecuteExW`, `WaitForSingleObject`,
//! `GetExitCodeProcess`, `CloseHandle`, `GetStdHandle`, `WriteFile`, `GetLastError`,
//! `ExitProcess`, `CreateJobObjectW`, `AssignProcessToJobObject`, `QueryInformationJobObject`,
//! `GetCurrentProcess`, `Sleep`); `SHELLEXECUTEINFOW` and the constants are `shellapi.h`'s, `winbase.h`'s and
//! `winuser.h`'s.

#![no_std]
#![no_main]

use core::ffi::c_void;
use poly_win32_rt as _;

type Handle = *mut c_void;

/// `SHELLEXECUTEINFOW` (shellapi.h).
#[repr(C)]
struct ShellExecuteInfoW {
    size: u32,
    mask: u32,
    hwnd: Handle,
    verb: *const u16,
    file: *const u16,
    parameters: *const u16,
    directory: *const u16,
    show: i32,
    inst_app: Handle,
    id_list: *mut c_void,
    class: *const u16,
    key_class: Handle,
    hot_key: u32,
    icon_or_monitor: Handle,
    process: Handle,
}

/// shellapi.h, winbase.h, winuser.h.
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
const SEE_MASK_FLAG_NO_UI: u32 = 0x400;
const INFINITE: u32 = 0xFFFF_FFFF;
const SW_SHOWNORMAL: i32 = 1;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetCommandLineW() -> *const u16;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(file: Handle, buffer: *const u8, to_write: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn GetLastError() -> u32;
    fn ExitProcess(code: u32) -> !;
    fn CreateJobObjectW(security: *const c_void, name: *const u16) -> Handle;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    fn QueryInformationJobObject(job: Handle, class: u32, info: *mut u8, length: u32, returned: *mut u32) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn Sleep(milliseconds: u32);
}

/// `JobObjectBasicAccountingInformation` (`winnt.h`) and where its `ActiveProcesses` is in the
/// 48-byte `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`.
const JOB_BASIC_ACCOUNTING: u32 = 1;
const ACTIVE_PROCESSES_AT: usize = 40;

/// The processes in `job`, `None` if it cannot be asked.
fn active_processes(job: Handle) -> Option<u32> {
    let mut info = [0u8; 48];
    // SAFETY: a 48-byte buffer for the 48-byte structure.
    let ok =
        unsafe { QueryInformationJobObject(job, JOB_BASIC_ACCOUNTING, info.as_mut_ptr(), 48, core::ptr::null_mut()) };
    let b = &info[ACTIVE_PROCESSES_AT..ACTIVE_PROCESSES_AT + 4];
    (ok != 0).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg_attr(target_arch = "x86", link(name = "shell32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "shell32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
}

/// The longest program path and parameter string taken.
const MAX: usize = 32768;

/// Where the next token of a command line ends (Learn, "Parsing C++ command-line arguments", for
/// the program name: quotes toggle, nothing is escaped) and the token without its quotes,
/// copied into `out` with a NUL. Returns the index after the token.
fn token(line: &[u16], mut i: usize, out: &mut [u16]) -> (usize, usize) {
    let mut quoted = false;
    let mut n = 0;
    while i < line.len() {
        let c = line[i];
        if c == u16::from(b'"') {
            quoted = !quoted;
        } else if !quoted && (c == u16::from(b' ') || c == u16::from(b'\t')) {
            break;
        } else if n + 1 < out.len() {
            out[n] = c;
            n += 1;
        }
        i += 1;
    }
    out[n] = 0;
    (i, n)
}

fn skip_blanks(line: &[u16], mut i: usize) -> usize {
    while i < line.len() && (line[i] == u16::from(b' ') || line[i] == u16::from(b'\t')) {
        i += 1;
    }
    i
}

/// Write `parts` and a line end to the standard output.
fn say(parts: &[&[u8]]) {
    // SAFETY: no preconditions.
    let out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if out.is_null() || out as isize == -1 {
        return;
    }
    let mut written = 0u32;
    for p in parts.iter().chain([&&b"\n"[..]]) {
        // SAFETY: `p` is valid for its length; `written` is writable.
        unsafe { WriteFile(out, p.as_ptr(), p.len() as u32, &mut written, core::ptr::null_mut()) };
    }
}

/// `n` in decimal.
fn decimal(mut n: u32, buf: &mut [u8; 10]) -> &[u8] {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            return &buf[i..];
        }
    }
}

static mut PROGRAM: [u16; MAX] = [0; MAX];
static mut PARAMETERS: [u16; MAX] = [0; MAX];

#[unsafe(no_mangle)]
pub extern "system" fn polyopen_main() -> ! {
    // SAFETY: GetCommandLineW returns the process's NUL-terminated command line, which lives as
    // long as the process.
    let line = unsafe {
        let p = GetCommandLineW();
        let mut n = 0;
        while !p.is_null() && *p.add(n) != 0 && n < MAX {
            n += 1;
        }
        if p.is_null() { &[][..] } else { core::slice::from_raw_parts(p, n) }
    };
    // SAFETY: the one thread of this program is the only user of the two buffers.
    let (program, parameters) = unsafe { (&mut *(&raw mut PROGRAM), &mut *(&raw mut PARAMETERS)) };
    let mut ignored = [0u16; 2];
    let (i, _) = token(line, 0, &mut ignored);
    let i = skip_blanks(line, i);
    let (i, n) = token(line, i, program);
    if n == 0 {
        say(&[b"usage: polyopen PROGRAM [ARGUMENTS...]"]);
        // SAFETY: ends the process.
        unsafe { ExitProcess(2) };
    }
    // The rest of the line, as it is, is the program's parameters.
    let rest = &line[skip_blanks(line, i)..];
    let len = rest.len().min(MAX - 1);
    parameters[..len].copy_from_slice(&rest[..len]);
    parameters[len] = 0;
    // A job of its own, before anything is started: what the program starts joins it.
    // SAFETY: no security attributes, no name.
    let job = unsafe { CreateJobObjectW(core::ptr::null(), core::ptr::null()) };
    // SAFETY: the job just made and this process's pseudo-handle.
    let in_job = !job.is_null() && unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) } != 0;
    let mut info = ShellExecuteInfoW {
        size: core::mem::size_of::<ShellExecuteInfoW>() as u32,
        // No message box of the shell's own on failure: the caller says why.
        mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        hwnd: core::ptr::null_mut(),
        verb: core::ptr::null(),
        file: program.as_ptr(),
        parameters: if len == 0 { core::ptr::null() } else { parameters.as_ptr() },
        directory: core::ptr::null(),
        show: SW_SHOWNORMAL,
        inst_app: core::ptr::null_mut(),
        id_list: core::ptr::null_mut(),
        class: core::ptr::null(),
        key_class: core::ptr::null_mut(),
        hot_key: 0,
        icon_or_monitor: core::ptr::null_mut(),
        process: core::ptr::null_mut(),
    };
    // SAFETY: a valid SHELLEXECUTEINFOW whose strings outlive the call.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // SAFETY: no preconditions.
        let error = unsafe { GetLastError() };
        let mut buf = [0u8; 10];
        say(&[b"polyopen: ShellExecuteExW failed: ", decimal(error, &mut buf)]);
        // SAFETY: ends the process.
        unsafe { ExitProcess(1) };
    }
    let mut code = 0u32;
    if !info.process.is_null() {
        // SAFETY: the process handle ShellExecuteExW returned (SEE_MASK_NOCLOSEPROCESS), closed
        // once.
        unsafe {
            WaitForSingleObject(info.process, INFINITE);
            GetExitCodeProcess(info.process, &mut code);
            CloseHandle(info.process);
        }
    }
    // Then what it started, until this program is the job's last process.
    let mut told = 0;
    while in_job {
        let others = active_processes(job).map_or(0, |n| n.saturating_sub(1));
        if others == 0 {
            break;
        }
        if others != told {
            told = others;
            let mut buf = [0u8; 10];
            say(&[b"polyopen: the program has ended; waiting for ", decimal(others, &mut buf), b" it started"]);
        }
        // SAFETY: no preconditions.
        unsafe { Sleep(100) };
    }
    // SAFETY: ends the process.
    unsafe { ExitProcess(code) }
}
