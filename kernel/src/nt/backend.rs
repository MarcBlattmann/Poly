//! The kernel's side of the NT memory and section services: it maps pages
//! into a process's `user::Memory` (ADR 0036) and keeps the pages of every
//! section. The NT personality's model (`poly_personality_nt::vm`) decides
//! what every page is; this only carries it out.
//!
//! - A pagefile section's pages are an [`AnonShared`] object, so every
//!   view (shared, in any process) maps the same frames; a copy-on-write
//!   view gets private copies that the first write forks.
//! - An image section's pages are made as views touch them (ADR 0155). An
//!   image of a file at its preferred base has one frame per page that
//!   every view of that file, in every process, maps until one writes it
//!   ([`SharedFrame`]s, ADR 0188); a relocated view's pages, and those of a
//!   minimal file object of ADR 0063 (bytes the kernel holds,
//!   `MemoryImage`), are private copies.
//! - A data section of a file of NT file I/O maps the file itself
//!   (`FileSource`, the page cache's pages on a cached file system): a
//!   shared view writes the file (ADR 0079).
//! - Committed private memory is demand-zero anonymous memory; a guard
//!   page is committed `PROT_NONE` and made accessible when it fires.

use crate::sync::SpinLock;
use crate::user::Memory;
use crate::user::UserFrame;
use crate::user::memory::{AnonShared, FileRef, IoError, MemoryImage, PageSource, Placement, Prot, SharedFrame};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_NO_MEMORY, STATUS_UNSUCCESSFUL};
use poly_personality_nt::HwProt;
use poly_personality_nt::backend::{Backend, SectionSource, ViewMap, ViewSource};
use poly_personality_nt::imageshare::{Registry, SharedPages};

/// The kernel's store of section pages, keyed by the id the NT model uses.
#[derive(Default)]
pub struct Sections {
    /// Each section's pages. A live view keeps its own reference through
    /// the `FileRef` in the kernel's memory map, so removing a backing here
    /// never pulls frames out from under a mapped view.
    map: BTreeMap<u64, Arc<dyn PageSource>>,
    /// A file object's contents, by the kernel's key for the file (the
    /// object's cookie): minimal file objects until NT file I/O exists.
    pub files: BTreeMap<u64, Arc<[u8]>>,
    next: u64,
}

impl Sections {
    fn source(&self, id: u64) -> Arc<dyn PageSource> {
        Arc::clone(self.map.get(&id).expect("live section backing"))
    }

    /// A section's pages, if it is still there (the compositor's mapping of a layer buffer,
    /// ADR 0224).
    pub fn pages(&self, id: u64) -> Option<Arc<dyn PageSource>> {
        self.map.get(&id).cloned()
    }
}

fn prot(p: HwProt) -> Prot {
    let mut bits = 0u64;
    if p.read || p.write || p.execute {
        bits |= 0x1; // x86 pages are readable whenever accessible
    }
    if p.write {
        bits |= 0x2;
    }
    if p.execute {
        bits |= 0x4;
    }
    Prot::from_bits(bits).unwrap_or(Prot::NONE)
}

/// The pages of an image, made when a view touches them (ADR 0155): a big
/// image is never held in kernel memory, its pages are read from its file.
/// An image of a file at its preferred base shares them with every other
/// section of the same file ([`ImageShare`], ADR 0188).
struct ImagePages {
    image: Arc<dyn poly_personality_nt::image::ImageBytes>,
    share: Option<Arc<ImageShare>>,
}

/// The frames of one image file's pages that some view maps.
struct ImageShare {
    pages: SpinLock<SharedPages<SharedFrame>>,
}

impl ImageShare {
    fn new() -> ImageShare {
        ImageShare { pages: SpinLock::new(SharedPages::new()) }
    }
}

/// Every image file some section uses, with its shared pages.
static IMAGE_SHARES: SpinLock<Registry<ImageShare>> = SpinLock::new(Registry::new());
/// Image pages made since boot, and image page faults that found another view's frame
/// (`poly.nt.mem`).
static IMAGE_PAGES_MADE: AtomicU64 = AtomicU64::new(0);
static IMAGE_PAGES_SHARED: AtomicU64 = AtomicU64::new(0);

/// `(images in use, pages made, faults that found a shared page)` (`poly.nt.mem`).
pub fn image_share_counts() -> (usize, u64, u64) {
    (IMAGE_SHARES.lock().live(), IMAGE_PAGES_MADE.load(Ordering::Relaxed), IMAGE_PAGES_SHARED.load(Ordering::Relaxed))
}

impl ImagePages {
    fn new(image: Arc<dyn poly_personality_nt::image::ImageBytes>) -> ImagePages {
        let share = image.share_key().map(|key| IMAGE_SHARES.lock().get_or_insert_with(key, ImageShare::new));
        ImagePages { image, share }
    }
}

impl PageSource for ImagePages {
    fn len(&self) -> u64 {
        self.image.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        Ok(self.image.read_at(offset, buf))
    }

