//! RtlAreAllAccessesGranted with generic bits, MAXIMUM_ALLOWED, ACCESS_SYSTEM_SECURITY and single bits:
//! the masks are compared bit by bit with no mapping.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

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

const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
const GENERIC_ALL_RIGHTS: u32 = 0x1000_0000;
const GENERIC_READ_RIGHT: u32 = 0x8000_0000;
const GENERIC_WRITE_RIGHT: u32 = 0x4000_0000;
const GENERIC_EXECUTE_RIGHT: u32 = 0x2000_0000;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlAreAllAccessesGranted(granted: u32, desired: u32) -> u8;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let specials = [
        GENERIC_READ_RIGHT,
        GENERIC_WRITE_RIGHT,
        GENERIC_EXECUTE_RIGHT,
        GENERIC_ALL_RIGHTS,
        MAXIMUM_ALLOWED,
        ACCESS_SYSTEM_SECURITY,
        0x0C00_0000,
    ];
    for s in specials {
        // SAFETY: pure computation.
        unsafe {
            say!(
                t,
                "bit {s:#010x}: granted=that desired=that: {}; granted=all desired=that: {}; granted=none desired=that: {}; granted=that desired=0x1: {}",
                RtlAreAllAccessesGranted(s, s),
                RtlAreAllAccessesGranted(0xFFFF_FFFF, s),
                RtlAreAllAccessesGranted(0, s),
                RtlAreAllAccessesGranted(s, 1)
            );
        }
    }
    let mut line = Buf::<300>::new();
    for bit in 0..32u32 {
        // SAFETY: pure computation.
        let r = unsafe { RtlAreAllAccessesGranted(1 << bit, 1 << bit) };
        let _ = write!(line, "{r}");
    }
    say!(t, "each single bit granted for itself, bit 0 to 31: {}", line.as_str());
}
