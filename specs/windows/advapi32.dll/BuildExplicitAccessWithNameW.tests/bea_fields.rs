//! BuildExplicitAccessWithNameW: the fields it stores for each access mode and inheritance value
//! (including values outside the enumerations), and the trustee it builds from a name.
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

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// The units of a NUL-terminated UTF-16 string.
fn wide_len(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// A UTF-16 string pointer as text (ASCII only; others as `?`).
fn wide_text<const N: usize>(p: *const u16) -> Buf<N> {
    let mut b = Buf::<N>::new();
    if p.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    let n = wide_len(p);
    for i in 0..n {
        // SAFETY: i < n, inside the string.
        let c = unsafe { *p.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

/// The SID in the text form of `ConvertSidToStringSidW`, or an error marker.
fn sid_text(sid: *const c_void) -> Buf<200> {
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is a valid SID or the call fails; `out` is a valid out pointer.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut out) };
    if ok == 0 {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "<invalid sid>");
        return b;
    }
    let b = wide_text::<200>(out);
    // SAFETY: the string was allocated by the call with LocalAlloc.
    unsafe { LocalFree(out.cast()) };
    b
}

/// The bytes of the little-endian u32 at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// The text of a SID with the machine-dependent parts masked: S-1-5-21-a-b-c-RID prints as
/// S-1-5-21-<machine>-RID and a logon-session SID S-1-5-5-x-y as S-1-5-5-<logon>.
fn mask_sid(sid: *const c_void) -> Buf<200> {
    let full = sid_text(sid);
    let s = full.as_str();
    let mut b = Buf::<200>::new();
    if s.starts_with("S-1-5-21-") && s.matches('-').count() >= 7 {
        let rid = s.rsplit('-').next().unwrap_or("?");
        if rid.parse::<u32>().map_or(false, |r| r >= 1000) {
            let _ = write!(b, "S-1-5-21-<machine>-<account>");
        } else {
            let _ = write!(b, "S-1-5-21-<machine>-{rid}");
        }
    } else if s.starts_with("S-1-5-5-") {
        let _ = write!(b, "S-1-5-5-<logon>");
    } else {
        let _ = write!(b, "{s}");
    }
    b
}

const NOT_USED_ACCESS: u32 = 0;
const GRANT_ACCESS: u32 = 1;
const SET_ACCESS: u32 = 2;
const DENY_ACCESS: u32 = 3;
const REVOKE_ACCESS: u32 = 4;
const SET_AUDIT_SUCCESS: u32 = 5;
const SET_AUDIT_FAILURE: u32 = 6;
const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_NAME: u32 = 1;
const SUB_OBJECTS_ONLY_INHERIT: u32 = 1;
const SUB_CONTAINERS_ONLY_INHERIT: u32 = 2;
const SUB_CONTAINERS_AND_OBJECTS_INHERIT: u32 = 3;
const INHERIT_NO_PROPAGATE: u32 = 4;
const INHERIT_ONLY: u32 = 8;
/// accctrl.h `TRUSTEE_W` (32 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct TRUSTEE_W {
    multiple: *mut c_void,
    multiple_op: u32,
    form: u32,
    kind: u32,
    name: *mut u16,
}

/// accctrl.h `EXPLICIT_ACCESS_W` (48 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct EXPLICIT_ACCESS_W {
    access: u32,
    mode: u32,
    inherit: u32,
    trustee: TRUSTEE_W,
}

impl EXPLICIT_ACCESS_W {
    fn filled(byte: u8) -> EXPLICIT_ACCESS_W {
        let mut e = EXPLICIT_ACCESS_W {
            access: 0,
            mode: 0,
            inherit: 0,
            trustee: TRUSTEE_W {
                multiple: core::ptr::null_mut(),
                multiple_op: 0,
                form: 0,
                kind: 0,
                name: core::ptr::null_mut(),
            },
        };
        // SAFETY: a plain-old-data structure; every byte is overwritten.
        unsafe {
            core::ptr::write_bytes(
                &mut e as *mut EXPLICIT_ACCESS_W as *mut u8,
                byte,
                core::mem::size_of::<EXPLICIT_ACCESS_W>(),
            )
        };
        e
    }
    /// An entry for a SID trustee, built by hand (no API call): form SID, type `kind`.
    fn for_sid(sid: PSID, access: u32, mode: u32, inherit: u32, kind: u32) -> EXPLICIT_ACCESS_W {
        EXPLICIT_ACCESS_W {
            access,
            mode,
            inherit,
            trustee: TRUSTEE_W {
                multiple: core::ptr::null_mut(),
                multiple_op: 0,
                form: TRUSTEE_IS_SID,
                kind,
                name: sid.cast(),
            },
        }
    }
}

