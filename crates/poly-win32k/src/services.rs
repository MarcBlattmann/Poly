//! Poly's win32k system calls (ADR 0128, Decision 2).
//!
//! Windows renumbers its win32k services with every build and documents
//! none of its `NtUser*` / `NtGdi*` entry points; applications call the
//! documented `user32` and `gdi32` functions. So, as for the executive
//! services (ADR 0063), Poly owns the table: `win32u.dll`'s stubs are
//! generated from [`WIN32K_SERVICES`] (its `build.rs` reads this file), the
//! NT personality dispatches table 1 by it, and `cargo xtask win32` checks
//! that every stub carries its number.
//!
//! - A number is `TABLE_WIN32K << 12 | index` (`0x1000 | index`), the layout
//!   of every NT service number; only the low 14 bits count.
//! - Names start with `PolyUser` or `PolyGdi`, never Windows' `NtUser` /
//!   `NtGdi`: a program that calls a Windows `win32u` export directly must
//!   fail to bind rather than reach a Poly service with arguments it reads
//!   differently. Windows names are added to `win32u` only as adapters, once
//!   a spec sheet documents one (ADR 0128).
//! - The window manager's requests travel through one service,
//!   `PolyUserCall`, as records of [`crate::proto`] (the first slice
//!   replaced the per-function starting list of ADR 0128 with it: the
//!   requests are one enum both sides share, so adding one touches neither
//!   the table nor the stubs). Numbers are never reused.
//!
//! | Index | Service |
//! |---|---|
//! | `0x000` | `PolyUserCall(request, request_length, reply, reply_capacity, reply_length*)` |
//! | `0x001` | `PolyUserCommitSurface(hwnd, rects, count)` (the desktop link) |

/// The table number of win32k services (bits 12–13 of a service number).
pub const TABLE_WIN32K: u32 = 1;

/// One win32k service Poly knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Win32kService {
    /// `PolyUserCall(request, request_length, reply, reply_capacity,
    /// reply_length*)`: one [`crate::proto::Request`] in, its
    /// [`crate::proto::Reply`] out (`STATUS_BUFFER_TOO_SMALL` with the
    /// length needed when the reply does not fit).
    Call,
    /// `PolyUserCommitSurface(hwnd, rects, count)`: the rectangles drawn
    /// into a window's surface since the last commit (ADR 0128, Decision 5).
    CommitSurface,
}

/// Every win32k service with its number and its name as `win32u.dll`
/// exports its stub. The numbers are written out (`0x1000 | index`) so
/// `win32u`'s `build.rs` can read them as they are. Sorted by number.
pub const WIN32K_SERVICES: [(Win32kService, u32, &str); 2] =
    [(Win32kService::Call, 0x1000, "PolyUserCall"), (Win32kService::CommitSurface, 0x1001, "PolyUserCommitSurface")];

impl Win32kService {
    /// The service number `win32u`'s stub puts in `eax`.
    pub fn number(self) -> u32 {
        WIN32K_SERVICES.iter().find(|(s, _, _)| *s == self).map(|&(_, n, _)| n).expect("every service is in the table")
    }

    /// The name `win32u.dll` exports its stub under.
    pub fn name(self) -> &'static str {
        WIN32K_SERVICES.iter().find(|(s, _, _)| *s == self).map(|&(_, _, n)| n).expect("every service is in the table")
    }

    /// The service a number selects, if it is one of win32k's. Only the low
    /// 14 bits count (table and index), as for every NT service number.
    pub fn from_number(number: u32) -> Option<Win32kService> {
        let number = number & 0x3fff;
        WIN32K_SERVICES.iter().find(|&&(_, n, _)| n == number).map(|&(s, _, _)| s)
    }
}

/// The bytes of a `win32u` stub for service `number` as LLVM's assembler
/// encodes it: `mov r10, rcx` (`49 89 CA`), `mov eax, imm32` (`B8` + the
/// number), `syscall` (`0F 05`), `ret` (`C3`) — what `cargo xtask win32`
/// expects at every export.
pub fn stub_bytes(number: u32) -> [u8; 11] {
    let n = number.to_le_bytes();
    [0x49, 0x89, 0xCA, 0xB8, n[0], n[1], n[2], n[3], 0x0F, 0x05, 0xC3]
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn numbers_and_names_are_unique_and_round_trip() {
        let numbers: HashSet<u32> = WIN32K_SERVICES.iter().map(|s| s.1).collect();
        let names: HashSet<&str> = WIN32K_SERVICES.iter().map(|s| s.2).collect();
        assert_eq!(numbers.len(), WIN32K_SERVICES.len());
        assert_eq!(names.len(), WIN32K_SERVICES.len());
        for &(s, n, name) in &WIN32K_SERVICES {
            assert_eq!(Win32kService::from_number(n), Some(s));
            assert_eq!(s.number(), n);
            assert_eq!(s.name(), name);
        }
        // Bits above the table are ignored; table 0 is not win32k's.
        assert_eq!(Win32kService::from_number(0x1_0000 | 0x1001), Some(Win32kService::CommitSurface));
        assert_eq!(Win32kService::from_number(0x0004), None);
    }

    #[test]
    fn every_service_is_in_table_one_sorted_and_named_as_polys() {
        let mut last = 0;
        for &(_, n, name) in &WIN32K_SERVICES {
            assert_eq!(n >> 12, TABLE_WIN32K, "{name}");
            assert!(n > last, "{name} out of order");
            last = n;
            assert!(name.starts_with("PolyUser") || name.starts_with("PolyGdi"), "{name}");
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric()), "{name}");
        }
    }

    #[test]
    fn stubs_carry_their_number() {
        assert_eq!(stub_bytes(0x1003), [0x49, 0x89, 0xCA, 0xB8, 0x03, 0x10, 0, 0, 0x0F, 0x05, 0xC3]);
    }
}
