//! The NT loader that our `ntdll` runs (ADR 0071), as host-tested logic.
//!
//! [`Loader`] keeps the process's modules and decides everything a load,
//! an unload or a thread's start and end needs: where an image goes, how it
//! is relocated (`poly_pe::relocate`), what each import resolves to (by
//! name with the hint, by ordinal, through forwarders and API sets, keeping
//! still-valid bound imports), which DLLs a delay-load thunk brings in, the
//! static TLS index and each thread's TLS blocks, the reference counts, the
//! documented DLL search order, and the PEB's loader lists in process
//! memory. It touches the process only through [`Process`], so the tests
//! run it against a fake address space and PE images they build with
//! [`builder`].
//!
//! **The loader never calls user code.** A call that makes modules need
//! their TLS callbacks or entry points returns them as [`Callout`]s, in the
//! order to run them; `ntdll` runs them under the loader lock with no
//! borrow of the loader held, so a `DllMain` may call back into the loader
//! (Learn: "Dynamic-Link Library Best Practices"). A `DllMain` that fails
//! `DLL_PROCESS_ATTACH` is reported back with [`Loader::fail_load`].
//!
//! Sources: the PE Format specification (Learn), "Dynamic-link library
//! search order", "Windows API sets" / "API set loader operation",
//! `DllMain` (entry-point function), "Thread Local Storage", `LoadLibraryW`,
//! `FreeLibrary`, `GetProcAddress`, `ResolveDelayLoadedAPI`; layouts and
//! constants from `winnt.h`, `winternl.h` and `ntstatus.h`. No input makes
//! it panic: malformed images are `STATUS_INVALID_IMAGE_FORMAT` (or a more
//! specific status), and every address it computes is checked.

#![no_std]

extern crate alloc;

pub mod apiset;
pub mod builder;
mod ldrdata;
mod loader;
pub mod path;

#[cfg(test)]
mod tests;

pub use ldrdata::{
    Bitness, LDR_IN_INIT_ORDER, LDR_IN_LOAD_ORDER, LdrLayout, PEB_LDR, walk as walk_list, walk_in as walk_list_in,
};
pub use ldrdata::{ENTRY_BASE_NAME, ENTRY_IN_INIT_ORDER, ENTRY_IN_LOAD_ORDER, ENTRY_IN_MEMORY_ORDER};
pub use ldrdata::{ENTRY_DLL_BASE, ENTRY_ENTRY_POINT, ENTRY_FULL_NAME, ENTRY_SIZE_OF_IMAGE, LDR_IN_MEMORY_ORDER};
pub use loader::{
    ADDREF_PIN, Callout, CalloutKind, Config, DelayFailure, LoadOp, Loader, MAX_FORWARDS, ModuleId, ModuleInfo,
    TEB_TLS_POINTER, Unload,
};

use alloc::vec::Vec;
pub use poly_ntobj::NtStatus;

/// `DLL_PROCESS_DETACH`, `DLL_PROCESS_ATTACH`, `DLL_THREAD_ATTACH`,
/// `DLL_THREAD_DETACH` (`winnt.h`).
/// The generated DLL an import binds to when the function (or the DLL) is
/// missing and this DLL is on the search path (ADR 0155, `poly-stubs`).
pub const STUB_DLL: &str = "polystub.dll";

/// The export of [`STUB_DLL`] that stands for `symbol` (a name, or
/// `#<ordinal>`) of `dll` as an importing module names it: the DLL name
/// in lower case without `.dll`, `$`, the symbol.
pub fn stub_key(dll: &[u8], symbol: &str) -> alloc::string::String {
    let mut name: alloc::string::String = dll.iter().map(|&b| char::from(b.to_ascii_lowercase())).collect();
    if let Some(short) = name.strip_suffix(".dll") {
        name = short.into();
    }
    alloc::format!("{name}${symbol}")
}

pub mod reason {
    pub const PROCESS_DETACH: u32 = 0;
    pub const PROCESS_ATTACH: u32 = 1;
    pub const THREAD_ATTACH: u32 = 2;
    pub const THREAD_DETACH: u32 = 3;
}

/// The `NTSTATUS` values the loader returns (`ntstatus.h`).
pub mod status {
    use poly_ntobj::NtStatus;
    pub const STATUS_SUCCESS: NtStatus = NtStatus(0);
    pub const STATUS_INVALID_PARAMETER: NtStatus = NtStatus(0xC000_000D);
    pub const STATUS_NO_MEMORY: NtStatus = NtStatus(0xC000_0017);
    pub const STATUS_CONFLICTING_ADDRESSES: NtStatus = NtStatus(0xC000_0018);
    pub const STATUS_ACCESS_VIOLATION: NtStatus = NtStatus(0xC000_0005);
    pub const STATUS_OBJECT_NAME_NOT_FOUND: NtStatus = NtStatus(0xC000_0034);
    pub const STATUS_OBJECT_PATH_NOT_FOUND: NtStatus = NtStatus(0xC000_003A);
    pub const STATUS_PROCEDURE_NOT_FOUND: NtStatus = NtStatus(0xC000_007A);
    pub const STATUS_INVALID_IMAGE_FORMAT: NtStatus = NtStatus(0xC000_007B);
    pub const STATUS_DLL_NOT_FOUND: NtStatus = NtStatus(0xC000_0135);
    pub const STATUS_ORDINAL_NOT_FOUND: NtStatus = NtStatus(0xC000_0138);
    pub const STATUS_ENTRYPOINT_NOT_FOUND: NtStatus = NtStatus(0xC000_0139);
    pub const STATUS_DLL_INIT_FAILED: NtStatus = NtStatus(0xC000_0142);
}

