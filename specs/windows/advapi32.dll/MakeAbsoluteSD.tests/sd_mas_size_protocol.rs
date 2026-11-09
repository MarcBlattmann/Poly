//! MakeAbsoluteSD size handling: all buffers missing, each one too small, exact sizes; which length variables
//! are filled in on ERROR_INSUFFICIENT_BUFFER.
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

type PSID = *mut c_void;
type PVOID = *mut c_void;
type BOOL = i32;
type PACL = *mut c_void;
type PSECURITY_DESCRIPTOR = *mut c_void;
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

const SECURITY_MAX_SID_SIZE: usize = 68;
const SDDL_REVISION_1: u32 = 1;
/// A `SID_IDENTIFIER_AUTHORITY` plus up to 15 sub-authorities in a buffer.
#[repr(C)]
#[derive(Clone, Copy)]
struct SidBuf {
    b: [u8; SECURITY_MAX_SID_SIZE + 4],
}

impl SidBuf {
    const fn new() -> SidBuf {
        SidBuf { b: [0; SECURITY_MAX_SID_SIZE + 4] }
    }
    fn as_psid(&mut self) -> PSID {
        self.b.as_mut_ptr().cast()
    }
    fn as_const(&self) -> *const c_void {
        self.b.as_ptr().cast()
    }
    /// The bytes of the SID (its length from the header).
    fn bytes(&self) -> &[u8] {
        let n = 8 + 4 * usize::from(self.b[1]);
        &self.b[..n.min(self.b.len())]
    }
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

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// A pointer stored in a result buffer, as an offset from the buffer start
/// (so printed values do not depend on addresses); -1 for null, -2 outside.
fn rel(base: *const u8, len: usize, p: *const c_void) -> i64 {
    if p.is_null() {
        return -1;
    }
    let d = (p as usize).wrapping_sub(base as usize);
    if d < len { d as i64 } else { -2 }
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A self-relative descriptor made by ConvertStringSecurityDescriptorToSecurityDescriptorW.
struct SdLocal {
    p: PSECURITY_DESCRIPTOR,
    size: u32,
    ok: i32,
    err: i64,
}

impl SdLocal {
    fn ptr(&self) -> *const c_void {
        self.p as *const c_void
    }
    fn bytes(&self) -> &[u8] {
        if self.p.is_null() {
            return &[];
        }
        // SAFETY: the allocation is `size` bytes as reported by the conversion.
        unsafe { core::slice::from_raw_parts(self.p as *const u8, self.size as usize) }
    }
}

impl Drop for SdLocal {
    fn drop(&mut self) {
        if !self.p.is_null() {
            // SAFETY: allocated with LocalAlloc by the conversion.
            unsafe { LocalFree(self.p) };
        }
    }
}

fn sd_from_rev(s: &str, rev: u32) -> SdLocal {
    let w = wide(s);
    let mut p: PSECURITY_DESCRIPTOR = core::ptr::null_mut();
    let mut size = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `w` is NUL-terminated; both out pointers are valid.
    let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), rev, &mut p, &mut size) };
    let err = gle_if(ok);
    if ok == 0 {
        p = core::ptr::null_mut();
    }
    SdLocal { p, size, ok, err }
}

fn sd_from(s: &str) -> SdLocal {
    sd_from_rev(s, SDDL_REVISION_1)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        revision: u32,
        sd: *mut PSECURITY_DESCRIPTOR,
        size: *mut u32,
    ) -> BOOL;
    fn MakeAbsoluteSD(
        rel: *const c_void,
        abs: PSECURITY_DESCRIPTOR,
        abs_len: *mut u32,
        dacl: PACL,
        dacl_len: *mut u32,
        sacl: PACL,
        sacl_len: *mut u32,
        owner: PSID,
        owner_len: *mut u32,
        group: PSID,
        group_len: *mut u32,
    ) -> BOOL;
}

poly_wtest::main!(test);

#[derive(Clone, Copy)]
struct Lens {
    abs: u32,
    dacl: u32,
    sacl: u32,
    owner: u32,
    group: u32,
}

