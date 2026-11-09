//! RtlAreAnyAccessesGranted(granted, desired): TRUE when at least one desired bit is in the granted mask (a BOOLEAN).
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

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlAreAnyAccessesGranted(granted: u32, desired: u32) -> u8;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let pairs: [(u32, u32); 16] = [
        (0, 0),
        (0, 1),
        (1, 0),
        (1, 1),
        (3, 1),
        (1, 3),
        (2, 1),
        (0x12, 0x10),
        (0x12, 0x01),
        (0x001F_01FF, 0x0012_0089),
        (0x0012_0089, 0x001F_01FF),
        (0xFFFF_FFFF, 0),
        (0, 0xFFFF_FFFF),
        (0xFFFF_FFFF, 0xFFFF_FFFF),
        (0x8000_0000, 0x7FFF_FFFF),
        (0x8000_0000, 0x8000_0001),
    ];
    for (g, d) in pairs {
        // SAFETY: pure computation.
        let r = unsafe { RtlAreAnyAccessesGranted(g, d) };
        say!(t, "granted {g:#010x} desired {d:#010x}: {r}");
    }
}
