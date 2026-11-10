//! Conditional ACE strings (XA, XD in the DACL, XU in the SACL): the bytes each condition becomes, conditions
//! that are refused and their error, a conditional type without a condition, a condition on a plain type, and
//! conditional types in the wrong section.
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

const SDDL_REVISION_1: u32 = 1;
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

const SD_SENTINEL: usize = 0x1234_5678_9ABC;
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

/// GetSecurityDescriptorDacl / Sacl.
struct SdGetAcl {
    ok: i32,
    present: BOOL,
    acl: *mut c_void,
    defaulted: BOOL,
    err: i64,
}

fn sd_get_dacl(sd: *const c_void) -> SdGetAcl {
    let mut acl = SD_SENTINEL as *mut c_void;
    let mut present: BOOL = 0x66;
    let mut d: BOOL = 0x77;
    clear_error();
    // SAFETY: `sd` is a descriptor; the out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut d) };
    SdGetAcl { ok, present, acl, defaulted: d, err: gle_if(ok) }
}

fn sd_get_sacl(sd: *const c_void) -> SdGetAcl {
    let mut acl = SD_SENTINEL as *mut c_void;
    let mut present: BOOL = 0x66;
    let mut d: BOOL = 0x77;
    clear_error();
    // SAFETY: `sd` is a descriptor; the out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorSacl(sd, &mut present, &mut acl, &mut d) };
    SdGetAcl { ok, present, acl, defaulted: d, err: gle_if(ok) }
}

