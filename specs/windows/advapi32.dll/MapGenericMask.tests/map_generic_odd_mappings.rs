//! MapGenericMask with unusual mappings: zero fields, fields with generic bits themselves, overlapping fields,
//! all bits; the result for a mask with all four generic bits.
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
/// winnt.h: `GENERIC_MAPPING`.
#[repr(C)]
#[derive(Clone, Copy)]
struct GENERIC_MAPPING {
    read: u32,
    write: u32,
    execute: u32,
    all: u32,
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MapGenericMask(access: *mut u32, mapping: *const GENERIC_MAPPING);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cases = [
        ("all fields zero", GENERIC_MAPPING { read: 0, write: 0, execute: 0, all: 0 }),
        (
            "read field has GENERIC_WRITE",
            GENERIC_MAPPING { read: GENERIC_WRITE | 0x1, write: 0x2, execute: 0x4, all: 0x7 },
        ),
        (
            "every field has GENERIC_ALL",
            GENERIC_MAPPING { read: GENERIC_ALL, write: GENERIC_ALL, execute: GENERIC_ALL, all: GENERIC_ALL },
        ),
        ("overlapping fields", GENERIC_MAPPING { read: 0xF, write: 0xF0, execute: 0xFF, all: 0xF0F }),
        (
            "every field 0xFFFFFFFF",
            GENERIC_MAPPING { read: 0xFFFF_FFFF, write: 0xFFFF_FFFF, execute: 0xFFFF_FFFF, all: 0xFFFF_FFFF },
        ),
        (
            "fields with MAXIMUM_ALLOWED and ACCESS_SYSTEM_SECURITY",
            GENERIC_MAPPING { read: MAXIMUM_ALLOWED, write: ACCESS_SYSTEM_SECURITY, execute: 0x1, all: 0x2 },
        ),
    ];
    for (name, m) in cases {
        sdsay!(t, "== {name}");
        for mask in [
            GENERIC_READ,
            GENERIC_WRITE,
            GENERIC_EXECUTE,
            GENERIC_ALL,
            GENERIC_READ | GENERIC_WRITE | GENERIC_EXECUTE | GENERIC_ALL,
            0x10 | GENERIC_READ,
            0x10,
            0xFFFF_FFFF,
        ] {
            let mut m2 = mask;
            // SAFETY: valid in/out mask and mapping.
            unsafe { MapGenericMask(&mut m2, &m) };
            sdsay!(t, "  {mask:#010x} -> {m2:#010x}");
        }
    }
}
