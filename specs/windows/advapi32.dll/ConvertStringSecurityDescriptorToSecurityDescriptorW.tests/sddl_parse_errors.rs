//! ConvertStringSecurityDescriptorToSecurityDescriptorW failures: syntax errors (error code and the out
//! parameters), revisions other than 1, a NULL string, a missing size pointer.
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
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for s in [
        "O",
        "O:",
        "O:BA ",
        " O:BA",
        "O:BA;",
        "O:BA G:SY",
        "O:BAO:SY",
        "G:SYG:BA",
        "D:D:",
        "D:(A;;GA;;;WD)D:(A;;GA;;;WD)",
        "S:S:",
        "X:BA",
        "o:ba",
        "O:BAG:",
        "G:",
        "D:(",
        "D:)",
        "D:(A",
        "D:(A;;GA;;;WD",
        "D:(A;;GA;;;WD))",
        "D:(A;;GA;;;WD)(",
        "D:((A;;GA;;;WD))",
        "O:BA\n",
        "O:BA\t",
        "\u{0}",
        " ",
        "  ",
        "BA",
        "O:BAD",
        "O:BAD:",
        "O:BAD:(",
        "O:BAD:()",
        "O:ZZ",
        "G:ZZ",
        "D:(A;;GA;;;ZZ)",
        "D:(A;;ZZ;;;WD)",
        "D:(A;ZZ;GA;;;WD)",
        "D:(ZZ;;GA;;;WD)",
        "T:BA",
        "L:BA",
        "O:BAL:BA",
        "O:BA:",
    ] {
        let sd = sd_from(s);
        if sd.ok != 0 {
            sdsay!(t, "{s:?}: ok size={}", sd.size);
        } else {
            sdsay!(t, "{s:?}: failed err={} size output={:#x}", sd.err, sd.size);
        }
    }
    for rev in [0u32, 1, 2, 3, 100, 0x8000_0001, 0xFFFF_FFFF] {
        let sd = sd_from_rev("O:BA", rev);
        sdsay!(t, "revision {rev:#x}: ok={} err={} size output={:#x}", sd.ok, sd.err, sd.size);
    }
    // Without a size pointer.
    let w = wide("O:BAG:SY");
    let mut p: PSECURITY_DESCRIPTOR = core::ptr::null_mut();
    clear_error();
    // SAFETY: the size pointer is documented as optional.
    let ok =
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), 1, &mut p, core::ptr::null_mut()) };
    sdsay!(t, "NULL size pointer: ok={ok} err={}", gle_if(ok));
    if !p.is_null() {
        // SAFETY: allocated by the call with LocalAlloc.
        unsafe { LocalFree(p) };
    }
}