/// `PAGE_*` protections (`winnt.h`).
pub mod page {
    pub const NOACCESS: u32 = 0x01;
    pub const READONLY: u32 = 0x02;
    pub const READWRITE: u32 = 0x04;
    pub const EXECUTE: u32 = 0x10;
    pub const EXECUTE_READ: u32 = 0x20;
    pub const EXECUTE_READWRITE: u32 = 0x40;
}

/// The page size the loader protects in.
pub const PAGE_SIZE: u64 = 0x1000;

/// What the loader needs from the process it runs in. In `ntdll` these are
/// system calls and the process heap; in the tests, a fake address space.
pub trait Process {
    /// The whole file at this NT path (`\??\C:\...`), or why not
    /// (`STATUS_OBJECT_NAME_NOT_FOUND`, `STATUS_OBJECT_PATH_NOT_FOUND`, ...:
    /// the search goes on to the next directory).
    fn read_file(&mut self, nt_path: &[u16]) -> Result<Vec<u8>, NtStatus>;

    /// The DLL at this NT path, for loading it: mapped as an image section
    /// view by the kernel ([`ImageFile::Mapped`]: its clean pages are shared
    /// with every other process that maps the file, ADR 0188), or the whole
    /// file ([`ImageFile::Bytes`], which the loader lays out in memory of
    /// its own). An error means the file is not there (the search goes on).
    /// By default the file is read.
    fn open_image(&mut self, nt_path: &[u16]) -> Result<ImageFile, NtStatus> {
        self.read_file(nt_path).map(ImageFile::Bytes)
    }

    /// Unmap a view [`open_image`](Self::open_image) mapped.
    fn unmap_image(&mut self, _base: u64) {}

    /// Reserve and commit `size` bytes of zeroed read/write memory at
    /// exactly `base` (failing with `STATUS_CONFLICTING_ADDRESSES` if any
    /// of it is in use), or anywhere (64 KiB aligned) when `base` is `None`.
    fn allocate(&mut self, base: Option<u64>, size: u64) -> Result<u64, NtStatus>;

    /// Give back an allocation [`allocate`](Self::allocate) made.
    fn free(&mut self, base: u64);

    /// Set the `PAGE_*` protection of `[addr, addr + size)`. For an image
    /// the kernel mapped (a section view), a writable protection means
    /// copy-on-write.
    fn protect(&mut self, addr: u64, size: u64, protect: u32) -> Result<(), NtStatus>;

    /// The process memory `[addr, addr + len)`, which must be mapped and
    /// readable and writable at the time (the loader only asks for memory
    /// it made so).
    fn memory(&mut self, addr: u64, len: usize) -> Result<&mut [u8], NtStatus>;

    /// `size` zeroed bytes for the loader's structures (loader table
    /// entries, names, TLS arrays and blocks), 16-byte aligned: the
    /// process heap.
    fn alloc(&mut self, size: usize) -> Result<u64, NtStatus>;

    /// Free what [`alloc`](Self::alloc) returned.
    fn free_alloc(&mut self, addr: u64);
}

/// A DLL file as [`Process::open_image`] hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageFile {
    /// Mapped by the kernel (`NtMapViewOfSection` of a `SEC_IMAGE`
    /// section): laid out at `base`, `size` bytes, with each section's
    /// protection. `relocate`: the view is not where the image's bytes say
    /// (`STATUS_IMAGE_NOT_AT_BASE`), the loader applies the relocations.
    Mapped { base: u64, size: u64, relocate: bool },
    /// The file's bytes.
    Bytes(Vec<u8>),
}

/// Little-endian reads and writes through a [`Process`].
pub(crate) fn read_u64(p: &mut dyn Process, addr: u64) -> Result<u64, NtStatus> {
    let b = p.memory(addr, 8)?;
    Ok(u64::from_le_bytes(b[..8].try_into().map_err(|_| status::STATUS_ACCESS_VIOLATION)?))
}

pub(crate) fn read_u32(p: &mut dyn Process, addr: u64) -> Result<u32, NtStatus> {
    let b = p.memory(addr, 4)?;
    Ok(u32::from_le_bytes(b[..4].try_into().map_err(|_| status::STATUS_ACCESS_VIOLATION)?))
}

pub(crate) fn write_bytes(p: &mut dyn Process, addr: u64, data: &[u8]) -> Result<(), NtStatus> {
    let m = p.memory(addr, data.len())?;
    if m.len() < data.len() {
        return Err(status::STATUS_ACCESS_VIOLATION);
    }
    m[..data.len()].copy_from_slice(data);
    Ok(())
}

pub(crate) fn write_u64(p: &mut dyn Process, addr: u64, v: u64) -> Result<(), NtStatus> {
    write_bytes(p, addr, &v.to_le_bytes())
}

/// A pointer-sized value (`size` 4 or 8 bytes) at `addr`.
pub(crate) fn read_ptr(p: &mut dyn Process, size: u64, addr: u64) -> Result<u64, NtStatus> {
    if size == 4 { read_u32(p, addr).map(u64::from) } else { read_u64(p, addr) }
}

/// Write a pointer-sized value; a value a 4-byte pointer cannot hold is
/// refused (`STATUS_INVALID_PARAMETER`) rather than truncated.
pub(crate) fn write_ptr(p: &mut dyn Process, size: u64, addr: u64, v: u64) -> Result<(), NtStatus> {
    if size == 4 {
        let v = u32::try_from(v).map_err(|_| status::STATUS_INVALID_PARAMETER)?;
        write_u32(p, addr, v)
    } else {
        write_u64(p, addr, v)
    }
}

pub(crate) fn write_u32(p: &mut dyn Process, addr: u64, v: u32) -> Result<(), NtStatus> {
    write_bytes(p, addr, &v.to_le_bytes())
}
