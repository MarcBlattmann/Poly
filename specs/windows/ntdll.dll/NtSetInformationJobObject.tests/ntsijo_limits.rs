//! NtSetInformationJobObject(JobObjectBasicLimitInformation / JobObjectExtendedLimitInformation): which LimitFlags
//! and field values are accepted (priority classes, scheduling classes, affinity, working set, times, breakaway
//! flags), what reads back, the length rules (64 and 144 bytes), and JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE ending a
//! child when the last job handle closes.
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
const JOB_OBJECT_LIMIT_WORKINGSET: u32 = 0x1;
const JOB_OBJECT_LIMIT_PROCESS_TIME: u32 = 0x2;
const JOB_OBJECT_LIMIT_JOB_TIME: u32 = 0x4;
const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x8;
const JOB_OBJECT_LIMIT_AFFINITY: u32 = 0x10;
const JOB_OBJECT_LIMIT_PRIORITY_CLASS: u32 = 0x20;
const JOB_OBJECT_LIMIT_PRESERVE_JOB_TIME: u32 = 0x40;
const JOB_OBJECT_LIMIT_SCHEDULING_CLASS: u32 = 0x80;
const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x100;
const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x200;
const JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x400;
const JOB_OBJECT_LIMIT_BREAKAWAY_OK: u32 = 0x800;
const JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK: u32 = 0x1000;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
const JOB_OBJECT_LIMIT_SUBSET_AFFINITY: u32 = 0x4000;
const JobObjectBasicLimitInformation: u32 = 2;
const JobObjectExtendedLimitInformation: u32 = 9;
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

/// winnt.h: `JOBOBJECT_BASIC_LIMIT_INFORMATION` (64 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

/// winnt.h: `IO_COUNTERS`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IO_COUNTERS {
    read_ops: u64,
    write_ops: u64,
    other_ops: u64,
    read_bytes: u64,
    write_bytes: u64,
    other_bytes: u64,
}