/// An ACL's bytes (by its own size field, at most 1024) as hex on one line; "(null)" for NULL.
fn sd_acl_hex(t: &mut Test, label: &str, acl: *const c_void) {
    if acl.is_null() || acl as usize == SD_SENTINEL {
        sdsay!(t, "{label}: (none)");
        return;
    }
    // SAFETY: the caller passes a pointer to an ACL header.
    let head = unsafe { core::slice::from_raw_parts(acl as *const u8, 8) };
    let size = usize::from(u16::from_le_bytes([head[2], head[3]])).clamp(8, 1024);
    // SAFETY: the ACL is `size` bytes.
    let b = unsafe { core::slice::from_raw_parts(acl as *const u8, size) };
    sdsay!(t, "{label}: {}", Hex(b));
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
    fn GetSecurityDescriptorDacl(sd: *const c_void, present: *mut BOOL, dacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorSacl(sd: *const c_void, present: *mut BOOL, sacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
}

poly_wtest::main!(test);

fn show(t: &mut Test, s: &str) {
    let sd = sd_from(s);
    if sd.ok == 0 {
        sdsay!(t, "{s:?}: failed err={} size output={:#x}", sd.err, sd.size);
        return;
    }
    sdsay!(t, "{s:?}: ok size={}", sd.size);
    let d = sd_get_dacl(sd.ptr());
    if d.present != 0 {
        sd_acl_hex(t, "  dacl bytes", d.acl);
    }
    let sc = sd_get_sacl(sd.ptr());
    if sc.present != 0 {
        sd_acl_hex(t, "  sacl bytes", sc.acl);
    }
}

fn test(t: &mut Test) {
    sdsay!(t, "-- conditions that parse");
    for s in [
        "D:(XA;;FR;;;BU;(Member_of {SID(SY)}))",
        "D:(XA;;FR;;;BU;(Member_of {SID(SY),SID(BA)}))",
        "D:(XA;;FR;;;BU;(Member_of{SID(SY)}))",
        "D:(XA;;FR;;;BU;( Member_of  { SID(SY) , SID(BA) } ))",
        "D:(XA;;FR;;;BU;(Not_Member_of {SID(SY)}))",
        "D:(XA;;FR;;;BU;(Member_of_Any {SID(SY)}))",
        "D:(XA;;FR;;;BU;(Device_Member_of {SID(SY)}))",
        "D:(XA;;FR;;;BU;(Device_Member_of_Any {SID(SY)}))",
        "D:(XA;;FR;;;BU;(Not_Device_Member_of {SID(SY)}))",
        "D:(XA;;FR;;;BU;(member_of {sid(sy)}))",
        "D:(XA;;FR;;;BU;(Member_of {SID(S-1-5-32-544)}))",
        "D:(XA;;FR;;;BU;(Member_of SID(SY)))",
        "D:(XA;;FR;;;BU;(@User.Title == \"PM\"))",
        "D:(XA;;FR;;;BU;(@User.Title==\"PM\"))",
        "D:(XA;;FR;;;BU;(@Device.Level != 7))",
        "D:(XA;;FR;;;BU;(@Resource.Level < 7))",
        "D:(XA;;FR;;;BU;(@User.Level <= 0x10))",
        "D:(XA;;FR;;;BU;(@User.Level > -3))",
        "D:(XA;;FR;;;BU;(@User.Level >= 010))",
        "D:(XA;;FR;;;BU;(Title == \"PM\"))",
        "D:(XA;;FR;;;BU;(@User.Tags Contains \"a\"))",
        "D:(XA;;FR;;;BU;(@User.Tags Any_of {\"a\",\"b\"}))",
        "D:(XA;;FR;;;BU;(@User.Tags Not_Contains {1,2}))",
        "D:(XA;;FR;;;BU;(@User.Tags Not_Any_of {1}))",
        "D:(XA;;FR;;;BU;(Exists @User.Title))",
        "D:(XA;;FR;;;BU;(Not_Exists @Resource.Title))",
        "D:(XA;;FR;;;BU;(!(@User.Level == 1)))",
        "D:(XA;;FR;;;BU;(@User.A == 1 && @User.B == 2))",
        "D:(XA;;FR;;;BU;(@User.A == 1 || @User.B == 2))",
        "D:(XA;;FR;;;BU;((@User.A == 1) && (@User.B == 2 || @User.C == 3)))",
        "D:(XA;;FR;;;BU;(@User.A == @Resource.B))",
        "D:(XA;;FR;;;BU;(@User.Blob == #0102ff))",
        "D:(XA;CIOI;FR;;;BU;(Member_of {SID(SY)}))(A;;GA;;;SY)",
        "D:(XD;;FR;;;BU;(@User.Level == 1))",
        "S:(XU;FA;FR;;;BU;(@User.Level == 1))",
    ] {
        show(t, s);
    }
    sdsay!(t, "-- conditions that are refused");
    for s in [
        "D:(XA;;FR;;;BU;(Member_of))",
        "D:(XA;;FR;;;BU;())",
        "D:(XA;;FR;;;BU;( ))",
        "D:(XA;;FR;;;BU;(Member_of {}))",
        "D:(XA;;FR;;;BU;(Member_of {SID(ZZ)}))",
        "D:(XA;;FR;;;BU;(Member_of {SY}))",
        "D:(XA;;FR;;;BU;(Member_of {SID(SY)))",
        "D:(XA;;FR;;;BU;(Member_of {SID(SY)})",
        "D:(XA;;FR;;;BU;Member_of {SID(SY)})",
        "D:(XA;;FR;;;BU;(Nonsense {SID(SY)}))",
        "D:(XA;;FR;;;BU;(@User.Title ==))",
        "D:(XA;;FR;;;BU;(== 1))",
        "D:(XA;;FR;;;BU;(@User.Title = 1))",
        "D:(XA;;FR;;;BU;(@User.A == 1 &&))",
        "D:(XA;;FR;;;BU;(@Nowhere.A == 1))",
        "D:(XA;;FR;;;BU;(@User.A == 1) extra)",
        "D:(XA;;FR;;;BU;(@User.A == 1);)",
        "D:(XA;;FR;;;BU;(@User.A == 1);x)",
        "D:(XA;;FR;;;BU;(1 == 1))",
        "D:(XA;;FR;;;BU;(\"a\" == \"a\"))",
    ] {
        show(t, s);
    }
    sdsay!(t, "-- a conditional type without a condition, a condition on other types");
    for s in [
        "D:(XA;;FR;;;BU)",
        "D:(XD;;FR;;;BU)",
        "S:(XU;SA;FR;;;BU)",
        "D:(XA;;FR;;;BU;)",
        "D:(A;;FR;;;BU;(Member_of {SID(SY)}))",
        "D:(D;;FR;;;BU;(@User.A == 1))",
        "S:(AU;SA;FR;;;BU;(@User.A == 1))",
        "S:(ML;;NW;;;LW;(@User.A == 1))",
        "D:(OA;;CR;;;BU;(@User.A == 1))",
    ] {
        show(t, s);
    }
    sdsay!(t, "-- conditional and attribute types in the other section");
    for s in [
        "S:(XA;;FR;;;BU;(@User.A == 1))",
        "S:(XD;;FR;;;BU;(@User.A == 1))",
        "D:(XU;SA;FR;;;BU;(@User.A == 1))",
        "D:(XU;;FR;;;BU;(@User.A == 1))",
        "S:(XA;;FR;;;BU)",
        "D:(XU;SA;FR;;;BU)",
        "D:(RA;;;;;WD;(\"A\",TI,0,1))",
        "D:(SP;;;;;S-1-17-1)",
        "D:(ZA;;FR;;;BU;(@User.A == 1))",
        "S:(XA;;FR;;;BU;(Member_of))",
        "D:(XU;SA;FR;;;BU;(Member_of))",
    ] {
        show(t, s);
    }
}
