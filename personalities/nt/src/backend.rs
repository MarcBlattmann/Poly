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
    /// copies of its pages, read from it as they are touched (ADR 0155).
    Image(Arc<dyn crate::image::ImageBytes>),
    /// The pages of a file object (a data file mapping), by the kernel's
    /// key for the file (the minimal file objects of ADR 0063).
    File { file: u64 },
    /// The pages of a VFS file a file object of NT file I/O is open on (a
    /// data file mapping, ADR 0079): its cached pages, so a shared view's
    /// writes are the file's contents.
    Vfs { file: Arc<poly_vfs::OpenFile> },
}

/// Where the pages of a view come from.
#[derive(Clone, Debug)]
pub enum ViewSource {
    /// A section's pages, by the backing id [`Backend::create_section`]
    /// returned.
    Section(u64),
    /// An image relocated for this view (ADR 0155).
    Image(Arc<dyn crate::image::ImageBytes>),
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
    /// Whether the kernel wants [`image_mapped`](Self::image_mapped) (its timeline, ADR 0247).
    fn wants_image_names(&self) -> bool {
        false
    }

    /// Image (executable or DLL) `path` was mapped in process `pid` at `base`, `size` bytes.
    fn image_mapped(&mut self, _pid: u32, _base: u64, _size: u64, _path: &[u16]) {}

    /// Put the window-state page (ADR 0249) at `addr`, one page the caller committed read-only
    /// there: the kernel's shared page replaces it. An error when the kernel has none.
    fn map_window_state_page(&mut self, _addr: u64) -> Result<(), NtStatus> {
        Err(poly_ntobj::status::STATUS_NOT_SUPPORTED)
    }

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

    /// Read the process's memory at `addr` (committed, readable pages: a
    /// thread's TEB, another process's memory for `NtReadVirtualMemory`). A backend that cannot
    /// read answers `STATUS_ACCESS_VIOLATION`.
    fn read(&mut self, _addr: u64, _buf: &mut [u8]) -> Result<(), NtStatus> {
        Err(poly_ntobj::status::STATUS_ACCESS_VIOLATION)
    }

    /// Whether the page at `addr` of a copy-on-write view was written through its mapping, so
    /// that it holds a private copy (`VirtualQuery` then shows it read-write). A backend that
    /// cannot tell says no.
    fn page_written(&mut self, _addr: u64) -> bool {
        false
    }

    /// Whether the page at `addr` is in the process's working set: it has a frame (a page that was
    /// only read counts, as Windows maps a zero page for it). A backend that cannot tell says no.
    fn page_resident(&mut self, _addr: u64) -> bool {
        false
    }

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