/// winnt.h: `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` (144 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    basic: JOBOBJECT_BASIC_LIMIT_INFORMATION,
    io: IO_COUNTERS,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
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
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
    fn NtAssignProcessToJobObject(job: HANDLE, process: HANDLE) -> i32;
    fn NtSetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, len: u32) -> i32;
    fn NtQueryInformationJobObject(job: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn base() -> JOBOBJECT_BASIC_LIMIT_INFORMATION {
    JOBOBJECT_BASIC_LIMIT_INFORMATION {
        per_process_user_time_limit: 0,
        per_job_user_time_limit: 0,
        limit_flags: 0,
        minimum_working_set_size: 0,
        maximum_working_set_size: 0,
        active_process_limit: 0,
        affinity: 0,
        priority_class: 0,
        scheduling_class: 0,
    }
}

fn read_back(job: HANDLE) -> Buf<200> {
    let mut raw = Raw::<256>::new();
    let (st, _) = nt_jq(job, JobObjectBasicLimitInformation, &mut raw, 64);
    let mut b = Buf::<200>::new();
    if st < 0 {
        let _ = write!(b, "read back {st:#010x}");
    } else {
        let _ = write!(
            b,
            "read back flags={:#x} process_time={} job_time={} min_ws={} max_ws={} active={} affinity={} priority={:#x} scheduling={}",
            raw.u32_at(16),
            raw.u64_at(0),
            raw.u64_at(8),
            raw.u64_at(24),
            raw.u64_at(32),
            raw.u32_at(40),
            raw.u64_at(48),
            raw.u32_at(56),
            raw.u32_at(60)
        );
    }
    b
}

fn try_basic(t: &mut Test, label: &str, l: &JOBOBJECT_BASIC_LIMIT_INFORMATION) {
    let job = nt_job();
    // SAFETY: a valid 64-byte JOBOBJECT_BASIC_LIMIT_INFORMATION.
    let st = unsafe {
        NtSetInformationJobObject(
            job,
            JobObjectBasicLimitInformation,
            (l as *const JOBOBJECT_BASIC_LIMIT_INFORMATION).cast(),
            64,
        )
    };
    say!(t, "{label}: status={st:#010x}; {}", read_back(job).as_str());
    // SAFETY: closing the job.
    unsafe { NtClose(job) };
}

fn test(t: &mut Test) {
    let job = nt_job();
    say!(t, "fresh job: {}", read_back(job).as_str());
    // SAFETY: closing the job.
    unsafe { NtClose(job) };
    let mut l = base();
    try_basic(t, "flags 0", &l);
    for v in [0u32, 1, 5, 100] {
        l = base();
        l.limit_flags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        l.active_process_limit = v;
        let mut s = Buf::<60>::new();
        let _ = write!(s, "ACTIVE_PROCESS {v}");
        try_basic(t, s.as_str(), &l);
    }
    for v in [0u32, 1, 2, 3, 4, 5, 6, 7, 8, 0x20, 0x40, 0x4000] {
        l = base();
        l.limit_flags = JOB_OBJECT_LIMIT_PRIORITY_CLASS;
        l.priority_class = v;
        let mut s = Buf::<60>::new();
        let _ = write!(s, "PRIORITY_CLASS {v:#x}");
        try_basic(t, s.as_str(), &l);
    }
    for v in [0u32, 1, 5, 9, 10, 100] {
        l = base();
        l.limit_flags = JOB_OBJECT_LIMIT_SCHEDULING_CLASS;
        l.scheduling_class = v;
        let mut s = Buf::<60>::new();
        let _ = write!(s, "SCHEDULING_CLASS {v}");
        try_basic(t, s.as_str(), &l);
    }
    for v in [0usize, 1, 3, usize::MAX] {
        l = base();
        l.limit_flags = JOB_OBJECT_LIMIT_AFFINITY;
        l.affinity = v;
        let mut s = Buf::<60>::new();
        let _ = write!(s, "AFFINITY {v:#x}");
        try_basic(t, s.as_str(), &l);
    }
    l = base();
    l.limit_flags = JOB_OBJECT_LIMIT_PROCESS_TIME;
    l.per_process_user_time_limit = 100_000_000;
    try_basic(t, "PROCESS_TIME 10s", &l);
    l.per_process_user_time_limit = 0;
    try_basic(t, "PROCESS_TIME 0", &l);
    l.per_process_user_time_limit = -1;
    try_basic(t, "PROCESS_TIME -1", &l);
    l = base();
    l.limit_flags = JOB_OBJECT_LIMIT_JOB_TIME;
    l.per_job_user_time_limit = 100_000_000;
    try_basic(t, "JOB_TIME 10s", &l);
    l.limit_flags = JOB_OBJECT_LIMIT_JOB_TIME | JOB_OBJECT_LIMIT_PRESERVE_JOB_TIME;
    try_basic(t, "JOB_TIME + PRESERVE_JOB_TIME", &l);
    l = base();
    l.limit_flags = JOB_OBJECT_LIMIT_PRESERVE_JOB_TIME;
    try_basic(t, "PRESERVE_JOB_TIME alone", &l);
    l = base();
    l.limit_flags = JOB_OBJECT_LIMIT_PROCESS_TIME | JOB_OBJECT_LIMIT_JOB_TIME;
    l.per_process_user_time_limit = 1;
    l.per_job_user_time_limit = 2;
    try_basic(t, "PROCESS_TIME 1 + JOB_TIME 2", &l);
    for (label, min, max) in [
        ("1M..2M", 1usize << 20, 2usize << 20),
        ("min > max", 2 << 20, 1 << 20),
        ("0..0", 0, 0),
        ("equal", 1 << 20, 1 << 20),
        ("not page multiples", 12345, 67890),
    ] {
        l = base();
        l.limit_flags = JOB_OBJECT_LIMIT_WORKINGSET;
        l.minimum_working_set_size = min;
        l.maximum_working_set_size = max;
        let mut s = Buf::<60>::new();
        let _ = write!(s, "WORKINGSET {label}");
        try_basic(t, s.as_str(), &l);
    }
    for (label, flags) in [
        ("BREAKAWAY_OK alone", JOB_OBJECT_LIMIT_BREAKAWAY_OK),
        ("SILENT_BREAKAWAY_OK alone", JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK),
        ("DIE_ON_UNHANDLED_EXCEPTION alone", JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION),
        ("BREAKAWAY_OK|SILENT_BREAKAWAY_OK", JOB_OBJECT_LIMIT_BREAKAWAY_OK | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK),
        ("KILL_ON_JOB_CLOSE (extended structure only)", JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE),
        ("PROCESS_MEMORY (extended only)", JOB_OBJECT_LIMIT_PROCESS_MEMORY),
        ("JOB_MEMORY (extended only)", JOB_OBJECT_LIMIT_JOB_MEMORY),
        ("SUBSET_AFFINITY alone", JOB_OBJECT_LIMIT_SUBSET_AFFINITY),
        ("undefined 0x8000", 0x8000),
        ("undefined 0x10000", 0x1_0000),
        ("0xFFFFFFFF", 0xFFFF_FFFF),
    ] {
        l = base();
        l.limit_flags = flags;
        try_basic(t, label, &l);
    }
    // Lengths.
    let job = nt_job();
    let buf = Raw::<256>::new();
    for len in [0u32, 4, 63, 64, 65, 144, 145] {
        // SAFETY: the buffer has 256 readable bytes, at least `len`.
        let st = unsafe { NtSetInformationJobObject(job, JobObjectBasicLimitInformation, buf.b.as_ptr().cast(), len) };
        // SAFETY: as above.
        let st2 =
            unsafe { NtSetInformationJobObject(job, JobObjectExtendedLimitInformation, buf.b.as_ptr().cast(), len) };
        say!(t, "length {len}: basic class {st:#010x}, extended class {st2:#010x}");
    }
    // SAFETY: closing the job.
    unsafe { NtClose(job) };
    // The extended structure.
    for (label, flags, pm, jm) in [
        ("PROCESS_MEMORY 64M", JOB_OBJECT_LIMIT_PROCESS_MEMORY, 64usize << 20, 0usize),
        ("JOB_MEMORY 128M", JOB_OBJECT_LIMIT_JOB_MEMORY, 0, 128 << 20),
        ("both", JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_JOB_MEMORY, 32 << 20, 64 << 20),
        ("PROCESS_MEMORY 0", JOB_OBJECT_LIMIT_PROCESS_MEMORY, 0, 0),
        ("KILL_ON_JOB_CLOSE", JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, 0, 0),
        ("BREAKAWAY_OK", JOB_OBJECT_LIMIT_BREAKAWAY_OK, 0, 0),
        ("SILENT_BREAKAWAY_OK", JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, 0, 0),
        ("DIE_ON_UNHANDLED_EXCEPTION", JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, 0, 0),
        (
            "BREAKAWAY_OK|SILENT_BREAKAWAY_OK|DIE_ON_UNHANDLED_EXCEPTION|KILL_ON_JOB_CLOSE",
            JOB_OBJECT_LIMIT_BREAKAWAY_OK
                | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK
                | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
                | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            0,
            0,
        ),
        ("undefined 0x8000", 0x8000, 0, 0),
        ("SUBSET_AFFINITY alone", JOB_OBJECT_LIMIT_SUBSET_AFFINITY, 0, 0),
        ("memory limits without their flags", 0, 1 << 20, 1 << 21),
    ] {
        let j = nt_job();
        let mut e = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            basic: base(),
            io: IO_COUNTERS::default(),
            process_memory_limit: pm,
            job_memory_limit: jm,
            peak_process_memory_used: 0,
            peak_job_memory_used: 0,
        };
        e.basic.limit_flags = flags;
        // SAFETY: a valid 144-byte JOBOBJECT_EXTENDED_LIMIT_INFORMATION.
        let st = unsafe {
            NtSetInformationJobObject(
                j,
                JobObjectExtendedLimitInformation,
                (&e as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                144,
            )
        };
        let mut raw = Raw::<256>::new();
        let (qs, ret) = nt_jq(j, JobObjectExtendedLimitInformation, &mut raw, 144);
        say!(
            t,
            "extended {label}: status={st:#010x}; read back {qs:#010x} flags={:#x} process_memory={} job_memory={} (length {ret})",
            raw.u32_at(16),
            raw.u64_at(112),
            raw.u64_at(120)
        );
        // SAFETY: closing the job.
        unsafe { NtClose(j) };
    }
    // KILL_ON_JOB_CLOSE with a child.
    let (p, th) = nt_spawn();
    if !p.is_null() {
        let j = nt_job();
        let mut e = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            basic: base(),
            io: IO_COUNTERS::default(),
            process_memory_limit: 0,
            job_memory_limit: 0,
            peak_process_memory_used: 0,
            peak_job_memory_used: 0,
        };
        e.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: valid structure and handles.
        let (s1, s2) = unsafe {
            (
                NtSetInformationJobObject(
                    j,
                    JobObjectExtendedLimitInformation,
                    (&e as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    144,
                ),
                NtAssignProcessToJobObject(j, p),
            )
        };
        say!(t, "KILL_ON_JOB_CLOSE job: set {s1:#010x}, assign {s2:#010x}");
        // SAFETY: closing the only handle of the job.
        unsafe { NtClose(j) };
        let code = wait_exit(p);
        say!(t, "child after the job handle closed: exit code {code:#x}");
        nt_reap(p, th);
    }
}
