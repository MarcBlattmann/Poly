//! RtlValidSid: TRUE (1) for revision 1 with a count of 0 to 15, whatever the authority; the
//! result is a BOOLEAN.
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

const SECURITY_NT_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 5];
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

/// A SID with authority {0,0,0,0,0,auth} and the given sub-authorities, built byte by byte.
fn rtl_sid(auth: u8, subs: &[u32]) -> SidBuf {
    let mut s = SidBuf::new();
    s.b[0] = 1;
    s.b[1] = subs.len().min(15) as u8;
    s.b[7] = auth;
    for (i, v) in subs.iter().enumerate().take(15) {
        s.b[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    s
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlInitializeSid(sid: PSID, authority: *const [u8; 6], count: u8) -> i32;
    fn RtlValidSid(sid: *const c_void) -> u8;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for count in [0u8, 1, 8, 15] {
        let mut sid = SidBuf::new();
        // SAFETY: `sid` can hold 15 sub-authorities.
        let v = unsafe {
            RtlInitializeSid(sid.as_psid(), &SECURITY_NT_AUTHORITY, count);
            RtlValidSid(sid.as_const())
        };
        say!(t, "initialized count {count}: {v}");
    }
    let sids = [
        ("null", rtl_sid(0, &[0])),
        ("world", rtl_sid(1, &[0])),
        ("system", rtl_sid(5, &[18])),
        ("administrators", rtl_sid(5, &[32, 544])),
        ("low label", rtl_sid(16, &[4096])),
    ];
    for (name, sid) in &sids {
        // SAFETY: valid SIDs.
        say!(t, "{name}: {}", unsafe { RtlValidSid(sid.as_const()) });
    }
    for auth in [[0u8; 6], [0xFF; 6], [1, 2, 3, 4, 5, 6]] {
        let mut sid = SidBuf::new();
        // SAFETY: `sid` can hold one sub-authority.
        let v = unsafe {
            RtlInitializeSid(sid.as_psid(), &auth, 1);
            RtlValidSid(sid.as_const())
        };
        say!(t, "authority {}: {v}", Hex(&auth));
    }
}
