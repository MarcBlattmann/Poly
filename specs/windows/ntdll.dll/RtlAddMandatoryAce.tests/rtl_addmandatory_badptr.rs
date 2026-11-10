//! RtlAddMandatoryAce with a SID argument that is not a pointer to a SID: NULL and small integers, alone and
//! together with a wrong type, mask, flags or revision argument; addresses that cannot be read (reserved
//! pages, a kernel address, all ones); and SIDs with a wrong revision byte or count.
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

/// An ACL in an 8-aligned buffer.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct AclBuf {
    b: [u8; 1024],
}

impl AclBuf {
    const fn new() -> AclBuf {
        AclBuf { b: [0; 1024] }
    }
    fn p(&mut self) -> PACL {
        self.b.as_mut_ptr().cast()
    }
    /// The ACL's bytes by its own size field.
    fn bytes(&self) -> &[u8] {
        let n = usize::from(u16::from_le_bytes([self.b[2], self.b[3]]));
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

/// The little-endian u16 at `off`.
fn rtl_le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

/// An ACL made by `RtlCreateAcl` in a zeroed buffer.
fn rtl_acl(len: u32, rev: u32) -> AclBuf {
    let mut a = AclBuf::new();
    // SAFETY: the buffer has 1024 bytes; callers pass lengths up to 1024.
    unsafe { RtlCreateAcl(a.p(), len, rev) };
    a
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn VirtualAlloc(addr: PVOID, size: usize, kind: u32, protect: u32) -> PVOID;
    fn VirtualFree(addr: PVOID, size: usize, kind: u32) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlCreateAcl(acl: PACL, len: u32, revision: u32) -> i32;
    fn RtlAddMandatoryAce(acl: PACL, revision: u32, flags: u32, sid: *const c_void, ace_type: u32, mask: u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: reserves (does not commit) 64 KiB of address space: reading it faults.
    let reserved = unsafe { VirtualAlloc(core::ptr::null_mut(), 0x10000, 0x2000, 1) };
    for (what, p) in [
        ("reserved pages", reserved as usize),
        ("a kernel address", 0xFFFF_8000_0000_0000usize),
        ("all ones", usize::MAX),
    ] {
        let mut line = Buf::<300>::new();
        for (name, ty, mask) in [("alone", 0x11u32, 1u32), ("type 0x10", 0x10, 1), ("mask 0x10", 0x11, 0x10)] {
            let mut acl = rtl_acl(64, 4);
            // SAFETY: a valid ACL buffer; the SID address cannot be read, the call answers with a status (observed).
            let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, p as *const c_void, ty, mask) };
            let _ = write!(line, "{name}={:#010x}/n{} ", st as u32, rtl_le16(&acl.b, 4));
        }
        say!(t, "SID argument {what} (reserved={}): {}", !reserved.is_null(), line.as_str());
    }
    // SAFETY: releases the reservation.
    unsafe { VirtualFree(reserved, 0, 0x8000) };
    for sid in [0usize, 2, 8, 0x100, 0xFFFF] {
        let mut line = Buf::<300>::new();
        for (name, rev, flags, ty, mask, len) in [
            ("alone", 4u32, 0u32, 0x11u32, 1u32, 64u32),
            ("type 0x10", 4, 0, 0x10, 1, 64),
            ("type 0", 4, 0, 0, 1, 64),
            ("mask 0x10", 4, 0, 0x11, 0x10, 64),
            ("flags 0x40", 4, 0x40, 0x11, 1, 64),
            ("revision 5", 5, 0, 0x11, 1, 64),
            ("ACL of 8 bytes", 4, 0, 0x11, 1, 8),
        ] {
            let mut acl = rtl_acl(len, 4);
            // SAFETY: a valid ACL buffer; the integer in the SID position is not readable memory, the call
            // answers with a status (observed).
            let st = unsafe { RtlAddMandatoryAce(acl.p(), rev, flags, sid as *const c_void, ty, mask) };
            let _ = write!(line, "{name}={:#010x}/n{} ", st as u32, rtl_le16(&acl.b, 4));
        }
        say!(t, "SID argument {sid:#x}: {}", line.as_str());
    }
    for (name, revision, count) in [
        ("revision 0", 0u8, 1u8),
        ("revision 2", 2, 1),
        ("revision 255", 255, 1),
        ("count 15", 1, 15),
        ("count 255", 1, 255),
    ] {
        let mut big = Raw::<1100>::new();
        big.b[0] = revision;
        big.b[1] = count;
        big.b[7] = 16;
        let mut line = Buf::<200>::new();
        for (what, ty, mask) in [("alone", 0x11u32, 1u32), ("type 2", 2, 1), ("mask 8", 0x11, 8)] {
            let mut acl = rtl_acl(200, 4);
            // SAFETY: a valid ACL buffer; the SID buffer is larger than any SID.
            let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, big.p(), ty, mask) };
            let _ = write!(line, "{what}={:#010x}/n{}/size{} ", st as u32, rtl_le16(&acl.b, 4), rtl_le16(&acl.b, 10));
        }
        say!(t, "label SID with {name}: {}", line.as_str());
    }
}
