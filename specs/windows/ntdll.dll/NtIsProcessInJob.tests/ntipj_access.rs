//! NtIsProcessInJob handle rules: process handles with single rights, job handles with single rights, NULL job
//! meaning any job, and NULL, closed, swapped and wrong-type handles.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PVOID = *mut c_void;
type BOOL = i32;
const TRUE: BOOL = 1;
const FALSE: BOOL = 0;
/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const JOB_OBJECT_ASSIGN_PROCESS: u32 = 0x1;
const JOB_OBJECT_SET_ATTRIBUTES: u32 = 0x2;
const JOB_OBJECT_QUERY: u32 = 0x4;
const JOB_OBJECT_TERMINATE: u32 = 0x8;
const JOB_OBJECT_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F;
const CREATE_SUSPENDED: u32 = 0x4;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_TERMINATE: u32 = 0x1;
const PROCESS_SET_QUOTA: u32 = 0x100;
const PROCESS_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0xFFFF;
/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

/// winbase.h: `STARTUPINFOW`.
#[repr(C)]
struct STARTUPINFOW {
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
    reserved2: u16,
    reserved2_ptr: *mut u8,
    std_input: HANDLE,
    std_output: HANDLE,
    std_error: HANDLE,
}

impl STARTUPINFOW {
    fn new() -> STARTUPINFOW {
        // SAFETY: all-zero is a valid STARTUPINFOW (null pointers, zero numbers).
        let mut s: STARTUPINFOW = unsafe { core::mem::zeroed() };
        s.cb = core::mem::size_of::<STARTUPINFOW>() as u32;
        s
    }
}

/// processthreadsapi.h: `PROCESS_INFORMATION`.
#[repr(C)]
struct PROCESS_INFORMATION {
    process: HANDLE,
    thread: HANDLE,
    process_id: u32,
    thread_id: u32,
}

impl PROCESS_INFORMATION {
    fn new() -> PROCESS_INFORMATION {
        PROCESS_INFORMATION {
            process: core::ptr::null_mut(),
            thread: core::ptr::null_mut(),
            process_id: 0,
            thread_id: 0,
        }
    }
}

/// The command line to start this very executable with `arg` appended.
fn self_command(arg: &str) -> [u16; 600] {
    let mut out = [0u16; 600];
    let img = image_path();
    let mut n = 0;
    out[n] = b'"' as u16;
    n += 1;
    for &c in img {
        if n < 590 {
            out[n] = c;
            n += 1;
        }
    }
    out[n] = b'"' as u16;
    n += 1;
    out[n] = b' ' as u16;
    n += 1;
    for b in arg.bytes() {
        if n < 598 {
            out[n] = u16::from(b);
            n += 1;
        }
    }
    out
}

/// ntdef.h: `OBJECT_ATTRIBUTES` (48 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct NT_OA {
    length: u32,
    root: HANDLE,
    name: *const UnicodeString,
    attributes: u32,
    descriptor: *const c_void,
    qos: *const c_void,
}

/// A suspended child process of this very executable (never resumed): (process, thread) or nulls. It is created
/// outside any job the test runs in when breakaway is allowed (otherwise it inherits the test's job).
fn nt_spawn() -> (HANDLE, HANDLE) {
    for flags in [CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB, CREATE_SUSPENDED | CREATE_NO_WINDOW]
    {
        let mut cmd = self_command("--child");
        let mut si = STARTUPINFOW::new();
        let mut pi = PROCESS_INFORMATION::new();
        // SAFETY: NUL-terminated writable command line, valid structures.
        let ok = unsafe {
            CreateProcessW(
                core::ptr::null(),
                cmd.as_mut_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                FALSE,
                flags,
                core::ptr::null_mut(),
                core::ptr::null(),
                &mut si,
                &mut pi,
            )
        };
        if ok != 0 {
            return (pi.process, pi.thread);
        }
    }
    (core::ptr::null_mut(), core::ptr::null_mut())
}

/// End a child made by `nt_spawn` and close its handles.
fn nt_reap(process: HANDLE, thread: HANDLE) {
    // SAFETY: handles from CreateProcess; the child is terminated and waited for.
    unsafe {
        TerminateProcess(process, 0);
        WaitForSingleObject(process, 10_000);
        CloseHandle(thread);
        CloseHandle(process);
    }
}

