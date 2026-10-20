//! The memory of a process: its virtual memory areas (VMAs) and the pages
//! behind them, brought in on demand.
//!
//! A [`Memory`] is what Linux calls an `mm`: one [`AddressSpace`] (the page
//! tables) plus the region map of `poly_vm` (ADR 0026), which says what
//! every page *should* hold. Threads started with
//! [`spawn_in`](super::spawn_in) share it. Regions are created without
//! touching a single page; the first access to a page faults, and
//! [`Memory::fault`] asks the region map what that means
//! ([`poly_vm::VmMap::fault`]) and does it:
//!
//! | Region map says | Kernel does |
//! |---|---|
//! | zero-fill | map a fresh zeroed frame |
//! | read a file page | map a fresh frame filled from the [`PageSource`] (always a private copy: there is no page cache yet) |
//! | set permissions | update the entry (a read-only mapped page of a writable private region, written now) |
//! | copy on write | cannot happen yet: every frame has one user |
//! | `SIGSEGV` | nothing: the thread is killed (a signal later), a user copy returns `Efault` |
//!
//! Faults reach it from ring 3 (`arch::user::exception`) and from the
//! kernel's user-copy routines (`copy_from_user` and friends), so a valid
//! page that is not present yet never makes a copy fail. The region map is
//! behind a [`Mutex`]: resolving a fault may block (on the mutex now, on a
//! disk read once files come from the VFS), and nothing may copy user
//! memory while holding it.
//!
//! `mmap`, `munmap`, `mprotect` and `brk` are [`Memory::map`],
//! [`Memory::unmap`], [`Memory::protect`] and [`Memory::brk`]: each changes
//! the region map, then the page tables (unmapped pages' frames are freed
//! at once). A populated page whose region becomes `PROT_NONE` has no
//! present encoding on x86-64, so its frame is *parked* here, out of the
//! page tables, until the region becomes accessible again or is unmapped.
//!
//! # Backing
//!
//! A file-backed region holds a [`FileRef`]: a [`PageSource`] (anything
//! that can read bytes at an offset: an image in memory now, a VFS file
//! later) and a limit past which the pages read as zero, which is how an
//! ELF segment's last page gets its zero tail without a copy of the file.
//! Shared file mappings need a page cache and are refused for now
//! ([`MemoryError::Unsupported`]); copy on write (for `fork`) needs frame
//! reference counts, which `PageState::Present { refs }` already carries.

use super::{AddressSpace, MapError, UserFrame};
use crate::arch::user::Prot as PageProt;
use crate::sync::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use poly_vm::{FaultAction, PageState, VmMap};

pub use poly_vm::{Access, Backing, Kind, Layout, MapRequest, PAGE_SIZE, Placement, Prot, SegvCode, Vma};

/// A read failed (an I/O error, or the source shrank under the mapping).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoError;

/// Bytes that can back user pages: an executable image in kernel memory
/// now, a VFS file later.
pub trait PageSource: Send + Sync {
    /// Length in bytes.
    fn len(&self) -> u64;

    /// Whether the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copy bytes from `offset` on into the start of `buf`; returns how
    /// many: 0 only at or past the end, possibly fewer than asked for
    /// otherwise (callers loop). May block.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError>;

    /// The whole contents, if they are in kernel memory anyway: `exec`
    /// parses the headers in place instead of reading a copy.
    fn contents(&self) -> Option<&[u8]> {
        None
    }
}

/// A [`PageSource`] over bytes the kernel holds: `&'static [u8]` (images
/// built into the kernel), `Arc<[u8]>`, `Vec<u8>`.
#[derive(Clone, Debug)]
pub struct MemoryImage<B>(pub B);

impl<B: AsRef<[u8]> + Send + Sync> PageSource for MemoryImage<B> {
    fn len(&self) -> u64 {
        self.0.as_ref().len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        let bytes = self.0.as_ref();
        let start = usize::try_from(offset).map_or(bytes.len(), |o| o.min(bytes.len()));
        let n = buf.len().min(bytes.len() - start);
        buf[..n].copy_from_slice(&bytes[start..start + n]);
        Ok(n)
    }

    fn contents(&self) -> Option<&[u8]> {
        Some(self.0.as_ref())
    }
}

/// What a file-backed region maps: a source, and the file offset from
/// which on its pages read as zero (`u64::MAX` for the whole file). Two
/// references are equal when they name the same source and limit, which
/// is when the region map may merge neighbouring regions.
#[derive(Clone)]
pub struct FileRef {
    source: Arc<dyn PageSource>,
    limit: u64,
}

impl FileRef {
    /// All of `source`.
    pub fn new(source: Arc<dyn PageSource>) -> FileRef {
        FileRef { source, limit: u64::MAX }
    }

