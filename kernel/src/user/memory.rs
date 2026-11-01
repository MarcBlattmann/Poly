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
//! | read a file page | map the page cache's frame if the [`PageSource`] has one ([`PageSource::cached_page`]: a file on a disk file system), read-only in a private region; else a fresh frame filled from the source (a private copy) |
//! | set permissions | update the entry (a read-only mapped page of a writable private region, written now; a page of a shared region, written for the first time) |
//! | copy on write | copy a page-cache page or a [`SharedFrame`] mapped into a private region into a frame of its own (or, for the frame's last user, take that frame back) |
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
//! that can read bytes at an offset: an image in memory, a VFS file through
//! `fs::cache::FileSource`) and a limit past which the pages read as zero,
//! which is how an ELF segment's last page gets its zero tail without a
//! copy of the file.
//!
//! A source with a page cache behind it (a file on a disk file system)
//! hands out the cache's own pages ([`PageSource::cached_page`]): the
//! memory maps that frame (not owned by the address space) and holds the
//! page while it is mapped, so `read`, `write` and every mapping of the
//! file see one copy of each page. A private region maps it read-only and
//! copies it on the first write (unless the page reaches past the region's
//! limit, which needs a private copy with a zero tail from the start). A
//! shared region (`MAP_SHARED`) maps it read-only first and writable on
//! the first write, from when on the page is dirty in the cache (written
//! back while mapped writable, and once more when unmapped): that is how
//! writes through the mapping reach the file. Shared mappings of a source
//! without a cache are refused ([`MemoryError::Unsupported`]).
//!
//! # Fork and shared frames
//!
//! [`Memory::fork`] (`fork.rs`) copies the region map and shares every
//! page of private regions with the child: the memory's own frames become
//! [`SharedFrame`]s, counted by `Arc`, mapped read-only in both; the first
//! write copies the page, unless the writer is the frame's last user, which
//! takes the frame back as its own. Shared anonymous memory
//! (`MAP_SHARED | MAP_ANONYMOUS`) is a shared mapping of an [`AnonShared`]
//! object whose pages are such frames, so every memory that maps it after
//! a fork finds the same ones. A frame is freed when its last reference
//! goes. See ADR 0041.

use super::{AddressSpace, MapError, UserFrame};
use crate::arch::user::Prot as PageProt;
use crate::fs::cache::CachedPage;
use crate::sync::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use poly_vm::{FaultAction, PageState, VmMap};

mod advice;
mod fork;
mod futex;
mod remap;
mod residency;
mod shared;
mod userfault;

pub use advice::Advice;
pub use shared::{AnonShared, SharedFrame};
pub use userfault::FillError;

pub use poly_vm::{Access, Backing, Kind, Layout, MapRequest, PAGE_SIZE, Placement, Prot, SegvCode, Vma};

/// A read failed (an I/O error, or the source shrank under the mapping).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoError;

