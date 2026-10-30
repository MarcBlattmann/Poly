//! The logic of our C runtime (ADR 0084): `ucrtbase.dll` (reached through
//! the `api-ms-win-crt-*` API sets) and the classic `msvcrt.dll`, as
//! host-tested code.
//!
//! The DLLs themselves (`personalities/nt/win32/crt`, `ucrtbase`, `msvcrt`)
//! hold the process state (streams, descriptors, the heap, `atexit` tables)
//! and call `kernel32`; what can be decided without a process is here:
//!
//! - [`unit`]: narrow (`char`) and wide (`wchar_t`, UTF-16) code units, so
//!   every string function is written once.
//! - [`printf`]: the format engine of the `printf` family with the
//!   documented MSVC specifics (size prefixes `h l ll w I I32 I64 z t j`,
//!   `%S`/`%C`/`%Z`, `inf`/`nan(ind)`, the UCRT option bits).
//! - [`float`]: exact binary → decimal digits and correctly rounded decimal
//!   / hexadecimal → binary conversion (`strtod`).
//! - [`convert`]: `strtol` and its family, `atoi`, `_itoa`.
//! - [`args`]: "Parsing C command-line arguments".
//! - [`ctype`]: the "C" locale's character classes (`_pctype`), case
//!   mapping.
//! - [`time`]: `time_t` ↔ `struct tm`, `asctime`, `strftime`.
//! - [`sort`]: `qsort`, `bsearch`.
//! - [`errno`]: `errno` values, messages, Win32 error → `errno`.
//! - [`fmode`]: `fopen` mode strings, `_open` flags → `CreateFileW`, text
//!   mode translation.
//! - [`string`]: the `str*` / `wcs*` / `mem*` functions over raw strings.
//! - [`fenv`]: `_controlfp` words ↔ `MXCSR`.
//! - [`exports`]: what each DLL exports and, for `ucrtbase`, through which
//!   `api-ms-win-crt-*` contract.
//! - [`find`]: `_findfirst`'s `_finddata*_t` structures (ADR 0093).
//! - [`wildcard`]: wildcard expansion of `argv` (ADR 0093).
//! - [`spawn`]: `_spawn*` / `system` command lines, the file search, the
//!   descriptors passed to a child (ADR 0093).
//! - [`conio`]: which console key events `_getch` returns (ADR 0093).
//!
//! Sources (clean room, CLEAN_ROOM.md): the C standard (C11/C17), Microsoft
//! Learn's C runtime reference (each function's page, "Format specification
//! syntax: printf and wprintf functions", "Parsing C command-line
//! arguments", "errno constants", "Text and binary mode file I/O",
//! "Universal C runtime routines by category"), and the layouts and
//! constants of the public SDK headers (`corecrt_stdio_config.h`,
//! `corecrt_startup.h`, `errno.h`, `fcntl.h`, `ctype.h`, `float.h`,
//! `time.h`, `stdio.h`, `stdlib.h`). Export names come from the coverage
//! map. Where they leave a behaviour open the code says what it picks ("to
//! be confirmed by the spec agent", ADR 0084).

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod args;
pub mod conio;
pub mod convert;
pub mod ctype;
pub mod errno;
pub mod exports;
pub mod fenv;
pub mod find;
pub mod float;
pub mod fmode;
pub mod math;
pub mod printf;
pub mod scanf;
pub mod sort;
pub mod spawn;
pub mod string;
pub mod time;
pub mod unit;
pub mod wildcard;

pub use unit::Unit;