    /// The bytes of `source` below the file offset `limit`; pages read as
    /// zero from there on. An ELF segment maps its `p_filesz` bytes this
    /// way, so the rest of its last page is zero whatever the file holds
    /// after it.
    pub fn window(source: Arc<dyn PageSource>, limit: u64) -> FileRef {
        FileRef { source, limit }
    }

    pub fn source(&self) -> &Arc<dyn PageSource> {
        &self.source
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Fill `page` (zeroed) with the file page at `offset`. A page wholly
    /// beyond the end of the file is `SIGBUS`, as mmap(2) documents; the
    /// part of the last page past the end, or past the limit, stays zero.
    fn fill(&self, offset: u64, page: &mut [u8; PAGE_SIZE as usize]) -> Result<(), FaultError> {
        let len = self.source.len();
        if offset >= len {
            return Err(FaultError::Bus);
        }
        let n = (len.min(self.limit).saturating_sub(offset)).min(PAGE_SIZE) as usize;
        let mut done = 0;
        while done < n {
            match self.source.read_at(offset + done as u64, &mut page[done..n]) {
                Ok(read) if read > 0 => done += read,
                // The source shrank under the mapping, or could not be read.
                _ => return Err(FaultError::Bus),
            }
        }
        Ok(())
    }
}

impl PartialEq for FileRef {
    fn eq(&self, other: &FileRef) -> bool {
        Arc::ptr_eq(&self.source, &other.source) && self.limit == other.limit
    }
}

impl Eq for FileRef {}

impl fmt::Debug for FileRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let addr = Arc::as_ptr(&self.source) as *const () as usize;
        f.debug_struct("FileRef").field("source", &format_args!("{addr:#x}")).field("limit", &self.limit).finish()
    }
}

/// Why a page fault was not resolved: the access is not allowed, or the
/// page cannot be brought in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultError {
    /// `SIGSEGV`: nothing mapped there (`SEGV_MAPERR`) or the region does
    /// not allow the access (`SEGV_ACCERR`).
    Segv(SegvCode),
    /// `SIGBUS`: a file page beyond the end of the file, or a read error.
    Bus,
    /// No frame (or page table) for the page.
    OutOfMemory,
    /// The faulting thread has no memory map (a bare address space from
    /// [`spawn`](super::spawn), or a kernel thread).
    NoMemoryMap,
    /// An action the kernel cannot carry out yet (copy on write).
    Unsupported,
}

impl FaultError {
    /// A short name for log lines.
    pub fn name(self) -> &'static str {
        match self {
            FaultError::Segv(SegvCode::MapErr) => "SEGV_MAPERR",
            FaultError::Segv(SegvCode::AccErr) => "SEGV_ACCERR",
            FaultError::Bus => "SIGBUS",
            FaultError::OutOfMemory => "out of memory",
            FaultError::NoMemoryMap => "no memory map",
            FaultError::Unsupported => "unsupported fault action",
        }
    }
}

/// Why `map`, `unmap` or `protect` failed. Nothing has changed then, except
/// as noted on [`Memory::protect`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    /// The region map refused (bad range, no room, protection above the
    /// ceiling, ...).
    Vm(poly_vm::Error),
    /// No memory for the address space or a page.
    OutOfMemory,
    /// A shared file mapping (needs a page cache).
    Unsupported,
}

impl MemoryError {
    /// The Linux errno: the region map's, `ENOMEM` (12), or `ENODEV` (19,
    /// "the underlying filesystem of the specified file does not support
    /// memory mapping", mmap(2)); values from `asm-generic/errno-base.h`.
    pub const fn linux_errno(self) -> u64 {
        match self {
            MemoryError::Vm(e) => e.linux_errno(),
            MemoryError::OutOfMemory => 12,
            MemoryError::Unsupported => 19,
        }
    }
}

impl From<poly_vm::Error> for MemoryError {
    fn from(e: poly_vm::Error) -> MemoryError {
        MemoryError::Vm(e)
    }
}

/// The page-table rights for a region protection: the same bits.
fn page_prot(prot: Prot) -> PageProt {
    PageProt::from_bits(prot.bits())
}

fn map_error(e: MapError) -> FaultError {
    match e {
        MapError::OutOfMemory => FaultError::OutOfMemory,
        // The region map only hands out page-aligned user pages that are
        // not mapped yet, with an accessible protection.
        _ => FaultError::Segv(SegvCode::MapErr),
    }
}

/// The memory of a process (see the module docs). Shared by its threads
/// behind an `Arc`; the last reference frees every page.
pub struct Memory {
    space: Arc<AddressSpace>,
    inner: Mutex<Inner>,
}

struct Inner {
    map: VmMap<FileRef>,
    /// Frames of populated pages whose region is `PROT_NONE`, by address.
    parked: BTreeMap<u64, UserFrame>,
}

