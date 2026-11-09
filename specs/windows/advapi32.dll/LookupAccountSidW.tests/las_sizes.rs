//! LookupAccountSidW buffer protocol: too small name or domain buffer (required sizes), zero
//! sizes, null buffers, invalid SIDs, a null SID.
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

const SECURITY_MAX_SID_SIZE: usize = 68;
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

/// A SID from its string form (up to 68 bytes), through ConvertStringSidToSidW.
fn sid_of(s: &str) -> SidBuf {
    let w = wide(s);
    let mut p: PSID = core::ptr::null_mut();
    let mut out = SidBuf::new();
    // SAFETY: `w` is NUL-terminated; `p` receives a LocalAlloc'ed SID, copied and freed below.
    unsafe {
        if ConvertStringSidToSidW(w.as_ptr(), &mut p) != 0 {
            let n = GetLengthSid(p) as usize;
            core::ptr::copy_nonoverlapping(p.cast::<u8>(), out.b.as_mut_ptr(), n.min(out.b.len()));
            LocalFree(p);
        }
    }
    out
}

/// A text for the units of a (maybe not NUL-terminated) UTF-16 buffer, ASCII only.
fn c2_units<const N: usize>(b: &[u16]) -> Buf<N> {
    let mut o = Buf::<N>::new();
    for &u in b {
        if u == 0 {
            break;
        }
        let _ = write!(o, "{}", if u < 0x80 { u as u8 as char } else { '?' });
    }
    o
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn LookupAccountSidW(
        system: *const u16,
        sid: *const c_void,
        name: *mut u16,
        cch_name: *mut u32,
        domain: *mut u16,
        cch_domain: *mut u32,
        kind: *mut u32,
    ) -> BOOL;
}

poly_wtest::main!(test);

fn run(
    t: &mut Test,
    label: &str,
    sid: *const c_void,
    cn_in: u32,
    cd_in: u32,
    null_name: bool,
    null_dom: bool,
    null_use: bool,
) {
    let mut name = [0xAAAAu16; 64];
    let mut dom = [0xAAAAu16; 64];
    let (mut cn, mut cd, mut kind) = (cn_in, cd_in, 0xDEADu32);
    clear_error();
    // SAFETY: the buffers are 64 units; the sizes passed never exceed that.
    let ok = unsafe {
        LookupAccountSidW(
            core::ptr::null(),
            sid,
            if null_name { core::ptr::null_mut() } else { name.as_mut_ptr() },
            &mut cn,
            if null_dom { core::ptr::null_mut() } else { dom.as_mut_ptr() },
            &mut cd,
            if null_use { core::ptr::null_mut() } else { &mut kind },
        )
    };
    let err = if ok == 0 { i64::from(gle()) } else { -1 };
    say!(
        t,
        "{label}: ok={ok} err={err} cch_name={cn} cch_domain={cd} use={kind:#x} name0={:#x} dom0={:#x} name={:?}",
        name[0],
        dom[0],
        if ok != 0 { c2_units::<40>(&name[..(cn as usize).min(64)]) } else { Buf::<40>::new() }.as_str()
    );
}

fn test(t: &mut Test) {
    let system = sid_of("S-1-5-18");
    run(t, "exact fit (SYSTEM: 6 + 12)", system.as_const(), 7, 13, false, false, false);
    run(t, "name buffer one too small", system.as_const(), 6, 13, false, false, false);
    run(t, "domain buffer one too small", system.as_const(), 7, 12, false, false, false);
    run(t, "both zero, buffers given", system.as_const(), 0, 0, false, false, false);
    run(t, "both zero, buffers null", system.as_const(), 0, 0, true, true, false);
    let world = sid_of("S-1-1-0");
    run(t, "Everyone (empty domain)", world.as_const(), 64, 64, false, false, false);
    run(t, "Everyone, domain size 0", world.as_const(), 64, 0, false, false, false);
    let bad = sid_of("S-1-5-21-1-2-3-4-5-6-7-8-9-10-11-12-13-14-15-16");
    run(t, "invalid SID (too many sub-authorities)", bad.as_const(), 64, 64, false, false, false);
    let mut raw = Raw::<64>::new();
    raw.b[0] = 2;
    raw.b[1] = 1;
    raw.b[7] = 5;
    run(t, "SID revision 2", raw.b.as_ptr().cast(), 64, 64, false, false, false);
    let mut raw = Raw::<64>::new();
    raw.b[0] = 1;
    raw.b[1] = 1;
    raw.b[7] = 77;
    raw.b[8] = 1;
    run(t, "SID with unknown authority 77", raw.b.as_ptr().cast(), 64, 64, false, false, false);
}