fn call(t: &mut Test, rel: *const c_void, label: &str, l: Lens, give: [bool; 5]) -> Lens {
    let mut abs = Raw::<64>::new();
    let mut dacl = Raw::<512>::new();
    let mut sacl = Raw::<512>::new();
    let mut owner = SidBuf::new();
    let mut group = SidBuf::new();
    let mut l = l;
    clear_error();
    let abs_p = if give[0] { abs.p() } else { core::ptr::null_mut() };
    let dacl_p = if give[1] { dacl.p() } else { core::ptr::null_mut() };
    let sacl_p = if give[2] { sacl.p() } else { core::ptr::null_mut() };
    let owner_p = if give[3] { owner.as_psid() } else { core::ptr::null_mut() };
    let group_p = if give[4] { group.as_psid() } else { core::ptr::null_mut() };
    // SAFETY: the buffers are larger than the sizes passed; unused ones are NULL with length 0 or too small.
    let ok = unsafe {
        MakeAbsoluteSD(
            rel,
            abs_p,
            &mut l.abs,
            dacl_p,
            &mut l.dacl,
            sacl_p,
            &mut l.sacl,
            owner_p,
            &mut l.owner,
            group_p,
            &mut l.group,
        )
    };
    sdsay!(
        t,
        "{label}: ok={ok} err={} abs={} dacl={} sacl={} owner={} group={}",
        gle_if(ok),
        l.abs,
        l.dacl,
        l.sacl,
        l.owner,
        l.group
    );
    l
}

fn test(t: &mut Test) {
    let rel = sd_from("O:BAG:SYD:(A;;GA;;;WD)S:(AU;SAFA;GA;;;WD)");
    let none = Lens { abs: 0, dacl: 0, sacl: 0, owner: 0, group: 0 };
    let need = call(t, rel.ptr(), "no buffers, all lengths 0", none, [false; 5]);
    call(
        t,
        rel.ptr(),
        "no buffers, lengths 0xAAAA",
        Lens { abs: 0xAAAA, dacl: 0xAAAA, sacl: 0xAAAA, owner: 0xAAAA, group: 0xAAAA },
        [false; 5],
    );
    call(t, rel.ptr(), "all buffers, all lengths 0", none, [true; 5]);
    call(t, rel.ptr(), "all given and exact", need, [true; 5]);
    call(
        t,
        rel.ptr(),
        "descriptor buffer NULL with length 0, others exact",
        Lens { abs: 0, ..need },
        [false, true, true, true, true],
    );
    call(
        t,
        rel.ptr(),
        "dacl buffer NULL with length 0, others exact",
        Lens { dacl: 0, ..need },
        [true, false, true, true, true],
    );
    call(
        t,
        rel.ptr(),
        "sacl buffer NULL with length 0, others exact",
        Lens { sacl: 0, ..need },
        [true, true, false, true, true],
    );
    call(
        t,
        rel.ptr(),
        "owner buffer NULL with length 0, others exact",
        Lens { owner: 0, ..need },
        [true, true, true, false, true],
    );
    call(
        t,
        rel.ptr(),
        "group buffer NULL with length 0, others exact",
        Lens { group: 0, ..need },
        [true, true, true, true, false],
    );
    call(t, rel.ptr(), "descriptor length one short", Lens { abs: need.abs - 1, ..need }, [true; 5]);
    call(t, rel.ptr(), "dacl length one short", Lens { dacl: need.dacl - 1, ..need }, [true; 5]);
    call(t, rel.ptr(), "sacl length one short", Lens { sacl: need.sacl - 1, ..need }, [true; 5]);
    call(t, rel.ptr(), "owner length one short", Lens { owner: need.owner - 1, ..need }, [true; 5]);
    call(t, rel.ptr(), "group length one short", Lens { group: need.group - 1, ..need }, [true; 5]);
    call(
        t,
        rel.ptr(),
        "all larger than needed",
        Lens { abs: 64, dacl: 512, sacl: 512, owner: 68, group: 68 },
        [true; 5],
    );
}