impl fmt::Debug for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memory").field("root", &format_args!("{:#x}", self.space.root())).finish_non_exhaustive()
    }
}

impl Memory {
    /// An empty memory map with `layout` in a new address space.
    pub fn new(layout: Layout) -> Result<Memory, MemoryError> {
        let map = VmMap::new(layout)?;
        let space = AddressSpace::new().map_err(|_| MemoryError::OutOfMemory)?;
        Ok(Memory { space: Arc::new(space), inner: Mutex::new(Inner { map, parked: BTreeMap::new() }) })
    }

    /// The page tables.
    pub fn space(&self) -> &Arc<AddressSpace> {
        &self.space
    }

    pub fn layout(&self) -> Layout {
        *self.inner.lock().map.layout()
    }

    /// Create a mapping (`mmap`): returns its address. A fixed placement
    /// replaces what was there; its pages are unmapped and freed.
    pub fn map(&self, req: MapRequest<FileRef>) -> Result<u64, MemoryError> {
        if matches!(req.backing, Backing::File { shared: true, .. }) {
            return Err(MemoryError::Unsupported);
        }
        let mut inner = self.inner.lock();
        let mapped = inner.map.map(req)?;
        self.release(&mut inner, &mapped.replaced);
        Ok(mapped.addr)
    }

    /// Private anonymous memory (`MAP_PRIVATE | MAP_ANONYMOUS`).
    pub fn map_anonymous(&self, placement: Placement, len: u64, prot: Prot) -> Result<u64, MemoryError> {
        self.map(MapRequest::anonymous(len, prot).at(placement))
    }

    /// A private mapping of `file` from `offset` (`MAP_PRIVATE`): writes go
    /// to private copies of its pages.
    pub fn map_file(
        &self,
        placement: Placement,
        len: u64,
        prot: Prot,
        file: FileRef,
        offset: u64,
    ) -> Result<u64, MemoryError> {
        self.map(MapRequest::file(len, prot, file, offset, false).at(placement))
    }

    /// Remove every mapping in `[addr, addr + len)` (`munmap`) and free the
    /// pages behind them. Holes are fine.
    pub fn unmap(&self, addr: u64, len: u64) -> Result<(), MemoryError> {
        let mut inner = self.inner.lock();
        let removed = inner.map.unmap(addr, len)?;
        self.release(&mut inner, &removed);
        Ok(())
    }

    /// Change the protection of `[addr, addr + len)` (`mprotect`), then of
    /// the pages present in it. Populated pages that become `PROT_NONE` are
    /// parked (kept, not mapped); parked pages that become accessible are
    /// mapped again. If mapping one back runs out of memory, its contents
    /// are lost and the call fails with [`MemoryError::OutOfMemory`] after
    /// the region map has changed.
    pub fn protect(&self, addr: u64, len: u64, prot: Prot) -> Result<(), MemoryError> {
        let mut inner = self.inner.lock();
        inner.map.protect(addr, len, prot)?;
        if len == 0 {
            return Ok(());
        }
        // The region map checked the range: aligned, inside the space.
        let end = addr + poly_vm::page_up(len).expect("checked by VmMap::protect");
        let rights = page_prot(prot);
        let mut result = Ok(());
        // TODO(perf): this visits every page of the range; a walk that
        // skips absent page tables would make large ranges cheap.
        for page in (addr..end).step_by(PAGE_SIZE as usize) {
            if prot == Prot::NONE {
                if let Some(frame) = self.space.take(page) {
                    inner.parked.insert(page, frame);
                }
            } else if let Some(frame) = inner.parked.remove(&page) {
                if self.space.map_owned(page, frame, rights).is_err() {
                    result = Err(MemoryError::OutOfMemory);
                }
            } else if self.space.translate(page).is_some() {
                self.space.protect(page, PAGE_SIZE, rights).expect("the page is mapped");
            }
        }
        result
    }

    /// Set the start of the program break (`exec`): the heap begins at the
    /// next page boundary.
    pub fn init_brk(&self, start: u64) -> Result<(), MemoryError> {
        Ok(self.inner.lock().map.init_brk(start)?)
    }

    /// Where the break started, if [`init_brk`](Self::init_brk) was called.
    pub fn brk_start(&self) -> Option<u64> {
        self.inner.lock().map.brk_start()
    }

    /// Move the program break to `new` (`brk(2)`): returns the break
    /// afterwards, which is the old one if the move was refused (and the
    /// current one for any `new` below the start, such as 0). Pages given up
    /// by a shrinking break are freed.
    pub fn brk(&self, new: u64) -> u64 {
        let mut inner = self.inner.lock();
        let result = inner.map.brk(new);
        self.release(&mut inner, &result.removed);
        result.brk
    }

