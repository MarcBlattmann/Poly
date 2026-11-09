//! NtQueryInformationJobObject on a fresh job: the bytes (first 64) and length of each class that answers at its
//! exact length (found by scanning the lengths), which are all zero for a job nothing was set on except the default
//! scheduling class, and the same classes after limits were set.
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

/// The bytes as lowercase hex, no separators.
struct Hex<'a>(&'a [u8]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const JOB_OBJECT_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F;
const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x8;
const JOB_OBJECT_LIMIT_PRIORITY_CLASS: u32 = 0x20;
const JOB_OBJECT_LIMIT_SCHEDULING_CLASS: u32 = 0x80;
const JOB_OBJECT_UILIMIT_HANDLES: u32 = 0x1;
const JOB_OBJECT_UILIMIT_DESKTOP: u32 = 0x40;
const JobObjectBasicAccountingInformation: u32 = 1;
const JobObjectBasicLimitInformation: u32 = 2;
const JobObjectBasicProcessIdList: u32 = 3;
const JobObjectBasicUIRestrictions: u32 = 4;
const JobObjectSecurityLimitInformation: u32 = 5;
const JobObjectEndOfJobTimeInformation: u32 = 6;
const JobObjectAssociateCompletionPortInformation: u32 = 7;
const JobObjectBasicAndIoAccountingInformation: u32 = 8;
const JobObjectExtendedLimitInformation: u32 = 9;
const JobObjectGroupInformation: u32 = 11;
const JobObjectNotificationLimitInformation: u32 = 12;
const JobObjectLimitViolationInformation: u32 = 13;
const JobObjectGroupInformationEx: u32 = 14;
const JobObjectCpuRateControlInformation: u32 = 15;
const JobObjectNetRateControlInformation: u32 = 32;
const JobObjectNotificationLimitInformation2: u32 = 34;
const JobObjectLimitViolationInformation2: u32 = 35;
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

/// winnt.h: `JOBOBJECT_BASIC_UI_RESTRICTIONS`.
#[repr(C)]
#[derive(Clone, Copy)]
struct JOBOBJECT_BASIC_UI_RESTRICTIONS {
    ui_restrictions_class: u32,
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

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
    fn NtSetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, len: u32) -> i32;
    fn NtQueryInformationJobObject(job: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn first_len(job: HANDLE, class: u32) -> u32 {
    for len in 0..=300u32 {
        let mut b = Raw::<512>::new();
        let (st, _) = nt_jq(job, class, &mut b, len);
        if st != 0xC000_0004u32 as i32 && st != 0xC000_0003u32 as i32 {
            return len;
        }
    }
    u32::MAX
}

fn dump(t: &mut Test, label: &str, job: HANDLE, class: u32) {
    let len = first_len(job, class);
    if len == u32::MAX {
        say!(t, "{label}: no length up to 300 is accepted");
        return;
    }
    let mut raw = Raw::<512>::new();
    let (st, ret) = nt_jq(job, class, &mut raw, len);
    let n = (ret as usize).min(64);
    say!(
        t,
        "{label}: first accepted length {len}: status={st:#010x} returned={ret} first {n} bytes={}",
        Hex(&raw.b[..n])
    );
}

fn test(t: &mut Test) {
    let job = nt_job();
    for (name, class) in [
        ("basic accounting (1)", JobObjectBasicAccountingInformation),
        ("basic limit (2)", JobObjectBasicLimitInformation),
        ("basic process id list (3)", JobObjectBasicProcessIdList),
        ("basic UI restrictions (4)", JobObjectBasicUIRestrictions),
        ("security limit (5)", JobObjectSecurityLimitInformation),
        ("end of job time (6)", JobObjectEndOfJobTimeInformation),
        ("associate completion port (7)", JobObjectAssociateCompletionPortInformation),
        ("basic and IO accounting (8)", JobObjectBasicAndIoAccountingInformation),
        ("extended limit (9)", JobObjectExtendedLimitInformation),
        ("group information (11)", JobObjectGroupInformation),
        ("notification limit (12)", JobObjectNotificationLimitInformation),
        ("limit violation (13)", JobObjectLimitViolationInformation),
        ("group information ex (14)", JobObjectGroupInformationEx),
        ("cpu rate control (15)", JobObjectCpuRateControlInformation),
        ("net rate control (32)", JobObjectNetRateControlInformation),
        ("notification limit 2 (34)", JobObjectNotificationLimitInformation2),
        ("limit violation 2 (35)", JobObjectLimitViolationInformation2),
    ] {
        dump(t, name, job, class);
    }
    let mut l = JOBOBJECT_BASIC_LIMIT_INFORMATION {
        per_process_user_time_limit: 0,
        per_job_user_time_limit: 0,
        limit_flags: JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_PRIORITY_CLASS
            | JOB_OBJECT_LIMIT_SCHEDULING_CLASS,
        minimum_working_set_size: 0,
        maximum_working_set_size: 0,
        active_process_limit: 3,
        affinity: 0,
        priority_class: 3,
        scheduling_class: 4,
    };
    // SAFETY: a valid 64-byte structure.
    let st = unsafe {
        NtSetInformationJobObject(
            job,
            JobObjectBasicLimitInformation,
            (&l as *const JOBOBJECT_BASIC_LIMIT_INFORMATION).cast(),
            64,
        )
    };
    say!(t, "set active=3 priority=3 scheduling=4: {st:#010x}");
    dump(t, "basic limit after", job, JobObjectBasicLimitInformation);
    dump(t, "extended limit after", job, JobObjectExtendedLimitInformation);
    l.limit_flags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    l.active_process_limit = 9;
    // SAFETY: a valid 64-byte structure.
    let st = unsafe {
        NtSetInformationJobObject(
            job,
            JobObjectBasicLimitInformation,
            (&l as *const JOBOBJECT_BASIC_LIMIT_INFORMATION).cast(),
            64,
        )
    };
    say!(t, "set active=9 only: {st:#010x}");
    dump(t, "basic limit after", job, JobObjectBasicLimitInformation);
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        ui_restrictions_class: JOB_OBJECT_UILIMIT_HANDLES | JOB_OBJECT_UILIMIT_DESKTOP,
    };
    // SAFETY: a valid 4-byte structure.
    let st = unsafe {
        NtSetInformationJobObject(
            job,
            JobObjectBasicUIRestrictions,
            (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
            4,
        )
    };
    say!(t, "set UI restrictions HANDLES|DESKTOP: {st:#010x}");
    dump(t, "UI restrictions after", job, JobObjectBasicUIRestrictions);
    // SAFETY: closing the job.
    unsafe { NtClose(job) };
}