/// Bytes that can back user pages: an executable image in kernel memory,
/// a VFS file (`fs::cache::FileSource`).
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

    /// The page-cache page holding the source's page at `offset` (a
    /// multiple of the page size), if a page cache keeps this source's
    /// pages: mappings then map that frame instead of a copy. `None` (the
    /// default) means every mapped page is a private copy. May block.
    fn cached_page(&self, _offset: u64) -> Option<Result<CachedPage, IoError>> {
        None
    }

    /// A number that names the object behind the source whichever
    /// `PageSource` value maps it (a file: its device and inode), for
    /// shared futex keys; `None` (the default): the value itself names it.
    /// Never has the top bit set (that is where kernel addresses live).
    fn object_id(&self) -> Option<u64> {
        None
    }

    /// Whether shared mappings (`MAP_SHARED`) of this source are possible:
    /// only through a page cache, whose pages carry writes to the file.
    fn shareable(&self) -> bool {
        false
    }

    /// The frame holding the page at `offset`, if the source is memory
    /// whose pages every mapping shares ([`AnonShared`]: shared anonymous
    /// memory). `None` (the default) for files.
    fn shared_frame(&self, _offset: u64) -> Option<Result<Arc<SharedFrame>, IoError>> {
        None
    }

    /// The serial of the System V shared memory segment the source is
    /// (`linux::shm`): its mappings are attachments, which shmdt(2) and
    /// exit detach. `None` (the default) for everything else.
    fn sysv_shm(&self) -> Option<u64> {
        None
    }

    /// The open file behind the source, if it is a VFS file: what
    /// `/proc/<pid>/maps` names the mapping by (ADR 0043).
    fn file(&self) -> Option<&alloc::sync::Arc<poly_vfs::OpenFile>> {
        None
    }

    /// A pseudo-path for `/proc/<pid>/maps` (`[vdso]`, say) for sources
    /// that are not files.
    fn label(&self) -> Option<&str> {
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

    /// The page-cache page for the file page at `offset`, if the source has
    /// one and the page lies wholly below the limit (or the limit is past
    /// the end of the file, where the cache's page is zero anyway). `Err`
    /// for a page past the end of the file (`SIGBUS`) or a failed read.
    fn cached_page(&self, offset: u64) -> Option<Result<CachedPage, FaultError>> {
        let len = self.source.len();
        if offset >= len {
            return Some(Err(FaultError::Bus));
        }
        if self.limit < (offset + PAGE_SIZE).min(len) {
            return None;
        }
        Some(self.source.cached_page(offset)?.map_err(|_| FaultError::Bus))
    }

    /// The shared frame for the page at `offset`, for a source that has
    /// them ([`PageSource::shared_frame`]); `SIGBUS` past its end.
    fn shared_frame(&self, offset: u64) -> Option<Result<Arc<SharedFrame>, FaultError>> {
        if offset >= self.source.len() {
            return Some(Err(FaultError::Bus));
        }
        Some(self.source.shared_frame(offset)?.map_err(|_| FaultError::Bus))
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
    /// An action the kernel cannot carry out (copy on write of a page that
    /// is neither a page-cache page nor a shared frame: never happens).
    Unsupported,
    /// The page is missing in a range registered with userfaultfd object
    /// `uffd` (userfaultfd(2)): the fault goes to that object, and the
    /// access is retried once the page is filled (`user::page_fault`).
    UserFault { uffd: u32, page: u64 },
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
            FaultError::UserFault { .. } => "userfaultfd fault",
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
    /// A shared mapping of a file without a page cache.
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
    /// membarrier(2): the `MEMBARRIER_CMD_REGISTER_*` commands done on
    /// this memory, as their bits. Shared by every task using the memory,
    /// copied by fork, empty after execve (a new memory).
    membarrier: core::sync::atomic::AtomicU32,
}

struct Inner {
    map: VmMap<FileRef>,
    /// Frames of populated pages whose region is `PROT_NONE`, by address.
    parked: BTreeMap<u64, UserFrame>,
    /// Page-cache pages mapped here (frames the address space does not
    /// own), by address. Each is unmapped before its entry goes.
    cached: BTreeMap<u64, CachedMapping>,
    /// Frames shared with other memories (copy on write after `fork`,
    /// shared anonymous memory), by address; not owned by the address
    /// space, unmapped before their entry goes.
    shared: BTreeMap<u64, SharedPage>,
}

/// A [`SharedFrame`] at some address of a memory.
struct SharedPage {
    frame: Arc<SharedFrame>,
    /// In the page tables (else the region is `PROT_NONE` for now).
    mapped: bool,
}

/// A page-cache page mapped at some address of a memory.
struct CachedMapping {
    page: CachedPage,
    /// Mapped by a shared region: writes reach the file.
    shared: bool,
    /// Mapped writable (shared regions only), counted in the page's
    /// writable mappings.
    writable: bool,
}

impl CachedMapping {
    /// The page-table rights for a page of a region with `prot`: never
    /// writable in a private region (written pages are copied), and in a
    /// shared one only once a write made it so.
    fn rights(&self, prot: Prot) -> Prot {
        if self.writable { prot } else { read_only(prot) }
    }

    fn make_writable(&mut self) {
        if !self.writable {
            self.page.map_writable();
            self.writable = true;
        }
    }

    fn make_read_only(&mut self) {
        if self.writable {
            self.page.unmap_writable();
            self.writable = false;
        }
    }
}

impl Drop for CachedMapping {
    fn drop(&mut self) {
        // What the mapping wrote goes to the file with the next writeback.
        self.make_read_only();
    }
}

/// `prot` without write permission (x86 pages are readable whenever they
/// are accessible).
fn read_only(prot: Prot) -> Prot {
    if prot.contains(Prot::WRITE) { prot.without(Prot::WRITE) | Prot::READ } else { prot }
}

impl fmt::Debug for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memory").field("root", &format_args!("{:#x}", self.space.root())).finish_non_exhaustive()
    }
}

/// A memory barrier on every CPU (membarrier(2) `MEMBARRIER_CMD_GLOBAL`):
/// a kernel TLB request for page 0, which is never a kernel mapping, that
/// every CPU takes and acknowledges; taking an interrupt is serializing.
pub fn global_barrier() {
    let mut flush = crate::arch::user::Flush::None;
    flush.add_page(0);
    crate::arch::tlb::flush_kernel(flush);
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}