    /// Create the stack region: the top `size` bytes below the layout's
    /// stack top. Returns its lowest address.
    pub fn setup_stack(&self, size: u64, prot: Prot) -> Result<u64, MemoryError> {
        Ok(self.inner.lock().map.setup_stack(size, prot)?)
    }

    /// The regions, in address order (for `/proc/<pid>/maps` later, and
    /// tests).
    pub fn regions(&self) -> Vec<Vma<FileRef>> {
        self.inner.lock().map.regions().cloned().collect()
    }

    /// Pages that have a frame: mapped, or parked under `PROT_NONE`.
    pub fn resident_pages(&self) -> u64 {
        let inner = self.inner.lock();
        self.space.resident_pages() + inner.parked.len() as u64
    }

    /// Resolve a page fault at `addr`: bring the page in or fix its
    /// permissions so that `access` succeeds when retried. Called by the
    /// fault handlers through [`user::page_fault`](super::page_fault), and
    /// by [`read`](Self::read) and [`write`](Self::write).
    pub fn fault(&self, addr: u64, access: Access) -> Result<(), FaultError> {
        let mut inner = self.inner.lock();
        self.resolve(&mut inner, addr, access)
    }

    fn resolve(&self, inner: &mut Inner, addr: u64, access: Access) -> Result<(), FaultError> {
        let page = poly_vm::page_down(addr);
        // Every frame has a single user until fork shares them.
        let present = inner.parked.contains_key(&page) || self.space.translate(page).is_some();
        let state = if present { PageState::Present { refs: 1 } } else { PageState::Absent };
        match inner.map.fault(addr, access, state) {
            FaultAction::ZeroFill { page, prot } => {
                let frame = UserFrame::zeroed().ok_or(FaultError::OutOfMemory)?;
                self.space.map_owned(page, frame, page_prot(prot)).map_err(map_error)
            }
            FaultAction::ReadFile { page, file, offset, prot, .. } => {
                // No page cache: every file page gets a frame of its own, so
                // `private_copy` changes nothing. `prot` lacks write for a
                // read of a private region; a later write only needs its
                // permissions updated (the frame has one user).
                let mut frame = UserFrame::zeroed().ok_or(FaultError::OutOfMemory)?;
                file.fill(offset, frame.bytes_mut())?;
                self.space.map_owned(page, frame, page_prot(prot)).map_err(map_error)
            }
            FaultAction::SetPermissions { page, prot } => {
                // Also the fix for a stale TLB entry: `protect` flushes.
                self.space.protect(page, PAGE_SIZE, page_prot(prot)).map_err(map_error)
            }
            FaultAction::CopyOnWrite { .. } => Err(FaultError::Unsupported),
            FaultAction::Segv(code) => Err(FaultError::Segv(code)),
        }
    }

    /// Copy from user memory at `addr` into `buf`, bringing pages in as a
    /// read would (the regions must allow reading). For the kernel's own
    /// use on a space that need not be loaded (a loader, a debugger); code
    /// acting for the running thread uses `copy_from_user`.
    pub fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), FaultError> {
        self.access(addr, buf.len(), Access::Read, |space, at, off, n| space.read(at, &mut buf[off..off + n]))
    }

    /// Copy `data` to user memory at `addr`, bringing pages in as a write
    /// would (the regions must allow writing); see [`read`](Self::read).
    pub fn write(&self, addr: u64, data: &[u8]) -> Result<(), FaultError> {
        self.access(addr, data.len(), Access::Write, |space, at, off, n| space.write(at, &data[off..off + n]))
    }

    /// Run `copy(space, at, offset, n)` for each piece of `[addr, addr +
    /// len)` within one page, after making that page accessible.
    fn access(
        &self,
        addr: u64,
        len: usize,
        access: Access,
        mut copy: impl FnMut(&AddressSpace, u64, usize, usize) -> Result<(), MapError>,
    ) -> Result<(), FaultError> {
        addr.checked_add(len as u64).ok_or(FaultError::Segv(SegvCode::MapErr))?;
        let mut inner = self.inner.lock();
        let mut off = 0;
        while off < len {
            let at = addr + off as u64;
            let n = ((PAGE_SIZE - at % PAGE_SIZE) as usize).min(len - off);
            self.resolve(&mut inner, at, access)?;
            copy(&self.space, at, off, n).map_err(map_error)?;
            off += n;
        }
        Ok(())
    }

    /// Unmap the pages of regions the region map removed, freeing their
    /// frames (parked ones too).
    fn release(&self, inner: &mut Inner, regions: &[Vma<FileRef>]) {
        for v in regions {
            self.space.unmap(v.start, v.size()).expect("regions lie in user space and are page-aligned");
            let mut inside = inner.parked.split_off(&v.start);
            let mut after = inside.split_off(&v.end);
            inner.parked.append(&mut after);
            drop(inside);
        }
    }
}
