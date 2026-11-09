//! RtlMapGenericMask with unusual mappings: zeros, all ones, mapping fields that contain generic bits
//! themselves (no second pass), and distinct values per field.
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

const GENERIC_ALL_RIGHTS: u32 = 0x1000_0000;
const GENERIC_READ_RIGHT: u32 = 0x8000_0000;
const GENERIC_WRITE_RIGHT: u32 = 0x4000_0000;
const GENERIC_EXECUTE_RIGHT: u32 = 0x2000_0000;
/// winnt.h: `GENERIC_MAPPING`.
#[repr(C)]
#[derive(Clone, Copy)]
struct GENERIC_MAPPING {
    read: u32,
    write: u32,
    execute: u32,
    all: u32,
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlMapGenericMask(access: *mut u32, mapping: *const GENERIC_MAPPING);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mappings = [
        ("zeros", GENERIC_MAPPING { read: 0, write: 0, execute: 0, all: 0 }),
        ("ones", GENERIC_MAPPING { read: 0xFFFF_FFFF, write: 0xFFFF_FFFF, execute: 0xFFFF_FFFF, all: 0xFFFF_FFFF }),
        ("distinct bits", GENERIC_MAPPING { read: 0x1, write: 0x2, execute: 0x4, all: 0x8 }),
        (
            "generic inside",
            GENERIC_MAPPING {
                read: GENERIC_WRITE_RIGHT | 1,
                write: GENERIC_EXECUTE_RIGHT | 2,
                execute: GENERIC_ALL_RIGHTS | 4,
                all: GENERIC_READ_RIGHT | 8,
            },
        ),
    ];
    let inputs = [
        GENERIC_READ_RIGHT,
        GENERIC_WRITE_RIGHT,
        GENERIC_EXECUTE_RIGHT,
        GENERIC_ALL_RIGHTS,
        GENERIC_READ_RIGHT | 0x10,
        0x10,
        0xFFFF_FFFF,
    ];
    for (name, m) in &mappings {
        let mut line = Buf::<300>::new();
        for inp in inputs {
            let mut a = inp;
            // SAFETY: valid access mask and mapping.
            unsafe { RtlMapGenericMask(&mut a, m) };
            let _ = write!(line, "{inp:#x}->{a:#x} ");
        }
        say!(t, "{name}: {}", line.as_str());
    }
}
