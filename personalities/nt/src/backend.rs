//! What the NT memory and section services need from the kernel: page
//! mappings in the calling process's memory and storage for section pages.
//!
//! The personality decides *what* every page of an NT process is
//! ([`crate::vm::VirtualMemory`]: reserved or committed, protections, views)
//! and tells the kernel what to map through this trait; the kernel
//! carries it out on its own memory map (`user::Memory` in Poly's kernel,
//! ADR 0036) without second-guessing it. Host tests implement it with a
//! fake that records pages.

use crate::protect::HwProt;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;

/// What backs a new section's pages.
#[derive(Clone, Debug)]
pub enum SectionSource {
    /// Pagefile-backed memory: `size` bytes of zeros, shared by every view.
    /// `committed`: every page exists from the start (`SEC_COMMIT`);
    /// otherwise only pages committed with
    /// [`Backend::commit_section`] may be touched (`SEC_RESERVE`).
    Pagefile { size: u64, committed: bool },
    /// An image laid out as it is mapped (`SEC_IMAGE`): views map private
    /// copies of its pages.
    Image(Arc<[u8]>),
    /// The pages of a file object (a data file mapping), by the kernel's
    /// key for the file.
    File { file: u64 },
}

/// Where the pages of a view come from.
#[derive(Clone, Debug)]
pub enum ViewSource {
    /// A section's pages, by the backing id [`Backend::create_section`]
    /// returned.
    Section(u64),
    /// Bytes of their own: an image relocated for this view.
    Bytes(Arc<[u8]>),
}

/// One piece of a view to map.
#[derive(Clone, Debug)]
pub struct ViewMap {
    pub source: ViewSource,
    /// Offset in the source of the piece's first byte (page-aligned).
    pub offset: u64,
    pub prot: HwProt,
    /// Shared with every other view of the source (writes are seen there);
    /// otherwise a private copy-on-write mapping.
    pub shared: bool,
}

/// The kernel's side of the memory services, for one process's memory.
/// Addresses and lengths are page-aligned and inside user space; the
/// personality never asks for a mapping over one it has not released.
pub trait Backend {
    /// Map fresh zero-filled private pages at `[addr, addr + len)`.
    fn commit(&mut self, addr: u64, len: u64, prot: HwProt) -> Result<(), NtStatus>;

    /// Unmap whatever is mapped in `[addr, addr + len)` and free its private
    /// pages. Holes are fine.
    fn release(&mut self, addr: u64, len: u64);

    /// Change the rights of the pages mapped in `[addr, addr + len)`.
    fn protect(&mut self, addr: u64, len: u64, prot: HwProt);

    /// Map `len` bytes of a view piece at `addr`.
    fn map_view(&mut self, addr: u64, len: u64, map: &ViewMap) -> Result<(), NtStatus>;

    /// Write `data` into the process's memory at `addr` (pages that are
    /// committed and writable): the TEB and PEB of a new process.
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), NtStatus>;

    /// Make the pages of a new section; returns the kernel's id for them.
    fn create_section(&mut self, source: &SectionSource) -> Result<u64, NtStatus>;

    /// Commit `[offset, offset + len)` of a `SEC_RESERVE` section's pages:
    /// every view of the section may touch them from now on.
    fn commit_section(&mut self, backing: u64, offset: u64, len: u64);

    /// The section's object is gone: free its pages once no view maps them.
    fn delete_section(&mut self, backing: u64);

    /// The size of a file object's contents, by the kernel's key for it.
    fn file_size(&mut self, file: u64) -> Option<u64>;

    /// A file object's whole contents (to parse an image).
    fn read_file(&mut self, file: u64) -> Result<Vec<u8>, NtStatus>;

    /// A file object was deleted: the kernel lets go of what its key names.
    fn file_deleted(&mut self, _file: u64) {}
}
