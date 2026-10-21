//! The NT run-time library (`Rtl*`) that our `ntdll` exports, as
//! host-tested logic (ADR 0071).
//!
//! - [`heap`]: the NT heap (`RtlCreateHeap`, `RtlAllocateHeap`,
//!   `RtlFreeHeap`, `RtlReAllocateHeap`, `RtlSizeHeap`, ...), with the
//!   documented `HEAP_*` flags, over a [`heap::Platform`] that reserves and
//!   commits virtual memory.
//! - [`critsec`]: `RTL_CRITICAL_SECTION` (`winnt.h` layout): recursive
//!   ownership, spinning, and waiting on an event created when first needed.
//! - [`string`]: `UNICODE_STRING` and `ANSI_STRING` (`ntdef.h`) and the
//!   `Rtl*String*` functions over them.
//! - [`codepage`]: Poly's ANSI code page, Windows-1252.
//! - [`status`]: `RtlNtStatusToDosError`'s table.
//! - [`version`]: the `RTL_OSVERSIONINFO(EX)W` `RtlGetVersion` fills in.
//!
//! Everything comes from public Microsoft documentation (Learn's `Rtl*` and
//! `Heap*` reference pages, "INFO: Mapping NT Status Error Codes to Win32
//! Error Codes") and the layouts and constants of the public SDK/WDK headers
//! `winnt.h`, `ntdef.h`, `wdm.h`, `ntstatus.h` and `winerror.h`. Where the
//! documentation leaves a behaviour open the code says what it picks
//! ("to be confirmed by the spec agent", docs/adr/0071).
//!
//! The crate works on real memory through raw pointers, because that is
//! what these functions are: `ntdll` passes the caller's pointers straight
//! through, and the host tests pass pointers into their own buffers. It
//! never allocates from a Rust allocator, so the heap can be the global
//! allocator of the DLL it lives in.

#![no_std]

pub mod codepage;
pub mod critsec;
pub mod heap;
pub mod status;
pub mod string;
pub mod version;

pub use poly_ntobj::NtStatus;