    /// The image's shared frame for the page at `offset`: one a view of the same file maps, or a
    /// new one read from the image (no lock held meanwhile; if another view made the page in the
    /// meantime, that one wins and this frame is freed).
    fn shared_frame(&self, offset: u64) -> Option<Result<Arc<SharedFrame>, IoError>> {
        let share = self.share.as_ref()?;
        let page = offset & !(crate::user::memory::PAGE_SIZE - 1);
        if let Some(frame) = share.pages.lock().get(page) {
            IMAGE_PAGES_SHARED.fetch_add(1, Ordering::Relaxed);
            return Some(Ok(frame));
        }
        let Some(mut frame) = UserFrame::zeroed() else { return Some(Err(IoError)) };
        self.image.read_at(page, frame.bytes_mut());
        IMAGE_PAGES_MADE.fetch_add(1, Ordering::Relaxed);
        Some(Ok(share.pages.lock().insert(page, Arc::new(SharedFrame::user(frame)))))
    }
}

/// A [`Backend`] over one process's memory and the shared section store.
pub struct MemBackend<'a> {
    pub memory: &'a Arc<Memory>,
    pub sections: &'a mut Sections,
}

impl MemBackend<'_> {
    fn map_source(
        &self,
        addr: u64,
        len: u64,
        source: Arc<dyn PageSource>,
        offset: u64,
        p: HwProt,
        shared: bool,
    ) -> Result<(), NtStatus> {
        let file = FileRef::new(source);
        let r = if shared {
            self.memory.map_shared_file(Placement::Fixed(addr), len, prot(p), file, offset)
        } else {
            self.memory.map_file(Placement::Fixed(addr), len, prot(p), file, offset)
        };
        r.map(drop).map_err(|_| STATUS_NO_MEMORY)
    }
}

impl Backend for MemBackend<'_> {
    fn wants_image_names(&self) -> bool {
        crate::timeline::on()
    }

    fn image_mapped(&mut self, pid: u32, base: u64, size: u64, path: &[u16]) {
        crate::timeline::name_module(pid, base, size, &alloc::string::String::from_utf16_lossy(path));
    }

    fn commit(&mut self, addr: u64, len: u64, p: HwProt) -> Result<(), NtStatus> {
        self.memory.map_anonymous(Placement::Fixed(addr), len, prot(p)).map(drop).map_err(|_| STATUS_NO_MEMORY)
    }

    fn release(&mut self, addr: u64, len: u64) {
        let _ = self.memory.unmap(addr, len);
    }

    fn protect(&mut self, addr: u64, len: u64, p: HwProt) {
        let _ = self.memory.protect(addr, len, prot(p));
    }

    fn map_view(&mut self, addr: u64, len: u64, map: &ViewMap) -> Result<(), NtStatus> {
        match &map.source {
            ViewSource::Section(id) => {
                let source = self.sections.source(*id);
                self.map_source(addr, len, source, map.offset, map.prot, map.shared)
            }
            ViewSource::Image(image) => {
                let source: Arc<dyn PageSource> = Arc::new(ImagePages::new(Arc::clone(image)));
                self.map_source(addr, len, source, map.offset, map.prot, false)
            }
        }
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), NtStatus> {
        self.memory.write(addr, data).map_err(|_| poly_ntobj::status::STATUS_ACCESS_VIOLATION)
    }

    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), NtStatus> {
        self.memory.read(addr, buf).map_err(|_| poly_ntobj::status::STATUS_ACCESS_VIOLATION)
    }

    fn page_written(&mut self, addr: u64) -> bool {
        self.memory.page_written(addr)
    }

    fn page_resident(&mut self, addr: u64) -> bool {
        matches!(
            self.memory.page_kind(addr),
            crate::user::memory::PageKind::Present | crate::user::memory::PageKind::Zero
        )
    }

    fn create_section(&mut self, source: &SectionSource) -> Result<u64, NtStatus> {
        let page_source: Arc<dyn PageSource> = match source {
            SectionSource::Pagefile { size, .. } => Arc::new(AnonShared::new(*size)),
            SectionSource::Image(image) => Arc::new(ImagePages::new(Arc::clone(image))),
            SectionSource::File { file } => {
                let bytes = self.sections.files.get(file).ok_or(STATUS_UNSUCCESSFUL)?;
                Arc::new(MemoryImage(Arc::clone(bytes)))
            }
            // A file of NT file I/O: its own (cached) pages, as a shared
            // mmap(2) of it maps them (ADR 0079).
            SectionSource::Vfs { file } => Arc::new(crate::fs::cache::FileSource::of(file)),
        };
        self.sections.next += 1;
        let id = self.sections.next;
        self.sections.map.insert(id, page_source);
        Ok(id)
    }

    fn commit_section(&mut self, _backing: u64, _offset: u64, _len: u64) {
        // Pagefile pages come in on demand (AnonShared); nothing to do.
    }

    fn delete_section(&mut self, backing: u64) {
        self.sections.map.remove(&backing);
    }

    fn file_size(&mut self, file: u64) -> Option<u64> {
        self.sections.files.get(&file).map(|f| f.len() as u64)
    }

    fn read_file(&mut self, file: u64) -> Result<Vec<u8>, NtStatus> {
        self.sections.files.get(&file).map(|f| f.to_vec()).ok_or(STATUS_UNSUCCESSFUL)
    }
}
