//! QueryInformationJobObject(JobObjectBasicLimitInformation) reports PriorityClass NORMAL_PRIORITY_CLASS (0x20) for a job
//! that has no priority limit, a fresh one and one with only the active-process limit set, while NtQueryInformationJobObject
//! (the same class) reports 0; SchedulingClass is 5 in both. The set goes through kernel32 or ntdll with the same result.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

use core::ffi::c_void;
use poly_wtest::*;

type BOOL = i32;

#[repr(C)]
#[derive(Clone, Copy)]
struct BASIC {
    per_process: i64,
    per_job: i64,
    flags: u32,
    min_ws: usize,
    max_ws: usize,
    active: u32,
    affinity: usize,
    priority: u32,
    sched: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateJobObjectW(sa: *const c_void, name: *const u16) -> HANDLE;
    fn SetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, len: u32) -> BOOL;
    fn QueryInformationJobObject(job: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> BOOL;
    fn CloseHandle(h: HANDLE) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtSetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, len: u32) -> NTSTATUS;
    fn NtQueryInformationJobObject(job: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> NTSTATUS;
}

poly_wtest::main!(test);

fn zero() -> BASIC {
    // SAFETY: all-zero is a valid structure.
    unsafe { core::mem::zeroed() }
}

fn k_query(j: HANDLE, what: &str, t: &mut Test) {
    let mut b = zero();
    let mut ret = 0u32;
    // SAFETY: b is the 64 bytes of the class.
    let ok = unsafe { QueryInformationJobObject(j, 2, (&mut b as *mut BASIC).cast(), 64, &mut ret) };
    let mut s = [0u8; 120];
    let n = fmt(&mut s, what, "kernel32", ok as i64, b);
    t.line(core::str::from_utf8(&s[..n]).unwrap_or("?"));
}

fn n_query(j: HANDLE, what: &str, t: &mut Test) {
    let mut b = zero();
    let mut ret = 0u32;
    // SAFETY: b is the 64 bytes of the class.
    let st = unsafe { NtQueryInformationJobObject(j, 2, (&mut b as *mut BASIC).cast(), 64, &mut ret) };
    let mut s = [0u8; 120];
    let n = fmt(&mut s, what, "ntdll", st as i64, b);
    t.line(core::str::from_utf8(&s[..n]).unwrap_or("?"));
}

fn fmt(out: &mut [u8], what: &str, via: &str, r: i64, b: BASIC) -> usize {
    use core::fmt::Write;
    struct W<'a>(&'a mut [u8], usize);
    impl Write for W<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for &c in s.as_bytes() {
                if self.1 < self.0.len() {
                    self.0[self.1] = c;
                    self.1 += 1;
                }
            }
            Ok(())
        }
    }
    let mut w = W(out, 0);
    let _ = write!(
        w,
        "{what} read via {via}: r={r:#x} flags={:#x} active={} prio={:#x} sched={}",
        b.flags, b.active, b.priority, b.sched
    );
    w.1
}

fn test(t: &mut Test) {
    // job A: kernel32 set, kernel32 and ntdll read
    // SAFETY: a plain unnamed job.
    let a = unsafe { CreateJobObjectW(core::ptr::null(), core::ptr::null()) };
    k_query(a, "fresh job", t);
    let mut b = zero();
    b.flags = 8;
    b.active = 4;
    // SAFETY: 64 bytes.
    let ok = unsafe { SetInformationJobObject(a, 2, (&b as *const BASIC).cast(), 64) };
    t.value("set (kernel32) ok", ok as i64);
    k_query(a, "job after SetInformationJobObject(active limit 4)", t);
    n_query(a, "job after SetInformationJobObject(active limit 4)", t);
    // job B: ntdll set
    // SAFETY: a plain unnamed job.
    let j = unsafe { CreateJobObjectW(core::ptr::null(), core::ptr::null()) };
    // SAFETY: 64 bytes.
    let st = unsafe { NtSetInformationJobObject(j, 2, (&b as *const BASIC).cast(), 64) };
    t.value_hex("set (ntdll) status", st as u32 as u64);
    k_query(j, "job after NtSetInformationJobObject(active limit 4)", t);
    n_query(j, "job after NtSetInformationJobObject(active limit 4)", t);
    // job C: kernel32 set with a second query first (read before any set)
    // SAFETY: a plain unnamed job.
    let c = unsafe { CreateJobObjectW(core::ptr::null(), core::ptr::null()) };
    n_query(c, "fresh job, ntdll first", t);
    // SAFETY: 64 bytes.
    let ok = unsafe { SetInformationJobObject(c, 2, (&b as *const BASIC).cast(), 64) };
    t.value("set (kernel32) ok, third job", ok as i64);
    n_query(c, "job after SetInformationJobObject, ntdll read", t);
    // SAFETY: handles of this test.
    unsafe {
        CloseHandle(a);
        CloseHandle(j);
        CloseHandle(c);
    }
}