/// A duplicate of `h` (in this process) with exactly `access`, or null.
fn nt_dup_access(h: HANDLE, access: u32) -> HANDLE {
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: DuplicateHandle with an explicit access mask (no DUPLICATE_SAME_ACCESS).
    unsafe { DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut out, access, FALSE, 0) };
    out
}

/// An unnamed job object with all access, or null.
fn nt_job() -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer; no object attributes.
    unsafe { NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, core::ptr::null()) };
    h
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn DuplicateHandle(
        src_process: HANDLE,
        src: HANDLE,
        dst_process: HANDLE,
        dst: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn TerminateProcess(process: HANDLE, code: u32) -> BOOL;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        proc_attrs: *const SECURITY_ATTRIBUTES,
        thread_attrs: *const SECURITY_ATTRIBUTES,
        inherit: BOOL,
        flags: u32,
        env: PVOID,
        dir: *const u16,
        si: *mut STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
    fn NtAssignProcessToJobObject(job: HANDLE, process: HANDLE) -> i32;
    fn NtIsProcessInJob(process: HANDLE, job: HANDLE) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let (p, th) = nt_spawn();
    if p.is_null() {
        say!(t, "child process could not be started");
        return;
    }
    let job = nt_job();
    // SAFETY: valid handles.
    let st = unsafe { NtAssignProcessToJobObject(job, p) };
    say!(t, "assign: {st:#010x}");
    for (name, access) in [
        ("PROCESS_ALL_ACCESS", PROCESS_ALL_ACCESS),
        ("PROCESS_QUERY_INFORMATION", PROCESS_QUERY_INFORMATION),
        ("PROCESS_QUERY_LIMITED_INFORMATION", PROCESS_QUERY_LIMITED_INFORMATION),
        ("SYNCHRONIZE", SYNCHRONIZE),
        ("PROCESS_TERMINATE", PROCESS_TERMINATE),
        ("PROCESS_SET_QUOTA", PROCESS_SET_QUOTA),
        ("access 0", 0),
    ] {
        let ph = nt_dup_access(p, access);
        // SAFETY: valid handles.
        let (a, b) = unsafe { (NtIsProcessInJob(ph, job), NtIsProcessInJob(ph, core::ptr::null_mut())) };
        say!(t, "process {name}: with the job {a:#010x}, with NULL job {b:#010x}");
        // SAFETY: closing the duplicate.
        unsafe { CloseHandle(ph) };
    }
    for (name, access) in [
        ("JOB_OBJECT_ALL_ACCESS", JOB_OBJECT_ALL_ACCESS),
        ("JOB_OBJECT_QUERY", JOB_OBJECT_QUERY),
        ("JOB_OBJECT_ASSIGN_PROCESS", JOB_OBJECT_ASSIGN_PROCESS),
        ("JOB_OBJECT_SET_ATTRIBUTES", JOB_OBJECT_SET_ATTRIBUTES),
        ("JOB_OBJECT_TERMINATE", JOB_OBJECT_TERMINATE),
        ("SYNCHRONIZE", SYNCHRONIZE),
        ("access 0", 0),
    ] {
        let jh = nt_dup_access(job, access);
        // SAFETY: valid handles.
        let a = unsafe { NtIsProcessInJob(p, jh) };
        say!(t, "job {name}: {a:#010x}");
        // SAFETY: closing the duplicate.
        unsafe { CloseHandle(jh) };
    }
    // SAFETY: handles of the right kinds, or deliberately wrong ones; an unnamed event closed afterwards.
    unsafe {
        let ev = CreateEventW(core::ptr::null(), TRUE, FALSE, core::ptr::null());
        say!(t, "NULL process: {:#010x}", NtIsProcessInJob(core::ptr::null_mut(), job));
        say!(t, "swapped (job, process): {:#010x}", NtIsProcessInJob(job, p));
        say!(t, "event as process: {:#010x}", NtIsProcessInJob(ev, job));
        say!(t, "event as job: {:#010x}", NtIsProcessInJob(p, ev));
        say!(t, "thread pseudo-handle as process: {:#010x}", NtIsProcessInJob(-2isize as HANDLE, job));
        CloseHandle(ev);
        say!(t, "closed process handle: {:#010x}", NtIsProcessInJob(ev, job));
        NtClose(job);
    }
    nt_reap(p, th);
}
