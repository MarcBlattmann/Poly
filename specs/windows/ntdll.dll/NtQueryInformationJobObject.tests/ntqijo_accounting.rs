//! NtQueryInformationJobObject with processes in the job: the basic accounting counts (total, active, terminated),
//! the process id list and its length rules with short buffers, the combined accounting structure, and the numbers
//! after NtTerminateJobObject ended the processes.
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
const JOB_OBJECT_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F;
const JobObjectBasicAccountingInformation: u32 = 1;
const JobObjectBasicProcessIdList: u32 = 3;
const JobObjectBasicAndIoAccountingInformation: u32 = 8;
const CREATE_SUSPENDED: u32 = 0x4;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const WAIT_OBJECT_0: u32 = 0;
/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

/// An 8-aligned scratch buffer for token information and other results.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Raw<const N: usize> {
    b: [u8; N],
}

impl<const N: usize> Raw<N> {
    const fn new() -> Raw<N> {
        Raw { b: [0; N] }
    }
    fn p(&mut self) -> PVOID {
        self.b.as_mut_ptr().cast()
    }
    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([self.b[off], self.b[off + 1], self.b[off + 2], self.b[off + 3]])
    }
    fn u16_at(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.b[off], self.b[off + 1]])
    }
    fn u64_at(&self, off: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.b[off..off + 8]);
        u64::from_le_bytes(a)
    }
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

/// A child's exit code after it ended (waits up to ten seconds), or -1.
fn wait_exit(process: HANDLE) -> i64 {
    let mut code = 0u32;
    // SAFETY: `process` is a process handle with SYNCHRONIZE and PROCESS_QUERY_*.
    unsafe {
        if WaitForSingleObject(process, 10_000) != WAIT_OBJECT_0 {
            return -1;
        }
        if GetExitCodeProcess(process, &mut code) == 0 {
            return -2;
        }
    }
    i64::from(code)
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

/// winnt.h: `JOBOBJECT_BASIC_PROCESS_ID_LIST` with room for `N` ids.
#[repr(C)]
#[derive(Clone, Copy)]
struct NT_JOB_PIDS<const N: usize> {
    assigned: u32,
    in_list: u32,
    ids: [usize; N],
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

/// An unnamed job object with all access, or null.
fn nt_job() -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer; no object attributes.
    unsafe { NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, core::ptr::null()) };
    h
}

/// NtQueryInformationJobObject into a buffer; (status, returned length).
fn nt_jq<const N: usize>(job: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationJobObject(job, class, buf.p(), len, &mut ret) };
    (st, ret)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetExitCodeProcess(process: HANDLE, code: *mut u32) -> BOOL;
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
    fn GetProcessId(process: HANDLE) -> u32;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
    fn NtAssignProcessToJobObject(job: HANDLE, process: HANDLE) -> i32;
    fn NtQueryInformationJobObject(job: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtTerminateJobObject(job: HANDLE, status: i32) -> i32;
}

poly_wtest::main!(test);

fn counts(t: &mut Test, label: &str, job: HANDLE) {
    let mut raw = Raw::<128>::new();
    let (st, ret) = nt_jq(job, JobObjectBasicAccountingInformation, &mut raw, 48);
    say!(
        t,
        "{label}: basic accounting {st:#010x} length={ret} total={} active={} terminated={}",
        raw.u32_at(36),
        raw.u32_at(40),
        raw.u32_at(44)
    );
    let mut raw2 = Raw::<256>::new();
    let (st, ret) = nt_jq(job, JobObjectBasicAndIoAccountingInformation, &mut raw2, 96);
    say!(
        t,
        "{label}: basic and IO accounting {st:#010x} length={ret} total={} active={} terminated={}",
        raw2.u32_at(36),
        raw2.u32_at(40),
        raw2.u32_at(44)
    );
}

fn test(t: &mut Test) {
    let job = nt_job();
    counts(t, "empty job", job);
    let (a, ath) = nt_spawn();
    let (b, bth) = nt_spawn();
    if a.is_null() || b.is_null() {
        say!(t, "children could not be started");
        return;
    }
    // SAFETY: valid handles.
    let pa = unsafe { GetProcessId(a) } as usize;
    // SAFETY: valid handles.
    let pb = unsafe { GetProcessId(b) } as usize;
    // SAFETY: valid handles.
    let (s1, s2) = unsafe { (NtAssignProcessToJobObject(job, a), NtAssignProcessToJobObject(job, b)) };
    say!(t, "assign two children: {s1:#010x} {s2:#010x}");
    counts(t, "two children", job);
    for len in [0u32, 7, 8, 9, 15, 16, 17, 24, 25, 32, 40, 64] {
        let mut ids = NT_JOB_PIDS::<4> { assigned: 0xEE, in_list: 0xEE, ids: [0; 4] };
        let mut ret = 0xAAAA_AAAAu32;
        // SAFETY: the structure is 40 bytes; `len` is at most 64 so the kernel is told 64 only when larger (see below).
        let l = len.min(40);
        let st = unsafe {
            NtQueryInformationJobObject(
                job,
                JobObjectBasicProcessIdList,
                (&mut ids as *mut NT_JOB_PIDS<4>).cast(),
                l,
                &mut ret,
            )
        };
        let both = ids.ids[..ids.in_list.min(4) as usize].contains(&pa)
            && ids.ids[..ids.in_list.min(4) as usize].contains(&pb);
        say!(
            t,
            "process id list, length {l}: status={st:#010x} returned={} assigned={:#x} in_list={:#x} contains_both_children={both}",
            if ret == 0xAAAA_AAAA { -1 } else { i64::from(ret) },
            ids.assigned,
            ids.in_list
        );
    }
    // SAFETY: valid job handle.
    let st = unsafe { NtTerminateJobObject(job, 0x77) };
    say!(t, "terminate: {st:#010x}; exit codes {:#x} {:#x}", wait_exit(a), wait_exit(b));
    counts(t, "after terminating", job);
    let mut ids = NT_JOB_PIDS::<4> { assigned: 0xEE, in_list: 0xEE, ids: [0; 4] };
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: the structure is 40 bytes.
    let st = unsafe {
        NtQueryInformationJobObject(
            job,
            JobObjectBasicProcessIdList,
            (&mut ids as *mut NT_JOB_PIDS<4>).cast(),
            40,
            &mut ret,
        )
    };
    say!(
        t,
        "process id list after terminating: status={st:#010x} returned={ret} assigned={} in_list={}",
        ids.assigned,
        ids.in_list
    );
    // SAFETY: closing the job.
    unsafe { NtClose(job) };
    nt_reap(a, ath);
    nt_reap(b, bth);
}
