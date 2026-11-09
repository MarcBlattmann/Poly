//! NtSetInformationJobObject other classes: UI restrictions (valid and undefined bits), end-of-job-time action,
//! every class 0 to 35 with a zeroed buffer of each length 0 to 300 (the lengths that are not rejected as a
//! mismatch and what they give; the silo and container classes 36 and up are not probed), the rights of the handle,
//! NULL and wrong-type handles and information pointers.
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
const JOB_OBJECT_SET_SECURITY_ATTRIBUTES: u32 = 0x10;
const JOB_OBJECT_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F;
const JobObjectBasicUIRestrictions: u32 = 4;
const JobObjectEndOfJobTimeInformation: u32 = 6;
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
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
    fn NtSetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, len: u32) -> i32;
    fn NtQueryInformationJobObject(job: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn set(job: HANDLE, class: u32, len: u32) -> i32 {
    let buf = Raw::<512>::new();
    // SAFETY: the buffer has 512 readable bytes, at least `len` (callers pass at most 300).
    unsafe { NtSetInformationJobObject(job, class, buf.b.as_ptr().cast(), len) }
}

fn test(t: &mut Test) {
    for v in [0u32, 1, 0x3F, 0xFF, 0x100, 0x1FF, 0x8000_0000] {
        let job = nt_job();
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS { ui_restrictions_class: v };
        // SAFETY: a valid 4-byte structure.
        let st = unsafe {
            NtSetInformationJobObject(
                job,
                JobObjectBasicUIRestrictions,
                (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
                4,
            )
        };
        let mut raw = Raw::<64>::new();
        let (qs, ret) = nt_jq(job, JobObjectBasicUIRestrictions, &mut raw, 4);
        say!(
            t,
            "UI restrictions {v:#x}: status={st:#010x}; read back {qs:#010x} value={:#x} length={ret}",
            raw.u32_at(0)
        );
        // SAFETY: closing the job.
        unsafe { NtClose(job) };
    }
    for action in [0u32, 1, 2, 3] {
        let job = nt_job();
        let v = [action];
        // SAFETY: a valid 4-byte structure.
        let st = unsafe { NtSetInformationJobObject(job, JobObjectEndOfJobTimeInformation, v.as_ptr().cast(), 4) };
        let mut raw = Raw::<64>::new();
        let (qs, _) = nt_jq(job, JobObjectEndOfJobTimeInformation, &mut raw, 4);
        say!(t, "end-of-job-time action {action}: status={st:#010x}; read back {qs:#010x} value={}", raw.u32_at(0));
        // SAFETY: closing the job.
        unsafe { NtClose(job) };
    }
    for class in 0..=35u32 {
        let mut line = Buf::<300>::new();
        let mut runs = 0;
        let mut cur: Option<(u32, u32, i32)> = None;
        for len in 0..=301u32 {
            let job = nt_job();
            let st = if len <= 300 { set(job, class, len) } else { 0xC000_0004u32 as i32 };
            // SAFETY: closing the job.
            unsafe { NtClose(job) };
            let mism = st == 0xC000_0004u32 as i32;
            if let Some((s, e, cs)) = cur {
                if !mism && cs == st && e + 1 == len {
                    cur = Some((s, len, cs));
                    continue;
                }
                if runs < 5 {
                    if s == e {
                        let _ = write!(line, "{s}:{cs:#x} ");
                    } else {
                        let _ = write!(line, "{s}..{e}:{cs:#x} ");
                    }
                }
                runs += 1;
                cur = None;
            }
            if !mism {
                cur = Some((len, len, st));
            }
        }
        if runs == 0 {
            say!(t, "class {class}: every length 0..=300 is STATUS_INFO_LENGTH_MISMATCH");
        } else {
            say!(t, "class {class}: lengths not rejected as a mismatch ({runs} runs): {}", line.as_str());
        }
    }
    let job = nt_job();
    for (name, access) in [
        ("JOB_OBJECT_SET_ATTRIBUTES", JOB_OBJECT_SET_ATTRIBUTES),
        ("JOB_OBJECT_QUERY", JOB_OBJECT_QUERY),
        ("JOB_OBJECT_ASSIGN_PROCESS", JOB_OBJECT_ASSIGN_PROCESS),
        ("JOB_OBJECT_SET_SECURITY_ATTRIBUTES", JOB_OBJECT_SET_SECURITY_ATTRIBUTES),
        ("JOB_OBJECT_TERMINATE", JOB_OBJECT_TERMINATE),
        ("access 0", 0),
    ] {
        let h = nt_dup_access(job, access);
        let mut raw = Raw::<64>::new();
        let (qs, _) = nt_jq(h, JobObjectBasicUIRestrictions, &mut raw, 4);
        say!(
            t,
            "handle with only {name}: set UI restrictions {:#010x}, query UI restrictions {qs:#010x}, set end-of-job-time {:#010x}",
            set(h, JobObjectBasicUIRestrictions, 4),
            set(h, JobObjectEndOfJobTimeInformation, 4)
        );
        // SAFETY: closing the duplicate.
        unsafe { CloseHandle(h) };
    }
    // SAFETY: an unnamed event closed afterwards; NULL information pointers.
    unsafe {
        let ev = CreateEventW(core::ptr::null(), TRUE, FALSE, core::ptr::null());
        say!(t, "NULL handle: {:#010x}", set(core::ptr::null_mut(), JobObjectBasicUIRestrictions, 4));
        say!(t, "process pseudo-handle: {:#010x}", set(NT_CURRENT_PROCESS, JobObjectBasicUIRestrictions, 4));
        say!(t, "event handle: {:#010x}", set(ev, JobObjectBasicUIRestrictions, 4));
        CloseHandle(ev);
        say!(t, "closed handle: {:#010x}", set(ev, JobObjectBasicUIRestrictions, 4));
        say!(
            t,
            "NULL information, length 0: {:#010x}",
            NtSetInformationJobObject(job, JobObjectBasicUIRestrictions, core::ptr::null(), 0)
        );
        say!(
            t,
            "NULL information, length 4: {:#010x}",
            NtSetInformationJobObject(job, JobObjectBasicUIRestrictions, core::ptr::null(), 4)
        );
        NtClose(job);
    }
}