/// A SID text with the machine- and user-specific parts masked (see `mask_sid`), or "(null)".
fn c2_sid(p: *const c_void) -> Buf<200> {
    if p.is_null() {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "(null)");
        return b;
    }
    mask_sid(p)
}

/// One `EXPLICIT_ACCESS_W` as text; the trustee name/SID pointer as text when the form says so.
fn c2_ea(t: &mut Test, label: &str, e: &EXPLICIT_ACCESS_W) {
    let name: Buf<120> = if e.trustee.name.is_null() {
        let mut b = Buf::<120>::new();
        let _ = write!(b, "(null)");
        b
    } else if e.trustee.form == TRUSTEE_IS_SID {
        let s = c2_sid(e.trustee.name.cast());
        let mut b = Buf::<120>::new();
        let _ = write!(b, "sid {}", s.as_str());
        b
    } else if e.trustee.form == TRUSTEE_IS_NAME {
        let mut b = Buf::<120>::new();
        let _ = write!(b, "name {}", wide_text::<100>(e.trustee.name).as_str());
        b
    } else {
        let mut b = Buf::<120>::new();
        let _ = write!(b, "ptr");
        b
    };
    say!(
        t,
        "{label}: access={:#x} mode={} inherit={:#x} trustee: multiple_null={} op={} form={} type={} {}",
        e.access,
        e.mode,
        e.inherit,
        e.trustee.multiple.is_null(),
        e.trustee.multiple_op,
        e.trustee.form,
        e.trustee.kind,
        name.as_str()
    );
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn BuildExplicitAccessWithNameW(e: *mut EXPLICIT_ACCESS_W, name: *mut u16, access: u32, mode: u32, inherit: u32);
}

poly_wtest::main!(test);

fn raw_pad(e: &EXPLICIT_ACCESS_W) -> (u32, u32) {
    // SAFETY: reading the bytes of a plain-old-data structure of 48 bytes.
    let b = unsafe { core::slice::from_raw_parts(e as *const EXPLICIT_ACCESS_W as *const u8, 48) };
    (le32(b, 12), le32(b, 36))
}

fn test(t: &mut Test) {
    let mut name = wide("Everyone");
    for (mode, inh) in [
        (NOT_USED_ACCESS, 0),
        (GRANT_ACCESS, 0),
        (SET_ACCESS, SUB_CONTAINERS_AND_OBJECTS_INHERIT),
        (DENY_ACCESS, SUB_OBJECTS_ONLY_INHERIT),
        (REVOKE_ACCESS, SUB_CONTAINERS_ONLY_INHERIT),
        (SET_AUDIT_SUCCESS, INHERIT_NO_PROPAGATE),
        (SET_AUDIT_FAILURE, INHERIT_ONLY),
        (7, 0),
        (0xFFFF_FFFF, 0xFFFF_FFFF),
    ] {
        let mut e = EXPLICIT_ACCESS_W::filled(0xAA);
        // SAFETY: valid pointers; the name is NUL-terminated and outlives the structure.
        unsafe { BuildExplicitAccessWithNameW(&mut e, name.as_mut_ptr(), 0x1F01FF, mode, inh) };
        c2_ea(t, alloc_label(mode, inh).as_str(), &e);
        let (p1, p2) = raw_pad(&e);
        say!(
            t,
            "  name pointer is the argument: {}; padding words {:#x} {:#x}",
            e.trustee.name == name.as_mut_ptr(),
            p1,
            p2
        );
    }
}

fn alloc_label(mode: u32, inh: u32) -> Buf<60> {
    let mut b = Buf::<60>::new();
    let _ = write!(b, "mode {mode:#x} inherit {inh:#x}");
    b
}
