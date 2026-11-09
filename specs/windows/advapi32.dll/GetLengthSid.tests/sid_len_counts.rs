//! GetLengthSid: 8 + 4 times the sub-authority count, for every count 0 to 15; the
//! lengths of the well-known SIDs.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PSID = *mut c_void;
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

const SECURITY_NT_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 5];
const SECURITY_MAX_SID_SIZE: usize = 68;
const WinNullSid: u32 = 0;
const WinWorldSid: u32 = 1;
const WinNtAuthoritySid: u32 = 7;
const WinLocalSystemSid: u32 = 22;
const WinBuiltinAdministratorsSid: u32 = 26;
const WinHighLabelSid: u32 = 68;
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

/// `ConvertSidToStringSidW` of a well-known SID built by `CreateWellKnownSid`.
fn well_known(kind: u32) -> SidBuf {
    let mut sid = SidBuf::new();
    let mut len = SECURITY_MAX_SID_SIZE as u32;
    // SAFETY: `sid` has room for the largest SID; `len` is its size.
    unsafe { CreateWellKnownSid(kind, core::ptr::null(), sid.as_psid(), &mut len) };
    sid
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn InitializeSid(sid: PSID, authority: *const [u8; 6], count: u8) -> BOOL;
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn CreateWellKnownSid(kind: u32, domain: *const c_void, sid: PSID, len: *mut u32) -> BOOL;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut line = Buf::<300>::new();
    for count in 0..=15u8 {
        let mut sid = SidBuf::new();
        // SAFETY: `sid` can hold 15 sub-authorities.
        let n = unsafe {
            InitializeSid(sid.as_psid(), &SECURITY_NT_AUTHORITY, count);
            GetLengthSid(sid.as_const())
        };
        let _ = write!(line, "{n} ");
    }
    t.line(line.as_str());
    for kind in
        [WinNullSid, WinWorldSid, WinNtAuthoritySid, WinLocalSystemSid, WinBuiltinAdministratorsSid, WinHighLabelSid]
    {
        let sid = well_known(kind);
        // SAFETY: a valid SID.
        say!(t, "kind {kind}: {}", unsafe { GetLengthSid(sid.as_const()) });
    }
}
