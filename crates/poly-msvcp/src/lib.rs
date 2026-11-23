//! The parts of Poly's `msvcp140.dll` (ADR 0199) that do not touch the running process, so
//! that they are tested on the host; the DLL (`personalities/nt/win32/msvcp140`) is the thin
//! glue over `kernel32` and `vcruntime140`.
//!
//! The behaviour is the spec sheets' (`specs/windows/msvcp140.dll/*.yaml`, spec-vcrt-1); the
//! record layouts are those of `specs/windows/msvcrt.dll/_CxxThrowException.yaml` (throw side)
//! and `specs/windows/vcruntime140.dll/__RTDynamicCast.yaml` (RTTI); constants cite the public
//! SDK header they come from. Nothing comes from another implementation of the library.
//!
//! - [`errc`]: the codes `std::_Throw_Cpp_error` takes, their `std::errc` values and texts.
//! - [`time`]: `_Xtime_get_ticks`' epoch and the absolute `xtime` arithmetic of `_Thrd_sleep`.
//! - [`mutex`]: the `_Mtx_*` types and what a lock, trylock and unlock do to the owner and count.
//! - [`classes`]: the standard exception classes the `_X*` helpers throw, and the assembly of
//!   their virtual tables, RTTI records and throw descriptors.
//! - [`winerror`]: the map from Windows error codes to generic values (`_Winerror_map`) and
//!   the length rule of `_Winerror_message`.
//! - [`exports`]: the fail-only exports (ADR 0155): which decorated names are data, how big
//!   their zeroed objects are, and the Rust source of the stubs.

#![no_std]

extern crate alloc;

pub mod classes;
pub mod errc;
pub mod exports;
pub mod mutex;
pub mod time;
pub mod winerror;
