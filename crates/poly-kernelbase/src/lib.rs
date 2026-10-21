//! The logic of our `kernelbase.dll` and `kernel32.dll` (ADR 0079), as
//! host-tested code.
//!
//! The DLLs themselves (`personalities/nt/win32`) are thin: each export
//! checks its arguments and converts them with this crate, makes the
//! `Nt*` / `Rtl*` calls of our `ntdll`, and converts the result back. What
//! can be decided without a running process is here:
//!
//! - [`error`]: the Win32 error codes (`winerror.h`) and the `NTSTATUS` →
//!   Win32 mapping (`RtlNtStatusToDosError`, [`poly_rtl::status`]), plus
//!   the few places where a function documents a different code.
//! - [`path`]: DOS paths: their kinds, `GetFullPathNameW`'s normalisation,
//!   `RtlDosPathNameToNtPathName_U`'s `\??\` form, `FindFirstFileW`'s split
//!   into a directory and a pattern, `GetTempPathW`'s choice.
//! - [`env`]: the environment block (`GetEnvironmentVariableW`,
//!   `SetEnvironmentVariableW`, `ExpandEnvironmentStringsW`).
//! - [`file`]: `CreateFileW`'s dispositions, access and flags, the file
//!   pointer, `WIN32_FIND_DATAW` from `FILE_BOTH_DIR_INFORMATION`.
//! - [`sync`]: wait timeouts and results, object names, the SRW lock.
//! - [`memory`]: `VirtualAlloc`/`VirtualFree`, `LocalAlloc`/`GlobalAlloc`
//!   flag checks.
//! - [`message`]: `FormatMessageW` for system messages and its inserts.
//! - [`version`]: `GetVersion`/`GetVersionExW` and the manifest rule,
//!   `SYSTEM_INFO`.
//! - [`exports`]: which functions `kernel32` forwards to `kernelbase`, by
//!   which API-set contract.
//! - [`text`]: the "copy to a caller's buffer or report the size" rule many
//!   functions share, and ANSI (code page 1252) conversions.
//! - [`registry`]: the registry API's predefined keys, buffer rules, type
//!   restrictions and conversions (ADR 0083).
//! - [`advapi`]: which functions `advapi32` forwards to `kernelbase`, by
//!   which API-set contract (ADR 0083).
//!
//! Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's reference pages
//! for each function, "Naming Files, Paths, and Namespaces", "File path
//! formats on Windows systems", "System Error Codes", "Environment
//! Variables", "Kernel object namespaces", "Targeting your application for
//! Windows", the PE format specification, and the layouts and constants of
//! the public SDK headers (`winerror.h`, `winbase.h`, `winnt.h`,
//! `minwinbase.h`, `fileapi.h`, `sysinfoapi.h`, `ntifs.h`, `winternl.h`).
//! Where they leave a behaviour open the code says what it picks ("to be
//! confirmed by the spec agent", ADR 0079).

#![no_std]

extern crate alloc;

pub mod advapi;
pub mod env;
pub mod error;
pub mod exports;
pub mod file;
pub mod memory;
pub mod message;
pub mod path;
pub mod registry;
pub mod sync;
pub mod text;
pub mod version;

pub use poly_ntobj::NtStatus;

/// A UTF-16 string from a `&str` (for tables and tests).
pub fn wide(s: &str) -> alloc::vec::Vec<u16> {
    s.encode_utf16().collect()
}