impl Memory {
    /// An empty memory map with `layout` in a new address space.
    pub fn new(layout: Layout) -> Result<Memory, MemoryError> {
        let map = VmMap::new(layout)?;
        let space = AddressSpace::new().map_err(|_| MemoryError::OutOfMemory)?;
        let inner = Inner { map, parked: BTreeMap::new(), cached: BTreeMap::new(), shared: BTreeMap::new() };
        Ok(Memory { space: Arc::new(space), inner: Mutex::new(inner), membarrier: Default::default() })
    }

    /// membarrier(2): the registration commands done on this memory.
    pub fn membarrier_registrations(&self) -> u32 {
        self.membarrier.load(core::sync::atomic::Ordering::SeqCst)
    }

    /// membarrier(2): record registration command bits `bits`.
    pub fn membarrier_register(&self, bits: u32) {
        self.membarrier.fetch_or(bits, core::sync::atomic::Ordering::SeqCst);
    }

    /// A memory barrier on every CPU that runs this memory now
    /// (membarrier(2) `MEMBARRIER_CMD_PRIVATE_EXPEDITED`): a TLB request
    /// for page 0, which no user program maps, sent to each of them and
    /// waited for. Taking the interrupt and returning from it is
    /// serializing (SDM Vol. 3A §9.3), so each such CPU has ordered its
    /// memory accesses before the call returns.
    pub fn barrier(&self) {
        let mut flush = crate::arch::user::Flush::None;
        flush.add_page(0);
        self.space.sync_tlb(flush);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
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
        if let Backing::File { shared: true, file, .. } = &req.backing
            && !file.source.shareable()
        {
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

    /// A shared mapping of `file` from `offset` (`MAP_SHARED`): writes reach
    /// the file. Only for sources with a page cache
    /// ([`MemoryError::Unsupported`] otherwise).
    pub fn map_shared_file(
        &self,
        placement: Placement,
        len: u64,
        prot: Prot,
        file: FileRef,
        offset: u64,
    ) -> Result<u64, MemoryError> {
        self.map(MapRequest::file(len, prot, file, offset, true).at(placement))
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
            if inner.shared.contains_key(&page) {
                if let Err(e) = self.protect_shared(&mut inner, page, prot) {
                    result = Err(e);
                }
                continue;
            }
            if let Some(m) = inner.cached.get_mut(&page) {
                if prot == Prot::NONE {
                    // Brought back from the cache when accessible again.
                    self.space.unmap(page, PAGE_SIZE).expect("a mapped user page");
                    inner.cached.remove(&page);
                } else {
                    if !prot.contains(Prot::WRITE) {
                        m.make_read_only();
                    }
                    let rights = page_prot(m.rights(prot));
                    self.space.protect(page, PAGE_SIZE, rights).expect("the page is mapped");
                }
                continue;
            }
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

    /// Pages that have a frame: mapped (page-cache pages included), or
    /// parked under `PROT_NONE`.
    pub fn resident_pages(&self) -> u64 {
        let inner = self.inner.lock();
        self.space.resident_pages() + inner.parked.len() as u64 + inner.cached.len() as u64 + inner.shared.len() as u64
    }

    /// Pages that map a page-cache page.
    pub fn cached_pages(&self) -> u64 {
        self.inner.lock().cached.len() as u64
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
        // The memory's own frames have a single user until fork shares
        // them; a page-cache page in a private region is the cache's too.
        let refs = match (inner.cached.get(&page), inner.shared.get(&page)) {
            (Some(m), _) if !m.shared => 2,
            // Device memory always has another user: its owner.
            (_, Some(s)) if s.frame.is_device() => u32::MAX,
            (_, Some(s)) => u32::try_from(Arc::strong_count(&s.frame)).unwrap_or(u32::MAX),
            _ => 1,
        };
        let present = inner.parked.contains_key(&page)
            || inner.shared.contains_key(&page)
            || self.space.translate(page).is_some();
        let state = if present { PageState::Present { refs } } else { PageState::Absent };
        let shared = inner.map.find(addr).is_some_and(|v| v.is_shared());
        match inner.map.fault(addr, access, state) {
            FaultAction::ZeroFill { page, prot } => {
                let frame = UserFrame::zeroed().ok_or(FaultError::OutOfMemory)?;
                self.space.map_owned(page, frame, page_prot(prot)).map_err(map_error)
            }
            FaultAction::ReadFile { page, file, offset, prot, private_copy } => {
                if !private_copy && let Some(frame) = file.shared_frame(offset) {
                    return self.map_shared(inner, page, frame?, prot);
                }
                if !private_copy && let Some(cached) = file.cached_page(offset) {
                    return self.map_cached(inner, page, cached?, prot, shared, access);
                }
                // A frame of its own: a copy of the file's page. `prot`
                // lacks write for a read of a private region; a later write
                // only needs its permissions updated (the frame has one
                // user).
                let mut frame = UserFrame::zeroed().ok_or(FaultError::OutOfMemory)?;
                file.fill(offset, frame.bytes_mut())?;
                self.space.map_owned(page, frame, page_prot(prot)).map_err(map_error)
            }
            FaultAction::SetPermissions { page, prot } if inner.shared.contains_key(&page) => {
                self.shared_permissions(inner, page, prot, shared)
            }
            FaultAction::SetPermissions { page, prot } => {
                let mut rights = prot;
                if let Some(m) = inner.cached.get_mut(&page) {
                    if m.shared && access == Access::Write {
                        m.make_writable();
                    }
                    rights = m.rights(prot);
                }
                // Also the fix for a stale TLB entry: `protect` flushes.
                self.space.protect(page, PAGE_SIZE, page_prot(rights)).map_err(map_error)
            }
            FaultAction::CopyOnWrite { page, prot } if inner.shared.contains_key(&page) => {
                self.copy_shared(inner, page, prot)
            }
            FaultAction::CopyOnWrite { page, prot } => {
                // A page-cache page in a private region.
                let Some(m) = inner.cached.get(&page) else { return Err(FaultError::Unsupported) };
                let mut frame = UserFrame::zeroed().ok_or(FaultError::OutOfMemory)?;
                m.page.read(0, frame.bytes_mut());
                self.space.unmap(page, PAGE_SIZE).map_err(map_error)?;
                inner.cached.remove(&page);
                self.space.map_owned(page, frame, page_prot(prot)).map_err(map_error)
            }
            FaultAction::Segv(code) => Err(FaultError::Segv(code)),
            FaultAction::UserFault { page, uffd } => Err(FaultError::UserFault { uffd, page }),
        }
    }

    /// Map the page-cache page `cached` at `page` for a region with `prot`:
    /// read-only unless the region is shared and this is a write.
    fn map_cached(
        &self,
        inner: &mut Inner,
        page: u64,
        cached: CachedPage,
        prot: Prot,
        shared: bool,
        access: Access,
    ) -> Result<(), FaultError> {
        let mut m = CachedMapping { page: cached, shared, writable: false };
        if shared && access == Access::Write {
            m.make_writable();
        }
        let rights = page_prot(m.rights(prot));
        // SAFETY: the frame belongs to the page cache, which keeps it while
        // `m` holds the page; `m` goes into `inner.cached`, and every path
        // that removes an entry from there (`release`, `protect`, copy on
        // write, dropping the memory) unmaps the page first. It holds file
        // data the process may see: it maps the file. Private regions map
        // it without write permission, so the process never changes it.
        let mapped = unsafe { self.space.map_frames(page, m.page.frame().phys(), PAGE_SIZE, rights) };
        mapped.map_err(map_error)?;
        inner.cached.insert(page, m);
        Ok(())
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
    /// frames (parked ones too) and letting go of page-cache pages.
    fn release(&self, inner: &mut Inner, regions: &[Vma<FileRef>]) {
        for v in regions {
            self.space.unmap(v.start, v.size()).expect("regions lie in user space and are page-aligned");
            let mut inside = inner.parked.split_off(&v.start);
            let mut after = inside.split_off(&v.end);
            inner.parked.append(&mut after);
            drop(inside);
            let mut inside = inner.cached.split_off(&v.start);
            let mut after = inside.split_off(&v.end);
            inner.cached.append(&mut after);
            drop(inside);
            let mut inside = inner.shared.split_off(&v.start);
            let mut after = inside.split_off(&v.end);
            inner.shared.append(&mut after);
            drop(inside);
        }
    }
}

impl Drop for Memory {
    fn drop(&mut self) {
        // No lock: `&mut self` is exclusive, and this may run where a mutex
        // cannot be taken (the scheduler reaping the last thread).
        let inner = self.inner.get_mut();
        // Page-cache frames are not the address space's: unmap them before
        // letting go of them, in case a thread's state still holds the
        // space for a moment. Spinlocks only, so fine anywhere.
        for &page in inner.cached.keys().chain(inner.shared.keys()) {
            let _ = self.space.unmap(page, PAGE_SIZE);
        }
        // Shared frames only need the frame allocator: they go now.
        inner.shared.clear();
        // Letting go of files and cached pages takes file system locks
        // (sleeping ones, and the page cache's index lock, which does not
        // mask interrupts). During reaping that may be inside an interrupt
        // handler or in a thread that holds those very locks: the writeback
        // thread does it instead.
        let cached = core::mem::take(&mut inner.cached);
        let empty = VmMap::new(*inner.map.layout()).expect("a layout that was valid before");
        let map = core::mem::replace(&mut inner.map, empty);
        if crate::thread::reaping() {
            crate::fs::cache::release_later(alloc::boxed::Box::new((cached, map)));
        }
    }
}
